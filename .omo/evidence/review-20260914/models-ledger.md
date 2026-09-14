# Lane: models-ledger
## Scope
- `src/models/events.rs`: 569 LOC
- `src/models/ledger.rs`: 42 LOC
- `src/models/messenger_policy.rs`: 143 LOC
- `src/models/mod.rs`: 15 LOC
- `src/models/session.rs`: 447 LOC
- `src/ledger/mod.rs`: 6 LOC
- `src/ledger/service.rs`: 943 LOC
- `src/memory/store.rs`: 235 LOC
- `src/memory/mod.rs`: 3 LOC
- `src/error.rs`: 45 LOC
- `src/lib.rs`: 48 LOC
Total: 11 files, 2,496 LOC. All files read fully.

## Findings
### [P0] Unbounded memory allocation and in-memory ranking in `MemoryStore::search`
- Location: src/memory/store.rs:106
- Evidence:
  ```rust
        let rows = sqlx::query_as::<_, MemoryRow>(
            "SELECT id, session_key, content, metadata_json, created_at, updated_at
             FROM memories WHERE session_key = ? ORDER BY updated_at DESC",
        )
  ```
- Why it matters: `MemoryStore::search` (and its aliases `keyword_search` and `similarity_search`) fetches the entire history of memories for a session via `fetch_all` without any SQL-level `LIMIT` or pagination. It then iterates over every row in memory, building a `HashMap<String, usize>` per row for term frequency counts, computing cosine similarity and substring matching, and deserializing JSON metadata. In long-running sessions with hundreds or thousands of stored memories, every search allocates unbounded heap memory and monopolizes CPU time on the async worker thread, creating an exploitable Denial of Service / OOM failure in production.
- Suggested fix: Add a bounded `LIMIT` clause (e.g., `LIMIT 200`) to the SQLite query to cap working memory and compute overhead, or migrate text search to SQLite FTS5 index tables before ranking.

### [P1] Race window in `sweep_recoverable` can steal and duplicate concurrently delivered obligations
- Location: src/ledger/service.rs:514
- Evidence:
  ```rust
            let result = sqlx::query(
                "UPDATE delivery_obligations
                 SET owner_pid = ?, owner_started_at = ?, attempts = attempts + 1, updated_at = ?
                 WHERE id = ? AND (owner_pid IS NULL OR owner_pid = ? OR owner_pid = ?)",
            )
  ```
- Why it matters: `sweep_recoverable` first selects candidates with `SELECT ... WHERE state IN ('pending', 'attempting', 'failed')` at line 482. However, the subsequent `UPDATE` statement that claims an obligation only checks `id` and `owner_pid`—it does NOT check that `state` remains non-delivered. If an obligation is successfully dispatched and transitioned to `'delivered'` by a concurrent delivery worker between the `SELECT` and `UPDATE`, `sweep_recoverable` still overwrites ownership, increments attempts, and returns the obligation as claimed. `recover_pending_delivery_obligations` will then re-dispatch the payload to Discord, resulting in duplicate user-visible messages and violating at-most-once/at-least-once delivery boundaries.
- Suggested fix: Include `AND state IN ('pending', 'attempting', 'failed')` in the `UPDATE delivery_obligations` WHERE clause so that obligations completed during the selection window cannot be reclaimed.

### [P1] Empty string `message_id` prefix stripping corrupts ledger duplicate and completion checks
- Location: src/ledger/service.rs:155
- Evidence:
  ```rust
        let alt = message_id.strip_prefix("discord:").unwrap_or_default();
  ```
- Why it matters: In `is_duplicate` (line 155), `is_completed` (line 170), and `get` (line 264), when `message_id` does not begin with `"discord:"`, `strip_prefix` returns `None`. Calling `unwrap_or_default()` causes `alt` to evaluate to `""` (empty string). The subsequent SQL queries bind `alt` in `WHERE message_id = ? OR message_id = ? ...`. This causes SQLite to evaluate `message_id = ''`. If any record in `delivery_ledger` has an empty `message_id`, every subsequent lookup for non-prefixed message IDs will match that empty row, falsely reporting incoming messages as duplicates, falsely reporting completion, or returning the wrong ledger entry.
- Suggested fix: Replace `unwrap_or_default()` with `unwrap_or(message_id)` so the fallback never binds an empty string when the prefix is absent.

