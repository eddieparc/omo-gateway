# U12 / S.F06

## Captured RED before production changes

All three literal registered commands ran one test, failed behaviorally, exit 101:
`U12-red-main.log`: left ["SECRET"], right ["OK"].
`U12-red-correlation_requires_ack_and_ignores_subscription_replay.log`: left ["SECRET"], right ["OK"].
`U12-red-correlation_distinguishes_interrupted_and_failed.log`: expect_err panicked `interrupted: ()`.
Each output records `0 passed; 1 failed`; no compilation/zero-test RED substituted.

## GREEN and local surface

The identical three commands each passed exactly one test, zero failures, exit 0:
- `U12-green-rejects_foreign_and_stale_turn_frames.log`
- `U12-green-correlation_requires_ack_and_ignores_subscription_replay.log`
- `U12-green-correlation_distinguishes_interrupted_and_failed.log`

These are real production-entry local fixtures, not a mocked reducer: TCP/WS initialize -> thread/resume r1 -> turn/start -> captured outgoing StreamChunk. Their ordered replay includes both flat turnId and nested turn.id formats. Shutdown channel is created before triggering the backend; each peer is joined and drops its listener/socket before assertions. No external daemon, Discord, production state, environment file, or persistent fixture resources used.

Adjacent command `cargo test --test test_omo_backend -- --nocapture`: 25 passed, 0 failed, exit 0 (`U12-adjacent.log`). Includes two-connection thread lifecycle, approval-denial responses and denial loop, tool summary, cron final-only/persistence/ack, cross-thread completion, empty terminal, deadline, and transport retry controls. `cargo build`: exit 0 (`U12-build.log`). `git diff --check`: exit 0, no output.

Production change is confined to omo_backend.rs: successful nonempty turn/start response fixes identity; real turn-bearing notifications must match both IDs after ACK; turn/started cannot rebind; uncorrelated idle cannot finalize; interrupted/failed are errors. Approval requests deliberately remain outside correlation because the installed peer uses placeholder approval turn IDs. No public API or omo_protocol.rs changes.

Existing test reconciliation: the old missing-terminal fallback explicitly expected thread idle to finalize, contradicting S.F06. Renamed it to `test_omo_backend_requires_turn_terminal_even_after_final_agent_message` and asserted error/no final. The tool-after-message fixture now supplies real thread/turn IDs and a correlated terminal rather than unidentifiable items plus idle; removed its fixed delay (the focused foundation review correctly identified this as lost temporal coverage; repaired below). Replaced the shared fake server's timing-luck 10ms approval pause with a bounded wait for response id 999. Other baseline behavior was retained; these are explicit requirement-driven test adjustments, not skips.

Diagnostics attempted once on each changed Rust file: both failed because `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock` was unreachable. No repair attempted. Cargo compilation/test/build succeeded. No monitor interface was available; commands were captured through bash rather than the requested asynchronous monitor. This is a verification-infrastructure deviation, not a claim that monitor was used.

Cleanup/scope: all writes by this unit are src/agent/omo_backend.rs, tests/test_omo_backend.rs, and U12-* evidence files. Concurrent workers' other dirty files were observed but not edited. U12-baseline.patch records the preexisting owned-file diff; original baseline.patch was never modified. No formatter, commits, dependency changes, global config, or real service operations. Full project tests were not run; this evidence establishes only S.F06's bounded surface, not overall parity completion.

## Registration BEFORE production edits

Exact integration test IDs (root module of test_omo_backend):
- `rejects_foreign_and_stale_turn_frames`
- `correlation_requires_ack_and_ignores_subscription_replay`
- `correlation_distinguishes_interrupted_and_failed`

Literal RED and identical GREEN commands:
```
cargo test --test test_omo_backend rejects_foreign_and_stale_turn_frames -- --exact --nocapture
cargo test --test test_omo_backend correlation_requires_ack_and_ignores_subscription_replay -- --exact --nocapture
cargo test --test test_omo_backend correlation_distinguishes_interrupted_and_failed -- --exact --nocapture
```

