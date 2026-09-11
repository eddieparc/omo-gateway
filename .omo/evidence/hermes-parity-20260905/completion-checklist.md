# Prompt-to-artifact completion checklist

This file is an honest checkpoint, not a completion certificate. All unmet requirements remain open.

| Requirement | Exact evidence / gate | Current state |
|---|---|---|
| C1 installed Hermes and upstream pinned | reference-version.txt; six *-audit.md; delta-io/state/config.md and upstream-tree-delta.json | Lead verified 176 upstream paths; matrix includes all 14 additional findings with pinned source callsites |
| C1 all historical gaps and six inventories | matrix.md mapping .omo/hermes-parity-gaps.md and every lane row | 153 installed rows with direct citations and historical backlog mapping verified; 14 latest rows included |
| C1 PRESENT/FIX/INTENTIONAL/NOT-APPLICABLE with both sides | Per-row Rust/Hermes file:line, verified by lead; no undocumented absence | Matrix direct citations and latest delta integration verified; classifications are audit dispositions, not implementation completion |
| C2 confirmed defect increments | implementation-units.json and upstream-implementation-units.json, per-unit scenario/evidence | 85 units tracked; 16 accepted by lead: U01, U02, U03, U04, U05, U06, U07, U08, U12, U34, U50, U55, U62, U63, U74, U75 (remaining 69 units OPEN). Verified: U03 (U03-lead-green.log); U04 (U04-green.log/sanitizer/adjacent and lead independent exact+sanitizer PASS); U07 (U07-evidence.md exact replay/atomic claim with RED/GREEN, rollback/root-replacement controls, build); U08 (U08-evidence.md/lead-green/controls); U12 (U12-resume-acceptance.md clock proof, full 25 backend tests, unchanged protected hashes); U55 (U55-evidence.md/lead-cli.log isolated CLI proof); U62 (U62-resume-evidence.md, lead independent 10 tests, final private signature restored; historical retrospective constituent RED chronology caveat preserved); U75 (U75-evidence.md, lead independent 7 tests/real watcher/source-format passed). NOT accepted / in progress: U51 NOT accepted (source audit found get_running_pid unconditionally calls _record_matches_live_gateway_pid after start-time check; new live-command identity fix delivered with matching-start/wrong-command RED/GREEN, lead review and independent verification pending); U09 Flash working on shared admission; U13 retried on Flash but NOT accepted; U47 calendar partial work NOT accepted (malformed once-expression validation feedback); runtime API DAG U84->U65->U69->verifier newly active; all remaining 69 units stay OPEN |
| C2 RED before production | Per-unit failing assertion captured before patch; fails for named behavior not syntax | Captured selected RED/GREEN evidence for 16 accepted units; remaining 69 units still require pre-production RED |
| C2 GREEN same proof | Same test invocation after minimum production change | Accepted selected proofs captured in lead foundation logs and per-unit verified logs for 16 accepted units; remaining 69 units pending |
| C2 happy/boundary/adversarial/adjacent checks | Unit scenario identifies file/function plus concrete expected behavior | Accepted 16 units have per-unit local-surface and controls; remaining 69 units pending |
| C2 faithful behavior, no sleeps/weakened tests | Diff self-review, event-gated fixtures, zero ignored/skip additions | Lead reviewed accepted 16 unit diffs and causal tests; remaining implementation review pending |
| C3 real compiled binary and isolated OMO | baseline-surface.md: literal commands, no Discord tokens, temporary DB/workspace | Baseline PASS, final post-change rerun required (OPEN) |
| C3 HTTP health | curl -i http://127.0.0.1:19744/api/health -> 200 status=ok | Baseline PASS (OPEN: requires post-change rerun) |
| C3 HTTP chat ingress | POST /api/sessions/hermes-parity/chat exact HERMES-PARITY-OK prompt -> 202 | Baseline PASS (OPEN: requires post-change rerun) |
| C3 actual OMO completion | WebSocket terminal event HERMES-PARITY-OK, GET transcript and SQLite assistant row, real ocx model | Baseline PASS in 9.6s; final rerun pending (OPEN) |
| C3 changed delivery/CLI local surface | Per-unit local HTTP/WS/file/SQLite scenario, no real Discord mutation | Accepted units include local Tool/SQLite/MCP/writer-reader-process/migration-CLI/drain-watcher surfaces; other 69 units pending (OPEN) |
| C3 teardown | baseline-surface.md receipt, process exit events, no 19744/19842 listeners, temp files removed | Baseline CLEAN; future resources must be registered (OPEN) |
| C4 fmt | cargo fmt --all -- --check exit 0 captured after edits | Foundation format and diff check exit 0 in lead-foundation-format-green.txt; final post-implementation gate pending (OPEN) |
| C4 clippy | cargo clippy --all-targets --all-features -- -D warnings exit 0 | Pending (OPEN) |
| C4 build | cargo build exit 0 | Pending explicit final build (OPEN) |
| C4 full tests | cargo test exit 0 | Baseline 421 PASS; selected post-change proofs PASS; final suite pending (OPEN: prior per-unit tests/builds or baseline full pass do not close whole goal) |
| C4 diagnostics | LSP on changed files, errors absent or infrastructure precisely explained | Latest lead-foundation-diagnostics.json: 4 files no diagnostics, 16 fresh-diagnostic timeouts; final changed-file attempt and compiler/clippy still required (OPEN) |
| Baseline user edits preserved | baseline.patch vs final diff for src/agent/omo_backend.rs, src/agent/omo_config.rs, tests/test_omo_backend.rs | Snapshot captured; final audit pending |
| No .env / production / Discord writes / push | git status + executed commands + QA isolated env and cleanup | No forbidden operation so far; final audit pending |
| User invariants preserved | final-only output, typing, PNG tables, daemon/workspace isolation, dedicated sticky sessions, 300s bound | Baseline/in-scope tests must cover each after changes |
| Self-review | Full attributable diff read, criteria proof review, no unproven worker claims, HEAVY retained | Pending; no ulw-plan reviewer gate triggered |
| Final report | Outcome, every criterion evidence, external Discord QA boundary, notepad, actual commits only | Not ready |
| Goal accounting | update_goal complete only once every above is proven; include elapsed time | Active, not achieved; C3/C4 final gates and remaining 69 units OPEN |

