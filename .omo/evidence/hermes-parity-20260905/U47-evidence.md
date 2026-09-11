# U47 - Bad-Job Isolation and Explicit Rearming (CR.C16)

Resumed from interrupted session `01a070bb-40f8-7c3e-a02e-e802d250b826` following verified completion of U46 (`test_cron_schedule_parity`).
This unit addresses finding **CR.C16** and plugs the full-validation hole in one-shot schedule parsing:
- **Store & job isolation**: In `HermesStoreSynchronizer::sync_at`, a malformed cron schedule expression or a corrupted store/profile previously caused `sync_at` to abort with `Err`, preventing valid jobs from booting or being registered. Now bad profiles and invalid schedule expressions are isolated with warning diagnostics, preserving healthy store boot.
- **One-shot validation hole plugged**: Previously, `Err(_) if expression.starts_with("once:") => None` tolerated *all* once errors, including `once:not-a-date`, not only valid-but-overdue timestamps. Malformed once schedules could still import as enabled/NULL-next rows or preserve a supplied `next_run_at`. Now `sync_at` explicitly validates `parse_timestamp(timestamp)` for `once:` expressions. If unparseable, the job is isolated with a diagnostic (never imported), while valid overdue timestamps preserve the existing overdue policy (`computed_next = None`).
- **Consistent per-job timezone evaluation**: In `sync_at`, `next_run_tz` now evaluates using `job.schedule.timezone.as_deref().or(timezone.as_deref())`. If a job defines its own timezone, recurrence is evaluated in that timezone rather than evaluating in the store profile timezone while storing the job timezone.
- **Operator once rearm**: In the SQL `ON CONFLICT(id) DO UPDATE SET` logic, an unconditional check for `cron_jobs.expression LIKE 'once:%' AND cron_jobs.next_run_at IS NULL` previously preserved the disabled NULL state before checking whether the operator updated the schedule to a new future time. Now unchanged terminal once jobs remain disabled, while operator changes to a future `run_at` rearm the job (`enabled = true`, `next_run_at = Some(future_time)`).
- **Finite repeat limit increase**: The SQL check previously compared `completed` against the stale `cron_jobs.payload_json, '$.repeat.times'`. Now it evaluates against the updated limit in `excluded.payload_json`. When the operator increases `repeat.times`, the job is re-enabled and its schedule resumed while preserving the prior completed count.
- **Runtime execution & delivery status**: When jobs complete or fail, `CronScheduler` now persists `last_status`, `last_run_at`, `last_error`, and `last_delivery_error` into `cron_jobs.payload_json` within the completion transaction. Furthermore, resynchronization preserves existing runtime status when `jobs.json` does not provide newer values.

## Scope and Constraints Honored

- **WRITE ONLY**: `src/cron/store.rs`, `src/cron/scheduler.rs`, `tests/test_cron_boundary_parity.rs`, and evidence artifacts (`U47-*`).
- No modifications to `main.rs`, tools, dashboard, migration, backend, storage/db, or other tests.
- Preserved all handoff dirty changes from the interrupted parent session and prior units (including completed U46 timezone support and dependencies).
- Zero dependency changes in this unit.
- No fixed sleeps or polling in tests; injected time at the existing `with_clock` scheduling boundary and awaited explicit channels.

## Binding Original Scenario (CR.C16)

```
| C16 | Isolate invalid records/profiles, preserve healthy boot; distinguish operator rearm from unchanged terminal import; expose current run/delivery state. `cargo test --test test_cron_boundary_parity sync_isolates_bad_jobs_and_rearms_changed_once` | One valid job plus malformed schedule; RED sync/boot error -> GREEN valid job available + invalid diagnostic. Completed once changed to future run_at: RED disabled -> GREEN enabled future next time. Increase finite limit likewise; fetched status reflects latest execution. |
```

### Exact Command
```bash
cargo test --test test_cron_boundary_parity sync_isolates_bad_jobs_and_rearms_changed_once -- --exact --nocapture
```

### Nonzero Behavioral RED -> GREEN Evidence

