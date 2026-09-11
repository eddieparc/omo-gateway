# U16 / S.F08: Durable Remote Conversation Binding

## 1. Summary

- **Unit**: U16 ("Durable remote conversation binding")
- **Findings & Defect Diagnosis (S.F08)**:
  - In unpatched code (`src/agent/omo_backend.rs`), a new remote thread ID returned by `thread/start` was saved only into in-memory metadata (`session.state.metadata["omo_thread_id"]`) and the backend's in-memory `thread_ids` map.
  - In `src/multiplexer/actor.rs`, the session actor executed turns using a clone (`turn_context`) and only copied `turn_context` back to `self.context` and flushed to SQLite upon successful completion (`if result.is_ok()`).
  - If turn 1 failed (e.g. app-server turn failure, connection error, or process crash), the durable remote thread binding was lost. When the gateway was reconstructed from the file SQLite database, the session record retained an unpopulated or stale `state_json`. A subsequent turn for the same session would issue a second `thread/start` instead of resuming `r1`, violating the permanent conversation binding invariant and creating fragmented remote threads.
  - Furthermore, if `thread/resume` returned an error such as `"no rollout found for thread id ..."` or `"thread not found"`, unpatched code caught the error, silently cleared `omo_thread_id`, set `start_replacement = true`, and fell through to `thread/start`. This silently discarded all canonical conversation history and replaced it with a blank remote thread containing only the latest user message.
  - In addition, thread binding was not persisted to SQLite before `turn/start` submission, meaning that persistence failures did not prevent side-effect submission.
- **Architectural Resolution**:
  - **Storage Durability (`src/storage/db.rs`)**:
    - Implemented `persist_session_binding(pool, session, omo_thread_id)` which guarantees that the session row exists in `sessions` and uses SQLite `json_set` to atomically checkpoint `$.metadata.omo_thread_id` into `state_json`.
  - **Pre-Submission Checkpoint & Fail-Closed Ingress (`src/agent/omo_backend.rs`)**:
    - In `resolve_thread_id`, immediately upon receiving a successful `thread/start` response, if `self.pool` is configured, `persist_session_binding` is committed to SQLite BEFORE any `turn/start` frame is written to the socket. If database persistence fails (e.g. database error, abort trigger, or lock), `resolve_thread_id` fails immediately, halting turn execution and preventing side-effect submission.
    - When `thread/resume` receives a missing rollout (`"no rollout found"` or `"thread not found"`), it returns an explicit `Err(OmonError::Llm("continuity_error: ..."))` rather than silently issuing a replacement `thread/start`.
  - **Stale Overwrite Protection in Session Actor (`src/multiplexer/actor.rs`)**:
    - In `SessionActor::run`, immediately after dropping the run future, `self.context` unconditionally adopts any acknowledged `omo_thread_id` from `turn_context.state.metadata`. This guarantees `self.context` never holds a stale thread binding, regardless of whether turn execution completed with success, error, stop, or shutdown.
    - In `SessionActor::flush()`, the SQL update uses a conditional `CASE` expression: if `state_json` in the database already has `$.metadata.omo_thread_id` and the in-memory actor state lacks it, SQLite preserves the database's existing `omo_thread_id`. This mathematically prevents a stale actor flush from ever clearing a persisted remote binding.
  - **Verification Suite (`tests/test_omo_backend.rs`)**:
    - Added `failed_first_turn_keeps_durable_binding`: executes turn 1 on Multiplexer 1 with a real SQLite file database; turn 1 starts `r1` and fails; Multiplexer 1 is shut down and dropped; Multiplexer 2 is reconstructed from the SQLite file database; turn 2 executes and verifies that wire transcript sends `thread/resume` with `r1` and exactly 1 `thread/start` occurred across the entire lifecycle.
    - Added `missing_rollout_fails_with_continuity_error`: verifies that missing rollout on resume returns an explicit `continuity_error` error and sends zero replacement `thread/start` frames.
    - Added `persistence_failure_prevents_turn_submission`: installs a SQLite trigger aborting session binding persistence; verifies that `turn/start` is never sent over the wire.
    - Updated `test_omo_backend_replaces_stale_cached_thread` to assert explicit `continuity_error` rather than expecting silent replacement.

---

## 2. Captured RED Before Production Edits

