# U19 / S.F02, S.F03, R.R01, R.R04: Durable Accepted Queue and Replay

## 1. Summary

- **Unit**: U19 ("Durable accepted queue and replay")
- **Findings & Defect Diagnosis (S.F02, S.F03, R.R01, R.R04)**:
  - In unpatched `src/multiplexer/actor.rs:228-243`, when an accepted turn completed execution in-memory, `clear_session_resume_pending` was called *before* `flush_if_dirty()`. If `flush_if_dirty()` failed (e.g. SQLite database locked, read-only filesystem, trigger abort, or transient failure), `resume_pending` was already cleared to `0`. Furthermore, if flush failed, no compensation marked `resume_pending = 1`.
  - When the gateway restarted or crashed, startup recovery (`recover_resume_pending_sessions`) queried `SELECT session_key FROM sessions WHERE resume_pending = 1` and found 0 sessions. Consequently, the completed turn whose state failed to flush was completely dropped, session metadata mutations were lost, the turn was never replayed, and the delivery claim remained marked as `failed`.
  - In `recover_resume_pending_sessions` (`src/storage/db.rs`), recovery previously minted a fresh random `Uuid::new_v4()` with `delivery_id: None` and `platform_message_id: String::new()`. This caused replayed events to bypass transcript deduplication indices and insert duplicate user rows into `messages`, while disconnecting the replayed execution from the original ingress delivery claim in `delivery_ledger`.
  - If a turn completed in-memory and wrote an assistant message to `messages`, but session state flush failed, a naive recovery sweep would blindly replay the turn from scratch, emitting duplicate user-visible replies.
- **Architectural Resolution**:
  - **Transactional / Flush-Safe Resume Pending Lifecycle (`src/multiplexer/actor.rs`)**:
    - In `SessionActor::run`, under `TurnOutcome::Completed(mut result)`, `clear_session_resume_pending` is executed *only* when `flush_if_dirty().await` succeeds.
    - If `flush_if_dirty().await` fails:
      1. `mark_session_resume_pending` is immediately invoked on the database to ensure the session remains flagged with `resume_pending = 1` across process boundaries.
      2. As required by U18 fail-closed delivery guarantees, `result` is converted to `Err(OmonError::Database(...))` so `complete_delivery` marks the claim as `failed` rather than `delivered` until durability is restored.
    - Graceful actor shutdown and drop also invoke `mark_session_resume_pending` if the final shutdown flush fails.
  - **Delivery-Aware Replay & Recovery (`src/multiplexer/actor.rs`, `src/storage/db.rs`)**:
    - Implemented `SessionActor::recover_resume_pending_sessions` and updated `storage::db::recover_resume_pending_sessions`.
    - Recovery retrieves the original user message from `messages`, parses its existing message ID as the event UUID (`uuid::Uuid::parse_str(&unfinished.message_id)`), and restores the associated `delivery_id` from `delivery_ledger`.
    - When `persist_inbound` runs during replay, `ON CONFLICT(id) DO NOTHING` fires on the identical message UUID, preventing duplicate user transcript rows.
    - When the replayed turn completes and cleanly flushes to SQLite, `complete_delivery` transitions the delivery claim in `delivery_ledger` to `delivered`.
    - If the assistant response was already written to `messages` before the flush failure (`find_last_unfinished_user_turn` returns `None`), recovery does not re-route the turn (preventing duplicate user-visible replies), but accurately marks the delivery claim as `delivered` and clears `resume_pending`.
  - **Regression Test Suite (`tests/test_multiplexer.rs`)**:
    - Added `flush_failure_replays_completed_turn_on_restart`:
      1. Ingests an inbound event with a registered delivery claim in `delivery_ledger`.
      2. Installs a SQLite abort trigger `fail_sessions_state_flush` on `UPDATE OF state_json ON sessions`.
      3. Routes event to `SessionMultiplexer`; turn succeeds in-memory, but flush fails due to the trigger.
      4. Bounded subscription on `OutboundAction::Typing { active: false }` confirms turn completion without sleeps or polling loops.
      5. Simulates crash / restart by dropping the multiplexer, dropping the failure trigger, and reconstructing a new `SessionMultiplexer`.
      6. Runs `SessionActor::recover_resume_pending_sessions`.
      7. Asserts the session is recovered (`recovered == 1`), replayed turn executes (`runs == 2`), clean flush succeeds, `resume_pending` returns to `0`, session state is durably persisted in SQLite, and `delivery_ledger` status transitions to `delivered`.

