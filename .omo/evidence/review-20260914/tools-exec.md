# Lane: tools-exec
## Scope
- `src/tools/terminal.rs`: 1,321 LOC, read completely, including tests.
- `src/tools/file.rs`: 575 LOC, read completely, including tests.
- `src/tools/browser.rs`: 135 LOC, read completely.
- `src/tools/mcp.rs`: 343 LOC, read completely.
- `src/tools/mod.rs`: 436 LOC, read completely, including tests.
- Total: 2,810 LOC. Findings concern only these five files; no product files were changed.

## Findings
### [P0] Terminal working-directory checks are not filesystem confinement
- Location: src/tools/terminal.rs:354
- Evidence: `.current_dir(cwd)`
- Why it matters: Authorization checks cover the starting directory and explicitly pathed executable, but operands go straight to the child without confinement. For example, `{"program":"cat","args":["/etc/passwd"]}` starts in the workspace yet reads outside it; an allowed interpreter can also change directories or open arbitrary host paths. Children retain the gateway's OS permissions and inherited environment. This defeats treating the configured roots as a terminal sandbox, independently of shell metacharacter escaping. Approval does not impose OS confinement after a command is allowed.
- Suggested fix: Run terminal children in an OS-enforced filesystem sandbox exposing only authorized roots and an allowlisted environment. Do not present cwd validation as a security boundary; filtering path-looking operands is not sufficient for arbitrary programs.

### [P0] Model-controlled environment changes execution after approval classification
- Location: src/tools/terminal.rs:369
- Evidence: `command.env(key, value);`
- Why it matters: `require_approval` receives only the quoted program and argv, before executable resolution and environment application. A bare executable name is preserved for PATH lookup, while the caller can replace PATH with an arbitrary directory, including one outside all authorized roots. With an executable named `echo` in that directory, a benign-looking `echo hello` invokes that executable instead; neither the path check nor the displayed/scanned command covers it. Loader and interpreter environment variables similarly change semantics without entering approval, and caller entries overwrite the session variables installed immediately beforehand.
- Suggested fix: Resolve bare programs to trusted absolute executables before approval, reject caller overrides of PATH, loader/interpreter injection variables and gateway session identity, and bind approval to the actual executable and permitted environment changes.

### [P0] Terminal output limit is applied after unbounded capture
- Location: src/tools/terminal.rs:372
- Evidence: `let output = tokio::time::timeout(self.timeout, command.output())`
- Why it matters: `Command::output()` collects complete stdout and stderr before `capture` applies the nominal 100,000-byte limit. A continuously printing command such as `yes`, or a fast child emitting a huge finite result, can exhaust gateway memory well before the default 600-second timeout. The timeout bounds elapsed time, not bytes allocated.
- Suggested fix: Spawn with piped output and drain both streams concurrently into bounded head/tail buffers; optionally terminate the process tree when a total output budget is exceeded.

### [P1] Terminal timeout does not terminate descendants
- Location: src/tools/terminal.rs:356
- Evidence: `.kill_on_drop(true);`
- Why it matters: This controls the immediate child only; no process group or job object is created or killed. An allowed process can start a long-lived background child with redirected stdio and exit, leaving that descendant running after a successful tool response. A descendant holding stdout/stderr open can instead keep collection pending until timeout and remain alive afterward. Repeated calls can accumulate orphan processes and continue side effects beyond the advertised deadline.
- Suggested fix: Own an isolated process group/job for each execution, terminate the entire group on completion, timeout and cancellation, and explicitly wait for the owned child during bounded cleanup.

### [P1] Output byte caps do not cap the returned UTF-8 strings
- Location: src/tools/terminal.rs:561
- Evidence: `return (String::from_utf8_lossy(bytes).into_owned(), false);`
- Why it matters: The limit is measured on raw bytes, but invalid bytes expand to three-byte UTF-8 replacement characters. Exactly `limit` invalid bytes can therefore produce roughly three times the advertised byte cap with `stdout_truncated=false`. The truncated branch adds its marker on top of the budget; FileTool's large-file branch has the same lossy-decoding and marker expansion. Consumers cannot use these settings as strict response-size budgets.
- Suggested fix: Budget the encoded returned text, reserve space for the truncation marker, and truncate on UTF-8 boundaries after decoding. Distinguish raw-capture budgets from serialized-response budgets if both are needed.

