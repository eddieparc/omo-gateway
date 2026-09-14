# Lane: discord-adapter
## Scope
- `src/discord/adapter.rs` - 4710 LOC, read in full, including its inline tests. Findings are restricted to this file.

## Findings
### [P0] Media root checks authorize sibling directories
- Location: src/discord/adapter.rs:64
- Evidence: `|| canonical_str.starts_with("/tmp")`
- Why it matters: A readable canonical file such as `/tmp-private/secret.txt` outside both the actual temporary directory and the workspace passes this string-prefix check. The `/private/tmp` and `/var/tmp` alternatives have the same component-boundary problem. A MEDIA directive can consequently upload files outside the advertised authorized roots.
- Suggested fix: Use component-aware `Path::starts_with` against canonical authorized root paths for every alternative; do not use string prefixes.

### [P0] Continuous channel traffic grows the debounce buffer without a bound
- Location: src/discord/adapter.rs:605
- Evidence: `batch.events.push(event);`
- Why it matters: Every arrival replaces the batch token, and a timer flushes only if its token remains current. An authorized busy guild channel with arrivals less than 600 ms apart never flushes, while all message bodies and attachments accumulate in its Vec. There is no maximum age, byte count, or event count. Guild participants share the session, making this reachable through aggregate channel traffic as well as a bot sending continuously.
- Suggested fix: Impose a maximum batch age and size and flush on either limit, independently of the trailing-edge debounce timer.

### [P0] One previously claimed constituent discards fresh messages in the same batch
- Location: src/discord/adapter.rs:646
- Evidence: `route_claimed_event_with_constituents(&data, coalesced, &constituent_ids)`
- Why it matters: The adapter merges all buffered messages before consulting durable deduplication. The called ledger operation returns false if any supplied constituent was already recorded. If an already-delivered message A is replayed alongside a new message B inside the debounce window, the entire merged event is rejected and B is removed from the buffer without being routed. This also occurs when startup backfill claims A while the live batch contains A and B.
- Suggested fix: Atomically claim/filter individual constituent deliveries before combining their content, and coalesce only the newly claimed events; do not treat an overlapping batch as wholly duplicate.

### [P0] REST guild history is misclassified as direct messages
- Location: src/discord/adapter.rs:1388
- Evidence: `let is_dm = message.guild_id.is_none() || channel_type == Some(ChannelType::Private);`
- Why it matters: REST channel-history Message objects can omit guild_id. Backfill explicitly retrieves the guild channel and its guild_id, but leaves msg unchanged and passes it to this converter. Even with channel_type Text/PublicThread, a missing message.guild_id selects DM semantics: guild/channel admission and primary-bot/mention routing use the DM path, and the event gets a guild-less, per-user session. Thus ordinary guild history can be answered as if it were private conversation despite successful guild metadata lookup.
- Suggested fix: Populate the fetched message's guild_id from the authoritative channel metadata before conversion, and determine DM status from the resolved channel kind rather than allowing absent optional message metadata to override it.

### [P0] Live traffic and startup recovery use different cursor stores
- Location: src/discord/adapter.rs:784
- Evidence: `let _ = update_channel_cursor(`
- Why it matters: Live ingress writes only discord_channel_cursors. Backfill discovers previously seen channels from discord_bot_cursors for the nonempty bot ID, and get_bot_channel_cursor only consults legacy cursors for an empty bot ID. A previously used DM or unrestricted guild channel not explicitly configured for scanning therefore disappears from recovery entirely. Configured channels without bot cursors start with the latest 50 messages, losing older offline messages instead of starting after the last live message.
- Suggested fix: Persist bot-scoped recovery state for live deliveries and discover channels from that same state. Advance the durable completion cursor after routing succeeds, not merely when an event arrives; migrate existing legacy rows explicitly.

### [P0] Send and edit dispatch discard the reasoning-filtered output
- Location: src/discord/adapter.rs:2734
- Evidence: `content.clone()`
- Why it matters: SendMessage computes filtered_content only for the silence check, then feeds original content into the footer, MEDIA extraction, table rendering, and Discord chunks. EditMessage likewise submits original content. For `<think>private details</think>Public answer`, the supposedly scrubbed reasoning is published. MEDIA directives inside reasoning are also executed on the send path, although the stream path correctly processes filtered_content.
- Suggested fix: Use filtered_content throughout both SendMessage and EditMessage rendering, including footer/title generation and MEDIA extraction.