---

## 2. Captured RED Before Production Edits

Prior to modifying production code in `src/multiplexer/actor.rs` and `src/storage/db.rs`, the exact regression test `flush_failure_replays_completed_turn_on_restart` was registered in `tests/test_multiplexer.rs` and executed against the unpatched codebase.

### Exact Verification Command
- **Command**: `cargo test --test test_multiplexer flush_failure_replays_completed_turn_on_restart -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-red.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.68s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test

  thread 'flush_failure_replays_completed_turn_on_restart' (1142735) panicked at tests/test_multiplexer.rs:882:5:
  assertion `left == right` failed: session with failed flush must remain marked resume_pending and be recovered on restart
    left: 0
   right: 1
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test flush_failure_replays_completed_turn_on_restart ... FAILED

  failures:

  failures:
      flush_failure_replays_completed_turn_on_restart

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 11 filtered out; finished in 0.02s

  error: test failed, to rerun pass `--test test_multiplexer`
  ```

---

## 3. Implementation Details

### `src/multiplexer/actor.rs`

1. **Flush Failure Retains `resume_pending` Marker**:
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
    self.complete_delivery(delivery_id.as_deref(), &result).await;
...
```

2. **Actor Drop / Shutdown Flush Compensation**:
```rust
if let Err(error) = self.flush_if_dirty().await {
    tracing::error!(session = %self.context.key, %error, "failed to flush session actor during shutdown");
    let _ = crate::storage::mark_session_resume_pending(
        &self.pool,
        &self.context.key.storage_key(),
    )
    .await;
}
```

3. **`SessionActor::recover_resume_pending_sessions`**:
```rust
impl SessionActor {
    /// Recovers sessions marked `resume_pending` from previous runs or failed flushes,
    /// re-dispatching unfinished user turns or marking completed deliveries.
    pub async fn recover_resume_pending_sessions(
        pool: &SqlitePool,
        multiplexer: &SessionMultiplexer,
    ) -> Result<usize> {
        let pending_keys = crate::storage::fetch_resume_pending_session_keys(pool).await?;
        let mut resumed_count = 0;
        for session_key in pending_keys {
            let storage_key = session_key.storage_key();
            let is_suspended = crate::storage::is_session_suspended(pool, &storage_key).await?;
            let cleared = crate::storage::clear_session_resume_pending(pool, &storage_key).await?;
            if !cleared {
                continue;
            }
            if is_suspended {
                tracing::info!(
                    session = %session_key,
                    "skipping restart recovery for suspended session"
                );
                continue;
            }

            if let Some(unfinished) =
                crate::storage::find_last_unfinished_user_turn(pool, &storage_key).await?
            {
                let delivery_id: Option<String> = if let Some(ref pid) = unfinished.platform_message_id {
                    sqlx::query_scalar(
                        "SELECT message_id FROM delivery_ledger WHERE session_key = ? AND (platform_message_id = ? OR message_id = ?) ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(&storage_key)
                    .bind(pid)
                    .bind(pid)
                    .fetch_optional(pool)
                    .await
                    .unwrap_or(None)
                } else {
                    sqlx::query_scalar(
                        "SELECT message_id FROM delivery_ledger WHERE session_key = ? ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(&storage_key)
                    .fetch_optional(pool)
                    .await
                    .unwrap_or(None)
                };

                let attachments: Vec<crate::MessageAttachment> =
                    serde_json::from_str(&unfinished.metadata_json).unwrap_or_default();
                let event = InboundEvent {
                    id: uuid::Uuid::parse_str(&unfinished.message_id)
                        .unwrap_or_else(|_| uuid::Uuid::new_v4()),
                    session: session_key.clone(),
                    platform_message_id: String::new(),
                    delivery_id,
                    content: unfinished.content,
                    attachments,
                    received_at: unfinished.created_at,
                };
                tracing::info!(
                    session = %session_key,
                    "re-dispatching unfinished user turn on restart recovery"
                );
                if let Err(error) = multiplexer.route(event).await {
                    tracing::error!(
                        session = %session_key,
                        %error,
                        "failed to route resumed session event"
                    );
                } else {
                    resumed_count += 1;
                }
            } else {
                let delivery_id: Option<String> = sqlx::query_scalar(
                    "SELECT message_id FROM delivery_ledger WHERE session_key = ? ORDER BY created_at DESC LIMIT 1",
                )
                .bind(&storage_key)
                .fetch_optional(pool)
                .await
                .unwrap_or(None);

                if let Some(del_id) = delivery_id {
                    let ledger = DeliveryLedgerService::new(pool.clone());
                    let _ = ledger.mark_delivered(&del_id).await;
                }
                resumed_count += 1;
            }
        }
        Ok(resumed_count)
    }
```

### `src/storage/db.rs`
Updated `recover_resume_pending_sessions` to match the exact same identity-preserving, delivery-recovering, and duplicate-suppressing semantics.

---

## 4. Captured GREEN After Production Edits

### Exact Verification Command
- **Command**: `cargo test --test test_multiplexer flush_failure_replays_completed_turn_on_restart -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-green.log`
- **Captured Output**:
  ```text
     Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
      Finished `test` profile [unoptimized + debuginfo] target(s) in 13.21s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 1 test
  test flush_failure_replays_completed_turn_on_restart ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 11 filtered out; finished in 0.02s
  ```

---

## 5. Suite & Adjacent Verifications

### 1. Multiplexer Integration Suite
- **Command**: `cargo test --test test_multiplexer -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-suite.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.57s
       Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 12 tests
  test delivery_ledger_deduplicates_concurrent_claims_and_records_latency ... ok
  test route_reports_actor_startup_failure_instead_of_acknowledging_a_lost_event ... ok
  test gc_does_not_evict_an_actor_with_an_active_turn ... ok
  test scale_to_zero_evicts_and_flushes_idle_sessions ... ok
  test stop_cancels_active_turn_and_clears_queued_events ... ok
  test dropping_multiplexer_releases_actor_cycle_and_flushes_dirty_state ... ok
  test stop_immediately_cancels_the_active_turn ... ok
  test events_arriving_during_running_turn_are_queued_and_processed_in_order ... ok
  test routes_multiple_sessions_in_parallel ... ok
  test flush_failure_replays_completed_turn_on_restart ... ok
  test handles_events_sequentially_within_one_session ... ok
  test transcript_level_inbound_dedup_skips_duplicate_platform_message_id ... ok

  test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
  ```

### 2. Storage DB Unit Suite
- **Command**: `cargo test --lib storage::db::tests -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-db.log`
- **Result**: 22 passed; 0 failed; 0 ignored; finished in 0.31s.

### 3. Multiplexer Actor Unit Suite
- **Command**: `cargo test --lib multiplexer::actor::tests -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-actor.log`
- **Result**: 5 passed; 0 failed; 0 ignored; finished in 0.11s.

### 4. Entrypoint Recovery Tests
- **Command**: `cargo test --bin omo-gateway test_recover_resume_pending -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U19-main.log`
- **Result**: 2 passed; 0 failed; 0 ignored; finished in 0.02s.

---

## 6. Build, Formatting, Diagnostics & Cleanup

- **Cargo Build**:
  - `cargo build` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U19-build.log`).
- **Formatting**:
  - `rustfmt --edition 2021 --check src/multiplexer/actor.rs src/storage/db.rs tests/test_multiplexer.rs` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U19-format.log`).
- **Diff Check**:
  - `git diff --check src/multiplexer/actor.rs src/storage/db.rs tests/test_multiplexer.rs` exited with code `0` (`.omo/evidence/hermes-parity-20260905/U19-diff.log`).
- **Predecessor Invariants Preserved**:
  - U12 Correlation: Interrupted / failed distinguishing logic and sequence checking preserved.
  - U13 Cancellation: Interruption and typing indicator releases preserved.
  - U14 Deadline: 300s budget preserved.
  - U15 No Resubmission: Split between `setup_turn` and `execute_turn` preserved.
  - U16 Durable Session Binding: `persist_session_binding` and reconnect durability checkpoint preserved.
  - U17 Canonical Storage Key Parsing: `SessionKey::from_storage_key` roundtrip preserved.
  - U18 Fail Before Undurable Side Effects: Inbound persistence abort-before-run and flush-failure-to-Err conversion preserved.
- **Hygiene & Discipline**:
  - Strictly modified only scoped files: `src/multiplexer/actor.rs`, `src/storage/db.rs`, `tests/test_multiplexer.rs`, and `.omo/evidence/hermes-parity-20260905/U19-*`.
  - Zero sleeps, polling loops, or yield-wait patterns; async synchronization uses bounded event subscriptions (`OutboundAction::Typing`).