### [P0] File confinement is vulnerable to path replacement between checks and use
- Location: src/tools/file.rs:137
- Evidence: `fs::write(&target, content).await.map_err(tool_error)?;`
- Why it matters: Metadata/canonicalization checks and the actual open use separate pathname lookups. A workspace writer can replace a checked final file or ancestor with a symlink after validation but before this write, causing an outside file to be truncated or overwritten; the later `ensure_inside_root` detects the escape only after damage. Reads, listing and search also reopen checked paths, permitting outside reads/traversal under the same race. Additionally, the primary root is canonicalized afresh on each authorization check, so replacing the configured root path with an outside-pointing symlink can redefine the boundary between calls.
- Suggested fix: Pin the authorized directory identities at construction and use descriptor-relative, beneath-root/no-follow operations for traversal and opening. Perform reads/writes through the validated handles rather than reopening canonical path strings; post-write checks are not a substitute.

### [P0] Hard links bypass the file write boundary without a race
- Location: src/tools/file.rs:130
- Evidence: `Ok(_) => {`
- Why it matters: An existing hard link inside the workspace is a regular file whose canonical path remains inside the root, so this branch accepts it. If it shares an inode with an outside file, writing the workspace path also truncates/changes the outside file. This needs same-filesystem hard-link creation permission or a preexisting link, but requires no symlink or timing race. Read/search also cannot distinguish such aliases by canonicalizing their path.
- Suggested fix: Replace files using a new safely created inode and atomic rename instead of truncating the existing inode. If outside-data reads through hard links must also be forbidden, enforce an isolated workspace filesystem/import policy rather than relying solely on pathname checks.

### [P0] File writes destroy old contents before replacement is safely available
- Location: src/tools/file.rs:137
- Evidence: `fs::write(&target, content).await.map_err(tool_error)?;`
- Why it matters: Writing an existing file truncates it in place. ENOSPC or another write error after opening leaves the previous document empty or partially replaced despite returning an error. Concurrent writes to the same path can also interleave rather than yielding one complete version. This is a data-loss path for ordinary workspace edits, separate from escaping the workspace.
- Suggested fix: Write a complete temporary sibling through a safe directory handle, flush it as required, then atomically rename it over the destination; preserve the original on failures before replacement.

### [P0] File read limits do not bound file buffering
- Location: src/tools/file.rs:77
- Evidence: `let bytes = fs::read(path).await.map_err(tool_error)?;`
- Why it matters: The entire file is allocated before the 200,000-byte limit is checked. Reading a multi-gigabyte or large sparse regular file can exhaust memory even though only head/tail text is returned. Search has a related size-check race: it checks metadata for a one-MiB size, then uses an unbounded `read_to_string` on a file that can grow before or during the read.
- Suggested fix: Open a validated regular-file handle and read only bounded data. For head/tail output, seek and read bounded slices; enforce a byte budget during search reads rather than relying on a prior metadata size.

### [P0] Directory listing and search traversal have no resource budget
- Location: src/tools/file.rs:208
- Evidence: `entries.push(json!({`
- Why it matters: Listing materializes and sorts every directory entry without an entry or byte cap. Search similarly pushes every child directory entry into `pending` before processing it; its 1,000-match limit does not bound traversal when a query does not match. Large workspace directories can consume unbounded memory and work. Even capped search matches can approach one GiB of content because each of 1,000 matching lines can be almost one MiB. There is no file-operation deadline or cooperative search cancellation in these modules.
- Suggested fix: Add paginated listing, entry/visited-path and aggregate-result byte budgets, and a deadline/cancellation signal for search. Traverse with a bounded frontier instead of enqueueing an entire directory and report budget exhaustion explicitly.

### [P1] File operations can block indefinitely on special files
- Location: src/tools/file.rs:329
- Evidence: `let Ok(content) = std::fs::read_to_string(&path) else {`
- Why it matters: Search skips symlinks/directories and large metadata lengths, but never requires `metadata.is_file()`. A FIFO in the workspace usually has length zero and makes this blocking open/read wait indefinitely for a peer. File writes reject symlinks and directories but also accept FIFOs/devices before writing, checking regular-file status only afterward. Repeated calls can occupy Tokio blocking workers; cancelling a `spawn_blocking` join does not stop an already running filesystem operation.
- Suggested fix: Reject every non-regular file before I/O and enforce that property on the opened handle using no-follow/nonblocking opening where appropriate. Use cancellable bounded reads for search rather than unbounded blocking device/FIFO I/O.

