# U62 / R.R06

## Registered before production edits

Exact test ID: `agent::omo_daemon::tests::daemon_restart_budget_and_unready_child`.

Literal RED/GREEN command: `cargo test --lib agent::omo_daemon::tests::daemon_restart_budget_and_unready_child -- --exact --nocapture`.

Payload: exited owned `/usr/bin/true`; replacement temporary executable runs a real Python loopback HTTP server returning 503 forever, sends PID over a pre-bound event socket. Advance Tokio clock beyond 30-second readiness deadline. Binary failure observable: owned child wait fails to finish (child remains alive) after deadline. Cleanup kills/reaps before assertion on RED. Restart-budget exit-on-start payload and adjacent controls will be recorded separately.

Production untouched at registration. Only new in-module test added. No monitor tool or executable available in this child toolset; commands launched asynchronously with shell output/status files, inspected through tools.

## Behavioral RED captured before production patch

Additional pre-patch registration: `agent::omo_daemon::tests::daemon_restart_budget_stops_spawn_failures`; literal command `cargo test --lib agent::omo_daemon::tests::daemon_restart_budget_stops_spawn_failures -- --exact --nocapture`. Payload absent executable `/nonexistent/U62-daemon`, invalid URL prevents network; virtual 15s drives repeated spawn-error backoff. Expected RED: circuit never stops failed attempts. This adjacent budget case supplements, not replaces, required exiting executable payload.

`U62-budget-red.log`: running 1 test; "restart circuit did not stop failed spawn attempts"; FAILED 0 passed / 1 failed, 294 filtered; exit 101. Captured before production edit.

`U62-red.log`: running 1 test; owned HTTP-503 replacement PID 88382 survived readiness deadline; FAILED, 0 passed / 1 failed, 293 filtered out. `U62-red.exit`: 101. Child killed and reaped by test cleanup. This proves the live-unready constituent of R.R06, not restart-budget enforcement. LSP attempted once: daemon unreachable at `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`; no repair attempted.

## Production patch and GREEN

Only `src/agent/omo_daemon.rs` changed by this worker. Three restart attempts in a sliding 60-second window; every attempt has 2-second backoff, including successfully spawned crashing replacements. Detect replacement exit early. A live unready replacement is killed and awaited after the 30-second readiness deadline, with terminal unavailable publication through `is_available()` and structured tracing. Tokio Instant makes deadlines virtual-clock testable. External takeover remains probe-before-spawn. Existing ensure signature/callers preserved; no configuration/dependency/environment files changed.

Identical registered commands:
- `U62-green.log`, exit 0: running 1 test; `daemon_restart_budget_and_unready_child ... ok`; 1 passed, 0 failed, 295 filtered; 2.67s.
- `U62-budget-green.log`, exit 0: running 1 test; `daemon_restart_budget_stops_spawn_failures ... ok`; 1 passed, 0 failed, 295 filtered; 0.03s.

Final adjacent execution: `cargo test --lib agent::omo_daemon::tests -- --nocapture`; `U62-adjacent.log`, exit 0, 9 passed / 0 failed / 290 filtered, 7.20s. Includes final additional unavailable-state assertion on HTTP-503 regression.

Local surfaces exercised in that run:
- `agent::omo_daemon::tests::daemon_exiting_child_budget_local_surface`: real temporary executable establishes socket, receives release byte, exits on every start; parent awaits each child exit before continuing. Exactly three starts; terminal unavailable; 120 virtual seconds admits no fourth start; last child reaped.
- `agent::omo_daemon::tests::daemon_external_takeover_is_untouched`: real loopback HTTP-200 server, exited child slot, production watcher yields ownership without spawning a missing executable.
- `agent::omo_daemon::tests::daemon_ensure_external_local_surface`: actual public production `ensure` entry against loopback HTTP-200 fixture returns None.
- Existing readyz HTTP200/refusal, argument, locality, and binary-resolution controls pass.

`cargo build --lib`: `U62-build.log`, exit 0, dev build finished in 20.59s. `git diff --check -- src/agent/omo_daemon.rs`: exit 0. One own test typo (`write_all(b'X')`, E0308) was corrected to byte slice before final adjacent pass; not treated as behavioral RED. Failed atomic apply_patch attempts applied no changes.

## Cleanup and limits

Temporary fixture directories use TempDir; live-unready child cleanup awaits kill on RED and production kill/reap on GREEN. Exiting payload waits every child. No fixture daemon found in final process listing (unrelated Java Kotlin daemon was not touched). No .env, Discord, global configuration, commits, or protected user files edited. Background commands monitored via macOS kqueue process-exit subscription with bounded wait; no polling loop. Output files are retained evidence.

Limits for lead: exit-on-every-start fixture was added as GREEN local-surface coverage after production patch; its independent prepatch behavioral RED was not captured. Prepatch budget RED uses failed spawn attempts instead. Actual public ensure is exercised for external ownership, while owned replacement lifecycle is exercised through its actual private watcher/daemon_command, not public ensure auto-spawn. No full gateway binary build/run or dashboard readiness integration claimed; `is_available()` exposes terminal state but consumers outside the owned file are unchanged. The initial startup failure/Drop kill semantics were not expanded. No claim of completing the broader runtime parity goal.

## Lead-requested fixture HOME isolation correction

Lead's finding confirmed: daemon_command opens its log in the supervisor process HOME, including when merely constructing the command. Original fixtures inherited production HOME. Earlier cleanup notes did not account for these filesystem side effects.

Changed only cfg(test) code in src/agent/omo_daemon.rs: all watcher/ensure fixtures and the command-argument test now re-execute their same exact test ID in a bounded child process with a fresh TempDir HOME. Parent process environment is never mutated. The temporary .omon directory is created so production log opening actually executes, and logging fixtures assert that a log exists there. TempDir is explicitly closed after child exit. The spawn-failure virtual clock is paused only inside the isolated child, not around the parent subprocess timeout. No production lifecycle code or U64 redesign changed.

Verification: `cargo test --lib agent::omo_daemon::tests -- --nocapture` passed, exit 0: 9 passed, 0 failed, 292 filtered; each of six isolated child runs selected exactly one test and exited 0. Output: `U62-isolated-green.log` / `.exit`. LSP attempted once this correction: daemon unreachable, no repair. Scoped git diff --check passed. Prior registered RED/GREEN distinction is unchanged; this rerun is fixture-isolation verification, not a new prepatch RED.

HOME audit: `U62-home-before.json` records names, sizes, birthtimes, mtimes and hashes of existing logs. After the isolated test run, the entire matching log inventory and metadata/hash map was identical: no new production-HOME logs or modifications. Removed only empty `~/.omon/omo-appserver-invalid-url.log`: its unique fixture literal and birth second 1788601644 match the captured spawn-failure RED completion. Details: `U62-home-cleanup.json`.

Seven empty ephemeral-port logs (50270, 61640, 60412, 49306, 64518, 49945, 63238) are consistent with prior fixture runs but old output did not record their listen ports; exclusive ownership is not proven, so they were retained rather than guessed at. Production 19742/19743 logs were retained unchanged. No broad cleanup or production-log deletion was performed.
