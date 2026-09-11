# Remote Control Phase Independent Verification Report: U13, U14, U15

**Session:** hephaestus (st_01a07908)  
**Parent Session:** 01a071f6-cc78-7761-86ab-a931f8c15133  
**Evaluated Units:** U13 (`S.F04`, `R.R11`), U14 (`S.F05`), U15 (`S.F07`)  
**Scope:** Independent verification against full manifest scenarios, code inspection of backend/actor/trait/test implementations, baseline preservation, chronology audit (RED vs GREEN), and fresh re-execution of test scenarios, target suites, build, and format gates.  
**Constraint Compliance:** Zero production or test code edits were made during this verification turn. All existing worktree changes and unassigned unit states were preserved untouched.

---

## Executive Summary & Independent Verdicts

| Unit | Title | Findings Covered | Manifest Verification Scenario | Independent Verdict |
| :--- | :--- | :--- | :--- | :--- |
| **U13** | Interrupt actual remote work | `S.F04`, `R.R11` | Stateful peer retains remote work on disconnect until interrupt; exact `turn/interrupt { threadId, turnId }` sent and acknowledged before stop returns; actor propagates failure if interrupt fails; typing released on all terminal paths (success, error, stop, shutdown) without cross-lane leakage; remote thread binding preserved. | **PASS** |
| **U14** | Absolute interactive deadline | `S.F05` | Single absolute deadline anchored at `run()` entry (interactive capped at 300s, cron independent); bounds connect, initialize, thread/start, thread/resume, turn ACK (<= 30s), streaming, and pre-submit retries; select timer wakes independently of incoming frames; retry budget retains original origin timestamp; cleanup reserve inside deadline for interrupt. | **PASS** |
| **U15** | No accepted-turn resubmission | `S.F07` | Strict phase separation: `setup_turn` (pre-submit) vs `execute_turn` (post-submit). Accepted disconnect produces exactly 1 `turn/start` and 1 side effect without resubmission; ambiguous pre-ACK disconnect produces exactly 1 `turn/start` and 1 side effect without resubmission; pre-submit transport failure still retries once within deadline budget. | **PASS** |

**Remote Control Parity Verdict:** All three units (U13, U14, U15) **PASS** their full manifest scenarios with deterministic, evidence-backed runtime proof.

---

## 1. Unit U13: Remote Interruption and Typing Lifecycle Release (`S.F04`, `R.R11`)

### Manifest Scenario & Requirements
- **S.F04 (Remote Interrupt before Stop ACK):**
  - Previously, `SessionActor` dropped the runner future before calling `cancel()`. Furthermore, `OmoBackend` inherited the default no-op `cancel()` from `AgentBackend`, leaving remote tools running on the daemon while the gateway acknowledged `/stop` locally. Subsequent turns queued behind runaway remote execution.
  - Requirement: Retain active remote turn identity independently of dropped futures; send and await `turn/interrupt { threadId, turnId }` with a bounded timeout before acknowledging stop locally; propagate interrupt failure/timeout to actor stop reply; retain authoritative remote thread binding in SQLite across stop.
  - Protocol Fake / Stateful Peer Requirement: Faithful peer behavior must keep work alive on disconnect until explicit `turn/interrupt` frame is received; disconnect must NOT simulate cancellation; verify no post-stop tool completion.
- **R.R11 (Typing Lifecycle Release):**
  - Previously, typing indicators leaked indefinitely upon normal completion, runner errors, or stops until manual commands or process termination.
  - Requirement: Lifecycle-owned `Typing(false)` emitted on every terminal path (success, error, stop, shutdown); multi-lane isolation ensures stopping Lane A does not drop or clear the typing guard of active Lane B.

### Source Code Inspection
1. **Runner Cancellation Trait (`src/agent/backend.rs`):**
   - In `AgentBackend::run_cancelable`:
     ```rust
     tokio::select! {
         result = self.run(session, event) => result,
         _ = cancellation.cancelled() => {
             let _ = self.cancel(session).await;
             Err(OmonError::Multiplexer("agent turn cancelled".into()))
         }
     }
     ```
