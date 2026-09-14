# Lane: storage-db

## Scope

The SQLite storage layer, full-text search engine, messenger policy override store, module exports, and all 27 database migrations:
- `src/storage/db.rs` (2,189 LOC)
- `src/storage/message_search.rs` (380 LOC)
- `src/storage/messenger_policy.rs` (100 LOC)
- `src/storage/mod.rs` (23 LOC)
- `migrations/0001_initial.sql` (63 LOC)
- `migrations/0002_delivery_tracking.sql` (7 LOC)
- `migrations/0003_message_sequence.sql` (24 LOC)
- `migrations/0004_cron_leases.sql` (18 LOC)
- `migrations/0005_cron_runs_indexes.sql` (5 LOC)
- `migrations/0006_delivery_obligations.sql` (20 LOC)
- `migrations/0007_approval_allowlist.sql` (4 LOC)
- `migrations/0008_resume_pending.sql` (3 LOC)
- `migrations/0009_messages_platform_id.sql` (4 LOC)
- `migrations/0010_pending_writes.sql` (10 LOC)
- `migrations/0011_pairing_codes.sql` (14 LOC)
- `migrations/0012_discord_channel_cursors.sql` (5 LOC)
- `migrations/0013_cron_runs_owner_pid.sql` (2 LOC)
- `migrations/0014_bot_profiles.sql` (10 LOC)
- `migrations/0015_message_search_fts.sql` (92 LOC)
- `migrations/0016_messenger_policy.sql` (5 LOC)
- `migrations/0018_bot_cursors.sql` (15 LOC)
- `migrations/0019_dead_targets.sql` (9 LOC)
- `migrations/0020_obligation_owner.sql` (1 LOC)
- `migrations/0021_cron_outputs.sql` (10 LOC)
- `migrations/0022_cron_authority.sql` (15 LOC)
- `migrations/0023_cutover_receipts.sql` (30 LOC)
- `migrations/0024_pairing_state.sql` (14 LOC)
- `migrations/0025_cron_monitor_states.sql` (6 LOC)
- `migrations/0026_cron_incidents.sql` (11 LOC)
- `migrations/0027_cron_notepads.sql` (11 LOC)
- `migrations/0028_legacy_guild_session_conflicts.sql` (31 LOC)
- `migrations/0029_cutover_replacement_bytes.sql` (3 LOC)

Total: 3,134 LOC across 32 files.

## Findings

### [P0] Premature resume_pending clearing before turn recovery completes risks silent message loss
- Location: src/storage/db.rs:529
- Evidence:
```rust
        let storage_key = session_key.storage_key();
        let is_suspended = is_session_suspended(pool, &storage_key).await?;
        let cleared = clear_session_resume_pending(pool, &storage_key).await?;
```
- Why it matters: In `recover_resume_pending_sessions`, `clear_session_resume_pending` resets `resume_pending = 0` in SQLite before the unfinished turn is loaded, reconstructed, or routed. If the gateway process crashes while executing `find_last_unfinished_user_turn` or `multiplexer.route(event)`, or if routing fails, `resume_pending` is already cleared. On the subsequent restart, the gateway will not detect or retry the in-flight user turn, resulting in silent and unrecoverable message loss. Furthermore, for suspended sessions, `clear_session_resume_pending` is executed before skipping recovery, permanently discarding the pending turn even if the session is later unsuspended.
- Suggested fix: Clear `resume_pending` only after `multiplexer.route(event).await` completes successfully, and leave `resume_pending = 1` intact for suspended sessions.

### [P0] Unbounded table growth across cron_outputs, delivery_ledger, and FTS tables with no retention or cascade pruning
- Location: migrations/0021_cron_outputs.sql:1
- Evidence:
```sql
CREATE TABLE IF NOT EXISTS cron_outputs (
    profile TEXT NOT NULL DEFAULT '',
    job_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
```
- Why it matters: Multiple tables grow unboundedly in production with zero retention or pruning queries:
  1. `cron_outputs` inserts row entries on every cron run execution, but contains no foreign keys and has zero `DELETE` statements anywhere in the codebase. Command stdout/stderr accumulates indefinitely.
  2. `delivery_ledger` (`migrations/0001_initial.sql`) records every inbound delivery event but is never pruned or cleaned up.
  3. `delivery_ledger_constituents` (`src/storage/db.rs:203`) records constituent message chunks without foreign keys or deletion logic.
  4. `message_search_documents` and `message_search_fts` (`migrations/0015_message_search_fts.sql`) have no foreign keys to `messages` or `sessions` and no triggers on `DELETE FROM messages`. Pruned or deleted messages remain permanently stored and searchable in FTS5, creating both unbounded disk growth and a data privacy leak.
