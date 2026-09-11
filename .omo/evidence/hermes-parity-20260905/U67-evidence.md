# U67 Evidence: Attach Dashboard To Actual Live Runtime

## Metadata
- Unit: U67 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: R.R08 (Attached dashboard was running a disconnected standalone instance with separate multiplexer, approvals, scheduler, and web-only dispatcher; could not stop active Discord actor turns, could not resolve approvals, and manual cron triggers bypassed primary egress)
- Citations:
  - Live: `src/entry.rs`, `src/main.rs`, `src/dashboard_runtime.rs`, `src/dashboard.rs`
  - Hermes / Upstream parity: `gateway/status.py:966-1020, 1125-1151`
- Date: 2026-09-07

## Implementation Summary
1. **Attached Runtime Wiring (`src/dashboard.rs`, `src/dashboard_runtime.rs`)**:
   - Added `/api/sessions/{id}/stop` endpoint to `dashboard.rs` delegating to `state.multiplexer.stop(&key)`.
   - Wired attached dashboard state to share the live `SessionMultiplexer`, live `SmartApprovalGuard`, live `CronScheduler`, and shared primary outbound dispatcher.
   - Preserves approval resolution parity: `/api/approvals/{id}/resolve` resolves approvals on the shared live guard.
   - Preserves cron execution parity: `/api/cron/jobs/{id}/trigger` executes through the live scheduler, dispatching output directly to primary egress.
2. **Regression Verification (`src/entry.rs::tests::attached_dashboard_controls_live_runtime`)**:
   - Live turn interruption: active in-flight session turn stopped via `/api/sessions/{id}/stop` (`stopped: true`).
   - Live approval resolution: pending approval resolved via `/api/approvals/{id}/resolve` (`decision: Once`).
   - Manual cron trigger: triggered scheduled job reaches recording primary egress on channel 999.

## Verification
- Captured RED: `U67-red.log`, `U67-red.exit` (exit 101)
- Captured GREEN: `U67-green.log`, `U67-green.exit` (exit 0)
- Single test execution: 1 passed in 0.03s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
