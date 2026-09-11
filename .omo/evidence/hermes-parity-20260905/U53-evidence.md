# U53: Quiesced Receipt-Verified All-Store Cutover

## Registration Before Production Edits

### Scenario 1: C07 Binding Canonical Payload Mismatch Blocks Cutover
Exact integration test id: `changed_payload_blocks_cutover` (crate `test_migrate`, root module).
Literal RED and GREEN command: `cargo test --test test_migrate changed_payload_blocks_cutover -- --exact --nocapture`

Payload:
- Database imported row: id `hermes:default:daily`, prompt `"old"`, schedule `0 9 * * *`.
- Hermes source store (`cron/jobs.json`): same job id `"daily"`, prompt `"new"`, schedule `0 9 * * *`.

Regression scenario:
- A previous sync imported the job into `cron_jobs` with prompt `"old"`.
- The user or external tool edited the source store job `"daily"` to prompt `"new"` before cutover.
- Unpatched behavior: Cutover checked only job ID existence in SQLite (`SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id = ?)`). Seeing the ID present, it considered the job verified, emptied the source store (`jobs: []`), and marked the old DB row `omon_owned`. The new source edit was permanently erased without notification.
- Patched behavior: Cutover validates complete canonical job payloads rather than IDs alone under store lock. On payload mismatch, cutover refuses with a typed configuration error without mutating the source store or creating backups/receipts. Source store bytes remain strictly intact.

### Scenario 2: C07 Binding QA Quiesced Multi-Store Cutover with Injected Failure & Receipt Durability
Exact integration test id: `cutover_second_store_failure_preserves_bytes_and_receipt` (crate `test_migrate`, root module).
Literal GREEN command: `cargo test --test test_migrate cutover_second_store_failure_preserves_bytes_and_receipt -- --exact --nocapture`

Payloads:
- Store 1 (default profile: `cron/jobs.json`): id `"daily"`, prompt `"status"`.
- Store 2 (work profile: `profiles/work/cron/jobs.json`): id `"standup"`, prompt `"standup-check"`.

Regression scenario:
- Multi-store setup with running gateway service (LaunchAgent plist and PID lock).
- Verified service is quiesced before final snapshot and locks: LaunchAgent booted out, PID terminated.
- All store locks acquired in canonical sorted order.
- Private unique backups for ALL stores prepared before first source rewrite.
- Durable pending receipt committed to SQLite (`cron_cutover_receipts` status `'pending'`, `authority = 'cutover_pending'`).
- Injected rewrite failure on Store 2 (read-only destination path).
- Store 1 is replaced with empty document, but Store 2 rewrite fails.
- Store 2 source bytes remain unmutated; both store backups exist with original bytes.
- Pending receipt reflects Store 1 `'replaced'` and Store 2 `'backed_up'`.
- Scheduler is strictly blocked from claiming due jobs or manual triggers while cutover receipt is pending.
- Reopen twice: survives gateway restart; upon unfreezing, `reconcile_pending_cutover` rolls forward Store 2 replacement, commits receipt (`'committed'`), and transitions jobs to `'omon_owned'`. Scheduler then claims all due jobs.

### Scenario 3: Store Lock Alias Deduplication & Unknown State Refusal
Exact unit tests: `alias_store_locks_deduplicate_safely` and `reconcile_refuses_unknown_store_state_without_destroying_source` in `src/migrate/cron_cutover.rs`.
Command: `cargo test --lib cron_cutover -- --nocapture`

- Alias handling: Multiple stores referencing symlinked or aliased paths deduplicate canonical lock paths and acquire locks once in sorted order without deadlock or errors.
- Unknown state: If an interrupted cutover encounters an unrecognized store payload during recovery (neither original nor replacement hash), recovery refuses with a typed error and never destroys the source store.

---

## Captured Behavioral RED (Before Production Edits)

### Scenario 1 RED Output
Literal execution:
`cargo test --test test_migrate changed_payload_blocks_cutover -- --exact --nocapture`