2. **Active Turn Retention & Remote Cancel (`src/agent/omo_backend.rs`):**
   - Retains active turn state in `OmoBackend::active_turns: Arc<ParkingMutex<HashMap<String, ActiveTurn>>>` keyed by `session.key.storage_key()`.
   - Populated immediately upon parsing successful `turn/start` response (`id == 3`, turn id extracted into `ActiveTurn { thread_id, turn_id }`).
   - `OmoBackend::cancel(&self, session)`:
     - Extracts `active_turns.lock().remove(...)`.
     - Connects over WebSocket, completes `initialize`, sends JSON-RPC id `9001` with `method: "turn/interrupt"`, `params: { "threadId": active.thread_id, "turnId": active.turn_id }`.
     - Awaits response id `9001` or terminal `turn/completed` with status `interrupted`/`completed`/`failed`.
     - Bounded by `cancel_timeout = config.request_timeout.min(5s)`.
   - Cleans up `active_turns` on terminal `turn/completed`, deadline expiry, repeated tool approval denials, silence timeout, and mid-turn stream errors.
   - Preserves `omo_thread_id` metadata in SQLite and `thread_ids` map; thread binding is never destroyed by cancellation.
3. **Actor Lifecycle & Typing Release (`src/multiplexer/actor.rs`):**
   - Implemented `SessionActor::release_typing(&self)` dispatching `OutboundAction::Typing { session, active: false }` for non-cron sessions.
   - Enforced on all terminal paths:
     - `TurnOutcome::Completed(result)`: dispatches `release_typing().await`.
     - `TurnOutcome::Stopped(reply)`: invokes `self.interrupt_turn()`, releases typing, and replies `cancel_res.map(|_| true)` (propagating cancel failures).
     - `TurnOutcome::Shutdown`: invokes `self.interrupt_turn()`, releases typing, and marks `resume_pending`.
     - `ActorCommand::Stop` when idle: drains pending queue and calls `release_typing().await`.

### Runtime Evidence & Regressions Executed
- **RED Baseline Audit:**
  - `U13-red-stop_interrupts_remote_turn_before_ack.log` (exit 101: panicked at `peer must receive turn/interrupt before stop reports success`).
  - `U13-red-turn_terminal_paths_release_typing.log` (exit 101: `assertion left == right failed: success path must start and release typing, left: [true], right: [true, false]`).
  - `U13-red-stop_interrupts_remote_turn.log` (exit 101: panicked at `interrupt sent for lane A`).
- **Independent Re-execution Log:** `.omo/evidence/hermes-parity-20260905/remote-control-verification-u13.log`
  1. `cargo test --test test_omo_backend stop_interrupts_remote_turn_before_ack -- --exact --nocapture` -> **PASS** (1 passed, 0 failed, 0.02s).
     - Confirmed: Peer kept work alive across disconnect (`peer_kept_work_alive = true`), received `turn/interrupt` before `stop` reported success (`interrupt_received = true`), no post-stop tool completion (`tool_completed = false`), and sticky thread binding `r1` preserved in SQLite.
  2. `cargo test --lib turn_terminal_paths_release_typing -- --nocapture` -> **PASS** (1 passed, 0 failed, 0.02s).
     - Confirmed: Success path emits `[true, false]`, error path emits `[true, false]`, stop path emits `[true, false]`, shutdown path emits `[true, false]`, and lane isolation ensures lane A stop releases typing while lane B typing remains active (`[true]`).
  3. `cargo test --test test_omo_backend stop_interrupts_remote_turn -- --exact --nocapture` -> **PASS** (1 passed, 0 failed, 0.01s).
     - Confirmed: Multi-session concurrent routing, Lane A stop sends interrupt to daemon for `(r1, t1)`, Lane A typing releases to `false` while Lane B typing remains active (`true`), and Lane B subsequently completes without corruption.

---

## 2. Unit U14: Absolute Interactive Deadline (`S.F05`)

### Manifest Scenario & Requirements
- **S.F05 (300s Interactive Cap Across All Protocol Phases):**
  - Previously, timeouts were reset independently on every frame arrival (e.g. `connect_timeout`, `request_timeout` on each handshake frame), total timeout started only after turn start, pinging initialize could hang indefinitely, and silent streaming exceeded timeouts until the next incoming frame.
  - Requirement: One absolute deadline anchored at `AgentBackend::run` entry; interactive turns capped at 300s (`config.total_timeout.min(300s)`), while preserving independent cron timeout policies; deadline spans connect, initialize, thread/start, thread/resume, turn/start ACK (capped <= 30s), streaming, and retries; select timer wakes independently of incoming frames (including ping-only streams); retry budget measures against original entry time; cleanup reserve inside deadline triggers bounded `turn/interrupt`.

