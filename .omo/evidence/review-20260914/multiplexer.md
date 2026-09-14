# Lane: multiplexer
## Scope
- `src/multiplexer/actor.rs`: 1,683 LOC, read in full including tests.
- `src/multiplexer/router.rs`: 549 LOC, read in full.
- `src/multiplexer/profile_routing.rs`: 820 LOC, read in full including tests.
- `src/multiplexer/gc.rs`: 96 LOC, read in full.
- `src/multiplexer/restart_loop_guard.rs`: 253 LOC, read in full including tests.
- `src/multiplexer/mod.rs`: 13 LOC, read in full.
- Total: 3,414 LOC. Review target and findings are confined to these six files; no product files were modified.

## Findings
### [P0] DashMap shard locks survive asynchronous waits
- Location: src/multiplexer/router.rs:413
- Evidence: `        if let Some(handle) = self.sessions.get(key) {`
- Why it matters: This binding is a DashMap read guard, not a cloned Arc. The next line awaits `handle.set_model(model)` while retaining the synchronous shard lock. `reset` and `session_context` repeat this pattern; `mark_in_flight_resume_pending` retains an iterator entry across a SQLite await. A concurrent actor insertion/removal on the same shard blocks a Tokio worker synchronously. On a current-thread executor, a pending actor reply plus such a writer deadlocks the executor; on the production multi-worker shape, enough contending writers can exhaust workers and prevent the awaited actor/database continuation from running. Shard collisions also affect unrelated sessions.
- Suggested fix: Clone each Arc out of `get` before awaiting, as `stop` already does. Snapshot owned session keys before the resume-pending database loop so no DashMap reference or iterator survives an await.

### [P0] Cancelling a backpressured send permanently leaks the in-flight count
- Location: src/multiplexer/router.rs:179
- Evidence: `        let result = self.sender.send(command).await;`
- Why it matters: `send_command` increments `in_flight` before this await and decrements it only on normal continuation. If the mailbox is full and the caller times out, is aborted, or loses a select, the future is dropped without `finish_send`. The next GC sets `accepting=false` and waits forever for the leaked count. New routes then wait forever for that session to become reusable, and the sequential collector cannot reach later sessions.
- Suggested fix: Install a drop guard immediately after incrementing `in_flight`, and have its Drop perform the decrement and drained notification on every exit, including cancellation.

### [P0] Cancelling garbage collection can strand a live session in retirement
- Location: src/multiplexer/gc.rs:61
- Evidence: `        handle.wait_for_in_flight().await;`
- Why it matters: `try_retire` has already changed `accepting` to false before this cancellation point. The public `collect_garbage` future can be dropped here, while sending eviction, or while awaiting the reply; none of these paths restores the handle. For example, cancel collection after enqueueing eviction and let a busy actor reply false: there is no collector left to call `resume`, subsequent collectors skip the already-retired handle, and all new routes wait indefinitely.
- Suggested fix: Give each retirement transaction an owned, cancellation-independent task that finishes the barrier/eviction/reply protocol. Restore acceptance on pre-enqueue cancellation; do not blindly resume after an eviction command may already have been accepted.

### [P1] One unresponsive actor blocks all GC and GC shutdown
- Location: src/multiplexer/gc.rs:24
- Evidence: `                        if let Err(error) = collect(&multiplexer).await {`
- Why it matters: Collection is awaited inside the timer branch, so the shutdown branch is not polled until the whole collection finishes. Collection processes handles sequentially with unbounded waits for sends and replies. An actor stuck in dispatcher/database/cancellation work outside its command-select loop can therefore hold the collector indefinitely, delay unrelated evictions, and make `ScaleToZero::shutdown` hang on its JoinHandle even after signalling shutdown. This does not require the cancellation-count defect above.
- Suggested fix: Bound per-session collection work and isolate sessions with bounded concurrency. Make shutdown observable while collecting, using the cancellation-safe retirement ownership described above rather than abandoning a partially retired handle.