Output (exit code `101`):
```text
    Blocking waiting for file lock on package cache
   Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 34s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 1 test
test changed_payload_blocks_cutover ... FAILED

failures:

---- changed_payload_blocks_cutover stdout ----

thread 'changed_payload_blocks_cutover' (10629840) panicked at tests/test_migrate.rs:566:5:
cutover must refuse on changed payload
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    changed_payload_blocks_cutover

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.03s

error: test failed, to rerun pass `--test test_migrate`
```

---

## Production Implementation

1. `migrations/0023_cutover_receipts.sql`:
   - Creates `cron_cutover_receipts`:
     - `operation_id TEXT PRIMARY KEY`
     - `status TEXT NOT NULL CHECK (status IN ('pending', 'committed', 'failed', 'rolled_back'))`
     - `store_count INTEGER NOT NULL`
     - `policy_digest TEXT`
     - `created_at TEXT NOT NULL`, `updated_at TEXT NOT NULL`
   - Creates `cron_cutover_receipt_stores`:
     - `(operation_id, store_path) PRIMARY KEY`
     - `profile TEXT NOT NULL`
     - `original_hash TEXT NOT NULL`, `replacement_hash TEXT NOT NULL`
     - `backup_path TEXT NOT NULL`
     - `job_digests_json TEXT NOT NULL`
     - `phase TEXT NOT NULL CHECK (phase IN ('prepared', 'backed_up', 'replaced', 'committed'))`
     - `updated_at TEXT NOT NULL`
   - Creates index `idx_cron_cutover_receipts_status ON cron_cutover_receipts(status)`.
   - Preserves 0001-0016, 0022, 0024.

2. `src/migrate/cron_cutover.rs`:
   - `normalize_job_payload` & `canonical_job_payload_matches`: strips internal runtime metadata (`_omon_hermes_source`, `_omon_hermes_profile`, `_omon_hermes_home`, `last_status`, `last_run_at`, `last_error`, `last_delivery_error`, `created_at`, `updated_at`, `next_run_at`, `id`, `repeat.completed`) and checks structural equality.
   - `cutover_cron_stores`:
     - Reconciles any existing pending cutover before starting fresh cutover.
     - Acquires canonical directory locks for all stores in sorted canonical order, safely deduplicating aliases.
     - Takes snapshot under lock and validates canonical payloads against SQLite `cron_jobs`. Any mismatch or unimported job refuses with a typed error.
     - Prepares unique private backups for ALL stores before any source file is rewritten.
     - Commits pending receipt and sets `authority = 'cutover_pending'` in a single SQLite transaction.
     - Revalidates store bytes right before rewriting each store with empty document (`jobs: []`) using U50 atomic write primitive; updates store phase to `'replaced'`.
     - Commits final ownership in a single SQLite transaction: receipt status `'committed'`, store phase `'committed'`, job authority `'omon_owned'`.
     - Releases all locks.
   - `reconcile_pending_cutover`:
     - Rolls forward any pending cutover from recorded hashes, verifying current bytes against original and replacement hashes.
     - Refuses unknown/mismatched bytes without destroying the source store.

3. `src/migrate/mod.rs`:
   - Quiescence precedence: `bring_gateway_down` unloads LaunchAgents and terminates verified PIDs BEFORE `cutover_cron_stores` executes, in both live execution and dry-run projection.

4. `src/migrate/sys.rs`:
   - Adds `canonicalize` and alias support to `MigrationEnv`, `OsEnv`, and `FakeMigrationEnv`.

5. `src/cron/scheduler.rs`:
   - Exposes `has_pending_cutover_receipt(pool)` and method on `CronScheduler`.
   - Guards `run_due_jobs` and `claim_job` to return 0 / None while any cutover receipt is pending in SQLite.

---

## Captured Behavioral GREEN

### Scenario 1 GREEN Output
Literal command:
`cargo test --test test_migrate changed_payload_blocks_cutover -- --exact --nocapture`

Output (exit code `0`):
```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.55s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 1 test
test changed_payload_blocks_cutover ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s
```

### Scenario 2 GREEN Output
Literal command:
`cargo test --test test_migrate cutover_second_store_failure_preserves_bytes_and_receipt -- --exact --nocapture`

