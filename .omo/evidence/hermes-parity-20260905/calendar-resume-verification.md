# Calendar Phase Independent Verification Report: U46, U47, U76

**Session:** hephaestus (st_01a077b4)  
**Parent Session:** 01a071f6-cc78-7761-86ab-a931f8c15133  
**Evaluated Units:** U46 (`CR.C15`, `CFG.C09`), U47 (`CR.C16`), U76 (`UP.DS03`)  
**Scope:** Independent verification against full manifest scenarios, code inspection, RED/GREEN chronology audit, and re-execution of test suites and build gates.  
**Constraint Compliance:** Zero production or test code edits were made. All workspace dirty changes belonging to other units and phases were preserved untouched.

---

## Executive Summary & Verdicts

| Unit | Title | Findings Covered | Manifest Verification Scenario | Independent Verdict |
| :--- | :--- | :--- | :--- | :--- |
| **U46** | Validated timezone-aware schedules | `CR.C15`, `CFG.C09` | Timezone evaluation in-zone, storage in UTC, preservation across completion and store reload; pre-insertion validation isolating malformed expressions | **PASS** |
| **U47** | Bad-job isolation and explicit rearming | `CR.C16` | Corrupt profile & bad expression isolation without sync crash; operator once rearm & repeat limit increase; runtime execution/delivery status | **PASS** |
| **U76** | Retire overdue unclaimed one-shot jobs | `UP.DS03` | Automatic retirement of one-shots older than 120s grace (T-121s retired as missed, T-120s claimed); recurring & manual trigger preserved | **PASS** |

**Calendar Parity Verdict:** All three units (U46, U47, U76) **PASS** their full manifest scenarios with deterministic, evidence-backed proof.

---

## 1. Unit U46: Validated Timezone-Aware Schedules (`CR.C15`, `CFG.C09`)

### Manifest Scenario & Requirements
- **CR.C15:** Persist and import source timezone, evaluate cron wall-clock schedule in that timezone, convert next instant to UTC only for storage. Recomputing next instant after scheduler execution must retain the configured timezone rather than collapsing to naive UTC.
- **CFG.C09:** Validate full expression before insertion and reject/isolate invalid schedules before writing any database row. Persist configured timezone from store `<home>/config.yaml`.
- **Supplementary:** Support IANA daylight saving time transitions (e.g. `America/New_York` noon EST 17:00Z vs EDT 16:00Z).

### Code & Architecture Inspection
- **Dependency:** Added `chrono-tz = "0.10"` to `Cargo.toml` and `Cargo.lock`. Genuinely necessary as `iana-time-zone` only detects the local host timezone and cannot parse arbitrary IANA strings.
- **Store Schema & Synchronization (`src/cron/store.rs`):**
  - Added `HermesSchedule::timezone: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` for byte-identical round-tripping of non-timezone fixtures.
  - Implemented `HermesStore::timezone(&self)` reading `timezone` from `<home>/config.yaml`.
  - In `HermesStoreSynchronizer::sync_at(now)`: validates expression via `next_run_tz` prior to insertion; stamps profile timezone when job schedule lacks an explicit one; computes initial `next_run_at` in-zone.
- **Scheduler Core (`src/cron/scheduler.rs`):**
  - Implemented `next_run_tz(expression, after, timezone)`: parses IANA timezone string into `chrono_tz::Tz`, converts `after` into the local timezone, evaluates recurrence, and translates the resulting instant back to UTC. Falls back cleanly to UTC for interval and one-shot schedules.
  - In `complete_success`: extracts `schedule.timezone` from the updated payload and invokes `next_run_tz` so subsequent runs re-arm in the configured timezone.
- **Integration Test (`tests/test_cron_schedule_parity.rs`):**
  - Configures `Asia/Seoul` (+09:00), job `0 9 * * *`.
  - Initial due: `2026-09-05T00:00:00Z` (09:00 Seoul). Completion at `2026-09-05T00:01:00Z`.
  - Asserts post-completion `next_run_at` advances to `2026-09-06T00:00:00Z` (09:00 Seoul), NOT `2026-09-05T09:00:00Z` (naive UTC).
  - Closes database, reopens, creates fresh `HermesStoreSynchronizer`, runs resync twice, and verifies `next_run_at` remains `2026-09-06T00:00:00Z`.

