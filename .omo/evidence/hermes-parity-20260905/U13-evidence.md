# U13 / S.F04 & R.R11: Remote Interruption and Typing Lifecycle Release

## 1. Summary

- **Unit**: U13 ("Interrupt actual remote work")
- **Findings**:
  - `S.F04`: Actor dropped run future before remote cancellation; `OmoBackend` inherited default no-op `cancel`, leaving remote tool work alive on disconnect while next turns queued behind wedged daemon execution.
  - `R.R11`: Lifecycle-owned `Typing(false)` was omitted on terminal completion, error, stop, and shutdown paths, leaking the typing indicator until explicit manual commands or process termination.
- **Files Modified**:
  - `src/agent/backend.rs`: `run_cancelable` triggers bounded remote `cancel` on cancellation token signal.
  - `src/agent/omo_backend.rs`: Retains acknowledged active turn identity (`ActiveTurn { thread_id, turn_id }`) in `active_turns` independently of dropped futures. Implements bounded `cancel(&self, session)` (up to 5s deadline) connecting over WebSocket, initializing, sending exact `turn/interrupt { threadId, turnId }`, awaiting ACK / terminal frame before reporting success, and cleaning up active turn state. Cleans up active turn on terminal completion, errors, deadline expiration, silence timeout, and approval denials.
  - `src/multiplexer/actor.rs`: Adds `release_typing(&self)` helper and releases typing on all terminal paths (normal completion `Ok`, runner error `Err`, user `Stop`, and graceful actor `Shutdown`). Awaits `self.runner.cancel(&self.context)` and propagates interrupt error/success to stop reply.
  - `tests/test_omo_backend.rs`: Adds stateful loopback peer regressions `stop_interrupts_remote_turn_before_ack` and `stop_interrupts_remote_turn`. Also adds unit regression `turn_terminal_paths_release_typing` in `actor.rs`'s internal test suite.

---

## 2. Captured RED Before Production Changes

All three regressions were registered first and executed against unmodified production logic, confirming behavioral RED with exit code 101.

### 1. `stop_interrupts_remote_turn_before_ack`
- **Command**: `cargo test --test test_omo_backend stop_interrupts_remote_turn_before_ack -- --exact --nocapture`
- **Exit Code**: `101`
- **Log**: `U13-red-stop_interrupts_remote_turn_before_ack.log`
- **Failure**:
  ```text
  running 1 test
  thread 'stop_interrupts_remote_turn_before_ack' panicked at tests/test_omo_backend.rs:2300:5:
  peer must receive turn/interrupt before stop reports success
  test stop_interrupts_remote_turn_before_ack ... FAILED
  failures:
      stop_interrupts_remote_turn_before_ack
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 26 filtered out; finished in 0.05s
  ```

### 2. `turn_terminal_paths_release_typing`
- **Command**: `cargo test --lib turn_terminal_paths_release_typing -- --nocapture`
- **Exit Code**: `101`
- **Log**: `U13-red-turn_terminal_paths_release_typing.log`
- **Failure**:
  ```text
  running 1 test
  thread 'multiplexer::actor::tests::turn_terminal_paths_release_typing' panicked at src/multiplexer/actor.rs:993:13:
  assertion `left == right` failed: success path must start and release typing
    left: [true]
   right: [true, false]
  test multiplexer::actor::tests::turn_terminal_paths_release_typing ... FAILED
  failures:
      multiplexer::actor::tests::turn_terminal_paths_release_typing
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 332 filtered out; finished in 0.01s
  ```

### 3. `stop_interrupts_remote_turn`
- **Command**: `cargo test --test test_omo_backend stop_interrupts_remote_turn -- --exact --nocapture`
- **Exit Code**: `101`
- **Log**: `U13-red-stop_interrupts_remote_turn.log`
- **Failure**:
  ```text
  running 1 test
  thread 'stop_interrupts_remote_turn' panicked at tests/test_omo_backend.rs:2499:5:
  interrupt sent for lane A
  test stop_interrupts_remote_turn ... FAILED
  failures:
      stop_interrupts_remote_turn
  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 26 filtered out; finished in 0.01s
  ```

---

## 3. Implementation

1. **Active Turn Retention Across Dropped Futures**:
   - Stored in `OmoBackend::active_turns: Arc<ParkingMutex<HashMap<String, ActiveTurn>>>` keyed by `session.key.storage_key()`.
   - Populated immediately when turn acknowledgement `{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"t1",...}}}` is parsed.
   - Preserved when the local execution future is dropped by `tokio::select!` / `drop(run)`.

2. **Bounded Remote Cancel and Error Propagation**:
   - `OmoBackend::cancel(&self, session: &SessionContext) -> Result<()>` removes the active turn from `active_turns`.
   - Connects to the configured `appserver_url`, completes initialization handshake, and sends `turn/interrupt { threadId, turnId }` with id `9001`.
   - Bounded by `request_timeout.min(5s)`.
   - Awaits response id `9001` or terminal `turn/completed` with status `interrupted`/`completed`/`failed`. Peer errors or timeouts return `Err(OmonError::Llm(...))` which propagate to actor stop response `reply.send(cancel_res.map(|_| true))`.