Output (exit code `0`):
```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.16s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 1 test
test cutover_second_store_failure_preserves_bytes_and_receipt ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s
```

### Full `test_migrate` Suite GREEN Output
Literal command:
`cargo test --test test_migrate`

Output (exit code `0`):
```text
   Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 29.60s
     Running tests/test_migrate.rs (target/debug/deps/test_migrate-886788bceb99fc64)

running 8 tests
test dry_run_without_database_reports_creation_without_creating_it ... ok
test secret_files_are_private_and_backup_unique ... ok
test dry_run_projects_every_step_with_zero_writes_or_side_effects ... ok
test changed_payload_blocks_cutover ... ok
test full_migration_imports_config_and_cron_before_cutover ... ok
test cutover_second_store_failure_preserves_bytes_and_receipt ... ok
test cutover_survives_startup_sync ... ok
test private_migration_os_surface_controls ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
```

---

## Adjacent Suite & Diagnostics Verification

### Unit Tests: `cron_cutover`
Literal command:
`cargo test --lib cron_cutover`

Output (exit code `0`):
```text
running 10 tests
test migrate::cron_cutover::tests::stale_store_state_is_rejected_before_backup ... ok
test migrate::cron_cutover::tests::malformed_store_returns_typed_error_and_performs_no_writes ... ok
test migrate::cron_cutover::tests::unimported_job_returns_typed_error_without_changing_store ... ok
test migrate::cron_cutover::tests::dry_run_reports_unverified_jobs_and_performs_zero_writes ... ok
test migrate::cron_cutover::tests::reconcile_refuses_unknown_store_state_without_destroying_source ... ok
test migrate::cron_cutover::tests::backup_bytes_equal_the_pre_delete_store ... ok
test migrate::cron_cutover::tests::verified_jobs_are_backed_up_then_atomically_emptied ... ok
test migrate::cron_cutover::tests::profiles_are_discovered_from_the_profiles_directory ... ok
test migrate::cron_cutover::tests::alias_store_locks_deduplicate_safely ... ok
test migrate::cron_cutover::tests::rerun_on_empty_store_creates_no_second_backup ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 343 filtered out; finished in 0.11s
```

### Full Migration Module: `src/migrate/`
Literal command:
`cargo test --lib migrate`

