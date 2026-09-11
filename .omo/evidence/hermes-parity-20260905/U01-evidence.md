# U01 AP.AP04 evidence

## Registration before production edits

Exact test ID: `tools::terminal::approval_tests::approval_detector_preserves_executable_semantics`.

Literal RED and GREEN command: `cargo test --lib tools::terminal::approval_tests::approval_detector_preserves_executable_semantics -- --exact --nocapture`

Scenarios and binary expected failures before repair:
- Detector-only `rm -rf //*`: None instead of hardline. Never execute root payload.
- Detector `echo "$(reboot)"`, `grep "$(reboot)" file`: None instead of hardline; inert `echo 'reboot'` remains benign. Local TerminalTool shell uses per-child PATH with temporary reboot that only writes marker: expected rejected, marker absent.
- `python3 -cprint(1)`, `node --eval=process.exit(0)`, `rg --pre ./helper needle file`: false instead of dangerous. Positional `python script.py -c` and long-option `bash --norc script.sh`: true instead of benign. Local rg helper only writes marker; rejection must prompt once and not run helper.
- TerminalTool JSON `{"program":"rm","args":["-rf","<temp>/a'b"]}` with rejecting requester: directory removed and zero prompts instead of intact/one prompt. `printf` argv `%s`, `a'b`: rejected instead of exact stdout.

The single test accumulates mismatches so every constituent observable is captured in one behavioral RED (not compile failure or zero tests). TempDir explicit close cleans fixtures even on accumulated failure. No global environment modifications, live Discord, or root command execution. This is dormant/public TerminalTool proof, not daemon-bypass proof.

## Captured RED before production edits

`U01-red.log`, `U01-red.exit`: exit 101, 1 executed test, 0 passed, 1 failed. Literal observed mismatches: repeated-slash root None; python attached/node equals/rg pre false; positional python and bash long-option true; printf hardline malformed; rg helper marker=true, requests=0.

Audit predictions NOT reproduced: both substitutions already returned system shutdown/reboot, both local Never fixtures blocked with marker=false; quoted-path rm returned Approval, exists=true, requests=1. They remain regression controls, not claimed repaired bypasses. Only confirmed behavior is patched. No production edits preceded this capture.

No monitor tool is exposed; asynchronous shell jobs are awaited using macOS kqueue process-exit events with bounded waits (no sleeps/polling). Requested programming/Rust/debugging skills were not available in the exposed skill directory.

## GREEN and local surface

Identical registered command: `U01-green.log`, exit `U01-green.exit` = 0; 1 passed, 0 failed. Same regression and fixture payloads as RED. Repeated-slash root now hardline; attached python/node and rg pre now dangerous; positional python and bash --norc now benign; printf stdout exactly a'b; rg helper denied with marker=false and requests=1. Already-passing substitution and rm controls retained their results.

Extended adjacent local controls then added to the same regression: actual TerminalTool rejects attached python/node with one request each; direct echo argv containing $(reboot) prints literal text; approving the same real rg helper executes it successfully and writes exactly `ran`. `U01-green-final.log` / `.exit`: identical command, 1 passed, exit 0. This exercises production Tool::execute/execute_with_context, detection, real requester dispatch, and real subprocess creation, not a mocked detector or shell. Only the requester decision is stubbed. No arbitrary sleeps or polling; process output completion is awaited, with production kill-on-drop and timeout. TempDir explicitly closes all scripts/files/marker after output completion; no background fixture task or listener exists.

## Adjacent verification

- `cargo test --lib security::tests::`: `U01-adjacent-security.log`, exit 0, all 7 passed (dangerous categories, interpreter payloads, grep isolation, hardline, deny globs/reasons).
- `cargo test --lib tools::terminal::approval_tests::`: `U01-adjacent-terminal.log`, exit 0, all 20 passed (including approved execution, rejection, Never/yolo, deny globs, paths and environment).
- `cargo build`: `U01-build.log`, exit 0.
- `git diff --check -- src/security/normalize.rs src/security/hardline.rs src/tools/terminal.rs`: exit 0, no output.
- LSP diagnostics attempted once for each of the three touched files, all returned daemon unreachable at `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`. No global repair attempted. Cargo compilation/build verified types instead; LSP diagnostics unavailable.

## Patch and boundaries

- `src/security/hardline.rs:27`: recognize empty slash groups in root path.
- `src/security/normalize.rs:848`: parse attached Python -c and Node --eval=/--print=, recognize rg --pre/--pre=, stop execution-option interpretation after positional scripts/--, distinguish shell long options from short -c clusters.
- `src/tools/terminal.rs:283`: quote argv operands for detector representation; real process still receives original program and argv. Interpreter payload extraction retains ownership of executable strings. No approval API change.
- `src/tools/terminal.rs:584`: registered deterministic regression and local surface QA.

Only these three owned source files and U01-* evidence were written by this worker. No edits to protected user backend/config/tests, baseline.patch, dependencies, global environment, production state, or other workers' files. Shared workspace contains other workers' concurrent changes; they were not reverted. No commit/push. No entire parity-goal or live daemon protection claim.

## Revalidation by resumed child st_01a0711b

The patch, regression, registration, and RED/GREEN captures above already existed at this child's entry. This child inspected them and preserved them; it did not recreate or claim a new pre-edit RED. No additional production/test edits were necessary. Current line references are hardline.rs:27, normalize.rs:848 (execution_flag_findings), terminal.rs:315 (argv representation), and terminal.rs:612 (registered regression). Scanner changes in terminal.rs belong to U05 and were left intact.

All five literal commands in U01-commands.json were executed successfully against the current shared tree. Fresh outputs are U01-revalidation-selected.log (1 passed), U01-revalidation-security.log (7 passed), U01-revalidation-terminal.log (20 passed), U01-revalidation-build.log, and U01-revalidation-diff.log, with matching .exit files all containing 0. The selected command is identical to the registered RED command. Its actual TerminalTool fixtures again prove quoted-path rm untouched with one request, printf exact stdout, both Never substitutions blocked without marker creation, denied rg without helper execution, and successful helper execution after consent. Explicit TempDir close passed.

Unlike the historical infrastructure limitation above, this child's single diagnostics attempt on each owned source file returned `No diagnostics found`. No global repair was performed. No monitor tool is exposed; commands used asynchronous Popen with bounded communicate completion and no fixed sleeps or test polling. This revalidation wrote only U01-* evidence files and did not modify production files or protected baseline/user changes.
