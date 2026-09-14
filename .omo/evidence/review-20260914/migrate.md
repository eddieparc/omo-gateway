# Lane: migrate
## Scope
- src/migrate/config_import.rs (1276 LOC)
- src/migrate/cron_cutover.rs (955 LOC)
- src/migrate/gateway_down.rs (1171 LOC)
- src/migrate/mod.rs (496 LOC)
- src/migrate/sys.rs (1290 LOC)
Total: 5188 LOC

## Findings

### [P0] Store Timezone Injected by Synchronizer Causes Cutover Verification Failure
- Location: src/migrate/cron_cutover.rs:343
- Evidence:
```rust
if !canonical_job_payload_matches(job_val, &db_payload) {
```
- Why it matters: When `HermesStoreSynchronizer::sync()` imports cron jobs into the gateway database (`src/cron/store.rs:941-943`), it automatically injects the store's default timezone from `config.yaml` into `job.schedule.timezone` if none was set in `jobs.json`. When `cutover_cron_stores` verifies imported jobs, `canonical_job_payload_matches` compares the raw store payload with the database payload. `normalize_job_payload` does not strip `schedule.timezone`. Because the source JSON lacks `schedule.timezone` while the database record contains it, `source_norm == db_norm` evaluates to `false`. Every job in a store with a configured default timezone is marked as unverified, causing `cutover_cron_stores` to abort with a fatal error after `bring_gateway_down` has already killed the Hermes gateway and disabled launchctl plists.
- Suggested fix: In `canonical_job_payload_matches` or `normalize_job_payload`, normalize `schedule.timezone` or pass the store's default timezone so that store-inherited default timezones match the database representation before comparing.

### [P0] Unfiltered Store Discovery in `cutover_cron_stores` Breaks Cutover on Multi-Profile Systems
- Location: src/migrate/cron_cutover.rs:281
- Evidence:
```rust
let stores = discover_stores(env, hermes_root)?;
```
- Why it matters: `run_migrate_with` restricts cron import to selected profiles via `discover_hermes_stores` and `HermesStoreSynchronizer::selected_profiles()`. However, `cutover_cron_stores` calls `discover_stores`, which discovers and attempts to cut over ALL profiles under `profiles/` unconditionally. If an unselected profile has jobs in `profiles/<profile>/cron/jobs.json`, those jobs were never imported into SQLite `cron_jobs`. `cutover_cron_stores` finds unverified jobs for that profile and returns `Err(OmonError::Config(... refusing to empty ...))` at line 416, aborting the entire migration after Hermes has already been stopped and its launch agents disabled.
- Suggested fix: Update `discover_stores` in `cron_cutover.rs` to filter profiles using `HermesStoreSynchronizer::selected_profiles()`, matching `discover_hermes_stores` in `mod.rs`.

### [P0] Non-Cutover and Partial Migration Lead to Dual Active Schedulers (Double-Scheduling)
- Location: src/migrate/mod.rs:182
- Evidence:
```rust
if args.no_cutover {
```
- Why it matters: In `run_after_config`, `synchronizer.sync().await` imports all Hermes cron jobs into the gateway SQLite database with `enabled = 1` and `authority = 'hermes_mirror'`. When `--no-cutover` is specified, `run_after_config` returns early without stopping the Hermes gateway or emptying Hermes cron stores. In `src/cron/scheduler.rs:990`, the gateway scheduler queries all enabled jobs where `authority != 'cutover_pending'`. It does not filter out `hermes_mirror`. Consequently, both Hermes and Omon Gateway concurrently execute the identical cron jobs, generating duplicate alerts, webhook deliveries, and automated messages. The same defect occurs if migration fails during `bring_gateway_down` or `cutover_cron_stores`: the imported jobs remain enabled in SQLite without rollback while Hermes continues running.
- Suggested fix: When importing jobs prior to cutover, set their `enabled` status to `0` or assign a non-executable authority (e.g. `imported_pending_cutover`) until `cutover_cron_stores` commits final ownership, enabling them atomically only when cutover succeeds.

