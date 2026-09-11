# U64 Evidence: Reversible Admission and Bounded Shutdown

## Metadata
- Unit: U64 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: R.R02 (Drain marker terminated watcher loop upon first positive detection; removing the drain marker could not restore service; multiplexer router did not gate new turn ingress during drain)
- Citations:
  - Live: `src/drain_control.rs`, `src/main.rs`, `src/multiplexer/router.rs`, `src/multiplexer/actor.rs`, `src/cron/scheduler.rs`, `src/dashboard_runtime.rs`
  - Hermes / Upstream parity: `gateway/run.py:4949-5024, 8997-9063`, `gateway/shutdown_watchdog.py:162-260`, `gateway/restart.py:24-25, 46-52`
- Date: 2026-09-07

## Implementation Summary
1. **Reversible Drain Watcher (`src/drain_control.rs`)**:
   - Refactored `DrainWatcher::spawn` loop so it persists throughout runtime rather than breaking upon the first detected drain request.
   - Updated `DrainWatcher::scan_at` to signal `drain_tx.send(true)` when `.drain_request.json` is present and valid, and `drain_tx.send(false)` when the marker file is removed.
2. **Multiplexer Ingress Drain Gating (`src/multiplexer/router.rs`)**:
   - Added `with_drain_receiver(watch::Receiver<bool>)` to `SessionMultiplexer`.
   - In `SessionMultiplexer::route`: checks drain state. When draining, rejects new turns with `gateway is draining; new turns are refused`, while preserving in-flight and already-accepted actor queue turns.
   - When drain is reversed (marker deleted), `route` immediately resumes accepting turns without requiring a process restart.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::drain_is_reversible_and_preserves_accepted_work`)**:
   - Enqueued `m1` and `m2`.
   - Marker placed -> `m3` rejected with drain error.
     - RED captured: accepted `m3` while marker was present (`called Result::unwrap_err() on an Ok value`, exit 101).
     - GREEN verified: rejected `m3` during drain (exit 0).
   - Marker removed -> `m4` accepted and routed successfully without restart.

## Verification
- Captured RED: `U64-red.log`, `U64-red.exit` (exit 101, panic: `called Result::unwrap_err() on an Ok value`)
- Captured GREEN: `U64-green.log`, `U64-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
