# Aggregate: agg-surface

## Coverage
- discord-adapter: 43 findings; read in full: yes.
- discord-commands: 20 findings; read in full: yes.
- discord-auth: 9 findings; read in full: yes.
- cron-scheduler: 28 findings; read in full: yes.
- cron-store-exec: 19 findings; read in full: yes.
- dashboard: 17 findings; read in full: yes.
- web-frontend: 16 findings; read in full: yes.

Reviewed 152 source findings. Carried forward 140: P0=5, P1=106, P2=29. Demoted 40, promoted 4, and dropped 12 source entries (including 2 deduplications).

Method: source-backed re-adjudication was chosen over mechanical relabeling because lane claims sometimes contradict caller behavior or the local trust model. Every carried-forward Location was checked with `sed -n '<LINE>p' /Users/indo/code/project/omon-gateway/<path>`; all resolved. Source context was reread for disputed severity, reachability, fallback, and policy claims. These are static findings, not claims of reproduced integration failures; no build, test, browser, or live Discord execution was performed.

Scope and assumptions: only the seven supplied reports and source files needed for verification were read. The mentioned tools-exec and multiplexer lanes were not supplied and were not read. Ordinary long-running compression concurrent with conversation writes is treated as normal operation, not a narrow race. Loopback-only dashboard access is distinguished from public network exposure; global paired-user approval delegation is not assumed to promise requester isolation. Browser-side loopback protections can mitigate the CSRF trigger, but the HTTP server itself does not reject it. No unrelated code or configuration was changed.

## Findings

### [P0] TOCTOU in `/compress` causes silent data loss of messages arriving during summarization  (from lane: discord-commands, originally P0)
- Location: src/discord/commands.rs:1092
- What: Compression deletes all session messages rather than only the snapshot it summarized.
- Trigger: A user or assistant appends messages while an ordinary /compress LLM summary is in flight, before the final DELETE.
- Impact: New unsummarized messages are permanently erased during a normal conversation operation.
- Fix: Capture the snapshot maximum sequence before summarization and delete only sequence <= that cutoff in the summary transaction, preserving later rows and correct summary ordering.

### [P0] Fan-out sends every destination to the job's stored session  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1674
- What: Fan-out delivery reuses the stored session instead of constructing each destination's session.
- Trigger: A normal job has a valid session for channel A and configured destinations A and B.
- Impact: Both dispatches go to A while B silently receives nothing and delivery records can claim the wrong destination.
- Fix: Build the dispatch session from each destination, including bot identity. Reuse a stored session only after validating its channel, thread, and bot against that destination, as the mirror helper already does.

### [P0] Updating a registered job silently retains its old session key  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:764
- What: Re-registering an existing job omits session_key from its conflict update.
- Trigger: An operator updates an existing job ID from conversation A to conversation B with a new CronJobSpec.session_key.
- Impact: The acknowledged update silently retains A and subsequent execution or delivery uses the wrong conversation.
- Fix: Include session_key = excluded.session_key in the conflict update.

### [P0] Partial updates in update_bot silently wipe unprovided fields to NULL  (from lane: dashboard, originally P0)
- Location: src/dashboard.rs:1927
- What: Partial bot updates bind omitted optional fields as SQL NULL instead of retaining stored values.
- Trigger: A normal client sends PUT /api/bots/{id} with only a new name for a bot that already has model, system_prompt, and enabled_toolsets.
- Impact: Existing bot configuration is silently erased by a successful update.
- Fix: Preserve existing values for omitted fields prior to executing the SQL update (e.g. `let model = payload.model.or(existing.model);`).

### [P0] State-changing HTTP mutation endpoints lack Origin/CSRF validation  (from lane: dashboard, originally P0)
- Location: src/dashboard.rs:229
- What: HTTP mutation requests ignore Origin even though WebSocket upgrades validate it.
- Trigger: A malicious page submits a simple form POST to http://127.0.0.1:9119/api/sessions/web-default/stop, or a known cron job's trigger endpoint, in a browser that permits loopback requests.
- Impact: The page can perform state changes without user authorization; JSON-only mutations and DELETE are not claimed as simple-form exploits.
- Fix: Reject mismatched Origin on state-changing HTTP requests and require a same-origin anti-CSRF header or token for mutations, including no-body POST routes.

### [P1] Media root checks authorize sibling directories  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:64
- What: String-prefix checks admit files in sibling directories outside the authorized temporary roots.
- Trigger: A MEDIA directive names a readable existing /tmp-private/report.txt outside the workspace and other deny rules.
- Impact: The adapter can upload a file outside its advertised roots on that filesystem layout.
- Fix: Use component-aware `Path::starts_with` against canonical authorized root paths for every alternative; do not use string prefixes.

### [P1] Continuous channel traffic grows the debounce buffer without a bound  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:605
- What: Trailing-edge debounce has no independent batch age or size bound.
- Trigger: Authorized messages keep arriving in one shared channel less than 600 ms apart for a sustained period.
- Impact: The batch never flushes and retains every event; exhaustion depends on traffic volume and duration.
- Fix: Impose a maximum batch age and size and flush on either limit, independently of the trailing-edge debounce timer.

### [P1] One previously claimed constituent discards fresh messages in the same batch  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:646
- What: An overlapping durable constituent claim causes the entire coalesced batch to be rejected.
- Trigger: A replayed or concurrently backfilled message A shares a debounce batch with fresh message B.
- Impact: B is discarded even though B was never delivered.
- Fix: Atomically claim/filter individual constituent deliveries before combining their content, and coalesce only the newly claimed events; do not treat an overlapping batch as wholly duplicate.

### [P1] REST guild history is misclassified as direct messages  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:1388
- What: Absent message.guild_id overrides authoritative guild channel type during history conversion.
- Trigger: REST history omits guild_id for a fetched guild message while channel metadata identifies a guild channel.
- Impact: An otherwise admitted guild user receives DM-style session and implicit-response routing; arbitrary user authorization bypass is not established.
- Fix: Populate the fetched message's guild_id from the authoritative channel metadata before conversion, and determine DM status from the resolved channel kind rather than allowing absent optional message metadata to override it.

### [P1] Live traffic and startup recovery use different cursor stores  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:784
- What: Live ingress writes legacy cursors while nonempty bot IDs recover from bot-scoped cursors.
- Trigger: The gateway restarts after live traffic in a previously used DM or unconfigured guild channel, or after more than 50 missed configured-channel messages.
- Impact: Recovery omits channels or starts too late and misses offline messages.
- Fix: Persist bot-scoped recovery state for live deliveries and discover channels from that same state. Advance the durable completion cursor after routing succeeds, not merely when an event arrives; migrate existing legacy rows explicitly.

### [P1] Send and edit dispatch discard the reasoning-filtered output  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:2734
- What: SendMessage and EditMessage render original content after computing a reasoning-filtered version.
- Trigger: An outbound send or edit contains <think>private details</think>Public answer, or a MEDIA directive inside that block.
- Impact: Supposedly hidden content is published and hidden media directives can execute on sends.
- Fix: Use filtered_content throughout both SendMessage and EditMessage rendering, including footer/title generation and MEDIA extraction.

### [P1] Failed final stream processing leaves unbounded retained stream entries  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:2644
- What: Terminal stream errors bypass removal of the allocated stream entry.
- Trigger: Fresh stream IDs repeatedly finalize with nonexistent MEDIA paths or failing HTTP operations after placeholder creation.
- Impact: Failed streams retain throttlers and related state indefinitely; realistic exhaustion rate is not demonstrated.
- Fix: Validate before allocating a placeholder and guarantee stream-map/typing cleanup on terminal error, retaining retry state only under an explicit bounded retry policy.

### [P1] Table-upload failure is converted into success on final-chunk retry  (from lane: discord-adapter, originally P0)
- Location: src/discord/adapter.rs:2667
- What: The sequence marker is committed before final table attachments finish sending.
- Trigger: Text succeeds, a final table upload fails, and the caller retries that same stream sequence.
- Impact: The retry reports success without delivering the missing table or completing cleanup.
- Fix: Record successful final completion only after all required deliveries finish. Track completed text and pending attachments separately so retrying attachments does not duplicate already delivered text.

### [P1] Synchronous filesystem validation runs on async executor workers  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:51
- What: Media validation performs synchronous filesystem operations inside async dispatch.
- Trigger: Canonicalization or existence checks hit a slow or stalled workspace mount.
- Impact: A Tokio worker is blocked and unrelated async work can be delayed.
- Fix: Move the complete filesystem validation operation into spawn_blocking, or use async filesystem APIs for the relevant checks.

### [P1] Media upload reopens an unbounded, untyped file source  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1855
- What: Media upload reads the entire validated pathname without a regular-file or byte limit and reopens it after validation.
- Trigger: A generated file exceeds the upload limit, a permitted path names a FIFO, or a local writer swaps the validated pathname before upload.
- Impact: Uploads can exhaust memory or stall; pathname replacement can also substitute an unauthorized file.
- Fix: Require a regular file and enforce the effective upload limit with a bounded read; open under a component-safe policy and validate/upload that same handle rather than reopening a mutable pathname.

### [P1] Routing can reorder adjacent debounce batches  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:617
- What: Detached debounce batches can reach routing out of arrival order.
- Trigger: Batch A waits for attachment hydration while a later batch B for the same session finishes hydration first.
- Impact: The conversation receives B before A.
- Fix: Route each session through a single ingress worker that owns hydration and dispatch ordering; do not let independent timer tasks race to enqueue turns.

### [P1] Stop can miss an already detached debounce batch  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1032
- What: Stop only cancels debounce batches still present in the shared map.
- Trigger: A timer detaches a batch and waits for ledger or attachment I/O before /stop executes.
- Impact: The detached old batch can start a new turn after stop returns.
- Fix: Associate a cancellation generation/token with each session and check it immediately before dispatch, or serialize stop and pending ingress work in the same session worker.

