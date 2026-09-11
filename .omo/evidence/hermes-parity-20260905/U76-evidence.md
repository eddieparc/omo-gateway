# U76 - Retire Overdue Unclaimed One-Shot Jobs (UP.DS03)

Resumed from interrupted session `01a070bb-40f8-7c3e-a02e-e802d250b826` following verified completion of U46 and U47.
This unit implements and proves **U76**, covering **UP.DS03** only, and includes atomic CAS hardening against competing manual claims and operator rearms.

## Problem and Defect Mechanism

- **Upstream Change:** `cron/jobs.py:2827-2839,2921-2923` retires an unclaimed one-shot more than 120s late, retaining a terminal missed-run diagnostic. This supersedes cron-audit C20's installed-source conclusion, which correctly noted that the older installed reference did not categorically discard already-persisted overdue one-shots. Creation-time grace remains intact; the new upstream policy adds automatic dispatch-time retirement.
- **Rust Defect Proof:** In `src/cron/scheduler.rs`, `run_due_jobs` previously queried all jobs `WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= now` with no lower age bound before claiming. `store.rs` preserves a supplied `next_run_at` timestamp upon import. When the gateway rebooted or loaded a pre-existing store with stale one-shot timestamps (e.g. reminders or one-off scripts hours or days late), the scheduler would claim and dispatch them immediately.
- **Minimal Boundary:** An automatic dispatch-time grace gate in `claim_job` (when `require_due = true`) enforcing that one-shot jobs (`expression.starts_with("once:")`) older than `ONESHOT_GRACE_DURATION` (120s) with no running lease are retired as missed:
  - `enabled` is set to `0`.
  - `next_run_at` is set to `NULL`.
  - `payload_json` records `last_status = "missed"`, `last_run_at = now.to_rfc3339()`, and `last_error = "missed: overdue unclaimed one-shot past grace"`.
  - The claim is refused (`Ok(None)`), returning claim count 0 to the due-scan loop and making zero calls to the task executor.
  - Boundary condition: exactly 120s late (`now - next_run_at == 120s`) claims once and executes.
  - Overdue recurring jobs (e.g. `interval:...` or cron expressions) remain eligible regardless of age.
  - Manual triggers (`scheduler.trigger(...)`, where `require_due = false`) bypass the due grace gate and execute the requested job.
  - Explicit rearm semantics (operator changing schedule or resuming with future `next_run_at`) are fully preserved.

## Atomic CAS Hardening Against Competing Claims and Rearms

To guarantee concurrency safety during scheduler due scans:
1. **Atomic CAS Update:** In `retire_overdue_oneshot`, retirement is executed in a single atomic SQL `UPDATE`:
   ```sql
   UPDATE cron_jobs
   SET enabled = 0, next_run_at = NULL, payload_json = ?, updated_at = ?
   WHERE id = ?
     AND enabled = 1
     AND expression = ?
     AND next_run_at = ?
     AND payload_json = ?
     AND NOT EXISTS (
         SELECT 1 FROM cron_runs
         WHERE job_id = ? AND status = 'running'
     )
   ```
   This ensures:
   - If a concurrent manual claim has taken ownership (`status = 'running'`), retirement CAS matches 0 rows and does not disable the job while executing.
   - If an operator concurrently rearms or modifies the job (changing `next_run_at` or `payload_json`), retirement CAS matches 0 rows and does not clobber the updated schedule or metadata.
2. **Affected-Row Verdict:** `retire_overdue_oneshot` checks `result.rows_affected() > 0` before emitting an info log or waking the scheduler, returning an explicit `Result<bool>` verdict.
3. **Payload Integrity:** Replaced fallback `{}` on JSON parsing with typed `OmonError::Config` to prevent corrupting unparsed payload metadata.
4. **Due Scan Gate:** In `claim_job`, when evaluating an overdue one-shot past grace (`require_due == true && overdue > grace`), the scheduler invokes `retire_overdue_oneshot` and returns `Ok(None)`. If CAS succeeded, the job is retired as missed; if CAS failed (due to competing claim or rearm), the automatic due scan does not claim the past-grace job.

## Scope and Constraints Honored