### Source Code Inspection
1. **Absolute Deadline Anchor (`src/agent/omo_backend.rs`):**
   - In `AgentBackend::run`:
     ```rust
     let effective_total_timeout = if is_cron_session {
         self.config.total_timeout
     } else {
         self.config.total_timeout.min(Duration::from_secs(300))
     };
     let deadline = tokio::time::Instant::now() + effective_total_timeout;
     ```
2. **Phase Bounding Across Setup and Retry:**
   - `connect_ws(deadline)`: Caps individual connection attempts with `deadline.min(now + connect_timeout)`.
   - `do_initialize(ws, deadline)`: Awaits initialize response bounded by `deadline.min(now + request_timeout)`.
   - `resolve_thread_id(ws, session, deadline)`: Resume and start calls bounded by `deadline.min(now + request_timeout)`.
   - Turn ACK bound: `started_at + Duration::from_secs(30).min(deadline.saturating_duration_since(started_at))`.
   - Retry budget preservation: On pre-submit transport failure, cooldown sleep checks `if tokio::time::Instant::now() + cooldown >= deadline`; if exceeded, sleeps until deadline and returns error without retrying. The second attempt reuses the original `deadline`.
3. **Frame-Independent Timer & Cleanup Reserve:**
   - In `execute_turn`:
     ```rust
     let cleanup_reserve = if effective_total_timeout >= Duration::from_secs(10) {
         Duration::from_secs(5).min(effective_total_timeout / 5)
     } else {
         Duration::ZERO
     };
     let work_deadline = deadline - cleanup_reserve;
     let timer = tokio::time::sleep_until(work_deadline);
     tokio::pin!(timer);
     ```
   - In event loop: `tokio::select!` matches `_ = &mut timer => { ... }` concurrently with incoming WebSocket frames. If the daemon sends endless `system/ping` frames or goes silent, the timer triggers unconditionally at `work_deadline`.
   - When timer triggers, it transmits `turn/interrupt`, awaits interrupt ACK or terminal frame within the reserved 5s window, cleans up `active_turns`, and returns `OmonError::Llm("turn exceeded total deadline...; turn/interrupt sent")`.

### Runtime Evidence & Regressions Executed
- **RED Baseline Audit:**
  - `U14-red.log` (exit 101: panicked at `RED run remains pending with a renewed setup budget (connect)`).
- **Independent Re-execution Log:** `.omo/evidence/hermes-parity-20260905/remote-control-verification-u14.log`
  - Command: `cargo test --test test_omo_backend interactive_deadline_bounds_all_protocol_phases -- --nocapture` -> **PASS** (1 passed, 0 failed, 0.52s).
  - All 8 parameterized phases executed and verified:
    1. `Phase: Connect`: Virtual clock advance +301s causes immediate deadline error.
    2. `Phase: Initialize`: Daemon sends ping and hangs; virtual advance +301s causes deadline error with 0 turn/start frames sent.
    3. `Phase: ThreadStart`: Daemon sends info frame and hangs; virtual advance +301s causes deadline error with 0 turn/start frames sent.
    4. `Phase: ThreadResume`: Daemon sends info frame and hangs; virtual advance +301s causes deadline error with 0 turn/start frames sent.
    5. `Phase: TurnStartAck`: Daemon acknowledges `turn/start` with ping; bounded 30s ACK cap triggers error at 31s.
    6. `Phase: Streaming`: Active turn streams deltas; virtual advance +301s causes deadline error and triggers bounded `turn/interrupt` cleanup to the peer.
    7. `Phase: RetryInitialize`: Spends 250s in first initialize, receives Connection Reset, waits 500ms cooldown, receives second initialize; virtual advance to start + 300s (49.5s) causes deadline error with 0 turn/start frames sent.
    8. `Phase: SmallerConfiguredCap`: Configured total timeout of 45s triggers deadline error after 46s.

---

## 3. Unit U15: No Accepted-Turn Resubmission (`S.F07`)