### [P1] Synchronous filesystem lookups run on async executor workers
- Location: src/tools/file.rs:71
- Evidence: `let metadata = std::fs::symlink_metadata(&path).map_err(tool_error)?;`
- Why it matters: Async file operations repeatedly invoke synchronous metadata/canonicalization/exists checks directly. Terminal execution also calls synchronous canonicalization from its async path. Slow or stalled network/FUSE storage can block runtime workers, delaying unrelated requests, approval processing and timers. Moving only the recursive search loop to `spawn_blocking` does not cover these calls.
- Suggested fix: Move each coherent validation/open operation to the blocking pool or use asynchronous filesystem APIs. Combine this with handle-based confinement so additional awaits do not enlarge pathname race windows.

### [P1] Concurrent creation of the same parent spuriously fails a valid write
- Location: src/tools/file.rs:182
- Evidence: `fs::create_dir(&current).await.map_err(tool_error)?;`
- Why it matters: Two writes creating different files under the same initially absent parent can both observe NotFound. One creates the directory; the other receives AlreadyExists and aborts even though the required parent is now valid. The subsequent metadata validation that would establish this is never reached on that error.
- Suggested fix: Treat AlreadyExists from directory creation as a request to revalidate the existing directory with the same no-symlink/authorization checks; propagate all other errors.

### [P1] Search silently reports incomplete results as ordinary success
- Location: src/tools/file.rs:330
- Evidence: `continue;`
- Why it matters: Any `read_to_string` error, including PermissionDenied or an I/O failure, silently removes that file from the search. Files above the size limit are also skipped, and reaching 1,000 matches returns early without a truncation indicator. A caller can receive an empty or apparently complete `matches` array despite unreadable files or undisclosed skipped results, which is unsafe for audits or automated edits based on search completeness.
- Suggested fix: Return completeness metadata with skipped/error counts and `truncated`; propagate unexpected I/O errors or report them explicitly, while distinguishing intentionally skipped binary/large files.

### [P0] Browser navigation permits loopback and link-local SSRF
- Location: src/tools/browser.rs:97
- Evidence: `if !matches!(parsed.scheme(), "http" | "https") {`
- Why it matters: Scheme validation is the only destination policy. Inputs such as `http://127.0.0.1:8080/`, `http://[::1]/`, or `http://169.254.169.254/` are passed to Chrome's new-tab navigation, reaching services from the browser host's network position. Public names resolving to private addresses and redirects are not constrained either. This permits internal GET side effects/probing even though this implementation cannot evaluate JavaScript or extract arbitrary response bodies.
- Suggested fix: Enforce a browser egress policy denying loopback, private, link-local and other nonpublic destinations unless explicitly authorized, including DNS resolution, redirects and subresources. Application-level URL checks alone cannot enforce Chrome's subsequent network behavior.

### [P0] Browser snapshot exposes pages across sessions
- Location: src/tools/browser.rs:87
- Evidence: `"pages": pages`
- Why it matters: Snapshot returns the entire `/json/list` result for the single configured CDP port. BrowserTool has no session-specific state or `execute_with_context` override, so sessions sharing the tool can see each other's page titles, URLs and debugger metadata; a CDP instance shared with a human browser exposes that user's tabs as well. URL query strings or fragments may contain sensitive information. This requires a shared instance/profile, not an attacker-selected CDP port.
- Suggested fix: Allocate isolated browser profiles/contexts per session and return only tracked pages belonging to that session. Do not attach the agent tool to a personal browser profile or expose unnecessary debugger metadata.

### [P0] Every browser navigation permanently adds a tab
- Location: src/tools/browser.rs:103
- Evidence: `"http://127.0.0.1:{}/json/new?{}",`
- Why it matters: Every navigate creates another target rather than reusing an owned one, and no action closes targets or limits their count/lifetime. Repeated model calls accumulate browser tabs, renderers and page background work without bound. The ten-second HTTP timeout limits CDP requests, not the lifetime of opened pages. Every subsequent call also fetches the growing complete page list.
- Suggested fix: Reuse a session-owned target, enforce a small per-session/global target quota, and close owned targets on replacement and session teardown, including cleanup after partial failures.

### [P1] Browser CDP responses have no byte or entry cap
- Location: src/tools/browser.rs:79
- Evidence: `.json()`
- Why it matters: Page listing and new-tab responses are decoded in full without response-size checks, and snapshot forwards all listing data. A compromised/replaced local CDP service or a sufficiently large page list can force large allocations and oversized tool results. The configured time limit does not bound bytes received within that interval.
- Suggested fix: Stream CDP response bodies through a strict byte limit before deserializing, validate expected shapes, and cap/redact returned page entries and fields.

