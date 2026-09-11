# U15 / S.F07: No Accepted-Turn Resubmission, Only Its Full Registered Constituent Behavior

## 1. Summary

- **Unit**: U15 ("No accepted-turn resubmission, only its full registered constituent behavior")
- **Findings & Defect Diagnosis (S.F07)**:
  - In `src/agent/omo_backend.rs`, `run` previously called `run_once` and checked if the resulting error message matched retry patterns like `"connection closed"`, `"Connection reset"`, or `"Broken pipe"`.
  - Because `run_once` returned `Err(OmonError::Llm("connection closed before turn completion".into()))` upon mid-turn or post-ACK socket disconnection, `run` treated post-submission disconnects as retryable.
  - Calling `run_once` a second time reconnected and issued a second `turn/start` JSON-RPC frame. Because the installed daemon treats `clientUserMessageId` as correlation-only and allocates a fresh turn on every `turn/start` without deduplicating requests, an already accepted or in-flight mutating task executed twice.
  - Conversely, pre-submit transport drops during `initialize` or `thread/resume` that returned `"closed before initialize response"` were not retried.
- **Architectural Resolution**:
  - Divided `OmoBackend` execution into two strictly ordered phases:
    1. `setup_turn(session, deadline, effective_total_timeout) -> Result<(WsStream, String)>`: Positively pre-submit connection, `initialize` handshake, and `thread/resume` / `thread/start` resolution. Transport failures here are proven pre-submit (no `turn/start` has been written to the socket) and are retried once within the overall deadline budget.
    2. `execute_turn(ws, thread_id, session, event, deadline, effective_total_timeout) -> Result<()>`: Begins with `ws.send(turn_start_request(...))`. From the moment socket write begins, the turn is considered submitted. Mid-turn drops, ACK timeouts, and post-ACK stream closures return a failure immediately without automatic replay.
- **Files Modified**:
  - `src/agent/omo_backend.rs`: Refactored `run_once` into `setup_turn` (pre-submit) and `execute_turn` (post-submit). Updated `AgentBackend::run` to retry only proven pre-submit transport failures in `setup_turn`.
  - `tests/test_omo_backend.rs`: Added stateful loopback peer tests:
    - `accepted_disconnect_does_not_resubmit_turn`: Daemon ACKs and records counter increment, closes stream before terminal. Demonstrates RED (2 `turn/start`s / counter increments) and GREEN (exactly 1 `turn/start` / counter increment, error returned).
    - `pre_ack_disconnect_does_not_resubmit_turn`: Daemon receives `turn/start`, increments side-effect counter, and disconnects before ACK. Demonstrates RED (2 counter increments) and GREEN (exactly 1 counter increment, error returned without replay).
    - `pre_submit_disconnect_retries_and_succeeds`: Demonstrates that positively pre-submit connection failures (disconnect during `initialize`) still retry once and succeed.

---

## 2. Captured RED Before Production Edits

- **Command**: `cargo test --test test_omo_backend accepted_disconnect_does_not_resubmit_turn -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U15-red.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.15s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test

  thread 'accepted_disconnect_does_not_resubmit_turn' (11004358) panicked at tests/test_omo_backend.rs:3503:5:
  assertion `left == right` failed: accepted turn must not be resubmitted after disconnect; turn/start must be sent exactly once
    left: 2
   right: 1
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test accepted_disconnect_does_not_resubmit_turn ... FAILED

  failures:

  failures:
      accepted_disconnect_does_not_resubmit_turn

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 30 filtered out; finished in 0.52s

  error: test failed, to rerun pass `--test test_omo_backend`
  ```

---

## 3. Implementation Details

1. **Phase Isolation in `src/agent/omo_backend.rs`**:
   - `setup_turn`:
     ```rust
     async fn setup_turn(
         &self,
         session: &mut SessionContext,
         deadline: tokio::time::Instant,
         effective_total_timeout: Duration,
     ) -> Result<(WsStream, String)> {
         let mut ws = self.connect_ws(deadline).await?;
         self.do_initialize(&mut ws, deadline).await?;
         let thread_id = self.resolve_thread_id(&mut ws, session, deadline).await?;

         if tokio::time::Instant::now() >= deadline {
             return Err(OmonError::Llm(format!(
                 "turn exceeded total deadline of {effective_total_timeout:?}"
             )));
         }

         Ok((ws, thread_id))
     }
     ```
   - `execute_turn`:
     ```rust
     async fn execute_turn(
         &self,
         mut ws: WsStream,
         thread_id: String,
         session: &mut SessionContext,
         event: InboundEvent,
         deadline: tokio::time::Instant,
         effective_total_timeout: Duration,
     ) -> Result<()> {
         let user_prompt = render_user_prompt(&event);
         let model = session
             .state
             .active_model
             .as_deref()
             .or(self.config.default_model.as_deref());
         ws.send(turn_start_request(&thread_id, &user_prompt, model))
             .await
             .map_err(|e| OmonError::Llm(format!("failed to send turn/start: {e}")))?;
         ...
     ```

