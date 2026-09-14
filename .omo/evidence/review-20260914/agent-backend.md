# Lane: agent-backend
## Scope
- `src/agent/omo_backend.rs`: 1,396 LOC, read fully.
- `src/agent/omo_daemon.rs`: 837 LOC, including tests, read fully.
- `src/agent/omo_protocol.rs`: 172 LOC, including tests, read fully.
- `src/agent/omo_activity.rs`: 178 LOC, including tests, read fully.
- `src/agent/backend.rs`: 57 LOC, read fully.
- Total: 2,640 lines as reported by `wc -l`. Review targets only; no product files changed.

## Findings
### [P0] Deadline drain can deliver another turn's output as this turn's success
- Location: src/agent/omo_backend.rs:638
- Evidence: `if status == Some("completed")`
- Why it matters: During deadline cleanup, the drain appends every agent-message delta, overwrites content on every completed agent-message item, and returns the first turn/completed, without filtering threadId or turnId. Although terminal_confirmed checks identity, it gates only removal from active_turns; this success branch ignores it. A replayed completion for a different turn can therefore cause this session to emit/persist unrelated text, run its cron acknowledgement, and return success while its own turn remains unresolved. Even a correctly correlated terminal can deliver content contaminated by unrelated preceding deltas.
- Suggested fix: Apply the normal thread/turn identity filter before modifying drain content or selecting a terminal; require terminal_confirmed and completed status before entering successful finalization.

### [P0] Completed message items erase earlier messages in the same turn
- Location: src/agent/omo_backend.rs:963
- Evidence: `full_content.clear();`
- Why it matters: Content is accumulated in one turn-wide string rather than by item ID. For two agent-message items A and B, completing B clears all of A, so the final chunk and persisted transcript contain only B. Interleaved item completions can also erase another item's in-flight deltas. The deadline drain repeats the same replacement behavior.
- Suggested fix: Track text per item ID and replace only that item's snapshot on completion; render the ordered collection of message items for streaming and final output.

### [P0] Interim streaming bypasses suppression and final-content filtering
- Location: src/agent/omo_backend.rs:978
- Evidence: `full_content.clone(),`
- Why it matters: Every nonempty delta is dispatched immediately as raw cumulative content. cron_suppress_direct_emission is checked only during finalization, and filter_reasoning/is_explicit_silence are also applied only then. A suppressed cron turn still produces OutboundAction::Stream requests, and text destined to be filtered or recognized as silence has already been sent to the dispatcher. Returning without a final chunk cannot undo those earlier emissions. The defect is at this backend's dispatch boundary; downstream rendering was outside scope.
- Suggested fix: Check direct-emission suppression before any stream dispatch. Apply a streaming-safe filter that withholds undecidable reasoning/silence prefixes, or buffer until finalization when those policies require it.

### [P0] Valid notifications arriving before the start response are discarded permanently
- Location: src/agent/omo_backend.rs:921
- Evidence: `&& (!turn_started_ack`
- Why it matters: All turn-bearing notifications are ignored until the numeric-ID-3 response has been consumed. If a fast peer emits current-turn deltas, item completions, or its terminal notification before replying to turn/start, those messages are lost rather than buffered. After the response identifies the turn, there is no replay of the discarded frames: the result can be incomplete, or an already completed turn can time out. JSON-RPC alone does not guarantee response-before-notification ordering.
- Suggested fix: Keep a bounded pre-ACK notification buffer, then replay only frames matching the acknowledged thread/turn; continue rejecting unrelated subscription replay.

### [P0] Stream accumulation has no byte bound and repeatedly copies the entire prefix
- Location: src/agent/omo_backend.rs:972
- Evidence: `full_content.push_str(delta);`
- Why it matters: An arbitrary number of individually valid delta frames grows full_content without a byte cap. Every delta clones the accumulated string for dispatch, producing quadratic cumulative copying/dispatch volume for many small deltas. A wall-clock deadline is not a memory quota: a fast peer, long cron deadline, or concurrent turns can exhaust memory well before it expires. started_ids and tool_call_counts likewise have no per-turn cardinality limits.
- Suggested fix: Enforce aggregate per-turn text/item limits, interrupt and report overflow explicitly, and coalesce cumulative stream updates instead of cloning every prefix on every delta.

### [P0] Session thread cache grows for the lifetime of the backend
- Location: src/agent/omo_backend.rs:414
- Evidence: `self.thread_ids.lock().insert(storage_key, id_str.clone());`
- Why it matters: Each new non-cron session adds a storage key and remote thread ID to the backend-wide HashMap. Successful turn completion and cancellation never evict it; the only removals in this file handle cron sessions or a missing remote rollout. Sustained creation of distinct sessions therefore produces unbounded retained memory, even when their turns finish and their session state already stores the binding.
- Suggested fix: Bound this fallback cache with eviction, remove entries on session retirement, or eliminate the duplicate cache when session metadata/persistence provides the binding.

