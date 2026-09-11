# Independent Verification: U52 / U53 / U54 Hermes Cron Cutover & Parity

**Date:** 2026-09-08  
**Verifier:** hephaestus (OmO senpi-task child `st_01a07c58`)  
**Workspace:** `/Users/indo/code/project/omon-gateway`  
**Scope:** U52 (Durable Cron Ownership), U53 (Quiesced Receipt-Verified All-Store Cutover), U54 (Dry-Run Shares Full Import Validation)  
**Deliverables:**
- Report: `.omo/evidence/hermes-parity-20260905/cutover-verification.md`
- Captured Logs: `.omo/evidence/hermes-parity-20260905/cutover-verification-logs.txt`

---

## 1. Executive Summary & Bounded Verdicts

Independent verification of the durable cron cutover, receipt journaling, and dry-run validation subsystems confirms that all design contracts from `shared-design.md` section 5 and unit manifests U52, U53, and U54 are strictly satisfied without regression or simulated shortcuts.

| Unit | Title | Defect Finding | Core Regression Tests | Verdict |
|---|---|---|---|---|
| **U52** | Durable cutover job ownership | `CFG.C01` | `cutover_survives_startup_sync`<br>`test_migration_0022_backfill_legacy_imported_rows` | **PASS** |
| **U53** | Quiesced receipt-verified all-store cutover | `CFG.C07` | `changed_payload_blocks_cutover`<br>`cutover_second_store_failure_preserves_bytes_and_receipt` | **PASS** |
| **U54** | Dry-run shares full import validation | `CFG.C08` | `dry_run_reports_nonimportable_jobs` | **PASS** |

**Aggregate Test Suite Status:**
- `cargo test --test test_migrate`: **9 passed; 0 failed** (0.15s)
- `cargo test --lib migrate`: **49 passed; 0 failed** (0.24s)
- `cargo test --test test_cron_authority_upgrade`: **1 passed; 0 failed** (0.03s)
- Zero source code modifications made during verification.

---

## 2. Unit Verification Analysis

### 2.1 U52: Durable Cutover Job Ownership (`CFG.C01`) — VERDICT: PASS

#### The Defect
Prior to U52, full cutover emptied the Hermes source store (`cron/jobs.json`) while leaving source provenance tags (`_omon_hermes_source`) on imported SQLite rows in `cron_jobs`. The initial normal startup synchronizer (`HermesStoreSynchronizer::sync_at`) queried for all jobs matching `_omon_hermes_source`, observed that the source store contained zero live jobs, and unconditionally executed `DELETE FROM cron_jobs WHERE id = ?`. Consequently, an imported cron schedule was erased immediately upon first startup.

#### Production Implementation Verified
1. **Migration 0022 (`migrations/0022_cron_authority.sql`):**
   - Added `authority TEXT NOT NULL DEFAULT 'omon_owned' CHECK (authority IN ('hermes_mirror', 'cutover_pending', 'omon_owned'))` to `cron_jobs`.
   - Executed retroactive backfill for pre-upgrade databases:
     ```sql
     UPDATE cron_jobs SET authority = 'hermes_mirror'
     WHERE json_extract(payload_json, '$._omon_hermes_source') IS NOT NULL;
     ```
   - Indexed `authority` via `idx_cron_jobs_authority`.
2. **Synchronizer Authority Filtering (`src/cron/store.rs`):**
   - `INSERT ... ON CONFLICT(id) DO UPDATE SET ... WHERE cron_jobs.authority = 'hermes_mirror'`: Prevents live sync from overwriting `omon_owned` or `cutover_pending` rows.
   - Deletion query scoped strictly to mirror rows:
     ```sql
     SELECT id FROM cron_jobs WHERE json_extract(payload_json, '$._omon_hermes_source') = ? AND authority = 'hermes_mirror'
     ```
     `DELETE FROM cron_jobs WHERE id = ? AND authority = 'hermes_mirror'`
3. **Scheduler Authority Guard (`src/cron/scheduler.rs`):**
   - Both `run_due_jobs` and `claim_job` enforce `AND authority != 'cutover_pending'`.
   - Scheduled claims and manual triggers (`trigger_job`) are atomically blocked while authority is `cutover_pending`.

#### Empirical Proof & Output Audit
1. **Survival across two consecutive production startup syncs:**
   - Test: `cargo test --test test_migrate cutover_survives_startup_sync -- --exact --nocapture`
   - Migrated job count = 1, `authority = 'omon_owned'`.
   - Store emptied; synchronizer run 1: `sync1_imported = 0`, `count_after_sync1 = 1`.
   - Synchronizer run 2: `sync2_imported = 0`, `count_after_sync2 = 1`.
   - Overwrite protection: Writing a mutated job with the same ID into `jobs.json` resulted in `sync_overwrite = 0`, and the stored prompt remained unmodified (`"status"`).
   - Provenance retention: `_omon_hermes_source` remained in `payload_json`.
