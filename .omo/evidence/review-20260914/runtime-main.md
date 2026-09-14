# Lane: runtime-main
## Scope
- `src/main.rs`: 3396 LOC (`wc -l`; includes the full integration-test module).
- `src/entry.rs`: 386 LOC.
- `src/readiness.rs`: 480 LOC.
- `src/drain_control.rs`: 642 LOC.
- `src/mirror.rs`: 336 LOC.

All five files were read fully. Other files were consulted only to resolve the behavior of dependencies/callers of these targets, not as additional review targets.

## Findings
### [P0] Invalid channel ACL entries silently disable the restriction
- Location: src/main.rs:939
- Evidence: `.filter_map(|p| p.trim().parse::<u64>().ok())`
- Why it matters: `DISCORD_ALLOWED_CHANNELS` and `DISCORD_IGNORED_CHANNELS` use this parser. A supplied value such as `123;456` becomes an empty list without a boot error. The authorization implementation in this same file enforces channel restrictions only when the corresponding list is nonempty. With an otherwise authorized user, an intended restrictive channel policy therefore becomes unrestricted; individual malformed deny entries also disappear.
- Suggested fix: Parse security-relevant ID lists as `Result<Vec<u64>>`, reject every nonempty malformed component, and fail configuration validation before startup side effects.

### [P0] Approval timeout accepts values that overflow at startup
- Location: src/main.rs:610
- Evidence: `std::time::Duration::from_secs(config.approval_timeout_secs + 5),`
- Why it matters: `approval_timeout_secs_from` accepts every positive `u64`, including `APPROVAL_TIMEOUT_SECS=18446744073709551615`. This addition, repeated for the terminal tool, panics with overflow checks enabled and wraps to four seconds in a normal optimized build. Boot behavior and the effective approval deadline therefore depend on the build profile for the same accepted configuration.
- Suggested fix: Reject values outside a documented supported timeout range at boot and use checked addition for the extra five-second allowance.

### [P0] Recovery consumes durable pending intent before dispatch succeeds
- Location: src/main.rs:449
- Evidence: `omon_gateway::storage::clear_session_resume_pending(pool, &storage_key).await?;`
- Why it matters: The pending marker is cleared before constructing and routing the recovered event. A process failure in the intervening await window, or a routing error such as actor initialization failure, leaves the unfinished turn without its recovery marker. The error branch only logs the route failure and the function returns success; the next boot no longer selects this work. The original transcript survives, but the durable obligation to resume is lost.
- Suggested fix: Keep the pending intent until a durable successful handoff/completion, with a separate atomic claim if concurrent recovery must be excluded. At minimum restore the marker and propagate route errors; restoring on errors alone does not fix the crash window.

### [P0] The optional dashboard controls a different runtime from Discord
- Location: src/entry.rs:87
- Evidence: `let dashboard = legacy::dashboard_runtime::run_standalone(settings, dashboard_shutdown, false);`
- Why it matters: Enabling the dashboard starts `run_standalone` alongside a separately constructed gateway. The called function constructs its own multiplexer, approval guard, dispatcher and scheduler; `false` only suppresses its scheduler loop. No live gateway handles are supplied here. Dashboard stop/approval/manual-cron operations therefore address the dashboard runtime, not active Discord turns and approvals, and manual execution uses the dashboard dispatcher rather than the gateway egress. The attached-dashboard test manually supplies shared handles and does not exercise this entry wiring.
- Suggested fix: Construct the runtime once and start an attached dashboard using its actual multiplexer, approval guard, scheduler and egress. Reserve `run_standalone` for the dashboard-only CLI.

### [P0] User matching bypasses the mirror's cross-bot ambiguity guard
- Location: src/mirror.rs:120
- Evidence: `return Ok(Some(matching.0.clone()));`
- Why it matters: Both the threaded branch here and the non-threaded branch at the analogous early return choose the first matching user before checking distinct bot identities. When the same user has sessions for bot A and bot B at the same origin and the delivery omits `bot_id`, the newest row wins instead of refusing the ambiguous destination. A cron response can then be appended to the wrong bot's transcript, contaminating its subsequent model context. Supplying a user does not uniquely identify a bot.
- Suggested fix: Apply bot-ambiguity checking before either user-match return, or check the full user-filtered candidate set for a unique bot before selecting a session.

