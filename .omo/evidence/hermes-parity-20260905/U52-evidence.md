# U52: Durable Cutover Job Ownership

## Registration Before Production Edits

### Scenario 1: C01 Cutover Job Ownership Survives Startup Sync
Exact integration test id: `cutover_survives_startup_sync` (crate `test_migrate`, root module).
Literal RED and GREEN command: `cargo test --test test_migrate cutover_survives_startup_sync -- --exact --nocapture`

Payload: Daily job `{"id":"daily","prompt":"status","schedule":{"kind":"cron","expr":"0 9 * * *"}}`.

Regression scenario:
- Migration runs against QA fixture containing the daily job.
- Job is imported into `cron_jobs` with id `hermes:default:daily` (count = 1).
- Hermes source store `cron/jobs.json` is emptied by cutover.
- Production synchronizer (`HermesStoreSynchronizer`) runs against the emptied store on startup.
- Unpatched behavior: Synchronizer queries all jobs where `_omon_hermes_source` matches the store path and, seeing that `live` is empty, deletes the imported job from `cron_jobs`. Count drops from 1 to 0.
- Patched behavior: Cutover durable ownership transitions the imported job to `omon_owned`. Synchronizer filters both overwrite updates and stale deletions to `authority = 'hermes_mirror'`. The job survives two consecutive startup syncs, retains its source provenance tags, and is claimed when due by the scheduler. A narrow scheduler guard atomically blocks claims (both scheduled and manual trigger) while authority is `cutover_pending`.

### Scenario 2: Legacy Pre-Upgrade Imported Row Backfill
Exact integration test id: `test_migration_0022_backfill_legacy_imported_rows` (crate `test_cron_authority_upgrade`, root module).
Literal RED and GREEN command: `cargo test --test test_cron_authority_upgrade -- --exact --nocapture`

Payload:
- Legacy imported job: `id = "hermes:default:daily"`, `payload_json = {"id":"daily","prompt":"status","_omon_hermes_source":"/path/to/jobs.json","schedule":{"kind":"cron","expr":"0 9 * * *"}}`.
- Native job: `id = "native_status"`, `payload_json = {"id":"native_status","prompt":"native system status"}` (no `_omon_hermes_source`).

Regression scenario:
- Database is created under the old schema (migrations 0001 through 0016 applied; no `authority` column in `cron_jobs`).
- Both the legacy imported row and the native row exist in `cron_jobs`.
- Migration 0022 runs to upgrade the schema to add `authority`.
- Unpatched behavior: `ALTER TABLE cron_jobs ADD COLUMN authority ... DEFAULT 'omon_owned'` assigns `omon_owned` to ALL existing rows. The legacy imported row is falsely marked `omon_owned` before cutover, causing future synchronizer runs to silently stop updating or deleting it.
- Patched behavior: Migration 0022 includes an initial backfill:
  `UPDATE cron_jobs SET authority = 'hermes_mirror' WHERE json_extract(payload_json, '$._omon_hermes_source') IS NOT NULL;`
  The legacy imported row is backfilled to `hermes_mirror` (so live source sync continues to update or prune it until verified cutover), while native rows remain `omon_owned`. Future cutovers transition the row through `cutover_pending` to `omon_owned`, and subsequent connections never blanket-reset authority.

## Captured Behavioral RED (Before Production Edits)

### Scenario 1 RED Output
Literal execution:
`cargo test --test test_migrate cutover_survives_startup_sync -- --exact --nocapture`

Output (exit code `101`):
```text
running 1 test
C01 RED check: count after migrate = 1, count after startup sync = 0

thread 'cutover_survives_startup_sync' panicked at tests/test_migrate.rs:404:5:
assertion `left == right` failed: cutover owned job must survive first startup sync (RED: count 1 -> 0)
  left: 0
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test cutover_survives_startup_sync ... FAILED

failures:
    cutover_survives_startup_sync

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.03s
```

### Scenario 2 RED Output
Literal execution:
`cargo test --test test_cron_authority_upgrade -- --exact --nocapture`