2. **Due-job execution and `cutover_pending` atomic blocking:**
   - Scheduler claimed due job when `authority = 'omon_owned'` (`claimed == 1`).
   - When transitioning to `CronAuthority::CutoverPending`, scheduled claim returned `0` and `scheduler.trigger_job(...)` returned `false`.
   - Restoring `CronAuthority::OmonOwned` unblocked execution (`trigger_job` returned `true`).
3. **Legacy pre-upgrade row backfill:**
   - Test: `cargo test --test test_cron_authority_upgrade test_migration_0022_backfill_legacy_imported_rows -- --exact --nocapture`
   - Pre-upgrade schema (0001-0016) seeded with a legacy imported job (with `_omon_hermes_source`) and a native job.
   - Migration 0022 executed:
     - Native row assigned `authority = 'omon_owned'`.
     - Legacy row backfilled to `authority = 'hermes_mirror'`.
   - Live sync continued updating the legacy row while Hermes source was active, pruned it when removed from source, and never touched the native row.
   - Once marked `omon_owned`, the row survived subsequent empty store synchronizations.

---

### 2.2 U53: Quiesced Receipt-Verified All-Store Cutover (`CFG.C07`) — VERDICT: PASS

#### The Defect
Cutover previously validated only job ID existence in SQLite (`SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id = ?)`). If an external user or process updated a job's prompt or schedule in the source store between sync and cutover, cutover emptied the source store and adopted the stale DB payload, permanently destroying source edits. Furthermore, source services were not quiesced prior to taking snapshots, store locks were not canonicalized/deduplicated, and failure during multi-store rewrites left stores partially destroyed without durable journal receipts.

#### Production Implementation Verified
1. **Migration 0023 (`migrations/0023_cutover_receipts.sql`):**
   - Created `cron_cutover_receipts`: `operation_id`, `status` (`pending`, `committed`, `failed`, `rolled_back`), `store_count`, `policy_digest`, timestamps.
   - Created `cron_cutover_receipt_stores`: `(operation_id, store_path)`, `profile`, `original_hash`, `replacement_hash`, `backup_path`, `job_digests_json`, `phase` (`prepared`, `backed_up`, `replaced`, `committed`).
2. **Canonical Job Payload Validation (`src/migrate/cron_cutover.rs`):**
   - `normalize_job_payload` strips runtime metadata (`_omon_hermes_*`, `last_*`, `created_at`, `updated_at`, `next_run_at`, `repeat.completed`) and normalizes JSON structures.
   - `canonical_job_payload_matches` validates that the source job exactly matches the SQLite DB payload under store lock. Any discrepancy aborts cutover with a typed error without writing backups or touching sources.
3. **Cutover Protocol Orchestration (`src/migrate/mod.rs` & `cron_cutover.rs`):**
   - **Step 1 Quiesce:** `bring_gateway_down` unloads LaunchAgents (`bootout`) and terminates running gateway PIDs before taking snapshots or locking stores.
   - **Step 2 Canonical Locks:** Resolves and canonicalizes all store lock paths (`.jobs.lock`) in sorted `BTreeSet` order, preventing deadlocks from aliased/symlinked stores.
   - **Step 3 Full Snapshot & Canonical Validation:** Reads all source files under lock, computes SHA-256 hashes, and compares all jobs against SQLite.
   - **Step 4 Pre-write Backups & Pending Receipt:** Generates private unique backups for ALL stores. Inserts `cron_cutover_receipts` with status `'pending'` and transitions affected jobs to `authority = 'cutover_pending'` within an atomic transaction.
   - **Step 5 Rewrites & Tracking:** Atomically rewrites stores with empty documents (`jobs: []`), updating each store's phase to `'replaced'`.
   - **Step 6 Commit:** Updates receipt status to `'committed'`, store phase to `'committed'`, and transitions jobs to `authority = 'omon_owned'`.
4. **Crash Recovery (`reconcile_pending_cutover`):**
   - Detects pending cutover operations upon startup.
   - Inspects each store's hash against original and replacement hashes.
   - Rolls forward unreplaced stores matching original hash, or accepts already replaced stores.
   - Refuses if store content is unknown (neither original nor replacement), preserving source and backups.
   - Finalizes commit and marks jobs `omon_owned`.

