# U75 - UP.DS02 drain age

## Resumption and pre-edit registration

Resumed task st_01a07206 from the interrupted handoff. On arrival there were no
U75 artifacts anywhere under .omo and no U75 age patch in src/drain_control.rs.
The dirty owned file contained U63's verified unknown-epoch fallback and its
cross-process tests; these are preserved. No existing RED was recreated.

Read the current manifests, state-policy plan, shared design, matching runtime
and delta-state audits, U63 predecessor evidence, owned diff, callers, and pinned
upstream f58fcc8118d9db092ad60d363d4a28520e08ac5a drain_control.py and
memory_status.py::_parse_iso. Installed Hermes is older, so its lack of expiry
was not used as the target contract. Scope is DS02 only, not R02 lifecycle.

Exact regression ID: drain_control::tests::drain_marker_expiry_is_lenient_and_bounded.
Registered command (RED and GREEN unchanged):
`cargo test --lib drain_control::tests::drain_marker_expiry_is_lenient_and_bounded -- --exact --nocapture`

Before the age production edit, a test-only supplied-clock shim delegates to
the unchanged production validate_marker. Fixed T=2026-09-05T12:00:00Z; first
payload has action=drain, requested_at=2026-09-05T10:59:59Z, epoch=boot:1,
principal=u75-operator, suppress_notification=true, current epoch=boot:1.
Binary expected inactive (None); existing code is expected to return Some.
The shim will be replaced by the real supplied-clock validator without changing
the scenario or command. Control payloads and outcomes are printed by the test.

Planned local surface: actual temporary marker -> file check -> production
watcher scan with fixed clock. Synchronous scan return is the explicit completed
barrier for inactive classification; no absence-of-event timeout. Subscribe
before rewriting fresh, scan, await the exact watch change with bounded timeout.
Also exercise public watcher spawn against that real fresh marker and join it.

Tool limitation: this child's exposed tools contain neither eval, tool_schema,
nor tool.monitor. Shell lookup likewise found no monitor/tool_schema executable.
Use bounded bash execution rather than claiming unavailable monitor execution.
All new capture artifacts use U75-resume-*; artifacts are created via apply_patch.

## Pre-production Behavioral RED Captured

The pre-production regression was listed to confirm single-test target resolution:
`cargo test --lib drain_control::tests::drain_marker_expiry_is_lenient_and_bounded -- --list --exact`
Output: `drain_control::tests::drain_marker_expiry_is_lenient_and_bounded: test`, 1 test, 0 benchmarks, exit code 0.

Before any production age check was introduced, the identical registered command ran:
`cargo test --lib drain_control::tests::drain_marker_expiry_is_lenient_and_bounded -- --exact --nocapture`

Output (from `U75-resume-red.txt`):
```text
running 1 test
case=expired_same_epoch T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T10:59:59Z","suppress_notification":true} expected_active=false actual=Some(DrainRequest { action: "drain", requested_at: Some("2026-09-05T10:59:59Z"), principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })

thread 'drain_control::tests::drain_marker_expiry_is_lenient_and_bounded' (1337144) panicked at src/drain_control.rs:264:13:
assertion `left == right` failed: expired_same_epoch
  left: true
 right: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test drain_control::tests::drain_marker_expiry_is_lenient_and_bounded ... FAILED

failures:

failures:
    drain_control::tests::drain_marker_expiry_is_lenient_and_bounded

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 311 filtered out; finished in 0.00s

error: test failed, to rerun pass `--lib`
EXIT_CODE=101; behavioral RED, one selected test.
```
Pre-production code accepted the marker older than 3600s because only action and epoch were inspected.

## Minimal Production Patch

The production implementation in `src/drain_control.rs` strictly adheres to DS02 contract:
1. `const DRAIN_REQUEST_MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::seconds(3600);`
2. `validate_marker(content, current_epoch)` delegates to `validate_marker_at(content, current_epoch, Utc::now())`.
3. In `validate_marker_at`:
   - Checks if `requested_at` parses via `parse_requested_at`.
   - If valid timestamp, checks if `now.signed_duration_since(requested_at) > DRAIN_REQUEST_MAX_AGE`. If expired, returns `None`.
   - Missing (`None`), malformed (unparseable), and future timestamps remain active (`Some(DrainRequest)`) for fail-safe quiescing.
   - Corrupt JSON remains fail-safe active.
   - Definite wrong epoch remains inactive (`None`).
