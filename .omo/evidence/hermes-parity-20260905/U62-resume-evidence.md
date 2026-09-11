# U62 / R.R06 Resumed Lifecycle Fixture Repair Evidence

## 1. Chronology and Resumed Context

- Resumed child `hephaestus` (task ID `st_01a07788`, parent `01a071f6-cc78-7761-86ab-a931f8c15133`).
- Two prior worker sessions were interrupted by provider rate limits / quotas while investigating historical test timeout failures on the full `agent::omo_daemon::tests` target.
- Historical baseline status: 7 tests passed, 3 failed with timeout `Elapsed(())` waiting on child startup events:
  1. `daemon_ensure_owned_local_surface` (panicked on `timeout(Duration::from_secs(10), OmoDaemonSupervisor::ensure(&cfg))`)
  2. `daemon_exiting_child_budget_local_surface` (panicked on `timeout(Duration::from_secs(10), events.accept())`)
  3. `daemon_restart_budget_and_unready_child` (panicked on `timeout(Duration::from_secs(10), started)`)
- Prior workers completed root-cause tracing:
  - Traced in `U62-resume-cargo-env-trace.log`, `U62-resume-launch-comparison.log`, and `U62-resume-exec-comparison.log`.
  - The shebang `#!/usr/bin/env python3` in temporary fixture scripts caused macOS to invoke the pyenv shim (`/Users/indo/.pyenv/shims/python3`).
  - The pyenv shim initiated a bash script that traversed directory hierarchies searching for `.python-version`, inspected pyenv hook plugins, loaded `pyenv-realpath.dylib`, and executed `pyenv-exec`.
  - Direct executable execution (`/usr/bin/python3`) completed in 0.371s, whereas the pyenv wrapper took over 4.3s in isolation and exceeded 10s when running under parallel test load with dyld initialization and file locking.
  - Diagnostic compilation error E0505 on fixture log dump was already resolved by borrowing `&path`.
  - Production supervisor algorithms (sliding-window restart budget, readyz probing, unready replacement kill/reap) were correct; the root cause was deterministic fixture launcher latency.

## 2. Fixture Repair Implementation & Signature Clean-Up

- File modified: `src/agent/omo_daemon.rs` only.
- In all fixture scripts (`daemon_exiting_child_budget_local_surface`, `daemon_ensure_owned_local_surface`, and `daemon_restart_budget_and_unready_child`), replaced `#!/usr/bin/env python3` with a polyglot shell/Python bootstrap header:
  ```sh
  #!/bin/sh
  ''''test -x /opt/homebrew/bin/python3 && exec /opt/homebrew/bin/python3 "$0" "$@"; exec /usr/bin/python3 "$0" "$@" # '''
  ```
- This directly invokes the native Homebrew or system Python binary without invoking `env`, skipping pyenv shim traversal, directory crawling, and bash wrapper overhead.
- **`spawn_watcher` Signature Audit and Reversion**:
  - Review of the diff against `U62-resume-before.rs` identified an inherited signature-only change: `fn spawn_watcher(&self, url: String, bin: String) -> tokio::task::JoinHandle<()>`.
  - Provenance: A previous worker experimented with joining the spawned watcher task during test teardown investigation, but abandoned that test seam in favor of event socket handshakes and process waits.
  - Usage: Zero callers (neither production `ensure` nor any of the test cases) consumed or assigned the returned `JoinHandle<()>`; all 5 call sites invoked `spawn_watcher` as a statement for side effects.
  - Resolution: The unused return type and naked expression were removed, restoring `fn spawn_watcher(&self, url: String, bin: String)` with trailing `;` on the `tokio::spawn` block. Production code matches `U62-resume-before.rs` exactly.
- No production algorithms, timers, or assertions were weakened or enlarged:
  - `READY_PROBE_TIMEOUT`: 2 seconds
  - `READY_WAIT_AFTER_SPAWN`: 30 seconds
  - `RESTART_BACKOFF`: 2 seconds
  - `RESTART_WINDOW`: 60 seconds
  - `MAX_RESTARTS`: 3 attempts
- Real unready-child kill/reap and sliding-window restart enforcement remain exact.

## 3. Retrospective RED Chronology Limitation

- As noted in original `U62-evidence.md`, the exit-on-every-start fixture (`daemon_exiting_child_budget_local_surface`) was introduced as post-patch GREEN local-surface coverage.
- Its independent pre-patch behavioral RED was never captured before the production changes were applied; the pre-patch budget RED was captured via `daemon_restart_budget_stops_spawn_failures` (`U62-budget-red.log`, exit 101).
- This retrospective limitation is retained explicitly; old source-copy proofs are not relabeled as preproduction RED.

## 4. Lifecycle Invariants and Isolation Guarantees

