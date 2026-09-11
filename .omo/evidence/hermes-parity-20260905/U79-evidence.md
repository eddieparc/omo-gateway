# U79 Evidence: Route Cron Failures to Configured Lane

## Metadata
- Unit: U79 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS06 (Separate failure delivery routing was silently ignored; `failure_deliver` was not parsed or resolved, causing failed executions to either incorrectly blast the normal delivery destination or drop designated failure alerts)
- Citations:
  - Live: `src/cron/store.rs`, `src/cron/scheduler.rs`, `tests/test_voice_cron.rs`
  - Hermes / Upstream parity: `cron/scheduler_delivery.py:784-803`, `cron/scheduler.py:2748-2756`
- Date: 2026-09-07

## Implementation Summary
1. **Config Contract (`src/cron/store.rs`)**:
   - Added `pub failure_deliver: Option<Vec<String>>` to `HermesJob` using `deserialize_deliver`.
   - Added `HermesJob::failure_destinations(&self) -> Result<Vec<HermesOrigin>>` which resolves dedicated failure routes when `failure_deliver` is present, with fallback to default delivery destinations.
   - Preserved `local` suppression semantics: `failure_deliver: "local"` resolves to zero destinations, suppressing outbound failure messages to Discord.
2. **Failure Routing in Scheduler (`src/cron/scheduler.rs`)**:
   - In `CronScheduler::execute_job`: on task error (`Err(error)`), decodes `HermesJob` payload and queries `h.failure_destinations()`.
   - Dispatches failure notifications specifically and only to designated failure destinations.
3. **Regression Verification (`tests/test_voice_cron.rs::failure_delivery_uses_its_own_lane`)**:
   - Job 1: `deliver: "discord:42", failure_deliver: "local"`.
     - RED captured: error notification was sent to `discord:42` (`left: 1, right: 0`, exit 101).
     - GREEN verified: failure notification was suppressed by `failure_deliver: "local"` (0 messages dispatched).
   - Job 2: `deliver: "local", failure_deliver: "discord:43"`.
     - GREEN verified: error alert routed to `discord:43` (1 message dispatched).

## Verification
- Captured RED: `U79-red.log`, `U79-red.exit` (exit 101, panic: `failure_deliver:local must suppress error notification to discord:42`)
- Captured GREEN: `U79-green.log`, `U79-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