### [P1] Regex typo in `SILENCE_NARRATION_RE` matches literal backslash or 's' instead of whitespace
- Location: src/models/events.rs:69
- Evidence:
  ```rust
        r"(?i)^[\s*_~`]*\(?\s*(silent|silence|no\s+response|no\s+reply)\s*\.?\)?[\\s*_~`]*$|^[\s*_~`]*[\x{1f507}\.\x{2026}]+[\s*_~`]*$",
  ```
- Why it matters: In the raw string regex literal `r"..."`, the closing delimiter specifies `[\\s*_~`]*$` with double backslashes. Because raw string literals do not escape backslashes, the regex engine compiles `[\\s...]` as a character class matching a literal backslash `\` or the literal letter `'s'`, NOT whitespace. Consequently, silence responses containing trailing whitespace (such as `"(silent) "` or `"(no reply)  "`) fail to match and will be posted as visible user messages, while responses ending with an `'s'` (such as `"(silent)s"`) unexpectedly match and are suppressed.
- Suggested fix: Replace `[\\s*_~`]*$` with `[\s*_~`]*$` using a single backslash for the whitespace character class.

### [P1] Silent error swallowing in ledger constituent inserts and cascading status updates
- Location: src/ledger/service.rs:207
- Evidence:
  ```rust
        let _ = sqlx::query(
            "INSERT INTO delivery_ledger_constituents (parent_delivery_id, constituent_id)
             VALUES (?, ?)
             ON CONFLICT(constituent_id) DO UPDATE SET parent_delivery_id = excluded.parent_delivery_id",
        )
  ```
- Why it matters: In `record_incoming_with_constituents` (lines 207, 220, 238) and `complete` (line 312), database executions against `delivery_ledger_constituents` and constituent entries in `delivery_ledger` discard their `Result` with `let _ = ...`. If an insertion or status update fails (due to connection reset, SQLite locking, or constraint violations), the error is silently ignored. The parent event reports success (`Ok(true)`), but constituent message IDs remain unregistered or stuck in `'in_progress'` forever, breaking deduplication and ledger accounting for grouped messages.
- Suggested fix: Wrap constituent operations in a database transaction (`pool.begin().await?`) and propagate errors using the `?` operator instead of discarding them.

### [P1] Staged memory write returns dummy Memory ID that breaks subsequent lookups
- Location: src/memory/store.rs:40
- Evidence:
  ```rust
        if crate::storage::write_approval_enabled() {
            let payload = serde_json::json!({
                "session_key": session.storage_key(),
                "content": &content,
                "metadata": &metadata,
            });
            let id =
  ```
- Why it matters: When `write_approval_enabled()` is true (lines 40-57), `remember` stages the payload into `pending_writes` and constructs an in-memory `Memory` instance using the pending write ID as its `id`. The memory is not inserted into the `memories` table. Any subsequent call to `store.get(&memory.id)` returns `None`, and `store.delete(&memory.id)` affects 0 rows. The API contract promises that a returned `Memory` is persisted and addressable by its `id`, but here the ID refers to an unapproved pending write, violating caller expectations and breaking workflows that immediately reference stored memories.
- Suggested fix: Return a distinct outcome type (e.g., `enum RememberOutcome { Stored(Memory), Staged(Uuid) }`) or update `store.get` and `store.delete` to be aware of staged pending writes.

### [P1] Error taxonomy collapses structured errors into stringly-typed variants discarding source chains
- Location: src/error.rs:33
- Evidence:
  ```rust
impl From<sqlx::Error> for OmonError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}
  ```
- Why it matters: `OmonError` maps `sqlx::Error` and `sqlx::migrate::MigrateError` to `Database(String)` via `.to_string()`, discarding underlying error types, SQLite error codes, and source error chains. Downstream callers cannot programmatically inspect error kinds (such as distinguishing transient timeouts, locking, and unique constraint violations) and are forced to rely on fragile string matching on error text (as seen in `service.rs:627`). Furthermore, `OmonError` lacks variants for serialization (`serde_json::Error`), forcing callers in `memory/store.rs:60` and `ledger/service.rs:355` to map JSON errors to `OmonError::Database`, falsely attributing serialization defects to database failures.
- Suggested fix: Preserve the underlying error types with `#[from]` and `#[source]` attributes (e.g., `Database(#[from] sqlx::Error)`), and add dedicated variants for `Serialization(#[from] serde_json::Error)` and `NotFound`.

