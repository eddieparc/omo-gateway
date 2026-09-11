# Foundation resume verification (U12 repair + U62 supplement)

Date: 2026-09-06. Independent verifier: hephaestus, task st_01a0773f (resume of
interrupted session 01a070bb-40f8-7c3e-a02e-e802d250b826).

This is a **new, additive** report. It does not overwrite and does not restate
`foundation-verification.md` (2026-09-05), which retains all 46 original command
outputs and the two historical qualified FAILs (U12, U62). The seven scoped
foundation PASS units (U01/U02/U05/U06/U34/U50/U63) are cached from that report
and were **not** rerun here. This run re-verifies only the two units whose
2026-09-05 disposition was FAIL: the repaired U12 quiet-tool coverage, and the
U62 supplemental counterfactual evidence with its chronology disposition.

**Scope discipline.** Writes by this task are `foundation-resume-verification.md`
and `U12-resume-verifier-*` logs only. No production or test source was edited.
Full-repository format and full-workspace test gates are **deferred**, not run,
because admission-storage writers are active (see admission-storage-plan.md);
nothing deferred is reported green below. Lead owns full-goal final closure.

---

## Current per-unit / phase disposition

| Unit / phase | 2026-09-05 | Current disposition | Basis |
| --- | --- | --- | --- |
| U12 quiet-tool coverage | **FAIL** (weakened adjacent delayed-tool temporal coverage) | **PASS (repaired)** | New pre-clock-edit behavioral RED + identical GREEN; verifier-independent exact rerun exit 0; scoped build/fmt/diff exit 0; source SHA unchanged |
| U62 exact-payload chronology | **FAIL** (missing constituent prepatch RED) | **Code behavior verified; chronology deviation stands (not closed by this record)** | Retrospective counterfactual RED/GREEN proves regression sensitivity + current behavior only; cannot become historical prepatch RED |
| U01/U02/U05/U06/U34/U50/U63 | PASS | **Cached — not rerun** | Bounded to each unit's 2026-09-05 contract in `foundation-verification.md` |
| Admission-storage phase (U03/U07/U51/U55/U75/U04/U08/U09/U10/U11) | n/a | **Authorized concurrent scope — NOT drift** | admission-storage-plan.md authorized file set; dirty paths and `Cargo.toml` chrono-tz addition are theirs |
| Whole-repo format / full-workspace suite | n/a | **DEFERRED — not green** | Deferred while admission writers active |

---

## U12 - repaired quiet-tool coverage: PASS

### What the original FAIL was
The 2026-09-05 review accepted U12's three correlation regressions but flagged
lost temporal coverage: the delayed-tool adjacent fixture had its 1100ms tool
gap removed and its timing-luck approval pause replaced, so it no longer proved
that the backend refuses to finalize a turn while a tool is outstanding and the
elapsed clock advances. An earlier std-clock quiet "GREEN" was **rejected** by
the lead because `std::time::Instant` did not age under Tokio's paused clock, so
that proof could not distinguish a real deadline from a no-op.

### The repair actually present in current source
Confirmed by reading `src/agent/omo_backend.rs` at the streaming loop: the three
streaming clock constructors are now `tokio::time::Instant::now()` — turn start
(`started_at`, L389), initial `last_activity_at` (L392), and the text-activity
refresh (L427). The already-Tokio connection clock (L73/L75) is unchanged. The
in-memory reversal of exactly these three expressions reproduces the protected
prepatch backend SHA256 `d0b05cb8...`, confirming this is the only new production
edit. Current on-disk hashes match every worker/lead-reported value:

- `src/agent/omo_backend.rs` = `e195fc3c5d877277c7d31d0228c715b5f1f63135d28b50a80d3d251f2cffb36d`
- `tests/test_omo_backend.rs` = `4381199989888595f675c0910d1d026adff8c621403afb4c0210bfd589d0e440`
- `src/agent/omo_config.rs` = `8f5b8218...` (protected caps/config unchanged)
- `baseline.patch` = `dd23d51f...` (protected baseline untouched)

### Event-before-time ordering (the mechanism this proves)
The backend's deadline enforcement is **frame-driven**, not a wall-clock wakeup.
The elapsed checks — turn/start ACK timeout, `last_activity_at.elapsed() >
silence_limit`, and `started_at.elapsed() > effective_total_timeout` for any
non-`turn/completed` method — are evaluated only when a frame arrives on the
socket. Because `started_at`/`last_activity_at` are now Tokio instants, elapsed
time reflects paused/advanced virtual time.