- **Bounded Restart Budget**: Enforces a maximum of 3 restarts within a sliding 60-second window. A 2-second backoff is inserted before each restart attempt.
- **Unready Child Termination**: If a spawned replacement daemon does not satisfy `probe_readyz` within the 30-second deadline (`READY_WAIT_AFTER_SPAWN`), the child process is immediately killed via `child.kill().await`, the slot is reaped, and `shutdown` is marked true, transitioning `is_available()` to false.
- **External Ownership**: `probe_readyz` checks whether an external process already owns the port before any spawn attempt. If ready, supervisor returns `Ok(None)` without spawning or interfering with the external daemon.
- **Private HOME Logging Isolation**: `isolated_home` executes each test inside a dedicated subprocess with an isolated `TempDir` assigned to `HOME`. It verifies that `omo-appserver-*.log` is written to the isolated `.omon` directory and that the temporary directory is completely removed on test completion.
- **Real Subprocess & Socket Seams**: Tests bind real loopback `tokio::net::TcpListener` sockets and spawn real OS subprocesses via `tokio::process::Command`. Event sockets are subscribed before triggers to eliminate race conditions. Time-advancement behaviors use `tokio::time::pause()` and `tokio::time::advance()` deterministically without arbitrary sleeps.

## 5. Verification Results

### Pre-Build Diagnostics
- `lsp_diagnostics` on `src/agent/omo_daemon.rs`: clean (0 errors, 0 warnings).

### Code Formatting
- `rustfmt --edition 2021 --check src/agent/omo_daemon.rs`: clean (exit 0, no formatting diff).

### Library Build
- `cargo build --lib`: exit 0.

### Exact Test Proofs
1. `cargo test --lib agent::omo_daemon::tests::daemon_ensure_owned_local_surface -- --exact --nocapture`
   - Output: `isolated agent::omo_daemon::tests::daemon_ensure_owned_local_surface: exit status: 0`
   - Subprocess reaped, HOME removed, public ensure verified HTTP-200 child ready, shutdown child reaped, socket refused.
   - Result: ok. 1 passed; 0 failed; finished in 0.81s (exit 0).

2. `cargo test --lib agent::omo_daemon::tests::daemon_exiting_child_budget_local_surface -- --exact --nocapture`
   - Output: `isolated agent::omo_daemon::tests::daemon_exiting_child_budget_local_surface: exit status: 0`
   - Bounded 3 restarts observed, exit socket handshake verified, child reaped, shutdown confirmed.
   - Result: ok. 1 passed; 0 failed; finished in 11.33s (exit 0).

3. `cargo test --lib agent::omo_daemon::tests::daemon_restart_budget_and_unready_child -- --exact --nocapture`
   - Output: `isolated agent::omo_daemon::tests::daemon_restart_budget_and_unready_child: exit status: 0`
   - HTTP-503 unready child killed after readiness deadline, socket EOF confirmed, supervisor unavailable.
   - Result: ok. 1 passed; 0 failed; finished in 2.50s (exit 0).

### Full Target Verification
- Command: `cargo test --lib agent::omo_daemon::tests -- --nocapture`
- Output:
  ```
  running 10 tests
  test agent::omo_daemon::tests::test_resolve_daemon_bin_falls_back_to_known_install_paths ... ok
  test agent::omo_daemon::tests::test_is_local_url ... ok
  test agent::omo_daemon::tests::test_probe_readyz_detects_http_200_and_refusal ... ok
  test agent::omo_daemon::tests::test_daemon_command_arguments ... ok
  test agent::omo_daemon::tests::daemon_ensure_external_local_surface ... ok
  test agent::omo_daemon::tests::daemon_restart_budget_stops_spawn_failures ... ok
  test agent::omo_daemon::tests::daemon_ensure_owned_local_surface ... ok
  test agent::omo_daemon::tests::daemon_external_takeover_is_untouched ... ok
  test agent::omo_daemon::tests::daemon_restart_budget_and_unready_child ... ok
  test agent::omo_daemon::tests::daemon_exiting_child_budget_local_surface ... ok

  test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 319 filtered out; finished in 12.36s
  ```
- Exit status: 0.

## 6. Resource and Artifact Cleanup

- **Subprocesses**: All child processes spawned during tests were explicitly waited on, killed, and reaped. Process table audit confirmed no leftover test daemons or Python scripts.
- **Filesystem**: User's production directory `~/.omon` was audited before and after test execution; no files were added or modified. All test-generated log files were confined to temporary directories managed by `tempfile::tempdir()` and removed upon child exit.
- **Scope Discipline**: Only `src/agent/omo_daemon.rs` and `U62-resume-*` evidence artifacts were touched. No git commits, no edits to `.env`, dependencies, or files owned by other parallel workers.