Payload 1: ACK r1/t2; delta r9/t9 SECRET; completed r1/t1; delta r1/t2 OK; completed r1/t2. Expected binary RED: final SECRET instead of OK. GREEN: sole final OK, no SECRET in any chunk.

Payload 2: resumed r1 subscription, pre-ACK t2 SECRET/completed; ACK t2; current O; stale idle; started t1; stale delta, foreign tool, stale item, foreign turn/error, stale nested-turn error, missing IDs; current K/completed. RED: pre-ACK final SECRET. GREEN: sole final OK, no contamination or premature abort.

Payload 3: ACK t2; partial; completed t2 interrupted, then failed (separate runs). RED: interrupted returns Ok; GREEN: error distinguishes interrupted/failed, no final.

All use public AgentBackend::run through loopback WebSocket thread/resume and captured production OutboundDispatcher. Peer buffers ordered frames; oneshot shutdown registered before run; bounded completion and joined peer, no sleeps/polling. Existing baseline captured in U12-baseline.patch; protected baseline.patch untouched. No monitor tool or executable is exposed in this child; shell command capture is used instead (infrastructure limitation).


## Quiet-tool coverage repair (st_01a0711e)

Test-only repair in `tests/test_omo_backend.rs`; no production changes in this task.
The existing delayed-tool test retains every original final-content assertion and
all three U12 correlation regressions plus user deadline/grace tests remain.

Causal sequence: register a recording-dispatcher oneshot before starting the real
public backend; real loopback peer sends correlated ACK, initial intent and
tool-start, then an ordered `U12_TOOL_STARTED` sentinel delta. The backend does
not dispatch tool-start itself, so this subsequent dispatcher event is the
available public causal witness that the same sequential receive loop processed
the preceding tool-start. It is not a server-send notification or a mocked
backend. The dispatcher records all actions without suppressing finals or
blocking backend progress. Peer holds completion on a separate release oneshot.
After receiving the witness, pause Tokio time (not before the socket handshake),
advance 1200ms (> original 1100ms), directly poll the actual pinned backend future
(not a JoinHandle), assert Pending and zero final chunks. Resume time, release
correlated tool completion/digest/terminal, and assert original actual-digest and
not-initial-intent assertions plus exactly one final. JoinSet peer is explicitly
stopped and joined; the empty set is asserted. No new wall-clock sleep, polling
loop, or yield loop. A single future poll is an assertion, not a completion wait.

Clock limitation: Tokio timers are advanced; production `std::time::Instant`
elapsed checks are not virtualized. This repairs timer-based quiet-finalization
protection under the requested no-production-change constraint; it does not
claim virtual coverage of the separate wall-clock deadline/grace checks. Those
existing user tests are untouched and passed in the target. No premature-final
mutation was run; no RED sensitivity claim is made for an unexecuted mutant.

Verification:
- Registered root-module fully-qualified exact test command in U12-commands.json
  before execution. U12-quiet-selected.log/.exit: 1 passed, 0 failed, exit 0;
  24 filtered, actual test duration 0.02s. Both causal checkpoints printed.
- U12-quiet-suite.log/.exit: full real backend target 25 passed, 0 failed,
  0 ignored, 0 filtered, exit 0; test duration 30.98s. Includes all three U12
  regressions and original deadline/grace controls.
- Important deviation: an initial full-target command exceeded its 600-second
  subprocess wrapper bound. The wrapper failed to retain buffered output, so
  no test count or child outcome is claimed. U12-quiet-suite-initial.log/.exit
  records this honestly. A subsequent process listing showed no repository
  cargo/backend test process. Capture was changed to direct-to-file before the
  successful bounded invocation. This is NOT a claim of single-attempt suite
  success, nor evidence that the initial stall was caused by any specific test.
- Language-server diagnostics returned `No diagnostics found` twice, including
  after final edited-line formatting. U12-quiet-diagnostics.json records this.