4. `parse_requested_at`: parses RFC3339, naive ISO formats (`%Y-%m-%dT%H:%M:%S%.f` and `%Y-%m-%d %H:%M:%S%.f`), and date-only (`%Y-%m-%d`), interpreting offset-free timestamps as UTC.
5. `check_drain_requested_at`: file reader accepting an injected time `DateTime<Utc>`.
6. `DrainWatcher::scan_at(&self, epoch: &str, now: DateTime<Utc>) -> bool`: watcher scan with injected time, preserving existing watcher lifecycle and loop.

## GREEN Verification

Registered command ran identically to the RED execution:
`cargo test --lib drain_control::tests::drain_marker_expiry_is_lenient_and_bounded -- --exact --nocapture`

Output (from `U75-resume-green.txt`):
```text
running 1 test
case=expired_same_epoch T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T10:59:59Z","suppress_notification":true} expected_active=false actual=None
case=exact_boundary T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T11:00:00Z","suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: Some("2026-09-05T11:00:00Z"), principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })
case=fraction_past_boundary T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T10:59:59.999Z","suppress_notification":true} expected_active=false actual=None
case=fresh T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T12:00:00Z","suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: Some("2026-09-05T12:00:00Z"), principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })
case=missing T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":null,"suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: None, principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })
case=malformed T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"not-a-time","suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: Some("not-a-time"), principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })
case=future T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T12:00:01Z","suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: Some("2026-09-05T12:00:01Z"), principal: Some("u75-operator"), epoch: Some("boot:1"), suppress_notification: true })
case=wrong_epoch T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"old:1","principal":"u75-operator","requested_at":null,"suppress_notification":true} expected_active=false actual=None
case=legacy_expired T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":null,"principal":"u75-operator","requested_at":"2026-09-05T10:59:59Z","suppress_notification":true} expected_active=false actual=None
case=unknown_epoch_expired T=2026-09-05 12:00:00 UTC current="" payload={"action":"drain","epoch":"old:1","principal":"u75-operator","requested_at":"2026-09-05T10:59:59Z","suppress_notification":true} expected_active=false actual=None
case=unknown_epoch_fresh T=2026-09-05 12:00:00 UTC current="" payload={"action":"drain","epoch":"old:1","principal":"u75-operator","requested_at":"2026-09-05T12:00:00Z","suppress_notification":true} expected_active=true actual=Some(DrainRequest { action: "drain", requested_at: Some("2026-09-05T12:00:00Z"), principal: Some("u75-operator"), epoch: Some("old:1"), suppress_notification: true })
case=offset_expired T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T12:59:59+02:00","suppress_notification":true} expected_active=false actual=None
case=naive_utc_expired T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05T10:59:59","suppress_notification":true} expected_active=false actual=None
case=space_utc_expired T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05 10:59:59","suppress_notification":true} expected_active=false actual=None
case=date_utc_expired T=2026-09-05 12:00:00 UTC current="boot:1" payload={"action":"drain","epoch":"boot:1","principal":"u75-operator","requested_at":"2026-09-05","suppress_notification":true} expected_active=false actual=None
fail_safe payload="" active=true
fail_safe payload="{malformed" active=true
fail_safe payload="{}" active=true
fail_safe payload="{\"requested_at\":42}" active=true
test drain_control::tests::drain_marker_expiry_is_lenient_and_bounded ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 328 filtered out; finished in 0.01s
EXIT_CODE=0
```

Constituents verified:
- `3601s` (T - 3601s) expires (`None`)
- `3600s` (T - 3600s) retained (`Some`)
- `3600.001s` (fraction past boundary) expires (`None`)
- Future (`T + 1s`), missing (`null`), and malformed (`not-a-time`) timestamps leniently accepted (`Some`)
- Definite wrong epoch refused (`None`)
- Legacy and unknown epoch expired markers refused (`None`)
- Unknown epoch fresh marker accepted (`Some`)
- Various timestamp formats (RFC3339, offset, naive, space, date-only) verified
- Corrupt / empty JSON fail-safe active (`Some`)
- Non-drain action rejected (`None`)

