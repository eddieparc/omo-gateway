# Aggregate: agg-runtime

## Coverage
- runtime-main: 27 findings; read in full.
- multiplexer: 29 findings; read in full.
- agent-backend: 23 findings; read in full.
- agent-workspace: 13 findings; read in full.
- voice: 12 findings; read in full.
- models-ledger: 14 findings; read in full.

Reviewed 118 source findings; retained 109: **P0: 1, P1: 90, P2: 18**. Demoted 37 retained findings, promoted 5, and dropped 9. No duplicate primary locations occurred across lanes; distinct implementations of the same defect remain separate and are linked below.

Method: chose source/caller verification over simply merging lane labels or executing the gateway. All primary citations were checked individually with `sed -n '<LINE>p' /Users/indo/code/project/omon-gateway/<path>`, then relevant source surroundings and callers were inspected. All 118 primary citations resolve; dropped findings fail substantive verification, not line resolution. No build, runtime reproduction, or dependency fault injection was performed. P1 protocol findings describe concrete conditional peer inputs, not proof that the installed daemon emits them. Public-helper behavior without a demonstrated production ingress is not treated as a P0 authorization bypass. Voice-library findings do not establish an active gateway voice-capture integration. Only the six assigned reports and source needed to check their claims were read; tools-exec and cron-scheduler reports are outside this aggregate's assigned scope.

## Findings

### [P0] Explicit reset restores the remote conversation binding it removed  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/actor.rs:758
- What: The flush query restores an existing database thread binding whenever reset supplies state without that binding.
- Trigger: A user resets an idle session with a persisted `metadata.omo_thread_id`, then the actor is reloaded after collection or restart.
- Impact: The acknowledged reset does not sever conversation continuity; subsequent turns recover pre-reset remote context, silently corrupting the requested session state.
- Fix: Give reset an explicit binding-deletion write and invalidate the backend binding cache; do not merge the old turn's binding after active reset.

### [P1] Malformed channel ACL configuration silently broadens access  (from lane: runtime-main, originally P0)
- Location: src/main.rs:939
- What: Invalid channel IDs are discarded, potentially converting a supplied restriction into an empty list.
- Trigger: An operator supplies `DISCORD_ALLOWED_CHANNELS=123;456`, or a malformed ignored-channel entry, and an otherwise authorized user sends a message outside the intended scope.
- Impact: Intended channel restrictions are not enforced; this requires malformed privileged configuration, not an independently demonstrated attacker-controlled configuration path.
- Fix: Parse security-relevant lists as a Result and reject every malformed nonempty component before startup.

### [P1] Gateway recovery consumes pending intent before routing succeeds  (from lane: runtime-main, originally P0)
- Location: src/main.rs:449
- What: Recovery clears its durable retry marker before dispatching the unfinished turn.
- Trigger: Actor startup/routing fails, or the process exits after clearing the marker but before durable completion.
- Impact: A subsequent restart no longer selects that unfinished turn through pending-session recovery.
- Fix: Retain durable pending/claimed state through terminal completion; release claims on failure instead of consuming the retry marker before handoff.

### [P1] Optional dashboard controls separate runtime handles  (from lane: runtime-main, originally P0)
- Location: src/entry.rs:87
- What: The optional dashboard constructs its own multiplexer, approval guard, scheduler, and dispatcher instead of attaching to the gateway's instances.
- Trigger: Dashboard mode is enabled alongside Discord and an operator stops a live Discord turn, answers its approval, or manually executes cron through the dashboard.
- Impact: Live actor and approval operations target different in-memory owners, and manual cron uses dashboard egress rather than gateway egress.
- Fix: Construct the runtime once and inject its actual handles into an attached dashboard; reserve standalone construction for dashboard-only commands.

### [P1] User matching bypasses cross-bot mirror ambiguity checks  (from lane: runtime-main, originally P0)
- Location: src/mirror.rs:120
- What: A matching user causes an early return before the lookup verifies bot uniqueness.
- Trigger: Two bots have sessions for the same user and origin, while a mirror request supplies the user but omits `bot_id`.
- Impact: The newest candidate can receive another bot's transcript content.
- Fix: Check bot uniqueness in the user-filtered candidate set before choosing a session in either lookup branch.

### [P1] Production startup never acquires the runtime ownership lock  (from lane: runtime-main, originally P1)
- Location: src/entry.rs:81
- What: Gateway entry bypasses the ownership-lock helper, whose acquisition calls occur only in tests.
- Trigger: A duplicate launch or overlapping deployment starts two processes against the same workspace/database and Discord identities.
- Impact: Independent consumers and schedulers can execute the same work concurrently.
- Fix: Acquire and retain a lock keyed by the actual shared runtime identity before starting gateway subsystems.

### [P1] Dashboard failure drops the running gateway future  (from lane: runtime-main, originally P1)
- Location: src/entry.rs:98
- What: Dashboard completion returns from the outer select without driving gateway cleanup.
- Trigger: The optional dashboard fails to bind its occupied HTTP port after concurrent gateway initialization has begun.
- Impact: Recovery or scheduled work may be abandoned without the normal persistence and shutdown sequence.
- Fix: Send shared cancellation to the gateway and await its cleanup before returning the original dashboard error.

### [P1] Startup admits work before fallible initialization is complete  (from lane: runtime-main, originally P1)
- Location: src/main.rs:765
- What: Recovery runs before cron configuration validation, and scheduler startup precedes additional fallible dependency construction.
- Trigger: Cron configuration is invalid, or later pairing-cache/downloader/client initialization fails after replay has started.
- Impact: Startup exits after external side effects without the common shutdown path.
- Fix: Validate configuration and construct fallible dependencies before admitting work; route subsequent errors through shared cancellation and joins.

### [P1] Workspace creation errors are discarded  (from lane: runtime-main, originally P1)
- Location: src/main.rs:158
- What: Configuration ignores failure to create the workspace directory.
- Trigger: The workspace path is a file, unwritable, or on a full filesystem.
- Impact: Database/daemon startup can proceed with an unusable workspace while losing the original actionable error.
- Fix: Propagate directory-creation failure with its path before starting dependent subsystems.

### [P1] SIGTERM bypasses gateway graceful shutdown  (from lane: runtime-main, originally P1)
- Location: src/main.rs:885
- What: The gateway subscribes only to Ctrl+C and only after initialization.
- Trigger: A service manager sends SIGTERM while a turn is active, or SIGINT arrives before signal registration.
- Impact: Pending-session marking and orderly subsystem shutdown are bypassed; interrupted work is less recoverable.
- Fix: Register Unix SIGTERM and SIGINT at entry and carry one cancellation signal through startup and cleanup.

### [P1] Drain watcher is created late and not attached to ingress  (from lane: runtime-main, originally P1)
- Location: src/main.rs:876
- What: Production starts the watcher after work admission and never attaches its receiver to the multiplexer.
- Trigger: A drain marker exists at startup, or a new turn arrives while drain shutdown is persisting its session snapshot.
- Impact: Recovery/cron can run despite the marker, and newly accepted turns can fall outside the recovery snapshot.
- Fix: Scan and attach the drain receiver before recovery or ingress starts; close admissions before checkpointing sessions.

### [P1] Shutdown ignores failed resume-marker persistence  (from lane: runtime-main, originally P1)
- Location: src/main.rs:888
- What: Signal and marker shutdown branches discard errors while marking sessions pending.
- Trigger: SQLite is busy, full, or unavailable during the marking pass.
- Impact: Shutdown can report success while interrupted sessions lack durable recovery markers.
- Fix: Preserve a failure shutdown result, report affected persistence errors, and apply a bounded retry before terminating work.

### [P1] Database closure is not ordered after interactive actor shutdown  (from lane: runtime-main, originally P1)
- Location: src/main.rs:906
- What: Cleanup closes SQLite without cancelling/draining and joining interactive actors.
- Trigger: An interactive turn remains active when shutdown reaches pool closure.
- Impact: Late transcript/state writes fail or runtime teardown drops the turn; preceding unbounded joins can also stall termination.
- Fix: Close admissions, checkpoint or drain actors, join them with an overall deadline, and close SQLite last.