### [P0] A zero GC interval panics the collector task
- Location: src/multiplexer/gc.rs:18
- Evidence: `            let mut interval = tokio::time::interval(multiplexer.gc_interval());`
- Why it matters: `MultiplexerConfig.gc_interval` is publicly configurable and constructors do not validate it. Passing `Duration::ZERO` reaches Tokio's zero-period panic in the spawned task. GC silently stops running; shutdown also discards the resulting JoinError, so the configured service can continue indefinitely without its collector.
- Suggested fix: Reject a zero GC interval at the configuration boundary and propagate collector task failures instead of discarding the JoinError.

### [P0] Transcript presence is incorrectly treated as successful turn completion
- Location: src/multiplexer/actor.rs:236
- Evidence: `                                self.complete_delivery(event.delivery_id.as_deref(), &Ok(()))`
- Why it matters: `persist_inbound` inserts the user message before the backend runs. If that run fails, is stopped, or the process crashes, the user row remains. A replay with the same nonempty platform message ID hits `has_platform_message_id`, executes this success branch, acknowledges success, and never runs the unfinished turn. An acknowledgement-driven backfill can advance its durability cursor past a turn that never succeeded.
- Suggested fix: Deduplicate execution using a durable successful terminal outcome, not the mere existence of the inbound transcript row. Reuse the persisted inbound record when retrying unfinished/failed turns.

### [P0] Overflow drops events after route has already reported acceptance
- Location: src/multiplexer/actor.rs:309
- Evidence: `                                                "pending turn queue full (max {}); dropping new event",`
- Why it matters: During one blocked turn, the actor drains the bounded mailbox into a second queue of 64 events. Further plain events are dropped here, without persistence or a reply to their sender. `route` has already returned Ok as soon as the mailbox accepted each event. When `delivery_id` is None, `complete_delivery` is a no-op, leaving only a warning and permanently losing an accepted conversation turn. The bounded mailbox therefore does not provide end-to-end admission backpressure.
- Suggested fix: Make every event carry an admission acknowledgement and return an explicit queue-full error to its caller, or reserve pending-turn capacity before reporting acceptance. Keep control-command capacity available rather than merely stopping mailbox consumption.

### [P1] Biased mailbox polling can starve the running backend
- Location: src/multiplexer/actor.rs:300
- Evidence: `                            biased;`
- Why it matters: The always-first branch receives commands; the backend future is polled only if that branch is pending. Under sustained input to one session, producers can keep the mailbox ready even after the pending queue fills. Processing/dropping events continues to win over backend progress or an already-ready completion, prolonging the turn and causing further overflow. Cooperative scheduling yields execution to other tasks but does not establish fairness between these select branches.
- Suggested fix: Remove the unconditional bias or enforce a finite command batch followed by backend polling. Preserve stop responsiveness through an explicit control policy rather than giving all event traffic strict priority.

### [P0] A successful running turn overwrites an acknowledged model change
- Location: src/multiplexer/actor.rs:424
- Evidence: `                                self.context = turn_context;`
- Why it matters: The runner receives a clone made before its execution. While it is running, `SetModel` updates `self.context.state.active_model` and acknowledges success, but cannot update the borrowed `turn_context`. If the old turn then succeeds, this whole-context assignment restores the old model and the following flush persists it, also undoing the router's earlier database write. Subsequent turns silently use the old selection.
- Suggested fix: Track actor-owned model changes made during a turn and merge them after completion, or serialize model changes at turn boundaries while keeping their acknowledgements pending until applied.

### [P1] Model persistence replaces unrelated state from a stale database snapshot
- Location: src/multiplexer/router.rs:406
- Evidence: `            "UPDATE sessions SET state_json = ?, updated_at = CURRENT_TIMESTAMP WHERE session_key = ?",`
- Why it matters: `set_model` reads all of `state_json`, changes one field, and later replaces the entire JSON without a transaction/version check. If the actor flushes a new remote binding, suspension flag, or other state between that read and write, the stale model update deletes those unrelated changes. If the actor is retiring or gone by the later handle lookup, no actor command repairs the overwritten state. Concurrent model operations also have separate database and mailbox orderings.
- Suggested fix: Let the live actor own the update; for an absent actor, use an atomic SQL JSON-field update coordinated with actor creation/retirement rather than replacing a previously read whole document.

