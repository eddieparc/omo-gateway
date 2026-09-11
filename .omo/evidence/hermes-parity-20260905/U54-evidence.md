# U54: Dry-run Shares Full Import Validation

## Registration Before Production Edits

### Scenario: C08 Store Selection and Pure Full-Job Validation Shared Between Projection and Apply
Exact integration test id: `dry_run_reports_nonimportable_jobs` (crate `test_migrate`, root module).
Literal RED and GREEN command: `cargo test --test test_migrate dry_run_reports_nonimportable_jobs -- --exact --nocapture`

### Payloads
- Store 1 (default profile: `cron/jobs.json`):
  - Job `daily`: prompt `"status"`, schedule `{"kind": "cron", "expr": "0 9 * * *"}`.
- Store 2 (work profile: `profiles/work/cron/jobs.json`):
  - Job `bad`: missing schedule expression (`{"id":"bad", "schedule":{"kind":"cron"}}`).
  - Job `restart`: forbidden gateway self-restart prompt (`"prompt":"hermes gateway restart"`, schedule `{"kind":"cron","expr":"0 9 * * *"}`).
  - Job `standup`: valid job (`"prompt":"team standup update"`, schedule `{"kind":"cron","expr":"0 9 * * *"}`).
- Profile restriction:
  - `OMON_HERMES_PROFILES="work"` (default profile is nonselected).

### Regression Scenario
- Before fix:
  - Dry-run projection in `src/migrate/mod.rs` parsed IDs only via `projected_cron_jobs` and ignored `OMON_HERMES_PROFILES`.
  - Dry-run bypassed all schedule validation, timestamp parsing, and gateway lifecycle prompt guards.
  - Dry-run advertised all jobs as `cron_importable`, including `bad` (missing schedule), `restart` (forbidden lifecycle command), and `daily` (nonselected default profile).
  - Dry-run unconditionally set `would_empty = store.jobs_found > 0`, falsely projecting that both `default` (nonselected) and `work` (containing non-importable jobs) would be emptied.
  - Applying migration would subsequently skip/reject the bad jobs, refuse to empty `work` store during cutover, or unexpectedly touch nonselected stores.
- After fix:
  - `HermesJob::validate(&self, default_timezone: Option<&str>, now: DateTime<Utc>)` provides a pure validation seam in `src/cron/store.rs`, shared identically by dry-run projection and runtime synchronization (`sync_at`).
  - Profile restriction via `OMON_HERMES_PROFILES` is honored identically in dry-run and apply.
  - Jobs in nonselected profiles are classified into `summary.cron_nonselected`.
  - Jobs failing validation are classified into `summary.cron_rejected` with typed `CronJobRejection` (recording `id`, `job_id`, `profile`, `reason`).
  - Only valid selected jobs enter `summary.cron_importable`.
  - `store.would_empty` evaluates to `false` when a store is nonselected or contains any rejected jobs; `would_empty` is `true` only when the store is selected, has jobs, and has zero rejected jobs.
  - QA import-only run (`--no-cutover`) imports exactly the jobs advertised in `cron_importable`.
  - Dry-run creates zero files, modifies zero files, performs zero database writes, acquires zero persistent locks, and sends zero signals.

---

## Captured Behavioral RED (Before Production Edits)

Literal execution command:
```bash
cargo test --test test_migrate dry_run_reports_nonimportable_jobs -- --exact --nocapture
```

Exit status: `101`

Captured output:
```text
   Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
warning: unused import: `CronJobRejection`
 --> tests/test_migrate.rs:3:47
  |
3 | use omon_gateway::migrate::{run_migrate_with, CronJobRejection, MigrateArgs, MigrationPaths};
  |                                               ^^^^^^^^^^^^^^^^
  |
  = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: `omo-gateway` (test "test_migrate") generated 1 warning (run `cargo fix --test "test_migrate" -p omo-gateway` to apply 1 suggestion)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2m 00s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 1 test

thread 'dry_run_reports_nonimportable_jobs' (116718) panicked at tests/test_migrate.rs:984:5:
job with missing schedule must be reported in cron_rejected, got: []
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test dry_run_reports_nonimportable_jobs ... FAILED

failures:

failures:
    dry_run_reports_nonimportable_jobs

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.01s

error: test failed, to rerun pass `--test test_migrate`
```

---

## Production Implementation

### 1. `src/cron/store.rs`
- Defined `ValidatedHermesJob`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct ValidatedHermesJob {
      pub expression: String,
      pub effective_timezone: Option<String>,
      pub computed_next: Option<DateTime<Utc>>,
  }
  ```
- Implemented `HermesJob::validate(&self, default_timezone: Option<&str>, now: DateTime<Utc>) -> Result<ValidatedHermesJob, String>`:
  - Rejects empty IDs (`"Hermes job has an empty id"`).
  - Enforces prompt and script lifecycle checks via `check_gateway_lifecycle`.
  - Enforces schedule expression parsing via `self.expression()`.
  - Validates one-shot timestamps (`parse_timestamp`).
  - Validates full schedule recurrence in the effective timezone via `super::scheduler::next_run_tz`.
- Exposed profile selection utilities on `HermesStoreSynchronizer`:
  - `HermesStoreSynchronizer::selected_profiles() -> Option<Vec<String>>`
  - `HermesStoreSynchronizer::is_profile_selected(profile: &str, selected_profiles: Option<&[String]>) -> bool`
- Updated `HermesStoreSynchronizer::sync_at` to use `job.validate(timezone.as_deref(), now)` directly.

### 2. `src/migrate/mod.rs`
- Defined `CronJobRejection`:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct CronJobRejection {
      pub id: String,
      pub job_id: String,
      pub profile: String,
      pub reason: String,
  }
  ```