### [P1] Missing terminal state guard in `DeliveryLedgerService::complete` allows overwriting delivered entries
- Location: src/ledger/service.rs:291
- Evidence:
  ```rust
        let result = sqlx::query(
            "UPDATE delivery_ledger
             SET status = ?, error = ?, completed_at = ?, updated_at = ?,
                 processing_latency_ms = MAX(0, CAST((julianday(?) - julianday(received_at)) * 86400000 AS INTEGER))
             WHERE message_id = ?",
        )
  ```
- Why it matters: The `complete` helper executes an unconditional `UPDATE delivery_ledger ... WHERE message_id = ?`. If a delivery was already marked `'delivered'` and a late error callback, duplicate completion event, or retry calls `mark_failed`, the delivered status is overwritten with `'failed'`, and `completed_at` is updated to the later timestamp, severely skewing `processing_latency_ms`. There is no check that the ledger entry is still in `in_progress`, allowing terminal states to be corrupted.
- Suggested fix: Add `AND status = 'in_progress'` to the WHERE clause, or return an error if attempting to transition an already delivered entry to failed.

### [P1] Unchecked integer cast in `is_process_alive` can pass negative PID to `libc::kill`
- Location: src/ledger/service.rs:64
- Evidence:
  ```rust
        let res = unsafe { libc::kill(pid as i32, 0) };
  ```
- Why it matters: `is_process_alive` is a public API taking `pid: u32`. If `pid > i32::MAX as u32`, `pid as i32` overflows into a negative integer. In POSIX `kill(2)`, passing a negative PID has special semantics: `-1` signals every process the caller has permission to signal, and `<-1` signals the entire process group `-pid`. Even with signal 0 (null signal), passing a negative number checks permission against entire process groups or all system processes instead of a single process, returning misleading liveness results.
- Suggested fix: Guard `if pid == 0 || pid > i32::MAX as u32 { return false; }` before invoking `libc::kill`.

### [P2] In-flight delivery theft due to asymmetric instance start timestamp comparison in `sweep_recoverable`
- Location: src/ledger/service.rs:493
- Evidence:
  ```rust
                        if started == current_started_at {
                            continue; // A live gateway instance still owns this row
                        }
  ```
- Why it matters: When checking whether an obligation owner is alive, if `pid != current_pid` but the process with `pid` is running on the host, the sweeper checks `if started == current_started_at`. However, `current_started_at` is the *current* process's start timestamp. Any other concurrently running gateway process (e.g. during a blue/green deployment or multi-instance gateway) will have a different start timestamp. The sweeper assumes that because `started != current_started_at`, the owning process must be dead, even though `is_process_alive` returned true. It then steals in-flight obligations and re-dispatches them concurrently.
- Suggested fix: When `pid != current_pid` and `is_process_alive(pid as u32)` is true, do not steal the obligation unless an external heartbeat or lease timeout has elapsed.

### [P2] Wildcard re-exports in `src/lib.rs` expose unintended internal surface and create shadowing hazards
- Location: src/lib.rs:18
- Evidence:
  ```rust
pub use agent::*;
pub use cron::*;
pub use discord::*;
  ```
