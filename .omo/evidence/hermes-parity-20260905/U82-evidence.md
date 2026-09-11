# U82 Evidence: Preflight Configured Cron Delivery Transports

## Metadata
- Unit: U82 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS09 (Unconfigured Discord delivery was not preflight-blocked before agent execution; jobs specifying explicit Discord targets would trigger agent execution and LLM inference, only failing during post-execution delivery)
- Citations:
  - Live: `src/cron/store.rs`, `src/cron/scheduler.rs`, `src/main.rs`
  - Hermes / Upstream parity: `cron/scheduler_preflight.py:217-274`, `cron/scheduler.py:1438-1489`
- Date: 2026-09-07

## Implementation Summary
1. **Explicit Transport Target Detection (`src/cron/store.rs`)**:
   - Added `HermesJob::has_explicit_discord_destination(&self) -> bool` inspecting both `deliver` and `failure_deliver` for explicit `discord:<channel>` targets.
   - Bypasses non-external / local destinations (`local`, `origin`, `all`).
2. **Preflight Gating in Scheduler (`src/cron/scheduler.rs`)**:
   - In `CronScheduler::execute_job`: checks if `self.dispatcher.is_none()` and the job specifies an explicit Discord delivery target.
   - Fast-fails before calling `self.executor.execute(job)` with error `unconfigured delivery transport: discord for job {job_id}`, preventing wasted LLM token usage and execution.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::cron_preflight_blocks_unconfigured_discord`)**:
   - Job with explicit `deliver: "discord:12345"` when dispatcher is None:
     - RED captured: execution proceeded without preflight check (`called Result::unwrap_err() on an Ok value: Some("completed")`, exit 101).
     - GREEN verified: preflight rejected immediately with `unconfigured delivery transport: discord` and `runs == 0` (exit 0).
   - Job with `deliver: "local"`: bypasses preflight check and executes successfully (`runs == 1`).

## Verification
- Captured RED: `U82-red.log`, `U82-red.exit` (exit 101, panic: `called Result::unwrap_err() on an Ok value: Some("completed")`)
- Captured GREEN: `U82-green.log`, `U82-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