### [P0] Failed final stream processing leaves unbounded retained stream entries
- Location: src/discord/adapter.rs:2644
- Evidence: `streams.insert(key.clone(), active.clone());`
- Why it matters: A stream entry and placeholder are created before validating media paths, uploading files, editing content, and sending tables. Every subsequent `?` can return without removing the entry; removal occurs only on explicit silence or fully successful finalization. Repeated final turns containing a nonexistent MEDIA path, or repeated transport failures with fresh stream UUIDs, retain a new ActiveDiscordStream indefinitely. There is no expiry or cancellation cleanup for this map.
- Suggested fix: Validate before allocating a placeholder and guarantee stream-map/typing cleanup on terminal error, retaining retry state only under an explicit bounded retry policy.

### [P0] Table-upload failure is converted into success on final-chunk retry
- Location: src/discord/adapter.rs:2667
- Evidence: `*last_sequence = Some(chunk.sequence);`
- Why it matters: The sequence is committed before dispatch_rendered_tables. If table attachment delivery fails, the stream remains in the map but a retry of the same final sequence returns Ok immediately at the sequence guard. The missing table is never retried, and cleanup is also bypassed. This is silent partial-delivery loss, not just a leaked placeholder.
- Suggested fix: Record successful final completion only after all required deliveries finish. Track completed text and pending attachments separately so retrying attachments does not duplicate already delivered text.

### [P1] Synchronous filesystem validation runs on async executor workers
- Location: src/discord/adapter.rs:51
- Evidence: `let canonical = path.canonicalize().map_err(|e| {`
- Why it matters: validate_media_path performs synchronous exists/canonicalize/current_dir operations and is called directly inside async send/stream dispatch. Slow or network-mounted workspace paths block Tokio workers instead of yielding, delaying unrelated Discord ingress, typing, and outbound requests.
- Suggested fix: Move the complete filesystem validation operation into spawn_blocking, or use async filesystem APIs for the relevant checks.

### [P1] Media validation and upload reopen a mutable pathname
- Location: src/discord/adapter.rs:1855
- Evidence: `let bytes = tokio::fs::read(path).await.map_err(|error| {`
- Why it matters: Validation returns a canonical PathBuf, not an opened file. Before this later async open, another process able to write the permitted temporary/workspace directory can replace the validated file or one of its parents with a symlink. The upload then reads a different, potentially unauthorized target without repeating the root check against the opened object.
- Suggested fix: Validate and open the file with a no-symlink/component-safe policy, then upload from that same handle or captured bytes instead of reopening its path.

### [P1] Uploads read the entire file without enforcing upload size or file type
- Location: src/discord/adapter.rs:1855
- Evidence: `let bytes = tokio::fs::read(path).await.map_err(|error| {`
- Why it matters: A large generated file is allocated in full before any Discord request; voice fallback additionally clones the bytes. Files exceeding Discord's attachment limit waste memory and fail only after the read. The validator also accepts existing non-regular files, so a permitted FIFO/device-like source can block completion or keep producing bytes without a useful upload bound.
- Suggested fix: Require a regular file, enforce the effective upload-size limit, and perform a bounded read that still detects growth after metadata inspection.

### [P1] Routing can reorder adjacent debounce batches
- Location: src/discord/adapter.rs:617
- Evidence: `lock.remove(&session).map(|b| b.events)`
- Why it matters: Removing a batch releases the session's only local coordination before ledger I/O and attachment hydration. A later batch for the same session can flush and reach multiplexer.route while the earlier batch is still downloading an attachment. The multiplexer then observes B before A even though A arrived first. Live handler metadata/context HTTP awaits and concurrently spawned backfill provide additional paths to the same ordering inversion.
- Suggested fix: Route each session through a single ingress worker that owns hydration and dispatch ordering; do not let independent timer tasks race to enqueue turns.