- Suggested fix: Add scheduled retention pruning tasks for `cron_outputs`, `delivery_ledger`, and `delivery_ledger_constituents`. Add an `AFTER DELETE ON messages` trigger to delete from `message_search_documents`.

### [P1] Discarding platform_message_id on turn recovery causes downstream reply failure and unindexed search
- Location: src/storage/db.rs:569
- Evidence:
```rust
                session: session_key.clone(),
                platform_message_id: String::new(),
                delivery_id,
```
- Why it matters: `UnfinishedTurn` correctly fetches `platform_message_id` from the transcript (`messages` table, lines 493, 510) and uses it to query the delivery ledger (line 542), but line 569 discards it by setting `platform_message_id: String::new()`. When this reconstructed event is routed to platform adapters (Discord/Slack), downstream handlers cannot thread, reply to, or acknowledge the original platform message. Furthermore, `MessageSearchIndex::index_inbound` ignores events with empty `platform_message_id` (line 191), meaning recovered user turns are never indexed into the search database.
- Suggested fix: Pass `platform_message_id: unfinished.platform_message_id.unwrap_or_default()` in the reconstructed `InboundEvent`.

### [P1] Synchronous blocking filesystem I/O inside async function while holding database transaction
- Location: src/storage/db.rs:877
- Evidence:
```rust
            let skill_dir = skill_file.with_file_name("");
            std::fs::create_dir_all(&skill_dir).map_err(|e| {
                crate::OmonError::ToolExecution(format!("failed to create skill dir: {e}"))
```
- Why it matters: In `apply_pending_write`, `std::fs::create_dir_all` and `std::fs::write` are synchronous blocking calls executed directly inside an `async fn` on a Tokio worker thread. Crucially, they run while holding an active transaction (`let mut transaction = pool.begin().await?`) on a pool configured with `max_connections(1)`. This stalls the single database connection for all async tasks across the entire application while disk I/O occurs, causing connection starvation and blocking the Tokio runtime.
- Suggested fix: Use `tokio::fs::create_dir_all` and `tokio::fs::write` (or `tokio::task::spawn_blocking`), and execute filesystem modifications outside the database transaction.

### [P1] Malformed pending write payload bricks scoped listing and rejection for all sessions
- Location: src/storage/db.rs:656
- Evidence:
```rust
                #[derive(Deserialize)]
                struct MemoryIdentity {
                    session_key: String,
                }
                let identity: MemoryIdentity = serde_json::from_str(&item.payload)
                    .map_err(|error| crate::OmonError::Database(error.to_string()))?;
```
- Why it matters: `stage_pending_write` inserts any arbitrary string into `pending_writes` without validating that it is valid JSON or contains `session_key`. If an invalid payload is staged, `scope.permits(&item)?` on line 670 returns `Err(OmonError::Database(...))`. Because `list_pending_writes_scoped` iterates over items and propagates errors with `?`, a single corrupted memory write causes `list_pending_writes_scoped`, `get_pending_write_scoped`, `reject_pending_write_scoped`, and `approve_pending_write_scoped` to fail unconditionally for every session in the gateway. The invalid write cannot even be rejected via the scoped API.
- Suggested fix: Validate JSON schema at insertion time in `stage_pending_write`. In `PendingWriteScope::permits`, treat deserialization failure as non-matching (`Ok(false)`) with a warning log instead of returning an error that aborts enumeration.

