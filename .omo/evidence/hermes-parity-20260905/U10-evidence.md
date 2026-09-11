# U10 Pairing Notification Throttling and Lockout Persistence

## Contract & Scope
- Finding: `AP.AP18` (Hermes parity: pairing rate limiting, lockout persistence, and unauthorized-DM admission default).
- Authorized write scope:
  - `src/discord/pairing.rs`
  - `src/discord/adapter.rs`
  - `migrations/0024_pairing_state.sql` (schema reservation: migrations 0017–0023 reserved by shared-design; existing directory ended at 0016; applied `0011_pairing_codes.sql` preserved unmodified)
  - Evidence: `.omo/evidence/hermes-parity-20260905/U10-*`
- Untouched boundaries:
  - Prohibited: `src/discord/commands.rs`, `src/discord/approval.rs`, `src/main.rs` (owned by U11 writer), backend/cron/dashboard, `.env`, production state, real Discord API calls, dependency changes, git commit/push.
  - Preserved operator boundary: `/pair` command in `src/discord/commands.rs` independently verifies configured operator `allowed_users` / `allowed_roles` / `allow_all_users` without paired bypass; paired users are never promoted to operators.
  - Preserved API outcome compatibility: `approve_code(&self, input_code: &str, operator_id: u64) -> Result<PairingOutcome>` preserves all enum variants (`Success { user_id }`, `InvalidCode`, `Expired`, `LockedOut`) and signature.
- Integration invariant: `Database::connect` configures `max_connections(1)`. Never hold a SQLx transaction while calling a helper that re-acquires the same pool; all atomic queries execute directly or pass `&mut transaction` internally, committing before cache or callback updates to prevent single-connection deadlocks.

## Pre-Production Baseline Defects
1. **Unthrottled DM notifications**: `request_pairing_code` reused an existing active code within the rate-limit window, yet every incoming DM from an unauthorized user triggered an immediate Discord message send (`channel_id.say(...)`). Repeated messages at the same timestamp resulted in repeated outbound notifications.
2. **Lockout reset vulnerability**: Failed attempts counter was stored directly on replaceable rows in `pairing_codes`. When `attempts >= 5` was reached, the next DM ingress or code request fell through `attempts < MAX_FAILED_ATTEMPTS`, deleted the locked row, and generated a fresh code with `attempts = 0`. An attacker could reset lockout indefinitely by sending another DM.
3. **Inverted unauthorized-DM admission policy**: Baseline `adapter.rs` required `!data.allowed_users.is_empty() || !data.allowed_roles.is_empty()` to prompt pairing, which was the exact inverse of Hermes specification. Configured allowlist bots spammed unsolicited pairing prompts to unauthorized users, while bots running with default open pairing admission ignored unauthorized DMs completely.
4. **Unbounded pending codes & missing global expired cleanup**: Expired codes were only deleted on single-user access or exact approval hits, and no pending code cap was enforced, enabling potential table bloat.

## Historical Preproduction RED Chronology

Pre-production tests were registered and executed against the baseline code before applying fixes:

### 1. Combined Scenario RED (`pairing_throttles_notifications_and_preserves_lockout`)
- Command: `cargo test --lib pairing_throttles_notifications_and_preserves_lockout`
- Command: `cargo test --lib discord::pairing::tests::pairing_throttles_notifications_and_preserves_lockout -- --exact --nocapture`
- Exit Code: 101 (`U10-red.exit`, `U10-red-exact.exit`)
- Result: 1 run, 0 passed, 1 failed (`U10-red.log`, `U10-red-exact.log`)
- Failure:
  ```
  thread 'discord::pairing::tests::pairing_throttles_notifications_and_preserves_lockout' panicked at src/discord/pairing.rs:401:9:
  assertion `left == right` failed: repeated rejected DM at same timestamp must send only 1 notification, got 2
    left: 2
   right: 1
  ```
- Analysis: In the baseline, every DM arrival triggered a notification delivery decision because code issuance was conflated with notification dispatch; two rejected DMs at the same injected timestamp resulted in 2 sends instead of 1.

### 2. Constituent 1 RED: Repeated Same-Clock DM Notification
- Command: `cargo test --lib discord::pairing::tests::pairing_throttles_notifications_on_repeated_rejected_dm -- --exact --nocapture`
- Exit Code: 101 (`U10-red-throttle.exit`)
- Result: 1 run, 0 passed, 1 failed (`U10-red-throttle.log`)
- Failure:
  ```
  thread 'discord::pairing::tests::pairing_throttles_notifications_on_repeated_rejected_dm' panicked at src/discord/pairing.rs:346:9:
  assertion `left == right` failed: repeated rejected DM at same timestamp must send only 1 notification, got 2
    left: 2
   right: 1
  ```

