# U38 Evidence: Atomic Finite Occurrence Budget

## Metadata
- Unit: U38 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C03 (Only success incremented completed; recurring failures never exhausted finite runs; claims neither consumed budget nor checked limits before side effects; explicit trigger ignored limits)
- Citations:
  - Live: `src/cron/scheduler.rs`
  - Hermes / Upstream parity: `cron/jobs.py:1545-1570, 1610-1664, 1705-1731`, `cron/executions.py:156-181`
- Date: 2026-09-07

## Implementation Summary
1. **Failure Occurrence Counting and Finite Budget Exhaustion**:
   - In `CronScheduler::complete_failure`:
     - Now invokes `increment_repeat_completed(&mut payload)` and `should_disable_after(completed_count, times)`.
     - When a recurring job with a finite repeat limit reaches its occurrence count via failure (e.g. `times: 2`, 2 failures), the job is automatically disabled (`enabled = 0`, `next_run_at = NULL`) rather than rescheduling indefinitely in an infinite failure loop.
2. **Preclaim Limit Check**:
   - In `CronScheduler::claim_job`:
     - Before acquiring lease or creating a run record, reads job `payload_json` and checks `extract_repeat_info`.
     - If `should_disable_after(completed, times)` is true, skips claim (`Ok(None)`), preventing both timer-based and explicit manual triggers (`trigger_job`) from running already-exhausted jobs.
3. **Regression Verification (`tests/test_cron_schedule_parity.rs::finite_budget_survives_failure_and_restart`)**:
   - Tested job with `interval:1m`, `repeat: {times:2, completed:0}` with failing executor:
     - RED captured: old code left `completed == 0` after first failure (exit 101).
     - GREEN verified: first failure increments `completed` to 1; second failure increments to 2 and disables job (`enabled = false`).
     - Subsequent `trigger_job` attempt on exhausted job is rejected (`false`).

## Verification
- Captured RED: `U38-red.log`, `U38-red.exit` (exit 101, assertion failed: First failure must increment completed count toward finite budget, left: 0, right: 1)
- Captured GREEN: `U38-green.log`, `U38-green.exit` (exit 0)
- Full `test_cron_schedule_parity` suite: 2 passed in 0.12s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