### [P1] Text stop detection occurs after context decoration and auto-thread creation  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1031
- What: Text stop detection occurs after content decoration and auto-thread side effects.
- Trigger: A user replies with /stop, enables topic/history decoration, or explicitly mentions the bot with /stop in an auto-thread channel.
- Impact: The command becomes a model turn or targets the newly created thread instead of stopping the intended session.
- Fix: Detect the normalized raw command body before reply/topic/history decoration or auto-thread side effects, then stop the existing target session directly.

### [P1] A routing error permanently removes the in-memory batch  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:649
- What: Routing failure is logged after the only buffered copy has been removed.
- Trigger: The incoming ledger operation fails during a transient database outage after debounce detachment.
- Impact: An admitted live message has neither a durable claim nor an in-memory retry.
- Fix: Durably stage ingress before removing it from memory, or retain/requeue the batch with bounded backoff and explicit terminal failure handling.

### [P1] Required thread mentions are bypassed by a free-response parent  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1455
- What: A free-response parent independently bypasses the configured thread mention requirement.
- Trigger: thread_require_mention is enabled and an authorized user posts an unmentioned followup under a free-response parent.
- Impact: The bot responds despite the thread mention policy.
- Fix: Enforce the thread mention requirement before evaluating implicit free-channel admission, or exclude threads from that bypass when the option is set.

### [P1] Auto-thread creation races between explicitly mentioned bots  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:987
- What: Independent bot handlers create the same starter thread before deduplicating their invocations.
- Trigger: One guild message explicitly mentions two configured bots with auto-thread creation enabled.
- Impact: The losing create request aborts one bot invocation rather than joining the existing thread.
- Fix: Resolve/reuse an existing starter thread and coordinate creation by starter message ID; claim/deduplicate the invocation before non-idempotent side effects.

### [P1] Placeholder HTTP send holds the global stream-map mutex  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2638
- What: Placeholder creation awaits network I/O while holding the global stream-map mutex.
- Trigger: One placeholder send is slow or Discord rate-limits it while other streams need the map.
- Impact: All bots and channels sharing the map suffer head-of-line blocking, not a proven permanent process deadlock.
- Fix: Reserve per-stream initialization state under the map lock, release the global lock, then perform network I/O behind a per-stream initialization guard.

### [P1] Successful finalization erases duplicate-sequence protection  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2679
- What: Removing a completed stream also removes its sequence replay protection.
- Trigger: The same final stream UUID and sequence is delivered again after successful cleanup.
- Impact: A second placeholder and duplicate response are sent.
- Fix: Keep a bounded completed-stream tombstone with the terminal sequence, or deduplicate final delivery durably before creating a new placeholder.

### [P1] Dead-target short circuit prevents typing shutdown  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3060
- What: The dead-target return precedes local typing-stop cleanup.
- Trigger: Typing starts, a later operation marks the target dead, and terminal cleanup sends Typing(false).
- Impact: The typing guard remains alive and can continue failed refresh requests.
- Fix: Always remove local typing state for active=false before checking target health or HTTP availability.

### [P1] Typing start can install a guard after typing stop completes  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3067
- What: Typing start installs its guard after an awaited broadcast without checking for a newer stop.
- Trigger: Typing(false) runs while the active branch awaits a slow or rate-limited broadcast.
- Impact: A stale start installs a guard after stop completed.
- Fix: Serialize typing state transitions or install a generation-tagged guard before network awaits and reject stale starts after a stop.

### [P1] Typing refresh throttling conflates bot identities and is not cleared on stop  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2001
- What: Typing refresh state uses channel identity alone and survives cancellation.
- Trigger: Two bots refresh in the same channel, or turns are cancelled before finalization across many channels.
- Impact: One bot suppresses another's indicator and abandoned timestamps accumulate.
- Fix: Key refresh state by bot identity and channel, and remove that identity's refresh state on stop/cancellation as well as successful finalization.

### [P1] Reactions to thread followups target the parent channel  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3085
- What: Reaction dispatch selects the session parent channel rather than the original thread message channel.
- Trigger: A normal followup is posted inside a thread whose session stores its parent in channel_id.
- Impact: Reactions fail with Unknown Message and a start reaction may not be cleared.
- Fix: Carry the original message channel with reaction actions; use the thread for genuine thread messages and the parent only for the auto-thread starter.

### [P1] Operation-specific permission errors poison the whole channel  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2338
- What: Operation-specific 403 errors are treated as permanent whole-channel failures.
- Trigger: The bot can send but lacks permission to edit or delete the particular target message.
- Impact: An unrelated permission failure disables subsequent channel delivery.
- Fix: Classify whole-target failure using operation context and Discord error codes; do not infer send unavailability from edit/delete permissions.

### [P1] Forum child-send errors mark the forum parent dead  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2799
- What: A failed forum continuation marks the forum parent rather than the failing post dead.
- Trigger: The new forum child is deleted or becomes inaccessible between continuation chunks.
- Impact: Future independent posts to the healthy forum are suppressed.
- Fix: Record the actual failing post_channel.id for continuation failures; leave the parent healthy unless a parent operation fails.

### [P1] Forum upload titles exceed Discord's name limit  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1812
- What: Forum upload titles concatenate an unbounded filename with a prefix.
- Trigger: A valid filename plus Voice Note or file prefix exceeds Discord's 100-character thread-name limit.
- Impact: An otherwise valid upload is rejected.
- Fix: Apply a shared Discord thread-name limiter to the prefixed filename before CreateForumPost.

### [P1] Approval mentions are appended outside the 2000-character budget  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3292
- What: Approval mentions are prepended after the command consumes the message budget.
- Trigger: A near-2000-character approval body receives at least one ordinary 18-digit user mention.
- Impact: Discord rejects the approval prompt and its buttons never arrive.
- Fix: Reserve space for the mention prefix before truncating the command, and bound/deduplicate the mention list itself.

### [P1] EditMessage does not handle Discord's content limit  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2945
- What: EditMessage sends content without an explicit overflow policy.
- Trigger: An edit contains 2001 ASCII characters.
- Impact: Discord rejects the edit instead of delivering a bounded update.
- Fix: Apply an explicit edit overflow policy: edit the first bounded chunk and send continuations, or reject oversized edits at the boundary with a clear error before transport.

### [P1] Dead-target persistence can run in the opposite order to in-memory mutations  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2159
- What: Dead-target mutations spawn unordered persistence operations and discard database results.
- Trigger: mark_dead is followed by clear but the spawned DELETE commits before the older INSERT, or either write fails.
- Impact: Restart can resurrect stale dead-target state without reporting the persistence failure.
- Fix: Serialize persistence per bot/channel or use generation-checked writes, propagate/log failures, and avoid unnecessary clears when no durable entry needs removal.

### [P1] Replay acknowledges success even when obligation state updates fail  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2488
- What: Replay reports delivered counts even when obligation-state persistence fails.
- Trigger: Discord accepts a replay and the following delivered-state write fails.
- Impact: Visible delivery and durable obligation state diverge, allowing duplicates or stranded work.
- Fix: Propagate or explicitly aggregate obligation-state update errors and distinguish claimed, delivered, and failed counts.

### [P1] Legacy cursor updates are a non-atomic check then write  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3347
- What: Legacy cursor advancement checks and overwrites in separate statements.
- Trigger: Handlers for snowflakes 200 and 300 both read 100, then 300 commits before 200.
- Impact: The supposedly monotonic cursor regresses.
- Fix: Enforce the numeric snowflake comparison in the atomic UPSERT update predicate, rather than relying on a preceding SELECT.

### [P1] Backfill advances its durability cursor past in-progress duplicates  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1235
- What: Backfill treats an in-progress duplicate claim as completed durable work.
- Trigger: Live ingress owns a message claim, backfill advances beyond it, and the live turn then fails or the process exits.
- Impact: Recovery can skip a message whose turn never completed.
- Fix: Distinguish delivered duplicates from in-progress claims and wait for their terminal outcome, or hold the cursor until durable completion is established.

### [P1] Transient role lookup failures silently skip recoverable history  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3675
- What: A failed role lookup is treated like successful empty-role metadata.
- Trigger: A REST message has no member metadata, admission relies on a role, and the role HTTP lookup temporarily fails.
- Impact: The authorized message is filtered and later cursor advancement can make the loss permanent.
- Fix: Keep lookup failure distinct from a successful empty-role result. If admission depends on unavailable roles, stop that channel's scan without advancing its cursor and report the lookup error.

### [P1] Backfill suppresses cursor persistence errors and advances anyway  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3733
- What: Backfill ignores cursor-write errors while advancing its local cursor.
- Trigger: A database write fails after a backfilled turn finishes.
- Impact: Recovery reports progress that is not durable and silently repeats work after restart.
- Fix: Propagate the write error or halt that channel, preserving the last durable cursor and reporting incomplete recovery.

### [P1] Filtered history never persists progress  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3754
- What: Conclusive history rejection advances only the scan-local cursor.
- Trigger: A channel accumulates a long tail of filtered bot, system, or unmentioned messages across reconnects.
- Impact: The same history is repeatedly fetched and checked, consuming API calls and recovery time.
- Fix: Persist progress for conclusively filtered messages too, while distinguishing transient metadata/admission failures that must hold the cursor.

### [P1] Backfilled mentions bypass live auto-thread routing  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3728
- What: Backfill dispatches without the live path's auto-thread target resolution.
- Trigger: auto_thread is enabled and a mention arrives while the gateway is offline.
- Impact: The recovered invocation replies in the parent rather than its intended starter thread.
- Fix: Share an idempotent target-resolution step between live and backfill routing, including existing starter-thread reuse, before choosing how to await the turn.

### [P1] Approval expiry removes its retry target before the HTTP edit succeeds  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:3198
- What: Approval expiry removes its tracked target before the remote edit succeeds.
- Trigger: Discord rejects or times out the expiry edit after the tuple is removed.
- Impact: Retry has no target and stale approval buttons remain visible.
- Fix: Keep the target until cleanup succeeds, or retain a bounded retry record; use idempotent lookup-and-edit followed by conditional removal.

### [P1] One bot's gateway traffic masks another bot's receive failure  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1947
- What: All bot watchdogs share one process-global receive timestamp.
- Trigger: Bot B stops receiving while healthy bot A continues normal gateway traffic.
- Impact: A masks B's silence and B's watchdog does not attempt recovery.
- Fix: Store the receive timestamp per client/bot (and shard if applicable), and pass that state to the matching watchdog.

