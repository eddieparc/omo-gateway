# U48 Evidence: Truthful Complete CronTool Mutations Parity

## Metadata
- Unit: U48 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C09 (Production constructed scheduler-less tool where trigger only rewrote next_run_at, did nothing for paused jobs, and consumed scheduled one-shots; imported jobs allowed mutations that were subsequently clobbered by synchronizer; job creation dropped repeat/context_from/skills/no_agent/ack/timeout/model and ignored enabled:false; update always recomputed next_run_at even for name-only edits)
- Citations:
  - Live: `src/tools/cron.rs`, `src/main.rs`, `src/dashboard_runtime.rs`, `tests/test_cron_tool_parity.rs`
  - Hermes / Upstream parity: `cron/jobs.py:1333-1499`, `cron/scheduler_provider.py:85-112`
- Date: 2026-09-07

## Implementation Summary
1. **Dynamic Scheduler Binding (`src/tools/cron.rs`, `src/main.rs`, `src/dashboard_runtime.rs`)**:
   - Replaced static `Option<Arc<CronScheduler>>` with `Arc<parking_lot::RwLock<Option<Arc<CronScheduler>>>>`.
   - Exposed `CronTool::bind_scheduler(&self, scheduler: Arc<CronScheduler>)` invoked right after scheduler initialization in `src/main.rs` and `src/dashboard_runtime.rs`.
   - `trigger` action queries the live scheduler to spawn immediate claims even on paused jobs without advancing or consuming the recurring schedule.
2. **Imported Job Read-Only Mutation Gate (`check_imported_read_only`)**:
   - Guarded `delete`, `update`, `pause`, and `resume` operations.
   - If a job has authority `hermes_mirror` / `hermes_synced` or an `hermes:` prefix, rejects mutation with `OmonError::ToolExecution("cannot modify imported job (read-only)")`.
3. **Full Argument Preservation and `enabled:false` Support**:
   - In `CronTool::execute_with_context` (action: `add`/`create`): preserves `repeat`, `context_from`, `skills`, `skill`, `no_agent`, `ack_command`, `timeout_secs`, `model`, and `bot_id` into the payload.
   - Respects `enabled: false`: pauses scheduler registration and stores `enabled = false` with `next_run_at = None`.
4. **Timestamp Preservation on Name-Only Updates**:
   - In `CronTool` (action: `update`/`edit`): detects whether `expression` or `enabled` actually changed.
   - Name/prompt/description-only edits preserve the existing `next_run_at` without spurious rescheduling.
5. **Regression Verification (`tests/test_cron_tool_parity.rs::mutations_preserve_contract`)**:
   - Added job with `enabled:false`, `repeat: {times: 2}`, `context_from: ["abc"]`.
     - RED captured: job registered as enabled = true (exit 101).
     - GREEN verified: job registered as enabled = false and payload retains all fields (exit 0).
   - Triggered paused job: executes 1 manual run without consuming schedule.
   - Deleted imported job (`hermes_mirror`): rejected with read-only error.
   - Name-only update: preserves original `next_run_at` timestamp.

## Verification
- Captured RED: `U48-red.log`, `U48-red.exit` (exit 101, panic: Job must be registered as disabled when enabled:false is specified)
- Captured GREEN: `U48-green.log`, `U48-green.exit` (exit 0)
- Full `test_cron_tool_parity` suite: 1 passed in 0.23s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