1. **Initial RED Phase** (`U47-resume-red.log`, exit 101):
   With `tests/test_cron_boundary_parity.rs` registered against the unmodified production code, `sync.sync_at(initial_time).await` failed with:
   ```
   thread 'sync_isolates_bad_jobs_and_rearms_changed_once' panicked at tests/test_cron_boundary_parity.rs:132:10:
   sync must isolate bad jobs and bad profiles without aborting healthy import: Config("invalid cron expression `garbage`: garbage\n^\nThe 'Seconds' field does not support using names. 'garbage' specified.")
   ```
   This confirmed that a malformed schedule aborted the entire sync and prevented valid jobs in the store from loading.

2. **Once Validation Hole RED Phase** (`U47-resume-red-once-validation.log`, exit 101):
   When testing `once:not-a-date` schedules (with and without `next_run_at`), the unmodified once-error branch tolerated parse failures:
   ```
   thread 'sync_isolates_bad_jobs_and_rearms_changed_once' panicked at tests/test_cron_boundary_parity.rs:186:5:
   assertion `left == right` failed: must import all valid jobs from healthy stores while isolating bad jobs and profiles
     left: 7
    right: 5
   ```
   Both `bad-once-no-next` and `bad-once-with-next` were imported (7 total rows) instead of being isolated (expected 5 valid jobs).

3. **GREEN Phase** (`U47-resume-green.log`, exit 0):
   With the minimal scoped patch applied:
   - **Isolation**: Corrupt profile (`{ not valid json syntax `), malformed cron expression (`expr: "garbage"`), and malformed once timestamps (`run_at: "not-a-date"`, both with and without `next_run_at`) are isolated with warning diagnostics. The 5 valid jobs import successfully (`imported == 5`).
   - **Timezone precedence**: `job-with-own-timezone` (`America/New_York`) evaluates noon EDT as `16:00:00Z` upcoming on Sep 5, even though the profile timezone (`Asia/Seoul`) has already passed noon (`03:00:00Z`).
   - **Overdue once preservation**: `valid-overdue-once` imports with `next_run_at = None` (overdue policy preserved).
   - **Execution & Run State**: Due jobs execute; `once-rearm` and `finite-repeat` become disabled with `next_run_at = None`. Their fetched payloads reflect `last_status = "succeeded"` and a populated `last_run_at` timestamp.
   - **Unchanged Terminal Preservation**: Resyncing with unchanged `jobs.json` preserves the disabled state (`enabled = false`, `next_run_at = None`) for both terminal jobs.
   - **Once Rearming**: Operator changes `once-rearm`'s `run_at` to a future timestamp (`2026-09-05T12:00:00Z`). Resyncing rearms the job to `enabled = true` with `next_run_at = Some(2026-09-05T12:00:00Z)`.
   - **Finite Limit Increase**: Operator increases `finite-repeat`'s `repeat.times` from 1 to 2. Resyncing rearms the job to `enabled = true` with a newly computed `next_run_at`, while retaining `repeat.completed = 1`.
   - **Execution Failure & Delivery Failure Status**:
     - Simulating execution failure sets `last_status = "failed"` and `last_error` to the failure reason.
     - Simulating delivery failure on `valid-daily` sets `last_status = "failed"` and populates `last_delivery_error` with `"delivery error: discord destination unreachable"`.

Output:
```
running 1 test
test sync_isolates_bad_jobs_and_rearms_changed_once ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```

## Reconciling Pre-existing Tests

In `src/cron/store.rs::tests::imports_timezone_and_rejects_invalid_schedule`:
- Previously (in U46), the test asserted `assert!(result.is_err())` when importing a store with an invalid schedule expression (`garbage`).
- In CR.C16, bad jobs are isolated so sync does not error out.
- Reconciled deliberately with stronger observable semantics: the invalid job is rejected from insertion (`assert_eq!(result.unwrap(), 0)` and `assert!(rows.is_empty())`), ensuring it never becomes an enabled NULL row and does not corrupt store boot.

## Minimal Scoped Implementation