- **WRITE ONLY**: `src/cron/scheduler.rs`, `tests/test_voice_cron.rs`, and evidence artifacts (`U76-*`).
- Zero modifications to `main.rs`, tools, dashboard, migration, backend, storage/db, or other tests.
- Preserved all handoff dirty changes from earlier units (including U46 timezone support and U47 bad-job isolation/rearming).
- Zero dependency additions or modifications.
- Strict test discipline: zero `thread::sleep`, zero polling loops; deterministic fixed clock injection via `with_clock` and bounded channels/notify gates via `with_pre_retire_gate`.

## Binding Original Scenario (UP.DS03)

```
### DS03 - Persisted one-shots older than grace still execute (runtime defect; new upstream policy)

- Upstream change: cron/jobs.py:2827-2839,2921-2923 now retires an unclaimed one-shot more than 120s late, retaining a missed-run diagnostic. This supersedes cron-audit C20's installed-source conclusion, which correctly noted that the older reference did not categorically discard already-persisted overdue one-shots. Do not retroactively call that report wrong.
- Rust proof: src/cron/scheduler.rs:698-714,760-795 selects every enabled next_run_at <= now, with no lower age bound before claim. Import preserves a supplied old timestamp at src/cron/store.rs:426-431. Creation-time grace does not protect restart dispatch. Stale time-sensitive reminders/scripts can execute hours late.
- Minimal boundary: due-scan/claim-time grace gate for automatic one-shots, with retained terminal missed state; preserve explicit rearm/manual-run semantics and do not disable recurring catch-up. Coordinate existing C03 claim accounting, but this is a distinct eligibility predicate.
- RED/GREEN: cargo test --test test_voice_cron persisted_oneshot_past_grace_is_retired. Seed a real DB row directly with once schedule and next_run=T-121s, no running claim; run due scan at T. RED claim count=1/backend execution; GREEN claim count=0, disabled/no next run and retained missed record. Boundary T-120s must claim once; overdue recurring job remains eligible. Use fixed clock, not Utc timing margins.
- Local surface: import an enabled old once job from a temp Hermes store and run the actual scheduler dispatch pass; capturing executor must receive zero calls and dashboard-visible job/run state must distinguish missed from succeeded. Existing active_lease_blocks_duplicate_claims_until_success_commits_one_shot covers a one-second-old row, not expiration.
```

## Nonzero Behavioral RED -> GREEN Chronology

### 1. Primary Boundary Test: RED Phase (`U76-resume-red.log`, exit 101)

The test `persisted_oneshot_past_grace_is_retired` was registered in `tests/test_voice_cron.rs` before production changes. Seeding an overdue one-shot with `next_run = T - 121s`, boundary one-shot `T - 120s`, and overdue recurring `T - 300s` at fixed time `T`:

```
    Blocking waiting for file lock on build directory
   Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 17.26s
     Running tests/test_voice_cron.rs (target/debug/deps/test_voice_cron-c8c51a9e617fa7d9)

running 1 test
test persisted_oneshot_past_grace_is_retired ... FAILED

failures:

failures:
    persisted_oneshot_past_grace_is_retired

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.04s

thread 'persisted_oneshot_past_grace_is_retired' (8516061) panicked at tests/test_voice_cron.rs:548:5:
assertion `left == right` failed: overdue one-shot past grace (T-121s) must not be claimed; boundary (T-120s) and recurring must be claimed
  left: 3
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
error: test failed, to rerun pass `--test test_voice_cron`
```

### 2. Primary Boundary Test: GREEN Phase (`U76-resume-green.log`, exit 0)

Running the identical command after the grace gate was added to `claim_job`:

```
running 1 test
test persisted_oneshot_past_grace_is_retired ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.03s
```

### 3. Race & CAS Hardening Test: RED Phase (`U76-resume-race-red.log`, exit 101)

The test `overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm` was registered in `tests/test_voice_cron.rs`. Prior to enforcing that overdue one-shots past grace always return `Ok(None)` from `claim_job` when competing claims take ownership, the scan claimed the job a second time:

```
running 1 test
test overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm ... FAILED

failures:

failures:
    overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.04s

thread 'overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm' (8931561) panicked at tests/test_voice_cron.rs:782:5:
assertion `left == right` failed: due scan must claim 0 after manual trigger took ownership
  left: 1
 right: 0
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
error: test failed, to rerun pass `--test test_voice_cron`
```