### [P1] Stop can miss an already detached debounce batch
- Location: src/discord/adapter.rs:1032
- Evidence: `global_debouncer().cancel(&event.session).await;`
- Why it matters: Once a timer removes its batch from the map, cancel cannot see it. If /stop arrives while that detached batch is waiting for the ledger or attachment download, stop runs before the batch reaches the multiplexer; the old batch then starts a new turn after stop has returned. Stopping the currently running actor does not cancel this in-flight ingress work.
- Suggested fix: Associate a cancellation generation/token with each session and check it immediately before dispatch, or serialize stop and pending ingress work in the same session worker.

### [P1] Text stop detection occurs after context decoration and auto-thread creation
- Location: src/discord/adapter.rs:1031
- Evidence: `if event.content.trim().eq_ignore_ascii_case("/stop") {`
- Why it matters: A reply containing /stop has already been prefixed with reply context. A guild /stop with topic/history context enabled has likewise been prefixed, so exact equality fails and it becomes an ordinary model turn. An explicit `@bot /stop` can also create a new thread before this check and stop that new session instead of the existing conversation.
- Suggested fix: Detect the normalized raw command body before reply/topic/history decoration or auto-thread side effects, then stop the existing target session directly.

### [P1] A routing error permanently removes the in-memory batch
- Location: src/discord/adapter.rs:649
- Evidence: `tracing::error!(session = %session, %error, "failed to route debounced Discord event");`
- Why it matters: The timer removes its batch before routing and handles every routing error with only this log. In particular, a database error before the incoming ledger record leaves neither a durable claim nor a retryable buffer entry. No error reaches handle_event and no bounded retry/requeue occurs, so an admitted live message can vanish during a transient database outage.
- Suggested fix: Durably stage ingress before removing it from memory, or retain/requeue the batch with bounded backoff and explicit terminal failure handling.

### [P1] Required thread mentions are bypassed by a free-response parent
- Location: src/discord/adapter.rs:1455
- Evidence: `let is_implicit_response_channel = is_dm || is_active_thread || is_free_channel;`
- Why it matters: is_free_channel includes the parent channel. thread_require_mention only disables is_active_thread, leaving this independent free-channel branch true. A thread under a free-response parent therefore still accepts unmentioned followups with thread_require_mention enabled, subject only to owner/primary identity selection.
- Suggested fix: Enforce the thread mention requirement before evaluating implicit free-channel admission, or exclude threads from that bypass when the option is set.

### [P1] Public route_message does not supply roles, ownership, or parent metadata
- Location: src/discord/adapter.rs:751
- Evidence: `user_roles: &[],`
- Why it matters: Unlike the gateway path, this public routing entry point unconditionally supplies empty roles/owners and no parent ID. A guild message carrying an authorized role in message.member is denied; owner-specific thread followups can route to the primary bot instead of their owner; parent-authorized threads fail admission or receive a different session key. Passing the same Message and channel type to the two entry points does not preserve routing semantics.
- Suggested fix: Populate roles from the message and resolve/pass the required parent and ownership metadata, or share one context-building entry point with handle_event.

### [P1] Auto-thread creation races between explicitly mentioned bots
- Location: src/discord/adapter.rs:987
- Evidence: `.create_thread_from_message(&ctx.http, new_message.id, builder)`
- Why it matters: A guild message explicitly mentioning two configured bots is admitted independently for both, and the delivery IDs deliberately support that case. Both handlers try to create a thread from the same starter message before deduplication. Discord permits only one such thread; the loser enters the error branch and aborts its invocation instead of joining the already-created thread. A replay of a starter event can hit the same problem before its delivery is checked.
- Suggested fix: Resolve/reuse an existing starter thread and coordinate creation by starter message ID; claim/deduplicate the invocation before non-idempotent side effects.

### [P1] Placeholder HTTP send holds the global stream-map mutex
- Location: src/discord/adapter.rs:2638
- Evidence: `.send_message_with_reference(channel, "\u{200b}".to_owned(), reference_id)`
- Why it matters: The streams mutex acquired at the beginning of this block remains held across the following await. A rate-limited or slow placeholder send in one channel prevents every other channel and bot from looking up, creating, or removing stream entries. This is process-wide head-of-line blocking rather than necessary per-stream serialization.
- Suggested fix: Reserve per-stream initialization state under the map lock, release the global lock, then perform network I/O behind a per-stream initialization guard.