### Chronology Audit & Independent Execution
- **RED Evidence:** `U46-resume-red-c15.log` (exit 101: panicked at `assert_eq!(updated.next_run_at, Some(2026-09-06T00:00:00Z))` with left `2026-09-05T09:00:00Z`); `U46-resume-red-c09.log` (exit 101: `garbage` imported as enabled row with NULL next run).
- **GREEN Evidence:** `U46-resume-green-c15.log` (exit 0); `U46-resume-green-c09.log` (exit 0); `U46-resume-green-dst.log` (exit 0).
- **Re-executed Commands (Captured in `calendar-resume-verification-u46.log`):**
  1. `cargo test --test test_cron_schedule_parity imported_cron_retains_timezone -- --exact --nocapture` -> **PASS** (exit 0, 1 passed).
  2. `cargo test --lib cron::store::tests::imports_timezone_and_rejects_invalid_schedule -- --exact --nocapture` -> **PASS** (exit 0, 1 passed).
  3. `cargo test --lib cron::scheduler::tests::next_run_tz_evaluates_wall_clock_and_tracks_dst_offset -- --exact --nocapture` -> **PASS** (exit 0, 1 passed).
  4. Composite manifest command: `cargo test --test test_cron_schedule_parity imported_cron_retains_timezone && cargo test --lib cron::store::tests::imports_timezone_and_rejects_invalid_schedule` -> **PASS** (exit 0, 2 passed).

---

## 2. Unit U47: Bad-Job Isolation and Explicit Rearming (`CR.C16`)

### Manifest Scenario & Requirements
- **CR.C16:** Isolate invalid records and corrupted profiles to preserve healthy boot; distinguish operator rearm from unchanged terminal import; expose current runtime run and delivery state.
- Plug validation holes for malformed one-shot schedules (`once:not-a-date`) so they are never imported as enabled NULL rows.
- Ensure per-job custom timezone overrides take precedence over the store profile timezone during synchronization.

### Code & Architecture Inspection
- **Store Synchronization (`src/cron/store.rs`):**
  - Corrupted store profiles: wrapped `store.load().await` in a `match` expression, logging `tracing::warn!` and continuing, preventing bad profiles from blocking healthy profiles or deleting healthy database rows.
  - Job validation: skips empty job IDs, expressions that fail `job.expression()`, and malformed one-shot timestamps (`parse_timestamp(timestamp).is_err()`).
  - Timezone precedence: computes `effective_timezone = job.schedule.timezone.as_deref().or(timezone.as_deref())`.
  - SQL `ON CONFLICT(id) DO UPDATE SET`:
    - `payload_json`: preserves `last_status`, `last_run_at`, `last_error`, `last_delivery_error`, and `repeat.completed`.
    - `enabled`: preserves disabled state for unchanged terminal once jobs (`cron_jobs.expression = excluded.expression AND cron_jobs.next_run_at IS NULL`) and exhausted repeat jobs. Rearms to `excluded.enabled` when expression/time changes or when operator increases `repeat.times`.
    - `next_run_at`: updates to `excluded.next_run_at` upon schedule modification or repeat limit increase; preserves NULL for unchanged terminal records.
- **Scheduler Core (`src/cron/scheduler.rs`):**
  - Added accessors: `last_status()`, `last_run_at()`, `last_error()`, `last_delivery_error()`.
  - In `complete_success`: persists `last_status = "succeeded"`, `last_run_at = now`, clears error fields.
  - In `complete_failure`: persists `last_status = "failed"`, `last_run_at = now`, `last_error`, and isolates delivery failures into `last_delivery_error`.
  - In `execute_job`: wraps delivery errors in `"delivery error: {err}"` to distinguish delivery failures from task execution errors.
- **Integration Test (`tests/test_cron_boundary_parity.rs`):**
  - Exercises: (1) corrupt JSON profile, (2) invalid cron expr (`garbage`), (3) malformed once without next run, (4) malformed once with next run, (5) valid overdue once, (6) per-job timezone priority (`America/New_York` vs `Asia/Seoul`), (7) run status capture, (8) unchanged terminal preservation, (9) operator once rearm to future `run_at`, (10) operator finite repeat limit increase, (11) execution failure status, and (12) delivery failure status.