### [P1] Discord task failure becomes successful process exit  (from lane: runtime-main, originally P1)
- Location: src/main.rs:881
- What: The first client completion leaves the select, logging ordinary errors and ignoring JoinError without retaining failure status.
- Trigger: One client returns an error or its task panics while other clients/turns remain active.
- Impact: All bots stop without signal-branch checkpointing, and the supervisor receives a successful gateway result.
- Fix: Match all task outcomes, retain the original error, and run common shutdown before returning failure.

### [P1] A cancelled drain update still exits the gateway  (from lane: runtime-main, originally P1)
- Location: src/main.rs:893
- What: The one-shot select exits even when the observed drain value is false or its channel closes.
- Trigger: Marker creation and removal coalesce before main polls the watch update, leaving the current value false.
- Impact: Scheduler and pool shutdown proceed without the branch's checkpoint and shard-shutdown operations.
- Fix: Loop over false updates and report channel closure separately; leave the loop only for an actual drain or another termination cause.

### [P1] Unreadable drain markers are treated as absent  (from lane: runtime-main, originally P1)
- Location: src/drain_control.rs:217
- What: Marker read failures become None and can reset a previously active drain state.
- Trigger: An existing marker becomes unreadable, contains invalid UTF-8, or encounters a transient filesystem error.
- Impact: Drain requests are suppressed or cancelled without a diagnostic.
- Fix: Distinguish NotFound from read failure, log the path/error, and retain the prior drain state on read failure.

### [P1] Drain scanning blocks async workers on filesystem I/O  (from lane: runtime-main, originally P1)
- Location: src/drain_control.rs:250
- What: The async watcher invokes synchronous marker existence and read operations.
- Trigger: The workspace resides on a stalled or slow mount during a scan.
- Impact: A Tokio worker blocks and delays unrelated work; async cancellation cannot interrupt the synchronous operation.
- Fix: Use asynchronous reads or bounded blocking execution, and do not require a stalled probe to finish before shutdown proceeds.

### [P1] Failed marker publication leaks temporary files  (from lane: runtime-main, originally P1)
- Location: src/drain_control.rs:182
- What: Write/rename failure leaves a UUID-named marker temporary file without cleanup.
- Trigger: A directory occupies the final marker path and an operator/controller retries publication.
- Impact: Failed retries accumulate files in the state directory; no realistic disk-exhaustion rate was established for P0.
- Fix: Use a temporary-file cleanup guard that removes the temporary file on both write and rename errors.

### [P1] Readiness probes the wrong backend setting and default port  (from lane: runtime-main, originally P1)
- Location: src/readiness.rs:286
- What: Readiness reads `OMON_APPSERVER_URL` and defaults to 18800 instead of the resolved OMO backend endpoint on 19742.
- Trigger: A default deployment uses the healthy daemon on 19742, or only `OMON_OMO_APPSERVER_URL` is configured.
- Impact: Backend health is falsely degraded, or an unrelated service makes the check falsely pass.
- Fix: Pass resolved interactive and distinct cron backend URLs into readiness instead of reparsing another variable.

### [P1] Configured bot count masquerades as connected health  (from lane: runtime-main, originally P1)
- Location: src/readiness.rs:228
- What: Any positive configured client count is labeled healthy with a `connected_bots` metric.
- Trigger: Main collects readiness before starting the clients, or configured clients never connect.
- Impact: Readiness cannot distinguish configured tokens from operational gateway connections.
- Fix: Keep configuration count separately named and derive connection health from actual shard ready/disconnected state.

### [P1] Credential readiness accepts empty unrelated keys  (from lane: runtime-main, originally P1)
- Location: src/readiness.rs:193
- What: Provider readiness accepts mere existence of any recognized API-key variable.
- Trigger: The model is Claude but the only key is `OPENAI_API_KEY=`.
- Impact: The report asserts usable credentials without a usable credential for the resolved provider; daemon-stored credentials can produce the inverse error.
- Fix: Probe authentication at the actual daemon/provider route rather than inferring it from unrelated local variables.

### [P1] Readiness timeout is hidden by a liveness fallback  (from lane: runtime-main, originally P1)
- Location: src/readiness.rs:261
- What: A readiness transport error falls back to `/health` and may become a healthy result.
- Trigger: `/readyz` times out under overload while `/health` remains responsive.
- Impact: Dependency unavailability is misreported as readiness.
- Fix: Fall back only for explicit unsupported readiness endpoints such as HTTP 404; preserve timeout failures.

### [P1] Mirroring ignores failed session-recency updates  (from lane: runtime-main, originally P1)
- Location: src/mirror.rs:51
- What: Transcript insertion and recency update are separate operations, and the second error is discarded.
- Trigger: SQLite fails or the pool closes after insertion but before updating `updated_at`.
- Impact: Mirroring reports success with stale ordering used by later origin lookups.
- Fix: Commit insertion and recency update in one transaction and propagate either error.

### [P1] Mirror fallback is decided before the requested bot is filtered  (from lane: runtime-main, originally P1)
- Location: src/mirror.rs:147
- What: Any unthreaded row suppresses channel-wide fallback even when it belongs to another bot.
- Trigger: Bot B has an unthreaded row while explicitly requested bot A has only threaded rows in the channel.
- Impact: Lookup returns None without examining A's otherwise eligible candidates.
- Fix: Apply bot constraints before testing whether the preferred unthreaded candidate set is empty.

### [P1] Mirror lookup materializes all historical candidates  (from lane: runtime-main, originally P1)
- Location: src/mirror.rs:93
- What: Origin lookup fetches all matching sessions before filtering bot/user identity in memory.
- Trigger: A long-lived origin accumulates many sessions and receives repeated mirror deliveries.
- Impact: Per-delivery memory and query/processing cost grow with retained history; realistic OOM was not demonstrated.
- Fix: Push exact constraints into SQL and fetch a bounded candidate set sufficient for selection and ambiguity detection.

### [P1] DashMap guards span actor and database awaits  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/router.rs:413
- What: Model/reset/context operations and the resume-marker iterator retain synchronous shard guards across awaits.
- Trigger: While an actor reply or SQLite operation is pending, enough insert/remove operations contend on the held shards to block runtime workers.
- Impact: Worker starvation can prevent the awaited continuation and stall the service; the production multi-worker hang needs contention, not just one ordinary lookup.
- Fix: Clone Arc handles and snapshot owned keys before any await so DashMap guards are dropped first.

### [P1] Cancelled backpressured sends leak the in-flight count  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/router.rs:179
- What: Cancellation during mailbox send bypasses the decrement after incrementing `in_flight`.
- Trigger: A full mailbox suspends send and its caller times out or is aborted before send resumes.
- Impact: GC waits forever for a phantom send, stranding that session and delaying sequential collection.
- Fix: Install an RAII decrement/notification guard immediately after incrementing the counter.

### [P1] Cancelled GC can strand a session in retirement  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/gc.rs:61
- What: The retirement protocol lacks cancellation ownership after setting `accepting=false`.
- Trigger: A caller drops collection after eviction is enqueued, then the busy actor replies false with no collector left to resume it.
- Impact: Later routes wait indefinitely and later collectors skip the retired handle.
- Fix: Give the retirement transaction an owned task that completes eviction/reply handling; restore acceptance only when pre-enqueue cancellation is certain.

### [P1] An unresponsive actor blocks GC and its shutdown  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/gc.rs:24
- What: The timer branch awaits sequential unbounded collection without polling shutdown.
- Trigger: An actor is stuck in dispatcher, database, or cancellation work when GC requests eviction.
- Impact: Later sessions are not collected and `ScaleToZero::shutdown` cannot finish its join.
- Fix: Bound per-session collection and use bounded concurrency with cancellation-safe retirement ownership.