### [P1] State JSON read-modify-write race in mark_session_suspended overwrites concurrent updates
- Location: src/storage/db.rs:384
- Evidence:
```rust
    let row: Option<(String,)> =
        sqlx::query_as("SELECT state_json FROM sessions WHERE session_key = ?")
            .bind(session_key)
```
- Why it matters: `mark_session_suspended` fetches `state_json`, deserializes it into `SessionState`, toggles `suspended`, and writes the serialized JSON back to SQLite without an active transaction or atomic update mechanism. Concurrent operations (such as `persist_session_binding` on line 363, or turn completion handlers modifying session metadata) that update `state_json` during this window will be silently clobbered when the stale JSON blob is written back.
- Suggested fix: Update the field atomically in SQL using SQLite's native `json_set(COALESCE(NULLIF(state_json, ''), '{}'), '$.suspended', ?)` without a read-modify-write cycle in Rust.

### [P1] Repeated DDL execution on hot query path acquires exclusive SQLite schema lock
- Location: src/storage/db.rs:273
- Evidence:
```rust
pub async fn get_thread_owner(pool: &SqlitePool, thread_id: u64) -> Result<Option<u64>> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS discord_thread_owners (
```
- Why it matters: `get_thread_owner`, `record_thread_owner`, and `load_all_thread_owners` all execute `CREATE TABLE IF NOT EXISTS discord_thread_owners` on every invocation. `get_thread_owner` is called on inbound Discord messages within threads to determine bot routing. Executing DDL on this hot path forces SQLite to acquire an exclusive schema lock (`sqlite3_schema`) and invalidates prepared statement caches across connections. Furthermore, this table and `delivery_ledger_constituents` (line 203) are created ad-hoc in Rust rather than managed through sqlx migrations.
- Suggested fix: Remove `CREATE TABLE IF NOT EXISTS` from query functions; move `discord_thread_owners` and `delivery_ledger_constituents` into proper sqlx migration files.

### [P1] N+1 sequential query pattern during restart recovery under single-connection pool
- Location: src/storage/db.rs:526
- Evidence:
```rust
    let pending_keys = fetch_resume_pending_session_keys(pool).await?;
    let mut resumed_count = 0;
    for session_key in pending_keys {
```
- Why it matters: In `recover_resume_pending_sessions`, for every pending session returned by `fetch_resume_pending_session_keys`, the loop sequentially executes up to 5 individual queries: `is_session_suspended`, `clear_session_resume_pending`, `find_last_unfinished_user_turn`, a `delivery_ledger` lookup, and optionally `mark_delivered`. Because `Database::connect` limits the pool to `max_connections(1)`, this 5N+1 pattern sequentially starves the entire gateway of database access during startup recovery, introducing severe startup latency when recovering dozens or hundreds of sessions.
- Suggested fix: Batch-fetch session suspension status, unfinished turns, and delivery entries using single joined queries or batched transactions.

### [P1] FTS5 search cursor pagination breaks BM25 relevance sorting and fails for non-integer IDs
- Location: src/storage/message_search.rs:161
- Evidence:
```rust
             WHERE message_search_fts MATCH ?
               AND d.platform = ?
               AND d.channel_id = ?
               AND (? IS NULL OR CAST(d.message_id AS INTEGER) < CAST(? AS INTEGER))
             ORDER BY rank ASC, d.timestamp DESC
             LIMIT ?",
```
- Why it matters:
  1. Semantic pagination defect: The query orders by `rank ASC, d.timestamp DESC` (BM25 relevance score), but `before_message_id` filters by `CAST(d.message_id AS INTEGER) < CAST(? AS INTEGER)`. Applying a chronological ID threshold to a relevance-sorted result set causes subsequent pages to omit highly relevant newer messages while returning lower-relevance older messages.
  2. Non-snowflake failure: `CAST(d.message_id AS INTEGER)` assumes message IDs are numeric Discord snowflakes. On any platform where message IDs are non-numeric strings (e.g. Slack dot-separated timestamps `"1712345678.123456"`, or UUIDs), SQLite casts non-numeric strings to `0`. `0 < 0` evaluates to false, causing paginated queries on non-snowflake platforms to return empty results.
- Suggested fix: Align cursor pagination with the `ORDER BY` clause (e.g. keyset cursor on `(rank, timestamp, message_id)`), or if chronological pagination is required, order by `timestamp DESC` / `message_id DESC`.