### [P1] Runtime ownership locking is never applied to a production command
- Location: src/entry.rs:81
- Evidence: `return legacy::run_gateway_public().await;`
- Why it matters: The entry path starts the gateway without acquiring `RuntimeOwnershipLock`; the enabled-dashboard path also does not acquire it. All references to `try_acquire` outside its definition are unit-test calls. Two processes with the same workspace, database and Discord tokens can therefore both start independent consumers and schedulers despite the explicit duplicate-start test. The test proves only the helper, not production exclusivity.
- Suggested fix: Acquire a stable runtime-identity lock before starting the gateway and hold the guard until all runtime cleanup completes. Ensure the identity reflects the actual shared database/workspace rather than a caller-selected test name.

### [P1] Dashboard failure drops the gateway instead of shutting it down
- Location: src/entry.rs:98
- Evidence: `result = &mut dashboard => {`
- Why it matters: If dashboard startup fails, for example because its HTTP port is occupied, this select arm immediately returns an error. The pinned gateway future is dropped wherever initialization or execution has reached; its resume marking, shard shutdown, scheduler shutdown and pool closure are not executed. Since the two futures start concurrently, recovered turns or scheduled work may already be running when an optional HTTP subsystem causes abrupt teardown.
- Suggested fix: Give the gateway a cancellation token and make both select arms request cancellation and await the other subsystem's common cleanup path before returning the original failure.

### [P1] Boot performs recovered and scheduled work before all fallible initialization finishes
- Location: src/main.rs:765
- Evidence: `OmoBackendConfig::cron_from_env()?.with_workspace_root(config.workspace_root.clone());`
- Why it matters: Pending delivery replay and session recovery have already occurred before cron configuration is even parsed. Later, `scheduler.start().await` precedes pairing-cache initialization, attachment-downloader construction and Discord-client construction, all of which can fail with `?`. A malformed cron configuration or later subsystem failure thus exits after external delivery/execution side effects have begun, without running the cleanup at the bottom of `run_gateway`. Recovered turns may already have had their pending markers consumed.
- Suggested fix: Validate every environment-derived configuration and construct fallible dependencies before admitting recovery or starting the scheduler. Once tasks start, route every failure through a shared cancellation-and-join cleanup path.

### [P1] Workspace creation failure is discarded
- Location: src/main.rs:158
- Evidence: `let _ = std::fs::create_dir_all(&workspace_root);`
- Why it matters: A non-directory path, permission error or full filesystem is treated as successful configuration. Database initialization, daemon startup and recovery can proceed with an unusable workspace before another component finally fails, or workspace-dependent tools and drain-marker operations fail later. The actionable original filesystem error is lost.
- Suggested fix: Propagate the creation error with the workspace path before initializing the database or starting background work.

### [P1] Normal service termination does not enter graceful shutdown
- Location: src/main.rs:885
- Evidence: `signal = tokio::signal::ctrl_c() => {`
- Why it matters: This is the only signal subscription in the gateway path and it is registered only after startup/recovery completes. Unix service managers and containers normally stop processes with SIGTERM, not Ctrl+C. SIGTERM therefore takes the default termination path rather than marking pending sessions, shutting down cron, closing the pool or running daemon-supervisor destructors. SIGINT during the lengthy pre-subscription startup window likewise has no gateway cleanup path.
- Suggested fix: Install SIGINT and Unix SIGTERM listeners at entry, feed a shared cancellation token through startup and runtime, and use the same orderly shutdown sequence for both signals.

### [P1] Drain detection is too late and is not wired into ingress
- Location: src/main.rs:876
- Evidence: `let mut drain_rx = drain_watcher.receiver();`
- Why it matters: The watcher is created only after recovery, scheduler startup and Discord-client spawning, and its receiver is never attached to the multiplexer with `with_drain_receiver`. An already-present drain marker does not prevent boot from executing recovered or cron work. On a new marker, ingress remains enabled while `mark_in_flight_resume_pending` runs and shard shutdown awaits; turns accepted after the marking pass are outside that snapshot. The reversible-drain test wires the receiver explicitly, unlike production.
- Suggested fix: Create and initially scan the watcher before recovery or ingress starts, attach the receiver before cloning the multiplexer, and stop admissions and scheduler dispatch before taking the shutdown snapshot.

