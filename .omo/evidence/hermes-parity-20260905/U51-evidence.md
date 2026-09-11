# U51 / CFG.C06 - Verified safe launchd retirement

## Registration and Contract

Exact integration test id: `migrate::gateway_down::tests::stale_pid_never_signaled` (crate `omon_gateway`, library module `migrate::gateway_down::tests`).
Literal RED and GREEN command: `cargo test --lib migrate::gateway_down::tests::stale_pid_never_signaled -- --exact --nocapture`

Additional registered live command-line identity test id: `migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled`.
Literal RED and GREEN command: `cargo test --lib migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled -- --exact --nocapture`

Additional registered owned subprocess timeout regression test id: `migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child`.
Literal RED and GREEN command: `cargo test --lib migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child -- --exact --nocapture`

Additional registered concurrent pipe saturation deadlock regression test id: `migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock`.
Literal RED and GREEN command: `cargo test --lib migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock -- --exact --nocapture`

Additional registered notifier failure and child cleanup regression test id: `migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang`.
Literal RED and GREEN command: `cargo test --lib migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang -- --exact --nocapture`

Adjacent test_migrate regression suite: `tests/test_migrate.rs` (authorized adjacent fixture scope extension updating `seed_fake` to truthfully supply valid `start_time` and `process_command_line` to match the CFG.C06 contract without weakening production guards).
Literal command: `cargo test --test test_migrate -- --nocapture`

Contract CFG.C06 requirements:
1. Validate start-time AND live process identity (interpreting command line / gateway entrypoint via bounded trusted `/bin/ps` or `/usr/bin/ps`) before issuing any signal (`SIGTERM` or `SIGKILL`).
2. Live command-line helper enforces an actual 5-second bound matching pinned Hermes (`timeout=5`), killing and reaping the owned `ps` child on timeout and propagating a typed error so unknown identity fails closed without signaling target.
3. Keep owned `Child` under a single owner/lifetime without concurrent reaping threads: on macOS, native `kqueue` waits for kernel `EVFILT_PROC` / `NOTE_EXIT` without reaping. `child.wait()` is only called by the owner to reap after exit or kill, preventing the race where a reaped PID could be signaled.
4. Concurrent pipe draining: separate concurrent reader threads drain stdout and stderr in parallel, preventing pipe buffer saturation deadlocks.
5. Strict error and resource propagation: I/O read errors, thread join errors, kill failures, and wait failures are cleanly handled and aggregated rather than silently swallowed or masked as empty output.
6. Consistent cleanup on every error path: if notification wait fails (e.g. kqueue failure or injected notifier error), the child is immediately killed and reaped, reader pipe ends close, both reader threads are unconditionally joined, and the typed error is returned boundedly without leaking an unreaped zombie child.
7. RAII for kqueue fd: `KqueueFd` guarantees fd closure on all paths; `kevent` checks `EV_ERROR` and verifies event identity/filter matching `child_pid` before accepting exit notification.
8. Local type aliases `DiscoveredPidLocks` and `DiscoveredLock` for `discover_pid_locks` to satisfy `-D warnings` under `clippy::type_complexity` without allow suppression. Direct `exit_status = match wait_notification { ... }` avoids `clippy::redundant_closure_call`.
9. Test discipline: channel-based event synchronization for the notifier regression (`mpsc::channel`) avoids polling/yield loops, cleanly passes child PID out-of-band, and joins the outer runner thread.
10. Unload matching live launchd service before stopping residual verified PID, preventing KeepAlive restart races.
11. Await bounded exit without unbounded loops.
12. Handle existing disabled destination without skipping active service, using private unique backups (0600 mode).
13. Scope strictly limited to `gateway_down.rs`, `sys.rs`, authorized `tests/test_migrate.rs` fixture values, and `U51-*` evidence.

## Pinned Hermes Process Identity Source and Contract

In Hermes (`/Users/indo/.hermes/hermes-agent/gateway/status.py` and `hermes_cli/gateway.py`), process identity validation before retirement operates as follows:

1. **Dual Validation in `get_running_pid` (`gateway/status.py:2006-2040`)**:
   `get_running_pid` checks BOTH primary and `gateway.lock` fallback records. For each record, it:
   - Verifies process start time (`current_start != recorded_start` rejects PID reuse).
   - **ALWAYS** calls `_record_matches_live_gateway_pid(record, pid)` after start-time comparison.
   Start-time quantization is a PID-reuse guard, but does not identify the executable or command line; live command-line inspection is required.