### [P0] Selecting a model before the first session row exists is a successful no-op
- Location: src/multiplexer/router.rs:411
- Evidence: `        .await?;`
- Why it matters: In `set_model`, a missing SELECT row becomes default state, but the only write is an UPDATE and its affected-row count is ignored at this line. For a fresh key with no actor and no stored session, zero rows are updated, the handle branch is skipped, and the method returns Ok. The first subsequent event creates a default/profile-selected session instead of using the requested model.
- Suggested fix: Create/load the session actor and apply the mutation through it, or upsert the session with its key fields and selected model. Do not return success for a zero-row mutation.

### [P0] Reset silently does nothing for collected or retiring sessions
- Location: src/multiplexer/router.rs:420
- Evidence: `        if let Some(handle) = self.sessions.get(key) {`
- Why it matters: The reset method has no storage fallback and returns Ok when the session has been GC'd, even though its SQLite state and remote thread binding remain. For a present but retiring handle, `SessionHandle::reset` also returns Ok when its command outcome is Retiring/Closed, so reset can be lost during normal collection. The next event reloads the supposedly reset session unchanged. Errors from an accepted reset are discarded too.
- Suggested fix: Give reset the same retirement/retry protocol as stop, propagate its reply error, and reset persisted state when no live actor exists (or load the actor to perform the operation).

### [P0] The remote-binding preservation rule undoes explicit reset
- Location: src/multiplexer/actor.rs:758
- Evidence: `                 THEN json_set(?, '$.metadata.omo_thread_id', json_extract(state_json, '$.metadata.omo_thread_id'))`
- Why it matters: Idle Reset removes `omo_thread_id` and assigns default state, but flush immediately restores the old database binding whenever the incoming JSON lacks it. After eviction/restart the old remote conversation is loaded again. Active Reset is worse: after removing the key, the unconditional copy from the old `turn_context` re-inserts it before shutdown handling. Thus an explicit reset cannot reliably sever session stickiness. The idle SQL behavior was reproduced using the exact source query against in-memory SQLite.
- Suggested fix: Distinguish intentional binding deletion from a stale missing value. Use a dedicated reset write or explicit tombstone/generation, and skip old-turn binding merge after a reset.

### [P1] Stop acknowledges success even when suspension could not be persisted
- Location: src/multiplexer/actor.rs:567
- Evidence: `                    let _ = self.flush_if_dirty().await;`
- Why it matters: Idle Stop ignores its flush error and immediately replies Ok(false). Active Stop likewise ignores its flush error and returns only the runner cancellation result. If SQLite fails and the process exits before a later successful flush, durable state does not contain the requested suspension; recovery can resume a session the user was told had stopped. Unlike normal turn-completion flush failures, these failures are not reported or explicitly marked for recovery.
- Suggested fix: Propagate the suspension flush failure in the stop reply, log it with the session key, and keep the dirty/recovery state until suspension is durably recorded.

### [P1] Failure paths discard the dirty flag for surviving state mutations
- Location: src/multiplexer/actor.rs:447
- Evidence: `                                self.dirty = false;`
- Why it matters: Before this error branch the actor deliberately copies any remote `omo_thread_id` from the failed runner's context into its own state. Clearing dirty then prevents idle GC and graceful shutdown from persisting that binding. The same unconditional dirty reset after `persist_inbound` failure can erase a pre-existing dirty obligation from an earlier failed flush. Eviction can report success without writing those retained in-memory mutations.
- Suggested fix: Keep dirty set for mutations that remain in `self.context`, including the remote binding and pre-existing failed writes; discard only turn-local changes that are actually rolled back.