### [P1] Receive watchdog tasks outlive the clients they monitor  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:1941
- What: A detached infinite receive watchdog is not tied to client termination.
- Trigger: An adapter stops or fails and is restarted in the same process.
- Impact: Old tasks retain shard managers and continue acting on stopped clients.
- Fix: Tie the watchdog to the client's lifetime with a cancellation token or retained task handle and shut it down when client.start returns.

### [P1] Oversized tables can cause excessive rasterization allocation  (from lane: discord-commands, originally P0)
- Location: src/discord/table_render/mod.rs:313
- What: Table rasterization derives an unchecked canvas size from row and column counts.
- Trigger: An agent emits an unusually large markdown table, such as 250 columns by 1000 rows, before outbound chunk limiting.
- Impact: Rasterization can demand tens of gigabytes; size-validation errors are handled, but allocation pressure can still exhaust memory.
- Fix: Enforce maximum limits on table rows (e.g., max 50 rows) and columns (e.g., max 10 columns), or cap total rendered width (e.g., 1920 px) and height (e.g., 4000 px). If a table exceeds these bounds, truncate rows/columns with an indicator or bypass PNG rendering and keep the raw text.

### [P1] Synchronous system font loading blocks async Tokio worker thread on every table render  (from lane: discord-commands, originally P1)
- Location: src/discord/table_render/mod.rs:304
- What: Each synchronous table render reloads system fonts on the async dispatch worker.
- Trigger: Concurrent responses contain tables or the font filesystem is slow.
- Impact: Filesystem traversal and rendering stall async workers and increase latency.
- Fix: Initialize the `usvg::fontdb::Database` once using a `LazyLock<Arc<usvg::fontdb::Database>>` or global cache, and share it across calls. Offload `svg_to_png` rendering to `tokio::task::spawn_blocking`.

### [P1] Multibyte retained chunks are over-truncated after the split cap  (from lane: discord-commands, originally P1)
- Location: src/discord/throttler.rs:335
- What: The retained final chunk is budgeted by UTF-8 bytes instead of Unicode characters.
- Trigger: The response exceeds MAX_SPLIT_MESSAGES and its final retained chunk contains multibyte text.
- Impact: The explicitly truncated response loses substantially more retained text than its character budget requires.
- Fix: Compare character counts instead of byte lengths: `if last.chars().count() > budget_chars` where `budget_chars = DISCORD_MESSAGE_LIMIT.saturating_sub(TRUNCATION_NOTICE.chars().count())`, and truncate using character boundary indices rather than byte offsets.

### [P1] `/skills action:search` response is sent without chunking and exceeds Discord 2,000-character limit  (from lane: discord-commands, originally P1)
- Location: src/discord/commands.rs:558
- What: Skill search sends the complete result list as one Discord message.
- Trigger: A search matches enough long skill names or descriptions to exceed 2000 characters.
- Impact: The slash-command response is rejected.
- Fix: Wrap the output of `search` in `chunk_slash_reply` and iterate over chunks with `ctx.say`, identical to the `list` handler.

### [P1] `/tools` response concatenates unbounded tool/endpoint lists and exceeds Discord 2,000-character limit  (from lane: discord-commands, originally P1)
- Location: src/discord/commands.rs:887
- What: The tools command sends all tool and endpoint descriptions in one message.
- Trigger: Configured tools and endpoints together exceed 2000 characters.
- Impact: The command fails at Discord's content limit.
- Fix: Chunk the formatted tools and endpoints text using `chunk_slash_reply` or truncate to 2,000 characters before sending.

### [P1] `/steer` embeds untruncated user guidance into ephemeral reply exceeding Discord 2,000-character limit  (from lane: discord-commands, originally P1)
- Location: src/discord/commands.rs:961
- What: Steering confirmation echoes the full guidance without reserving a content budget.
- Trigger: The slash option accepts guidance long enough that the prefixed confirmation exceeds 2000 characters.
- Impact: The confirmation fails even though guidance was queued.
- Fix: Use `preview_text(&text, 100)` (which is already implemented and used in `undo` and `retry`) to truncate the echoed text in the confirmation message.

### [P1] Discord thread name length validation missing in `/title` and `/thread` commands  (from lane: discord-commands, originally P1)
- Location: src/discord/commands.rs:1126
- What: Thread creation and title edits omit local validation of Discord's name bounds.
- Trigger: A user submits whitespace-only text or more than 100 characters.
- Impact: Discord rejects the operation rather than receiving a valid name or the user receiving a clear validation response.
- Fix: Validate that trimmed thread names are between 1 and 100 characters before calling `EditThread` or `CreateThread`. If invalid, return an informative ephemeral error message.

### [P1] Temporary `.part` download files leak indefinitely on cancelled futures  (from lane: discord-commands, originally P1)
- Location: src/discord/attachments.rs:375
- What: Partial attachment files are only removed on a returned download error.
- Trigger: The hydration future is cancelled after creating a .part file but before stream_to_file returns.
- Impact: Partial files survive cancellation and accumulate across repeated interrupted downloads.
- Fix: Implement a drop-guard (RAII temp-file wrapper) that removes the partial file when dropped unless marked committed, and scan/remove `.part` files in `AttachmentDownloader::new`.

### [P1] `decode_wav_pcm` parses audio data as 16-bit PCM without validating format tag or bit depth  (from lane: discord-commands, originally P1)
- Location: src/discord/attachments.rs:35
- What: WAV decoding interprets all sample data as 16-bit PCM without checking format or bit depth.
- Trigger: An admitted WAV uses 8-bit, 24-bit, float, or compressed sample encoding.
- Impact: The transcriber receives misdecoded audio rather than a supported decode or explicit rejection.
- Fix: Validate the WAV format tag and bit depth; decode supported PCM or reject unsupported WAV explicitly rather than treating it as Opus.

### [P1] `LiveEditThrottler` holds mutex lock across network I/O and issues redundant `start_typing` on final update  (from lane: discord-commands, originally P1)
- Location: src/discord/throttler.rs:200
- What: The throttler serializes network operations and starts typing even for a final update.
- Trigger: A final edit follows a typing stop, or slow Discord I/O overlaps another update on the same throttler.
- Impact: Typing can persist after completion and same-stream updates stall; per-stream serialization alone is not a deadlock.
- Fix: Do not start typing on a final update; keep necessary per-stream serialization, isolating slow operations only if concurrent update ordering is preserved.

### [P1] `/undo` and `/retry` mutate session messages without stopping active turns in multiplexer  (from lane: discord-commands, originally P2)
- Location: src/discord/commands.rs:974
- What: Undo and retry mutate persistent history without coordinating with an active session turn.
- Trigger: A user runs /undo or /retry while the actor is still generating and persisting a response.
- Impact: A late actor write can reintroduce stale response history after deletion.
- Fix: Serialize undo/retry with the session actor and wait for active-turn termination before deleting or replaying history.

### [P1] Unrecognized `mode` argument in `/yolo` unexpectedly toggles YOLO mode instead of returning validation error  (from lane: discord-commands, originally P2)
- Location: src/discord/commands.rs:1265
- What: An unrecognized yolo mode takes the toggle branch instead of rejecting the value.
- Trigger: An authorized operator enters /yolo mode:check or another invalid mode expecting no state change.
- Impact: Approval-bypass mode flips unintentionally; this is not an unprivileged authorization exploit.
- Fix: Match `None => !effective`, and return an error for unrecognized `Some(unknown)` strings: "Invalid mode. Use 'on' or 'off'".

### [P1] `is_voice_attachment` fallback omits `.wav` extension when Content-Type is missing  (from lane: discord-commands, originally P2)
- Location: src/discord/attachments.rs:90
- What: Voice classification omits the wav extension in its MIME-less fallback.
- Trigger: A file named recording.wav has no Content-Type, waveform flag, or voice-message filename marker.
- Impact: The WAV is not recognized for voice transcription.
- Fix: Add `|| lower.ends_with(".wav")` to the filename extension fallback check.

### [P1] Unauthorized DM notification records have no retention bound  (from lane: discord-auth, originally P0)
- Location: src/discord/pairing.rs:370
- What: Notification throttle rows have no expiry or deletion path in the reviewed pairing lifecycle.
- Trigger: Many distinct unauthorized Discord accounts send DMs over sustained operation.
- Impact: Persistent rows outlive codes; a realistic disk-exhaustion rate from unique accounts is not established.
- Fix: Prune notification rows whose timestamps are older than the rate-limit window during issuance/cleanup, and remove a user's notification row after successful pairing. Keep recent throttle reservations intact.

### [P1] Remembered session approvals grow without eviction  (from lane: discord-auth, originally P0)
- Location: src/discord/approval.rs:468
- What: Remembered session grants have no automatic lifetime or size bound.
- Trigger: Long-lived use accumulates distinct approved commands and sessions without clear_session.
- Impact: The resident cache grows; neither ordinary approval volume nor an exhaustion horizon was demonstrated for P0.
- Fix: Bound the remembered session cache by entry count/bytes and expire it with the session lifecycle. Evicting a remembered grant is safe because the next matching operation can prompt again; use a digest for exact command/reason identity rather than retaining entire raw strings.

### [P1] A resolved approval can resurrect grants after session clear  (from lane: discord-auth, originally P1)
- Location: src/discord/approval.rs:388
- What: A resolved decision can publish a grant after clear_session removes the session's state.
- Trigger: Resolution removes the pending entry, clear_session completes, then the waiting requester resumes and caches Session or Always.
- Impact: Cleared authorization can be restored by a stale request.
- Fix: Associate requests and grants with a session generation. Increment it on clear and atomically reject stale generations when publishing a grant; carry that generation to the execution boundary so a clear between approval return and execution cannot use the old authorization.