- Added fields to `MigrationSummary`:
  ```rust
  pub cron_rejected: Vec<CronJobRejection>,
  pub cron_nonselected: Vec<String>,
  ```
- Implemented `read_store_timezone` to resolve source timezone from `<home>/config.yaml`.
- Implemented `discover_all_hermes_stores` and updated `discover_hermes_stores` to respect `HermesStoreSynchronizer::selected_profiles()`.
- Rewrote `project_migration`:
  - Scans all stores on disk.
  - Classifies nonselected store jobs into `cron_nonselected`.
  - Validates selected store jobs using the shared `job.validate(timezone.as_deref(), now)` seam.
  - Classifies rejected jobs into `cron_rejected` with typed reasons.
  - Classifies valid jobs into `cron_importable` or `cron_already_present`.
  - Determines store cutover feasibility:
    `store.would_empty = is_selected && store.jobs_found > 0 && !has_rejected;`
- Updated `print_summary` to display `rejected` and `nonselected` job lists.

### 3. `tests/test_migrate.rs`
- Added integration test `dry_run_reports_nonimportable_jobs`:
  - Configures default store with `daily` and work store with `bad`, `restart`, `standup`.
  - Restricts `OMON_HERMES_PROFILES="work"`.
  - Runs dry-run and asserts zero disk writes, zero renames, zero DB writes, zero signals.
  - Asserts `cron_rejected` contains `bad` and `restart`.
  - Asserts `cron_nonselected` contains `daily`.
  - Asserts `cron_importable` contains only `["hermes:work:standup"]`.
  - Asserts `would_empty` is `false` for both `default` (nonselected) and `work` (rejected jobs present).
  - Runs import-only apply (`dry_run: false, no_cutover: true`) and asserts exactly 1 job (`hermes:work:standup`) is imported into `cron_jobs`.
- Added `SERIAL_TEST_LOCK` to ensure parallel test isolation during environment variable tests.

---

## Captured Behavioral GREEN (After Production Edits)

Literal execution command:
```bash
cargo test --test test_migrate dry_run_reports_nonimportable_jobs -- --exact --nocapture
```

Exit status: `0`

Captured output:
```text
   Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 8.27s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 1 test
test dry_run_reports_nonimportable_jobs ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.04s
```

Full suite execution:
```bash
cargo test --test test_migrate -- --nocapture
```

Exit status: `0`

Captured output:
```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.24s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 9 tests
C05 target_mode=600 backup_mode=600 atomic=true unique=true failed=true intact=true no_temps=true
test secret_files_are_private_and_backup_unique ... ok
test full_migration_imports_config_and_cron_before_cutover ... ok
test dry_run_projects_every_step_with_zero_writes_or_side_effects ... ok
test cutover_second_store_failure_preserves_bytes_and_receipt ... ok
test dry_run_reports_nonimportable_jobs ... ok
test dry_run_without_database_reports_creation_without_creating_it ... ok
test changed_payload_blocks_cutover ... ok
OsEnv full migration: private config/cron/backup; exclusive symlink collision; failed rename cleanup; fixture removed
test private_migration_os_surface_controls ... ok
C01 RED check: count after migrate = 1, count after startup sync = 1
test cutover_survives_startup_sync ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
```

---

## Adjacent Verification

1. `cargo test --lib cron::store`:
   11 passed; 0 failed (including schedule validation, timezone imports, lifecycle checks).
2. `cargo test --lib cron_cutover`:
   10 passed; 0 failed (including verified cutover, atomic replacements, recovery reconciliation, store lock aliasing).
3. `cargo test --lib gateway_down`:
   17 passed; 0 failed (including service unload, verified process lifecycle, PID handling).
4. `cargo test --lib config_import`:
   14 passed; 0 failed (including dotenv fidelity, value escaping, dry-run masking).
5. `cargo test --test test_cron_schedule_parity`:
   1 passed; 0 failed.
6. `cargo test --test test_cron_boundary_parity`:
   1 passed; 0 failed.
7. `cargo test --test test_cron_authority_upgrade`:
   1 passed; 0 failed.
8. `cargo fmt -- --check src/cron/store.rs src/migrate/mod.rs tests/test_migrate.rs`:
   Clean (exit code 0).
9. `cargo clippy --test test_migrate`:
   Clean (0 warnings in test_migrate or migrate/store crates).