#### Empirical Proof & Output Audit
1. **Changed payload blocks cutover:**
   - Test: `cargo test --test test_migrate changed_payload_blocks_cutover -- --exact --nocapture`
   - Database had prompt `"old"`; source store updated to prompt `"new"`.
   - Cutover returned `Err(OmonError::Config(...))`.
   - Source store bytes remained strictly intact (`current_source == new_source`).
   - Database row remained unmodified (`"old"`).
   - Zero backups or receipts created; zero source rewrites occurred.
2. **Quiesced multi-store cutover with injected failure & receipt durability:**
   - Test: `cargo test --test test_migrate cutover_second_store_failure_preserves_bytes_and_receipt -- --exact --nocapture`
   - Multi-store configuration: default profile (`cron/jobs.json`) and work profile (`profiles/work/cron/jobs.json`).
   - Injected failure: Work store path marked read-only prior to cutover rewrite.
   - Causal operation order verified via `FakeMigrationEnv::operations()`:
     - `bootout_idx < lock1_idx` (Service unloaded before locking)
     - `term_idx < lock1_idx` (Process terminated before locking)
     - `lock1_idx < lock2_idx` (Locks acquired in canonical sorted order)
     - `default_backup_idx < first_rename_idx` (Backup 1 written before any source rewrite)
     - `work_backup_idx < first_rename_idx` (Backup 2 written before any source rewrite)
     - `first_rename_idx < release1_idx` & `first_rename_idx < release2_idx` (Locks released after failure)
   - Store bytes on failure:
     - Store 1: Replaced (`jobs: []`).
     - Store 2: Original bytes unmutated (`work_current == work_jobs`).
     - Backups for both stores verified with original byte content.
   - Durable receipt in SQLite:
     - `cron_cutover_receipts.status == 'pending'`
     - Store 1 phase: `'replaced'`
     - Store 2 phase: `'backed_up'`
     - Row authority: `cutover_pending`
   - Scheduler execution blocked:
     - Scheduled claims claimed `0` jobs.
     - Manual trigger returned `false`.
   - Repeated reopen safety:
     - Reopen 1: Gateway restarted with Work store still frozen; scheduler still claimed 0 jobs.
     - Reopen 2: Work store unfrozen; `reconcile_pending_cutover` executed.
     - Work store rolled forward and emptied (`jobs: []`).
     - Receipt status transitioned to `'committed'`.
     - Row authority transitioned to `'omon_owned'`.
     - Scheduler successfully claimed both due jobs (`claimed_after_recovery == 2`).

---

### 2.3 U54: Dry-Run Shares Full Import Validation (`CFG.C08`) — VERDICT: PASS

#### The Defect
Previously, dry-run projection parsed IDs only via lightweight regex/JSON scans without verifying schedule expressions, one-shot timestamps, or gateway lifecycle prompt constraints. Furthermore, dry-run ignored profile filtering (`OMON_HERMES_PROFILES`) and unconditionally set `would_empty = true` whenever a store contained any jobs. Consequently, invalid or nonselected jobs appeared as `cron_importable`, and dry-run falsely claimed stores would be emptied when apply would fail or skip them.

#### Production Implementation Verified
1. **Shared Pure Validation Seam (`src/cron/store.rs`):**
   - `HermesJob::validate(&self, default_timezone: Option<&str>, now: DateTime<Utc>) -> Result<ValidatedHermesJob, String>`:
     - Rejects empty job IDs.
     - Enforces prompt and script lifecycle restrictions via `check_gateway_lifecycle`.
     - Parses schedule expressions (`cron`, `interval`, `once`).
     - Validates RFC 3339 timestamps for one-shot jobs.
     - Computes next run time via `next_run_tz` in the effective timezone.
   - Shared identically between `HermesStoreSynchronizer::sync_at` (live apply) and `project_migration` (dry-run).
2. **Profile Selection & Filtering (`src/cron/store.rs` & `src/migrate/mod.rs`):**
   - `HermesStoreSynchronizer::selected_profiles()` inspects `OMON_HERMES_PROFILES`.
   - `project_migration` classifies jobs into distinct categories:
     - `cron_importable`: Valid jobs in selected profiles not already present in SQLite.
     - `cron_already_present`: Valid jobs in selected profiles already in SQLite.
     - `cron_rejected`: Jobs in selected profiles failing `HermesJob::validate`, with structured `CronJobRejection` (id, job_id, profile, reason).
     - `cron_nonselected`: Jobs belonging to nonselected profiles.