### [P1] Always approval reports success even when persistence fails  (from lane: discord-auth, originally P1)
- Location: src/discord/approval.rs:489
- What: Always approval publishes the in-memory grant before a fallible database write and returns no persistence result.
- Trigger: The allowlist INSERT fails because the database is read-only, full, or unavailable.
- Impact: The requester sees permanent approval while durable and live permissions disagree.
- Fix: Return a persistence Result, persist before publishing the global cache entry, and propagate failure to the requester rather than reporting a permanent grant. If persistence is unavailable, explicitly downgrade to Once only with a visible non-persistent outcome.

### [P1] Pairing can consume the code without publishing authorization to the cache  (from lane: discord-auth, originally P1)
- Location: src/discord/pairing.rs:513
- What: Pairing commits code consumption before cancellable cache publication.
- Trigger: The approval future is cancelled after commit while awaiting the paired-cache write lock.
- Impact: The code is gone but the live cache continues denying the durably paired user.
- Fix: Make committed cache publication cancellation-safe, for example by giving a separately owned operation responsibility for both commit and publication, or add database reconciliation on cache misses so durable paired state cannot remain invisible. Preserve the current rule against granting access before a successful commit.

### [P1] Pairing expiry is evaluated against stale request-start time  (from lane: discord-auth, originally P1)
- Location: src/discord/pairing.rs:462
- What: Pairing validates expiry using the pre-queue request time and accepts equality.
- Trigger: A code is submitted just before expiration and waits on the pool until after its deadline.
- Impact: An expired code can be consumed, with inconsistent behavior at the exact cleanup boundary.
- Fix: Reject now >= expires_at and sample the production clock at the claim/check boundary after waiting for the connection. Make the consuming DELETE conditional on the same expiry check; preserve deterministic injected-clock support for tests without reusing a pre-queue production timestamp.

### [P1] Heartbeat wait can accept an approval after its deadline  (from lane: discord-auth, originally P1)
- Location: src/discord/approval.rs:99
- What: The approval waiter prioritizes a ready decision over an already elapsed timeout.
- Trigger: Executor contention delays polling across the deadline and a late click resolves the still-pending entry.
- Impact: An approval can be accepted after its configured deadline.
- Fix: Store the effective deadline with the pending request and check it when resolving under the pending lock. In the heartbeat waiter, reject an elapsed deadline before consuming a newly received approval, or include the decision's resolution timestamp if pre-deadline decisions must remain valid after delayed polling.

### [P1] Registration, resumption, and failure recovery ignore the job timezone  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:756
- What: Registration, resume, and failure recovery compute deadlines without the payload timezone.
- Trigger: A daily 09:00 Asia/Seoul job is registered, resumed, or advanced after failure.
- Impact: It runs at 09:00 UTC and can switch cadence after success; this is scheduling failure, not by itself P0-class loss or bypass.
- Fix: Extract and validate the payload timezone at every scheduling boundary and pass it to next_run_tz, including failure recovery.

### [P1] Large valid intervals panic when added to the current date  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1927
- What: A representable interval duration can overflow the resulting DateTime on addition.
- Trigger: An operator registers interval:10000000000000s.
- Impact: Registration panics on exceptional configuration rather than returning an error; routine-input process failure is not established.
- Fix: Use checked_add_signed and turn an out-of-range result into OmonError::Config; apply the same checked-boundary policy to clock-derived lease additions.

### [P1] User-provided repeat counts overflow on completion  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:267
- What: Repeat completion increments an externally supplied u64 without an overflow check.
- Trigger: A stored or registered payload sets repeat.completed to 18446744073709551615 without a positive limit.
- Impact: Completion panics in checked builds or wraps accounting; this is an exceptional imported/configured value.
- Fix: Reject an unincrementable count at registration and use checked_add in completion so persisted or imported payloads cannot bypass the boundary validation.

### [P1] Predecessor fallback crosses profile boundaries and treats IDs as LIKE patterns  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:147
- What: Predecessor fallback ignores profile isolation and interprets job-ID wildcards in LIKE.
- Trigger: The requested profile has no nonempty exact output and another profile has a newer assistant session ending with that ID.
- Impact: The agent consumes the wrong profile's context; attacker control over isolated profiles is not demonstrated.
- Fix: Restrict fallback to explicitly enumerated exact session keys for the requested profile/job. If any LIKE matching remains necessary, escape wildcard characters and preserve the profile boundary.

### [P1] Manual completion exhausts repeat limits without disabling the job  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1245
- What: Manual completion consumes repeat accounting without disabling an exhausted schedule.
- Trigger: A repeat.times=1 job is manually triggered before its scheduled occurrence.
- Impact: It stays enabled and due but subsequent claims never execute it.
- Fix: Either keep manual runs outside the scheduled repeat budget, or atomically disable/clear next_run_at when manual completion reaches that budget. Make the behavior consistent with the manual-run contract.

### [P1] Pausing an executing recurring job discards its completion accounting  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1282
- What: Recurring completion's enabled predicate discards accounting after a concurrent pause.
- Trigger: An operator pauses a recurring job during its execution, then resumes it after completion.
- Impact: The finished run is not counted and repeat-limited jobs can execute extra times.
- Fix: Persist completion accounting independently of whether scheduling is enabled; condition only next_run_at advancement on enabled, and retain revision checks to avoid overwriting an edited payload.

### [P1] Successful execution is rolled back when a cron expression has no next occurrence  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1278
- What: Failure to compute a next occurrence rolls back already-successful run completion.
- Trigger: A finite-year cron reaches its last occurrence, or a previously accepted timezone is rejected on completion.
- Impact: Already-executed side effects remain paired with a running durable lease and can be reclaimed or repeated.
- Fix: Treat schedule exhaustion as a successful terminal job state and commit the run while disabling the schedule. Validate timezone before execution, and preserve/retry completion separately from re-executing side effects when persistence fails.

### [P1] Execution tasks and their retained payloads have no concurrency bound  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:1137
- What: Due and manually triggered jobs spawn without a shared concurrency bound.
- Trigger: Many distinct jobs become due while their commands or backends remain slow or never finish.
- Impact: Active tasks, payload snapshots, and child processes accumulate under sustained load.
- Fix: Acquire a bounded execution permit before claiming/spawning and page due-job selection. Add an execution deadline/cancellation policy so permanently stuck jobs cannot occupy capacity indefinitely.

### [P1] Shell output is accumulated without a size limit  (from lane: cron-scheduler, originally P0)
- Location: src/cron/scheduler.rs:446
- What: The scheduler's direct shell-command path collects all output and has no execution deadline.
- Trigger: An operator schedules a command such as yes or an accidentally endless verbose script.
- Impact: The running child can exhaust gateway memory; this is an accepted pathological command, not routine input or an unprivileged exploit.
- Fix: Read child pipes incrementally with explicit byte limits and a bounded execution lifetime, reporting truncation or terminating the process when the limit is exceeded.

### [P1] Reclamation can invalidate a lease that was refreshed after selection  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1037
- What: Lease reclamation does not compare the lease value observed during selection.
- Trigger: The owner refreshes a selected expired lease before the reclaimer updates it.
- Impact: A live execution is invalidated and a replacement can overlap it.
- Fix: Compare-and-swap the observed lease_expires_at, and any owner identity used for the decision, in the reclaim UPDATE; abandon reclamation if the row changed.

### [P1] Live-owner reclamation has no execution fence and is vulnerable to clock jumps  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:53
- What: Lease loss does not fence the old executor from delivery or other side effects.
- Trigger: A large forward clock step or long owner stall causes live-owner reclamation before the old executor resumes.
- Impact: Original and replacement runs can both act externally.
- Fix: Detect lease loss from rows_affected, cancel lease-lost executions, and fence delivery/external effects using the active claim token. Do not treat elapsed wall-clock time alone as permission for a live owner and a replacement to both act.

### [P1] Cutover receipt checking races with claim insertion  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1012
- What: The pending-cutover receipt check is separate from claim insertion.
- Trigger: A receipt is created after the guard query but before a still-eligible job's INSERT SELECT.
- Impact: The scheduler can start a run inside the intended cutover exclusion window.
- Fix: Check pending receipt absence as part of the same atomic claim operation or serialize receipt creation and claiming under the same database write transaction.

### [P1] Claim validation and the executed job snapshot are not atomic  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1118
- What: Eligibility checks, claim insertion, and fetching the executed job snapshot are separate operations.
- Trigger: Re-registration or manual completion interleaves between those awaits, or the post-claim job fetch fails.
- Impact: A claim can execute a different revision, exceed a stale repeat budget, or retain an unserviced running lease.
- Fix: Validate, claim, and capture the exact job revision in one transaction; include revision/limit checks in the claim and return that snapshot. Roll back the claim if its snapshot cannot be obtained.

### [P1] Completion can overwrite a newer schedule revision  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1264
- What: Completion guards omit schedule revision changes that leave expression and payload unchanged.
- Trigger: Pause/resume or same-spec re-registration changes a deadline while an older run completes.
- Impact: Stale completion overwrites the new deadline or disables the newly scheduled job.
- Fix: Add a schedule revision captured by CronClaim and compare it in schedule mutations, or at minimum include the observed next_run_at and a revision that changes on every reschedule. Keep run-result accounting independent of schedule ownership.

### [P1] Backward wall-clock steps can replay an already executed cron occurrence  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1194
- What: Recurring advancement uses completion wall time without the claimed occurrence as a lower bound.
- Trigger: A job due at 12:01 completes after the wall clock steps back to 12:00:30.
- Impact: The next deadline becomes 12:01 again and the same nominal occurrence can run twice.
- Fix: Persist the claimed nominal firing time and compute advancement after at least max(completion_now, claimed_scheduled_time). Use an occurrence key if duplicate exclusion must survive clock corrections and reclamation.

### [P1] Delete and reschedule leave old executions running  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:875
- What: Deleting or rescheduling a job does not cancel its already claimed execution.
- Trigger: An operator deletes or replaces a job while its command or backend is still running.
- Impact: Old work can deliver after the mutation using its retained snapshot.
- Fix: Track executions by job/run revision and cancel them when the applicable mutation invalidates that revision. Revalidate the active revision before delivery and terminate owned subprocesses on cancellation.