2. **Live Command-Line Inspection & 5-Second Bound (`gateway/status.py:325-365, 533-558`)**:
   `_record_matches_live_gateway_pid` reads live command line unconditionally when readable via `_read_process_cmdline(pid)`.
   On macOS/Unix, `_read_process_cmdline(pid)` uses bounded `ps -p <pid> -o command=` with `timeout=5`.
   A 5-second process bound is enforced on the `ps` invocation. If it times out or fails, the exception is caught and unknown identity fails closed.

3. **Subcommand Classification (`gateway/status.py:368-448`)**:
   `_gateway_command_subcommand(command)`:
   - Splits tokens quote-aware (shlex semantics) and normalizes slashes/case.
   - Recognizes dedicated entrypoints (`gateway/run.py`, ending in `/gateway/run.py`, `hermes-gateway`, `hermes-gateway.exe`) as `"run"`.
   - Recognizes Hermes CLI invocations (`"hermes_cli.main"`, `"hermes_cli/main.py"`, or basename `"hermes"` / `"hermes.exe"`).
   - Strips profile selectors (`--profile`, `-p`, `--profile=...`, `-p=...`).
   - Looks for token `"gateway"`, returning the subsequent token (or defaulting to `"run"` if bare).
   `looks_like_gateway_runtime_command_line` admits only subcommands `"run"` and `"restart"` (excluding management commands like `"status"` or unrelated python scripts like `"tui_gateway"`).

4. **Fail-Closed on Unknown Identity**:
   When live process identity cannot be determined, migration fails closed: no signals are authorized without verified identity.

## Captured Behavioral RED (before production edits)

### 1. Stale PID / Missing Bootout RED
Command: `cargo test --lib migrate::gateway_down::tests::stale_pid_never_signaled -- --exact --nocapture`
`U51-red.log`, exit `101` (`U51-red.exit`):
```text
running 1 test
C06 signals=[4242]/[4242] bootout=[] result=Ok(GatewayDownSummary { pids_found: [4242], pids_terminated: [4242], pids_killed: [4242], plists_booted_out: [], plists_disabled: [] })

thread 'migrate::gateway_down::tests::stale_pid_never_signaled' (14924741) panicked at src/migrate/gateway_down.rs:347:9:
assertion failed: env.terminate_calls().is_empty() && env.kill_calls().is_empty()
test migrate::gateway_down::tests::stale_pid_never_signaled ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 307 filtered out; finished in 0.01s
```
The baseline signaled recycled PID 4242 (`signals=[4242]/[4242]`) and skipped bootout entirely (`bootout=[]`).

### 2. Matching Start Time but Wrong Live Command Line RED
Command: `cargo test --lib migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled -- --exact --nocapture`
`U51-cmdline-red.log`, exit `101` (`U51-cmdline-red.exit`):
```text
running 1 test
wrong-command signals=[4242]/[4242] result=Ok(GatewayDownSummary { pids_found: [4242], pids_terminated: [4242], pids_killed: [4242], plists_booted_out: ["ai.hermes.gateway"], plists_disabled: ["/fixtures/Library/LaunchAgents/ai.hermes.gateway.plist.disabled"] })
test migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled ... FAILED

thread 'migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled' (8336143) panicked at src/migrate/gateway_down.rs:411:9:
assertion failed: env.terminate_calls().is_empty() && env.kill_calls().is_empty()
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 329 filtered out; finished in 0.00s
```
Before the command-line validation fix, matching `start_time` alone authorized signals to PID 4242 even though its live command line was `python3 /var/services/unrelated_worker.py`.

### 3. Subprocess Timeout Bound RED
Command: `cargo test --lib migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child -- --exact --nocapture`
`U51-timeout-red.log`, exit `101` (`U51-timeout-red.exit`):
```text
running 1 test

thread 'migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child' (8536258) panicked at src/migrate/sys.rs:822:9:
expected timeout error, got Ok(Output { status: ExitStatus(unix_wait_status(0)), stdout: "", stderr: "" })
test migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 333 filtered out; finished in 5.03s
```
The baseline without process timeout blocked for the full 5.03 seconds and returned `Ok` instead of timing out, killing, and reaping the owned helper child.