### [P0] Recovery clears its durable retry marker before doing recoverable work
- Location: src/multiplexer/actor.rs:88
- Evidence: `            let cleared = crate::storage::clear_session_resume_pending(pool, &storage_key).await?;`
- Why it matters: The marker is cleared before querying the unfinished turn, resolving the ledger, or routing the event. A later query error, actor startup failure, drain refusal, queue rejection, or process crash leaves the session unmarked. Routing errors are only logged, and successful enqueueing increments the count without awaiting terminal success. A later boot no longer finds this unfinished session through the resume-pending recovery path.
- Suggested fix: Keep a durable pending/claimed marker until the recovered turn has a terminal successful outcome; use `route_awaiting_turn` and clear only after its success. Restore/release a durable claim on every failure rather than consuming the retry flag up front.

### [P1] Acknowledged routing bypasses the drain gate
- Location: src/multiplexer/router.rs:360
- Evidence: `        let key = event.session.clone();`
- Why it matters: `route_awaiting_turn` immediately obtains/creates an actor and sends its event without checking `drain_rx`, whereas `route` explicitly refuses new turns when draining. Calling the acknowledged API after drain is already true still starts a new turn. Shutdown/backfill can therefore create work after the gateway has announced that new turns are refused.
- Suggested fix: Share one admission gate between both route APIs and re-evaluate drain when a retirement wait causes routing to retry.

### [P1] Idle age includes the entire duration of the last active turn
- Location: src/multiplexer/actor.rs:595
- Evidence: `                    let idle = self.last_active_at.elapsed() > idle_timeout`
- Why it matters: Ordinary turn activity sets `last_active_at` when the event starts, but terminal handling updates only `context.updated_at`. A turn lasting longer than `idle_timeout`, with no explicit heartbeat, becomes immediately eligible for eviction as soon as it completes. A GC check just after completion therefore removes a just-used actor instead of preserving the configured idle grace period, causing avoidable churn at long-turn boundaries.
- Suggested fix: Refresh the monotonic activity timestamp at every terminal turn outcome, in addition to start/heartbeat updates.

### [P0] Guild or catch-all routes preempt the documented parent-channel fallback
- Location: src/multiplexer/profile_routing.rs:302
- Evidence: `            .find(|r| r.matches(guild_id, channel_id, thread_id))`
- Why it matters: The purported direct-channel stage accepts any matching route, including a guild-only or unconstrained route, and returns before the parent-channel stage. With guild=100, channel/thread=300, parent=200, a parent-channel route for 200 and a guild-only route for 100, this stage selects the guild route although the documented precedence requires the parent route. Its model, prompt, tools, and policy can differ from the intended parent configuration.
- Suggested fix: Restrict the direct-channel stage to routes explicitly targeting that channel, evaluate the parent stage next, and defer guild/catch-all matching until the final fallback stages.

### [P1] Actor initialization never uses parent-aware event routing
- Location: src/multiplexer/actor.rs:858
- Evidence: `                router.apply_to_session(&mut context);`
- Why it matters: Both existing and fresh session loading route only from the SessionKey. `apply_to_session` calls `match_session`, which has no parent-channel input. Although `match_event` can read `parent_chat_id`, the actor never invokes it when processing an event. For the supported representation where channel_id is the thread ID and its parent is event metadata, a parent-only profile works in `match_event` but is never applied to the actual new actor. Fixing precedence alone does not repair this path.
- Suggested fix: Carry the initial event's parent context into actor profile selection, or apply event-aware defaults before persisting/executing the first turn; retain the routing identity needed on reload.

### [P0] Authorization matching drops guild and thread restrictions
- Location: src/multiplexer/profile_routing.rs:235
- Evidence: `        if let Some(route) = self.match_route(None, channel_id, None) {`
- Why it matters: The authorization API cannot supply guild, thread, or parent context and explicitly matches with None. For a restrictive route `{guild:100, channel:200, allowed_users:[1]}`, a request from user 2 in that actual guild/channel fails to match the restriction. With no matching bot fallback and an empty global allowlist, the function returns true. Thread-specific and inherited parent policies have the same mismatch between configuration routing and authorization routing.
- Suggested fix: Authorize against the same full event context and resolved route used for execution, passing guild/thread/parent IDs rather than substituting None.

