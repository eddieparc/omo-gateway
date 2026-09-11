# U63 - R.R10 cross-process drain epoch

## Pre-production-edit registration

Scenario: on macOS process A calls public `write_drain_request` in a fresh
temporary directory, exits, and separate process B calls public
`check_drain_requested` with its own `current_instantiation_epoch`.
Exact test ID: `drain_control::tests::drain_marker_cross_process_epoch`.
Literal RED and GREEN command:
`cargo test --lib drain_control::tests::drain_marker_cross_process_epoch -- --exact --nocapture`

Payload: writer-produced JSON with action `drain`, principal
`u63-external-writer`, suppress_notification `true`, actual timestamp and epoch.
Expected behavioral RED: reader prints `detected=None`, helper exits nonzero,
parent regression fails (not zero tests or compilation failure).
Expected GREEN: reader returns active request preserving principal/suppression;
real production DrainWatcher (the entry used by main) notifies its pre-subscribed
receiver. Controls: synthetic old-boot:1 vs current-boot:2 rejected, unknown epoch
accepted, non-drain rejected, empty/malformed accepted, clear idempotent, no temp
files remain. All children exit/are awaited; watcher is aborted and joined.

Only test additions have been applied at registration. Production fallback still
uses a per-process UUID. No public signatures change. Hermes's empty unknown-boot
fallback is the selected minimal fix; known Linux identity remains unchanged.

LSP attempted once: daemon unreachable at
`/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`; no global repair attempted.
No monitor tool is exposed to this child; shell validation uses an asynchronous
subprocess monitor with bounded completion, without sleeps or polling.

## Captured RED (before production edits)

`U63-red.log` captures the literal registered command: compiled successfully,
ran 1 selected test, writer helper passed, reader printed `detected=None` and
failed `same-boot external marker must be active`. Parent: 0 passed, 1 failed;
cargo EXIT_CODE=101. Actual payload epoch:
`boot:5c34e54d-fb4c-4839-8a64-d18c3a3f800e`, requested_at
`2026-09-05T09:38:36.798212+00:00`; remaining fields match registration.

## GREEN and local surface

`U63-green.log`: identical registered cargo command, EXIT_CODE=0; 1 passed,
0 failed. Writer payload now has `epoch:""`, timestamp
`2026-09-05T09:43:40.888343+00:00`. Reader printed `detected=Some(DrainRequest
...)` with matching principal and suppression and `watcher_active=true`.
This runs two real helper subprocesses through public marker functions, then
the real `DrainWatcher::new/receiver/spawn` surface used by `src/main.rs:1414`.
No Discord or whole gateway startup was used; that would exceed this unit's
isolated marker scope. Known synthetic old boot, unknown epoch, non-drain,
legacy, malformed, clear, missing marker, and directory cleanup assertions all
ran in the same selected regression.

`U63-adjacent.log`: `cargo test --lib drain_control::tests -- --nocapture`,
EXIT_CODE=0; 5 passed, 0 failed, including the three unchanged adjacent tests,
helper entry and subprocess regression. The helper without its fixture env is
only an entry stub; actual writer and reader assertions run in the regression's
children, each selecting exactly one helper test.

Production change is only the unknown-identity fallback and its comments;
Linux boot/PID1 parsing, UUID atomic temp filename, signatures and callers are
unchanged. Empty epoch intentionally cannot reject a truly old boot when OS
identity is unavailable; definite known-vs-known synthetic mismatch still rejects.

Formatting check initially requested one test-only line wrap; applied exactly
that wrap, no behavioral change. Final formatting/whitespace results are in
`U63-static.log`. LSP remains unavailable as recorded above. Cargo compiled the
library test target successfully; no whole-repository build/suite is claimed.

Cleanup: helper processes completed and were awaited; watcher aborted/joined;
marker cleared twice, zero remaining directory entries asserted, TempDir closed.
No sleeps, polling tests, real state/config changes, dependency changes, commits,
or edits outside `src/drain_control.rs` and U63 evidence. Other workers/user
changes remain untouched, including all three protected baseline files and
baseline.patch. No programming/Rust/debugging skill was found in the inspected
installed skills directories; broader read-only lookup timed out, no changes.

## Recovery-child revalidation (2026-09-05)

This child found the production patch, regression, registration, and RED/GREEN
logs already present on arrival. It preserved them without reverting existing
production changes. The historical RED above is read from the preserved log,
not claimed as a newly executed RED by this recovery child.

Current execution is captured in `U63-revalidation.log`: the identical fully
qualified registered command passed 1 test (exit 0), the adjacent module passed
5 tests (exit 0), and rustfmt/whitespace checks both exited 0. The reader child
again returned `Some` with epoch empty and printed `watcher_active=true`.
Synthetic mismatch and all cleanup/control assertions ran successfully.
Commands ran through Python asyncio subprocess completion monitoring with a
bounded await; no monitor tool/CLI is exposed, no sleeps or polling were used.

Unlike the earlier recorded LSP outage, this child's single diagnostics call on
`src/drain_control.rs` returned `No diagnostics found`. The current main caller
was read at `src/main.rs:1413-1418`, confirming the fixture uses the same public
watcher entry and receiver-before-spawn ordering. No public API changes or
additional production edits were needed. Only this U63 evidence append and
`U63-revalidation.log` were written by the recovery child; pre-existing user and
concurrent-worker changes were not modified. Whole-gateway startup and the
whole-repository suite remain outside this proof.