### Chronology Audit & Independent Execution
- **RED Evidence:** `U47-resume-red.log` (exit 101: panicked at line 132 on unhandled `garbage` expression); `U47-resume-red-once-validation.log` (exit 101: panicked at line 186, `left: 7, right: 5` because `once:not-a-date` was imported instead of isolated).
- **GREEN Evidence:** `U47-resume-green.log` (exit 0).
- **Re-executed Commands (Captured in `calendar-resume-verification-u47.log`):**
  1. `cargo test --test test_cron_boundary_parity sync_isolates_bad_jobs_and_rearms_changed_once -- --exact --nocapture` -> **PASS** (exit 0, 1 passed).
  2. Manifest redCommand: `cargo test --test test_cron_boundary_parity sync_isolates_bad_jobs_and_rearms_changed_once` -> **PASS** (exit 0, 1 passed).

---

## 3. Unit U76: Retire Overdue Unclaimed One-Shot Jobs (`UP.DS03`)

### Manifest Scenario & Requirements
- **UP.DS03:** Automatically retire unclaimed one-shots older than the 120s grace window (`T - 121s`) at claim/due-scan time, recording a terminal missed diagnostic (`last_status = "missed"`).
- Exact boundary condition: `T - 120s` must claim once and execute.
- Overdue recurring jobs (e.g. `interval:...` or cron expressions) must remain eligible regardless of age.
- Manual triggers (`scheduler.trigger(...)`) and explicit operator rearm semantics must remain fully functional.
- Local surface verification: import stale once job from temporary Hermes store; scheduler dispatch pass must make zero calls to the task executor, and dashboard-visible state must distinguish missed from succeeded.

### Code & Architecture Inspection
- **Scheduler Core (`src/cron/scheduler.rs`):**
  - Enforces `ONESHOT_GRACE_DURATION = Duration::from_secs(120)`.
  - In `claim_job(id, require_due, advance_schedule)`:
    When `require_due == true`, inspects the job's `expression` and `next_run_at`. If `expression.starts_with("once:")` and `now.signed_duration_since(next_run) > grace` and no active running lease exists in `cron_runs`:
    Atomically calls `self.retire_overdue_oneshot(id, &expression, &payload_json, now)` and returns `Ok(None)`.
  - In `retire_overdue_oneshot`: sets `enabled = 0`, `next_run_at = NULL`, `payload.last_status = "missed"`, `payload.last_error = "missed: overdue unclaimed one-shot past grace"`.
  - Manual triggers bypass `require_due` (`require_due = false`), preserving manual execution capability.
- **Integration Test (`tests/test_voice_cron.rs::persisted_oneshot_past_grace_is_retired`):**
  - Seeds DB rows: (1) one-shot at `T - 121s`, (2) one-shot at `T - 120s`, (3) recurring job at `T - 300s`.
  - Verifies due scan claims exactly 2 jobs; `T - 121s` is not claimed and retired as missed.
  - Verifies manual trigger of retired job succeeds and executes.
  - Verifies explicit rearm (setting future time and calling `scheduler.resume`) re-enables the job.
  - Local surface test: imports stale once (`T - 180s`) alongside fresh once; dispatch pass produces 0 executor calls; stale job has `last_status = "missed"`; fresh job triggers and completes with `last_status = "succeeded"`.

### Chronology Audit & Independent Execution
- **RED Evidence:** `U76-resume-red.log` (exit 101: panicked at line 548, `left: 3, right: 2` because unpatched scheduler claimed all 3 jobs including `T - 121s`).
- **GREEN Evidence:** `U76-resume-green.log` (exit 0).
- **Re-executed Commands (Captured in `calendar-resume-verification-u76.log`):**
  1. `cargo test --test test_voice_cron persisted_oneshot_past_grace_is_retired -- --exact --nocapture` -> **PASS** (exit 0, 1 passed).
  2. Manifest redCommand: `cargo test --test test_voice_cron persisted_oneshot_past_grace_is_retired` -> **PASS** (exit 0, 1 passed).