- rustfmt --check --edition 2021 tests/test_omo_backend.rs and scoped git diff
  --check both exit 0 (U12-quiet-format-final and U12-quiet-diff-check logs/exits).
  Initial check identified only new-line formatting; apply_patch adjusted those
  lines alone. No broad writing formatter used. Initial apply_patch rejected an
  ambiguous hunk without changes; narrower hunks applied successfully.
- This is a test-only single-domain change: the exact test and full integration
  target execute the real affected entry point. No additional production build
  or live daemon/Discord run was needed or performed during this repair.

Cleanup: repaired fixture uses an ephemeral loopback port, no file/process/HOME
fixtures, held channels, and a joined peer. Existing suite fixtures clean up by
runtime/tempdir scope. U12-quiet-cleanup.txt records no remaining backend test
process and unchanged SHA256 for backend/config/original baseline.patch.
U12-quiet-before.patch records the already-dirty test diff at entry;
U12-quiet-current.patch records its final diff. Other workers' preexisting dirty
paths were neither reset nor edited. All writes by this repair are this test and
U12-* evidence. No .env/global configuration/production file, commit or push.

## Clock correction registration (before production clock edit)
Lead rejected the earlier quiet GREEN as timing proof: std Instant did not age. Same exact command now runs the preserved success case plus a one-second total-deadline positive witness after the identical 1200ms held interval and configured one-second grace. Expected RED: std-clock backend returns Ok rather than deadline error. Command: `cargo test --test test_omo_backend test_omo_backend_waits_for_final_message_after_tool_activity -- --exact --nocapture`.

## Resumed clock correction (st_01a07201, 2026-09-05)

Resumed the dirty handoff from session 01a070bb-40f8-7c3e-a02e-e802d250b826;
did not recreate the earlier correlation implementation or its RED tests.
The held-tool fixture and its new positive deadline witness were already present.
The production streaming clocks were still std Instant at all three construction
sites. The prior `U12-quiet-clock-red.log` contains only package/build lock waits:
it has no test result and is not counted as a behavioral RED.

Actual chronology in this child:

1. Before any further production edit, ran the exact registered command above.
   It waited for the build lock, compiled (cargo profile time 3m 21s), then ran
   exactly one test. The held success case passed both checkpoints; the deadline
   case returned `Ok(())` and panicked at `backend clock must age past the
   one-second deadline: ()`. Result: **0 passed, 1 failed, 24 filtered, exit 101**,
   test duration 0.01s (`U12-resume-quiet-clock-red.log/.exit`). This is a new
   pre-clock-edit behavioral observation, not a relabeling of the stalled log.
2. Replaced only the three streaming `std::time::Instant::now()` expressions
   with `tokio::time::Instant::now()`: turn start, initial last activity, and text
   activity refresh. The already-Tokio connection clock was unchanged. Real
   production elapsed checks for ACK/deadline/grace/silence now use the same
   clock domain as Tokio timers, not a test-only elapsed assertion.
3. Ran the identical registered command: **1 passed, 0 failed, 24 filtered,
   exit 0**, test duration 0.11s, cargo profile time 27.92s. Both held checkpoints,
   the actual-digest final, and the deadline-error checkpoint printed
   (`U12-resume-quiet-clock-green.log/.exit`). No assertion was weakened to obtain
   GREEN; the test body was unchanged between RED and GREEN.
4. Ran `cargo test --test test_omo_backend` **once** in this child:
   **25 passed, 0 failed, 0 ignored, 0 filtered, cargo exit 0**, test duration
   30.65s. This includes all three correlation regressions and existing user
   ping/deadline/grace/terminal-edge/retry/cron ACK controls. Cargo reported
   5m 38s including build-lock contention; enclosing subprocess elapsed was
   442.59s. The subsequent artifact writer failed with exit 1 because it passed
   a patch as argv rather than stdin. Its exception retained the complete cargo
   output and exit, which were transcribed with apply_patch into
   `U12-resume-quiet-suite.log/.exit/.command`. This is cargo success plus a
   capture failure, not a successful outer wrapper or a second suite attempt.
