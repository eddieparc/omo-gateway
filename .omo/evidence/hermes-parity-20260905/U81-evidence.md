# U81 Evidence: Persist Profile Scoped Cron Cursor Notes

## Metadata
- Unit: U81 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS08 (Durable per-job notepad cursor state was not carried into cron runs; `cron_notepads` store was absent, `CronTool` lacked notepad operations, and assembled cron prompts omitted durable operator cursor state)
- Citations:
  - Live: `migrations/0027_cron_notepads.sql`, `src/cron/store.rs`, `src/cron/executor.rs`, `src/tools/cron.rs`, `src/main.rs`
  - Hermes / Upstream parity: `cron/notepad.py:71-97, 118-158`, `cron/scheduler_prompt.py:229-234`
- Date: 2026-09-07

## Implementation Summary
1. **Durable Notepad State Persistence (`migrations/0027_cron_notepads.sql`)**:
   - Created table `cron_notepads (profile TEXT NOT NULL, job_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY (profile, job_id, key))`.
   - Created index `idx_cron_notepads_lookup ON cron_notepads(profile, job_id)`.
2. **Notepad Store API & Size Bounds (`src/cron/store.rs`)**:
   - Implemented `set_cron_notepad`, `get_cron_notepads`, `delete_cron_notepad`.
   - Enforced 16 KiB limit per value (`MAX_NOTEPAD_VALUE_BYTES = 16384`) and 64 KiB total per `(profile, job_id)` (`MAX_NOTEPAD_TOTAL_BYTES = 65536`).
   - Added `HermesJob::profile(&self) -> &str` resolving profile from `extra["profile"]`, origin, or fallback to `"default"`.
3. **Prompt Injection in Executor (`src/cron/executor.rs`)**:
   - In `AgentCronExecutor::execute`: loads `get_cron_notepads(&self.pool, profile, &hermes.id)`.
   - Formats non-empty notepad key-values into `[Notepad]\nkey: value\n...\n` block and prepends it to the assembled agent prompt.
4. **Tool Surface Support (`src/tools/cron.rs`)**:
   - Added `notepad`, `notepad_set`, `notepad_get`, `notepad_list`, `notepad_delete` actions to `CronTool`.
5. **Regression Verification (`src/main.rs::legacy::runner_tests::cron_notepad_is_durable_and_profile_scoped`)**:
   - Profile scoping: profile alpha receives its note (`cursor: 100`), profile beta receives its distinct note (`cursor: 200`).
   - Size enforcement: notes > 16 KiB rejected with Config error.
   - Prompt injection: verified agent execution receives `[Notepad]\ncursor: 100` and does not leak cross-profile notes.
   - RED captured: empty notepad returned prior to implementation (`assertion left == right failed: left: [], right: [("cursor", "100")]`, exit 101).
   - GREEN verified: exit 0.

## Verification
- Captured RED: `U81-red.log`, `U81-red.exit` (exit 101, panic: `Profile alpha must retain its scoped cursor note`)
- Captured GREEN: `U81-green.log`, `U81-green.exit` (exit 0)
- Single test execution: 1 passed in 0.05s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