### [P1] Transcript deduplication falsely acknowledges unfinished turns  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/actor.rs:236
- What: Existing inbound transcript presence is treated as proof of successful execution.
- Trigger: A persisted turn fails, is stopped, or crashes, then backfill replays its nonempty platform message ID.
- Impact: Replay is skipped and acknowledged successfully even though the prior turn never completed.
- Fix: Deduplicate on a durable successful terminal outcome, reusing the existing inbound row for unfinished retries.

### [P1] Pending overflow drops already accepted plain events  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/actor.rs:309
- What: The actor drops events when its secondary queue is full after route has already returned mailbox acceptance.
- Trigger: A blocked turn receives more than 64 pending plain events with no delivery acknowledgement ID.
- Impact: Accepted conversation turns are lost with only a warning; this requires burst/sustained load.
- Fix: Reserve pending capacity before reporting admission or return an explicit admission acknowledgement for every event.

### [P1] Biased command polling can starve backend completion  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:300
- What: Commands always win the biased select when both mailbox and backend are ready.
- Trigger: Sustained traffic to one session keeps its mailbox continuously ready.
- Impact: Backend progress/completion is postponed and pending-queue overflow increases.
- Fix: Remove unconditional bias or enforce a finite command batch before polling the backend.

### [P1] Turn completion overwrites an acknowledged concurrent model change  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/actor.rs:424
- What: Successful completion replaces actor state with the pre-change runner clone.
- Trigger: SetModel is acknowledged while a turn is running, and that old turn subsequently succeeds.
- Impact: The new model selection is silently overwritten in memory and in the following flush.
- Fix: Merge actor-owned model changes after completion or defer model acknowledgements until serialized application at the turn boundary.

### [P1] Model persistence replaces unrelated state from a stale snapshot  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/router.rs:406
- What: SetModel reads and later replaces all state JSON without version coordination.
- Trigger: An actor persists a new binding or suspension between the router's read and write, then retires before its model command can repair state.
- Impact: Unrelated durable fields are overwritten by the stale snapshot.
- Fix: Let live actors own mutations and use a coordinated atomic JSON-field update for absent actors.

### [P1] Model selection before session creation is a successful no-op  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/router.rs:411
- What: SetModel ignores zero affected rows from its UPDATE and returns success without an actor.
- Trigger: Model selection targets a fresh session key with neither a persisted row nor a live actor.
- Impact: The next event uses defaults instead of the acknowledged model.
- Fix: Create/load the actor or upsert session state, and do not acknowledge a zero-row mutation as applied.

### [P1] Reset is lost for collected or retiring sessions  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/router.rs:420
- What: Reset has no storage fallback and discards live-handle/reset outcomes.
- Trigger: Reset targets a GC-collected session or races with its retirement.
- Impact: The caller receives success while the next event reloads unchanged state.
- Fix: Use retirement-aware retry and propagate reset errors; load/reset persisted state when no actor exists.

### [P1] Stop acknowledges suspension without successful persistence  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:567
- What: Stop ignores suspension flush errors before replying successfully.
- Trigger: SQLite fails during Stop and the process exits before a later successful flush.
- Impact: Restart can recover a session that the user was told had stopped.
- Fix: Return flush failures in the stop result and retain the dirty/recovery obligation until suspension is durable.

### [P1] Failure handling clears dirty state despite surviving mutations  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:447
- What: Error paths unconditionally clear the dirty flag after retaining state changes.
- Trigger: A failed runner leaves a binding mutation, or inbound persistence fails after an earlier unsuccessful state flush.
- Impact: GC/shutdown can skip persistence of retained dirty state.
- Fix: Preserve the prior dirty obligation and mark surviving mutations dirty; roll back only genuinely discarded turn-local state.

### [P1] Actor recovery clears retry intent before recoverable work  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/actor.rs:88
- What: The alternate recovery implementation clears pending state before querying and routing unfinished work.
- Trigger: A later SQLite query or route fails, draining refuses admission, or the process crashes after the clear.
- Impact: The unfinished session disappears from subsequent pending recovery attempts.
- Fix: Retain a durable claim through acknowledged terminal completion and release it on failure rather than consuming the marker up front.

### [P1] Acknowledged routing bypasses the drain gate  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/router.rs:360
- What: `route_awaiting_turn` does not apply the drain check used by plain route.
- Trigger: Backfill or another acknowledged caller submits while an attached drain receiver is already true.
- Impact: New turns start after this API should refuse admissions.
- Fix: Share the admission check across both APIs and repeat it after retirement waits.

### [P1] Idle age includes the last active turn's duration  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:595
- What: Ordinary turn completion does not refresh the monotonic activity timestamp used for eviction.
- Trigger: A turn lasts longer than idle_timeout without heartbeat and GC checks it immediately after completion.
- Impact: A just-used actor is evicted without its configured idle grace, creating avoidable reload churn.
- Fix: Refresh `last_active_at` at every terminal turn outcome.

### [P1] Guild fallback preempts parent-channel profile selection  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/profile_routing.rs:302
- What: The direct-channel stage accepts guild-only/catch-all matches before parent-channel lookup.
- Trigger: A thread in channel 300 with parent 200 matches both a parent-200 route and a guild-100 fallback.
- Impact: The guild profile's model/prompt/toolsets win instead of the intended parent profile; no production authorization bypass is inferred.
- Fix: Require an explicit channel target in the direct stage, evaluate the parent next, and defer guild/catch-all rules.

### [P1] Actor initialization ignores parent-aware event routing  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:858
- What: Actor initialization resolves profiles from SessionKey without the event's parent-channel metadata.
- Trigger: An event represents its thread as channel_id and supplies its parent only in `parent_chat_id` metadata.
- Impact: A parent-only profile is not applied to the actual actor even though `match_event` can resolve it.
- Fix: Carry initial event routing context into actor creation and preserve the routing identity needed for reload.

### [P1] Negative route identifiers broaden execution matching  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/profile_routing.rs:27
- What: Negative guild/channel/thread IDs deserialize to None, removing their constraints.
- Trigger: Privileged configuration supplies a route such as `{"channel":-1,"model":"other"}`.
- Impact: The malformed targeted profile can become a wildcard; the unused authorization helper does not establish an exploitable production bypass.
- Fix: Reject negative identifiers at deserialization and retain the last valid configuration on reload failure.

### [P1] One malformed profile entry discards the entire configured route set  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/profile_routing.rs:402
- What: A route-array parse error becomes an empty set instead of configuration rejection.
- Trigger: One configured route contains a nonnumeric channel string alongside valid profile overrides.
- Impact: All overrides disappear and execution falls back to defaults; the claimed live access-control bypass was not demonstrated.
- Fix: Return a parse Result and fail startup/reload or retain the previous valid route set.

### [P1] Equivalent channel spellings select prompts nondeterministically  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/profile_routing.rs:460
- What: Stable sorting preserves randomized HashMap order among keys that normalize to the same numeric ID.
- Trigger: Prompt configuration contains both `"123"` and `"0123"` with different settings.
- Impact: Identical configuration can choose different prompts/toolsets across starts.
- Fix: Reject duplicate normalized channel IDs before constructing routes.

### [P1] Bot-profile query errors silently select defaults  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/actor.rs:793
- What: Actor loading treats bot-profile query failure like an absent row.
- Trigger: Session lookup succeeds but the following bot-profile lookup fails because of SQLite/pool availability.
- Impact: An actor runs with fallback model/prompt/toolsets and may persist those defaults.
- Fix: Propagate database errors and use defaults only for a successfully absent profile.

### [P1] Restart-breaker persistence errors disable its threshold  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/restart_loop_guard.rs:75
- What: Restart history read/write/rename errors are discarded.
- Trigger: The state directory is unwritable or full while repeated crashing resumes occur.
- Impact: Each process sees missing or stale history and can repeatedly resume instead of tripping the breaker.
- Fix: Return and report persistence failures and suppress automatic resume when safety history cannot be maintained.

### [P1] Future restart timestamps survive backward clock jumps  (from lane: multiplexer, originally P1)
- Location: src/multiplexer/restart_loop_guard.rs:89
- What: Future timestamps are skipped for gap comparison but retained in the counted chain.
- Trigger: Persisted boots are `[1000,1010,1020]` and the wall clock moves back to 0.
- Impact: The restart breaker can block legitimate recovery based on future history.
- Fix: Filter/reset anomalous future entries under an explicit clock-skew policy before chain counting and retention.