### [P1] Successful finalization erases duplicate-sequence protection
- Location: src/discord/adapter.rs:2679
- Evidence: `streams.remove(&key);`
- Why it matters: last_sequence exists only inside the removed ActiveDiscordStream. After a final chunk succeeds, delivering the identical stream UUID/sequence again finds no entry, sends another placeholder, and publishes the same final response again. The sequence guard therefore handles concurrent overlap but not a sequential replay of a completed final chunk.
- Suggested fix: Keep a bounded completed-stream tombstone with the terminal sequence, or deduplicate final delivery durably before creating a new placeholder.

### [P1] Dead-target short circuit prevents typing shutdown
- Location: src/discord/adapter.rs:3060
- Evidence: `if self.dead_targets.is_dead_for_bot(&bot_id, channel_id) {`
- Why it matters: This check returns before distinguishing active=true from active=false. If typing starts, a later send marks the channel dead, and terminal cleanup dispatches Typing(false), the existing Typing guard is never removed. Its refresh task remains alive and can continue failed typing requests even though the dead-target registry is intended to suppress requests.
- Suggested fix: Always remove local typing state for active=false before checking target health or HTTP availability.

### [P1] Typing start can install a guard after typing stop completes
- Location: src/discord/adapter.rs:3067
- Evidence: `let _ = http.broadcast_typing(channel).await;`
- Why it matters: The active branch creates a live Typing guard, awaits an HTTP broadcast, and only afterward inserts it into the map. A concurrent Typing(false) during that await removes nothing and returns; the delayed active branch then installs a guard that outlives the completed turn. A rate-limit wait widens this race substantially.
- Suggested fix: Serialize typing state transitions or install a generation-tagged guard before network awaits and reject stale starts after a stop.

### [P1] Typing refresh throttling conflates bot identities and is not cleared on stop
- Location: src/discord/adapter.rs:2001
- Evidence: `typing_refresh: Arc<Mutex<HashMap<u64, std::time::Instant>>>,`
- Why it matters: keep_typing records only channel ID, even though the same egress supports multiple bot clients. Bot A's refresh suppresses bot B's indicator in the same channel. Also Typing(false) does not remove this entry; streams cancelled before any final chunk retain one timestamp per visited channel indefinitely.
- Suggested fix: Key refresh state by bot identity and channel, and remove that identity's refresh state on stop/cancellation as well as successful finalization.

### [P1] Reactions to thread followups target the parent channel
- Location: src/discord/adapter.rs:3085
- Evidence: `let channel = session`
- Why it matters: The converter deliberately stores a thread's parent in session.channel_id and the actual thread in session.thread_id. React always parses session.channel_id, so a normal message posted inside a thread is reacted to using the parent channel and fails with Unknown Message. Only the initial parent message that caused auto-thread creation matches this assumption. Both reaction failures are merely debug-logged, so the start reaction in the actual thread can remain stuck.
- Suggested fix: Carry the original message channel with reaction actions; use the thread for genuine thread messages and the parent only for the auto-thread starter.

### [P1] Operation-specific permission errors poison the whole channel
- Location: src/discord/adapter.rs:2338
- Evidence: `if code == 403 {`
- Why it matters: Every 403 is classified as channel death, and EditMessage/DeleteMessage use that classification. A bot allowed to send but not delete another user's message can receive Missing Permissions and then refuse all future sends to that channel. An edit of a message not owned by the bot similarly does not prove the channel is dead. The default registry has no TTL/probe interval, so an unrelated operation failure can disable delivery indefinitely.
- Suggested fix: Classify whole-target failure using operation context and Discord error codes; do not infer send unavailability from edit/delete permissions.

### [P1] Forum child-send errors mark the forum parent dead
- Location: src/discord/adapter.rs:2799
- Evidence: `self.dead_targets.mark_dead_for_bot(`
- Why it matters: After creating a forum post, continuation chunks are sent to post_channel.id, but the error handler records channel_id, which still names the forum parent. If the newly created child is deleted or becomes inaccessible between chunks, the valid forum is suppressed for future independent posts.
- Suggested fix: Record the actual failing post_channel.id for continuation failures; leave the parent healthy unless a parent operation fails.