### 3. Constituent 2 RED: Lockout Reset by Fresh Request
- Command: `cargo test --lib discord::pairing::tests::pairing_preserves_lockout_across_fresh_requests -- --exact --nocapture`
- Exit Code: 101 (`U10-red-lockout.exit`)
- Result: 1 run, 0 passed, 1 failed (`U10-red-lockout.log`)
- Failure:
  ```
  thread 'discord::pairing::tests::pairing_preserves_lockout_across_fresh_requests' panicked at src/discord/pairing.rs:379:9:
  assertion `left != right` failed: five invalid operator approvals then fresh DM must remain locked until deadline; baseline reset lockout with fresh code attempts=0
    left: 0
   right: 0
  ```
- Analysis: After 5 invalid operator approval attempts, calling `request_pairing_code` deleted the row and issued a new code with `attempts = 0`, completely erasing the lockout.

---

## Production Implementation

### 1. Schema Migration (`migrations/0024_pairing_state.sql`)
- Added `pairing_platform_lockout (platform_key TEXT PRIMARY KEY, locked_until TEXT NOT NULL, failed_attempts INTEGER NOT NULL DEFAULT 0)` for persistent platform lockout deadlines that fresh users and codes cannot reset.
- Added `pairing_notifications (user_id TEXT PRIMARY KEY, last_notified_at TEXT NOT NULL)` to separate code generation from notification throttle state.
- Preserved `0011_pairing_codes.sql` unmodified; numbered 0024 to respect shared-design reservation of 0017–0023.

### 2. Pairing Store (`src/discord/pairing.rs`)
- **Constants**:
  - `LOCKOUT_DURATION_SECONDS: i64 = 900` (15-minute platform lockout deadline).
  - `MAX_FAILED_ATTEMPTS: i64 = 5`.
  - `RATE_LIMIT_SECONDS: i64 = 600` (10-minute notification rate limit window).
  - `MAX_PENDING_CODES: usize = 100` (bounded pending code count).
  - `DEFAULT_PLATFORM_KEY: &str = "discord"`.
- **Platform Lockout Deadline**:
  - `is_platform_locked_at(&self, now: DateTime<Utc>) -> Result<bool>`: returns `true` if `failed_attempts >= 5` and `now < locked_until`.
  - `record_failed_attempt_at(&self, now: DateTime<Utc>)`: atomically increments failed attempts. On attempt 5, sets `locked_until = now + LOCKOUT_DURATION_SECONDS`. If previous lockout expired, resets attempt counter to 1.
  - Successful approval (`PairingOutcome::Success`) resets `failed_attempts = 0` and clears the lockout deadline.
- **Lockout Preservation**:
  - `request_pairing_code_at(&self, user_id: u64, now: DateTime<Utc>)`: checks `is_platform_locked_at(now)`. If locked, returns `Err(OmonError::Approval(...))` without deleting or resetting code rows.
  - If a specific code has `attempts >= 5`, it cannot be reused while locked out. Only after `now >= locked_until` can a new code be issued.
- **Bounded Pending Count & Expiry Cleanup**:
  - `cleanup_expired_codes_at(&self, now: DateTime<Utc>)`: deletes expired codes across all users globally.
  - Enforces `MAX_PENDING_CODES`: if active pending count reaches capacity, evicts the oldest pending codes FIFO before inserting a new code.
- **Separated Notification Throttling**:
  - `check_and_record_notification_at(&self, user_id: u64, now: DateTime<Utc>) -> Result<Option<String>>`:
    - Refuses notification if platform is locked out (`Ok(None)`).
    - Queries `pairing_notifications.last_notified_at`. If `(now - last_notified_at) < RATE_LIMIT_SECONDS`, returns `Ok(None)` (throttled).
    - If allowed, requests/reuses code, updates `last_notified_at = now`, and returns `Ok(Some(code))`.

