# U18 / S.F15: Fail Before Undurable Side Effects

## 1. Summary

- **Unit**: U18 ("Fail before undurable side effects")
- **Findings & Defect Diagnosis (S.F15)**:
  - In `src/multiplexer/actor.rs:127-129` (unpatched lines 240-244), when `self.persist_inbound(&event).await` failed (e.g. SQLite database locked, read-only, disk full, or trigger abort on `messages` table), the actor merely logged a tracing error and proceeded directly to start the runner:
    ```rust
    if let Err(error) = self.persist_inbound(&event).await {
        tracing::error!(session = %self.context.key, %error, "failed to persist inbound event");
    }
    ```
    This dispatched un-persisted inbound messages to the agent backend and remote daemon, executing undurable side effects and violating the durable execution invariant.
  - In `src/multiplexer/actor.rs:220-226` (unpatched lines 388-396), when a turn completed and `self.flush_if_dirty().await` failed to commit updated actor state to SQLite, the error was logged but the delivery claim was still acknowledged via `self.complete_delivery(delivery_id.as_deref(), &result)` with `&Ok(())`, falsely claiming successful delivery despite failed durability:
    ```rust
    TurnOutcome::Completed(result) => {
        if result.is_ok() {
            self.context = turn_context;
            let _ = crate::storage::clear_session_resume_pending(...).await;
            if let Err(error) = self.flush_if_dirty().await {
                tracing::error!(...);
            }
        }
        self.complete_delivery(delivery_id.as_deref(), &result).await;
    }
    ```
    Replays could execute without durable dedup/history/binding, and delivery claims were falsely marked delivered when persistence was broken.
- **Architectural Resolution**:
  - **Fail-Fast Inbound Ingress (`src/multiplexer/actor.rs`)**:
    - If `self.persist_inbound(&event).await` returns an error, the session actor logs the failure, resets `self.dirty = false;`, marks the delivery claim as failed via `self.complete_delivery(event.delivery_id.as_deref(), &Err(OmonError::Database(...)))`, resolves any waiting turn acknowledgement with `Err(...)`, and skips turn execution via `continue;` before the engine runner is ever invoked.
  - **Fail-Closed Delivery Completion on Flush Failure (`src/multiplexer/actor.rs`)**:
    - In `TurnOutcome::Completed(mut result)`, if the turn executed in-memory (`result.is_ok()`) but `self.flush_if_dirty().await` fails, `result` is converted to `Err(OmonError::Database(...))` and `mark_session_resume_pending` is recorded.
    - As a direct consequence, `self.complete_delivery` marks the delivery ledger entry as `failed` rather than `delivered`, preventing false acknowledgement of undurable turns.
    - The pending turn acknowledgement (`self.resolve_pending_ack(...)`) receives `Err(...)` rather than `Ok(())`.
  - **Binding Regression Tests (`tests/test_multiplexer.rs`)**:
    - Added real SQLite triggers with `RAISE(ABORT, 'write blocked')` on the messages table (Constituent 1) and on the sessions table state_json column (Constituent 2).
    - Verified that on failed user insert, the engine runner is never called, the turn ack reports error, and the delivery claim is marked `failed`.
    - Verified that on failed session flush, the engine runner executed, but the turn ack reports error and the delivery claim is marked `failed` (never `delivered`).
    - Registered command `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn -- --exact --nocapture` covers both variants end-to-end.

---

## 2. Captured RED Before Production Edits

Prior to modifying `src/multiplexer/actor.rs`, the regression scenarios were added to `tests/test_multiplexer.rs` and executed against the unpatched actor logic.

### Constituent 1: User Insert Persistence Failure
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn_on_user_insert -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-red-user-insert.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.69s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test

  thread 'persistence_failure_never_acknowledges_turn_on_user_insert' (3517755) panicked at tests/test_multiplexer.rs:1472:5:
  engine runner must not be called when user insert persistence fails
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test persistence_failure_never_acknowledges_turn_on_user_insert ... FAILED

  failures:

  failures:
      persistence_failure_never_acknowledges_turn_on_user_insert

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s

  error: test failed, to rerun pass `--test test_multiplexer`
  ```

### Constituent 2: Session Update Persistence Failure
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn_on_session_update -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-red-session-update.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test

  thread 'persistence_failure_never_acknowledges_turn_on_session_update' (3517838) panicked at tests/test_multiplexer.rs:1564:5:
  actor must surface durable flush failure rather than reporting success
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test persistence_failure_never_acknowledges_turn_on_session_update ... FAILED

  failures:

  failures:
      persistence_failure_never_acknowledges_turn_on_session_update

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s

  error: test failed, to rerun pass `--test test_multiplexer`
  ```

