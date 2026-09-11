# U66 Evidence: One Multiplexed Daemon For All Work

## Metadata
- Unit: U66 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: R.R14 (One daemon multiplexed invariant: default configuration spawned and managed a second isolated daemon process on port 19743 for cron alongside interactive on 19742, violating the required multiplexer architecture of one daemon supervising multiple distinct agent threads with per-agent workspace/memory)
- Citations:
  - Live: `src/agent/omo_config.rs`, `src/main.rs`, `src/dashboard_runtime.rs`
  - Hermes / Upstream parity: `gateway/status.py:1005-1016`
- Date: 2026-09-07

## Implementation Summary
1. **Unified Multiplexed Endpoint Default (`src/agent/omo_config.rs`)**:
   - Changed `CRON_APPSERVER_URL_DEFAULT` from `"ws://127.0.0.1:19743"` to `"ws://127.0.0.1:19742"`.
   - Both interactive and scheduled cron sessions now default to the same daemon supervisor endpoint while maintaining independent per-turn timeout ceilings.
   - Updated config test `test_cron_from_env_shares_multiplexed_daemon` to assert shared endpoint invariant.
2. **Single Daemon Supervisor Management (`src/main.rs`, `src/dashboard_runtime.rs`)**:
   - When `cron_omo_config.appserver_url == omo_config.appserver_url`, avoids creating a redundant second supervisor child process and reuses the single supervised daemon lifecycle.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::cron_and_interactive_share_one_daemon`)**:
   - RED captured: separate endpoints `19743` vs `19742` (`assertion left == right failed: left: "ws://127.0.0.1:19743", right: "ws://127.0.0.1:19742"`, exit 101).
   - GREEN verified: unified endpoint `19742` with preserved distinct timeout policies (exit 0).

## Verification
- Captured RED: `U66-red.log`, `U66-red.exit` (exit 101, panic: `Cron and interactive must share one daemon endpoint`)
- Captured GREEN: `U66-green.log`, `U66-green.exit` (exit 0)
- Single test execution: 1 passed in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
