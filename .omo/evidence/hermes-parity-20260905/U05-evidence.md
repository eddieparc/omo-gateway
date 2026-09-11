# U05 / AP.AP12

## Scope-cap registration (before cap production edits)

Captured cap RED before production edits: U05-cap-red-final.log / .exit, exit 101, 1 failed, 0 passed. Warn (both plain and combined): prompts=1 executions=3 persisted=1 HTTP=3. Block (both): prompts=0 executions=0 persisted=0 HTTP=3. U05-cap-red.log was a zero-test launch before test insertion succeeded; NOT behavioral evidence.

Lead approved the backward-compatible requester scope seam. Trait actually lives in src/discord/approval.rs; do not move it to tools/mod.rs.

Exact test: `security::tirith::tests::scanner_caps_precede_persistence`

Literal RED/GREEN command: `cargo test --lib security::tirith::tests::scanner_caps_precede_persistence -- --exact --nocapture`

Payloads: local HTTP scanner actions warn/block, findings [{rule_id:x,title:risk}], real echo and `sh -c 'echo u05'` (combined dangerous interpreter finding). Real DiscordApprovalRequester/SmartApprovalGuard, in-memory SQLite, local dispatcher resolves actual request IDs with Always. Three calls per case: same session twice, other session once. Expected RED: warn persists one global Always and prompts only once; block never prompts/executes. Expected GREEN: warn maxSession => two prompts, three successful echo executions, zero persisted Always; block maxOnce => three prompts, three successful executions, zero remembered/persisted grants. Existing ordinary Always grant preloaded in cache must not bypass scanner gate. Listener started before calls, shutdown/join and DB close before final assertions. Unknown/missing schema and Never retained in original exact regression.

## Status: scoped AP12 implementation verified after authorized extension

Production changes preserve Warn separately from Allow/Deny; unknown/missing actions follow fail_open. Terminal hardline and user deny precede Never/yolo bypass, which now precedes scanning. Warning reason combines with the existing command gate.

The approved extension adds ApprovalScope and a backward-compatible request_approval_with_max_scope at the existing trait definition in src/discord/approval.rs. Original request_approval and scoped MCP calls retain Always-cap behavior. Discord requester caps the resolved decision BEFORE cache/SQLite persistence. Block is now distinct from scanner failure (Deny) and can be approved Once; warning maxSession applies even with a dangerous command finding. Scanner cache keys are separate from ordinary category grants and include command/reason; Once ignores caches, Session reads only its session cache. No permanent Tirith grants are written. No tools/mod.rs edit was necessary.

Final cap proof: U05-cap-green-final.log / .exit, exit 0, 1 passed. Warn plain/combined each prompts=2 executions=3 persisted=0 HTTP=3. Block plain/combined each prompts=3 executions=3 persisted=0 HTTP=3. Strengthened GREEN control preloads an ordinary permanent category grant in memory; scanner gate still prompts. This supplements the same registered test/command; initial RED did not preload that grant.

Final original regression: U05-final-original.log / .exit, exit 0, 1 passed, prompts=1 executions=0 HTTP=2; Never executes echo, hardline/user deny reject without scanning.

U02 exact regression: `cargo test --lib tools::tests::tool_approval_scopes_do_not_cross_rules -- --exact --nocapture`; U05-mcp.log / .exit, exit 0, 1 passed, prompts=2 MCP calls=2 rule B denied=true. Ordinary terminal category grants and restored SQLite MCP grants remain functional.

Final adjacent scanner suite: U05-final-scanner.log / .exit, 6 passed. Additional schema controls cover unknown/null/missing action, invalid top-level shape, both fail_open values, warn and block/deny/reject aliases. Existing valid block expectations were corrected from Deny to Block because valid scanner findings are approvable while transport/schema failure remains unconditionally fail-closed.

Final adjacent terminal suite: U05-final-terminal.log / .exit, 20 passed. Full `cargo build`: U05-final-build.log / .exit, exit 0. git diff --check on three changed source files passed. LSP attempted on newly changed approval.rs once; same unreachable daemon limitation. No dashboard/adapter edits. All newly added fixtures are deterministic and close listeners/SQLite/tempdirs; cap fixture uses real requester, guard and SQLite, resolving exact pending IDs via local dispatcher before execution, not a mocked approval cache.

## Initial-stage captured verification (retained history)

- Exact registered command GREEN: U05-green-final.log / U05-green-final.exit, exit 0, 1 passed. Warning denial: prompts=1, executions=0; invalid: fail-closed error; Never harmless echo success; hardline/user deny errors; scanner_requests=2.
- Adjacent scanner: U05-adjacent-scanner.log / .exit, exit 0, 4 passed (allow, block/deny, failure policy, local surface).
- Adjacent terminal: U05-adjacent-terminal.log / .exit, exit 0, 20 passed, including AP04 actual helper and argv preservation.
- Library build: U05-build.log / .exit, exit 0.
- U05-green.log is an intermediate attempt launched before the successful patch application; retained rather than relabeled as passing evidence. Final authoritative output is U05-green-final.log.
- Diagnostics attempted once per owned file: both failed because LSP daemon was unreachable at /Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock. No global repair.

## Local surface and cleanup

Regression invokes public Tool::execute_with_context/execute on real TerminalTool, actual reqwest HTTP to bound loopback Axum scanner, real harmless echo subprocess. The requester supplies only a deny decision and counts actual calls. Server bound before trigger, oneshot shutdown subscribed before work; joined under bounded timeout; tempdir explicitly closed before final assertion. No live scanner/Discord, no process-global environment changes, no sleeps/polling. Monitor tool/CLI unavailable in this child; commands launched in background and process completion awaited with macOS kqueue NOTE_EXIT, not polling.

Only U05-owned source paths and U05 evidence were written. Existing AP04 argv/test changes in terminal were preserved; no backend/config/test_omo_backend edits, dependency changes, commits or pushes. Formatting applied only to tirith via rustfmt stdout -> apply_patch.

Behavioral RED captured before production edits: U05-red.log / U05-red.exit, exit 101, 1 test failed, 0 passed. Both warning and invalid returned successful echo, prompts=0, executions=2, scanner_requests=5. Additional unrelated deny-glob control using echo was ineffective because detector masks inert echo operands; corrected control to existing supported `npm publish *` rule before final GREEN (no npm execution because deny precedes process spawn).

## Registered before production edits

Exact test: `security::tirith::tests::scanner_warning_and_invalid_payload_never_silently_pass`

Literal RED and GREEN command:
`cargo test --lib security::tirith::tests::scanner_warning_and_invalid_payload_never_silently_pass -- --exact --nocapture`

Scenario: loopback HTTP scanner returns `{"action":"warn","findings":[{"rule_id":"x","title":"risk"}]}` then `{}`, fail_open=false. Actual TerminalTool executes harmless echo only if permitted. Expected RED: both parse as Allow, zero warning prompts, two executions. Expected GREEN: warn prompts and denial prevents execution; invalid schema fails closed before execution. Never skips scanner, but hardline/user deny still reject. Local listener is bound before actions and joined via graceful shutdown; no sleeps/polling.

Initial scope request (subsequently approved): DiscordApprovalRequester persisted Always before returning; maximum scope required the additional requester API in src/discord/approval.rs. Original request_approval signature remains unchanged. Requested block Once / warning Session caps are now implemented as recorded above.