### 3. Unauthorized DM Admission Boundary (`src/discord/adapter.rs`)
- **Explicit Unauthorized-DM Admission Policy**:
  - `pub fn should_prompt_unauthorized_dm(is_dm: bool, is_bot: bool, is_paired: bool, allow_all_users: bool, allowed_users: &[u64], allowed_roles: &[u64]) -> bool`:
    - Returns `false` for non-DMs, bot authors, paired users, and `allow_all_users = true`.
    - Returns `false` if an explicit allowlist exists (`!allowed_users.is_empty() || !allowed_roles.is_empty()`), suppressing unsolicited codes.
    - Returns `true` when NO explicit allowlist exists (`allowed_users.is_empty() && allowed_roles.is_empty()`), entering pairing mode.
- **Separated Delivery Evaluation**:
  - `pub async fn decide_unauthorized_dm(...) -> Option<String>`: gates on `should_prompt_unauthorized_dm`, then invokes `check_and_record_notification_at`. Returns `Some(code)` only when authorized to notify.
  - In `handle_event`, message ingress routes through `decide_unauthorized_dm` with zero wall sleeps and fully deterministic event flow.

---

## Exact GREEN Proof and Verification

### 1. Primary Targeted Regression
Command:
`cargo test --lib pairing_throttles_notifications_and_preserves_lockout`
- Exit code: 0 (`U10-green-final.exit`)
- Result: 1 passed, 0 failed (`U10-green-final.log`)
- Output:
  ```
  running 1 test
  test discord::pairing::tests::pairing_throttles_notifications_and_preserves_lockout ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 343 filtered out; finished in 0.01s
  ```
- Verified:
  1. Part 1: First DM at $T_0$ returns code (send 1). Repeated DM at same $T_0$ is throttled (0 sends). Send count at $T_0$ is exactly 1. Advancing clock past 600s permits next notification.
  2. Part 2: 5 invalid operator approvals establish platform lockout until $T_0 + 900$s. 6th approval and approval of active code return `LockedOut`. Fresh DM while locked out yields `None`; fresh code request fails with error; database `attempts` counter remains 5 (not reset to 0). Advancing clock past 900s allows lockout to expire and fresh DM to succeed.

### 2. Constituent Tests
- `cargo test --lib discord::pairing::tests::pairing_throttles_notifications_on_repeated_rejected_dm -- --exact --nocapture` (1 passed, exit 0)
- `cargo test --lib discord::pairing::tests::pairing_preserves_lockout_across_fresh_requests -- --exact --nocapture` (1 passed, exit 0)
- `cargo test --lib discord::pairing::tests::pairing_bounded_pending_count_and_expiry_cleanup -- --exact --nocapture` (1 passed, exit 0; verifies global expired cleanup and 100-code pending bound with FIFO eviction)
- `cargo test --lib discord::adapter::tests::test_unauthorized_dm_allowlist_default_matrix -- --exact --nocapture` (1 passed, exit 0; verifies full truth table: no allowlist -> pair; explicit allowlist -> ignore; bots/guilds/paired/allow_all -> ignore)
- `cargo test --lib discord::adapter::tests::test_decide_unauthorized_dm_throttling -- --exact --nocapture` (1 passed, exit 0; verifies delivery decision recording with injected clock)

### 3. Full Adjacent Suites
- `cargo test --lib discord::pairing`: 8 passed, 0 failed, exit 0.
- `cargo test --lib discord::adapter`: 45 passed, 0 failed, exit 0 (`U10-adjacent.log`, `U10-adjacent.exit`).

### 4. Diagnostics, Build, and Scoped Formatting
- **LSP Diagnostics**: checked via language server on `src/discord/pairing.rs` and `src/discord/adapter.rs`; 0 errors, 0 warnings.
- **Cargo Build**: `cargo build` exit 0 (`U10-build.log`, `U10-build.exit`).
- **Rustfmt (edition 2021)**: `rustfmt --edition 2021 --check src/discord/pairing.rs src/discord/adapter.rs` clean (exit 0).
- **Diff check**: `git --no-pager diff --check src/discord/pairing.rs src/discord/adapter.rs migrations/0024_pairing_state.sql` clean (exit 0, `U10-diff.log`, `U10-diff.exit`).
- **Clean execution**: Real SQLite in-memory database with injected virtual clock; no wall sleeps, polling, or timing-luck loops. No real Discord operations or unowned file edits.

---

## U10-Atomic: Concurrency & Database Error Hardening