The repaired fixture exploits exactly this ordering:
1. It subscribes a recording dispatcher and registers release/shutdown channels
   **before** starting the real public `AgentBackend::run` over ephemeral
   loopback WS. No fixed sleep or polling is used.
2. The loopback peer sends correlated ACK, intent, and tool-start, then an
   ordered `U12_TOOL_STARTED` sentinel delta. The backend does not itself emit
   tool-start, so the dispatcher recording that sentinel is the public causal
   witness that the same sequential receive loop already processed tool-start —
   an event-before, not a timer race.
3. Only **after** that witness does the test pause Tokio time, advance 1200ms,
   and directly poll the pinned backend future. It asserts `Pending` and zero
   final chunks while the peer withholds completion.

**Early finalization would be observable.** If the backend incorrectly finalized
an idle turn with an outstanding tool (the S.F06 defect), the direct poll after
+1200ms would return `Ready` and/or emit a final chunk — the fixture's `Pending`
+ `finals=0` assertions would fail. The positive clock witness (1s total, 1s
grace) then releases a frame after the advance: the frame-driven
`started_at.elapsed() > effective_total_timeout` branch fires, producing a
deadline error with `finals=0`. Under the rejected std clock this branch never
tripped (`Ok(())` returned), which is precisely what the new prepatch RED shows.

### RED chronology (genuine, pre-clock-edit)
`U12-resume-quiet-clock-red.log/.exit`: with std clocks still in place, the exact
registered command ran one test — the held success checkpoints printed, then the
one-second deadline case returned `Ok(())` and panicked at `backend clock must
age past the one-second deadline` → **0 passed, 1 failed, 24 filtered, exit 101**,
duration 0.01s. This is a real behavioral RED observed before the clock edit, not
a relabeling of the earlier stalled `U12-quiet-clock-red.log` (which contained
only build-lock waits and no test result, and is not counted).

### GREEN + verifier-independent rerun
- Worker GREEN after the three-expression edit (`U12-resume-quiet-clock-green.log`):
  **1 passed, 0 failed, 24 filtered, exit 0**; both held checkpoints, the
  actual-digest final, and the clock-witness deadline error all printed. The test
  body was unchanged between RED and GREEN — no assertion was weakened.
- **This verifier's independent rerun** of the exact scenario
  (`U12-resume-verifier-exact.log`):
  `cargo test --test test_omo_backend test_omo_backend_waits_for_final_message_after_tool_activity -- --exact --nocapture`
  → **1 passed, 0 failed, 24 filtered, exit 0**, duration 0.11s. All four
  checkpoint lines printed (held x2, digest final, clock-witness deadline error,
  finals=0). The freshly built test binary hash differs (`a074f7c420765738`)
  because a concurrent worker's `Cargo.toml` dependency addition forced a full
  recompile; the U12 sources are byte-identical to the reported SHAs.
- **cargo build once** (`U12-resume-verifier-build-format.log`): exit 0.
- **Scoped rustfmt** `--check --edition 2021` on `src/agent/omo_backend.rs` and
  `tests/test_omo_backend.rs`: exit 0; scoped `git diff --check` exit 0.

### Adjacent target
No new failed adjacent scenario was found; the changed-target full backend suite
is the producer's/lead's to rerun (per task), and is already recorded green at
25/25 (`U12-resume-quiet-suite.log` exit 0; lead `U12-lead-verification.log` /
`U12-resume-acceptance-suite.log` 25 passed exit 0). This verifier did not
re-launch the full suite; only the repaired exact scenario + build + scoped fmt
were in scope here.

**U12 verdict: PASS (repaired).** The lost quiet-tool coverage is reconstructed
with a clock-sensitive, event-before-time fixture whose assertions would observe
early finalization; the pre-clock-edit RED and identical GREEN are genuine, and
an independent exact rerun plus scoped build/fmt gates are exit 0.

---

## U62 - supplemental counterfactual evidence: code verified, chronology deviation stands