### [P0] Approval requests bypass thread and turn ownership validation
- Location: src/agent/omo_backend.rs:880
- Evidence: `let _ = ws.send(approval_allow_response(req_id)).await;`
- Why it matters: Approval handling occurs before the thread/turn filter and exits via continue. A request carrying another threadId/turnId, or arriving before this turn is acknowledged, is evaluated using the current session's toolsets and YOLO flag. Under YOLO it can receive an allow response for work this session does not own. The later replay/correlation guard never sees that request.
- Suggested fix: Validate request ownership before evaluating approval policy. For pre-ACK requests, correlate them after ACK using a bounded pending-request buffer; reject requests that cannot be safely attributed.

### [P0] Readiness probe buffers an unlimited HTTP response body
- Location: src/agent/omo_daemon.rs:54
- Evidence: `stream.read_to_end(&mut buf).await.ok()?;`
- Why it matters: Readiness needs only the HTTP status, but read_to_end grows a Vec until EOF. A service on the local configured port can stream a very large response during the two-second probe window, exhausting memory; repeated probes repeat the allocation. The timeout bounds elapsed time, not response bytes.
- Suggested fix: Read only a bounded status line/header, reject oversized headers, and stop as soon as the status is established without consuming the body.

### [P1] Turn deadlines do not bound socket writes or downstream dispatch
- Location: src/agent/omo_backend.rs:508
- Evidence: `ws.send(turn_start_request(&thread_id, &user_prompt, model))`
- Why it matters: This send awaits without timeout, as do initialize/resume/start and in-turn interrupt/approval writes. A peer that stops reading can block a large prompt or later write beyond the total deadline. Typing and stream dispatch, database work, and final acknowledgement execution are also awaited outside a turn-wide timeout. Deadline checks surrounding ws.next cannot run while any such await is pending. In particular, a stalled interrupt write prevents the supposedly reserved cleanup timer from even starting.
- Suggested fix: Bound external awaits by the applicable work/cleanup deadline and preserve unresolved ownership on ambiguous write timeout; bound finalization separately rather than silently abandoning a delivered-but-unrecorded result.

### [P1] A WebSocket ping or pong terminates interrupt cleanup
- Location: src/agent/omo_backend.rs:588
- Evidence: `while let Some(Ok(Message::Text(text))) = ws.next().await {`
- Why it matters: This while-let ends the entire drain on any non-Text frame, not just on disconnect/error. A normal ping/pong arriving ahead of the interrupt ACK or matching terminal makes cleanup stop immediately, return a deadline error, and retain unresolved active-turn ownership despite a live connection and remaining cleanup budget.
- Suggested fix: Match frames inside the loop; continue over control frames, handle close/errors explicitly, and stop only on a correlated interrupt response/terminal or the actual cleanup deadline.

### [P1] Fast empty terminal notifications are forgotten instead of finalized
- Location: src/agent/omo_backend.rs:1034
- Evidence: `if started_at.elapsed() < no_content_grace {`
- Why it matters: A correctly correlated completed terminal with no text/tool calls is discarded during the startup grace interval. If it is a genuine empty completion, the peer need not send another terminal. Nothing records it for reconsideration when the grace elapses, so the session waits until inactivity/total timeout and may unnecessarily interrupt a finished turn instead of returning the intended empty-result error and clearing ownership.
- Suggested fix: Finalize a correlated terminal immediately, or retain the suspected premature terminal and schedule an explicit bounded reconsideration rather than losing the only terminal evidence.

### [P1] Initialize and resume correlate by numeric ID without checking message kind
- Location: src/agent/omo_backend.rs:167
- Evidence: `if val.get("id").and_then(Value::as_u64) == Some(1) {`
- Why it matters: Initialize accepts any parsed object with ID 1 and no error as success, even a server-to-client JSON-RPC request with method and no result. Resume similarly treats ID 2 as a successful response and persists the binding. Request IDs in opposite directions can coincide; a reverse request must not satisfy a pending client request. Malformed response objects also advance setup without a valid result.
- Suggested fix: Require a response envelope (no method, expected ID, exactly one of result/error) and validate each method's result shape before advancing setup; route reverse requests separately.

### [P1] Cooperative cancellation hides an unresolved remote interrupt
- Location: src/agent/backend.rs:47
- Evidence: `let _ = self.cancel(session).await;`
- Why it matters: OmoBackend::cancel deliberately returns errors for an unacknowledged start, rejected interrupt, disconnect, and cleanup timeout. This default wrapper discards all those distinctions and returns only agent turn cancelled. The remote turn may still be executing, and its retained ownership can reject subsequent submissions, but the caller receives no cleanup failure or indication that cancellation is unresolved.
- Suggested fix: Preserve the cancellation error in the returned error/context and log it with session identity; distinguish a cancelled local future from confirmed remote cancellation.