No PR or push is requested; no PR status is a completion proxy. Source comments and intended architecture are not runtime evidence. Local actual OMO success proves the backend surface, not Discord GUI delivery.

## Execution and Delegation Directives

- **Delegation Policy**: All further source edits are delegated, preferring Gemini 3.8 Flash; the lead reads, coordinates, and verifies only.
- **Provider / Route Status**: `checka` Flash failed with HTTP 401; the local `quotio/gemini3.8` route actually executed and completed several jobs but has individual-account quota errors. No account or global configuration changes are to be made.
- **Unit Reconciliation**: 16 of 85 units accepted by lead: U01, U02, U03, U04, U05, U06, U07, U08, U12, U34, U50, U55, U62, U63, U74, U75. All remaining 69 units remain strictly OPEN. Prior per-unit tests/builds or baseline full pass do not close the overall goal. Test counts or current success must not be invented for ongoing tasks.
- **Unaccepted / In-Progress Units**:
  - **U51**: NOT accepted. Source audit confirmed `get_running_pid` unconditionally calls `_record_matches_live_gateway_pid` after start-time check; worker delivered a new live-command identity fix with matching-start/wrong-command RED/GREEN; lead review and independent verification pending.
  - **U09**: Flash is working on shared admission.
  - **U13**: Retried on Flash but NOT accepted.
  - **U47**: Calendar partial work is NOT accepted and has malformed once-expression validation feedback.
  - **Runtime API DAG**: `U84 -> U65 -> U69 -> verifier` is newly active.
- **Final Gates**: C3 final real OMO HTTP/SQLite QA and C4 final full fmt/clippy/build/tests remain strictly OPEN. Baseline user edits in `src/agent/omo_backend.rs`, `src/agent/omo_config.rs`, and `tests/test_omo_backend.rs` remain preserved against `baseline.patch`. No `.env`, production state, Discord mutations, commits, or pushes.