### 1. Concurrency Defect Analysis
1. **Concurrent Notification Race**:
   - `check_and_record_notification_at` performed separate `SELECT last_notified_at` followed by `INSERT INTO pairing_notifications`.
   - Concurrent calls for the same user simultaneously found no record (or elapsed time >= 600s), both requested pairing codes, and both returned `Some(code)` decisions, causing duplicate outbound Discord prompts.
2. **Concurrent Lockout Increment & Insert Conflict**:
   - `record_failed_attempt_at` used read-then-insert/update logic on `pairing_platform_lockout`. Concurrent attempts crashed on SQLite `UNIQUE constraint failed: pairing_platform_lockout.platform_key` when uninitialized, or suffered lost increments where 5 concurrent failures failed to trigger platform lockout.
3. **Double Issuance & Pending Capacity TOCTOU**:
   - Concurrent `request_pairing_code_at` calls for the same user concurrently generated and inserted separate pairing codes, leaving multiple active codes per user.
   - Pending count check was performed prior to deletion/insertion without a lock, allowing concurrent requests at `MAX_PENDING_CODES` to breach capacity.
4. **Silent DB Error Masking**:
   - Expired cleanup, code deletions, and attempt increments used `let _ =`, discarding SQLite errors.
   - `active_count` used `.unwrap_or(0)`, masking DB failures.
   - `check_and_record_notification_at` caught errors from `request_pairing_code_at` and returned `Ok(None)`, masking database failures as benign throttled success.
5. **Pool Re-entrancy Invariant**:
   - Production SQLite pool operates with `max_connections(1)`. Transactions must execute exclusively on `&mut transaction` without re-acquiring `self.pool`.

### 2. Deterministic RED Captures (Explicit Barriers)

Before hardening the transition logic, deterministic concurrent tests were registered using `tokio::sync::Barrier`:

1. **Concurrent Same-Clock Notifications**:
   - Test: `discord::pairing::tests::test_atomic_concurrent_notifications_single_send_decision`
   - Command: `cargo test --lib test_atomic_concurrent_notifications_single_send_decision -- --nocapture`
   - Exit Code: 101 (`U10-atomic-red-notification.exit`)
   - Failure:
     ```
     thread 'discord::pairing::tests::test_atomic_concurrent_notifications_single_send_decision' panicked at src/discord/pairing.rs:805:9:
     assertion `left == right` failed: concurrent same-time notifications must yield exactly 1 send decision, got 2
       left: 2
      right: 1
     ```
   - Log: `U10-atomic-red-notification.log`

2. **Concurrent Failed Attempts Lockout**:
   - Test: `discord::pairing::tests::test_atomic_concurrent_failed_attempts_lockout`
   - Command: `cargo test --lib test_atomic_concurrent_failed_attempts_lockout -- --nocapture`
   - Exit Code: 101 (`U10-atomic-red-lockout.exit`)
   - Failure:
     ```
     thread 'discord::pairing::tests::test_atomic_concurrent_failed_attempts_lockout' panicked at src/discord/pairing.rs:831:30:
     called `Result::unwrap()` on an `Err` value: Database("error returned from database: (code: 1555) UNIQUE constraint failed: pairing_platform_lockout.platform_key")
     ```
   - Log: `U10-atomic-red-lockout.log`

3. **Concurrent Issuance and Capacity**:
   - Test: `discord::pairing::tests::test_atomic_concurrent_issuance_and_capacity`
   - Command: `cargo test --lib test_atomic_concurrent_issuance_and_capacity -- --nocapture`
   - Exit Code: 101 (`U10-atomic-red-capacity.exit`)
   - Failure:
     ```
     thread 'discord::pairing::tests::test_atomic_concurrent_issuance_and_capacity' panicked at src/discord/pairing.rs:881:9:
     assertion `left == right` failed: concurrent requests for same user must yield identical active code
       left: "ZTGC-QGQ7"
      right: "RDG9-YGN2"
     ```
   - Log: `U10-atomic-red-capacity.log`

4. **Combined Pre-Hardening RED Suite**:
   - Command: `cargo test --lib test_atomic_concurrent -- --nocapture`
   - Exit Code: 101 (`U10-atomic-red.exit`, `U10-atomic-red.log`)

### 3. Hardening Implementation