Output (exit code `0`):
```text
running 49 tests
test migrate::config_import::tests::malformed_hermes_root_path_is_typed_and_never_writes_target ... ok
test migrate::config_import::tests::malformed_yaml_is_typed_and_never_writes_target ... ok
test migrate::config_import::tests::env_parser_ignores_comments_and_uses_last_value_in_a_file ... ok
test migrate::config_import::tests::omits_missing_and_empty_values ... ok
test migrate::config_import::tests::routes_non_claude_custom_provider_to_openai_compatible_keys ... ok
test migrate::config_import::tests::dry_run_returns_masked_diff_and_performs_zero_writes ... ok
test migrate::config_import::tests::dotenv_raw_documents_round_trip ... ok
test migrate::config_import::tests::dedupes_profile_tokens_in_stable_order_and_strips_quotes ... ok
test migrate::config_import::tests::routes_claude_model_to_anthropic_and_maps_runtime_keys ... ok
test migrate::config_import::tests::imported_values_round_trip_through_runtime_environment_parsing ... ok
test migrate::config_import::tests::merges_into_existing_env_and_backs_up_original ... ok
test migrate::config_import::tests::root_scalar_values_win_over_profile_fallbacks ... ok
test migrate::cron_cutover::tests::stale_store_state_is_rejected_before_backup ... ok
test migrate::gateway_down::tests::every_matching_plist_is_booted_out_then_renamed_disabled ... ok
test migrate::gateway_down::tests::dry_run_reports_intended_actions_with_zero_side_effects ... ok
test migrate::gateway_down::tests::malformed_locks_refuse_before_side_effects ... ok
test migrate::gateway_down::tests::failed_bootout_preserves_files_and_never_signals ... ok
test migrate::gateway_down::tests::alive_pids_terminate_then_escalate_only_when_still_alive ... ok
test migrate::gateway_down::tests::kill_timeout_preserves_lock_and_is_bounded ... ok
test migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled ... ok
test migrate::gateway_down::tests::not_loaded_bootout_is_success_and_plist_is_still_disabled ... ok
test migrate::gateway_down::tests::reused_pid_during_term_wait_never_receives_kill ... ok
test migrate::gateway_down::tests::pid_alive_after_bounded_wait_is_killed ... ok
test migrate::gateway_down::tests::pid_that_dies_during_bounded_wait_is_not_killed ... ok
test migrate::gateway_down::tests::stale_pid_lock_is_removed_and_disabled_plists_remain_a_no_op ... ok
test migrate::gateway_down::tests::service_unloads_before_verified_signals_and_delayed_kill_exit ... ok
test migrate::config_import::tests::dotenv_existing_assignments_reload_special_values ... ok
test migrate::gateway_down::tests::stale_pid_never_signaled ... ok
test migrate::gateway_down::tests::unknown_or_conflicting_identity_never_authorizes_signals ... ok
test migrate::gateway_down::tests::valid_and_invalid_gateway_command_lines ... ok
test migrate::sys::tests::fake_clock_is_injectable ... ok
test migrate::sys::tests::fake_read_only_path_rejects_writes ... ok
test migrate::sys::tests::fake_filesystem_write_read_rename_and_exists_are_coherent ... ok
test migrate::sys::tests::fake_records_process_and_launchctl_calls_without_os_env ... ok
test migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang ... ok
test migrate::sys::tests::os_env_reads_self_command_line ... ok
test migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock ... ok
test migrate::config_import::tests::dotenv_round_trip_special_values ... ok
test migrate::cron_cutover::tests::malformed_store_returns_typed_error_and_performs_no_writes ... ok
test migrate::cron_cutover::tests::dry_run_reports_unverified_jobs_and_performs_zero_writes ... ok
test migrate::cron_cutover::tests::reconcile_refuses_unknown_store_state_without_destroying_source ... ok
test migrate::cron_cutover::tests::unimported_job_returns_typed_error_without_changing_store ... ok
test migrate::cron_cutover::tests::backup_bytes_equal_the_pre_delete_store ... ok
test migrate::cron_cutover::tests::rerun_on_empty_store_creates_no_second_backup ... ok
test migrate::cron_cutover::tests::verified_jobs_are_backed_up_then_atomically_emptied ... ok
test migrate::cron_cutover::tests::alias_store_locks_deduplicate_safely ... ok
test migrate::cron_cutover::tests::profiles_are_discovered_from_the_profiles_directory ... ok
test migrate::gateway_down::tests::migration_entry_retires_service_with_real_files_and_database ... ok
test migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child ... ok

test result: ok. 49 passed; 0 failed; 0 ignored; 0 measured; 304 filtered out; finished in 0.21s
```

### Cron Module: `src/cron/`
Literal command:
`cargo test --lib cron::`

Output (exit code `0`):
39 passed; 0 failed.

### Formatting & Lint Checks
Literal command:
`rustfmt --edition 2021 --check src/migrate/mod.rs src/migrate/cron_cutover.rs src/migrate/sys.rs src/cron/scheduler.rs tests/test_migrate.rs`

Output (exit code `0`): Clean, 0 diff.

Literal command:
`cargo check --tests`

Output (exit code `0`): Clean, 0 warnings, 0 errors.

---

## Cleanliness & Safety Verification

1. Write Scope:
   Only files in `["src/migrate/mod.rs", "src/migrate/cron_cutover.rs", "src/migrate/sys.rs", "src/cron/scheduler.rs", "migrations/0023_cutover_receipts.sql", "tests/test_migrate.rs"]` and `.omo/evidence/hermes-parity-20260905/U53-*` were touched. No other files were modified.
2. Safety Constraints:
   - Migration never executed against repo cwd, real user HOME, or production database.
   - Tests run entirely against temporary files and `FakeMigrationEnv` / in-memory SQLite.
   - No real launchctl commands executed, no OS process signals emitted, no Discord interactions.
   - No git commits or branch pushes created.