### [P1] Deadline drain can finalize another turn's output  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:638
- What: Deadline cleanup accumulates uncorrelated content and its success branch does not require `terminal_confirmed`.
- Trigger: The turn reaches its deadline and the peer sends another thread/turn's deltas or completed terminal during cleanup.
- Impact: Unrelated content can be delivered/persisted and cron acknowledged while the actual turn remains unresolved.
- Fix: Apply thread/turn correlation before every drain mutation and require a correlated completed terminal for success.

### [P1] Completed message items erase earlier same-turn content  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:963
- What: An item completion replaces the entire turn-wide text rather than that item's text.
- Trigger: One daemon turn emits distinct agent-message items A and B and completes B after A.
- Impact: Final output and transcript omit A; upstream multi-item emission frequency was not established as routine production input.
- Fix: Track ordered text by item ID and replace only the completed item's snapshot, including during deadline drain.

### [P1] Interim streaming bypasses final suppression and filtering  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:978
- What: Raw cumulative deltas are dispatched before direct-emission suppression or reasoning/silence filtering is applied.
- Trigger: A cron turn sets `cron_suppress_direct_emission=true`, or streamed text contains a prefix removed only at finalization.
- Impact: The backend emits stream actions contrary to final policy; downstream visibility and an authorization boundary breach are not established here.
- Fix: Gate every emission on suppression and buffer/filter undecidable prefixes before dispatch.

### [P1] Notifications before the start acknowledgement are discarded  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:921
- What: Turn-bearing notifications received before the start response are dropped rather than correlated later.
- Trigger: A peer emits current-turn deltas or its only terminal before replying to request ID 3.
- Impact: Output is incomplete or a finished turn waits until timeout.
- Fix: Buffer a bounded set of pre-ACK notifications and replay only those matching the acknowledged thread/turn.

### [P1] Turn output accumulation is uncapped and copies every prefix  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:972
- What: Text and item tracking have no aggregate quota, while each delta clones the growing text.
- Trigger: A fast peer emits many small deltas or many unique item IDs during long/concurrent turns.
- Impact: Sustained load causes large allocations and quadratic copying; a realistic normal-uptime exhaustion bound was not demonstrated.
- Fix: Enforce per-turn text/item limits and coalesce updates, interrupting with an explicit overflow error.

### [P1] Backend thread cache retains completed session identities indefinitely  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:414
- What: New non-cron sessions add bindings to a backend-wide map without normal retirement eviction.
- Trigger: A long-lived backend handles sustained creation of distinct session keys.
- Impact: Retained memory grows after actors finish/are collected; the source does not establish an OOM rate within realistic uptime.
- Fix: Bound or remove the duplicate fallback cache, or evict entries on session retirement/reset.

### [P1] Approval handling precedes thread and turn ownership checks  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_backend.rs:880
- What: Reverse approval requests use the current session's policy without first validating request ownership.
- Trigger: A peer sends an approval for another thread/turn on a YOLO session's connection, including before start acknowledgement.
- Impact: The client can approve misattributed work; no ordinary unprivileged actor's ability to inject those frames was demonstrated.
- Fix: Correlate ownership before approval policy evaluation and reject or bounded-buffer unattributable pre-ACK requests.

### [P1] Daemon readiness buffers the entire HTTP response  (from lane: agent-backend, originally P0)
- Location: src/agent/omo_daemon.rs:54
- What: A status-only probe reads to EOF into an uncapped Vec.
- Trigger: A faulty or substituted local endpoint streams a large body within the two-second probe window.
- Impact: Large allocations can exhaust memory under adverse dependency behavior; normal readiness responses do not establish a P0 exhaustion path.
- Fix: Read and parse only a bounded status line/header and stop without consuming the body.

### [P1] Turn deadlines do not bound writes and downstream awaits  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:508
- What: Socket writes and several dispatcher/database/finalization awaits sit outside an enforced turn-wide timeout.
- Trigger: The peer stops reading a large start/interrupt write, or downstream dispatch stalls.
- Impact: Deadline checks cannot run, so turn completion and reserved cleanup can hang past their budgets.
- Fix: Bound external awaits by work/cleanup deadlines and separately bound finalization while retaining ambiguous remote ownership.

### [P1] Ping or pong ends interrupt cleanup prematurely  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:588
- What: The cleanup while-let terminates on any non-Text WebSocket frame.
- Trigger: Ping/pong arrives before the matching interrupt response or terminal during deadline cleanup.
- Impact: A live connection's remaining cleanup budget is abandoned and remote ownership stays unresolved.
- Fix: Match frame kinds inside the loop, continuing control frames and ending only for correlated completion, close/error, or deadline.

### [P1] Fast empty terminal notifications are forgotten  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:1034
- What: A correlated empty completed terminal is discarded during the grace interval without deferred reconsideration.
- Trigger: A genuine empty turn completes inside `no_content_grace` and the peer sends no later terminal.
- Impact: The gateway times out/interrupts an already finished turn and retains ownership unnecessarily.
- Fix: Finalize correlated terminals immediately or retain and reconsider the suspected premature terminal at a bounded deadline.

### [P1] Setup treats reverse requests as responses when IDs collide  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:167
- What: Initialize/resume setup accepts matching numeric IDs without requiring a response envelope.
- Trigger: A server-to-client request uses ID 1 or 2 while the corresponding client setup request is outstanding.
- Impact: Setup advances or persists a binding without receiving a valid response.
- Fix: Require no method, the expected ID, exactly one result/error member, and a valid method-specific result shape.

### [P1] Cooperative cancellation hides remote interrupt failure  (from lane: agent-backend, originally P1)
- Location: src/agent/backend.rs:47
- What: The default cancellation wrapper discards the backend cancellation Result.
- Trigger: Cancellation occurs while start is unacknowledged or interrupt is rejected, disconnected, or times out.
- Impact: Callers receive only local cancellation without knowing the remote turn remains unresolved.
- Fix: Preserve cancellation cleanup errors in the returned context and distinguish local cancellation from confirmed remote interruption.

### [P1] Approval denial count does not enforce a per-turn limit  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:870
- What: Disabled-terminal denials bypass counting and item lifecycle events reset the count.
- Trigger: A peer repeatedly requests a disabled terminal, or interleaves other denied requests with item-start/completion events.
- Impact: The intended loop limit never trips and the turn consumes its remaining total budget.
- Fix: Count every policy denial cumulatively and reset only for a genuinely new turn.

### [P1] Approval response write failures are ignored  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:902
- What: Allow and deny sends discard their transport Result and continue processing input.
- Trigger: The WebSocket fails while the gateway answers a pending approval.
- Impact: The immediate failure is lost and the peer may wait for a response until timeout.
- Fix: Propagate the send failure with request/session context while retaining unresolved ownership for safe cleanup.

### [P1] Unsupported reverse requests receive no JSON-RPC response  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:856
- What: Inbound requests are answered only if their method matches the approval heuristic.
- Trigger: The daemon sends an unsupported request with both id and method and waits for its reply.
- Impact: The turn can stall until timeout instead of receiving an explicit unsupported-method error.
- Fix: Classify requests separately from notifications and return correlated JSON-RPC -32601 for unsupported methods.

### [P1] Readiness requires EOF and accepts malformed status prefixes  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:56
- What: HTTP status is prefix-matched only after the full connection has closed.
- Trigger: A complete 200 response leaves the connection open beyond the probe budget, or the peer sends `HTTP/1.1 2000`.
- Impact: A ready daemon is rejected or a malformed response is accepted, distorting spawn/restart decisions.
- Fix: Parse an exact three-digit status from bounded headers without waiting for body EOF.