### [P0] Command Flag Parsing in `gateway_down.rs` Causes Process Termination Failure and Lock Deletion
- Location: src/migrate/gateway_down.rs:208
- Evidence:
```rust
return Some(filtered[i + 1].to_string());
```
- Why it matters: `gateway_command_subcommand` searches for the token `"gateway"` and blindly takes `filtered[i + 1]` as the subcommand name. If the Hermes process was invoked with options between `gateway` and the subcommand (e.g. `hermes gateway -v run`, `hermes gateway --port 8080 run`, or `python3 -m hermes_cli.main gateway -c config.yaml run`), `filtered[i + 1]` is the flag (e.g. `"-v"` or `"--port"`). `looks_like_gateway_runtime_command_line` only recognizes `"run"` and `"restart"`, so it returns `false`. `verified_alive` returns `false`, causing `bring_gateway_down` to skip process termination (neither SIGTERM nor SIGKILL are sent). However, line 110 proceeds to remove `gateway.lock`. The Hermes gateway remains running untracked in the background, competing with Omon Gateway for resources and ports.
- Suggested fix: Skip flags and their arguments following the `"gateway"` token until encountering the actual subcommand verb (`"run"`, `"restart"`, `"start"`, or `"daemon"`).

### [P0] Reconcile Pending Cutover Only Clears 1 Pending Op, Indefinitely Freezing Gateway Scheduler
- Location: src/migrate/cron_cutover.rs:146
- Evidence:
```rust
"SELECT operation_id FROM cron_cutover_receipts WHERE status = 'pending' ORDER BY created_at DESC LIMIT 1",
```
- Why it matters: `reconcile_pending_cutover` queries a single pending operation using `LIMIT 1`. Furthermore, `reconcile_pending_cutover` is only called inside `cutover_cron_stores`; it is never invoked on gateway startup (`main.rs`). In `src/cron/scheduler.rs:1008`, `has_pending_cutover_receipt` halts all cron job claiming if ANY row exists with `status = 'pending'` in `cron_cutover_receipts`. If multiple interrupted migrations created pending receipts, only the most recent can ever be reconciled, leaving older pending receipts in the database forever and permanently disabling the gateway cron scheduler.
- Suggested fix: Reconcile or mark failed all pending receipts in a loop, and ensure `reconcile_pending_cutover` is invoked during gateway startup before the scheduler begins polling.

### [P1] Missing `start_time` in Lock File Triggers Hard Error Instead of Graceful Fallback
- Location: src/migrate/gateway_down.rs:228
- Evidence:
```rust
_ => Err(OmonError::Config(format!(
```
- Why it matters: `GatewayLock` defines `start_time` as `Option<u64>`. In `verified_alive`, matching `(start, env.process_start_time(pid)?)` handles `(Some(expected), Some(actual))` and `(Some(_), Some(_))`, but all other combinations fall into `_ => Err(OmonError::Config("cannot verify live Hermes gateway pid ..."))`. If a valid lock file omits `start_time` (e.g. written by an older Hermes version or on a system without start-time tracking), `verified_alive` errors out instead of verifying the process via its command line, crashing the migration prematurely.
- Suggested fix: In `verified_alive`, when `start` is `None`, verify process identity using `process_command_line` and `looks_like_gateway_runtime_command_line` rather than returning a fatal error.

### [P1] TOCTOU Race Condition on Process Exit Crashes Migration
- Location: src/migrate/sys.rs:395
- Evidence:
```rust
if read != size {
```
- Why it matters: In `OsEnv::process_start_time`, if `proc_pidinfo` fails with `read != size` (which occurs when a target process exits between `pid_alive` and `process_start_time`), it returns `Err(OmonError::Config(...))`. In `verified_alive`, this error is propagated with `?`. Similarly, if `process_command_line` returns `Ok(None)` because `/bin/ps` found no process, `verified_alive` returns `Err`. If the Hermes process exits on its own while `bring_gateway_down` is checking it, the migration crashes with an unhandled error instead of recognizing that the process is already stopped.
- Suggested fix: Treat ESRCH / process lookup failures in `process_start_time` and `process_command_line` as `Ok(None)` or `Ok(false)` rather than hard errors.