### [P1] Missing index on foreign key constraint cron_jobs(session_key) causes full table scans on session delete
- Location: migrations/0001_initial.sql:48
- Evidence:
```sql
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (session_key) REFERENCES sessions(session_key) ON DELETE CASCADE
);
```
- Why it matters: With foreign keys enabled (`PRAGMA foreign_keys = true`), any `DELETE` on parent table `sessions(session_key)` requires SQLite to verify or cascade child table `cron_jobs`. Because `session_key` is not indexed on `cron_jobs` (unlike `messages`, `memories`, `delivery_ledger`, and `delivery_obligations`, which all index `session_key`), SQLite must execute a full table scan of `cron_jobs` whenever any session is deleted.
- Suggested fix: Add `CREATE INDEX IF NOT EXISTS idx_cron_jobs_session ON cron_jobs(session_key);`.

### [P1] Missing composite index for delivery obligation retention cleanup
- Location: migrations/0006_delivery_obligations.sql:16
- Evidence:
```sql
CREATE INDEX IF NOT EXISTS idx_delivery_obligations_state
    ON delivery_obligations(state, attempts, created_at);
```
- Why it matters: In `src/ledger/service.rs:551`, the obligation pruning worker repeatedly executes:
  `DELETE FROM delivery_obligations WHERE state IN ('delivered', 'abandoned') AND updated_at < ?`
  and
  `SELECT id FROM delivery_obligations WHERE state IN ('delivered', 'abandoned') ORDER BY updated_at ASC LIMIT ?`.
  However, `idx_delivery_obligations_state` is defined on `(state, attempts, created_at)`. It lacks `updated_at`. As a result, range filtering and sorting on `updated_at` cannot utilize the index, forcing temporary table allocation and file sorts on every pruning pass as the table grows.
- Suggested fix: Add `CREATE INDEX IF NOT EXISTS idx_delivery_obligations_prune ON delivery_obligations(state, updated_at);`.

### [P1] Missing index on delivery_ledger for session created_at ordering causes temporary B-tree sorts
- Location: migrations/0001_initial.sql:36
- Evidence:
```sql
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_delivery_ledger_event
    ON delivery_ledger(session_key, event_id);
```
- Why it matters: In `src/storage/db.rs:544, 553, 586`, startup recovery queries `delivery_ledger` with:
  `SELECT message_id FROM delivery_ledger WHERE session_key = ? ... ORDER BY created_at DESC LIMIT 1`.
  The only index with `session_key` is `idx_delivery_ledger_event(session_key, event_id)`. Because `created_at` is not in the index, SQLite cannot satisfy `ORDER BY created_at DESC LIMIT 1` via an index scan. It must load every delivery ledger record for that session and perform an in-memory or temporary B-tree sort.
- Suggested fix: Add `CREATE INDEX IF NOT EXISTS idx_delivery_ledger_session_created ON delivery_ledger(session_key, created_at DESC);`.

### [P1] Type mismatch and integer sign overflow for Discord snowflake channel IDs in dead_targets
- Location: migrations/0019_dead_targets.sql:3
- Evidence:
```sql
CREATE TABLE IF NOT EXISTS dead_targets (
    bot_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    status_code INTEGER NOT NULL,
```
- Why it matters: In `dead_targets`, `channel_id` is typed as `INTEGER NOT NULL`, whereas every other table in the schema (`sessions`, `messages`, `delivery_obligations`, `discord_bot_cursors`, `discord_channel_cursors`, `message_search_documents`) defines `channel_id` as `TEXT`. In Rust (`src/storage/db.rs:916`), `channel_id` is a `u64` bound using `channel_id as i64`. For channel IDs `>= 2^63`, the signed two's-complement cast stores negative integers in SQLite. While casting back with `as u64` in Rust reconstructs the bits, storing negative integers breaks cross-table joins, external SQL queries, and admin dashboard views.
- Suggested fix: Change `channel_id` column in `dead_targets` to `TEXT` to match the rest of the gateway schema.

### [P1] Destructive table drop without migration reversibility
- Location: migrations/0003_message_sequence.sql:20
- Evidence:
```sql
ORDER BY rowid;

DROP TABLE messages;
ALTER TABLE messages_v2 RENAME TO messages;
```
- Why it matters: Migration `0003_message_sequence.sql` executes an unconditional `DROP TABLE messages;` as part of rebuilding the table for monotonic sequence numbers. There are no accompanying down migrations anywhere in `migrations/`, making the migration strictly one-way. If an error occurs after dropping the table or during a rollback attempt, the original message table and foreign key relationships cannot be recovered automatically.
- Suggested fix: Provide corresponding `.down.sql` reversal scripts or run migrations within a reversible migration framework.