### 4. Race & CAS Hardening Test: GREEN Phase (`U76-resume-race-green.log`, exit 0)

With the CAS retirement and `claim_job` past-grace return `Ok(None)` in place:

```
running 1 test
test overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.23s
```

## Local Surface Verification

The test `persisted_oneshot_past_grace_is_retired` includes the full real local surface scenario:
1. **Hermes store import:** imports an enabled once job with `next_run_at = 2026-09-06T11:57:00Z` (180s old at `T = 12:00:00Z`) alongside a fresh once job (`2026-09-06T12:10:00Z`) from a temporary Hermes store using `HermesStoreSynchronizer`.
2. **Scheduler dispatch pass:** `scheduler.run_due_jobs().await` executes.
3. **Capturing executor verification:** The capturing channel receives zero calls for `hermes:surface:old-once` (`rx.try_recv().is_err()` is confirmed).
4. **Dashboard-visible state distinction:**
   - For `old-once`: `enabled == false`, `next_run_at == None`, `last_status == Some("missed")`.
   - For `fresh-once`: when triggered, executes successfully, yielding `last_status == Some("succeeded")`.
   - Job/run state clearly distinguishes missed from succeeded.

## Adjacent Test Suite Regression Results

All adjacent test suites and compiler checks passed without error (`U76-resume-adjacent-cron.log`):

1. **Voice Cron Suite** (`cargo test --test test_voice_cron`):
   ```
   running 11 tests
   test audio_frame_buffer_serializes_without_losing_pcm_samples ... ok
   test dead_process_expired_lease_is_reclaimed_by_scheduler ... ok
   test failed_one_shot_due_run_disables_job_and_records_failed_lease ... ok
   test failed_interval_advances_next_run_and_applies_backoff ... ok
   test live_process_expired_lease_is_not_reclaimed_and_completes_successfully ... ok
   test background_scheduler_executes_due_interval_and_notifies_channel ... ok
   test active_lease_blocks_duplicate_claims_until_success_commits_one_shot ... ok
   test cron_scheduler_registers_triggers_and_manages_jobs ... ok
   test overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm ... ok
   test repeated_failures_scale_backoff_deterministically ... ok
   test persisted_oneshot_past_grace_is_retired ... ok

   test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s
   ```

2. **Boundary Parity Suite** (`cargo test --test test_cron_boundary_parity`):
   ```
   running 1 test
   test sync_isolates_bad_jobs_and_rearms_changed_once ... ok

   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s
   ```

3. **Schedule Parity Suite** (`cargo test --test test_cron_schedule_parity`):
   ```
   running 1 test
   test imported_cron_retains_timezone ... ok

   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
   ```

4. **Cron Library Unit Tests** (`cargo test --lib cron::`):
   ```
   test result: ok. 39 passed; 0 failed; 0 ignored; 0 measured; 297 filtered out; finished in 0.53s
   ```

5. **Build and Formatting**:
   - `cargo build`: Finished `dev` profile with 0 errors, 0 warnings (`U76-resume-build.log`).
   - `rustfmt --edition 2021 --check src/cron/scheduler.rs tests/test_voice_cron.rs`: 0 diffs, exit code 0.

## External Shared Blocker Accounting

- **Separate Issue:** Temporary compilation errors in `src/agent/omo_backend.rs` (E0061 missing deadline args) were isolated to ongoing concurrent unit U14 work.
- **Verification Integrity:** Zero compilation or test errors originated from `src/cron/scheduler.rs` or `tests/test_voice_cron.rs`. All cron tests compile and execute cleanly with no warnings or regressions.

## U76 Race-Proof Fixture Repair & Full CAS Predicate Verification

### 1. Defect Analysis in Prior Fixture

The initial version of `overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm` suffered from two proven fixture weaknesses:

1. **Stale Job Retention After Manual Trigger:**
   In Case 1, manual trigger executes with `advance_schedule = false`. On successful execution, the manual run updates `last_status = "succeeded"`, but does NOT advance or disable the one-shot job. Thus, `job_id_1` remained `enabled = 1` and `next_run_at = overdue_next` in SQLite.