### [P1] Synchronous Blocking I/O and Thread Sleep Inside Async Context
- Location: src/migrate/sys.rs:625
- Evidence:
```rust
std::thread::sleep(duration);
```
- Why it matters: `OsEnv::sleep` calls `std::thread::sleep(duration)` synchronously. In `wait_until_dead` (`src/migrate/gateway_down.rs:258`), this is called in a loop up to 20 times (10 iterations after SIGTERM, 10 after SIGKILL) with 100ms intervals, blocking the OS thread for up to 2 seconds. Because `run_migrate` and `run_after_config` are Tokio `async fn` entry points, blocking the thread blocks Tokio runtime worker threads. The same blocking pattern exists for file I/O operations (`std::fs::read`, `std::fs::write`, `std::fs::rename`) and `flock` in `OsMigrationLock`.
- Suggested fix: Use `tokio::time::sleep` or execute the synchronous migration pipeline within `tokio::task::spawn_blocking`.

### [P1] Missing Rollback Across Migration Steps Leaves Partially Migrated State
- Location: src/migrate/mod.rs:85
- Evidence:
```rust
import_config(&env, &paths.hermes_root, &paths.target_env, false).map_err(|error| {
```
- Why it matters: The migration executes four non-atomic phases in sequence: `import_config` writes the target `.env`, `synchronizer.sync` inserts cron jobs into SQLite, `bring_gateway_down` disables launchctl agents and terminates processes, and `cutover_cron_stores` empties Hermes JSON stores. If any subsequent step fails, earlier modifications are not rolled back: `.env` remains modified, and SQLite contains half-migrated cron jobs. Rerunning `run_migrate` creates additional timestamped backup files (`.env.bak-...`) without restoring consistency.
- Suggested fix: Introduce an explicit compensation/rollback mechanism that restores `.env` from its backup and deletes imported `hermes_mirror` cron jobs if a subsequent migration step fails.

### [P1] Config Import Discards Discord Bot Tokens in Target `.env` When Hermes Discord is Disabled
- Location: src/migrate/config_import.rs:531
- Evidence:
```rust
} else if key == "DISCORD_BOT_TOKEN" || key == "DISCORD_BOT_TOKENS" {
```
- Why it matters: In `render_merged_env`, lines in the existing target `.env` matching `DISCORD_BOT_TOKEN` or `DISCORD_BOT_TOKENS` are skipped if the incoming `overlay` does not contain them. In `merged_values` (`lines 505-510`), those keys are explicitly removed from `merged`. If Hermes had Discord disabled (`discord: { enabled: false }`) or Hermes was running with Slack/Telegram only, `overlay` contains no Discord tokens. The migration silently deletes all existing `DISCORD_BOT_TOKEN` and `DISCORD_BOT_TOKENS` entries from the gateway `.env`. The gateway subsequently fails to boot with `"missing required environment variable DISCORD_BOT_TOKEN"`.
- Suggested fix: Preserve existing `DISCORD_BOT_TOKEN` and `DISCORD_BOT_TOKENS` in target `.env` unless new valid replacement tokens are explicitly provided by the import source.

### [P1] Config Import Misses API Keys Stored in Hermes `.env`
- Location: src/migrate/config_import.rs:351
- Evidence:
```rust
"OPENAI_API_KEY",
```
- Why it matters: `map_values` extracts `OPENAI_API_KEY`, `OPENAI_API_BASE`, `ANTHROPIC_API_KEY`, and `ANTHROPIC_BASE_URL` exclusively from `root_config.model` (the YAML file `config.yaml`). It never checks `root_env` (`~/.hermes/.env`) or profile `.env` files for these keys, and they are omitted from `SCALAR_ENV_KEYS`. Operators following security best practices by storing secret API keys in `.env` rather than YAML configs will have their API keys omitted from the migrated `.env`, leaving the gateway without LLM credentials.
- Suggested fix: Include `OPENAI_API_KEY`, `OPENAI_API_BASE`, `ANTHROPIC_API_KEY`, and `ANTHROPIC_BASE_URL` in `SCALAR_ENV_KEYS` or check `root_env` and `profile_env` as fallbacks in `map_values`.

