# U78 Evidence: Persist Change Monitor Gates Per Job Parity

## Metadata
- Unit: U78 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS05 (Change monitor fields `monitor_script`, `monitor_url` were captured but ignored; no monitor state hash/snapshot persistence; jobs executed the agent on every period regardless of whether monitored content changed)
- Citations:
  - Live: `src/cron/executor.rs`, `src/cron/store.rs`, `migrations/0025_cron_monitor_states.sql`, `src/main.rs`
  - Hermes / Upstream parity: `cron/monitor.py:107-167`, `cron/scheduler.py:1307-1344, 2032-2034`, `cron/jobs.py:1635-1653`
- Date: 2026-09-07

## Implementation Summary
1. **Persistent State Storage (`migrations/0025_cron_monitor_states.sql`)**:
   - Created table `cron_monitor_states (job_id TEXT PRIMARY KEY, last_hash TEXT NOT NULL, last_snapshot TEXT, updated_at TEXT NOT NULL)`.
2. **Hermes Job Specification & Mode Validation (`src/cron/store.rs`)**:
   - Added `monitor_script`, `monitor_url`, `monitor_state` to `HermesJob`.
   - In `HermesJob::validate`: rejects conflicting modes when both `monitor_script` and `monitor_url` are specified.
   - Enforced lifecycle safety on `monitor_script` via `check_gateway_lifecycle`.
3. **Execution Gating & Differential Triggering (`src/cron/executor.rs`)**:
   - In `AgentCronExecutor::execute`: evaluates monitor script/URL with timeout and captured piped stdout/stderr.
   - Computes SHA256 digest of monitor output and queries `cron_monitor_states` by `job_id`.
   - If previous hash matches current hash, suppresses agent invocation with `Ok(None)`.
   - When snapshot changes or on initial run, updates `cron_monitor_states` and appends `[Monitored change detected]` to prompt before dispatching to the backend.
4. **Regression Verification (`src/main.rs::legacy::runner_tests::cron_monitor_unchanged_skips_agent`)**:
   - Run 1 (initial snapshot): executes agent (`run_count == 1`).
   - Run 2 (unchanged content): agent skipped (`run_count == 1`).
     - RED captured: agent executed on unchanged state (`assertion left == right failed: Agent must be SKIPPED when monitor snapshot is unchanged, left: 2, right: 1`, exit 101).
     - GREEN verified: agent skipped when snapshot is unchanged (exit 0).
   - Run 3 (updated content): agent executes (`run_count == 2`).

## Verification
- Captured RED: `U78-red.log`, `U78-red.exit` (exit 101, panic: Agent must be SKIPPED when monitor snapshot is unchanged)
- Captured GREEN: `U78-green.log`, `U78-green.exit` (exit 0)
- Full bin test suite: 1 passed in 0.07s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
