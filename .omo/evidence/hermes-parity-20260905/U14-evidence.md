# U14 / S.F05 & R.R12: Absolute Deadline Bounding Across All Protocol Phases

## 1. Summary

- **Unit**: U14 ("Absolute deadline bounding across protocol phases")
- **Findings & Deadlock Diagnosis**:
  - Prior test deadlock: In `PhaseToBlock` tests, pinning `backend.run` and polling once followed by `rx.recv().await` left the lazy future unpolled, causing tests to hang indefinitely. In addition, the streaming phase relied on an uncoordinated `while backend.active_turns.lock().is_empty()` loop with `yield_now()`, and spawned peer tasks were aborted without joining or bounding cleanup.
  - Clock witness reconciliation: Frame-independent deadlines require that an expired witness (1s total budget, 1.2s virtual advance) immediately returns a deadline error without requiring subsequent frames, while inside-budget executions (10s total budget, 1.2s quiet interval) hold pending status during tool execution and deliver the final digest chunk on completion. Stale terminals arriving past the deadline budget are refused.
- **Files Modified**:
  - `src/agent/omo_backend.rs`: Absolute deadline Instant (`deadline = Instant::now() + effective_total_timeout`) anchored once at `run()` entry. Cap interactive sessions at 300s while preserving distinct cron policy. Propagate `deadline` across `connect_ws`, `do_initialize`, `resolve_thread_id` (`thread/resume` and `thread/start`), turn ACK (bounded <= 30s), streaming, and cleanup. Retries never reset the deadline. Reserve up to 5s cleanup budget inside global deadline for `turn/interrupt`.
  - `tests/test_omo_backend.rs`: Fixed test-harness deadlocks by replacing all witness waits with `tokio::select!` concurrently polling `&mut run` with bounded channel receives (`timeout(5s, ...)`). Emitted each phase name (`Phase: ...`) before execution. Eliminated the streaming poll/yield loop by routing stream start chunks via `dispatcher.turn_started_tx`. Bounded and joined peer cleanup on all paths. Cleaned dead duplicate code in `run_quiet_tool_interval`.

---

## 2. Captured RED Before Fixes

- **Harness Deadlock / Stale Setup Failure**:
  - **Command**: `cargo test --test test_omo_backend interactive_deadline_bounds_all_protocol_phases -- --nocapture`
  - **Exit Code**: `101`
  - **Log**: `U14-red.log`
  - **Failure**:
    ```text
    running 1 test

    thread 'interactive_deadline_bounds_all_protocol_phases' (8767833) panicked at tests/test_omo_backend.rs:2626:21:
    RED run remains pending with a renewed setup budget (connect)
    note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
    error: test failed, to rerun pass `--test test_omo_backend`
    test interactive_deadline_bounds_all_protocol_phases ... FAILED

    failures:
        interactive_deadline_bounds_all_protocol_phases

    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 27 filtered out; finished in 0.00s
    ```

---

## 3. Implementation Details

1. **Absolute Entry Instant**:
   - Anchored at `AgentBackend::run`:
     ```rust
     let effective_total_timeout = if is_cron_session {
         self.config.total_timeout
     } else {
         self.config.total_timeout.min(Duration::from_secs(300))
     };
     let deadline = tokio::time::Instant::now() + effective_total_timeout;
     ```
   - Maintained across `run_once` attempts without re-anchoring on automatic retries:
     ```rust
     if tokio::time::Instant::now() + cooldown >= deadline {
         tokio::time::sleep_until(deadline).await;
         return Err(OmonError::Llm(format!(
             "turn exceeded total deadline of {effective_total_timeout:?}"
         )));
     }
     tokio::time::sleep(cooldown).await;
     return self
         .run_once(session, event, deadline, effective_total_timeout)
         .await;
     ```

2. **Protocol Phase Deadline Bounds**:
   - `connect_ws(deadline)`: Iterative connection retry loop capped by `deadline`, with individual connection timeouts bounded by `deadline.min(now + connect_timeout)`.
   - `do_initialize(ws, deadline)`: Bounded by `deadline.min(now + request_timeout)`.
   - `resolve_thread_id(ws, session, deadline)`: Resume and start handshakes each bounded by `deadline.min(now + request_timeout)`.
   - Turn ACK: Bounded by `started_at + 30s.min(deadline - started_at)`.
   - Cleanup Reserve: `work_deadline = deadline - cleanup_reserve` (up to 5s reserve for >= 10s turns, or 20% of budget for smaller turns) allows `turn/interrupt` to be sent and acknowledged within the total deadline.

3. **Deadlock Elimination in Test Harness**:
   - Replaced unpolled `recv().await` calls with `tokio::select!` driving `&mut run` alongside bounded event receipts (`tokio::time::timeout(Duration::from_secs(5), rx.recv())`).
   - Removed `while backend.active_turns.lock().is_empty() { poll!; yield_now(); }` from `PhaseToBlock::Streaming`; instead, wired `stream_started_rx` to `CapturingDispatcher::turn_started_tx`, guaranteeing `active_turns` is populated as soon as the first stream delta is emitted.
   - Bounded and joined peer cleanup: `peer_handle.abort(); tokio::time::timeout(5s, peer_handle).await;` on both success and failure paths across all phases.
   - Emitted phase identifiers before each execution: `eprintln!("Phase: {phase:?}");`.

---

## 4. Verification Evidence

### Exact Phase Deadline Test
- **Command**: `cargo test --test test_omo_backend interactive_deadline_bounds_all_protocol_phases -- --nocapture`
- **Exit Code**: `0`
- **Log**: `U14-green.log`
- **Output**:
  ```text
  running 1 test
  Phase: Connect
  Phase: Initialize
  Phase: ThreadStart
  Phase: ThreadResume
  Phase: TurnStartAck
  Phase: Streaming
  Phase: RetryInitialize
  Phase: SmallerConfiguredCap
  test interactive_deadline_bounds_all_protocol_phases ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 27 filtered out; finished in 0.54s
  ```

### Full Backend Integration Test Suite
- **Command**: `cargo test --test test_omo_backend -- --nocapture`
- **Exit Code**: `0`
- **Log**: `U14-suite.log`
- **Output Summary**:
  ```text
  test result: ok. 28 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.52s
  ```

### Diagnostics, Build & Format Checks
- `rustfmt --check --edition 2021 src/agent/omo_backend.rs tests/test_omo_backend.rs`: Exit `0` (`U14-format.log`)
- `cargo check --tests`: Exit `0` (`U14-build.log`)
- `git diff --check src/agent/omo_backend.rs tests/test_omo_backend.rs`: Exit `0` (`U14-diff.log`)