### Exact Verification Command
- **Command**: `cargo test --test test_omo_backend failed_first_turn_keeps_durable_binding -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-red.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.43s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test

  thread 'failed_first_turn_keeps_durable_binding' (772043) panicked at tests/test_omo_backend.rs:4046:5:
  assertion `left == right` failed: durable binding must not issue a second thread/start after first turn failure
    left: 2
   right: 1
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test failed_first_turn_keeps_durable_binding ... FAILED

  failures:

  failures:
      failed_first_turn_keeps_durable_binding

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.02s

  error: test failed, to rerun pass `--test test_omo_backend`
  ```

### Companion RED: Missing-Rollout Subcase
- **Command**: `cargo test --test test_omo_backend missing_rollout_fails_with_continuity_error -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-red-missing-rollout.log`
- **Captured Output**:
  ```text
  thread 'missing_rollout_fails_with_continuity_error' (771193) panicked at tests/test_omo_backend.rs:4204:5:
  assertion `left == right` failed: missing rollout must not silently issue thread/start replacement
    left: 1
   right: 0
  ```

### Companion RED: Persistence Failure Subcase
- **Command**: `cargo test --test test_omo_backend persistence_failure_prevents_turn_submission -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-red-persistence-failure.log`
- **Captured Output**:
  ```text
  thread 'persistence_failure_prevents_turn_submission' (771588) panicked at tests/test_omo_backend.rs:4327:5:
  assertion `left == right` failed: failure to persist thread binding must prevent turn submission (turn/start must not be sent)
    left: 1
   right: 0
  ```

---

## 3. Implementation Details

1. **Storage Session Binding Helper in `src/storage/db.rs`**:
   ```rust
   pub async fn persist_session_binding(
       pool: &SqlitePool,
       session: &crate::SessionContext,
       omo_thread_id: &str,
   ) -> Result<()> {
       sqlx::query(
           "INSERT INTO sessions (
               session_key, platform, guild_id, channel_id, thread_id, user_id,
               state_json, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, '{}', (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))
            ON CONFLICT(session_key) DO NOTHING",
       )
       .bind(session.key.storage_key())
       .bind(&session.key.platform)
       .bind(&session.key.guild_id)
       .bind(&session.key.channel_id)
       .bind(&session.key.thread_id)
       .bind(&session.key.user_id)
       .execute(pool)
       .await?;

       sqlx::query(
           "UPDATE sessions
            SET state_json = json_set(COALESCE(NULLIF(state_json, ''), '{}'), '$.metadata.omo_thread_id', ?),
                updated_at = (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            WHERE session_key = ?",
       )
       .bind(omo_thread_id)
       .bind(session.key.storage_key())
       .execute(pool)
       .await?;

       Ok(())
   }
   ```

2. **Durable Binding Checkpoint & Missing Rollout Invariant in `src/agent/omo_backend.rs`**:
   ```rust
   // In resolve_thread_id: missing rollout returns explicit continuity_error
   if message.contains("no rollout found") || message.contains("thread not found") {
       session.state.metadata.remove("omo_thread_id");
       self.thread_ids.lock().remove(&storage_key);
       return Err(OmonError::Llm(format!(
           "continuity_error: remote thread rollout missing for {id}: {message}"
       )));
   }

   // In resolve_thread_id: persist new thread binding before returning for turn submission
   if let Some(id) = val.pointer("/result/thread/id").and_then(Value::as_str) {
       let id_str = id.to_string();
       if !is_cron {
           session
               .state
               .metadata
               .insert("omo_thread_id".into(), json!(id_str));
           self.thread_ids.lock().insert(storage_key, id_str.clone());
           if let Some(pool) = &self.pool {
               persist_session_binding(pool, session, &id_str).await?;
           }
       }
       return Ok(id_str);
   }
   ```

3. **Actor Context Synchronization & Stale Overwrite Guard in `src/multiplexer/actor.rs`**:
   ```rust
   drop(run);

   // Durable remote binding must never be lost on failure or overwritten by a stale actor copy.
   if let Some(thread_id) = turn_context.state.metadata.get("omo_thread_id") {
       self.context
           .state
           .metadata
           .insert("omo_thread_id".into(), thread_id.clone());
   }

   match outcome {
       TurnOutcome::Completed(result) => {
           if result.is_ok() {
               self.context = turn_context;
               let _ = crate::storage::clear_session_resume_pending(
                   &self.pool,
                   &self.context.key.storage_key(),
               )
               .await;
               if let Err(error) = self.flush_if_dirty().await {
                   tracing::error!(session = %self.context.key, %error, "failed to flush session actor on turn completion");
               }
           } else {
               self.dirty = false;
           }
           ...
       }
   }

   // In flush: never overwrite a persisted thread binding with a stale actor copy
   sqlx::query(
       "UPDATE sessions
        SET state_json = CASE
            WHEN json_extract(state_json, '$.metadata.omo_thread_id') IS NOT NULL
                 AND json_extract(?, '$.metadata.omo_thread_id') IS NULL
            THEN json_set(?, '$.metadata.omo_thread_id', json_extract(state_json, '$.metadata.omo_thread_id'))
            ELSE ?
        END,
        updated_at = ?
        WHERE session_key = ?",
   )
   ```