### [P1] SQLite single-connection pool bottleneck and lack of busy handler retry policy
- Location: src/storage/db.rs:182
- Evidence:
```rust
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .connect_with(options)
```
- Why it matters: The pool is hardcoded to `max_connections(1)` for all configurations. While `max_connections(1)` is required for in-memory databases without shared cache, in file databases operating in WAL mode, SQLite supports multiple concurrent readers alongside a single writer. Restricting the pool to 1 connection forces all read queries (FTS message searches, dashboard queries, session lookups, policy checks) to serialize behind writes. Furthermore, if any task holding a connection across an `.await` boundary indirectly attempts another database checkout, it will deadlock indefinitely on pool acquisition. Additionally, `options.busy_timeout(Duration::from_secs(5))` fails with `SQLITE_BUSY` after 5 seconds if an external process locks the database, without application-level retry or backoff.
- Suggested fix: Distinguish in-memory from file databases: configure separate read and write pools (or increase max_connections for readers in WAL mode with a dedicated writer connection), and handle `SQLITE_BUSY` with exponential backoff.

### [P1] Inconsistent datetime formats across tables prevent uniform ISO-8601 parsing and ordering
- Location: migrations/0001_initial.sql:9
- Evidence:
```sql
    user_id TEXT NOT NULL,
    state_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
```
- Why it matters: Migrations alternate between SQLite's `CURRENT_TIMESTAMP` (which produces `YYYY-MM-DD HH:MM:SS` without 'T', millisecond precision, or 'Z' timezone indicator) and `(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` (which produces ISO-8601 with fractional seconds and 'Z'). For instance, `sessions`, `cron_jobs`, and `memories` use `CURRENT_TIMESTAMP`, while `messages` and `delivery_obligations` use ISO-8601 with fractional seconds. In Rust, standard `chrono::DateTime<Utc>` deserialization fails if the format lacks timezone indicators. Furthermore, lexicographical string comparisons between the two formats can produce incorrect temporal ordering.
- Suggested fix: Standardize all datetime default expressions in migrations on `(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))`.

### [P2] Redundant duplicate indexes on primary and unique keys in migrations
- Location: migrations/0018_bot_cursors.sql:10
- Evidence:
```sql
CREATE TABLE IF NOT EXISTS discord_bot_cursors (
    bot_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    last_message_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (bot_id, channel_id)
);

CREATE INDEX IF NOT EXISTS idx_discord_bot_cursors_bot_channel
    ON discord_bot_cursors (bot_id, channel_id);
```
- Why it matters:
  1. `migrations/0018_bot_cursors.sql`: Table defines `PRIMARY KEY (bot_id, channel_id)`. SQLite automatically creates an internal unique index for composite primary keys. Line 10 then creates `idx_discord_bot_cursors_bot_channel` on the exact same columns `(bot_id, channel_id)`. This index is 100% redundant, doubling B-tree index maintenance overhead on every cursor update.
  2. `migrations/0026_cron_incidents.sql`: Line 8 specifies `UNIQUE(job_id, error_signature)`, creating a unique index. Line 11 creates `idx_cron_incidents_lookup` on `(job_id, error_signature)`, creating an identical duplicate index.
  3. `migrations/0027_cron_notepads.sql`: Primary key is `(profile, job_id, key)`. Line 11 creates `idx_cron_notepads_lookup` on `(profile, job_id)`. Because SQLite composite indexes support prefix lookups, queries on `(profile, job_id)` are already served by the primary key index.
- Suggested fix: Drop redundant indexes `idx_discord_bot_cursors_bot_channel`, `idx_cron_incidents_lookup`, and `idx_cron_notepads_lookup`.