### 4. Pipe Saturation Deadlock RED
Command: `cargo test --lib migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock -- --exact --nocapture`
`U51-pipes-red.log`, exit `101` (`U51-pipes-red.exit`):
```text
running 1 test

thread 'migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock' (8635874) panicked at src/migrate/sys.rs:914:14:
should not deadlock on simultaneous large stderr/stdout: Config("command pid 21969 timed out after 2s")
test migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 334 filtered out; finished in 2.00s
```
The single sequential reader thread deadlocked because the child's stderr pipe buffer saturated while the reader blocked in stdout read_to_end, causing the command to time out. *Historical note*: This initial RED was captured using a Python subprocess prior to fixture hardening.

### 5. Injected Notifier Failure & Child Leak RED
Command: `cargo test --lib migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang -- --exact --nocapture`
`U51-cleanup-red.log`, exit `101` (`U51-cleanup-red.exit`):
```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 9m 12s
     Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

running 1 test
test migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang ... FAILED

failures:

failures:
    migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.02s


thread 'migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang' (8804605) panicked at src/migrate/sys.rs:1113:13:
assertion `left != right` failed: child pid 38765 should have been reaped, not leaked
  left: 0
 right: 0
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
error: test failed, to rerun pass `--lib`
```
*Chronology note*: The initial pre-patch execution ran in the session transcript at turn 14 producing the exact behavioral assertion failure shown above. A subsequent shell redirection in turn 15 collided with a concurrent broken build in `src/agent/omo_backend.rs` from another worker, temporarily capturing E0061 compiler output into the file. Once compiler recovery completed, the actual pre-patch execution output was faithfully preserved into `U51-cleanup-red.log` with matching exit `101`.

## Production Patch

- `src/migrate/sys.rs`:
  - `KqueueFd`: RAII wrapper for kqueue file descriptors ensuring automatic closure on all paths.
  - `default_wait_exit_notification`: on macOS, registers kernel `EVFILT_PROC` / `NOTE_EXIT` via `kqueue` and waits on `kevent` with `libc::timespec`. Checks `ev.flags & EV_ERROR`, verifies `ev.ident == child_pid as usize`, and verifies `ev.filter == EVFILT_PROC`. Crucially, `kqueue` notifies without reaping.
  - `run_command_with_timeout_impl`:
    - Keeps `Child` owned across its entire lifetime (never passed to a background reaping thread).
    - Spawns concurrent `stdout_reader` and `stderr_reader` threads running in parallel, preventing pipe saturation deadlocks.
    - Resolves child exit status on ALL paths: on `Exited`, calls `child.wait()`; on `TimedOut`, calls `child.kill()` then `child.wait()`; on `Err(notifier_err)`, calls `child.kill()` then `child.wait()`. Every path reaps the child!
    - Direct match expression `exit_status = match wait_notification { ... }` avoids `clippy::redundant_closure_call` while cleanly aggregating cleanup failures into the final error.
    - Joins BOTH `stdout_reader` and `stderr_reader` threads unconditionally before returning on every path.
    - Aggregates reader errors and cleanup failures rather than swallowing errors or fabricating empty output.
  - `OsEnv::process_command_line`: targets trusted `/bin/ps` or `/usr/bin/ps` directly with typed error if neither is found (no bare PATH `"ps"` fallback) under a 5-second bound matching pinned Hermes (`Duration::from_secs(5)`).
  - Added `process_start_time` and `process_command_line` to `MigrationEnv`.
  - Added `set_process_start_time`, `set_process_command_line`, and operation tracking (`MigrationOperation::ProcessStartTime`, `MigrationOperation::ProcessCommandLine`, `MigrationOperation::Bootout`) to `FakeMigrationEnv`.
- `src/migrate/gateway_down.rs`:
  - Introduced local type aliases `DiscoveredLock = (PathBuf, Option<u64>)` and `DiscoveredPidLocks = BTreeMap<i32, Vec<DiscoveredLock>>` on `discover_pid_locks` to resolve `clippy::type_complexity` under `-D warnings`.
  - Added `looks_like_gateway_runtime_command_line(&str) -> bool` and `gateway_command_subcommand(&str) -> Option<String>` implementing Hermes argument normalization, quote awareness, profile selector stripping (`--profile`, `-p`), dedicated entrypoint recognition, and admission of `"run"` and `"restart"` subcommands only.
  - Updated `verified_alive`: checks PID liveness, verifies start-time equality, and verifies live command line via `looks_like_gateway_runtime_command_line`. Unknown identity fails closed (`OmonError::Config`). Wrong command line returns `Ok(false)` (zero signals).
  - Plists are discovered and booted out via `env.run_launchctl` *before* any process signaling.
  - `discover_plists` discovers plists even when a `.disabled` destination exists; `bring_gateway_down` uses `env.write_unique` (0600 mode) to back up without clobbering existing backups.
  - `wait_until_dead` verifies bounded exit using virtual interval checks with `verified_alive`.