---

## 4. Captured GREEN After Production Edits

### Exact Verification Command
- **Command**: `cargo test --test test_omo_backend failed_first_turn_keeps_durable_binding -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-green.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 16.63s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test
  test failed_first_turn_keeps_durable_binding ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.03s
  ```

### Companion GREEN: Missing Rollout Continuity Error
- **Command**: `cargo test --test test_omo_backend missing_rollout_fails_with_continuity_error -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-green-missing-rollout.log`
- **Captured Output**:
  ```text
  running 1 test
  test missing_rollout_fails_with_continuity_error ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.00s
  ```

### Companion GREEN: Persistence Failure Prevents Turn Submission
- **Command**: `cargo test --test test_omo_backend persistence_failure_prevents_turn_submission -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-green-persistence-failure.log`
- **Captured Output**:
  ```text
  running 1 test
  test persistence_failure_prevents_turn_submission ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.01s
  ```

### Database Unit Target
- **Command**: `cargo test --lib test_persist_session_binding -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-db-target.log`
- **Captured Output**:
  ```text
  running 1 test
  test storage::db::tests::test_persist_session_binding_persists_durable_binding ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 353 filtered out; finished in 0.01s
  ```

### Multiplexer Target
- **Command**: `cargo test --test test_multiplexer -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-actor-target.log`
- **Captured Output**:
  ```text
  running 11 tests
  test route_reports_actor_startup_failure_instead_of_acknowledging_a_lost_event ... ok
  test delivery_ledger_deduplicates_concurrent_claims_and_records_latency ... ok
  test gc_does_not_evict_an_actor_with_an_active_turn ... ok
  test stop_cancels_active_turn_and_clears_queued_events ... ok
  test dropping_multiplexer_releases_actor_cycle_and_flushes_dirty_state ... ok
  test scale_to_zero_evicts_and_flushes_idle_sessions ... ok
  test stop_immediately_cancels_the_active_turn ... ok
  test events_arriving_during_running_turn_are_queued_and_processed_in_order ... ok
  test routes_multiple_sessions_in_parallel ... ok
  test handles_events_sequentially_within_one_session ... ok
  test transcript_level_inbound_dedup_skips_duplicate_platform_message_id ... ok

  test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
  ```

### Full Backend Target Suite
- **Command**: `cargo test --test test_omo_backend`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U16-suite.log`
- **Result**: 34 passed, 0 failed, 0 ignored, finished in 30.63s.

---

## 5. Verification & Environmental Hygiene

- **LSP Diagnostics**:
  - LSP daemon socket `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock` was unreachable during invocation; compiler checks (`cargo check`, `cargo build`, `cargo clippy --lib`) were used with 0 errors and 0 warnings on touched files.
- **Cargo Build**:
  - `cargo build` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U16-build.log`).
- **Formatting**:
  - `rustfmt --check --edition 2021 src/agent/omo_backend.rs src/multiplexer/actor.rs src/storage/db.rs tests/test_omo_backend.rs` exited with code `0`.
- **Diff Check**:
  - `git diff --check src/agent/omo_backend.rs src/multiplexer/actor.rs src/storage/db.rs tests/test_omo_backend.rs` exited with code `0`.
- **Predecessor Invariants Preserved**:
  - U12 Correlation: Active ACK correlation and frame filtering intact across test suite.
  - U13 Cancellation: Remote interruption and typing indicator release preserved across terminal paths.
  - U14 Deadline: 300s absolute interactive budget preserved.
  - U15 No Resubmission: Post-submission disconnect fails fast without resubmitting turns.
- **Isolation & Cleanup**:
  - All test database paths were isolated via `tempfile::tempdir()`.
  - All loopback server listeners and tasks were cleanly aborted and awaited.
  - No `.env`, real Discord, production database, or unowned files were accessed or modified.
