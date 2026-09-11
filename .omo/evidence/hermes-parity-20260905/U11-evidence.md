# U11 Bot-Preserving Persisted YOLO Restoration

## Contract & Scope
- Finding: Interrupted U11 completion (Hermes parity: bot-scoped persistent YOLO restoration, twin-bot DM isolation, reset/stop lifecycle, and fail-closed malformed state handling across commands and storage).
- Authorized write scope:
  - `src/discord/approval.rs`
  - `src/discord/commands.rs`
  - `src/main.rs`
  - `src/storage/db.rs::mark_session_suspended` error handling and narrow unit test
  - Evidence: `.omo/evidence/hermes-parity-20260905/U11-*`
- Untouched boundaries:
  - Preserved: Actor serialization (`src/multiplexer/actor.rs`, U26), daemon consent bridge (`src/agent/omo_daemon.rs`, U59), pairing/adapter (`src/discord/pairing.rs`, `src/discord/adapter.rs`), U07/U08 scoped pending write logic in `db.rs`, `.env`, production state, Discord API calls, git commit/push.
- Integration invariant: Single-connection pool (`max_connections(1)`). Queries avoid nested locks; successful database operations commit before in-memory cache changes.

## Pre-Production Defect Analysis & Intermediate Audit
1. **Omission of `bot_id` on Persisted Restoration**:
   - `load_persisted_yolo` selected columns `(platform, guild_id, channel_id, thread_id, user_id, state_json)` and reconstructed `SessionKey::new(...)` omitting `bot_id`.
   - Discord keys always contain length-prefixed `bot_id` in their canonical storage key (the SQLite primary key `session_key`). Reconstructing keys without `bot_id` caused restored sessions to have `bot_id: None`.
   - In twin-bot environments (same DM channel `dm-chan-99`, user `dm-user-1`, twin bots `84` and `42`), bot84 lost its persisted YOLO permission across restart, while an unauthenticated botless alias erroneously gained YOLO permissions.
2. **Silent Malformed State Overwriting in Commands & Storage**:
   - Baseline `yolo_toggle` called `serde_json::from_str(&state_json).unwrap_or_default()`, silently replacing corrupted state with defaults and erasing other fields (like `active_model`).
   - Baseline `stop_session` similarly called `serde_json::from_str(&state_json).unwrap_or_default()`.
   - Furthermore, `stop_session` delegates idle turn cancellation to `multiplexer.stop(key)`, which invokes `crate::storage::mark_session_suspended(pool, key, true)`. Baseline `mark_session_suspended` used `serde_json::from_str(&state_json).unwrap_or_default()` and `to_string().unwrap_or_else(|_| "{}".to_string())`, overwriting invalid JSON with normalized default state in SQLite before `stop_session` could inspect it.
3. **Honest Accounting of Intermediate Omission**:
   - During the first pass, when `stop_session` on malformed state returned `Ok(false)` due to `mark_session_suspended`'s fallback normalization, the test was temporarily adjusted by deleting `malformed_session` before calling `stop_session`.
   - As flagged by lead review, this masked the real defect path: `stop_session` -> `multiplexer.stop` -> `storage::mark_session_suspended`.
   - With explicit authorization to modify `mark_session_suspended` error handling, the root cause has now been addressed in `src/storage/db.rs` by propagating typed serialization errors (`crate::OmonError::Database(...)`), and the full `stop_session` malformed-state case and unchanged-raw-JSON assertion have been restored and verified GREEN.

## Pre-Production RED Evidence

The full regression test was registered against the baseline code to capture all constituent failures:
- Command: `cargo test --lib yolo_toggle_matches_effective_state_across_reset_and_restart`
- Exit code: 101 (`U11-red.exit`)
- Log: `U11-red.log`
- Observed constituent failures:
  ```
  U11 constituent failures: [
      "recreated guard does not restore persisted yolo for bot84",
      "recreated guard unexpectedly restored yolo for botless alias",
      "yolo_toggle unexpectedly succeeded on malformed session state",
      "yolo_toggle erased or rewrote malformed session state in DB",
      "stop_session unexpectedly succeeded on malformed session state",
      "load_persisted_yolo unexpectedly succeeded when malformed session state was present",
  ]
  ```
- Dedicated approval test `test_twin_bot_same_dm_yolo_restoration` also failed with `bot84 must be yolo` (exit code 101).

## Implementation Details