### [P1] Portless WebSocket URLs cannot pass local readiness  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:51
- What: Raw TCP connection uses the URL authority without adding WebSocket's default port.
- Trigger: Local configuration is `ws://localhost` or `ws://127.0.0.1/path` with a service on port 80.
- Impact: Readiness fails despite a URL the WebSocket client can interpret, causing unsuccessful spawn attempts.
- Fix: Parse the URL once and use its host and explicit/default port consistently across probing and spawning.

### [P1] Respawn bypasses the initial daemon spawn lock  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:348
- What: The watcher spawns replacements outside the mutex used by ensure.
- Trigger: Ensure for an endpoint overlaps its supervisor's restart after both observe the port unready.
- Impact: Duplicate children race to bind and readiness can be attributed to the wrong child.
- Fix: Acquire the same endpoint-scoped lock and re-probe immediately before installing a replacement.

### [P1] Daemon shutdown does not own descendant processes  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:397
- What: Managed cleanup targets only the direct Child without an owned process group/tree.
- Trigger: The configured daemon wrapper forks a server or leaves long-lived tool subprocesses before shutdown.
- Impact: Descendants can survive outside supervisor ownership; kill-on-drop covers only the direct child.
- Fix: Start managed daemons in an owned process group/job and terminate/reap that group during cleanup.

### [P1] Daemon setup performs blocking filesystem work on async workers  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:180
- What: Async ensure/watcher paths synchronously resolve binaries and create/open log files.
- Trigger: HOME/PATH candidates or log directories reside on a stalled filesystem.
- Impact: A runtime worker blocks, and ensure can hold the global spawn mutex throughout the stall.
- Fix: Prepare filesystem-dependent command resources using asynchronous or bounded blocking work before the spawn ownership transition.

### [P1] Failed daemon log setup silently discards both output streams  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_daemon.rs:195
- What: Log creation/open/clone failures fall through to null stdout/stderr without reporting the cause.
- Trigger: HOME is unwritable, disk is full, or file descriptors are exhausted.
- Impact: Repeated daemon startup failures lose their diagnostics and surface only as readiness errors.
- Fix: Report log setup failure and fall back to an observable sink such as inherited stderr.

### [P1] Fixed reconnect cadence synchronizes outage load  (from lane: agent-backend, originally P1)
- Location: src/agent/omo_backend.rs:89
- What: Concurrent turns retry failing connections every 500 ms without jitter or a shared gate.
- Trigger: A burst of turns overlaps a daemon outage or recovery.
- Impact: Synchronized retry traffic adds load to the recovering dependency and retries permanent handshake failures too.
- Fix: Apply capped exponential backoff with jitter, classify permanent failures, and share endpoint reconnect admission.

### [P1] Per-chunk lossy UTF-8 decoding corrupts split characters  (from lane: agent-workspace, originally P1)
- Location: src/agent/llm.rs:245
- What: Raw stream byte chunks are independently converted with `from_utf8_lossy` before line framing.
- Trigger: The residual LlmClient receives a multibyte character split between HTTP byte chunks.
- Impact: Replacement characters permanently corrupt streamed text.
- Fix: Accumulate bytes and decode only complete newline-delimited UTF-8 records, retaining incomplete byte sequences.

### [P1] Mid-stream SSE error objects are reported as successful completion  (from lane: agent-workspace, originally P1)
- Location: src/agent/llm.rs:250
- What: The SSE parser ignores provider error objects and EOF still produces an Ok final chunk.
- Trigger: A provider emits an OpenAI error object or Anthropic error event after streaming has begun.
- Impact: An aborted generation appears successful with missing or partial output.
- Fix: Parse typed error events and forward an error to both stream and tool-call result consumers.

### [P1] Stalled LLM streams retain background tasks without timeout  (from lane: agent-workspace, originally P1)
- Location: src/agent/llm.rs:152
- What: The HTTP client and spawned byte-stream reader lack explicit request/read deadlines and cancellation handling.
- Trigger: A provider stalls mid-stream without closing TCP, including after its stream consumer is dropped.
- Impact: The background task and connection can remain indefinitely; retries alone would not fix this.
- Fix: Configure connection/read budgets and select the reader against consumer closure or cancellation.

### [P1] Image payload construction synchronously reads files on Tokio workers  (from lane: agent-workspace, originally P1)
- Location: src/agent/llm.rs:461
- What: Async stream setup calls synchronous attachment reads during payload construction.
- Trigger: A large image or slow filesystem is read while constructing an LLM request.
- Impact: A runtime worker stalls and unrelated async work suffers latency.
- Fix: Read attachment bytes asynchronously before building payloads or offload file preparation to bounded blocking work.

### [P1] Zero-valued configured turn timeouts are accepted  (from lane: agent-workspace, originally P2)
- Location: src/agent/omo_config.rs:151
- What: Environment timeout parsing accepts zero despite requiring a positive timeout operationally.
- Trigger: An operator sets `OMON_OMO_TURN_TIMEOUT_SECS=0` or an interactive/cron total-timeout variable to zero.
- Impact: Turns immediately hit their gap/total deadline rather than configuration failing at boot.
- Fix: Reject zero at environment parsing and report the offending variable before startup side effects.

### [P1] Numeric non-version URL suffixes select the wrong endpoint  (from lane: agent-workspace, originally P2)
- Location: src/agent/llm.rs:94
- What: The endpoint helper treats any all-digit last segment as a version prefix.
- Trigger: An OpenAI-compatible proxy uses a base such as `http://proxy/models/42` and expects the normal `/v1/chat/completions` suffix.
- Impact: The client appends only `/chat/completions` and requests the wrong endpoint.
- Fix: Require an actual supported version prefix before recognizing a version segment, and trim configured base URL whitespace.

### [P1] Multi-frame transcription submits only the first frame  (from lane: voice, originally P0)
- Location: src/voice/pipeline.rs:149
- What: The transcriber accepts a frame slice but encodes only its first frame.
- Trigger: A library caller supplies an utterance containing two or more PCM frames to SpeechPipeline/transcribe.
- Impact: Audio after the first frame is omitted from STT; no production multi-frame voice-capture caller was found to justify P0.
- Fix: Validate matching PCM formats and concatenate all supplied frames into the WAV payload.

### [P1] Dropped audio receivers leave handlers attempting failed sends  (from lane: voice, originally P1)
- Location: src/voice/mod.rs:49
- What: Send failure breaks only the current tick loop and the handler returns no cancellation request.
- Trigger: An attached listener's receiver is dropped while Songbird continues delivering events.
- Impact: Later callbacks continue allocating/copying frames and attempting sends to a closed channel for the handler's remaining lifetime.
- Fix: Request handler cancellation on closed-channel send failure rather than returning the keep-listening result.

### [P1] Audio callback waits indefinitely under channel backpressure  (from lane: voice, originally P1)
- Location: src/voice/mod.rs:70
- What: Event handling awaits a bounded channel send without a backpressure policy.
- Trigger: The downstream consumer stops draining while the frame channel is full.
- Impact: The callback cannot return; the lane's stronger claim of a gateway-wide central event-loop stall is not established by repository source.
- Fix: Use nonblocking admission with an explicit overflow/drop metric, keeping event callbacks bounded.

### [P1] Raw Opus packets are mislabeled as an Ogg file  (from lane: voice, originally P1)
- Location: src/voice/pipeline.rs:153
- What: The transcriber sends raw Opus bytes with filename `audio.ogg` without creating an Ogg container.
- Trigger: A caller passes an AudioPayload::Opus containing a raw packet rather than an already encoded Ogg file.
- Impact: The STT endpoint receives an invalid audio file and rejects or cannot decode it.
- Fix: Decode/concatenate PCM into WAV or encapsulate the packets in a valid supported container before upload.

### [P1] STT requests have no explicit timeout  (from lane: voice, originally P2)
- Location: src/voice/pipeline.rs:141
- What: The STT client uses default request timeout behavior without a configured bound.
- Trigger: OpenAI or a configured proxy stalls without closing the connection.
- Impact: The transcription caller can remain blocked indefinitely.
- Fix: Configure explicit connection and request/read timeouts suitable for transcription.