### [P0] Bot authorization can use a disabled or unrelated route
- Location: src/multiplexer/profile_routing.rs:242
- Evidence: `                if route.name.as_deref() == Some(bot) || route.profile.as_deref() == Some(bot) {`
- Why it matters: The bot fallback checks only name/profile equality, not `enabled` or the Discord hierarchy. With a disabled route named for this bot allowing user 2 and a global list allowing only user 1, user 2 is authorized through the disabled rule whenever the initial channel match supplies no allowlist. A rule scoped to a different channel/guild can likewise become a bot-wide authorization override; first-match order decides which unrelated rule wins.
- Suggested fix: Ignore disabled rules and separate explicit bot-wide access policy from context-scoped profile routes. Reuse a context-valid resolved route for route-specific access checks.

### [P0] Invalid negative route IDs silently become wildcards
- Location: src/multiplexer/profile_routing.rs:27
- Evidence: `                Ok(None)`
- Why it matters: A negative integer for a guild/channel/thread ID is accepted and converted to None, which matching interprets as no constraint. For example, `{channel:-1, allowed_users:[2]}` becomes a catch-all authorization override and can allow user 2 outside the intended scope despite a restrictive global allowlist. A malformed target should not broaden a rule to every conversation.
- Suggested fix: Reject negative identifiers with a deserialization error and reject the invalid configuration without replacing the active routing/access policy.

### [P0] One malformed profile entry removes every route-level access restriction
- Location: src/multiplexer/profile_routing.rs:402
- Evidence: `            Vec::new()`
- Why it matters: Failure to deserialize any element causes `parse_profile_routes` to return an empty route set. With route-level allowlists as the restriction and the supported empty global allowlist, `ProfileRouter::from_json` then builds a router whose authorization helper allows everyone. A typo such as a nonnumeric channel string thus changes invalid restrictive configuration into valid unrestricted configuration rather than preventing startup/reload.
- Suggested fix: Return a parse Result to the configuration boundary and reject the startup/reload or retain the last valid policy. Do not substitute an empty authorization policy for malformed nonempty input.

### [P1] Equivalent channel-ID spellings make prompt selection nondeterministic
- Location: src/multiplexer/profile_routing.rs:460
- Evidence: `            routes.sort_by_key(|r| r.channel);`
- Why it matters: Channel prompt entries first come from a randomized HashMap and normalize keys with trim/parse. JSON keys such as `"123"` and `"0123"` (or `" 123 "`) are distinct map entries but become the same numeric channel. This stable sort ties them, preserving randomized map order; the stable specificity sort also preserves the tie, and first-match routing chooses whichever prompt/toolset happened to come first. Identical configuration can select different behavior on different starts.
- Suggested fix: Detect and reject duplicate normalized channel IDs, or define a deterministic conflict rule before constructing routes rather than relying on HashMap iteration order.

### [P1] Bot-profile database errors silently select fallback settings
- Location: src/multiplexer/actor.rs:793
- Evidence: `                if let Ok(Some((model, prompt, toolsets))) = sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(`
- Why it matters: Both load branches discard Err exactly like an absent bot profile. If the session query succeeds but the bot-profile query encounters a pool/SQLite error, actor startup still succeeds using default or generic routed model/prompt/toolsets instead of the bot's configured settings. Those defaults can subsequently be persisted, making a transient read failure sticky. There is no error log identifying the lost override.
- Suggested fix: Propagate bot-profile query failures from actor loading, distinguishing a successful None row from Err. Apply fallback only for an actually absent profile.

### [P1] Restart-breaker persistence failures silently disable the threshold
- Location: src/multiplexer/restart_loop_guard.rs:75
- Evidence: `            if fs::write(&tmp_path, raw).is_ok() {`
- Why it matters: Directory creation, writes, renames, reads, and clear errors are discarded. With an unwritable state directory or a full filesystem and no existing log, every new process reads an empty history, records one boot only in memory, returns false for the default threshold of three, and attempts the same crash-triggering resume again. If an old below-threshold log exists, failed saves similarly prevent the persisted count from advancing. The guard supplies no failure result for the caller to act on.
- Suggested fix: Return and log I/O errors, distinguish NotFound from other read errors, and suppress automatic resume when the restart guard cannot reliably read/write its safety state. Make clear failures observable too.