### [P1] Failed shutdown persistence is silently ignored
- Location: src/main.rs:888
- Evidence: `let _ = multiplexer.mark_in_flight_resume_pending().await;`
- Why it matters: Both Ctrl+C and marker-drain branches discard this result. If SQLite is busy, full or read-only, the marking operation can fail after only part of the tracked sessions have been processed. Shutdown nevertheless proceeds and reports success, with no indication that some interrupted work lacks its recovery marker.
- Suggested fix: Observe and log/propagate persistence failure, retain a non-success shutdown result, and use a bounded retry or explicit durable fallback before terminating active work.

### [P1] The database is closed while interactive actors are still running
- Location: src/main.rs:906
- Evidence: `pool.close().await;`
- Why it matters: The preceding cleanup stops the cron scheduler and garbage collector, but never drains, cancels or joins the multiplexer actors or accepted queued turns. Shard shutdown only stops transport intake. A slow interactive turn can complete after pool closure and fail its transcript/state flush; when the top-level runtime returns, remaining tasks are simply dropped. There is also no overall shutdown deadline around the preceding joins, so a stuck task can prevent process termination indefinitely.
- Suggested fix: After closing admissions, explicitly drain or checkpoint/cancel and join all interactive actors and transport tasks while SQLite is available, then close the pool. Bound the complete sequence with a documented deadline and report forced termination.

### [P1] Discord client failure is converted into successful process exit
- Location: src/main.rs:881
- Evidence: `if let Ok(Err(err)) = res {`
- Why it matters: The first client completion exits the one-shot select. An ordinary client error is only logged; a `JoinError` from an unwinding panic is not handled at all. Unlike the signal branches, this branch neither marks pending sessions nor shuts down remaining shards, and the function ultimately returns `Ok(())`. One bot failure therefore stops all bots without the normal persistence path and without a failure exit status for the process supervisor.
- Suggested fix: Match every client result, retain the original error or panic-derived error, and run the same full shutdown sequence before returning a failure status. If successful client completion is allowed, define explicitly whether other clients should continue.

### [P1] The drain watcher JoinHandle is never supervised or reaped
- Location: src/main.rs:877
- Evidence: `let _drain_handle = drain_watcher.spawn();`
- Why it matters: The handle is neither polled for failure nor aborted and awaited on shutdown. The spawned loop has no cancellation path of its own. If it terminates by an unwinding panic, main observes at most a closed watch channel, not the task failure; during a long scheduler/dashboard cleanup the watcher continues scanning and logging unnecessarily. Dropping a Tokio JoinHandle detaches rather than cancels its task.
- Suggested fix: Keep the handle as a supervised select input, propagate unexpected task termination, and cancel/abort then await it in every shutdown and startup-error path.

### [P1] A false drain update still exits the gateway
- Location: src/main.rs:893
- Evidence: `changed = drain_rx.changed() => {`
- Why it matters: This select runs only once. The watcher publishes both `true` and `false`. If a marker is detected and then cleared while main is not scheduled, watch coalescing can leave an unseen update whose current value is false. The branch then skips its inner shutdown work but still exits the select and shuts down scheduler/pool. A closed channel has the same effect. A canceled drain can therefore stop the service without marking sessions or shutting down shards.
- Suggested fix: Wait in a loop until an actual true drain state is observed; treat false updates as continuation and channel closure as a separately reported watcher failure.

### [P1] Marker read failures silently cancel or suppress draining
- Location: src/drain_control.rs:217
- Evidence: `Err(_) => None,`
- Why it matters: An existing marker with unreadable permissions, invalid UTF-8 or a transient I/O error is indistinguishable from no marker. `scan_at` consequently changes an already-true drain state back to false, contradicting the fail-safe handling of malformed JSON. Operators can have a drain request on disk while the process continues admitting work, with no diagnostic explaining why.
- Suggested fix: Return a result that distinguishes absent markers from read failure, log the error with the path, and retain the previous drained state or explicitly fail closed for an unreadable existing marker.

### [P1] Drain scans perform blocking filesystem operations on Tokio workers
- Location: src/drain_control.rs:250
- Evidence: `self.scan_at(&epoch, Utc::now());`
- Why it matters: The asynchronous watcher calls synchronous `Path::exists` and `fs::read_to_string` every tick. If the configured workspace is on a stalled network filesystem or other slow mount, a runtime worker blocks in filesystem I/O; async cancellation cannot interrupt it. The same pattern exists when `collect_runtime_readiness` calls the synchronous `fs2` disk probe. This can delay unrelated gateway tasks and the shutdown signal they depend on.
- Suggested fix: Use asynchronous file reads for the watcher and execute filesystem-stat probes via a bounded blocking-work facility. Keep shutdown responsive if the filesystem operation does not return.