### [P1] Resume writes a deadline calculated from a stale expression  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:859
- What: Resume updates by ID after calculating a deadline from a separately fetched expression.
- Trigger: Concurrent re-registration changes the expression, or deletion removes the row between resume's read and write.
- Impact: Resume installs a stale deadline or reports success after a zero-row update.
- Fix: Compare the observed job revision in the UPDATE and use rows_affected to detect a conflicting edit or deletion; retry against a fresh snapshot only when appropriate.

### [P1] A stopped scheduler cannot be started again  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:675
- What: Scheduler restart retains the previously signalled shutdown value.
- Trigger: The same scheduler instance follows start, shutdown, then start.
- Impact: The replacement polling loop exits immediately and no jobs run.
- Fix: Reset or replace the shutdown channel under the scheduler lifecycle lock before spawning a new polling task, with a defined start-versus-shutdown ordering.

### [P1] Shutdown holds the task mutex across an unbounded join  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:700
- What: Shutdown holds the task mutex while joining and later awaits executions without a deadline.
- Trigger: A due-job sweep blocks or an executing command never completes when shutdown starts.
- Impact: Lifecycle queries block and shutdown may never return; ordinary steady-state process deadlock is not demonstrated.
- Fix: Take the handle in a separate scoped statement, drop the guard before awaiting it, and implement bounded cancellation/join behavior for both the polling loop and executions.

### [P1] Cancelling wait_idle permanently detaches tracked executions  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:565
- What: wait_idle transfers tracked handles into a cancellable caller future.
- Trigger: That waiting future is cancelled while executions are still active.
- Impact: Tasks detach, active counts underreport them, and concurrent shutdown cannot join them.
- Fix: Keep execution ownership in shared state until completion; implement waiting as an observation of tracked completion rather than transferring every handle into a cancellable caller future.

### [P1] Delivery obligation IDs collide within a millisecond  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1696
- What: Delivery obligation identity contains only job ID and a wall-clock millisecond.
- Trigger: Two deliveries for one job share a millisecond, or the clock repeats a value.
- Impact: Distinct attempts or destinations share a ledger identity and cannot be accounted for independently.
- Fix: Give each delivery obligation a collision-resistant ID, preferably derived from run ID plus destination identity when retry idempotency is required; do not use wall time as uniqueness.

### [P1] Successful-run output persistence errors are silently discarded  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1310
- What: Successful-run output insertion discards its database result.
- Trigger: The cron_outputs statement fails while the surrounding completion transaction can still commit.
- Impact: Success can be recorded without its predecessor output or an explicit output-persistence error.
- Fix: Propagate the insertion error or record an explicit output-persistence failure with a defined recovery path; do not silently commit an incomplete successful-run record.

### [P1] Incident read/write failures silently defeat acknowledgement handling  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1603
- What: Incident reads and writes convert database failures into ordinary acknowledgement state.
- Trigger: Incident SELECT, INSERT, UPDATE, or DELETE fails during notification handling.
- Impact: Alerts can ignore acknowledgement or later be suppressed by stale acknowledgement state.
- Fix: Handle and log incident persistence/read errors explicitly, preserving a deliberate acknowledgement policy when state is unavailable rather than silently treating database errors as normal state.

### [P1] Full shell commands expose embedded credentials in normal logs  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:436
- What: Normal INFO logging includes the raw shell command.
- Trigger: A scheduled command embeds a token, password argument, or credential-bearing URL.
- Impact: Credentials are copied into the operational log without redaction.
- Fix: Log job ID and execution metadata, not the raw command; only log an explicitly redacted command representation if operationally necessary.

### [P1] Validation failure during synchronization causes silent permanent deletion of existing cron jobs  (from lane: cron-store-exec, originally P0)
- Location: src/cron/store.rs:942
- What: A job failing validation is omitted from the live set and deleted as an orphan.
- Trigger: An existing mirrored job's source definition temporarily gains an invalid schedule or timezone.
- Impact: Its previous valid database row and runtime accounting are deleted rather than retained for repair.
- Fix: Distinguish between parse/validation errors and deliberate job deletions: only delete jobs if all jobs in the source file parsed and validated cleanly, or retain unvalidated existing IDs in `live` with an error flag so they are not pruned.

### [P1] Premature commit of monitor hash drops state change alerts on downstream execution or delivery failure  (from lane: cron-store-exec, originally P0)
- Location: src/cron/executor.rs:164
- What: Monitor state is committed before the downstream agent and delivery succeed.
- Trigger: A changed monitor snapshot is stored and the agent run or delivery then fails.
- Impact: The next identical snapshot is treated as unchanged and its alert is lost.
- Fix: Record the new hash in session metadata or return it alongside the output, and only commit the new hash to `cron_monitor_states` in `CronScheduler::complete_success` after execution and delivery have succeeded.

### [P1] Subprocess leak on ack command timeout due to missing process group and process tree termination  (from lane: cron-store-exec, originally P1)
- Location: src/cron/ack.rs:38
- What: Ack timeout kills only the shell rather than its process group.
- Trigger: An ack shell launches a long-lived child or pipeline and exceeds its timeout.
- Impact: Descendants can survive and keep consuming resources or holding locks.
- Fix: On Unix, configure `command.process_group(0)` before spawning, extract `child.id()`, and send `SIGKILL` to `-(pid as i32)` in the timeout branch.

### [P1] Ack command executed without working directory or augmented environment PATH  (from lane: cron-store-exec, originally P1)
- Location: src/cron/ack.rs:31
- What: Ack execution inherits the daemon directory and PATH rather than the job's execution context.
- Trigger: An ack uses ./scripts/checkpoint.sh or a binary present only in an augmented job PATH.
- Impact: A previously delivered run's acknowledgement fails or targets the wrong working tree.
- Fix: Pass the resolved job working directory and execution PATH to ack execution rather than relying on daemon inheritance.

### [P1] Cross-profile collision on `cron_monitor_states` primary key  (from lane: cron-store-exec, originally P1)
- Location: src/cron/executor.rs:145
- What: Monitor hashes are keyed by the unscoped Hermes job ID.
- Trigger: Two configured profiles each run a monitor job named weather.
- Impact: One profile overwrites the other's state, causing false changes or missed alerts.
- Fix: Bind `&job.id` (the globally unique scoped identifier) instead of `&hermes.id`.

### [P1] Monitor output discarded and omitted from agent prompt  (from lane: cron-store-exec, originally P1)
- Location: src/cron/executor.rs:252
- What: Changed monitor content is hashed but never appended to the agent prompt.
- Trigger: A monitor_script or monitor_url changes and the task relies on that returned content to explain the change.
- Impact: The agent is invoked without the observed state it needs to report.
- Fix: Append `monitor_output` under a `\n\n[Monitor output]\n` section in `prompt` alongside `script_output`.

### [P1] TOCTOU race condition and unbounded key growth in `set_cron_notepad`  (from lane: cron-store-exec, originally P1)
- Location: src/cron/store.rs:1022
- What: Notepad capacity checks are non-atomic and count values but not keys.
- Trigger: Concurrent writes pass the same 64 KiB check, or many distinct long keys store empty values.
- Impact: Storage and assembled prompt size exceed the intended cap.
- Fix: Enforce aggregate key-plus-value bytes and entry count under one serialized database write transaction.

### [P1] Non-transactional store synchronization risks inconsistent state and scheduler races  (from lane: cron-store-exec, originally P1)
- Location: src/cron/store.rs:974
- What: Store synchronization commits each row mutation independently.
- Trigger: A database failure interrupts a multi-job import after earlier upserts succeeded.
- Impact: The database remains partially synchronized; scheduler observation of partial state is conditional on concurrent claim activity.
- Fix: Wrap the entire synchronization loop per store inside a single database transaction (`let mut tx = self.pool.begin().await?`) and commit only upon full success.

### [P1] Blocking filesystem I/O inside asynchronous execution paths  (from lane: cron-store-exec, originally P1)
- Location: src/cron/executor.rs:808
- What: Cron execution performs synchronous file reads and traversal on async workers.
- Trigger: Scripts or skills reside on a slow or stalled filesystem.
- Impact: Worker starvation delays execution and time-sensitive scheduling tasks.
- Fix: Use `tokio::fs` or offload synchronous file traversal to `tokio::task::spawn_blocking`.

### [P1] Unbounded agent backend execution duration lacking timeout enforcement  (from lane: cron-store-exec, originally P1)
- Location: src/cron/executor.rs:336
- What: The agent executor awaits the backend without its own execution deadline.
- Trigger: A backend dependency never returns or a model/tool loop fails to terminate.
- Impact: The cron task and its lease heartbeat remain active indefinitely.
- Fix: Wrap `self.backend.run` in `tokio::time::timeout(agent_timeout, ...)`.

### [P1] Reachable panic on large retention days in `prune_terminal_cron_runs`  (from lane: cron-store-exec, originally P1)
- Location: src/cron/store.rs:127
- What: Retention cutoff construction accepts values beyond chrono's representable range.
- Trigger: An operator sets CRON_RUNS_RETENTION_DAYS to 999999999999.
- Impact: Startup pruning can panic instead of reporting invalid configuration.
- Fix: Use `chrono::TimeDelta::try_days(retention_days).ok_or_else(...)` and clamp or return a configuration error.

### [P1] `failure_deliver = []` inverts user intent by falling back to origin delivery  (from lane: cron-store-exec, originally P2)
- Location: src/cron/store.rs:478
- What: An explicitly empty failure destination list falls through to origin delivery.
- Trigger: A job specifies failure_deliver: [] and later fails.
- Impact: It sends a failure notification despite the empty destination override.
- Fix: Check `if let Some(list) = &self.failure_deliver { if list.is_empty() { return Ok(Vec::new()); } }`.

### [P1] Loopback dashboard has no per-user authentication  (from lane: dashboard, originally P0)
- Location: src/dashboard.rs:698
- What: Loopback administrative endpoints have no per-user or session authentication.
- Trigger: On a shared host, a different unprivileged OS account connects to the loopback port and invokes execution or approval APIs.
- Impact: That account can act through the gateway's privileges; arbitrary remote-network access is blocked by loopback validation.
- Fix: Add authentication middleware requiring a secret bearer token or cookie on all `/api/` endpoints except health check routes (`/api/health`, `/api/readiness`).