3. **Sticky Binding Preservation**:
   - Neither `cancel` nor actor stop mutates `omo_thread_id` metadata in SQLite or `OmoBackend::thread_ids`. Subsequent turns reuse the authoritative remote thread binding.

4. **Typing Lifecycle Cleanup**:
   - Added `SessionActor::release_typing(&self)` dispatching `OutboundAction::Typing { session, active: false }` for non-cron sessions.
   - Invoked on all terminal paths:
     - `TurnOutcome::Completed`: after delivery completion and error message send.
     - `TurnOutcome::Stopped`: after remote `interrupt_turn` completes.
     - `TurnOutcome::Shutdown`: after remote `interrupt_turn` completes.
     - `ActorCommand::Stop` when actor is idle.
   - Multi-lane isolation: Stopping Lane A releases Lane A's typing guard immediately while Lane B's typing guard remains active until Lane B finishes.

---

## 4. Exact GREEN Verification

All three target commands passed with exit code 0:

### 1. `stop_interrupts_remote_turn_before_ack`
- **Command**: `cargo test --test test_omo_backend stop_interrupts_remote_turn_before_ack -- --exact --nocapture`
- **Exit Code**: `0`
- **Log**: `U13-green-stop_interrupts_remote_turn_before_ack.log`
- **Output**:
  ```text
  running 1 test
  test stop_interrupts_remote_turn_before_ack ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 26 filtered out; finished in 0.02s
  ```

### 2. `turn_terminal_paths_release_typing`
- **Command**: `cargo test --lib turn_terminal_paths_release_typing -- --nocapture`
- **Exit Code**: `0`
- **Log**: `U13-green-turn_terminal_paths_release_typing.log`
- **Output**:
  ```text
  running 1 test
  test multiplexer::actor::tests::turn_terminal_paths_release_typing ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 333 filtered out; finished in 0.03s
  ```

### 3. `stop_interrupts_remote_turn`
- **Command**: `cargo test --test test_omo_backend stop_interrupts_remote_turn -- --exact --nocapture`
- **Exit Code**: `0`
- **Log**: `U13-green-stop_interrupts_remote_turn.log`
- **Output**:
  ```text
  running 1 test
  test stop_interrupts_remote_turn ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 26 filtered out; finished in 0.05s
  ```

---

## 5. Adjacent Verification & Diagnostics

1. **All `test_omo_backend` tests**:
   - `cargo test --test test_omo_backend`
   - Result: 27 passed; 0 failed; 0 ignored (`U13-adjacent.log`, exit 0).
2. **Actor unit tests**:
   - `cargo test --lib multiplexer::actor`
   - Result: 5 passed; 0 failed; 0 ignored (exit 0).
3. **Multiplexer integration tests**:
   - `cargo test --test test_multiplexer`
   - Result: 11 passed; 0 failed; 0 ignored (exit 0).
4. **Cargo clippy**:
   - `cargo clippy --lib -- -D warnings`
   - Initial run identified two attributable lints in `src/agent/omo_backend.rs`:
     - `single_match` around `item.type == "agentMessage"`
     - `collapsible_if` around active turn terminal status checking in `cancel`
   - Both were refactored cleanly without `#[allow(...)]` attributes while preserving exact correlation and terminal checks.
   - Re-check: `cargo clippy --lib -- -D warnings` exit code 0 (`U13-clippy.log`, `U13-clippy.exit`).
5. **Cargo build**:
   - `cargo build`
   - Result: compiled successfully with zero errors (`U13-build.log`, exit 0).
6. **LSP diagnostics**:
   - `lsp_diagnostics` on `src/agent/backend.rs`, `src/agent/omo_backend.rs`, `src/multiplexer/actor.rs`, and `tests/test_omo_backend.rs`: 0 errors, 0 warnings.
7. **Formatting & Diff checks**:
   - `rustfmt --check --edition 2021 src/agent/backend.rs src/agent/omo_backend.rs src/multiplexer/actor.rs tests/test_omo_backend.rs`: exit 0, clean (`U13-format.log`).
   - `git diff --check src/agent/backend.rs src/agent/omo_backend.rs src/multiplexer/actor.rs tests/test_omo_backend.rs`: exit 0, clean (`U13-diff.log`).

---

## 6. Scope Audit

- Edits strictly limited to:
  - `src/agent/backend.rs`
  - `src/agent/omo_backend.rs`
  - `src/multiplexer/actor.rs`
  - `tests/test_omo_backend.rs`
  - `.omo/evidence/hermes-parity-20260905/U13-*`
- No git commits, pushes, external Discord API calls, or global configuration edits.
- All test fixtures use bounded in-memory or loopback WebSocket primitives; all spawned peers and tasks cleanly aborted upon test completion.