### [P0] MCP input writes can deadlock before the timeout starts
- Location: src/tools/mcp.rs:133
- Evidence: `stdin.write_all(&encoded).await.map_err(mcp_error)?;`
- Why it matters: All request bytes are written and `stdin.shutdown()` is awaited before stderr draining and stdout reading begin, with no deadline around these writes. A server that fills stderr or stdout before reading stdin can deadlock with a request larger than the stdin pipe capacity. A server that simply does not consume stdin can also hang the write indefinitely. The stdio mutex remains held, blocking all subsequent calls on that client; the later read-only timeout is never reached.
- Suggested fix: Start draining stderr/stdout immediately after spawn and write requests concurrently. Apply one end-to-end deadline covering lock acquisition, spawn/write, response and cleanup, with explicit cancellation cleanup.

### [P0] MCP server output can allocate unbounded buffers
- Location: src/tools/mcp.rs:149
- Evidence: `while let Some(line) = lines.next_line().await.map_err(mcp_error)? {`
- Why it matters: Stdio `lines()` buffers an unlimited line before parsing it. A malicious or broken configured server can stream bytes without a newline until the gateway runs out of memory. The SSE parser likewise grows its String without a maximum until newline, and ordinary HTTP JSON bodies are collected without a byte limit. Results are then returned without an aggregate output cap. The 600-second default timeout is not a memory bound.
- Suggested fix: Enforce per-frame, per-response and total-output byte limits before allocation/deserialization for every transport. Abort oversized streams and terminate/reap the corresponding stdio process tree.

### [P0] MCP process cleanup can hang after a response and leak after timeout
- Location: src/tools/mcp.rs:163
- Evidence: `let _ = stderr_task.await;`
- Why it matters: After a response, only the immediate server child is killed, then stderr draining is awaited without a timeout while the client mutex remains held. A server descendant retaining the stderr descriptor prevents EOF forever, so a successful response never returns and later calls stall. On read timeout, the preceding `?` exits before this cleanup; dropping the JoinHandle detaches the drain task, and descendants are not killed by the child's kill-on-drop setting. Kill and drain errors are discarded, concealing failed cleanup.
- Suggested fix: Put process-tree termination and bounded task cleanup on every completion/cancellation path. Abort/join the stderr task when its deadline expires, explicitly reap the owned child, and surface cleanup failures instead of ignoring them.

### [P1] MCP ordinary JSON response bodies are outside the timeout
- Location: src/tools/mcp.rs:199
- Evidence: `response.json().await.map_err(mcp_error)`
- Why it matters: The HTTP timeout wraps only `builder.send()`, which can finish after headers; the client is constructed without a total request timeout. For a non-SSE response, a server can send successful JSON headers and then stall or trickle the body indefinitely. This path never reaches the SSE read timeout and does not honor `with_timeout` for the complete request.
- Suggested fix: Use a single deadline spanning send and bounded body decoding, or configure the reqwest client's total timeout and retain explicit size limits.

### [P0] MCP calls omit the required session initialization lifecycle
- Location: src/tools/mcp.rs:81
- Evidence: `"method": "tools/call",`
- Why it matters: Every stdio call starts a fresh server and sends `tools/call` as its first and only request. No `initialize` exchange or `notifications/initialized` is sent. Standard MCP servers that require initialization reject these calls before executing the requested tool. HTTP requests likewise never negotiate or retain a session ID. A one-shot JSON-RPC fixture can work while a compliant stateful MCP server cannot.
- Suggested fix: Implement MCP initialization/version/capability negotiation before calls, keep stdin/session state alive until the matching response, and retain/reuse the session as required by the selected transport. If only one-shot RPC is intended, expose it separately rather than claiming general MCP compatibility.

### [P0] MCP treats notifications as the requested response
- Location: src/tools/mcp.rs:151
- Evidence: `return Ok(value);`
- Why it matters: The stdio reader returns the first parseable JSON value, and the SSE reader does the same for a data line. A legitimate progress/log notification with no ID, or a server-initiated request, arriving before the tool result is therefore returned to `decode_response`, which fails the ID check instead of waiting for the actual response. For stdio, the server is killed before decoding, so the real response is lost even if the tool already performed its side effect.
- Suggested fix: Correlate response IDs inside the bounded receive loop; dispatch/ignore valid notifications appropriately and handle server requests separately. Stop only on the matching result/error or a protocol violation/deadline.