### [P1] Approval-denial budget does not bound actual denials per turn
- Location: src/agent/omo_backend.rs:870
- Evidence: `let _ = ws.send(approval_denial_response(req_id)).await;`
- Why it matters: Disabled-terminal denials take this branch and continue before incrementing approval_denials, so that denial loop never reaches APPROVAL_DENIAL_TURN_LIMIT. For other approvals, every item/started or item/completed resets the counter to zero, allowing a peer to interleave denied attempts with normal item lifecycle events and avoid the purported per-turn limit indefinitely until the total deadline.
- Suggested fix: Increment one cumulative turn-scoped counter for every policy denial, including disabled tools, and reset it only when beginning a genuinely new turn.

### [P1] Approval write failures are silently ignored
- Location: src/agent/omo_backend.rs:902
- Evidence: `let _ = ws.send(approval_denial_response(req_id)).await;`
- Why it matters: Both allow and deny paths discard the send Result and continue consuming input. If the connection fails while answering a pending approval, the backend loses the immediate transport error and may leave the peer waiting for an answer while consuming the remaining turn budget. There is no retry/cancellation/error propagation tied to that failed response.
- Suggested fix: Propagate approval send failures with request/session context while retaining ambiguous active-turn ownership; make the caller's cancellation path aware that the reply was not confirmed sent.

### [P1] Unsupported reverse JSON-RPC requests receive no response
- Location: src/agent/omo_backend.rs:856
- Evidence: `if is_approval_request(method) {`
- Why it matters: An inbound object with both id and method is answered only when the method matches the approval heuristic. An unknown server request falls through to notification handling and ultimately the no-op match arm. A peer waiting on such a request receives neither a result nor a JSON-RPC method-not-found error and can stall the turn until timeout. Setup loops also ignore reverse requests except for the numeric-ID misclassification described above.
- Suggested fix: Explicitly distinguish requests from notifications and send a correlated -32601 error for unsupported request methods; notifications may still be ignored.

### [P1] Readiness depends on EOF and accepts malformed HTTP status prefixes
- Location: src/agent/omo_daemon.rs:56
- Evidence: `Some(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200"))`
- Why it matters: Status is inspected only after read_to_end completes. A service that has already sent a complete 200 response but delays connection close past the probe limit is reported unready, causing unnecessary spawn attempts or termination of an owned replacement. Conversely, strings such as HTTP/1.1 2000 or HTTP/1.1 200garbage pass this prefix check despite not being HTTP status 200.
- Suggested fix: Parse an exact three-digit status from a bounded HTTP status line and determine readiness without waiting for EOF or a body.

### [P1] Valid WebSocket URLs without an explicit port cannot pass readiness
- Location: src/agent/omo_daemon.rs:51
- Evidence: `let mut stream = tokio::net::TcpStream::connect(authority).await.ok()?;`
- Why it matters: For ws://localhost or ws://127.0.0.1/path, authority has no port. TcpStream::connect on that string cannot infer WebSocket's default port 80, although the backend's WebSocket URL parser can. is_local_url accepts these URLs, so ensure can try to spawn over an already serving endpoint and can never recognize readiness even if its child successfully listens there.
- Suggested fix: Parse the URL once and connect using its hostname and explicit/default port; use the same canonical endpoint interpretation for locality, probing, and spawning.

### [P1] Respawn bypasses the lock protecting initial daemon creation
- Location: src/agent/omo_daemon.rs:348
- Evidence: `match daemon_command(&bin, &url).spawn() {`
- Why it matters: ensure holds SPAWN_LOCK across probe/spawn/readiness, but the watcher does not acquire it. If ensure is invoked for the same local endpoint while an existing supervisor is restarting, both paths can observe an unready port and spawn children. One child then loses the bind race; a probe can even attribute the winner's readiness to the losing child, leaving duplicate or misleading supervision. The existing lock therefore does not serialize all in-process spawn paths.
- Suggested fix: Acquire the same endpoint-scoped spawn lock in the watcher and re-probe under it immediately before spawning; tie ownership/readiness to the child being installed.

### [P1] Shutdown kills only the direct process, leaving daemon descendants alive
- Location: src/agent/omo_daemon.rs:397
- Evidence: `let _ = child.start_kill();`
- Why it matters: The spawn command creates no owned process group/job, and cleanup addresses only the Child handle. If the configured daemon binary/wrapper forks a server or launches long-lived tool subprocesses, killing the wrapper/daemon does not kill those descendants. The watcher explicitly treats a forked survivor serving the port as external and returns, so that survivor is then outside shutdown ownership. Direct-child kill_on_drop does not solve descendant ownership.
- Suggested fix: Launch managed daemons in an owned process group/session on Unix (job object on Windows), terminate/reap that owned tree during shutdown, and distinguish owned descendants from genuinely external takeovers.