### [P1] Empty transcription still invokes LLM and TTS  (from lane: voice, originally P2)
- Location: src/voice/pipeline.rs:258
- What: Speech processing forwards an empty transcript to language generation and synthesis.
- Trigger: SpeechPipeline receives no frames or STT returns only whitespace for silence.
- Impact: Empty intervals still invoke paid/side-effecting downstream work and can produce unsolicited audio.
- Fix: Return an empty pipeline result before LLM/TTS when the normalized transcript is empty.

### [P1] Memory search loads and ranks the entire session history  (from lane: models-ledger, originally P0)
- Location: src/memory/store.rs:106
- What: Search fetches every memory before ranking and applying the requested result limit.
- Trigger: A session accumulates a large retained memory history and searches it repeatedly or concurrently.
- Impact: Working memory and CPU grow with history despite a small result limit; hundreds/thousands of rows alone do not prove realistic OOM.
- Fix: Use indexed candidate selection or bounded pagination/top-k ranking instead of full-history materialization; avoid an arbitrary recent-row cutoff that silently changes search recall.

### [P1] Recovery claim can reclaim an obligation delivered after selection  (from lane: models-ledger, originally P1)
- Location: src/ledger/service.rs:514
- What: The claim UPDATE checks owner identity but not whether obligation state remains recoverable.
- Trigger: An eligible candidate becomes delivered between the sweeper SELECT and claim UPDATE while still matching its owner predicate.
- Impact: A completed obligation can be returned for replay, producing duplicate user-visible delivery.
- Fix: Repeat the recoverable-state predicate in the claim UPDATE and only return successfully claimed rows.

### [P1] Missing Discord prefix introduces an empty-ID lookup alternative  (from lane: models-ledger, originally P1)
- Location: src/ledger/service.rs:155
- What: Unprefixed message IDs bind an empty alternate ID in duplicate/completed/get queries.
- Trigger: The ledger contains an empty `message_id` and a later lookup uses an unprefixed nonempty ID.
- Impact: The unrelated empty row can make messages falsely duplicate/completed or return the wrong entry.
- Fix: Use `unwrap_or(message_id)` rather than the empty default in all three lookup variants.

### [P1] Constituent ledger writes discard database errors  (from lane: models-ledger, originally P1)
- Location: src/ledger/service.rs:207
- What: Parent/constituent inserts and completion updates are not atomic and several Results are ignored.
- Trigger: SQLite locking, storage, or constraint failure occurs after the parent operation succeeds.
- Impact: Grouped message IDs remain unregistered or in-progress while the parent API reports success.
- Fix: Put related parent/constituent mutations in one transaction and propagate every failure.

### [P1] Staged memory IDs cannot be addressed through the returned Memory API  (from lane: models-ledger, originally P1)
- Location: src/memory/store.rs:40
- What: Approval staging returns a Memory carrying a pending-write ID even though no memory row exists.
- Trigger: Write approval is enabled and a caller immediately passes the returned ID to get or delete.
- Impact: Lookup returns None and deletion affects no memory, hiding the staged-versus-stored distinction.
- Fix: Return a distinct Stored/Staged outcome with its appropriate identifier instead of presenting a staged write as stored Memory.

### [P1] Late completion can overwrite a delivered ledger status  (from lane: models-ledger, originally P1)
- Location: src/ledger/service.rs:291
- What: Completion unconditionally updates status and completion timestamps by message ID.
- Trigger: A late failure callback or duplicate completion runs after the message has been marked delivered.
- Impact: Delivered status and processing-latency accounting are overwritten, potentially enabling unintended retry.
- Fix: Guard allowed state transitions in SQL and make repeated terminal completion idempotent without regressing delivered state.

### [P1] Recovery mistakes another live instance's start time for stale ownership  (from lane: models-ledger, originally P2)
- Location: src/ledger/service.rs:493
- What: A live owner's stored start time is compared to the current process's start time rather than that owner's identity.
- Trigger: Two gateway processes share SQLite; one sweeps obligations owned by the other, whose start timestamp differs.
- Impact: The sweeper can steal and replay work still being delivered by the live owner.
- Fix: Do not reclaim another live PID solely for a different local start timestamp; use a verified owner identity or expiring lease/heartbeat.

### [P2] Approval timeout arithmetic lacks an upper-bound check  (from lane: runtime-main, originally P0)
- Location: src/main.rs:610
- What: Adding five seconds to an accepted u64 timeout can overflow.
- Trigger: A privileged operator sets `APPROVAL_TIMEOUT_SECS=18446744073709551615`; no routine-input crash is established.
- Impact: Checked builds panic and unchecked arithmetic wraps, making pathological configuration build-dependent.
- Fix: Reject unsupported upper values at configuration parsing and use checked addition.

### [P2] Drain watcher handle is detached rather than supervised  (from lane: runtime-main, originally P1)
- Location: src/main.rs:877
- What: Main neither polls nor aborts/awaits the watcher handle.
- Trigger: No concrete production panic source in this fixed-nonzero-interval watcher was established; ordinary shutdown leaves it alive until runtime teardown.
- Impact: Task lifecycle ownership is incomplete, but a demonstrated service failure from this alone is absent.
- Fix: Retain the handle and explicitly cancel/reap it on shutdown; supervise unexpected termination.

### [P2] Descendant cleanup test relies on an arbitrary sleep  (from lane: runtime-main, originally P2)
- Location: src/main.rs:1007
- What: The test substitutes a fixed delay for a child-exit/reaping completion signal.
- Trigger: Scheduler or reaper timing differs from the assumed 100 ms grace.
- Impact: Test results depend on timing rather than the production cleanup completion boundary.
- Fix: Establish child startup with a handshake and await bounded exit/reaping completion without fixed sleeps.

### [P2] Public GC configuration accepts a zero interval  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/gc.rs:18
- What: A zero public gc_interval reaches Tokio interval construction without validation.
- Trigger: A library caller supplies Duration::ZERO; inspected production constructors use nonzero defaults.
- Impact: The spawned collector panics, not a demonstrated routine-input process-wide crash.
- Fix: Validate the interval at the constructor boundary and expose unexpected collector task failure.

### [P2] Unused authorization helper omits guild and thread context  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/profile_routing.rs:235
- What: The helper matches routes with None for guild and thread, losing scoped restrictions.
- Trigger: A direct helper caller checks user 2 against a guild-100/channel-200 allowlist for user 1 with empty global defaults.
- Impact: The helper can return true, but source-reference verification found no production caller, so a reachable authorization bypass is not established.
- Fix: Remove the unused helper or require the full event/resolved-route context before integrating it into authorization.

### [P2] Unused bot-authorization fallback accepts disabled or unrelated routes  (from lane: multiplexer, originally P0)
- Location: src/multiplexer/profile_routing.rs:242
- What: The helper's bot-name fallback ignores enabled state and contextual route constraints.
- Trigger: A direct helper call finds a disabled bot-named rule allowing a user excluded by the global list.
- Impact: The helper's return value contradicts rule scope, but no production ingress invokes it in the inspected source.
- Fix: Remove it if unnecessary or authorize only against an enabled context-valid route or explicit bot-wide policy.

### [P2] Typing error-path test can pass without running the failing backend  (from lane: multiplexer, originally P2)
- Location: src/multiplexer/actor.rs:1402
- What: The test queues Stop immediately after Event and waits for the stop result rather than the backend error.
- Trigger: The biased mailbox branch consumes the already queued Stop before polling FailingRunner.
- Impact: Stop emits the expected typing pair and masks broken error-completion cleanup.
- Fix: Submit an acknowledged event and await its bounded terminal error before ending the actor.

### [P2] Workspace identity omits the platform namespace  (from lane: agent-workspace, originally P1)
- Location: src/agent/agent_workspace.rs:48
- What: User/bot workspace categories omit the platform except for the special web case.
- Trigger: Hypothetical integrations provide equal IDs across different platforms; no deployed cross-platform collision was established.
- Impact: The public slug function maps those identities to the same directory, a future isolation hazard rather than proven live tenant crossover.
- Fix: Include a normalized platform namespace when supporting distinct platforms, with an explicit migration for existing workspace identities.