- Why it matters: Wildcard re-exports (`pub use module::*`) are used across eight modules (`agent`, `cron`, `discord`, `drain_control`, `mirror`, `models`, `readiness`, `voice`). Any item marked `pub` for crate-internal convenience becomes part of the root public API. This pollutes the root namespace, obscures the library's intentional surface, and introduces risk of silent naming collisions when new types are added. In contrast, key ledger types such as `DeliveryObligation` and `DeliveryObligationState` are omitted from the root export despite being returned by `DeliveryLedgerService` APIs.
- Suggested fix: Replace glob re-exports with explicit item lists, and export `DeliveryObligation` and `DeliveryObligationState` at root.

### [P2] Dead and orphaned `DeliveryReceipt` domain model exported in `src/models/ledger.rs`
- Location: src/models/ledger.rs:17
- Evidence:
  ```rust
pub struct DeliveryReceipt {
  ```
- Why it matters: `DeliveryReceipt` is declared in `src/models/ledger.rs` and re-exported at the crate root, but is completely unused across the entire codebase. The ledger service instead uses `DeliveryLedgerEntry` and `DeliveryObligation`. Having multiple overlapping domain types representing message delivery status causes confusion for developers and consumers of the library.
- Suggested fix: Either remove `DeliveryReceipt` or document it as deprecated and migrate to `DeliveryLedgerEntry`.

### [P2] Strongly-typed status enums bypassed in favor of raw strings in ledger entities
- Location: src/ledger/service.rs:47
- Evidence:
  ```rust
    pub state: String,
  ```
- Why it matters: Although `DeliveryObligationState` and `DeliveryStatus` are defined, `DeliveryObligation.state` (line 47) and `DeliveryLedgerEntry.status` (line 98) store raw `String`s. Methods such as `update_obligation_state(id, state: &str, ...)` accept unvalidated string slices, and state checks throughout the service rely on string equality (`obligation.state == "pending"`). Typographical errors in state strings are not caught at compile time.
- Suggested fix: Use `DeliveryObligationState` and `DeliveryStatus` directly in the structs and service methods, implementing `sqlx::Type` or converting via `FromStr`/`TryFrom`.

### [P2] `SessionState` silently discards unknown JSON fields on read-modify-write
- Location: src/models/session.rs:268
- Evidence:
  ```rust
pub struct SessionState {
  ```
- Why it matters: `SessionState` is deserialized from SQLite `sessions.state_json` during session operations and re-serialized back to the database. By default, Serde silently drops unknown fields. If newer gateway versions, CLI tools, or external extensions add fields to `state_json`, reading and updating session state will permanently erase those unknown fields on write.
- Suggested fix: Add `#[serde(flatten)] pub extra: HashMap<String, serde_json::Value>` to `SessionState` to capture and preserve unrecognized fields across round-trips.

## Strengths
- **Robust collision-resistant session routing keys**: `SessionKey::storage_key` and `from_storage_key` implement strict length-prefixed encoding that cleanly handles arbitrary delimiter characters, UTF-8 multi-byte boundaries, and optional routing dimensions without key ambiguity.
- **Crash-resilient delivery ledger architecture**: The delivery ledger tracks obligations durably in SQLite with host PID and process start timestamp verification, preventing message loss during gateway restarts.
- **Platform-agnostic event model**: `InboundEvent` and `OutboundAction` provide clear, well-tagged Serde representations with distinct lifecycle semantics, isolating gateway protocol logic from platform adapters.
- **Prompt timestamp sanitization and idempotency**: `strip_leading_message_timestamps` and `format_message_timestamp` prevent runaway prompt prefix inflation across multi-turn LLM conversations.

## Notes
- **Local host assumption in crash recovery**: The process liveness check (`is_process_alive`) relies on POSIX `libc::kill(pid, 0)`, which inherently assumes all gateway workers execute on the same host and process namespace. If the SQLite database is shared across multiple hosts or containers, PID-based crash detection is invalid and requires distributed leases or heartbeats.
- **Approval staging decoupling**: `MemoryStore::remember` delegates staging to `pending_writes` when write approval is enabled, but `MemoryStore` itself provides no approval notification or reconciliation loop, relying on an external approval runner.