---

## 4. Calendar Targets & Adjacent Regressions

All relevant calendar integration targets and library suites were executed independently. All calendar tests pass with 100% success rate:

| Test Suite / Target | Command | Result | Pass/Fail Count | Log Artifact |
| :--- | :--- | :--- | :--- | :--- |
| **Schedule Parity Target** | `cargo test --test test_cron_schedule_parity` | **PASS** | 1 passed, 0 failed | `calendar-resume-verification-targets.log` |
| **Boundary Parity Target** | `cargo test --test test_cron_boundary_parity` | **PASS** | 1 passed, 0 failed | `calendar-resume-verification-targets.log` |
| **Voice Cron Integration Target** | `cargo test --test test_voice_cron` | **PASS** | 10 passed, 0 failed | `calendar-resume-verification-targets.log` |
| **Cron Library Suite** | `cargo test --lib cron::` | **PASS** | 39 passed, 0 failed | `calendar-resume-verification-adjacent-cron.log` |

---

## 5. Build, Diagnostics, and Style Verification

1. **Workspace Compilation (`cargo build`):**
   - Output: `Finished dev profile [unoptimized + debuginfo] target(s) in 0.48s`.
   - Exit code: 0. 0 compilation errors, 0 warnings in calendar modules.
   - Log: `calendar-resume-verification-build.log`.
2. **Scoped Formatting (`rustfmt --edition 2024 --check`):**
   - `rustfmt --edition 2024 --check src/cron/scheduler.rs tests/test_cron_boundary_parity.rs tests/test_voice_cron.rs` -> Exit code 0 (clean, zero diffs).
   - `src/cron/store.rs` and `tests/test_cron_schedule_parity.rs` preserve handoff formatting differences from earlier RED registrations; under the zero-code-edit mandate for verification, these files were left untouched.
   - Log: `calendar-resume-verification-fmt.log`.

---

## 6. Explicit Isolation of External Suite Failures

Per task scope instructions (*"Other phases own approvals/storage/migration/backend/daemon; do not mark their edits as drift"* and *"Existing other cron suite failures stay explicit rather than false full gate"*), external test suites were inspected:

- **Target:** `tests/test_migrate.rs` (`cargo test --test test_migrate`)
  - Result: 3 passed, 2 failed (`dry_run_projects_every_step_with_zero_writes_or_side_effects`, `full_migration_imports_config_and_cron_before_cutover`).
  - Cause: Migration step `gateway-down` fails to verify live Hermes gateway PID 4242 (`Config("migration step 'gateway-down' failed: configuration error: cannot verify live Hermes gateway pid 4242")`).
  - Analysis: This failure is confined to migration units (U51 / U55 / U75) and does not originate from or affect calendar scheduling, timezone evaluation, isolation, or overdue retirement. It is explicitly recorded in `calendar-resume-verification-other-cron.log` and is properly isolated from the calendar verdicts.

---

## 7. Test Discipline & Tool Limitation Record

- **Test Determinism:** All tests utilize virtual clock injection via `with_clock` and `Arc<Mutex<DateTime<Utc>>>`. Zero `thread::sleep` or polling loops exist in the calendar test suites. Asynchronous completions synchronize through explicit channels with bounded timeouts.
- **Fixture Hygiene:** Every test fixture creates isolated temporary directories via `tempfile::tempdir()` and closes all SQLite database connections upon test completion.
- **Tool Limitations:** Neither `apply_patch` nor `tool.monitor` was exposed in the agent runtime environment. All inspection and command executions were conducted using `read`, `bash`, and direct file tools.

---

## Conclusion

The calendar resumption phase (U46, U47, U76) is **fully verified and approved**:
- **U46 PASS:** Timezone-aware cron schedules accurately re-arm in their source timezone (including DST tracking) and survive store reload/recreation cycles.
- **U47 PASS:** Malformed stores and bad expressions are isolated with warning diagnostics without halting startup, operator rearming functions correctly, and execution runtime status is persisted.
- **U76 PASS:** Unclaimed one-shots older than 120s grace are retired as missed, while exact-boundary, recurring, and manual jobs execute reliably.