### [P0] MCP SSE decoding corrupts UTF-8 split across network chunks
- Location: src/tools/mcp.rs:308
- Evidence: `buffer.push_str(&String::from_utf8_lossy(&chunk.map_err(mcp_error)?));`
- Why it matters: HTTP chunk boundaries need not align with UTF-8 character boundaries. Converting each chunk independently with lossy decoding replaces pieces of a valid split character with U+FFFD. A result containing, for example, a non-ASCII filename split mid-character is silently changed before JSON parsing and returned as successful data. This is deterministic data corruption for that network segmentation, not invalid server JSON.
- Suggested fix: Buffer bounded raw bytes until a complete SSE line/event is available, or use an incremental UTF-8 decoder preserving incomplete trailing sequences; reject genuinely malformed UTF-8 rather than silently changing protocol data.

### [P0] MCP SSE parsing ignores event framing and multiline data
- Location: src/tools/mcp.rs:313
- Evidence: `if let Ok(value) = serde_json::from_str::<Value>(data) {`
- Why it matters: SSE combines consecutive `data:` lines into one event separated by newlines and dispatches at a blank line. This parser instead tries to parse each data line independently and returns immediately on any standalone JSON value. A valid event with `data: {` followed by JSON members and a closing-brace data line is discarded line by line, yielding timeout/closed-stream error instead of its result. Conversely a complete JSON data line is accepted before the event is complete.
- Suggested fix: Use an SSE parser or accumulate data fields until the blank-line event boundary, then parse the combined payload and correlate its JSON-RPC ID under explicit size/deadline limits.

## Strengths
- Terminal and MCP launches use `Command::new(...).args(...)`, not implicit shell interpolation. Terminal approval text shell-quotes argv, including embedded single quotes; deliberately invoking a shell remains distinct from accidentally injecting one.
- File and terminal checks canonicalize paths and use component-aware `Path::starts_with`; ordinary static `..` and outside symlink traversal are rejected. File writes explicitly reject final-component symlinks before attempting the write.
- Terminal and registry approval paths fail closed when a required session/requester is absent, preserve denial reasons and bound approval waits. Registry rule identities length-prefix the tool name and include the reason; aggregate MCP approval reasons distinguish registered clients/methods.
- MCP validates response IDs and JSON-RPC errors, uses kill-on-drop for immediate children, and drains stderr to a sink rather than accumulating logs. Browser navigation rejects non-HTTP(S) schemes and percent-encodes its CDP query argument.

## Notes
- This is a source-proven review, not a claim that exploits were executed. Complete reads, targeted rereads and structural scans covered panic/unwrap sites, blocking filesystem calls, ignored results, locks/awaits, growth, timeouts and TODO/FIXME/HACK markers. Panic-style unwraps found by the requested scan are in test code; no production explicit unwrap/panic was identified in these five files. No SQL or scheduling implementation exists in this target surface.
- The request's P0 rubric includes correctness bugs, security holes, data loss and unbounded production resource growth; P0 above does not mean every issue is remotely exploitable without prerequisites. Workspace mutation, configured-server misbehavior, shared browser profiles and absence of an outer OS sandbox are stated where relevant.
- MCP program/argv/cwd/HTTP URL are configured transport fields, not taken from per-call model arguments; model arguments are JSON-serialized, including embedded newlines. Arbitrary configured programs running with inherited host permissions/environment is a trust assumption, not independently proven model-controlled shell injection. Provisioning authorization and any outer sandbox/egress controls are outside this lane and were not assumed to exist or to be absent globally.
- Browser `eval` and `screenshot` return explicit not-configured errors, so no JavaScript injection through those actions is claimed. The SSRF finding concerns browser network requests, not demonstrated extraction of metadata bodies.
- No definite zombie-reaping defect is claimed from kill-on-drop alone: Tokio has child-reaping behavior. The identified lifetime defects concern descendants, pipe ownership, detached drain tasks and unbounded cleanup waits visible in these implementations.
- Source-bound review with deterministic citation checking was chosen over exploit execution: filesystem races, hostile process trees and browser mutations would require writes or side effects incompatible with the read-only lane. No product build/tests or live exploit probes were run; embedded tests were read, not presented as passing. No tests were added for this report.