### [P1] Forum upload titles exceed Discord's name limit
- Location: src/discord/adapter.rs:1812
- Evidence: `format!("Voice Note: {filename}")`
- Why it matters: send_forum_file uses the complete filename with a prefix and never calls the existing title limiter. Valid filesystem filenames longer than 88 characters for voice uploads or 92 characters for ordinary uploads produce names over Discord's 100-character limit, causing otherwise valid file uploads to fail.
- Suggested fix: Apply a shared Discord thread-name limiter to the prefixed filename before CreateForumPost.

### [P1] Approval mentions are appended outside the 2000-character budget
- Location: src/discord/adapter.rs:3292
- Evidence: `format!("{mentions}\n\n{plain_content}")`
- Why it matters: build_approval_content budgets the command against the entire message limit, then this function adds an unrestricted mention list. A long ASCII command with reason `dangerous command` produces 1994 characters before mentions; one normal 18-digit user ID raises it to 2017. Discord rejects the approval request, preventing the operator from receiving its buttons.
- Suggested fix: Reserve space for the mention prefix before truncating the command, and bound/deduplicate the mention list itself.

### [P1] EditMessage does not handle Discord's content limit
- Location: src/discord/adapter.rs:2945
- Evidence: `.content(content)`
- Why it matters: SendMessage splits output and streams use a throttler, but EditMessage submits arbitrarily long content unchanged. Editing with a 2001-character ASCII answer fails instead of delivering a bounded edit or continuation. There is no adapter-side overflow policy or useful prevalidation.
- Suggested fix: Apply an explicit edit overflow policy: edit the first bounded chunk and send continuations, or reject oversized edits at the boundary with a clear error before transport.

### [P1] Injected message transport silently drops reply references
- Location: src/discord/adapter.rs:2756
- Evidence: `transport.send_message(channel, chunk).await?;`
- Why it matters: SendMessage parses reply_id but the message_transport branch never uses it, despite DiscordMessageTransport exposing send_message_with_reference and the stream branch using that method. With with_message_transport configured, replies become unthreaded sends and the transport seam cannot verify production reply behavior.
- Suggested fix: Enumerate chunks and call send_message_with_reference with should_chunk_reference for the first chunk, matching the normal transport path.

### [P1] Dead-target persistence can run in the opposite order to in-memory mutations
- Location: src/discord/adapter.rs:2159
- Evidence: `tokio::spawn(async move {`
- Why it matters: mark_dead_for_bot and clear_for_bot update memory synchronously but spawn independent unordered SQLite writes and ignore their results. A mark followed by a successful clear can execute DELETE first and INSERT second, resurrecting a dead target on restart even though memory is healthy. Database errors also silently destroy the advertised durability. Every successful send additionally spawns a DELETE even when no entry existed.
- Suggested fix: Serialize persistence per bot/channel or use generation-checked writes, propagate/log failures, and avoid unnecessary clears when no durable entry needs removal.

### [P1] Replay acknowledges success even when obligation state updates fail
- Location: src/discord/adapter.rs:2488
- Evidence: `let _ = ledger.mark_obligation_delivered(&obl.id).await;`
- Why it matters: After a replayed message is sent, a database failure marking it delivered is ignored, as are failed-state writes. replay_failed_transport_obligations still returns Ok(count). The durable obligation can remain claimed/undelivered despite visible delivery, allowing duplicate replay or leaving work stranded with no reported persistence failure.
- Suggested fix: Propagate or explicitly aggregate obligation-state update errors and distinguish claimed, delivered, and failed counts.

### [P1] Legacy cursor updates are a non-atomic check then write
- Location: src/discord/adapter.rs:3347
- Evidence: `last_message_id = excluded.last_message_id,`
- Why it matters: update_channel_cursor selects the current cursor outside a transaction and later unconditionally upserts. Two concurrent events can both read 100; the 300 update commits first and the delayed 200 update overwrites it. A supposedly monotonic last-seen cursor therefore regresses under normal concurrent gateway handlers.
- Suggested fix: Enforce the numeric snowflake comparison in the atomic UPSERT update predicate, rather than relying on a preceding SELECT.