### Manifest Scenario & Requirements
- **S.F07 (Durable Dedup Across Reconnect):**
  - Previously, `run` caught mid-turn connection drops and invoked `run_once` again. Because the installed peer treats `clientUserMessageId` as correlation-only and allocates a new turn on every `turn/start`, an already accepted or mutating task executed twice.
  - Requirement: Retry only proven pre-accept failures; accepted and ambiguous pre-ACK disconnects must fail without resubmitting side effects; daemon side-effect counter must increment exactly once; positively pre-submit transport failures (dropped during initialize or resume) must still retry once within the deadline budget.

### Source Code Inspection
1. **Two-Phase Separation (`src/agent/omo_backend.rs`):**
   - Divided backend execution into:
     - `setup_turn(&self, session, deadline, timeout) -> Result<(WsStream, String)>`: Covers `connect_ws`, `do_initialize`, and `resolve_thread_id`. Any transport error here occurs strictly before `turn/start` serialization.
     - `execute_turn(&self, ws, thread_id, session, event, deadline, timeout) -> Result<()>`: Begins immediately with `ws.send(turn_start_request(...))`.
2. **Strict Retry Boundary in `run`:**
   - In `AgentBackend::run`:
     ```rust
     let setup_outcome = self.setup_turn(session, deadline, effective_total_timeout).await;
     let (ws, thread_id) = match setup_outcome {
         Ok(pair) => pair,
         Err(err) => {
             if is_retryable(&err) {
                 // Retry setup once within deadline budget
                 ...
                 self.setup_turn(session, deadline, effective_total_timeout).await?
             } else {
                 return Err(err);
             }
         }
     };
     self.execute_turn(ws, thread_id, session, event, deadline, effective_total_timeout).await
     ```
   - Errors returned from `execute_turn` are never caught or retried; they propagate immediately to the caller, preventing replay of in-flight or accepted work.

### Runtime Evidence & Regressions Executed
- **RED Baseline Audit:**
  - `U15-red.log` (exit 101: `assertion left == right failed: accepted turn must not be resubmitted after disconnect; left: 2, right: 1`).
- **Independent Re-execution Log:** `.omo/evidence/hermes-parity-20260905/remote-control-verification-u15.log`
  1. `cargo test --test test_omo_backend accepted_disconnect_does_not_resubmit_turn -- --exact --nocapture` -> **PASS** (1 passed, 0 failed, 0.01s).
     - Confirmed: Daemon ACKs `turn/start`, increments side-effect counter, and drops connection. `turn_start_count == 1`, `side_effect_counter == 1`, error returned, zero second connection attempt.
  2. `cargo test --test test_omo_backend pre_ack_disconnect_does_not_resubmit_turn -- --exact --nocapture` -> **PASS** (1 passed, 0 failed, 0.01s).
     - Confirmed: Daemon receives `turn/start`, increments side-effect counter, and drops connection immediately before sending ACK. `turn_start_count == 1`, `side_effect_counter == 1`, error returned without resubmission.
  3. `cargo test --test test_omo_backend pre_submit_disconnect_retries_and_succeeds -- --exact --nocapture` -> **PASS** (1 passed, 0 failed, 0.52s).
     - Confirmed: Pre-submit drop during `initialize` retries once after 500ms cooldown, reconnects (`connection_count == 2`), sends `turn/start` exactly once (`turn_start_count == 1`), and completes successfully.

---

## 4. Full Backend and Multiplexer Target Re-runs

All assigned backend and actor test targets were re-run to guarantee that no regressions were introduced to previous foundation units (such as U12 ACK correlation and quiet-clock proof):

| Target / Suite | Command | Result | Pass/Fail Count | Execution Time | Log Artifact |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Backend Integration Suite** | `cargo test --test test_omo_backend -- --nocapture` | **PASS** | 31 passed, 0 failed | 30.52s | `remote-control-verification-backend-target.log` |
| **Multiplexer Actor Target** | `cargo test --test test_multiplexer -- --nocapture` | **PASS** | 11 passed, 0 failed | 0.45s | `remote-control-verification-actor-target.log` |
| **Multiplexer Actor Unit Tests** | `cargo test --lib multiplexer::actor -- --nocapture` | **PASS** | 5 passed, 0 failed | 0.07s | `remote-control-verification-actor-target.log` |