### [P1] Unexpected server exit leaves standalone runtime waiting for shutdown  (from lane: dashboard, originally P0)
- Location: src/dashboard_runtime.rs:206
- What: Standalone runtime waits only for cancellation before observing the server task.
- Trigger: After successful listener binding, the server task panics or unexpectedly exits without cancelling the token.
- Impact: The process remains alive without a serving task until explicit shutdown; bind failure itself already propagates.
- Fix: Select between cancellation and server-task completion; unexpected completion must cancel the runtime and enter scheduler/resource cleanup.

### [P1] Race condition in create_cron_job allows initially paused jobs to trigger execution  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:1449
- What: Creating a paused cron job registers it enabled before a separate pause operation.
- Trigger: An immediately due job with enabled:false is claimed between registration and pause.
- Impact: Work executes even though creation requested a paused state.
- Fix: Pass the initial `enabled` state into the scheduler registration methods and insert the record with `enabled = 0` when `input.enabled == Some(false)`.

### [P1] OutboundAction::ExpireApproval is never forwarded to session WebSocket subscribers  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:518
- What: Approval-expiry actions have no session routing identity for WebSocket delivery.
- Trigger: An approval shown to a connected session is resolved or expires.
- Impact: The session socket never receives its retirement event and the client can retain stale approval UI.
- Fix: Track the mapping from `request_id` to `SessionKey` in `WebDashboardDispatcher` and inspect this mapping to provide the target session for `ExpireApproval` events.

### [P1] Synchronous fs2 disk space metrics block async worker threads in status and readiness handlers  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:583
- What: Status and readiness handlers synchronously sample filesystem capacity.
- Trigger: The workspace is on a slow or unresponsive NFS, SMB, or FUSE mount.
- Impact: statvfs blocks an async worker and delays unrelated requests.
- Fix: Offload disk sampling to `tokio::task::spawn_blocking` or cache sampled values in a background polling loop.

### [P1] Unbounded SQL queries and response payloads in cron, bot, and allowlist listings  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:1418
- What: Cron, bot, and approval-allowlist listings materialize all rows without pagination.
- Trigger: A deployment accumulates many entries and clients repeatedly request the listings.
- Impact: Large queries and responses increase latency and memory use under sustained load.
- Fix: Apply standard pagination (`PageQuery`) with maximum limit clamping to all listing endpoints.

### [P1] Half-open WebSockets have no application liveness deadline  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:1209
- What: WebSocket loops have no application-level liveness deadline for silent peers.
- Trigger: A connection becomes half-open through a network blackhole while no traffic exposes the disconnect.
- Impact: The connection can retain resources indefinitely; absence of explicit builder limits does not mean Axum has unlimited default frame sizes.
- Fix: Add bounded ping/pong or idle-deadline handling and set explicit conservative frame/message limits rather than assuming framework defaults are unlimited.

### [P1] Stop clears UI state without confirmed backend cancellation  (from lane: web-frontend, originally P0)
- Location: web/src/App.tsx:624
- What: Stop uses a new socket and clears local sending state without awaiting a stop acknowledgement.
- Trigger: The new WebSocket handshake fails or the server cannot process the stop before the client retires the socket.
- Impact: The UI appears stopped while the backend turn may continue; the 500 ms timer starts after onopen, not before connection establishment.
- Fix: Use POST /api/sessions/{id}/stop and await its response before reporting cancellation, with visible failure handling.

### [P1] Live logs stay disconnected after a transient socket closure  (from lane: web-frontend, originally P0)
- Location: web/src/App.tsx:1434
- What: Live logs do not reconnect after the socket closes.
- Trigger: The gateway restarts or a transient connection failure closes the telemetry socket.
- Impact: Streaming stays disconnected until remount; the existing badge reports Disconnected and no JavaScript crash is established.
- Fix: Provide bounded-backoff reconnection with effect cleanup, or a manual Reconnect action; preserve the existing disconnected indicator.

### [P1] Missing cleanup and race condition on session message fetching in ChatPlaygroundPage  (from lane: web-frontend, originally P1)
- Location: web/src/App.tsx:524
- What: Session history requests publish results without checking that the session is still selected.
- Trigger: The user switches A then B and A's slower response arrives after B's.
- Impact: A's messages overwrite the visible history for selected session B.
- Fix: Use an active/cancelled flag inside `useEffect` or an `AbortController` to abort stale in-flight fetches when `currentSessionId` changes.

### [P1] Missing debounce or abort on search input creates request storms and race conditions  (from lane: web-frontend, originally P1)
- Location: web/src/App.tsx:1155
- What: Search requests can update results after their query has been superseded.
- Trigger: A user types successive queries and an earlier request completes after a later one.
- Impact: The table shows stale results; request frequency also increases with every keystroke.
- Fix: Abort or generation-check obsolete search requests before publishing results; optionally debounce to reduce request volume.

### [P1] Unconditional scrollIntoView on every message chunk disrupts user reading during streaming  (from lane: web-frontend, originally P1)
- Location: web/src/App.tsx:531
- What: Each streamed update scrolls to the bottom without respecting the user's current position.
- Trigger: The user scrolls up to read older content during an active streaming response.
- Impact: Incoming chunks repeatedly disrupt reading by forcing the viewport back down.
- Fix: Check whether the user is scrolled near the bottom (e.g., `scrollTop + clientHeight >= scrollHeight - 50`) before invoking `scrollIntoView`.

### [P2] Public route_message lacks gateway metadata parity  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:751
- What: The public routing helper builds a different metadata context from live gateway ingress.
- Trigger: A caller uses route_message for a role-authorized user or a parent-authorized owned thread; no production caller was established in this aggregation.
- Impact: The public seam cannot preserve gateway admission and ownership semantics.
- Fix: Populate roles from the message and resolve/pass the required parent and ownership metadata, or share one context-building entry point with handle_event.

### [P2] Injected message transport silently drops reply references  (from lane: discord-adapter, originally P1)
- Location: src/discord/adapter.rs:2756
- What: The injected SendMessage transport omits parsed reply references.
- Trigger: A test or alternate integration configures with_message_transport and sends a reply; a production use was not established.
- Impact: The transport seam differs from the normal Discord path and cannot test reply parity.
- Fix: Enumerate chunks and call send_message_with_reference with should_chunk_reference for the first chunk, matching the normal transport path.

### [P2] The per-user thread-session setting is ignored  (from lane: discord-adapter, originally P2)
- Location: src/discord/adapter.rs:1548
- What: The thread_sessions_per_user setting is propagated but ignored in session construction.
- Trigger: An operator changes the setting expecting separate thread-user sessions.
- Impact: Configuration advertises a distinction absent from routing; this also covers the duplicate PoiseData-field finding.
- Fix: Either honor the setting consistently in thread session construction, including auto-thread starters, or explicitly remove/deprecate it if shared guild sessions are intentional.

### [P2] Debouncer coalescing test depends on a real-time scheduling window  (from lane: discord-adapter, originally P2)
- Location: src/discord/adapter.rs:4148
- What: The debounce test relies on real scheduler windows for batch membership and absence assertions.
- Trigger: The test task is descheduled between enqueues or before the negative timeout expires.
- Impact: Correct code can fail the test nondeterministically.
- Fix: Use paused Tokio time, enqueue all inputs before explicitly advancing the virtual clock, then synchronize completion of all timer work and inspect the collected events without a real-time absence window.

### [P2] Pagination convergence lacks a proved fixed point  (from lane: discord-commands, originally P1)
- Location: src/discord/throttler.rs:307
- What: Pagination convergence is limited to two header-aware passes without an asserted fixed point.
- Trigger: No concrete production input demonstrating nonconvergence was supplied or reproduced; the reported 3-to-4 example does not change denominator width.
- Impact: This is a boundary-test and algorithm-hardening gap, not a confirmed mislabeled production response.
- Fix: Add deterministic denominator-digit-boundary cases and either establish convergence or budget a conservative maximum header width.

### [P2] XML-illegal table controls force the raw-text fallback  (from lane: discord-commands, originally P1)
- Location: src/discord/table_render/mod.rs:291
- What: SVG text escaping does not remove XML-illegal control characters.
- Trigger: A markdown table cell contains a NUL or terminal escape control character.
- Impact: PNG parsing can fail, but the caller logs the error and preserves raw table text; no delivery loss is established.
- Fix: Filter XML-illegal characters before SVG generation while retaining the existing raw-text fallback.