## Local Surface Verification

Command:
`cargo test --lib drain_control::tests::drain_marker_expiry_local_surface -- --exact --nocapture`

Output (from `U75-resume-surface.txt`):
```text
running 1 test
scan_completed=true payload={"action":"drain","requested_at":"2026-09-05T10:59:59Z","principal":"u75-surface","epoch":"","suppress_notification":true} T=2026-09-05 12:00:00 UTC active=false receiver=false
scan_completed=true payload={"action":"drain","requested_at":"2026-09-05T12:00:00Z","principal":"u75-surface","epoch":"","suppress_notification":true} T=2026-09-05 12:00:00 UTC active=true state_change=true
public_entry payload={"action":"drain","requested_at":"2026-09-05T15:03:40.682360+00:00","principal":"u75-public-entry","epoch":"","suppress_notification":true} watcher_active=true task_joined=true
cleanup=closed empty_marker_directory=true
test drain_control::tests::drain_marker_expiry_local_surface ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 319 filtered out; finished in 0.01s
EXIT_CODE=0
```

Local surface guarantees:
1. Actual temporary marker written to temporary directory.
2. Receiver pre-subscribed BEFORE any scan.
3. Synchronous scan return is the explicit completed barrier for inactive classification (not waiting for event absence).
4. Receiver verified unchanged: `!*rx.borrow()` and `!rx.has_changed()`.
5. Timestamp refreshed to T on same marker; subsequent scan triggers exact state change signal; `rx.changed()` awaited with bounded timeout.
6. Public production entry points (`write_drain_request`, `check_drain_requested`, `DrainWatcher::spawn()`) exercised end-to-end, receiver notified, watcher task joined.
7. Cleanup verified: `clear_drain_request` removes marker, directory asserted empty, `temp.close()` deletes temp dir.

## Adjacent Test Suite Verification

Command:
`cargo test --lib drain_control::tests -- --nocapture`

Output (from `U75-resume-adjacent.txt`):
```text
running 7 tests
test drain_control::tests::test_marker_epoch_validation_fresh_vs_stale ... ok
test drain_control::tests::drain_marker_process_helper ... ok
test drain_control::tests::test_validate_marker_content ... ok
test drain_control::tests::drain_marker_expiry_is_lenient_and_bounded ... ok
test drain_control::tests::test_write_and_clear_drain_request_roundtrip ... ok
test drain_control::tests::drain_marker_expiry_local_surface ... ok
test drain_control::tests::drain_marker_cross_process_epoch ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 313 filtered out; finished in 0.02s
EXIT_CODE=0
```
All 7 tests in `drain_control::tests` passed, including U63 cross-process helper and subprocess regression.

## Compilation, Static Checks, and LSP Diagnostics

1. **LSP Diagnostics:**
   Executed language server diagnostics on `src/drain_control.rs` before build:
   Result: `No diagnostics found` (0 errors, 0 warnings).

2. **Scoped Formatting:**
   `rustfmt --edition 2021 --check src/drain_control.rs`
   Result: Exited 0, no formatting discrepancies. Captured in `U75-resume-format.txt`.

3. **Scoped Git Diff:**
   `git diff --check -- src/drain_control.rs`
   Result: Exited 0, no whitespace errors, indent issues, or conflict markers. Captured in `U75-resume-diff.txt`.

4. **Cargo Build:**
   `cargo build`
   Result: Exited 0, successfully compiled `omo-gateway v0.1.0`. Captured in `U75-resume-build.txt`.

## Cleanup and Limitations

- **Fixture Resources:** All temporary test directories created via `tempfile::tempdir()` were explicitly closed and deleted. All child processes spawned by tests were awaited, joined, and reaped.
- **Test Discipline:** Virtual clock / supplied timestamps were used for deterministic expiry validation; channels awaited with bounded 5s timeouts; zero `thread::sleep`, zero polling loops.
- **Scope Compliance:** Only `src/drain_control.rs` and U75-* evidence files were modified. No `.env`, real Discord, external services, or unowned files were touched.
- **Limitations:** Only unit U75 (UP.DS02 - drain marker expiry by age) is implemented and certified. Other units (e.g. U04, U62, U74, U76-U85) remain owned by their respective workers. Full 85-unit goal is not claimed complete.