1. **Atomic Conditional Platform Lockout (`record_failed_attempt_tx`)**:
   - Replaced read-then-write with an atomic conditional UPSERT:
     ```sql
     INSERT INTO pairing_platform_lockout (platform_key, locked_until, failed_attempts)
     VALUES (?, ?, 1)
     ON CONFLICT(platform_key) DO UPDATE SET
         failed_attempts = CASE
             WHEN strftime('%s', ?) >= strftime('%s', pairing_platform_lockout.locked_until) AND pairing_platform_lockout.failed_attempts >= ? THEN 1
             ELSE pairing_platform_lockout.failed_attempts + 1
         END,
         locked_until = CASE
             WHEN strftime('%s', ?) >= strftime('%s', pairing_platform_lockout.locked_until) AND pairing_platform_lockout.failed_attempts >= ? THEN ?
             WHEN pairing_platform_lockout.failed_attempts + 1 >= ? THEN ?
             ELSE pairing_platform_lockout.locked_until
         END
     WHERE platform_key = ?
     ```
   - Guarantees 0 lost increments and 0 unique constraint collisions across concurrent failures.

2. **Atomic Notification Slot Reservation**:
   - Enforced atomic single-send semantics with conditional UPSERT:
     ```sql
     INSERT INTO pairing_notifications (user_id, last_notified_at)
     VALUES (?, ?)
     ON CONFLICT(user_id) DO UPDATE SET last_notified_at = excluded.last_notified_at
     WHERE (strftime('%s', ?) - strftime('%s', pairing_notifications.last_notified_at)) >= ?
     ```
   - Under concurrency at identical timestamp, exactly 1 caller gets `rows_affected == 1` and issues a code; competing callers get `rows_affected == 0` and return `Ok(None)`.

3. **Transaction-Scoped Linearizability & Single Connection Safety**:
   - `request_pairing_code_tx`, `is_platform_locked_tx`, and `record_failed_attempt_tx` operate strictly on `&mut Transaction<'_, Sqlite>`.
   - Single-connection pool (`max_connections(1)`) cannot deadlock because transactions never re-acquire from the pool.

4. **Atomic Code Claiming & Post-Commit Cache Updates**:
   - `approve_code_at` atomically claims codes via `DELETE FROM pairing_codes WHERE code = ?` within transaction. If competing approvals race, `rows_affected == 0` returns `PairingOutcome::InvalidCode`.
   - Cache mutation (`self.paired_cache.write().await.insert(...)`) executes strictly after `tx.commit().await?`.

5. **Attempt vs Confirmed Delivery Timestamp Clarification**:
   - `check_and_record_notification_at` records the notification *attempt reservation* timestamp prior to outbound delivery to serialize decision callers.
   - `record_confirmed_delivery_at` records the *confirmed delivery* timestamp upon successful transport (`new_message.channel_id.say(...)`).

6. **Full DB Error Propagation**:
   - Removed all `let _ =` queries and `.unwrap_or(0)`.
   - `check_and_record_notification_at` propagates `request_pairing_code_tx` errors with `?` instead of returning `Ok(None)`.
   - `decide_unauthorized_dm` logs database errors with `tracing::error!` rather than treating them as normal throttle conditions.

### 4. GREEN Verification & Evidence Artifacts

1. **Hardened Atomic Concurrent Suite**:
   - Command: `cargo test --lib test_atomic_concurrent -- --nocapture`
   - Exit Code: 0 (`U10-atomic-green.exit`)
   - Log: `U10-atomic-green.log`
   - Results: 4 passed; 0 failed:
     - `test_atomic_concurrent_notifications_single_send_decision ... ok`
     - `test_atomic_concurrent_failed_attempts_lockout ... ok`
     - `test_atomic_concurrent_approval_single_claim ... ok`
     - `test_atomic_concurrent_issuance_and_capacity ... ok`

2. **Adjacent Suites Verification**:
   - Command: `cargo test --lib discord::pairing && cargo test --lib discord::adapter`
   - Exit Code: 0 (`U10-atomic-adjacent.exit`)
   - Log: `U10-atomic-adjacent.log`
   - Pairing suite: 12 passed, 0 failed.
   - Adapter suite: 45 passed, 0 failed.

3. **Build, LSP, and Scoped Formatting**:
   - LSP diagnostics: 0 errors, 0 warnings across all touched files.
   - Build: `cargo build` exit 0 (`U10-atomic-build.exit`, `U10-atomic-build.log`).
   - Format: `rustfmt --edition 2021 --check src/discord/pairing.rs src/discord/adapter.rs` exit 0.
   - Diff check: `git --no-pager diff --check src/discord/pairing.rs src/discord/adapter.rs` clean (`U10-atomic-diff.exit`, `U10-atomic-diff.log`).