### [P1] Future boot timestamps remain counted after a backward clock jump
- Location: src/multiplexer/restart_loop_guard.rs:89
- Evidence: `                if boot > now {`
- Why it matters: Returning false from the rposition predicate skips a future timestamp for gap comparison but does not remove it from the returned vector. For persisted boots [1000,1010,1020] and a clock reset to 0, no old-gap index is found, so all three future boots remain and `is_tripped_at(0)` is true. Real new boots can be blocked by timestamps arbitrarily far in the future; appending and truncating before re-sorting can also keep stale future entries while discarding recent legitimate ones.
- Suggested fix: Filter or explicitly reset anomalous future timestamps before computing the consecutive chain, and keep a documented clock-skew policy. Sort/filter before applying the retained-history cap.

### [P2] The typing error-path test can pass without executing the failing runner
- Location: src/multiplexer/actor.rs:1402
- Evidence: `                .send(ActorCommand::Stop { reply: reply_tx })`
- Why it matters: The test queues Stop immediately after Event and then waits for the stop reply, not for the runner's error outcome. Both sends fit the channel, so Stop can already be waiting when the actor reaches its biased select; the mailbox branch cancels the turn before `FailingRunner` is polled. The stop path emits the same true/false typing pair being asserted, so this test remains green even if error-completion typing cleanup breaks.
- Suggested fix: Send EventWithAck and await its error result before closing the channel, or subscribe to an explicit terminal-error signal before triggering the event. Use a bounded timeout and no sleeps.

## Strengths
- Production event mailboxes and pending-turn queues are explicitly bounded at 64; the unbounded channels found in these files are test instrumentation, not production actor mailboxes.
- Normal route/stop paths clone Arc handles out of the DashMap before awaiting. Pointer-identity removal, weak actor ownership, startup watch state, and the GC barrier protect against replacing/removing the wrong actor generation on normal completion paths.
- Session SQL uses bound parameters, and actor state/transcript writes are generally centralized. Eviction flush errors keep the actor alive and resume acceptance rather than dropping dirty state immediately.
- Profile precedence sorting is stable for ordinary distinct inputs. Restart history retention is capped, and tests explicitly cover the intended third-boot threshold, disabled threshold, inclusive 300-second chain gap, and reset after a longer quiet gap.

## Notes
- Severity follows the requested rubric: P0 includes proven correctness/data-loss/authorization/deadlock defects, not only catastrophic operational incidents. Edge-condition risks and missing error handling are P1.
- All six files were read completely, followed by targeted re-reads and structural scans for panic/unwrap/expect, blocking operations, TODO/FIXME/HACK, locks/awaits, channels, ignored errors, and SQL. Recent target-file history was inspected for context. No out-of-scope code was reviewed as a finding target.
- This is a source-level review, not a claim that the Rust suite or gateway was executed. The exact actor flush SQL was executed against in-memory SQLite and confirmed to restore an old remote thread ID after reset. No build/test commands that create repository artifacts were run.
- No Rust memory-unsafe use-after-free was established. The proven post-GC defects are logical lifecycle failures (lost reset, stranded retirement, and stale state), not unsafe pointer dereferences. No independent parking_lot lock-order cycle, SQL injection, or DST scheduling defect was established in the scoped code.
- The restart guard deliberately uses a consecutive-boot chain with a minimum 300-second gap and trips on the configured count including the recorded boot; that policy and its boundary tests are consistent, so it is not reported as an off-by-one/window defect. Its synchronous filesystem functions are not themselves async; async-caller blocking was not asserted without an in-scope caller.
- The authorization findings prove the scoped helper's return values for the stated inputs; this review does not claim that every ingress surface uses that helper without additional checks. Likewise, the parent-aware actor finding concerns the thread-as-channel representation explicitly supported by this module's matcher/tests.
- Targeted cancellation ownership, actor mutation serialization, and shared route resolution are recommended over a multiplexer rewrite: they address demonstrated failures while retaining the existing bounded mailbox, actor ownership, and persistence architecture. No implementation was performed.