1. `src/cron/store.rs`:
   - In `sync_at`:
     - Wrapped `store.load().await` in a `match`: on error, logs a warning and skips the store without deleting existing rows for that store.
     - In job loop: empty IDs, invalid expressions (`job.expression()`), and invalid timestamps are logged as warnings and skipped (`continue`).
     - Added strict check for one-shot schedules: `if let Some(timestamp) = expression.strip_prefix("once:") { if parse_timestamp(timestamp).is_err() { tracing::warn!(...); continue; } }`. Malformed timestamps never import.
     - Evaluated `effective_timezone = job.schedule.timezone.as_deref().or(timezone.as_deref())` so per-job timezone takes precedence over store profile timezone during `next_run_tz`.
     - In the `ON CONFLICT(id) DO UPDATE SET` query:
       - `payload_json`: preserves `repeat.completed` and copies over `last_status`, `last_run_at`, `last_error`, `last_delivery_error` from `cron_jobs.payload_json` when `cron_jobs` has runtime execution state.
       - `enabled`: checks for once job unchanged terminal import (`cron_jobs.expression = excluded.expression AND cron_jobs.next_run_at IS NULL`) and finite repeat exhaustion against `excluded.payload_json`'s `repeat.times`. When operator updates the once schedule or increases finite repeat times, it falls through to `excluded.enabled`.
       - `next_run_at`: preserves NULL for unchanged once terminal import and exhausted repeat times; takes `excluded.next_run_at` on changed expressions or changed repeat limits; preserves `cron_jobs.next_run_at` for advanced recurring schedules.
   - Reconciled unit test `imports_timezone_and_rejects_invalid_schedule` with stronger assertion of zero imported rows without aborting.

2. `src/cron/scheduler.rs`:
   - Added accessor methods on `CronJob`: `last_status()`, `last_run_at()`, `last_error()`, `last_delivery_error()`.
   - In `complete_success`: updates `payload_json` with `last_status = "succeeded"`, `last_run_at = now.to_rfc3339()`, `last_error = null`, `last_delivery_error = null`.
   - In `complete_failure`: updates `payload_json` with `last_status = "failed"`, `last_run_at = now.to_rfc3339()`, `last_error = error.to_string()`, and conditionally sets `last_delivery_error` if the error was delivery-related. Updates `cron_jobs` table with `payload_json`.
   - In `execute_job`: wraps `self.deliver` errors with `"delivery error: {err}"` to distinguish delivery errors from executor errors.

3. `tests/test_cron_boundary_parity.rs`:
   - Parity integration test covering all facets of CR.C16 end-to-end: bad-job/bad-profile isolation, rejection of malformed once timestamps with and without `next_run_at`, preservation of overdue once policy, per-job timezone precedence, run status update, unchanged terminal preservation, once rearm, finite limit increase rearm, execution failure status, and delivery failure status.

## Verification Chronology

1. Authored `tests/test_cron_boundary_parity.rs` with exact test name `sync_isolates_bad_jobs_and_rearms_changed_once`.
2. Captured initial behavioral RED (`U47-resume-red.log`, exit 101).
3. Added test cases for `once:not-a-date` with/without `next_run_at`, valid overdue once, and per-job timezone priority; captured validation-hole RED proof (`U47-resume-red-once-validation.log`, exit 101).
4. Applied minimal scoped patch to `src/cron/store.rs` and `src/cron/scheduler.rs`.
5. Verified cargo build and diagnostics: 0 errors (`U47-resume-build.log`).
6. Captured identical test GREEN (`U47-resume-green.log`, exit 0).
7. Regression test runs:
   - `cargo test --lib cron::` -> 39 passed, 0 failed (`U47-resume-adjacent-cron.log`).
   - `cargo test --test test_cron_schedule_parity` -> 1 passed (U46 preserved).
   - `cargo test --test test_voice_cron` -> 9 passed.
8. Formatted changed files with `rustfmt --edition 2024`.

## Files Changed

- `src/cron/store.rs`
- `src/cron/scheduler.rs`
- `tests/test_cron_boundary_parity.rs`
- `.omo/evidence/hermes-parity-20260905/U47-commands.json`
- `.omo/evidence/hermes-parity-20260905/U47-evidence.md`
- `.omo/evidence/hermes-parity-20260905/U47-resume-red.log`
- `.omo/evidence/hermes-parity-20260905/U47-resume-red-once-validation.log`
- `.omo/evidence/hermes-parity-20260905/U47-resume-green.log`
- `.omo/evidence/hermes-parity-20260905/U47-resume-adjacent-cron.log`
- `.omo/evidence/hermes-parity-20260905/U47-resume-build.log`
