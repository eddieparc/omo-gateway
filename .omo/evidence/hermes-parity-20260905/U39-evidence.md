# U39 Evidence: Profile-Scoped Persistent Cron Results

## Metadata
- Unit: U39 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C04 (Script-only/local results had no output persistence; DB lookup used unscoped suffix LIKE without profile provenance; wildcards %/_ in job_id leaked context across jobs; global disk fallback leaked across profiles)
- Citations:
  - Live: `src/cron/scheduler.rs`, `src/main.rs`, `migrations/0021_cron_outputs.sql`
  - Hermes / Upstream parity: `cron/scheduler.py:2389-2434, 3801`, `cron/jobs.py:2208`
- Date: 2026-09-07

## Implementation Summary
1. **Dedicated `cron_outputs` Persistence Table (`migrations/0021_cron_outputs.sql`)**:
   - Created table `cron_outputs (profile TEXT NOT NULL DEFAULT '', job_id TEXT NOT NULL, run_id TEXT NOT NULL, output TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (profile, job_id, run_id))`.
   - Indexed on `(profile, job_id, created_at DESC)` for immediate O(1) indexed lookups.
2. **Profile Provenance and Output Capture**:
   - Implemented `CronJob::profile(&self) -> String` extracting profile from job payload (`profile` field) or session key (`discord:<bot_id>:...`).
   - In `CronScheduler::complete_success`: when execution succeeds with output, commits the output record to `cron_outputs` keyed by exact profile, job ID, and run ID.
3. **Exact-Match Profile Scoping and Wildcard Immunity (`resolve_predecessor_output`)**:
   - Updated `resolve_predecessor_output(pool, profile, job_id)` to query `cron_outputs WHERE profile = ? AND job_id = ? ORDER BY created_at DESC LIMIT 1`.
   - Replaced unscoped wildcard `LIKE '%cron:{job_id}'` with exact SQL parameter binding. Disallows wildcard injection (`%` or `_`) from matching arbitrary outputs.
   - Preserves profile isolation: jobs in profile A never leak output into downstream jobs in profile B.
4. **Regression Verification (`tests/test_cron_schedule_parity.rs::predecessor_is_profile_scoped`)**:
   - Profiles A and B each execute jobs with distinct outputs.
   - RED captured: old code returned `None` for predecessor output due to missing persistence / scoping (exit 101).
   - GREEN verified: Profile A resolves exact "Output A"; Profile B resolves exact "Output B".
   - Cross-profile query and wildcard `%` query strictly return `None`.

## Verification
- Captured RED: `U39-red.log`, `U39-red.exit` (exit 101, panic: assertion failed: Profile A predecessor must resolve exact Output A, left: None, right: Some("Output A"))
- Captured GREEN: `U39-green.log`, `U39-green.exit` (exit 0)
- Full `test_cron_schedule_parity` suite: 3 passed in 0.13s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