### [P1] Backfill advances its durability cursor past in-progress duplicates
- Location: src/discord/adapter.rs:1235
- Evidence: `if !ledger.record_incoming_as(&event, &delivery_id).await? {`
- Why it matters: record_incoming_as declines both already-delivered and currently in-progress claims; this helper turns either into Ok(false). Backfill treats that as a completed outcome and advances its cursor. If live ingress owns the duplicate and later fails, or the process dies before that turn completes, recovery has already advanced past work that never completed durably.
- Suggested fix: Distinguish delivered duplicates from in-progress claims and wait for their terminal outcome, or hold the cursor until durable completion is established.

### [P1] Transient role lookup failures silently skip recoverable history
- Location: src/discord/adapter.rs:3675
- Evidence: `Err(_) => msg`
- Why it matters: A failed member-role lookup falls back to message.member or empty roles. REST history commonly lacks member metadata. A user admitted only by an allowed role is then treated as unauthorized; the loop advances its in-memory cursor, and a later admitted message can persist a cursor beyond the skipped one. The authorized message is never retried after the transient lookup error clears.
- Suggested fix: Keep lookup failure distinct from a successful empty-role result. If admission depends on unavailable roles, stop that channel's scan without advancing its cursor and report the lookup error.

### [P1] Backfill suppresses cursor persistence errors and advances anyway
- Location: src/discord/adapter.rs:3733
- Evidence: `let _ = update_bot_channel_cursor(`
- Why it matters: After awaiting a turn, backfill ignores any cursor-write error, advances current_cursor_str, and eventually reports success. Durable progress can remain stale while the rest of the scan continues. On restart the same history must be fetched again; an exhausted scan budget or persistent database issue is hidden from its caller.
- Suggested fix: Propagate the write error or halt that channel, preserving the last durable cursor and reporting incomplete recovery.

### [P1] Filtered history never persists progress
- Location: src/discord/adapter.rs:3754
- Evidence: `current_cursor_str = Some(msg_id_str);`
- Why it matters: The rejected-message branch advances only the local cursor. A channel with no newly admitted messages after its stored cursor repeatedly scans the same growing tail of bot/system/unmentioned messages on every Ready. Each historical message also incurs channel and member lookups before filtering, making reconnects increasingly expensive and consuming Discord rate limits without new work.
- Suggested fix: Persist progress for conclusively filtered messages too, while distinguishing transient metadata/admission failures that must hold the cursor.

### [P1] Backfilled mentions bypass live auto-thread routing
- Location: src/discord/adapter.rs:3728
- Evidence: `match route_claimed_event_awaiting_turn(data, event).await {`
- Why it matters: Backfill converts the message and dispatches it directly; it never executes the live handler's auto-thread creation/reuse or ownership updates. With auto_thread enabled, a missed mention in a normal guild channel is answered in the parent channel rather than in the thread used for the same live input. A thread already created before a crash is not recovered as the invocation target either.
- Suggested fix: Share an idempotent target-resolution step between live and backfill routing, including existing starter-thread reuse, before choosing how to await the turn.

### [P1] Approval expiry removes its retry target before the HTTP edit succeeds
- Location: src/discord/adapter.rs:3198
- Evidence: `self.remove_approval_message(&request_id).await`
- Why it matters: ExpireApproval removes the only tracked channel/message tuple before requesting the edit. If HTTP fails, or the target is temporarily short-circuited, the entry is already gone. Retrying the same action returns success without removing the buttons, leaving a stale actionable-looking approval in Discord.
- Suggested fix: Keep the target until cleanup succeeds, or retain a bounded retry record; use idempotent lookup-and-edit followed by conditional removal.

### [P1] One bot's gateway traffic masks another bot's receive failure
- Location: src/discord/adapter.rs:1947
- Evidence: `let last_event = LAST_DISCORD_EVENT_MS.load(Ordering::Relaxed);`
- Why it matters: Every adapter updates the same process-global timestamp and every watchdog reads it. In a multi-bot process, regular events from healthy bot A keep silence_secs below the threshold for failed bot B, so B's runner health is never checked and its watchdog never restarts it.
- Suggested fix: Store the receive timestamp per client/bot (and shard if applicable), and pass that state to the matching watchdog.