### What the supplement is and what it proves
`U62-counterfactual-evidence.md` embeds the original supervisor source blob
(`949084fa...`) and the fixed source as sibling modules and runs the identical
new test `supervisor::counterfactual::real_exiting_child_budget` against each
(real temporary Python daemon: HTTP 200 once, ready event, exit on release byte;
parent awaits three real child exits, advances the restart-budget clock 4s,
inspects terminal circuit state; HOME isolated; all waits bounded; child reaped
before assertion even on RED).

- `U62-counterfactual-red.txt`: baseline blob → after three real child exits,
  `circuit_stopped=false`, `live_pid=Some(69819)`; panics "restart budget
  admitted work after three real child exits" → **0 passed, 1 failed, exit 101**.
- `U62-counterfactual-green.txt`: fixed source → `circuit_stopped=true`,
  `live_pid=None` → **1 passed, 0 failed, exit 0**.

This demonstrates **regression sensitivity of the real exiting-child constituent
and correctness of current behavior**. It raises behavioral confidence for the
constituent that the original 2026-09-05 record covered GREEN-only.

### Chronology limitation (stated explicitly, not fabricated closure)
This retrospective harness compares the original vs. fixed **source** *now*; it
was authored after the production fix. It is **NOT** a pre-production RED and
cannot become one. The historical facts are unchanged and remain:

- The original author captured genuine prepatch RED for the **spawn-failure**
  path (`daemon_restart_budget_stops_spawn_failures`) and for the **live-unready
  replacement child** path — both are real historical prepatch REDs and stand.
- The original author captured only **postpatch GREEN** for the **real
  successfully-spawned-then-exiting child** constituent. No historical prepatch
  RED exists for that specific constituent.

Therefore the distinction the task requires is preserved:
- **Code behavior:** verified — current source stops the budget after three real
  exiting children (counterfactual GREEN), and the defect is observable
  (counterfactual RED against the original blob).
- **Evidence policy:** the constituent-specific *sequencing* deviation — that its
  prepatch RED was never captured in real time — **is not erased**. A
  retrospective source comparison improves confidence but cannot retroactively
  establish historical prepatch RED. U62 is therefore **not closed** on this
  record; its 2026-09-05 FAIL for missing constituent chronology stands, now
  qualified by verified current behavior. No closure is fabricated.

---

## Deferred / not-green and honest deviations

- **Whole-repository rustfmt / full-workspace test suite: DEFERRED, not run,
  not green.** Admission-storage writers are active; only U12-owned files were
  formatted/diff-checked. These gates belong to lead's full-goal final closure.
- **LSP diagnostics:** the earlier post-format attempts timed out awaiting fresh
  results (`U12-resume-final-diagnostics.json`, `clean_result_claimed: false`);
  those are NOT clean diagnostics. Pre-edit and post-clock-edit diagnostics were
  clean (`U12-resume-diagnostics.json`). This verifier did not re-query LSP.
- **Monitor / eval / tool_schema:** not exposed in this child's tool set (prior
  child confirmed `command -v` failures). Long commands ran through the exposed
  bash tool with bounded output; no monitor-use is claimed. Logs were written via
  the file-write (apply_patch-equivalent) tool, not shell redirection.
- **Incidental lock refresh (NOT foundation drift):** a concurrent cron-parity
  worker added `chrono-tz = "0.10"` to `Cargo.toml` (for the untracked
  `tests/test_cron_schedule_parity.rs`). This verifier's authorized `cargo
  test`/`build` resolved it into `Cargo.lock` (+chrono-tz, +phf, +phf_shared).
  This is admission-storage authorized scope; it was neither authored as a
  foundation edit nor reverted (reverting would break the concurrent worker).
- **Admission-storage authorized scopes are not drift.** The dirty paths for
  U03/U07/U51/U55/U75/U04/U08/U09/U10/U11 (approval/adapter/dashboard, skills/db/
  memory, migrate, drain_control, etc.) are authorized by admission-storage-plan.md
  and were neither edited nor labeled worker drift here.

## Stop condition

Repaired U12 has an actual justified verdict — **PASS (repaired)** — backed by a
genuine pre-clock-edit RED, identical GREEN, a verifier-independent exact rerun
(exit 0), and scoped build/fmt/diff gates (exit 0), with early finalization shown
observable via the frame-driven event-before-time ordering. U62 has a truthful
disposition — **current code behavior verified; historical constituent chronology
deviation stands, not closed**. No deferred check is reported as green. Full-goal
final closure remains with the lead.