### [P1] Daemon setup performs synchronous filesystem operations on async workers
- Location: src/agent/omo_daemon.rs:180
- Evidence: `std::fs::OpenOptions::new()`
- Why it matters: ensure and the async watcher call daemon_command directly, which synchronously creates directories, opens the log, and clones its handle. Binary resolution also synchronously stats PATH/home candidates. A slow or unavailable home/network filesystem blocks the Tokio worker rather than yielding; ensure additionally retains the global spawn mutex through this work. Probe timeouts do not bound these operations.
- Suggested fix: Move filesystem-dependent binary/log preparation to spawn_blocking or async filesystem APIs before constructing/spawning the command, keeping serialization only around the necessary ownership transition.

### [P1] Daemon log setup failures silently discard both output streams
- Location: src/agent/omo_daemon.rs:195
- Evidence: `cmd.stdout(Stdio::null()).stderr(Stdio::null());`
- Why it matters: Directory creation errors are ignored, open failures are converted to None, and try_clone failures fall through here without any warning. With an unwritable HOME, exhausted file descriptors, or full filesystem, the daemon can repeatedly fail to start while all of its diagnostics disappear; the caller sees only the generic readiness failure.
- Suggested fix: Report the actual log-setup error and fall back to an observable sink (for example inherited stderr), rather than silently switching both streams to null.

### [P1] Connection retries synchronize outage traffic instead of backing off
- Location: src/agent/omo_backend.rs:89
- Evidence: `(tokio::time::Instant::now() + Duration::from_millis(500)).min(retry_limit);`
- Why it matters: Each concurrent turn retries immediately failing connections at the same fixed 500-ms cadence for up to 15 seconds, and setup can repeat the sequence once. There is no exponential delay, jitter, or shared reconnect limiter. A burst of N turns during a daemon outage generates approximately 2N attempts per second, all competing as the daemon recovers and potentially extending the outage under load. Authorization/other deterministic handshake failures are retried by the same loop.
- Suggested fix: Use capped exponential backoff with jitter within the existing deadline, avoid retrying permanent handshake failures, and share a reconnect gate for the same daemon endpoint.

## Strengths
- Normal turn execution correlates notifications by both thread and turn identity, validates the start ACK's turn ID, and distinguishes failed/interrupted/completed terminal states.
- Submission ownership is recorded before the first turn/start write. Setup-only retry avoids automatically duplicating a possibly accepted turn, and cancellation removes ownership only after confirmation and an identity comparison.
- Direct children use kill_on_drop; restarts have a sliding-window budget, replacement readiness is bounded, and an unready replacement is killed/reaped with errors logged.
- Persistence uses parameter-bound SQL and UTC timestamps. Activity previews truncate by Unicode characters rather than potentially panicking on a byte boundary.

## Notes
- This is a static, read-only review. All five targets, including their embedded tests, were read fully, and candidate surroundings were re-read. Structural scans covered panic/unwrap/expect sites, blocking filesystem calls, ignored Results, locks/awaits, collections, and TODO/FIXME/HACK markers. No production panic/unwrap site, SQL interpolation injection, or lock-order deadlock was proven in these files. Test unwraps were not reported as production panics.
- Findings describe explicit input/timing conditions, not a claim that the current installed daemon was observed producing them. In particular, pre-ACK notifications and unrelated approval/cleanup frames are protocol inputs the client mishandles; whether the current upstream implementation emits them was not verified within this target-only review.
- No tests/builds were run and no daemon was spawned, preserving the read-only boundary. Source inspection and citation verification were chosen over a new runtime harness because the latter would require additional artifacts/process side effects; runtime reproduction remains unverified.
- No finding asserts that fixed request IDs across separate WebSocket connections are inherently wrong, that initialized is necessarily a required notification, that JSON-RPC batches/binary payloads are required by this app-server, or that the approval result schema is incompatible. Those claims need upstream protocol evidence not established here. omo_protocol.rs and omo_activity.rs were reviewed but yielded no separate proven finding.
- Unknown turn outcomes deliberately remain fail-closed; blindly clearing ownership or retrying turn/start would risk duplicate execution. Reconnection/reconciliation for those outcomes is absent here, but this report does not mislabel that intentional safety property as permission to resubmit.
- The direct-child drop path was inspected with kill_on_drop and Arc ownership in mind. A spawn/shutdown race alone does not prove a permanently orphaned direct child: the watcher eventually drops its handle. The descendant-process finding is narrower and conditional on a wrapper/daemon actually forking children.
- Severity follows the requested rubric: demonstrated data loss, policy bypass, or unbounded retained/buffered memory is P0; edge-condition lifecycle, transport, and operational risks are P1. There are no P2-only findings.