### [P1] Receive watchdog tasks outlive the clients they monitor
- Location: src/discord/adapter.rs:1941
- Evidence: `tokio::spawn(async move {`
- Why it matters: start spawns an infinite watchdog holding an Arc to the shard manager, discards the JoinHandle, and returns when client.start finishes. There is no cancellation on normal shutdown or start failure. Restarting adapters in the same process accumulates old tasks and managers; old watchdogs continue checking and issuing restart requests against stopped clients.
- Suggested fix: Tie the watchdog to the client's lifetime with a cancellation token or retained task handle and shut it down when client.start returns.

### [P2] The per-user thread-session setting is ignored
- Location: src/discord/adapter.rs:1548
- Evidence: `let user_id = if is_dm {`
- Why it matters: InboundFilterConfig exposes thread_sessions_per_user, defaults it to true, and every ingress path copies the configured value, but conversion never reads it. All guild/thread sessions receive an empty user ID regardless of the setting. The public configuration therefore promises a distinction that this adapter cannot implement, and the debouncer also merges different guild users into the same session batch.
- Suggested fix: Either honor the setting consistently in thread session construction, including auto-thread starters, or explicitly remove/deprecate it if shared guild sessions are intentional.

### [P2] Debouncer coalescing test depends on a real-time scheduling window
- Location: src/discord/adapter.rs:4148
- Evidence: `// Enqueue all three chunks back-to-back so they are guaranteed to land`
- Why it matters: The test uses an actual 50 ms debounce timer and three separate awaited enqueues. They are not guaranteed to occur within the window: descheduling between enqueues can flush the first batch early. The subsequent 300 ms negative timeout likewise cannot prove there will never be a late second dispatch. The event subscription is good, but it does not make batch membership deterministic.
- Suggested fix: Use paused Tokio time, enqueue all inputs before explicitly advancing the virtual clock, then synchronize completion of all timer work and inspect the collected events without a real-time absence window.

## Strengths
- Inbound filtering rejects self messages, webhooks, unwanted bot traffic, and unsupported system-message kinds before creating model events; the live gateway path fails closed on unresolved guild channel metadata.
- Normal send dispatch bounds message chunks and applies safe allowed mentions, and the first-chunk-only reply helper avoids repeatedly pinging a referenced author.
- Attachment coalescing and referenced-parent attachment handling deduplicate IDs, and failed hydration is explicitly logged while retaining remote attachment metadata.
- Backfill sorts each fetched page by snowflake and awaits newly claimed turn outcomes; SQL values are bound rather than interpolated, and Unknown Message is explicitly excluded from dead-channel classification.

## Notes
- This is a static, read-only review. No product files were changed, no build/tests or live Discord requests were run, and no fixes were attempted. Dependency contracts for ledger claims and reasoning filtering were read only to validate adapter findings, not reviewed as separate targets.
- The entire target and its inline tests were read. Structural scans covered unwrap/expect/panic, blocking operations, TODO/FIXME/HACK, lock acquisition, ignored Results, spawn sites, and awaits. Runtime unwraps on nonempty coalescing and constant regexes have local invariants; no finding alleges those are reachable panics.
- The stream-map mutex is held across placeholder HTTP I/O. The per-stream sequence mutex is also held across media upload and throttler update, but that is scoped serialization; no lock-order cycle was proven. The watchdog explicitly drops the shard-runner lock before its restart await.
- Root-prefix and approval-length examples were checked with non-mutating Python calculations: `/tmp-private/secret.txt` passes the string prefix, and a long ASCII approval with one 18-digit-user mention reaches 2017 characters. Other failure paths are established by code flow and stated timing/input conditions, not by executing production integrations.
- Concrete static paths were preferred over adding a test harness because this lane permits writing only this evidence file. Discord history's optional guild_id and the 100-character thread-name/2000-character message limits are API assumptions underlying the corresponding findings. Exact supplementary-Unicode length accounting, zero-snowflake constructor behavior, and codec-level voice validity were not independently verified and are not claimed as findings.