5. `cargo build`: **exit 0**, dev profile time 53.02s, enclosing elapsed 53.30s
   (`U12-resume-build.log/.exit/.command`). No daemon/service binary was started.

The real surface is public `AgentBackend::run` over ephemeral loopback WS with
the actual production dispatcher call. Recording subscription, held release,
and shutdown channels exist before backend execution. The ordered
`U12_TOOL_STARTED` delta witnesses receipt-loop processing after tool-start;
the dispatcher records every action and does not suppress finals. Pause occurs
only after this causal event. Advancing 1200ms and directly polling the pinned
backend yields Pending and no final while the peer withholds completion. In
the success case (10s total, 3s request, 1s grace), release yields exactly one
actual-digest final and not the initial intent. In the positive witness (1s
total), the first post-release nonterminal frame exercises the production
deadline check, producing a deadline error and zero finals. Both cases join
the peer before final outcome assertions and assert an empty JoinSet.

Important bound: existing deadline checking is frame-driven. The witness proves
the production elapsed clock aged across the held interval when the next frame
arrives; it does not claim a frame-free hard-deadline wakeup. The same corrected
started_at clock also feeds empty-content grace, but this nonempty fixture is
not an isolated empty-grace witness. Existing grace controls passed in the full
target. No sleeps, polling loops, or yield loops were added. The baseline 100ms
production interrupt drain wait and 500ms retry cooldown were not changed.

Diagnostics returned `No diagnostics found` for both Rust files before RED and
again after the clock edit before GREEN/build (`U12-resume-diagnostics.json`).
After build, rustfmt found only two formatting regions in the interrupted
author's positive witness. Applied whitespace-only wrapping there, preserving
every value/assertion. Final rustfmt on both owned files and scoped diff check
each exited 0 (`U12-resume-final-check.log`). Final post-format diagnostic
attempts timed out awaiting fresh results within 3000ms for both files; those
attempts are NOT clean diagnostics (`U12-resume-final-diagnostics.json`). The
GREEN/suite/build evidence predates this whitespace-only wrapping; no duplicate
behavioral run was manufactured. An earlier 10s auxiliary format/tool-lookup
wrapper timed out without output and is not counted as a validation pass.

Scope and preservation: final backend SHA256 is
`e195fc3c5d877277c7d31d0228c715b5f1f63135d28b50a80d3d251f2cffb36d`.
Reversing just the three new clock expressions **in memory** reproduces the
prior protected backend SHA256
`d0b05cb8913a05f0b40d47fc62c66ee4674f4aca8c11c4760d0b6b2ba0e3b4a0`.
Config SHA256 remains `8f5b821841aaaee33f2d6d9390c591a5cd2ecee51293eb0a2baa98a967190309`;
original baseline.patch remains
`dd23d51f825104cd4b4192256a8524370de9dfe171aceafa24d93d7eba3f7964`.
Thus existing caps/settings/correlation semantics were retained. Final process
snapshot found zero backend-target cargo/test processes. All authored writes
are the two scoped Rust files and U12-* evidence; other workers' dirty paths,
including newly appearing cron/dashboard paths, were neither edited nor reset.
This is not a claim that the shared worktree contains only U12 changes.

Workflow deviations are explicit in `U12-resume-capture-deviations.log`: initial
production edit used functions.edit and RED artifacts used functions.write,
not the requested apply_patch-only path. Remaining writes used apply_patch.
No eval/tool_schema/monitor API is exposed in this child's callable tool set;
shell lookups also found neither tool_schema nor tool.monitor, so bash was used
instead. baseline.patch was read after the first clock edit/GREEN, later than
requested, then checked against the unchanged protected hashes. Earlier logs
were not overwritten. No .env/global configuration, real Discord/production
service, dependency edit, commit or push operation was performed.