### Registered Binding Scenario
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-red.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.15s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test

  thread 'persistence_failure_never_acknowledges_turn' (3517859) panicked at tests/test_multiplexer.rs:1472:5:
  engine runner must not be called when user insert persistence fails
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test persistence_failure_never_acknowledges_turn ... FAILED

  failures:

  failures:
      persistence_failure_never_acknowledges_turn

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s

  error: test failed, to rerun pass `--test test_multiplexer`
  ```

---

## 3. Implementation Details

### `src/multiplexer/actor.rs`

1. **Abort Turn Before Side Effects on Inbound Write Failure**:
   ```rust
   if let Err(error) = self.persist_inbound(&event).await {
       tracing::error!(
           session = %self.context.key,
           %error,
           "failed to persist inbound event; aborting turn before side effects"
       );
       self.dirty = false;
       self.complete_delivery(
           event.delivery_id.as_deref(),
           &Err(OmonError::Database(format!(
               "failed to persist inbound event: {error}"
           ))),
       )
       .await;
       self.resolve_pending_ack(Err(OmonError::Database(format!(
           "failed to persist inbound event: {error}"
       ))));
       continue;
   }
   ```

2. **Propagate Flush Failure on Turn Completion**:
   ```rust
   TurnOutcome::Completed(mut result) => {
       if result.is_ok() {
           self.context = turn_context;
           if let Err(error) = self.flush_if_dirty().await {
               tracing::error!(
                   session = %self.context.key,
                   %error,
                   "failed to flush session actor on turn completion"
               );
               let _ = crate::storage::mark_session_resume_pending(
                   &self.pool,
                   &self.context.key.storage_key(),
               )
               .await;
               result = Err(OmonError::Database(format!(
                   "failed to flush session actor on turn completion: {error}"
               )));
           } else {
               let _ = crate::storage::clear_session_resume_pending(
                   &self.pool,
                   &self.context.key.storage_key(),
               )
               .await;
           }
       } else {
           self.dirty = false;
       }
       self.complete_delivery(delivery_id.as_deref(), &result)
           .await;
       self.resolve_pending_ack(match &result {
           Ok(()) => Ok(()),
           Err(error) => {
               Err(OmonError::Multiplexer(format!("turn failed: {error}")))
           }
       });
   ```

3. **`src/storage/db.rs`**:
   - Added `use crate::OmonError;` to support compiler diagnostics and dead_targets storage methods.

---

## 4. Captured GREEN After Production Edits

### Constituent 1: User Insert Persistence Failure
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn_on_user_insert -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-green-user-insert.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test
  test persistence_failure_never_acknowledges_turn_on_user_insert ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.02s
  ```

### Constituent 2: Session Update Persistence Failure
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn_on_session_update -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-green-session-update.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test
  test persistence_failure_never_acknowledges_turn_on_session_update ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.01s
  ```

### Registered Binding Scenario
- **Command**: `cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-green.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test
  test persistence_failure_never_acknowledges_turn ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 17 filtered out; finished in 0.02s
  ```

---

## 5. Full Target & Adjacent Suite Verifications

### Full Actor Target (`tests/test_multiplexer.rs`)
- **Command**: `cargo test --test test_multiplexer -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-actor-target.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.18s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 18 tests
  test route_reports_actor_startup_failure_instead_of_acknowledging_a_lost_event ... ok
  test stop_cancels_active_turn_and_clears_queued_events ... ok
  test gc_does_not_evict_an_actor_with_an_active_turn ... ok
  test delivery_ledger_deduplicates_concurrent_claims_and_records_latency ... ok
  test dropping_multiplexer_releases_actor_cycle_and_flushes_dirty_state ... ok
  test scale_to_zero_evicts_and_flushes_idle_sessions ... ok
  test stop_immediately_cancels_the_active_turn ... ok
  test persistence_failure_never_acknowledges_turn_on_session_update ... ok
  test terminal_outcomes_release_typing ... ok
  test guild_lane_is_shared_per_bot_not_author ... ok
  test events_arriving_during_running_turn_are_queued_and_processed_in_order ... ok
  test flush_failure_replays_completed_turn_on_restart ... ok
  test routes_multiple_sessions_in_parallel ... ok
  test persistence_failure_never_acknowledges_turn_on_user_insert ... ok
  test handles_events_sequentially_within_one_session ... ok
  test thread_inherits_parent_profile ... ok
  test persistence_failure_never_acknowledges_turn ... ok
  test transcript_level_inbound_dedup_skips_duplicate_platform_message_id ... ok

  test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.52s
  ```

### Full Backend Target (`tests/test_omo_backend.rs`)
- **Command**: `cargo test --test test_omo_backend`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U18-backend-target.log`
- **Result**: 34 passed; 0 failed; 0 ignored; finished in 30.61s.

### Lib Identity & Storage Targets
- **Session Identity**: `cargo test --lib models::session` -> Exit Code `0` (`.omo/evidence/hermes-parity-20260905/U18-models-session.log`).
- **Database Storage**: `cargo test --lib storage::db` -> Exit Code `0` (`.omo/evidence/hermes-parity-20260905/U18-storage-db.log`).

---

## 6. Build, Formatting, Diagnostics & Cleanup

- **LSP / Compiler Diagnostics**:
  - LSP daemon socket `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock` unreachable during invocation; compiler diagnostics checked with `cargo check --test test_multiplexer` and `cargo check --lib` (zero errors, zero warnings).
- **Cargo Build**:
  - `cargo build` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U18-build.log`).
- **Formatting**:
  - `rustfmt --check --edition 2021 src/multiplexer/actor.rs src/storage/db.rs tests/test_multiplexer.rs` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U18-format.log`).
- **Diff Check**:
  - `git diff --check src/multiplexer/actor.rs src/storage/db.rs tests/test_multiplexer.rs` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U18-diff.log`).
- **Predecessor Invariants Preserved**:
  - U07: Staged memory / write approval intact.
  - U13: Stop cancellation and typing release intact.
  - U14: Interactive 300s deadline bound intact.
  - U15: No resubmission after turn start intact.
  - U16: Durable session binding intact.
  - U17: Canonical storage key parser and bot identity intact.
