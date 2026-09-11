# U49 Evidence: Cron Runs and Status Tool Surface Parity

## Metadata
- Unit: U49 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C17 (CronTool previously accepted neither runs nor status; no ticker heartbeat, active claim state, or attempt history exposed on the agent tool surface)
- Citations:
  - Live: `src/tools/cron.rs`, `src/cron/scheduler.rs`, `tests/test_cron_tool_parity.rs`
  - Hermes / Upstream parity: `cron/executions.py:184-228`, `cron/jobs.py:809-855`
- Date: 2026-09-07

## Implementation Summary
1. **Tool Actions for Execution History and Status (`src/tools/cron.rs`)**:
   - Implemented `action: "runs"`: queries `cron_runs` table for a specific `job_id` (or all recent runs), returning structured attempt records including `run_id`, `job_id`, `claim_token`, `started_at`, `completed_at`, `status`, `attempt`, and `error`.
   - Implemented `action: "status"`: aggregates global scheduler metrics including `total_jobs`, `enabled_jobs`, `paused_jobs`, and live ticker health (`running` boolean, active execution count, health status).
   - Updated `CronTool::input_schema` to expose `runs` and `status` in the action enum and description.
2. **Scheduler State Introspection (`src/cron/scheduler.rs`)**:
   - Added `CronScheduler::is_running(&self) -> bool` checking whether the background poll loop handle is active.
   - Added `CronScheduler::active_executions_count(&self) -> usize` returning the current in-flight claim count.
3. **Regression Verification (`tests/test_cron_tool_parity.rs::runs_and_status_are_exposed`)**:
   - `Tool::execute` with `{"action": "runs", "id": "j"}`:
     - RED captured: rejected with error `"unknown action: runs"` (exit 101).
     - GREEN verified: returns structured array with seeded attempt (`run_id: "r1"`, `status: "succeeded"`) (exit 0).
   - `Tool::execute` with `{"action": "status"}`: returns live running state, job counts, and ticker health.

## Verification
- Captured RED: `U49-red.log`, `U49-red.exit` (exit 101, panic: action 'runs' must succeed: ToolExecution("unknown action: runs"))
- Captured GREEN: `U49-green.log`, `U49-green.exit` (exit 0)
- Full `test_cron_tool_parity` suite: 2 passed in 0.23s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