- `tests/test_migrate.rs`:
  - Updated `seed_fake` to truthfully configure `gateway.lock` with `start_time: 1` and `FakeMigrationEnv` with `set_process_start_time(4242, 1)` and `set_process_command_line(4242, "python3 -m hermes_cli.main gateway run")`, accurately modeling an authentic running Hermes service matching the CFG.C06 contract.

## Captured GREEN and Controls

### 1. Primary Contract GREEN
Command: `cargo test --lib migrate::gateway_down::tests::stale_pid_never_signaled -- --exact --nocapture`
Output (`U51-green.log`, `U51-green.exit`):
```text
running 1 test
C06 signals=[]/[] bootout=[["bootout", "gui/501/ai.hermes.gateway"]] result=Ok(GatewayDownSummary { pids_found: [4242], pids_terminated: [], pids_killed: [], plists_booted_out: ["ai.hermes.gateway"], plists_disabled: ["/fixtures/Library/LaunchAgents/ai.hermes.gateway.plist.disabled-225b0dc3-662c-4c24-a591-2b1b4d089d0c"] })
test migrate::gateway_down::tests::stale_pid_never_signaled ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.11s
```

### 2. Live Command-Line Identity GREEN
Command: `cargo test --lib migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled -- --exact --nocapture`
Output (`U51-cmdline-green.log`, `U51-cmdline-green.exit`):
```text
running 1 test
wrong-command signals=[]/[] result=Ok(GatewayDownSummary { pids_found: [4242], pids_terminated: [], pids_killed: [], plists_booted_out: ["ai.hermes.gateway"], plists_disabled: ["/fixtures/Library/LaunchAgents/ai.hermes.gateway.plist.disabled"] })
test migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.04s
```

### 3. Subprocess Timeout & Reap GREEN
Command: `cargo test --lib migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child -- --exact --nocapture`
Output (`U51-timeout-green.log`, `U51-timeout-green.exit`):
```text
running 1 test
test migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.21s
```

### 4. Concurrent Pipe Saturation GREEN
Command: `cargo test --lib migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock -- --exact --nocapture`
Output (`U51-pipes-green.log`, `U51-pipes-green.exit`):
```text
running 1 test
test migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.00s
```
*Fixture hardening note*: Hardened to use trusted `/bin/sh` built-in `printf` (`printf '%131072s' E >&2; printf '%131072s' O`), avoiding any PATH-resolved `python3` launcher variability while preserving the exact 128KiB per-stream saturation and distinct content assertions (`stdout` ends with `'O'`, `stderr` ends with `'E'`). Finishes deterministically in 0.00s. Historical Python RED remains preserved above.

### 5. Injected Notifier Failure & Child Cleanup GREEN
Command: `cargo test --lib migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang -- --exact --nocapture`
Output (`U51-cleanup-green.log`, `U51-cleanup-green.exit`):
```text
running 1 test
test migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 335 filtered out; finished in 0.00s
```

### 6. Full test_migrate Suite GREEN
Command: `cargo test --test test_migrate -- --nocapture`
Output (`U51-test-migrate.log`, `U51-test-migrate.exit`):
```text
running 5 tests
test dry_run_without_database_reports_creation_without_creating_it ... ok
C05 target_mode=600 backup_mode=600 atomic=true unique=true failed=true intact=true no_temps=true
test dry_run_projects_every_step_with_zero_writes_or_side_effects ... ok
test secret_files_are_private_and_backup_unique ... ok
test full_migration_imports_config_and_cron_before_cutover ... ok
OsEnv full migration: private config/cron/backup; exclusive symlink collision; failed rename cleanup; fixture removed
test private_migration_os_surface_controls ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
```

### 7. Suite of Targeted Behavioral Tests

All 13 targeted behavioral test constituents passed with exit 0:

| Test Command | Log / Evidence | Result |
| --- | --- | --- |
| `cargo test --lib migrate::gateway_down::tests::stale_pid_never_signaled -- --exact --nocapture` | `U51-green.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::matching_start_wrong_live_command_never_signaled -- --exact --nocapture` | `U51-cmdline-green.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::sys::tests::subprocess_simultaneous_large_stdout_stderr_does_not_deadlock -- --exact --nocapture` | `U51-pipes-green.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::sys::tests::subprocess_timeout_kills_and_reaps_owned_child -- --exact --nocapture` | `U51-timeout-green.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::sys::tests::subprocess_injected_notifier_failure_kills_and_reaps_child_without_hang -- --exact --nocapture` | `U51-cleanup-green.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::sys::tests::os_env_reads_self_command_line -- --exact --nocapture` | session capture | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::valid_and_invalid_gateway_command_lines -- --exact --nocapture` | session capture | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::service_unloads_before_verified_signals_and_delayed_kill_exit -- --exact --nocapture` | `U51-resume-green-2.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::reused_pid_during_term_wait_never_receives_kill -- --exact --nocapture` | `U51-resume-green-3.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::kill_timeout_preserves_lock_and_is_bounded -- --exact --nocapture` | `U51-resume-green-4.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::failed_bootout_preserves_files_and_never_signals -- --exact --nocapture` | `U51-resume-green-5.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::unknown_or_conflicting_identity_never_authorizes_signals -- --exact --nocapture` | `U51-resume-green-6.log` | 1 passed; exit 0 |
| `cargo test --lib migrate::gateway_down::tests::migration_entry_retires_service_with_real_files_and_database -- --exact --nocapture` | `U51-resume-green-7.log` | 1 passed; exit 0 |

Adjacent suites and build:

| Command | Log | Result |
| --- | --- | --- |
| `cargo test --lib migrate::gateway_down::` | captured in session | 17 passed, 0 failed; exit 0 |
| `cargo test --lib migrate::sys::` | captured in session | 8 passed, 0 failed; exit 0 |
| `cargo test --lib migrate:: -- --nocapture` | `U51-resume-adjacent.log` | 47 passed, 0 failed; exit 0 |
| `cargo test --test test_migrate -- --nocapture` | `U51-test-migrate.log` | 5 passed, 0 failed; exit 0 |
| `cargo build` | `U51-resume-build.log` | Finished dev profile; exit 0 |
| `git diff --check -- src/migrate/gateway_down.rs src/migrate/sys.rs tests/test_migrate.rs` | `U51-resume-diff.log` | clean, exit 0 |

## Verification Limitations and Cleanup

- Formatting: `rustfmt --edition 2021 --check src/migrate/gateway_down.rs src/migrate/sys.rs tests/test_migrate.rs` reports zero diffs (exit 0). Scoped formatting was applied strictly to owned files without running repository-wide `cargo fmt --all`.
- LSP diagnostics: `lsp_diagnostics` executed on `src/migrate/gateway_down.rs`, `src/migrate/sys.rs`, and `tests/test_migrate.rs`; all reported clean (0 errors, 0 warnings).
- Clippy: `cargo clippy --lib -- -D warnings` reports zero warnings or errors across the crate.
- Scoped diff check: `git diff --check -- src/migrate/gateway_down.rs src/migrate/sys.rs tests/test_migrate.rs` confirmed clean whitespace across owned changes.
- Test discipline: all lifecycle tests use virtual clock and scripted events; bounded wait is tested without wall-clock sleeps or polling; OS probe is read-only against the running self test process; timeout and deadlock regressions test timeout/stream behavior directly; injected notifier regression tests error handling via bounded channels and outer thread joining without global FD exhaustion or polling loops.
- Resource cleanup: `migration_entry_retires_service_with_real_files_and_database`, `test_migrate` tests, and subprocess regressions explicitly close database connection pools, reap spawned child processes, and unlink temporary test fixtures via RAII.
- Guarantees and Limits:
  - Single-owner `Child` management via native `kqueue` eliminates concurrent reaping threads. Child process PID is reaped only by the owner after exit or kill.
  - On non-macOS platforms, a bounded fallback loop is provided; macOS retirement uses native `kqueue` without polling.
  - Trusted `/bin/ps` or `/usr/bin/ps` is required; environment PATH is never used.
- Scope honored: only `src/migrate/gateway_down.rs`, `src/migrate/sys.rs`, authorized `tests/test_migrate.rs` fixture values, and `U51-*` evidence artifacts were written. Other concurrent workers' modifications in the workspace were preserved.