### [P2] `sqlite_file_path` Rejects Standard SQLx SQLite URIs in Dry-Run
- Location: src/migrate/mod.rs:440
- Evidence:
```rust
let value = database_url.strip_prefix("sqlite://")?;
```
- Why it matters: `sqlite_file_path` strictly requires the URI prefix `"sqlite://"`. SQLx also accepts `"sqlite:path.db"` (single colon, no double slash). While `Database::connect` connects successfully to `"sqlite:omon_gateway.db"`, running `--dry-run` fails with `Err("dry-run requires a file-backed SQLite DATABASE_URL, got sqlite:omon_gateway.db")`.
- Suggested fix: Strip both `"sqlite://"` and `"sqlite:"` prefixes when parsing the database file path.

### [P2] Dead Code `cutover_store` in `cron_cutover.rs`
- Location: src/migrate/cron_cutover.rs:613
- Evidence:
```rust
pub fn cutover_store(env: &dyn MigrationEnv, store: &PreparedStore) -> Result<()> {
```
- Why it matters: `pub fn cutover_store` is an obsolete single-store cutover function that lacks receipt tracking, authority transition, and multi-store atomicity. It is only called from unit tests (`stale_store_state_is_rejected_before_backup`) and is never used in the production cutover path (`cutover_cron_stores`).
- Suggested fix: Remove `cutover_store` and update the unit test to exercise `cutover_cron_stores` directly.

### [P2] Dotenv Parser Errors Suppressed Without Line Number or Context
- Location: src/migrate/config_import.rs:287
- Evidence:
```rust
let assignment = parser.next().transpose().map_err(|_| {
```
- Why it matters: When `dotenvy` encounters a parsing syntax error in an environment file, `parse_env_document` discards the underlying error to prevent leaking secrets. However, it also strips the line number and syntax failure reason, reporting only `"failed to parse environment <path>"`. This prevents operators from diagnosing syntax errors in their `.env` files.
- Suggested fix: Extract and report the line number or error variant from `dotenvy::Error` without formatting the raw line value.

### [P2] Inactive Profile Configs and Tokens Imported Without Checking Selected Profiles
- Location: src/migrate/config_import.rs:460
- Evidence:
```rust
.filter(|(_, cfg)| cfg.as_ref().is_none_or(|c| c.discord.enabled))
```
- Why it matters: `import_config` discovers all subdirectories under `profiles/` via `read_profiles` and merges their tokens and scalar environment variables. Unlike `HermesStoreSynchronizer`, it does not check `selected_profiles()`. If an operator has archive or staging profiles with different settings (e.g. `APPROVAL_MODE=always`), `map_values` merges values from unselected profiles into the gateway `.env`.
- Suggested fix: Filter profiles in `read_profiles` using `HermesStoreSynchronizer::selected_profiles()`.

## Strengths
- Atomic file writes and backups: File updates in `config_import` and `cron_cutover` use atomic temporary file writing followed by rename (`write_atomic`), with unique timestamped backups created prior to overwriting.
- Safe lock acquisition: `cron_cutover` canonicalizes lock paths and deduplicates aliases in a `BTreeSet` before acquiring locks, avoiding self-deadlock when multiple profiles alias the same store directory.
- Two-phase cutover receipts: `cron_cutover` records cutover operations and store replacement hashes durably in SQLite receipts (`cron_cutover_receipts`), enabling verifiable store recovery.
- Abstracted OS environment: The `MigrationEnv` trait and `FakeMigrationEnv` implementation allow thorough testing of filesystem, signal, and launchctl interactions without mutating the host environment.

## Notes
- `Database::connect` automatically runs pending migrations when opening a writable pool, which is why tables like `cron_cutover_receipts` are available during normal cutover; however, during `--dry-run`, if the database file does not exist, `open_read_only_pool_if_present` correctly yields `None`.
