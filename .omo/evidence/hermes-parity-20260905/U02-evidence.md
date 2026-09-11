# U02 - AP.AP06

## Registered BEFORE production edits

- Exact test: `tools::tests::tool_approval_scopes_do_not_cross_rules`
- Literal RED and GREEN command: `cargo test --lib tools::tests::tool_approval_scopes_do_not_cross_rules -- --exact --nocapture`
- Payload A: `{"tool":"client_a","arguments":{"sentinel":"A"}}`; remote method `rule_a`. Payload B: `{"tool":"client_b","arguments":{"sentinel":"B"}}`; remote method `rule_b`.
- Scenario: real ToolRegistry -> McpTool -> DiscordApprovalRequester -> SmartApprovalGuard with real SQLite. Local dispatcher resolves first prompt Always and subsequent prompts Deny through the real custom-id API. Recreate guard/requester from SQLite; repeat A in another session, then B. Local loopback HTTP MCP fixture records actual tools/call payloads.
- Expected binary RED: B silently autoapproved, prompts=1 and MCP calls=3; assertion expects prompts=2. Expected GREEN: B prompts and is denied, prompts=2 and MCP calls=2. Same-rule Always survives reload across sessions; terminal category grants remain broad.
- Cleanup precedes the RED assertion: graceful HTTP shutdown and join, SQLite pool close, pending guards checked empty. No sleeps/polling or real Discord.
- RED output and exit code will be captured before any production patch.

## Behavioral RED captured before production patch

`U02-red.txt`: exit 101; 1 selected test, 0 passed, 1 failed. Literal observable: `AP06 prompts=1, MCP calls=3, rule B denied=false`. Assertion: `rule B autoapproved using rule A's Always grant`, left 1 / right 2. This is behavioral RED, not a compile failure or zero-test run.

## Patch and GREEN

- `src/tools/mod.rs:149-165`: SHA-256 over length-prefixed tool name plus reason, with `tool-rule:v1:` namespace. Display target remains separate. No argument-based over-scoping or changes to terminal category policy.
- `src/discord/approval.rs:129`: additive `request_approval_scoped` trait method defaults to the existing request method for compatible non-caching implementations. The existing public signature remains unchanged; all current callers compile.
- `src/discord/approval.rs:200-211`: terminal entry derives the existing category key; Discord's scoped implementation uses the supplied identity for lookup and remembered grants, sharing the original lifecycle body. AP08 owner should make lifecycle changes in this scoped implementation; no AP08 behavior changed here.
- `src/tools/mod.rs:218`: registered regression is also the real local production-entry proof: registry, MCP routing/HTTP transport, dispatcher, custom-id resolver, guard and migrated SQLite are real. Only human decisions and remote MCP responses are fixtures. The prompt receiver is installed by the real guard before synchronous dispatch/resolution; no timing-based waits.
- Identical registered GREEN command, `U02-green.txt`: exit 0, 1 passed, 0 failed; literal observable `AP06 prompts=2, MCP calls=2, rule B denied=true`.
- Happy control: A's raw arguments reach HTTP unchanged, repeated A in another session is approved after SQLite reload without another prompt. Adversarial control: distinct B reason prompts and denial prevents its HTTP invocation. Adjacent control: a remembered terminal recursive-delete category accepts another target without executing any shell command.
- `U02-checks.txt`: existing Once-approved/denied registry tests: 2 passed; exact permanent allowlist persistence test: 1 passed; `cargo build`: finished successfully. Commands registered in `U02-commands.json`.

## Execution, diagnostics and cleanup

Cargo jobs were launched asynchronously, output redirected to the named logs, and monitored by Darwin kqueue NOTE_EXIT (bounded 600s); no sleep/poll loops. RED and GREEN logs include shell exit codes. Test fixture gracefully shuts down and joins its loopback HTTP server, closes its in-memory SQLite pool, and checks both guards have zero pending approvals before the final binary assertions, including RED. No real Discord, production state, environment files, dependency files, or global configuration were changed.

LSP diagnostics attempted once on each touched Rust file; both failed because the daemon was unreachable at `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`. No infrastructure repair attempted. Cargo test/build supply compiler verification; LSP verification remains unavailable.

Only this unit's two owned Rust files and U02 evidence artifacts were written by this worker. Other concurrent workers' changes visible in git status were left untouched. The protected user files and baseline.patch were not written; baseline.patch final SHA-256 is `dd23d51f825104cd4b4192256a8524370de9dfe171aceafa24d93d7eba3f7964`. No commit, push, or repository-wide formatter ran.

Applicability remains the audited public Rust registry/requester surface, not proof that daemon-owned live agent tools use this registry. AP08/AP10 and the separate pre-existing heartbeat timing test defect AP23 are outside U02; no claim those findings are repaired.

## Child st_01a0711c revalidation

This child found the production patch, regression, pre-edit registration and captured RED/GREEN already present. It preserved those artifacts and made no additional Rust edits; the original RED chronology above is inherited evidence, not a newly performed rollback. The current shared requester also contains U05 scanner scope changes, which were preserved.

The identical registered exact command was executed against the current workspace: `U02-revalidated-final-1.txt`, exit 0, one test passed, observable `AP06 prompts=2, MCP calls=2, rule B denied=true`. This exercises the real registry/requester/guard/SQLite/loopback MCP surface and its cleanup again. Adjacent Once-approved/denied tests both passed (`U02-revalidated-final-2.txt`); exact permanent-allowlist persistence test passed (`U02-revalidated-final-3.txt`); `cargo build` exited 0 (`U02-revalidated-final-4.txt`). All four literal rerun commands remain in `U02-commands.json`.

Both Rust LSP diagnostic calls succeeded in this child with `No diagnostics found`; this supersedes the earlier infrastructure limitation for this revalidation. The AP08 handoff seam is now `DiscordApprovalRequester::request_with_scope` at approval.rs:269, reached by `request_approval_scoped` at :236; lifecycle changes belong there, not in the registry hash.

Runner disclosure: the first Python monitor invocation failed because Darwin's `select.kqueue` does not implement the context-manager protocol. Its already-launched exact test completed successfully (`U02-revalidated-1.txt`), but no shell exit code was captured for that invocation. No cargo child remained in the subsequent process listing. The corrected monitor explicitly closed kqueue in `finally`, awaited NOTE_EXIT with a 600s bound, and captured all four final exit codes. No test failed or was weakened; no sleeps or polling loops were used. This child wrote only U02 evidence files and did not touch the protected user files or baseline.patch.