2. **Strict Retry Policy in `AgentBackend::run`**:
   - Retries are scoped exclusively to `setup_turn`:
     ```rust
     let setup_outcome = self
         .setup_turn(session, deadline, effective_total_timeout)
         .await;
     let (ws, thread_id) = match setup_outcome {
         Ok(pair) => pair,
         Err(err) => {
             let retryable = matches!(
                 &err,
                 OmonError::Llm(msg)
                     if msg.contains("Connection reset")
                         || msg.contains("os error 54")
                         || msg.contains("Broken pipe")
                         || msg.contains("connection closed")
                         || msg.contains("closed before")
                         || msg.contains("Handshake not finished")
                         || msg.contains("Connection refused")
                         || msg.contains("os error 61")
                         || msg.contains("failed to connect to omo app-server")
                         || msg.contains("ws error in initialize")
                         || msg.contains("ws error in thread/")
             );
             if retryable {
                 tracing::warn!(
                     "omo daemon connection reset or refused before turn submission; retrying once after cooldown"
                 );
                 let cooldown = Duration::from_millis(500);
                 if tokio::time::Instant::now() + cooldown >= deadline {
                     tokio::time::sleep_until(deadline).await;
                     return Err(OmonError::Llm(format!(
                         "turn exceeded total deadline of {effective_total_timeout:?}"
                     )));
                 }
                 tokio::time::sleep(cooldown).await;
                 self.setup_turn(session, deadline, effective_total_timeout)
                     .await?
             } else {
                 return Err(err);
             }
         }
     };

     self.execute_turn(
         ws,
         thread_id,
         session,
         event,
         deadline,
         effective_total_timeout,
     )
     .await
     ```

---

## 4. Verification Evidence

### Exact Test: Accepted Disconnect Does Not Resubmit
- **Command**: `cargo test --test test_omo_backend accepted_disconnect_does_not_resubmit_turn -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U15-green.log`
- **Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test
  test accepted_disconnect_does_not_resubmit_turn ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 0.02s
  ```

### Pre-ACK Disconnect Does Not Resubmit
- **Command**: `cargo test --test test_omo_backend pre_ack_disconnect_does_not_resubmit_turn -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U15-green-preack.log`
- **Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.20s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test
  test pre_ack_disconnect_does_not_resubmit_turn ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 0.01s
  ```

### Pre-Submit Disconnect Retries and Succeeds
- **Command**: `cargo test --test test_omo_backend pre_submit_disconnect_retries_and_succeeds -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U15-green-presubmit.log`
- **Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.18s
       Running tests/test_omo_backend.rs (target/debug/deps/test_omo_backend-a074f7c420765738)

  running 1 test
  test pre_submit_disconnect_retries_and_succeeds ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 30 filtered out; finished in 0.52s
  ```

### Full Backend Integration Test Suite
- **Command**: `cargo test --test test_omo_backend -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U15-suite.log`
- **Output Summary**:
  ```text
  test result: ok. 31 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.53s
  ```

### Static Checks, Diagnostics & Build
- `lsp_diagnostics`: 0 errors, 0 warnings on `src/agent/omo_backend.rs` and `tests/test_omo_backend.rs`
- `cargo check --tests`: Exit `0` (`.omo/evidence/hermes-parity-20260905/U15-build.log`)
- `rustfmt --check --edition 2021 src/agent/omo_backend.rs tests/test_omo_backend.rs`: Exit `0` (`.omo/evidence/hermes-parity-20260905/U15-format.log`)
- `git diff --check src/agent/omo_backend.rs tests/test_omo_backend.rs`: Exit `0` (`.omo/evidence/hermes-parity-20260905/U15-diff.log`)
- Patch artifact: `.omo/evidence/hermes-parity-20260905/U15-current.patch`