Output (exit code `101`):
```text
running 1 test
Upgrade check: legacy_authority=omon_owned, native_authority=omon_owned

thread 'test_migration_0022_backfill_legacy_imported_rows' panicked at tests/test_cron_authority_upgrade.rs:104:5:
assertion `left == right` failed: legacy imported rows with source provenance must be backfilled to hermes_mirror on upgrade (RED: was omon_owned)
  left: "omon_owned"
 right: "hermes_mirror"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test test_migration_0022_backfill_legacy_imported_rows ... FAILED

failures:
    test_migration_0022_backfill_legacy_imported_rows

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Both tests proved genuine non-zero defect behavior before their respective fixes (exit 101).

## Production Implementation

1. `migrations/0022_cron_authority.sql`:
   - Adds `authority` column to `cron_jobs`:
     `ALTER TABLE cron_jobs ADD COLUMN authority TEXT NOT NULL DEFAULT 'omon_owned' CHECK (authority IN ('hermes_mirror', 'cutover_pending', 'omon_owned'));`
   - Backfills legacy imported rows:
     `UPDATE cron_jobs SET authority = 'hermes_mirror' WHERE json_extract(payload_json, '$._omon_hermes_source') IS NOT NULL;`
   - Creates index `idx_cron_jobs_authority ON cron_jobs(authority);`.
   - Preserves all applied migrations (0001-0016, 0024); leaves 0017-0021 and 0023 reserved.
   - Note: Migration 0022 is new and unreleased in this development phase; no production database was migrated outside isolated test fixtures.

2. `src/cron/store.rs`:
   - Introduces `CronAuthority` enum with variants `HermesMirror`, `CutoverPending`, and `OmonOwned`, implementing `Display`, `FromStr`, and serde serialization.
   - Provides public helper functions `get_cron_authority` and `update_cron_authority`.
   - In `HermesStoreSynchronizer::sync_at`:
     - Inserts new rows with `authority = 'hermes_mirror'`.
     - Updates existing rows on conflict only `WHERE cron_jobs.authority = 'hermes_mirror'`, protecting `omon_owned` and `cutover_pending` rows from overwrite.
     - Prunes missing store jobs only `WHERE authority = 'hermes_mirror'`, protecting cutover owned/pending rows from deletion while retaining `_omon_hermes_source` provenance.

3. `src/cron/scheduler.rs`:
   - Adds `authority` column to `CronJob` struct with `#[sqlx(default)]` and `#[serde(default)]` falling back to `"omon_owned"`.
   - Adds helper methods `authority()`, `is_hermes_mirror()`, `is_cutover_pending()`, and `is_omon_owned()` on `CronJob`.
   - Exposes `get_authority` and `set_authority` on `CronScheduler`.
   - Implements narrow scheduler guard:
     - `run_due_jobs`: filters `WHERE ... AND authority != 'cutover_pending'`.
     - `claim_job`: queries job row `AND authority != 'cutover_pending'`, and inserts into `cron_runs` with atomic condition `WHERE id = ? ... AND authority != 'cutover_pending'`.
     - Atomically blocks both scheduled claims and manual trigger (`trigger_job`) when authority is `cutover_pending`.

4. `src/cron/mod.rs`:
   - Re-exports `CronAuthority`, `get_cron_authority`, and `update_cron_authority`.

5. `src/migrate/cron_cutover.rs`:
   - During cutover in `cutover_cron_stores`:
     - Identifies all verified imported jobs.
     - Atomically transitions them to `cutover_pending`.
     - Atomically empties and backs up stores via U50 primitives under `.jobs.lock`.
     - Upon successful replacement of all stores, transitions rows to `omon_owned`.
     - If dry_run is true, performs zero writes or updates.

## Captured GREEN and Verification Controls

### Scenario 1 GREEN
`cargo test --test test_migrate cutover_survives_startup_sync -- --exact --nocapture`

Output (exit code `0`):
```text
running 1 test
C01 RED check: count after migrate = 1, count after startup sync = 1
test cutover_survives_startup_sync ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 0.06s
```

### Scenario 2 GREEN
`cargo test --test test_cron_authority_upgrade -- --exact --nocapture`

Output (exit code `0`):
```text
running 1 test
Upgrade check: legacy_authority=hermes_mirror, native_authority=omon_owned
test test_migration_0022_backfill_legacy_imported_rows ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

All verification criteria proved:
- Actual schema execution from pre-upgrade schema (0001-0016) to 0022.
- Pre-upgrade cron_jobs schema verified to lack `authority` column.
- On upgrade: legacy imported rows with `_omon_hermes_source` backfill to `hermes_mirror`.
- On upgrade: native rows without `_omon_hermes_source` default to `omon_owned`.
- Source provenance retained: `_omon_hermes_source` preserved verbatim.
- Live synchronizer continues updating `hermes_mirror` rows when source changes.
- Live synchronizer prunes `hermes_mirror` rows when removed from live source before cutover.
- Native `omon_owned` rows are never modified or deleted by source sync.
- Post-cutover `omon_owned` rows survive empty store syncs.
- Real cutover ownership is not blanket-reset on subsequent startups.

### Test Verification Matrix

| Command | Result | Exit |
| --- | --- | --- |
| `cargo test --test test_cron_authority_upgrade -- --exact --nocapture` | 1 passed, 0 failed | 0 |
| `cargo test --test test_migrate cutover_survives_startup_sync -- --exact --nocapture` | 1 passed, 0 failed | 0 |
| `cargo test --lib cron` | 55 passed, 0 failed | 0 |
| `git diff --check -- migrations/0022_cron_authority.sql tests/test_cron_authority_upgrade.rs` | Clean, no whitespace/format errors | 0 |

### Diagnostics
LSP diagnostics on touched files:
- `tests/test_cron_authority_upgrade.rs`: No diagnostics found (0 warnings, 0 errors).

## Scope and Boundary Preservation

- Only files within requested scope for this blocker were modified or created:
  - `migrations/0022_cron_authority.sql`
  - `tests/test_cron_authority_upgrade.rs`
  - `.omo/evidence/hermes-parity-20260905/U52-evidence.md`
  - `.omo/evidence/hermes-parity-20260905/U52-commands.json`
- `src/migrate/cron_cutover.rs`, `src/migrate/mod.rs`, `src/cron/scheduler.rs`, and `tests/test_migrate.rs` were NOT touched during this turn, avoiding conflicts with concurrent U53 work.
- Applied migrations (0001-0016, 0024) preserved intact; 0022 initial backfill corrected in the unreleased migration.
- No real launchctl, signals, Discord, production `.env`, or commits executed.