2. **Crosstalk & False Gate Firing in Shared Database:**
   Because Case 2 reused the same database, `cron_jobs` contained both `job_id_1` and `job_id_2`, both overdue and enabled. When `run_due_jobs` was spawned in Case 2, its SQL query selected both jobs. The scheduler processed `job_id_1` first and triggered the pre-retire gate for `job_id_1`, NOT `job_id_2`. The test operator rearm concurrently updated `job_id_2`. When the gate was released, the scheduler completed retirement of `job_id_1` and evaluated `job_id_2` whose `next_run_at` had already been advanced to the future. Consequently, `job_id_2` was never actually raced against retirement at all.

3. **Incomplete Predicate Coverage:**
   The original Case 2 rearm changed `expression`, which would have been rejected even by an older `expression = ?` check alone. Races proving the new `next_run_at = ?` and `payload_json = ?` CAS predicates under identical expressions were missing.

### 2. Strengthened Postpatch Controls & Fixture Repair

1. **Per-Case Database Isolation:**
   Each race case is executed against a fresh, independent in-memory SQLite database (`Database::connect("sqlite::memory:")`). The fixture job under test is the only job in the database, ensuring `run_due_jobs` selects and evaluates only that exact candidate and the pre-retire gate reliably fires for that exact job.

2. **Four Discrete Real SQLite Races:**
   - **Case 1: Competing Manual Claim (`NOT EXISTS (SELECT 1 FROM cron_runs WHERE job_id = ? AND status = 'running')`):**
     Scheduler pauses at pre-retire gate. Competing manual claim starts running lease in `cron_runs`. Gate is released; retirement CAS update matches 0 rows. Job remains enabled while running. Manual run completes with status `"succeeded"`.
   - **Case 2: Competing Full Rearm (`expression = ?` + `next_run_at = ?` + `payload_json = ?`):**
     Scheduler pauses at pre-retire gate. Concurrent operator rearm changes expression, next_run_at, and payload. Gate is released; retirement CAS update matches 0 rows. Rearmed timestamp and metadata are fully preserved.
   - **Case 3: Competing Same-Expression Next-Run-At-Only Reschedule (`next_run_at = ?`):**
     Scheduler pauses at pre-retire gate. Operator reschedules job to future instant (`t0 + 2h`), changing ONLY `next_run_at` and `updated_at`. `expression` and `payload_json` remain identical to the observed row. Retirement CAS update matches 0 rows, proving that `next_run_at = ?` protects against clearing the rescheduled instant to NULL.
   - **Case 4: Competing Same-Expression, Same-Timestamp Payload-Only Update (`payload_json = ?`):**
     Scheduler pauses at pre-retire gate. Concurrent operation updates ONLY `payload_json` (e.g. metadata/version) and `updated_at`. `expression` and `next_run_at` remain identical to the observed row. Retirement CAS update matches 0 rows, proving that `payload_json = ?` protects against overwriting concurrent payload edits with stale retirement payloads.

3. **Deterministic Synchronization, Bounded Awaits & Task Safety:**
   - Pre-retire gate future (`entered.notified()`) is subscribed and pinned BEFORE triggering/spawning `run_due_jobs`.
   - Every await (`entered_wait`, `recv()`, `run_task.join()`, `shutdown()`, `database.close()`) is bounded with a 5s timeout.
   - `NotifyOnDrop` guards unblock pending gate waiters if an assertion or step fails early, preventing task hangs.
   - `SpawnGuard` retains task handles, enforces bounded joins, and aborts+joins background tasks on drop, eliminating detached background task leaks.
   - Zero `thread::sleep`, zero polling loops, zero yield loops.

### 3. Verification Artifacts

- **Exact Race Test:**
  `cargo test --test test_voice_cron overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm -- --exact --nocapture`
  Passed: 1 passed; 0 failed; finished in 0.04s (`U76-proof-race.log`).
- **Full Voice Cron Target:**
  `cargo test --test test_voice_cron -- --nocapture`
  Passed: 11 passed; 0 failed; finished in 0.11s (`U76-proof-voice-cron.log`).
- **Formatting:**
  `rustfmt --edition 2021 --check tests/test_voice_cron.rs` (clean, exit code 0, `U76-proof-fmt.log`).
- **Scoped Diff:**
  `U76-proof-diff.patch` captures the focused changes in `tests/test_voice_cron.rs`.

