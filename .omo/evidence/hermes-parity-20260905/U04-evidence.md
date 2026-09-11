# U04 display-only credential redaction

## Registration and Handoff Preservation

Registered before production changes: `cargo test --lib discord::approval::tests::approval_display_redacts_credentials_without_changing_execution -- --exact --nocapture`. Exercise real ToolRegistry -> DiscordApprovalRequester -> recording dispatcher -> explicit Once -> recording Tool execution. Payload carries Authorization: Bearer sentinel-secret-123 in command and reason. Expected RED: secret present in serialized ApprovalRequest/embed. Expected GREEN: absent on display surfaces, identical raw args delivered to tool only after approval. No HTTP/curl is executed; the narrow tool double records execution data, while the actual approval/dispatch boundary remains production.

Lead captured preproduction exact RED (monitor `mon_715V5E3NN8VJYFCC`):
- Command: `cargo test --lib discord::approval::tests::approval_display_redacts_credentials_without_changing_execution -- --exact --nocapture`
- Outcome: 1 test, 0 passed, 1 failed, exit 101.
- Output: `U04 secret_exposed=true raw_args_equal=true executions=1 pending=0`
- Analysis: Sentinel credential was exposed in serialized approval display and embed structures before display-only redaction wiring.

Write scope: `src/discord/approval.rs` plus new `src/security/approval_display.rs` and `src/security/mod.rs` export. Preserve raw command/policy keys and U02/U05 scopes, U03 cancellation lifecycle. Real Discord, .env, global accounts/config, commits and push prohibited.

## Review Issue Resolution: URL Userinfo Redaction

Review finding: URL regex matched normal `host:port` patterns without requiring the `@` userinfo delimiter, improperly treating benign host:port targets (such as `localhost:8080`) as credentials and stripping them.

Resolution in `src/security/approval_display.rs`:
- Updated credential redaction pattern for URLs to require the `@` delimiter: `(?i)([a-z][a-z0-9+.-]*://)[^/\s@]+@`.
- Replacement string `${1}[REDACTED]@` preserves the `@` delimiter, host, port, and path while redacting userinfo credentials preceding `@`.
- Added benign URL-with-port controls: `curl http://localhost:8080/metrics`, `http://localhost:8080/health`, `http://localhost:8080`, `https://example.invalid:8443/api/v1`, and `http://127.0.0.1:3000`.
- Added test `preserves_userinfo_delimiter_and_host_path` verifying exact preservation of `@`, host, port, and path.
- Raw execution arguments delivered to tools and private policy keys used by approval caches remain strictly unredacted.

## Exact Proof and Verification

### 1. Exact GREEN Regression
Command:
`cargo test --lib discord::approval::tests::approval_display_redacts_credentials_without_changing_execution -- --exact --nocapture`
- Exit code: 0 (`U04-green.exit`)
- Result: 1 passed, 0 failed (`U04-green.log`)
- Output: `U04 secret_exposed=false raw_args_equal=true executions=1 pending=0`
- Verified: Credential absent from outbound `ApprovalRequest` and Discord embed; raw execution arguments delivered to tool only after consent; pending requests clean up immediately upon resolution.

### 2. Sanitizer Unit Suite
Command:
`cargo test --lib security::approval_display -- --nocapture`
- Exit code: 0 (`U04-sanitizer.exit`)
- Result: 2 passed, 0 failed (`U04-sanitizer.log`)
- Verified: All recognized credential forms redacted (Bearer, Basic, env/JSON pairs, CLI flags, URLs with userinfo); benign commands and URL-with-port patterns preserved unchanged.

### 3. Adjacent Approval Suite
Command:
`cargo test --lib discord::approval::tests -- --nocapture`
- Exit code: 0 (`U04-adjacent-approval.exit`)
- Result: 13 passed, 0 failed (`U04-adjacent-approval.log`)
- Verified: All approval lifecycle, bot isolation, drop cleanup, custom ID parsing, and caching mechanisms remain green and unaffected.

### 4. Build and Diagnostics
- Diagnostics: LSP attempted once per touched file (`src/security/approval_display.rs`, `src/security/mod.rs`, `src/discord/approval.rs`); timed out waiting for shared daemon, standard compiler check used.
- Build: `cargo build` exit 0 (`U04-build.exit`, `U04-build.log`).
- Rustfmt: `rustfmt --edition 2024 --check src/security/approval_display.rs` clean (exit 0).
- Diff check: `git --no-pager diff --check src/security/approval_display.rs src/security/mod.rs src/discord/approval.rs` clean (exit 0, `U04-diff.exit`).

No network curl, actual Discord operations, or resource leaks.
