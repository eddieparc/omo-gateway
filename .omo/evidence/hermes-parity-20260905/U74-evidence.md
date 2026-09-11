# U74 - slow restart chain

Finding UP.DS01. Scope src/multiplexer/restart_loop_guard.rs (production/tests), src/main.rs (test only). No other writers own these files in current foundation phase.

## Binding scenario before production edits
Persisted boot clocks 0,150,300 with recreated default guard must return false,false,true; >300 gap resets while equality300 links; fast-cycle, disabled threshold, corrupt file and explicit clear retain contracts. History retained at most max(50,max_restarts). Actual startup guard seam must suppress existing recovery, leave the real SQLite pending marker, then permit explicitly routed user event through SessionMultiplexer. No real Discord/OMO. Event channel subscribed before route; bounded receive, no sleeps. TempDir owns files and pool closes after stop.

Literal RED/GREEN commands (must execute nonzero tests):
- cargo test --lib multiplexer::restart_loop_guard::tests::restart_chain_trips_for_slow_cycles -- --exact --nocapture
- cargo test --bin omo-gateway legacy::runner_tests::restart_chain_preserves_pending_until_explicit_input -- --exact --nocapture

RED expected first third-boot verdict false rather than true; second auto-recovered count1 instead of0. Existing window-only test will change to new stated chain requirement, never skipped/weakened. Upstream gateway/restart_loop_guard.py:26-34,56-88,98-114. Rust production entry src/main.rs:1303-1319 uses RestartLoopGuard::new/check_and_record; no per-message gate.

## Initial RED
Library command executed one test and failed exactly: left [false,false,false], right [false,false,true], exit101. Captured U74-red-first.txt. Binary command failed compilation from missing test-local RestartLoopGuard import; NOT accepted as RED. Added test-local import only, production unchanged, then reran binary with correct legacy::tests module prefix discovered from entry.rs.

Correction: entry includes main.rs under legacy but test module is runner_tests (main.rs:1488), not tests. Previous binary rerun selected zero tests and is rejected. Registration corrected to legacy::runner_tests before actual binary RED. No production change yet.

## Actual binary RED captured
Exact registered legacy::runner_tests command now ran1 test, failed recovered count assertion left1/right0, exit101; U74-red-binary.txt contains literal output. Both independent RED proofs are now behavioral. Next production patch changes recent-window pruning to bounded consecutive-gap chain while preserving public constructor.

## GREEN and local surface
Exact two registered tests passed1 each, same commands and payloads as accepted RED; adjacent guard module passed5/5, aggregate monitor exit0. Literal output: U74-green.txt; reproducible commands U74-commands.json. The real binary test exercised production guard, real temporary SQLite pending marker and actual SessionMultiplexer event dispatch; auto replay0, pending count1, explicit event received. No sleeps/network/Discord. Runner channel registered before dispatch; timeout5s is only failure bound. stopped actor, explicitly closed pool, TempDir dropped all runtime state; tests exited, no listener was opened.

Self-review read actual own diff: one production guard file, main.rs test-only. Existing window expiry case now proves301s quiet gap under latest contract; no skipped/weakened failures. Public constructor preserved, sorted consecutive-gap scan, max50 or configured threshold retained history; disabled0, corruption/clear/fast loop controls pass. LSP attempts on both changed files failed unreachable /Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock; not reported clean. Compiler tests pass; final fmt/clippy/build/full test still globally required.