### U12 Quiet-Clock & ACK Correlation Audit
During the full backend target execution, the U12 quiet-clock witness and correlation regressions passed cleanly:
```text
U12 quiet: processed tool-start; virtual_ms=1200; backend=pending; finals=0; result=held
U12 quiet: released correlated completion/digest/terminal; actual digest final=1; peers=0
U12 clock witness: processed tool-start +1200ms exceeds grace=1000ms and backend deadline=1000ms; deadline error; finals=0
test test_omo_backend_waits_for_final_message_after_tool_activity ... ok
```

---

## 5. Build, Diagnostics, Format, and Style Audits

1. **Workspace Compilation (`cargo build`):**
   - Command: `cargo build`
   - Result: Exit code 0 (`Finished dev profile [unoptimized + debuginfo] target(s)`).
   - Log: `remote-control-verification-build.log`.
2. **Git Diff Line & Whitespace Check:**
   - Command: `git diff --check src/agent/backend.rs src/agent/omo_backend.rs src/multiplexer/actor.rs tests/test_omo_backend.rs`
   - Result: Exit code 0 (zero trailing whitespace or conflict markers).
   - Log: `remote-control-verification-fmt.log`.
3. **Rustfmt Code Style Check:**
   - Command: `rustfmt --check --edition 2021 src/agent/backend.rs src/agent/omo_backend.rs src/multiplexer/actor.rs tests/test_omo_backend.rs`
   - Result: Exit code 0 (100% formatted to Rust edition 2021 standards).
   - Log: `remote-control-verification-fmt.log`.
4. **Clippy Diagnostics & Isolation of External Issues:**
   - Command: `cargo clippy --lib -- -D warnings`
   - Result: Exit code 101 due to pre-existing finding in `src/discord/adapter.rs:1149:1` (`clippy::too_many_arguments` in `decide_unauthorized_dm`, introduced by unit U10).
   - Isolation: `cargo clippy --tests` outputs zero warnings or errors for `src/agent/backend.rs`, `src/agent/omo_backend.rs`, `src/multiplexer/actor.rs`, and `tests/test_omo_backend.rs`. The remote control phase introduces zero clippy regressions.
   - Log: `remote-control-verification-clippy.log`.

---

## 6. Test Discipline, Determinism, and Worktree Scope Audit

- **Test Determinism & Time Control:**
  - All deadline tests utilize Tokio's virtual clock pause/advance mechanism (`tokio::time::pause()`, `tokio::time::advance()`, `tokio::time::resume()`).
  - Zero fixed sleeps, polling delays, or `yield_now()` loops exist in the test suites.
  - Event subscriptions precede actions using bounded `tokio::sync::mpsc` and `tokio::sync::oneshot` channels wrapped with `tokio::time::timeout(5s, ...)`.
- **Peer & Resource Cleanup:**
  - Every loopback WebSocket server fixture spawns with an explicit task handle and joins cleanly on both success and failure paths via `peer_handle.abort(); tokio::time::timeout(5s, peer_handle).await;`.
  - SQLite databases use isolated `:memory:` instances and clean up automatically.
- **Baseline User Config and Shared-Worktree Preservation:**
  - Zero modifications were made to `.env`, user credentials, global configuration, or unassigned unit files.
  - Changes in other units (such as calendar, admission, and migration units) were observed and preserved in their exact uncommitted states.

---

## Conclusion & Parity Assessment

Units U13, U14, and U15 satisfy all constituent requirements specified in the command manifest and `shared-design.md`:
1. **U13 PASS:** Stopping an active turn sends an authenticated `turn/interrupt` over the WebSocket connection, awaits peer acknowledgment or terminal status before reporting stop success, preserves the remote thread binding in SQLite, and reliably releases typing guards across all terminal paths without cross-session leakage.
2. **U14 PASS:** An absolute 300-second deadline anchors at backend entry and strictly bounds connect, initialize, resume, start, turn ACK, streaming, and retry phases, with a frame-independent select timer and reserved cleanup window for interrupt dispatch.
3. **U15 PASS:** Backend execution enforces strict phase separation, retrying only verified pre-submit handshake errors while ensuring accepted and ambiguous pre-ACK disconnects execute exactly one `turn/start` and side effect without automatic resubmission.

All verification logs and artifacts have been captured into `.omo/evidence/hermes-parity-20260905/`. The overall 85-unit Hermes parity milestone remains open for remaining admission, storage, and runtime units.