### [P1] Failed marker publication leaves unique temporary files behind
- Location: src/drain_control.rs:182
- Evidence: `fs::rename(&tmp_path, &path)`
- Why it matters: A successfully written UUID-named temporary file is never removed if rename fails. For example, a directory at `.drain_request.json` makes each attempted drain publication fail after creating another temporary file. Repeated operator/controller retries accumulate files in the state directory; a partially successful write can also leave a temporary file on its error path.
- Suggested fix: Use a scoped temporary-file guard, or explicitly remove the temporary file on both write and rename failure while preserving the original error.

### [P1] Readiness probes a different backend setting and default port
- Location: src/readiness.rs:286
- Evidence: `let appserver_url = std::env::var("OMON_APPSERVER_URL").ok();`
- Why it matters: The running gateway uses `OmoBackendConfig::from_env`, whose setting is `OMON_OMO_APPSERVER_URL` and whose default is `ws://127.0.0.1:19742` (also asserted in the target's own daemon-sharing test). This probe instead reads `OMON_APPSERVER_URL` and defaults to port 18800. A healthy default deployment is reported degraded, or an unrelated service on 18800 can make the backend check pass while the actual agent backend is down. A separately configured cron backend is not probed either.
- Suggested fix: Pass the resolved interactive and, when distinct, cron backend configuration into readiness rather than reparsing an unrelated environment variable.

### [P1] Configured bot count is reported as connected bot health
- Location: src/readiness.rs:228
- Evidence: `CheckResult::ok().with_metric("connected_bots", bot_count)`
- Why it matters: Any positive integer is healthy. Main passes `clients.len()` before calling `client.start`, so the startup report can claim connected bots before even one gateway WebSocket has connected. Invalid gateway intents, disconnection and loss of all live shards are not represented by this input. Consumers cannot use the result to distinguish a configured bot from an operational one.
- Suggested fix: Feed live shard/connection state into readiness, keep configuration count as a separately named metric, and only report ready after required connections have completed their ready handshake.

### [P1] Credential readiness accepts empty or unrelated provider keys
- Location: src/readiness.rs:193
- Evidence: `if std::env::var("ANTHROPIC_API_KEY").is_ok() || has_any_api_key {`
- Why it matters: `has_any_api_key` checks only environment-variable existence, and every provider branch accepts it. For example, a Claude model with only `OPENAI_API_KEY=` reports credentials present even though neither usable Anthropic credentials nor a configured compatible provider route has been demonstrated. Conversely, a daemon authenticated through its own stored credentials can work while this local-environment check reports degraded. This is not a test of the actual backend's authentication configuration.
- Suggested fix: Check nonempty credentials for the resolved provider/route, or obtain authenticated capability/readiness from the daemon that actually performs inference. Do not infer provider access from an unrelated variable's existence.

### [P1] Readiness timeout is replaced with a liveness success
- Location: src/readiness.rs:261
- Evidence: `Err(_) => client.get(&target_health).send().await,`
- Why it matters: A backend whose `/readyz` stalls beyond 800 ms but whose `/health` remains responsive is reported healthy. This is precisely a reachable overload or dependency-failure state where liveness and readiness differ. Falling back on transport errors hides a failure of an existing readiness endpoint rather than merely supporting an older server without that endpoint.
- Suggested fix: Fall back to `/health` only for an explicit unsupported-endpoint response such as 404; preserve timeout/transport failures of `/readyz` as degraded with their original cause.

### [P1] Mirroring reports success after failing to update session recency
- Location: src/mirror.rs:51
- Evidence: `let _ = sqlx::query(`
- Why it matters: The message insert commits independently, then the session `updated_at` update discards any database error. A concurrent lock, pool shutdown or storage failure in between leaves the transcript appended but recency stale while returning `Ok(true)`. `find_session_by_origin` orders candidates by that field, so later out-of-band deliveries can choose an older/wrong candidate based on stale ordering.
- Suggested fix: Commit the message insert and session timestamp update in one transaction and propagate both errors.

### [P1] Mirror lookup chooses fallback scope before filtering the requested bot
- Location: src/mirror.rs:147
- Evidence: `let rows = if rows.is_empty() {`
- Why it matters: For a delivery without a thread ID, any unthreaded row suppresses the channel-wide fallback before bot filtering occurs. With an unthreaded session for bot B and only a threaded session for explicitly requested bot A in the same channel, the query finds B, skips fallback, filters B away, and returns None. The intended A candidate is never examined. Thus another bot's unrelated session can make otherwise resolvable cron transcript mirroring disappear.
- Suggested fix: Apply the requested bot constraint before deciding whether the preferred unthreaded candidate set is empty; only then perform the channel-wide fallback.

### [P1] Mirror origin resolution materializes all historical candidates
- Location: src/mirror.rs:93
- Evidence: `.fetch_all(pool)`
- Why it matters: Both thread and channel lookup branches select all matching sessions and only then filter by bot and user in memory, building additional vectors/sets. On a long-lived channel with many per-user or per-thread sessions, each delivery's memory use and latency grow with the full retained session history, even when `bot_id` and `user_id` specify a narrow target. There is no limit or streaming bound.
- Suggested fix: Push exact constraints and ordering into the database and fetch only enough candidates to establish a unique bot and select the preferred row; use bounded queries or streaming where legacy key decoding is unavoidable.

### [P2] Descendant cleanup test depends on an arbitrary sleep
- Location: src/main.rs:1007
- Evidence: `tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;`
- Why it matters: The test checks child disappearance only after an assumed 100 ms reaping interval. Scheduler/process-reaper delays can make a correct cleanup fail intermittently, while cleanup that is incorrectly delayed can pass because of the extra grace. The child PID file is also assumed to be written within the script's one-second execution timeout rather than established by a startup handshake.
- Suggested fix: Establish child startup with an explicit handshake and await the actual child-exit/reaping completion with a bounded timeout; make the production cleanup's completion signal the assertion boundary rather than elapsed wall-clock delay.

## Strengths
- Discord client tasks are at least grouped in a `JoinSet`, the dispatcher is installed before recovery, and duplicate bot identities within one configured process are explicitly rejected.
- Startup recovery rechecks current actor authorization, retains unknown/unauthorized pending markers, respects suspension, and applies a restart-loop guard rather than blindly replaying everything.
- Drain markers are published with same-directory rename, have explicit epoch and expiry validation, and the newer watcher tests subscribe before triggering changes and reap their spawned handles.
- Mirror SQL binds external values instead of interpolating them; empty content and missing target sessions are handled explicitly. Disk readiness reports unreadable/zero-capacity samples as degraded rather than healthy.

## Notes
- This is a static review, not a production execution or fault-injection run. No product files were changed and no builds/tests were run, to honor the one-file write boundary. Findings are based on fully read target code plus narrowly consulted call contracts. Structural scans covered panic sites, ignored results, task creation, blocking calls and TODO/FIXME/HACK markers; history/blame was consulted for entry and shutdown wiring.
- Compared a source/call-contract review with boot/fault-injection testing; the former was chosen because real gateway startup requires external credentials/services and performs database/recovery side effects, while this task permits only the report file to be written. No claim here depends on having booted the gateway.
- Cargo selects `src/entry.rs` as the binary, which includes `src/main.rs` in `legacy`; the nested `main` in that file is not the process entry. The repository Cargo manifest contains no `panic = "abort"` profile setting. With the ordinary unwind strategy, Tokio task panics become `JoinError` (the default panic hook can still print); the report's silent-failure language means no supervisory handling, not guaranteed silence on stderr. An externally supplied abort strategy would instead terminate the process immediately and bypass destructors. External build flags were not established.
- Direct `unwrap`/`expect`/explicit panic scan hits in these targets were test code; the boot integer overflow is a separate reachable panic path. `DrainWatcher::new` also accepts a zero interval that would panic inside `tokio::time::interval`, but main supplies a fixed nonzero interval, so no production-input panic finding is asserted for that library edge.
- The marker format intentionally treats empty/malformed JSON and missing/future timestamps as active, and intentionally shares a host/container epoch across processes. Those documented choices were not reported as defects. A marker surviving a service restart remains active until cleared/expired; whether orchestration consumes it externally is outside this lane.
- Optional user matching and no-thread fallback in mirror lookup may be intended best-effort routing. A missing requested user alone was therefore not labeled an authorization vulnerability. The reported cross-bot early return and bot-filter/fallback ordering have concrete contradictory candidate-selection behavior.
- Database readiness proves a read query, not write durability or schema completeness; disk readiness proves space, not workspace writability. These limitations remain relevant when interpreting the report but are not separately counted as bugs without a stronger readiness contract.