### [P2] Derived configuration Debug exposes secret fields when formatted  (from lane: agent-workspace, originally P1)
- Location: src/agent/llm.rs:28
- What: LlmConfig and OmoBackendConfig derive Debug with raw API/auth-token fields.
- Trigger: A caller formats those configurations with Debug; an actual logging/exposure site was not established.
- Impact: Future diagnostics could disclose secrets, but the lane's claimed live plaintext leakage is unverified.
- Fix: Implement redacted Debug formatting for secret fields in both configuration types.

### [P2] Voice channel constructor accepts a panic-inducing zero capacity  (from lane: voice, originally P0)
- Location: src/voice/pipeline.rs:278
- What: The public constructor forwards capacity directly to Tokio mpsc without validating zero.
- Trigger: A library caller passes zero; no production caller supplying zero was found.
- Impact: The caller can panic, but a gateway process crash on routine input is not demonstrated.
- Fix: Require NonZeroUsize or return a configuration error for zero rather than silently changing requested capacity.

### [P2] WAV construction does not reserve its known final size  (from lane: voice, originally P2)
- Location: src/voice/pipeline.rs:155
- What: WAV output starts with an empty Vec and grows while samples are appended.
- Trigger: A large PCM frame requires repeated capacity growth.
- Impact: Avoidable allocations/copies occur; Vec's geometric growth does not establish the lane's exaggerated allocation count or a correctness failure.
- Fix: Reserve header bytes plus encoded sample bytes before appending.

### [P2] Silence regex has an incorrect trailing character class  (from lane: models-ledger, originally P1)
- Location: src/models/events.rs:69
- What: The raw regex uses a double-backslash s character class, matching literal backslash/s rather than whitespace there.
- Trigger: A direct silence check receives an odd suffix such as `(silent)s`; ordinary trailing whitespace is already removed by the caller's trim.
- Impact: The accepted silence-token set is broader than intended; the report's `(silent) ` visible-output example is contradicted by the surrounding code.
- Fix: Use a single-backslash whitespace class and test the machine-consumed sentinel classifications rather than prose wording.

### [P2] Error conversion discards typed database causes  (from lane: models-ledger, originally P1)
- Location: src/error.rs:33
- What: Database errors are converted into strings and lose their structured source chains.
- Trigger: Callers need to classify a database failure rather than display its text.
- Impact: Diagnostics and future retry classification are harder; no concrete failed retry policy was proven by this finding.
- Fix: Preserve typed/source errors and distinguish serialization failures when the API next needs structured classification.

### [P2] PID liveness helper does not reject values above signed PID range  (from lane: models-ledger, originally P1)
- Location: src/ledger/service.rs:64
- What: Casting u32 to i32 permits negative special PID values in a signal-zero liveness probe.
- Trigger: A direct caller supplies a value above i32::MAX; ordinary OS-issued process IDs do not provide this trigger.
- Impact: The probe can check a process group instead of one process; signal zero does not terminate or signal those processes.
- Fix: Reject out-of-range PID values before the cast, retaining the existing zero check.

### [P2] Root wildcard exports obscure the intended public API  (from lane: models-ledger, originally P2)
- Location: src/lib.rs:18
- What: Root glob re-exports expose new public module items without explicit API review.
- Trigger: A module adds a public item or a name colliding with another export.
- Impact: Public-surface clarity and compatibility become harder to maintain; no current runtime failure is established.
- Fix: Replace globs with deliberate export lists as API maintenance, preserving existing supported consumers.

### [P2] DeliveryReceipt duplicates an unused delivery representation  (from lane: models-ledger, originally P2)
- Location: src/models/ledger.rs:17
- What: DeliveryReceipt is defined and re-exported but has no internal production use in source references.
- Trigger: Maintainers or external consumers choose between overlapping delivery types.
- Impact: The public domain model is confusing; lack of internal use alone does not prove external consumers are absent.
- Fix: Clarify/deprecate its role before any removal and prefer the service's actual ledger/obligation representations.

### [P2] Ledger entities bypass their typed status enums  (from lane: models-ledger, originally P2)
- Location: src/ledger/service.rs:47
- What: Ledger entity fields and mutation helpers use strings despite corresponding state enums.
- Trigger: A future caller or edit introduces an invalid state spelling.
- Impact: State-machine mistakes lose compile-time checking; current constants alone do not demonstrate corruption.
- Fix: Use typed state/status values internally and explicit database conversions.

### [P2] Session state does not preserve unknown top-level JSON fields  (from lane: models-ledger, originally P2)
- Location: src/models/session.rs:268
- What: Serde ignores unrecognized top-level fields that are then absent on reserialization.
- Trigger: A future/newer writer adds top-level fields and an older gateway rewrites that state; no existing mixed-version field was identified.
- Impact: Forward-compatible round-tripping is not guaranteed, while arbitrary keys inside metadata remain preserved.
- Fix: Define the compatibility policy and, if unknown top-level fields must round-trip, capture them in a flattened extra map.

## Demoted

| Finding (lane and primary location) | Original | Re-grade | Reason |
| --- | --- | --- | --- |
| Invalid channel ACL entries (runtime-main, src/main.rs:939) | P0 | P1 | Requires malformed privileged configuration; not an attacker-controlled normal-input bypass. |
| Approval timeout overflow (runtime-main, src/main.rs:610) | P0 | P2 | Pathological operator-supplied maximum u64; no routine-input crash. |
| Recovery consumes pending intent (runtime-main, src/main.rs:449) | P0 | P1 | Requires route/dependency failure or crash in the handoff window. |
| Separate optional dashboard runtime (runtime-main, src/entry.rs:87) | P0 | P1 | Real control-plane mismatch under optional deployment wiring, without demonstrated P0 impact. |
| Cross-bot mirror early return (runtime-main, src/mirror.rs:120) | P0 | P1 | Requires ambiguous multi-bot origin and omitted bot identity. |
| Unsupervised drain watcher (runtime-main, src/main.rs:877) | P1 | P2 | No concrete watcher panic/failure trigger established; lifecycle hardening. |
| DashMap guards across awaits (multiplexer, src/multiplexer/router.rs:413) | P0 | P1 | Production worker exhaustion requires contending writers and pending continuations. |
| Cancelled send counter leak (multiplexer, src/multiplexer/router.rs:179) | P0 | P1 | Requires mailbox backpressure plus caller cancellation. |
| Cancelled retirement (multiplexer, src/multiplexer/gc.rs:61) | P0 | P1 | Requires cancellation during an incomplete retirement transaction. |
| Zero GC interval (multiplexer, src/multiplexer/gc.rs:18) | P0 | P2 | Public invalid input; production uses a nonzero default and panic is task-local. |
| Transcript-as-completion dedup (multiplexer, src/multiplexer/actor.rs:236) | P0 | P1 | Requires replay of a previously failed/stopped/crashed turn. |
| Accepted-event overflow (multiplexer, src/multiplexer/actor.rs:309) | P0 | P1 | Requires backlog beyond the bounded pending capacity. |
| Model overwrite on turn completion (multiplexer, src/multiplexer/actor.rs:424) | P0 | P1 | Requires overlapping model mutation and successful old-turn completion. |
| Fresh-session model no-op (multiplexer, src/multiplexer/router.rs:411) | P0 | P1 | Requires the absent-row/absent-actor edge state. |
| Reset missing/retiring actor (multiplexer, src/multiplexer/router.rs:420) | P0 | P1 | Requires collection or retirement overlap. |
| Actor recovery consumes marker (multiplexer, src/multiplexer/actor.rs:88) | P0 | P1 | Requires downstream failure or crash after clearing durable intent. |
| Parent-route precedence (multiplexer, src/multiplexer/profile_routing.rs:302) | P0 | P1 | Specific hierarchical route combination; live authorization bypass unproven. |
| Missing authorization context (multiplexer, src/multiplexer/profile_routing.rs:235) | P0 | P2 | Source search found no production caller of the helper. |
| Disabled/unrelated authorization fallback (multiplexer, src/multiplexer/profile_routing.rs:242) | P0 | P2 | Source search found no production caller of the helper. |
| Negative route IDs (multiplexer, src/multiplexer/profile_routing.rs:27) | P0 | P1 | Malformed operator configuration broadens profile matching, not proven live authorization. |
| Malformed route-array fallback (multiplexer, src/multiplexer/profile_routing.rs:402) | P0 | P1 | Configuration failure loses overrides; access-control bypass claim exceeds caller evidence. |
| Uncorrelated deadline drain (agent-backend, src/agent/omo_backend.rs:638) | P0 | P1 | Requires deadline cleanup plus unrelated peer frames. |
| Completed-item replacement (agent-backend, src/agent/omo_backend.rs:963) | P0 | P1 | Requires multiple message items; current upstream routine occurrence unverified. |
| Interim emission policy (agent-backend, src/agent/omo_backend.rs:978) | P0 | P1 | Dispatch-boundary defect; no demonstrated authorization bypass or durable loss. |
| Pre-ACK notification discard (agent-backend, src/agent/omo_backend.rs:921) | P0 | P1 | Peer response/notification ordering edge, not observed routine daemon behavior. |
| Uncapped turn output (agent-backend, src/agent/omo_backend.rs:972) | P0 | P1 | Sustained high-volume peer output; no normal-rate exhaustion bound. |
| Lifetime thread cache (agent-backend, src/agent/omo_backend.rs:414) | P0 | P1 | Sustained unique-session creation; realistic exhaustion rate not established. |
| Uncorrelated approvals (agent-backend, src/agent/omo_backend.rs:880) | P0 | P1 | Requires misattributed peer requests; ordinary attacker injection not established. |
| Unbounded readiness body (agent-backend, src/agent/omo_daemon.rs:54) | P0 | P1 | Requires abnormal/substituted endpoint output, not ordinary readiness behavior. |
| Cross-platform slug namespace (agent-workspace, src/agent/agent_workspace.rs:48) | P1 | P2 | Equal-ID production cross-platform actors not established. |
| Secret Debug formatting (agent-workspace, src/agent/llm.rs:28) | P1 | P2 | Actual secret-bearing logging site not established. |
| First-frame-only STT (voice, src/voice/pipeline.rs:149) | P0 | P1 | Multi-frame library input fails, but production multi-frame capture wiring was not found. |
| Zero voice capacity (voice, src/voice/pipeline.rs:278) | P0 | P2 | Invalid public-constructor input with no production zero-capacity caller. |
| Full-history memory search (models-ledger, src/memory/store.rs:106) | P0 | P1 | Large-history/load scaling issue without realistic normal-uptime OOM evidence. |
| Silence regex suffix (models-ledger, src/models/events.rs:69) | P1 | P2 | Surrounding trim defeats the reported whitespace trigger; retain the narrower parser typo. |
| Stringified database errors (models-ledger, src/error.rs:33) | P1 | P2 | API/diagnostic maintainability, not a demonstrated failed runtime policy. |
| Out-of-range PID cast (models-ledger, src/ledger/service.rs:64) | P1 | P2 | Invalid synthetic PID and signal zero; normal OS PIDs do not demonstrate impact. |