### [P2] FTS5 query builder contains dead quote replacement and strips exact phrase capability
- Location: src/storage/message_search.rs:223
- Evidence:
```rust
    let terms = query
        .split_whitespace()
        .map(|term| term.trim_matches('"').replace('"', ""))
        .filter(|term| !term.is_empty())
```
- Why it matters:
  1. Dead code: Line 223 executes `replace('"', "")`, which strips every double quote character from `term`. On line 225, `term.replace('"', "\"\"")` is called to escape quotes for FTS5, but because all quotes were already removed two lines prior, this replacement is entirely dead code.
  2. Loss of phrase search: Because `split_whitespace()` splits on space and line 223 strips quotes, user-provided quoted phrases (e.g. `"fatal error"`) cannot be searched as exact phrases. Instead, they are disassembled into individual prefix wildcards (`"fatal"* AND "error"*`), matching documents where the words appear in different contexts or parts of the text.
- Suggested fix: Parse quoted phrases properly before splitting on whitespace, or escape quotes with `.replace('"', "\"\"")` without first stripping them with `.replace('"', "")`.

### [P2] Short 8-character hex ID in stage_pending_write has high collision probability
- Location: src/storage/db.rs:731
- Evidence:
```rust
pub async fn stage_pending_write(pool: &SqlitePool, kind: &str, payload: &str) -> Result<String> {
    let raw_uuid = Uuid::new_v4().to_string();
    let short_id = raw_uuid.replace('-', "")[..8].to_string();
    let now = Utc::now();
```
- Why it matters: `stage_pending_write` truncates a UUIDv4 to its first 8 hexadecimal characters (`[..8]`), yielding only 32 bits of entropy. In SQLite, `id` is the `PRIMARY KEY` of `pending_writes`. Due to the birthday paradox, a collision has a 1% probability after ~9,000 writes and a 50% probability after ~77,000 writes. Because abandoned writes are never pruned, collisions will eventually cause `INSERT INTO pending_writes` to fail with a `UNIQUE constraint failed: pending_writes.id` error, crashing the pending write operation.
- Suggested fix: Use full UUIDs (128 bits) or at least 16 hex characters (64 bits), or add retry-on-conflict logic.

### [P2] Hardcoded SQL string interpolation in upgrade_legacy_guild_session_keys
- Location: src/storage/db.rs:126
- Evidence:
```rust
            "delivery_obligations",
        ] {
            sqlx::query(&format!(
                "UPDATE {table} SET session_key = ? WHERE session_key = ?"
            ))
```
- Why it matters: `sqlx::query(&format!("UPDATE {table} ..."))` constructs SQL via string interpolation in Rust. Although `{table}` is currently constrained to a static array of table names, string-formatting SQL queries bypasses prepared statement caching and static query analysis, and presents a security hazard if the list is ever made dynamic or parameterized by caller input.
- Suggested fix: Execute static query strings for each table rather than formatting the table name into the SQL string.

## Strengths

- WAL Mode and Pragmas: Durable file databases are properly configured for concurrency with `journal_mode = WAL`, `synchronous = NORMAL`, and `wal_autocheckpoint = 1000`, along with `foreign_keys = true` and `busy_timeout = 5s`.
- Migration Safety and Monotonic Sequencing: Migration `0003_message_sequence.sql` replaced non-deterministic wall-clock causality with an explicit monotonic `AUTOINCREMENT` sequence column, ensuring strictly ordered message causality across turns.
- Legacy Guild Collision Quarantine: `upgrade_legacy_guild_session_keys` and migration `0028_legacy_guild_session_conflicts.sql` carefully detect key collisions, isolate conflicting lanes in `legacy_guild_session_conflicts`, and use SQLite `BEFORE INSERT` triggers (`RAISE(ABORT)`) to prevent shadow lane creation or accidental state clobbering.
- FTS5 Triggers with Contentless Sync: Migration `0015_message_search_fts.sql` sets up external content FTS5 tables with synchronized `AFTER INSERT`, `AFTER UPDATE`, and `AFTER DELETE` triggers, ensuring FTS index consistency with `message_search_documents`.

## Notes

- `Database::connect` uses `max_connections(1)`. While this avoids `SQLITE_BUSY` contention between writers within the single process, it severely bottlenecks concurrent readers that SQLite WAL mode is specifically designed to support. Moving to a dedicated writer connection pool plus a multi-connection reader pool would significantly increase throughput.
- `defer_foreign_keys = ON` is used during `upgrade_legacy_guild_session_keys`. This is necessary for in-place rekeying of parent `sessions` and child tables, but means foreign key consistency is only validated when the transaction commits.