### [P2] `scan_fences` in `throttler.rs` mishandles 4+ backtick fences and fails to track tilde (`~~~`) code blocks  (from lane: discord-commands, originally P2)
- Location: src/discord/throttler.rs:431
- What: Markdown fence tracking only recognizes a three-backtick prefix.
- Trigger: A split response contains a tilde fence or a longer backtick fence.
- Impact: Chunk formatting may not preserve the original fence form.
- Fix: Count fence markers and match opening/closing fence lengths dynamically, supporting both `` ` `` and `~`.

### [P2] Requester isolation is absent from the global approval policy  (from lane: discord-auth, originally P0)
- Location: src/discord/approval.rs:559
- What: Approval resolution receives no actor context and relies on the caller's global paired-or-allowlisted policy.
- Trigger: Paired user B can see A's approval and click it, but the reviewed sources do not establish that paired users are meant to lack global approval authority.
- Impact: Requester isolation is not enforced; this is a delegation-policy hardening gap rather than a proven bypass of the current operator policy.
- Fix: Make the global approval-operator policy explicit; if paired users are not operators, pass actor/context to resolution and check that policy before consuming the pending entry.

### [P2] Heartbeat tests still depend on scheduler timing  (from lane: discord-auth, originally P2)
- Location: src/discord/approval.rs:1051
- What: Heartbeat tests assert real-time ordering and an empty receive buffer after resolution.
- Trigger: The test task is delayed long enough for another pre-resolution heartbeat or the short approval deadline.
- Impact: Tests can fail without any post-resolution heartbeat defect.
- Fix: Use Tokio's controlled clock to advance to explicit heartbeat events, resolve before deliberately advancing to the deadline, and verify producer termination separately from already-buffered events. Bound event waits; drain/count pre-resolution events rather than requiring the buffer to be empty.

### [P2] Execution authority predicate needs an explicit ownership contract  (from lane: cron-scheduler, originally P1)
- Location: src/cron/scheduler.rs:1096
- What: Claim predicates admit mirror and unknown authorities rather than positively naming execution ownership.
- Trigger: Duplicate execution additionally requires a separate active Hermes executor, which was not verified in this scope.
- Impact: The authority contract needs clarification or explicit fencing; mirror execution alone is not proven wrong.
- Fix: Define which authorities this scheduler owns, then enforce that explicit allowlist atomically in claims and document any external exclusion protocol.

### [P2] In-file timezone tests never exercise a DST gap or fold  (from lane: cron-scheduler, originally P2)
- Location: src/cron/scheduler.rs:2062
- What: Timezone tests check seasonal offsets but not DST gaps and folds.
- Trigger: Scheduling logic is changed around nonexistent or repeated local firing times.
- Impact: The current tests do not establish a transition policy or catch its regression.
- Fix: Add deterministic next-fire cases at the exact spring and fall transition instants, including repeated advancement through the fold, and explicitly assert the chosen skip/duplicate policy without real-time sleeps.

### [P2] Gateway lifecycle guard regexes bypassed by multiline commands and line continuations  (from lane: cron-store-exec, originally P0)
- Location: src/cron/guard.rs:14
- What: The lifecycle regex misses shell line continuations between command action and target.
- Trigger: An already command-authorized operator supplies systemctl restart followed by a backslash-newline and omon-gateway.
- Impact: A best-effort lifecycle safeguard is bypassed, not a demonstrated new execution privilege.
- Fix: Replace `[^\n]*` with `(?:\s|\\\n|[^\s])*` or match over any whitespace, or parse command argv tokens rather than relying on single-line string regexes.

### [P2] Modern macOS launchctl subcommands (`bootout`, `kill`) bypass gateway lifecycle guard  (from lane: cron-store-exec, originally P0)
- Location: src/cron/guard.rs:10
- What: The launchctl lifecycle pattern omits modern bootout and kill forms.
- Trigger: An operator with launchctl authority schedules bootout or kill against the gateway service.
- Impact: The guard misses a privileged lifecycle operation; it is not an unprivileged authorization boundary.
- Fix: Include `bootout`, `kill`, `bootstrap`, and `reboot` in the launchctl subcommands alternation list.

### [P2] Import-time script-body diagnostics depend on late-injected metadata  (from lane: cron-store-exec, originally P0)
- Location: src/cron/store.rs:342
- What: Import-time script-body validation depends on metadata injected only after validation.
- Trigger: A normal imported script job lacks _omon_hermes_home in its source extra fields.
- Impact: Import misses early diagnostics, but run_cron_script checks the body again before execution.
- Fix: Pass the store home explicitly into import validation to obtain early script-body diagnostics; retain execution-time validation.

### [P2] Script timeout scope is per attempt rather than explicitly total  (from lane: cron-store-exec, originally P2)
- Location: src/cron/executor.rs:409
- What: Script timeout is per retry attempt rather than an explicitly shared deadline.
- Trigger: No normal loader-failure path consuming three full timeouts was demonstrated; timeout itself returns without retry.
- Impact: The total-versus-per-attempt budget contract is unclear, not a proven three-times timeout defect.
- Fix: Document whether timeout is per attempt; if a total budget is intended, use a shared deadline and deterministic retry-budget tests.

### [P2] Native cron execution omits gateway lifecycle check on assembled prompt  (from lane: cron-store-exec, originally P2)
- Location: src/cron/executor.rs:477
- What: The native assembled prompt is injection-scanned without a separate lifecycle scan.
- Trigger: A command-authorized operator creates a native prompt requesting gateway lifecycle changes.
- Impact: This is inconsistent best-effort policy coverage, not demonstrated direct command execution or privilege escalation.
- Fix: Add `check_gateway_lifecycle(&task)?;` before dispatching the inbound message.

### [P2] ApiError leaks internal database error details and schema structure to callers  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:772
- What: SQL errors are returned verbatim in API error messages.
- Trigger: A request encounters a database error; no separate exploit enabled by schema details was demonstrated.
- Impact: The local administrative API discloses internal diagnostics rather than a stable sanitized error.
- Fix: Log detailed error messages internally via `tracing::error!` and return a generic error message (e.g. `"internal database error"`) in the client JSON response.

### [P2] Cron API exposes internal lease bookkeeping  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:1572
- What: Cron run responses serialize internal claim tokens and process IDs.
- Trigger: A dashboard client lists cron runs; no API accepting these values as credentials was found in the reviewed router.
- Impact: Internal lease bookkeeping is unnecessarily coupled to the public response, not a proven lease-takeover capability.
- Fix: Omit claim_token from API serialization and retain owner_pid only if a documented diagnostics consumer needs it.

### [P2] Standalone tool roots default to the whole home directory  (from lane: dashboard, originally P1)
- Location: src/dashboard_runtime.rs:228
- What: Standalone tool roots default to the entire home directory rather than only the workspace.
- Trigger: An operator starts standalone mode without OMON_TOOL_ROOTS.
- Impact: The local administrative agent has broad filesystem scope; the independent remote or unprivileged exploit is not established by this default alone.
- Fix: Restrict default tool roots to `workspace_root` rather than the user's entire `$HOME` directory unless explicitly overridden.

### [P2] Static-file serving lacks canonical containment hardening  (from lane: dashboard, originally P1)
- Location: src/dashboard.rs:2201
- What: Static-file validation checks lexical components but neither canonical containment nor hidden-file policy.
- Trigger: An operator places secrets or an escaping symlink inside web_root; no such shipped artifact was established.
- Impact: The static server can expose those deliberately present artifacts, making this deployment hardening rather than a routine bypass.
- Fix: Canonicalize the served file and enforce containment in canonical web_root; define an explicit hidden-file policy for production assets.

### [P2] Dead --insecure CLI and environment option is ignored during host validation  (from lane: dashboard, originally P2)
- Location: src/dashboard.rs:100
- What: The insecure option is ignored by unconditional loopback validation.
- Trigger: An operator passes the documented insecure option with a non-loopback host.
- Impact: The option cannot perform its advertised function; removing it is safer than silently widening exposure.
- Fix: Remove or deprecate the ignored insecure option and its contradictory documentation rather than enabling public administration implicitly.

### [P2] serve_static returns HTTP 200 with HTML for /api requests without trailing slash  (from lane: dashboard, originally P2)
- Location: src/dashboard.rs:2175
- What: The SPA fallback excludes /api/ paths but not the exact /api path.
- Trigger: A client requests GET /api.
- Impact: The response is HTML with a success status instead of the expected API-not-found response.
- Fix: Check `if uri.path() == "/api" || uri.path().starts_with("/api/")`.

### [P2] Markdown code props bypass type checking through any  (from lane: web-frontend, originally P1)
- Location: web/src/App.tsx:817
- What: The markdown code component uses any for props rather than the renderer's supported type.
- Trigger: A future plugin or dependency changes the component prop contract.
- Impact: Type checking cannot catch mismatches; any casting does not itself unescape HTML or demonstrate DOM injection.
- Fix: Use the react-markdown component prop types and remove any without inventing an HTML-unescaping workaround.

### [P2] Pending approvals badge in header lacks accessible role and keyboard activation  (from lane: web-frontend, originally P2)
- Location: web/src/App.tsx:247
- What: The approvals navigation badge is a clickable span without keyboard semantics.
- Trigger: A keyboard-only or assistive-technology user tries to activate the badge.
- Impact: This shortcut is not keyboard-accessible.
- Fix: Wrap the badge in a `<button>` or add `role="button"`, `tabIndex={0}`, and `onKeyDown={(e) => e.key === 'Enter' && selectPage('settings')}`.

### [P2] Modal dialog backdrop lacks escape key listener and focus trapping  (from lane: web-frontend, originally P2)
- Location: web/src/components/ui/dialog.tsx:10
- What: The custom modal lacks focus trapping, Escape handling, and modal accessibility semantics.
- Trigger: A keyboard user opens the dialog and tabs or presses Escape.
- Impact: Focus can leave the dialog and expected dismissal does not work.
- Fix: Add a `keydown` event listener for `Escape` and use focus-trap or standard Radix dialog primitives.

### [P2] Dual codebase divergence between web/src and web/src/lib/api  (from lane: web-frontend, originally P2)
- Location: web/src/pages/ChatPage.tsx:29
- What: Legacy page and API source coexist with the active App implementation.
- Trigger: A maintainer edits a residual page or client rather than the active surface.
- Impact: Parallel implementations invite contract drift; this citation alone does not prove a runtime authentication mismatch.
- Fix: Identify the active entry-point import graph and remove or clearly quarantine redundant legacy pages and clients.

### [P2] Window alert and confirm dialogs block browser UI thread  (from lane: web-frontend, originally P2)
- Location: web/src/pages/BotsPage.tsx:42
- What: Bot-page errors and confirmations use blocking browser dialogs.
- Trigger: An API failure opens alert or a destructive operation invokes confirm.
- Impact: The interaction blocks the tab instead of using the application's nonblocking UI.
- Fix: Replace `alert()` and `confirm()` with a toast notification hook or the existing `ConfirmDialog` component.

### [P2] Form inputs lack accessibility labels  (from lane: web-frontend, originally P2)
- Location: web/src/App.tsx:1104
- What: Cron form labels are not associated with their input elements.
- Trigger: A screen reader focuses the Job ID input or another similarly unlinked field.
- Impact: The visible label may not be announced as the input's accessible name.
- Fix: Add `htmlFor="job-id"` on `<label>` and matching `id="job-id"` on `<Input>`.

### [P2] Missing empty state UI on capabilities lists  (from lane: web-frontend, originally P2)
- Location: web/src/App.tsx:1233
- What: Capabilities cards render no explanatory empty state.
- Trigger: The tools or skills endpoint returns an empty items array.
- Impact: Users see blank lists without guidance about unavailable capabilities.
- Fix: Add fallback empty state notices when `tools.length === 0` or `skills.length === 0`.

## Demoted

| Finding (source lane) | Original | Re-graded | Why |
|---|---|---|---|
| Media root checks authorize sibling directories (discord-adapter) | P0 | P1 | Requires an existing sibling-root file and a MEDIA path reaching it; a routine unprivileged exploit was not established. |
| Continuous channel traffic grows the debounce buffer without a bound (discord-adapter) | P0 | P1 | Sustained sub-debounce traffic is required; no realistic production exhaustion horizon was shown. |
| One previously claimed constituent discards fresh messages in the same batch (discord-adapter) | P0 | P1 | Requires duplicate/live-backfill overlap inside one debounce window. |
| REST guild history is misclassified as direct messages (discord-adapter) | P0 | P1 | Guild metadata omission produces routing defects, but user admission and scan restrictions remain; an attacker-triggerable authorization bypass was not proved. |
| Live traffic and startup recovery use different cursor stores (discord-adapter) | P0 | P1 | Loss is conditional on restart/offline recovery and missing bot-scoped cursor history. |
| Send and edit dispatch discard the reasoning-filtered output (discord-adapter) | P0 | P1 | Requires reasoning-bearing outbound content; neither routine confidential content nor an independent authorization bypass was shown. |
| Failed final stream processing leaves unbounded retained stream entries (discord-adapter) | P0 | P1 | Needs repeated terminal errors or dependency failures; resource-exhaustion rate is not established. |
| Table-upload failure is converted into success on final-chunk retry (discord-adapter) | P0 | P1 | Requires attachment delivery failure followed by same-sequence retry. |
| Public route_message does not supply roles, ownership, or parent metadata (discord-adapter) | P1 | P2 | Public-helper parity issue without a demonstrated production caller. |
| Injected message transport silently drops reply references (discord-adapter) | P1 | P2 | Injected transport parity gap without demonstrated production use. |
| Unbounded table dimensions in PNG rasterization allow multi-gigabyte memory allocations and process abort (discord-commands) | P0 | P1 | Requires an unusually large table; allocation-size errors already have a raw-text fallback, and routine-input OOM was not established. |
| Incomplete two-pass convergence in `chunk_markdown_paginated` causes mislabeled `(i/N)` headers (discord-commands) | P1 | P2 | No concrete nonconvergent input; changing 3 to 4 does not increase header denominator width. |
| `html_escape` retains XML 1.0 illegal control characters causing `usvg::Tree::from_str` parser failure (discord-commands) | P1 | P2 | SVG parsing failure is handled by retaining raw table text; this is rendering hardening. |
| Approval resolution does not bind the clicker to the requesting session (discord-auth) | P0 | P2 | Current caller grants global paired/allowlisted approval authority; requester-isolation policy is not established. |
| Unauthorized DM notification records have no retention bound (discord-auth) | P0 | P1 | Growth requires sustained distinct accounts and no realistic disk exhaustion horizon was demonstrated. |
| Remembered session approvals grow without eviction (discord-auth) | P0 | P1 | Growth requires accumulating approvals over long use; no realistic memory exhaustion horizon was demonstrated. |
| Registration, resumption, and failure recovery ignore the job timezone (cron-scheduler) | P0 | P1 | Incorrect non-UTC scheduling is a real functional defect but does not itself meet a P0 impact class. |
| Large valid intervals panic when added to the current date (cron-scheduler) | P0 | P1 | Exceptional operator configuration, not routinely received input; process-wide failure is not shown. |
| User-provided repeat counts overflow on completion (cron-scheduler) | P0 | P1 | Requires a maximal imported/configured counter, not normal completion volume. |
| Predecessor fallback crosses profile boundaries and treats IDs as LIKE patterns (cron-scheduler) | P0 | P1 | Requires missing exact output and a matching other-profile fallback; no unprivileged isolation boundary was established. |
| Manual completion exhausts repeat limits without disabling the job (cron-scheduler) | P0 | P1 | Requires the manual-trigger and finite-repeat combination, leaving a stuck individual job. |
| Pausing an executing recurring job discards its completion accounting (cron-scheduler) | P0 | P1 | Requires pause during execution and subsequent resume, an explicit concurrency edge. |
| Successful execution is rolled back when a cron expression has no next occurrence (cron-scheduler) | P0 | P1 | Requires schedule exhaustion, invalid deferred timezone, or persistence failure. |
| Execution tasks and their retained payloads have no concurrency bound (cron-scheduler) | P0 | P1 | Requires sustained accumulation of distinct unfinished jobs. |
| Shell output is accumulated without a size limit (cron-scheduler) | P0 | P1 | Pathological or faulty operator-authorized shell output is not a routine-input or unprivileged exploit. |
| Claim eligibility is not restricted to Omon-owned authority (cron-scheduler) | P1 | P2 | Independent Hermes ownership and exclusion policy were not verified; retain only authority-contract hardening. |
| Validation failure during synchronization causes silent permanent deletion of existing cron jobs (cron-store-exec) | P0 | P1 | Destructive behavior requires an invalid replacement source definition. |
| Premature commit of monitor hash drops state change alerts on downstream execution or delivery failure (cron-store-exec) | P0 | P1 | Alert loss requires downstream execution or delivery failure. |
| Gateway lifecycle guard regexes bypassed by multiline commands and line continuations (cron-store-exec) | P0 | P2 | Already command-authorized operator bypasses a best-effort regex, not an authorization boundary. |
| Modern macOS launchctl subcommands (`bootout`, `kill`) bypass gateway lifecycle guard (cron-store-exec) | P0 | P2 | Requires existing launchctl authority over the gateway service. |
| Script body lifecycle check in store validation is dead code and bypassed due to missing `_omon_hermes_home` (cron-store-exec) | P0 | P2 | Execution-time script-body validation remains present, so import omission is diagnostic hardening. |
| Dashboard lacks authentication on high-privilege execution and administrative endpoints (dashboard) | P0 | P1 | Loopback-only service; cross-user abuse requires a shared-host trust condition, not unrestricted remote access. |
| Premature server termination in run_standalone causes indefinite hang and zombie process (dashboard) | P0 | P1 | Only unexpected post-bind server termination triggers the wait; bind errors already propagate. |
| ApiError leaks internal database error details and schema structure to callers (dashboard) | P1 | P2 | No demonstrated exploit from diagnostic disclosure on the local administrative surface. |
| list_cron_runs exposes internal lease claim_token and owner process IDs (dashboard) | P1 | P2 | No reviewed API accepts the exposed token to take over a lease; internal identifiers alone are not credentials here. |
| Tool root defaults authorize the entire home directory to unauthenticated callers (dashboard) | P1 | P2 | Broad local tool policy is hardening; no independent remote exploit follows from the default alone. |
| safe_relative_path permits access to dotfiles and does not resolve symlinks outside web root (dashboard) | P1 | P2 | Requires an operator-supplied secret or escaping symlink in web_root, neither demonstrated. |
| Stop turn creates an unauthenticated orphaned WebSocket connection with hardcoded 500ms closure (web-frontend) | P0 | P1 | Connection or processing failure is required; the original authentication and pre-open timer claims were incorrect. |
| Live logs WebSocket lacks reconnect logic and crashes on unhandled socket errors (web-frontend) | P0 | P1 | Requires disconnect/restart; no crash and an explicit Disconnected badge already exist. |
| ReactMarkdown code block component unescapes HTML props via loose any casting (web-frontend) | P1 | P2 | any weakens type checking but neither unescapes text nor proves injection. |

Promotions: discord-commands findings 15, 16, and 19 and cron-store-exec finding 16 were raised from P2 to P1 because they have concrete failure inputs or active-turn timing, rather than only maintenance concerns.

## Dropped

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
- **Telemetry poll error state retains stale status values** (web-frontend, originally P2; web/src/App.tsx:296): The containing App already renders an Offline label and destructive status indicator when statusError is set (App.tsx:212-217). Retaining a last-known snapshot is not the claimed absence of offline feedback.
- **Unbounded message list rendering without virtualization** (web-frontend, originally P2; web/src/App.tsx:735): The active api.sessionMessages call uses the paginated server endpoint rather than fetching thousands of rows. No unbounded message-growth path was established from messages.map; virtualization would be optional hardening for a different loading contract.

No finding was dropped for a missing or shifted source citation: all 152 original primary citations resolved. Two dropped entries were deduplicated; the other ten were wrong or lacked the claimed failure contract/reachability.

## Cross-cutting patterns

- **Progress published before durable success:** `src/discord/approval.rs:489` logs persistence failure after granting Always in memory; `src/cron/executor.rs:164` stores a monitor hash before execution/delivery; `src/discord/adapter.rs:3198` removes an expiry retry target before the remote edit succeeds. Publish or retire state only at the actual success boundary.
- **Check-then-act without revision or generation fencing:** `src/cron/scheduler.rs:1037` reclaims without comparing the observed lease; `src/cron/scheduler.rs:859` resumes from a separately read expression; `src/dashboard.rs:1449` pauses only after enabled registration; `src/discord/approval.rs:388` can republish a grant after clear. Use atomic revision checks or lifecycle generations.
- **Identity lost at fallback and reuse boundaries:** `src/discord/adapter.rs:1388` lets absent guild metadata select DM semantics; `src/cron/scheduler.rs:1674` reuses a session across different delivery destinations; `src/cron/executor.rs:145` uses an unscoped monitor ID. Derive identity from authoritative context and preserve each routing dimension.
- **Accumulation before a bound is applied:** `src/discord/adapter.rs:605` retains an indefinitely extended batch; `src/cron/scheduler.rs:446` captures complete shell output; `src/discord/table_render/mod.rs:313` allocates from table dimensions before outbound message limits. Bound bytes, age, and concurrency at the producing boundary, not after collection.
- **Background lifetimes lack terminal supervision:** `src/discord/adapter.rs:1941` detaches the client watchdog; `src/dashboard_runtime.rs:206` waits without supervising server exit; `web/src/App.tsx:1434` does not recover the telemetry socket after closure. Tie tasks/connections to explicit owners and terminal-state handling.