Promoted five concrete conditional defects from P2 to P1: zero configured turn timeouts (src/agent/omo_config.rs:151), numeric non-version endpoints (src/agent/llm.rs:94), missing STT timeout (src/voice/pipeline.rs:141), empty-transcript downstream calls (src/voice/pipeline.rs:258), and stealing another live process's obligations (src/ledger/service.rs:493).

## Dropped

| Source finding | Why removed |
| --- | --- |
| agent-workspace P0: boot-time migration wipes bindings on every reboot (src/agent/workspace_migration.rs:15) | The destructive helper exists, but source references show only definition, export, and tests, not a production call. The asserted src/main.rs:480 is a closing brace, not a migration call. Current gateway startup does not demonstrate the claimed unconditional wipe. |
| agent-workspace P0: arbitrary workspace path traversal (src/agent/agent_workspace.rs:79) | The production caller at src/agent/omo_backend.rs:305-310 passes `agent_workspace_slug`, whose sanitizer removes separators and adds a fixed category prefix. Public raw Path::join behavior alone does not establish attacker-controlled breakout in this gateway. |
| agent-workspace P2: missing config root disables production workspaces (src/agent/omo_config.rs:201) | Actual gateway and dashboard constructors immediately inject their resolved workspace roots (src/main.rs:662; src/dashboard_runtime.rs:117). The report names only a hypothetical secondary caller, not a failing existing path. |
| agent-workspace P2: empty cron identifier produces illegal/colliding slug (src/agent/agent_workspace.rs:54) | `cron-` is a legal directory component. No production source of distinct empty-ID cron jobs or enforced trailing-hyphen prohibition was established; identical empty identifiers are not proof of a collision between valid distinct jobs. |
| agent-workspace P2: cron total timeout below gap timeout is defective (src/agent/omo_config.rs:233) | Backend waits take the minimum of total/work and gap deadlines (src/agent/omo_backend.rs:797-801); a shorter total budget is valid and still enforced. Clamping the longer gap does not repair a demonstrated failure. |
| voice P0: RTP raw-packet offset corrupts every packet (src/voice/mod.rs:67) | The slicing line resolves, but the claim depends on the external Songbird version's exact RtpData offset/buffer contract. That dependency implementation is not established by the allowed repository source, so the claimed offset error cannot be confirmed. |
| voice P1: receive mutex prevents concurrent processing (src/voice/pipeline.rs:298) | A single mpsc receiver necessarily serializes dequeue access. The guard is released when receive returns, allowing consumers to process dequeued frames concurrently; no deadlock or processing serialization was demonstrated. |
| voice P1: equality-only buffer eviction permits unbounded growth (src/voice/pipeline.rs:82) | Capacity/frames are private, push preserves the bound, and deserialize explicitly rejects frames.len() > capacity. The proposed over-capacity state is unreachable through this implementation. |
| voice P1: multiple speakers are irreversibly mixed (src/voice/mod.rs:41) | Every emitted frame preserves source_id, so a shared channel does not itself lose speaker identity. No downstream consumer that ignores it or combines speakers without demultiplexing was found; nondeterministic ordering across distinct sources is not itself corrupted audio. |

## Cross-cutting patterns

1. **Success or retry-marker consumption precedes durable completion.** Gateway recovery clears intent before routing (src/main.rs:449), actor recovery repeats it (src/multiplexer/actor.rs:88), and transcript presence substitutes for execution success (src/multiplexer/actor.rs:236). Preserve a durable terminal/claim distinction instead of acknowledging enqueue or prior input presence.
2. **State is duplicated without one mutation owner.** A runner clone overwrites concurrent actor changes (src/multiplexer/actor.rs:424), router model updates replace a stale JSON snapshot (src/multiplexer/router.rs:406), and binding-preservation SQL undoes reset (src/multiplexer/actor.rs:758). Serialize intentional state transitions and distinguish deletion from absence.
3. **Errors are discarded after partial side effects.** Shutdown loses marker persistence errors (src/main.rs:888), mirror recency failure follows a committed transcript insert (src/mirror.rs:51), and constituent ledger failures follow parent success (src/ledger/service.rs:207). Propagate failure and transact related durable updates.
4. **Async ownership and cancellation boundaries are incomplete.** DashMap guards survive awaits (src/multiplexer/router.rs:413), send cancellation leaks a lifecycle counter (src/multiplexer/router.rs:179), GC cancellation strands retirement (src/multiplexer/gc.rs:61), and socket writes evade deadlines (src/agent/omo_backend.rs:508). Own lifecycle cleanup across cancellation and bound external awaits without forgetting ambiguous work.
5. **Filtering, bounds, and interpretation happen after capture instead of at the boundary.** Stream text is emitted before suppression/filtering (src/agent/omo_backend.rs:978), readiness captures an entire body before inspecting status (src/agent/omo_daemon.rs:54), and memory search loads the entire history before limiting results (src/memory/store.rs:106). Apply policy/correlation and resource budgets before accumulating or exposing data.