3. **Accurate `would_empty` Assessment:**
   - In `project_migration`:
     ```rust
     store.would_empty = is_selected && store.jobs_found > 0 && !has_rejected;
     ```
   - A store is marked `would_empty = false` if it is nonselected OR contains any rejected jobs.
4. **Zero Side Effects Guarantee:**
   - Dry-run performs zero file writes, zero file renames, zero process terminations, zero launchctl commands, and zero database writes.

#### Empirical Proof & Output Audit
1. **Pure validation and classification in dry-run:**
   - Test: `cargo test --test test_migrate dry_run_reports_nonimportable_jobs -- --exact --nocapture`
   - Store 1 (default profile): Job `daily` (valid).
   - Store 2 (work profile):
     - Job `bad`: Missing schedule expression.
     - Job `restart`: Forbidden gateway self-restart prompt (`"hermes gateway restart"`).
     - Job `standup`: Valid job.
   - Profile filter: `OMON_HERMES_PROFILES="work"` (default profile nonselected).
   - Dry-run projection assertions:
     - Zero side effects: `env.write_calls()` unchanged, `env.rename_calls()` unchanged, `env.terminate_calls().is_empty()`, `env.launchctl_calls().is_empty()`, SQLite `COUNT(*) == 0`.
     - Rejections: `summary.cron_rejected` contained `bad` (missing schedule) and `restart` (lifecycle violation).
     - Nonselected: `summary.cron_nonselected` contained `daily`.
     - Importable: `summary.cron_importable` contained strictly `["hermes:work:standup"]`.
     - Store emptying: Work store `would_empty == false` (contained rejected jobs); Default store `would_empty == false` (nonselected profile).
2. **1:1 Parity between dry-run and apply:**
   - Actual import run with `--no-cutover` imported exactly 1 job (`summary.cron_imported == 1`).
   - SQLite table `cron_jobs` contained exactly `["hermes:work:standup"]`, matching `summary.cron_importable` identically.

---

## 3. Registered Regression Test Matrix

All required regression tests were executed directly in the target environment:

```text
====================================================================================================
TEST IDENTIFIER                                                  CRATE / TARGET      RESULT   TIME
====================================================================================================
cutover_survives_startup_sync                                    test_migrate        PASSED   0.03s
changed_payload_blocks_cutover                                   test_migrate        PASSED   0.01s
cutover_second_store_failure_preserves_bytes_and_receipt         test_migrate        PASSED   0.03s
dry_run_reports_nonimportable_jobs                               test_migrate        PASSED   0.04s
test_migration_0022_backfill_legacy_imported_rows                test_cron_upgrade   PASSED   0.03s
cargo test --test test_migrate (9 tests)                         test_migrate        PASSED   0.15s
cargo test --lib migrate (49 tests)                              lib / migrate       PASSED   0.24s
====================================================================================================
```

---

## 4. State Transition & Safety Audit

The test suite and production code were audited to verify that assertions evaluate genuine state transitions rather than shallow ID comparisons:

1. **Payload and Content Verification:**
   - Payload equality is evaluated using deep normalized JSON comparison (`canonical_job_payload_matches`), ignoring volatile operational timestamps and keys while strictly asserting core job definitions (prompts, schedules, scripts, parameters).
   - Changed prompt (`"old"` vs `"new"`) was verified to cause immediate cutover rejection and zero byte mutations.
2. **Filesystem and OS Interactions:**
   - Causal sequence tracking in `FakeMigrationEnv` proved that service unloading precedes process termination, which precedes sorted canonical locking, which precedes all store backups, which precedes source renaming.
   - Injected write failures proved that unwritten stores retain their original bytes, all backups exist on disk with pre-delete byte equality, and pending receipts accurately record per-store progress phases (`'replaced'` vs `'backed_up'`).
3. **Database and Scheduler Integrity:**
   - SQLite authority transitions (`hermes_mirror` -> `cutover_pending` -> `omon_owned`) were directly verified via SQL queries.
   - Scheduler atomic guards were verified against both scheduled polling (`run_due_jobs`) and manual interactions (`trigger_job`).
   - Crash recovery (`reconcile_pending_cutover`) was proven idempotent across multiple reopens and safe against unknown store content.

---

## 5. Conclusion & Stop Condition Verification

All observable verification conditions for units U52, U53, and U54 have been satisfied with passing test executions, exact log captures, and deep source inspection:
- **U52:** PASS
- **U53:** PASS
- **U54:** PASS

All outputs have been durably captured in `.omo/evidence/hermes-parity-20260905/cutover-verification-logs.txt` and `.omo/evidence/hermes-parity-20260905/cutover-verification.md`. No source code edits were performed.