1. **Authoritative Canonical Storage Key Cache (`src/discord/approval.rs`)**:
   - Changed `SmartApprovalGuard.yolo_sessions` from `Arc<RwLock<HashSet<SessionKey>>>` to `Arc<RwLock<HashSet<String>>>`.
   - `load_persisted_yolo` queries `SELECT session_key, state_json FROM sessions`, caching the authoritative primary key string directly without incomplete key reconstruction.
   - `is_yolo`, `set_yolo`, and `clear_session` operate on `session.storage_key()`.
2. **Fail-Closed Malformed State in Commands & Approval (`src/discord/approval.rs`, `src/discord/commands.rs`)**:
   - In `load_persisted_yolo`, `serde_json::from_str(&state_json).map_err(|err| sqlx::Error::Decode(Box::new(err)))?` fails closed visibly when encountering corrupted state, preventing silent row skips.
   - In `yolo_toggle` and `stop_session`, replaced `unwrap_or_default()` with `serde_json::from_str(&state_json)?`.
3. **Fail-Closed Suspended State in Storage (`src/storage/db.rs`)**:
   - In `mark_session_suspended`, removed `unwrap_or_default()` and `unwrap_or_else(...)`. Both deserialization and serialization propagate typed errors via `crate::OmonError::Database(...)`.
   - If SQLite contains corrupted `state_json`, `mark_session_suspended` immediately fails without executing `UPDATE sessions SET state_json = ...`, preserving raw database contents.
4. **Persistence Sequencing & Pool Safety (`src/discord/commands.rs`)**:
   - `yolo_toggle`, `reset_session`, and `stop_session` commit SQLite queries before calling approval guard cache methods.
   - Prevents deadlocks on `max_connections(1)` and ensures failed persistence never alters runtime authorization.
5. **Startup Wiring (`src/main.rs`)**:
   - Startup sequence loads persisted allowlist followed by `approval_guard.load_persisted_yolo().await?`.
   - Sets `poise_data.approvals = approval_guard.clone()`.

## Verification & Proof

### 1. Target Regression Suite GREEN
- Command: `cargo test --lib discord::commands::tests::yolo_toggle_matches_effective_state_across_reset_and_restart -- --exact --nocapture`
- Exit Code: 0 (`U11-green.exit`)
- Log: `U11-green.log`
- Output: `U11 constituent failures: []`, `test result: ok. 1 passed; 0 failed`
- Verified: All 6 constituents pass, including twin-bot same-DM isolation, malformed load fail-closed, malformed toggle fail-closed with untouched raw JSON, malformed stop fail-closed with untouched raw JSON, reset isolation, and stop isolation.

### 2. Dedicated Twin-Bot Restoration Suite
- Command: `cargo test --lib discord::approval::tests::test_twin_bot_same_dm_yolo_restoration -- --exact --nocapture`
- Exit Code: 0
- Output: `test discord::approval::tests::test_twin_bot_same_dm_yolo_restoration ... ok`

### 3. Dedicated Storage Suspended Serialization Suite
- Command: `cargo test --lib storage::db::tests::mark_session_suspended_fails_on_malformed_state_without_overwriting -- --exact --nocapture`
- Exit Code: 0
- Output: `test storage::db::tests::mark_session_suspended_fails_on_malformed_state_without_overwriting ... ok`

### 4. Adjacent Suites
- Commands:
  - `cargo test --lib discord::commands::tests` -> 13 passed, 0 failed.
  - `cargo test --lib discord::approval::tests` -> 15 passed, 0 failed.
  - `cargo test --lib storage::db` -> 20 passed, 0 failed.
- Exit Code: 0 (`U11-adjacent.exit`)
- Log: `U11-adjacent.log`

### 5. Build, Format, and Diff Check
- Build: `cargo build` exit 0 (`U11-build.log`, `U11-build.exit`).
- Diagnostics: LSP diagnostics clean across `src/storage/db.rs`, `src/discord/approval.rs`, `src/discord/commands.rs`, and `src/main.rs` (0 errors, 0 warnings).
- Format: `rustfmt --edition 2021 --check src/storage/db.rs src/discord/approval.rs src/discord/commands.rs src/main.rs` exit 0.
- Diff check: `git --no-pager diff --check src/storage/db.rs src/discord/approval.rs src/discord/commands.rs src/main.rs` exit 0 (`U11-diff.log`, `U11-diff.exit`).
- Scoped patch: `.omo/evidence/hermes-parity-20260905/U11-current.patch`.
