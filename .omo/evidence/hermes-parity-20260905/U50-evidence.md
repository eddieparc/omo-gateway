# U50 / CFG.C05

## Registration before production edits

Exact integration test id: `secret_files_are_private_and_backup_unique` (crate `test_migrate`, root module).
Literal RED and GREEN command: `umask 022; cargo test --test test_migrate secret_files_are_private_and_backup_unique -- --exact --nocapture`

Payload: temporary 0600 gateway.env containing DEFAULT_MODEL=old; YAML model new and synthetic api_key sentinel-private. OsEnv importer plus a hard link to the old inode proves replacement rather than in-place mutation. Two FakeMigrationEnv imports at the identical injected instant 2026-09-05T12:00:00Z must retain distinct original/intermediate backups. Read-only target injects failure before rename; old bytes must remain and temporary files must be removed.

Expected binary RED: one selected test fails (nonzero exit), backup mode 644 and/or atomic=false, unique=false. GREEN: target/backup modes 600, atomic/unique/failed/intact/no_temps all true. Real filesystem runs under caller umask 022, never changes global process umask in tests, never signals any process.

Monitor tool/executable is not exposed in this child session. Commands are launched in a background subprocess with captured log/status files; completion is observed without sleeps or polling loops.

## Captured behavioral RED (before production edits)

`U50-red.log`, exit `101` (`U50-red.exit`): 1 selected test, 0 passed, 1 failed, 3 filtered out. Output: `C05 target_mode=600 backup_mode=644 atomic=false unique=false failed=true intact=true no_temps=true`. This confirms public backup permissions, in-place target mutation, and overwritten same-time backup, not a compile/zero-test failure.

## Production patch

- `src/migrate/sys.rs:41`: additive fail-closed exclusive-write default preserves existing trait implementors' source compatibility. OsEnv uses create_new with mode 0600, writes and syncs bytes before publishing, and retries only AlreadyExists with a unique suffix. Existing files/symlinks are never opened/truncated by backup creation. FakeMigrationEnv implements exclusive reservation under its filesystem mutex.
- `src/migrate/sys.rs:49`: shared atomic writer creates its private temporary in the target directory, renames only after successful write, removes temporary on rename failure and propagates cleanup failures. OsEnv.write also uses this path.
- `src/migrate/config_import.rs:106`: exclusively reserve backup and return the actual chosen path, then atomically replace target.
- `src/migrate/cron_cutover.rs:215`: use the same exclusive backup and atomic replacement operations while retaining existing jobs lock and stale-byte guard.
- Existing importer unit test explicitly pinned no rename and a direct target write. Corrected those assertions to the required same-directory temporary/rename contract; retained backup, contents, merge and masked-diff assertions. No tests deleted or skipped.

## Captured GREEN and controls

Identical registered command ran GREEN, exit 0 (`U50-green.log`, `U50-green.exit`):

```text
running 1 test
C05 target_mode=600 backup_mode=600 atomic=true unique=true failed=true intact=true no_temps=true
test secret_files_are_private_and_backup_unique ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out
```

The additional filtered test is the OsEnv local-surface control added after RED; the registered test's payload/assertions were unchanged apart from formatting.

| Command | Captured result |
| --- | --- |
| `umask 022; cargo test --test test_migrate` | `U50-integration.log`: 5 passed, 0 failed; exit 0 |
| `cargo test --lib migrate::` | `U50-adjacent.log`: 31 passed, 0 failed; exit 0 |
| `cargo build` | `U50-build.log`: Finished dev profile; exit 0 |
| `umask 022; python3 .omo/evidence/hermes-parity-20260905/U50-cli.py` | `U50-cli.log`: two actual binary migrate --no-cutover invocations exit 0, two distinct same-second backups and target mode 600; exit 0 |
| `git diff --check -- src/migrate/sys.rs src/migrate/config_import.rs src/migrate/cron_cutover.rs tests/test_migrate.rs` | no output, exit 0 |

`private_migration_os_surface_controls` (`tests/test_migrate.rs:75`) runs the production run_migrate_with entry with OsEnv, temporary real cron source, and SQLite: happy full migration imports one job, preserves cron backup bytes, stats config/cron target/backup at 0600. Adversarial preoccupied symlink backup candidates retain sentinel bytes and get distinct new paths. Real rename failure against a nonempty directory preserves its original member and leaves no temporary. Registered regression additionally preserves the original regular target on injected failure before rename. Adjacent suite retains dry-run zero writes, malformed/unimported rejection, stale-byte guard, empty-store idempotence and profile discovery.

CLI proof uses the built production entry, temporary cwd/HOME/HERMES_HOME and SQLite only, not fake OS. It preserves old inode bytes, inspects filesystem permissions and requires both backups. Its observed backups shared timestamp `20260905T094804Z`, one with collision suffix; timing is not an assertion (fixed-time regression covers collision deterministically).

## Verification limitations and cleanup

LSP diagnostics attempted once each for all four touched Rust files: all returned `LSP daemon unreachable ... /Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`. No repair attempted. Cargo tests/build are successful compiler checks, not a claim of LSP success. No monitor API/executable was available; background shell jobs were awaited via bounded macOS kqueue process-exit events, without sleeps or polling loops.

Only added/changed Rust ranges were formatted via rustfmt stdout plus apply_patch after successful behavioral verification; no cargo fmt --all or unrelated formatting. Final diff check is clean. Regression temps use RAII; OsEnv control explicitly closes SQLite and removes temp directory, CLI context manager removes fixture including SQLite. No real signals, launchctl, network, .env, database, dependency/configuration edits or commits. Evidence artifacts are intentionally retained under U50-*.

This worker changed only the four authorized Rust files and U50-* evidence artifacts. Other concurrent/user modifications remain in the workspace; they were not edited, restored, or claimed as this unit's work. Protected agent/backend/config and test_omo_backend changes and baseline.patch were untouched. This evidence proves CFG.C05 only, not other migration/cutover findings or the entire parity goal.

## Child st_01a07120 revalidation

This invocation found the production patch, tests, registration and RED/GREEN artifacts already present; it preserved them without recreating RED or editing production. The historical ordering above is recorded by the existing evidence, not newly witnessed by this invocation.

All six literal commands in U50-commands.json were executed again in order. Captured outputs are U50-recheck-1.log through U50-recheck-6.log, with corresponding .exit files, all exit 0: selected regression 1 passed; integration 5 passed; adjacent migration unit tests 31 passed; build successful; real CLI fixture successful with two backups at 0600 and original inode intact; scoped diff check clean. Temporary CLI fixture was removed. Commands ran as child subprocesses with bounded completion waits, without test sleeps or polling loops; no monitor tool was exposed.

Unlike the historical infrastructure limitation above, this invocation's diagnostics calls returned `No diagnostics found` for each of the four owned Rust files. No infrastructure repair was performed. Only U50 evidence artifacts were written during this revalidation; existing production/test and other workers' changes were preserved.
