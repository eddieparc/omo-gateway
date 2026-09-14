# Lane: cron-scheduler
## Scope
- `src/cron/scheduler.rs` - 2842 LOC, read in full, including its in-file tests. No product files modified.

## Findings
### [P0] Registration, resumption, and failure recovery ignore the job timezone
- Location: src/cron/scheduler.rs:756
- Evidence: `let next_run_at = next_run(&spec.expression, now)?;`
- Why it matters: Both registration methods and resume call the UTC-only helper; failure recovery also calls it through next_run_after_failure. Only successful recurring completion reads payload.schedule.timezone. A daily 09:00 Asia/Seoul job initially runs at 09:00 UTC instead of 00:00 UTC, then changes cadence after success; a failure or resume changes it back. Updating the timezone through register_with_id also computes the new deadline in UTC. An invalid timezone is accepted at registration and only rejected during successful completion, after execution and delivery.
- Suggested fix: Extract and validate the payload timezone at every scheduling boundary and pass it to next_run_tz, including failure recovery.

### [P0] Large valid intervals panic when added to the current date
- Location: src/cron/scheduler.rs:1927
- Evidence: `return Ok(after + delta);`
- Why it matters: parse_interval accepts u64 seconds and TimeDelta::from_std only checks the duration's range, not the resulting DateTime's range. For example, interval:10000000000000s fits TimeDelta but advances a contemporary date beyond chrono's maximum representable year. DateTime addition panics instead of returning the advertised configuration error. Registration exposes this input directly.
- Suggested fix: Use checked_add_signed and turn an out-of-range result into OmonError::Config; apply the same checked-boundary policy to clock-derived lease additions.

### [P0] User-provided repeat counts overflow on completion
- Location: src/cron/scheduler.rs:267
- Evidence: `let completed = obj.get("completed").and_then(Value::as_u64).unwrap_or(0) + 1;`
- Why it matters: A payload containing repeat.completed = 18446744073709551615 with no positive times limit passes the claim check. Completion then panics with overflow checking enabled, or wraps to zero otherwise. The flat completed representation has the same unchecked increment. A panic leaves the durable run unfinished; wrapping corrupts repeat accounting.
- Suggested fix: Reject an unincrementable count at registration and use checked_add in completion so persisted or imported payloads cannot bypass the boundary validation.

### [P0] Predecessor fallback crosses profile boundaries and treats IDs as LIKE patterns
- Location: src/cron/scheduler.rs:147
- Evidence: `WHERE (session_key = ? OR session_key LIKE ?) \`
- Why it matters: When no nonempty cron_outputs row exists for the requested profile, the fallback OR matches any session ending in the job ID, regardless of profile. Requesting profile A's job_alpha can therefore return profile B's assistant output as predecessor context. The permitted job IDs also include percent and underscore, which broaden the LIKE match further. An in-memory SQLite probe confirmed the cross-profile match.
- Suggested fix: Restrict fallback to explicitly enumerated exact session keys for the requested profile/job. If any LIKE matching remains necessary, escape wildcard characters and preserve the profile boundary.

### [P0] Fan-out sends every destination to the job's stored session
- Location: src/cron/scheduler.rs:1674
- Evidence: `SessionKey::from_storage_key(key).unwrap_or_else(|_| {`
- Why it matters: For a valid job.session_key, deliver uses that session unchanged instead of the destination's channel/thread. With a stored channel A session and destinations A and B, both OutboundAction values target A; B receives nothing and A receives duplicates. The notification and transcript-mirroring paths still use the requested destination, so their records can disagree with the actual dispatch. The destination's bot_id is also ignored when constructing the fallback session.
- Suggested fix: Build the dispatch session from each destination, including bot identity. Reuse a stored session only after validating its channel, thread, and bot against that destination, as the mirror helper already does.

### [P0] Updating a registered job silently retains its old session key
- Location: src/cron/scheduler.rs:764
- Evidence: `expression = excluded.expression,`
- Why it matters: register_with_id inserts session_key for new jobs, but its conflict-update assignments only replace expression, payload_json, next_run_at, and updated_at. Re-registering an existing ID with a new CronJobSpec.session_key silently preserves the old session; execution and delivery continue using the old conversation/channel.
- Suggested fix: Include session_key = excluded.session_key in the conflict update.

### [P0] Manual completion exhausts repeat limits without disabling the job
- Location: src/cron/scheduler.rs:1245
- Evidence: `let (times, completed_count) = increment_repeat_completed(&mut payload);`
- Why it matters: Success and failure increment the counter even when advance_schedule is false. For a job with repeat.times = 1, a manual trigger persists completed = 1 via the manual payload-only update, but leaves enabled = 1 and next_run_at intact. All subsequent scheduled claims hit should_disable_after and return without disabling or advancing the row. The job remains active and permanently due while never firing; every poll rechecks it.
- Suggested fix: Either keep manual runs outside the scheduled repeat budget, or atomically disable/clear next_run_at when manual completion reaches that budget. Make the behavior consistent with the manual-run contract.

### [P0] Pausing an executing recurring job discards its completion accounting
- Location: src/cron/scheduler.rs:1282
- Evidence: `WHERE id = ? AND enabled = 1 AND expression = ? AND payload_json = ?",`
- Why it matters: pause changes enabled to zero. Recurring success and failure then finalize cron_runs but their combined payload/schedule update matches no job. The completed count, last status, and last error are lost. For repeat.times = 2, pausing during the first execution and resuming afterward allows two additional executions because the first was never counted. The exact completion UPDATE was checked with an in-memory SQLite probe and affected zero rows for a paused job.
- Suggested fix: Persist completion accounting independently of whether scheduling is enabled; condition only next_run_at advancement on enabled, and retain revision checks to avoid overwriting an edited payload.

### [P0] Successful execution is rolled back when a cron expression has no next occurrence
- Location: src/cron/scheduler.rs:1278
- Evidence: `let next = next_run_tz(&claim.job.expression, now, timezone)?;`
- Why it matters: A valid finite-year cron can execute its final occurrence successfully, then have no next run. This question-mark exits complete_success before its transaction commits, rolling back the succeeded status as well as job updates. Execution and delivery have already happened and the heartbeat has stopped. The run remains running until reclamation, and the due schedule remains unchanged. The same stranded state occurs for a timezone that was accepted at registration but rejected here. Other completion database errors are likewise only logged, with no completion retry.
- Suggested fix: Treat schedule exhaustion as a successful terminal job state and commit the run while disabling the schedule. Validate timezone before execution, and preserve/retry completion separately from re-executing side effects when persistence fails.

### [P0] Execution tasks and their retained payloads have no concurrency bound
- Location: src/cron/scheduler.rs:1137
- Evidence: `executions.push(handle);`
- Why it matters: run_due_jobs fetches every due ID and spawns each claim without a concurrency limit; manual triggers on different jobs use the same unrestricted path. The executions vector retains every unfinished handle, and each execution owns a job snapshot plus a heartbeat task. A stream of distinct jobs with commands or executors that never finish causes unbounded task/process/retained-state growth. Pruning completed handles does not bound active work.
- Suggested fix: Acquire a bounded execution permit before claiming/spawning and page due-job selection. Add an execution deadline/cancellation policy so permanently stuck jobs cannot occupy capacity indefinitely.

### [P0] Shell output is accumulated without a size limit
- Location: src/cron/scheduler.rs:446
- Evidence: `let out = command.output().await.map_err(|e| {`
- Why it matters: Command::output collects stdout and stderr until the child exits. An accepted command such as yes can grow the gateway's memory without bound without ever producing a completed run. Large finite output is additionally copied into formatted completion text and persisted. MAX_CONTEXT_CHARS only constrains a separate context helper and is not applied here.
- Suggested fix: Read child pipes incrementally with explicit byte limits and a bounded execution lifetime, reporting truncation or terminating the process when the limit is exceeded.

### [P1] Reclamation can invalidate a lease that was refreshed after selection
- Location: src/cron/scheduler.rs:1037
- Evidence: `WHERE run_id = ? AND status = 'running'",`
- Why it matters: Expired candidate leases are selected and evaluated before the UPDATE. If the owner refreshes a reclaimable stale lease between that SELECT and UPDATE, the reclaimer still marks it failed because the UPDATE checks neither the observed lease timestamp nor current expiry. It can then insert a replacement run while the original execution continues. An in-memory SQLite probe using the exact UPDATE confirmed that a newly refreshed lease is still marked failed.
- Suggested fix: Compare-and-swap the observed lease_expires_at, and any owner identity used for the decision, in the reclaim UPDATE; abandon reclamation if the row changed.

### [P1] Live-owner reclamation has no execution fence and is vulnerable to clock jumps
- Location: src/cron/scheduler.rs:53
- Evidence: `if lease_age >= STALE_LEASE_SAFETY_NET {`
- Why it matters: The safety net reclaims even a live owner. A forward wall-clock jump of at least 150 minutes immediately after refresh makes the 30-minute lease at least 120 minutes overdue; another scheduler polling before the next heartbeat can reclaim it. Likewise, an owner stalled past the safety net can later resume. Neither reclamation nor refresh_lease cancels the old executor: a refresh affecting zero rows returns Ok, and execute_job can still deliver before token validation in completion. Original and replacement executions can therefore both perform side effects.
- Suggested fix: Detect lease loss from rows_affected, cancel lease-lost executions, and fence delivery/external effects using the active claim token. Do not treat elapsed wall-clock time alone as permission for a live owner and a replacement to both act.

### [P1] Claim eligibility is not restricted to Omon-owned authority
- Location: src/cron/scheduler.rs:1096
- Evidence: `AND authority != 'cutover_pending'`
- Why it matters: The atomic claim statement excludes only cutover_pending, not hermes_mirror or unknown non-null authority strings. run_due_jobs and the earlier job read use the same exclusion. Thus an enabled due Hermes mirror is executable by this scheduler even without an ownership transfer. If Hermes is still the authoritative executor, Omon's local cron_runs lease does not exclude that external owner and both can run the job. The typed is_omon_owned/is_hermes_mirror helpers are not consulted on this path.
- Suggested fix: Require the explicit execution-owner authority in the atomic claim predicate, and reject unknown authority values instead of treating them as executable. If mirrors are intentionally executable, encode and document the shared exclusion protocol rather than relying on this negative filter.

### [P1] Cutover receipt checking races with claim insertion
- Location: src/cron/scheduler.rs:1012
- Evidence: `if has_pending_cutover_receipt(&self.pool).await? {`
- Why it matters: The global pending-receipt guard is a separate query before multiple other awaits. A pending receipt inserted after this check but before INSERT INTO cron_runs does not stop the claim; the insertion only rechecks the job's authority. For a job whose authority has not yet changed, execution can begin inside the window the receipt is supposed to block.
- Suggested fix: Check pending receipt absence as part of the same atomic claim operation or serialize receipt creation and claiming under the same database write transaction.

### [P1] Claim validation and the executed job snapshot are not atomic
- Location: src/cron/scheduler.rs:1118
- Evidence: `let job = self`
- Why it matters: claim_job reads repeat/grace information, inserts a durable claim, then independently fetches the job again. Re-registering the ID after insertion can cause a claim for an old due schedule to execute a new payload/expression immediately even when the new next_run_at is in the future. A post-insert fetch error also leaves a running lease without an execution or heartbeat. Separately, repeat limits are checked before insertion but not in its predicate: two manual claim attempts can interleave around another manual completion and exceed the previously read limit.
- Suggested fix: Validate, claim, and capture the exact job revision in one transaction; include revision/limit checks in the claim and return that snapshot. Roll back the claim if its snapshot cannot be obtained.

### [P1] Completion can overwrite a newer schedule revision
- Location: src/cron/scheduler.rs:1264
- Evidence: `WHERE id = ? AND expression = ? AND payload_json = ?",`
- Why it matters: Completion guards compare expression and payload, but not next_run_at, updated_at, or an explicit revision. pause/resume and same-spec re-registration change scheduling state without changing those compared values. A stale recurring completion can overwrite a newly set deadline; a stale one-shot or repeat-limit completion can disable the newer scheduling decision. The exact recurring UPDATE was shown to overwrite a newer next_run_at in an in-memory SQLite probe.
- Suggested fix: Add a schedule revision captured by CronClaim and compare it in schedule mutations, or at minimum include the observed next_run_at and a revision that changes on every reschedule. Keep run-result accounting independent of schedule ownership.

### [P1] Backward wall-clock steps can replay an already executed cron occurrence
- Location: src/cron/scheduler.rs:1194
- Evidence: `let now = (self.clock)();`
- Why it matters: Successful completion advances from the completion wall clock rather than a lower bound containing the claimed scheduled instant. An every-minute job due at 12:01 can execute, then observe 12:00:30 after a clock step. Its next run becomes 12:01 again, so the same nominal occurrence executes twice when the clock catches up. Failure advancement has the same unbounded wall-clock base. UUID run IDs distinguish attempts, not scheduled occurrences.
- Suggested fix: Persist the claimed nominal firing time and compute advancement after at least max(completion_now, claimed_scheduled_time). Use an occurrence key if duplicate exclusion must survive clock corrections and reclamation.

### [P1] Delete and reschedule leave old executions running
- Location: src/cron/scheduler.rs:875
- Evidence: `let result = sqlx::query("DELETE FROM cron_jobs WHERE id = ?")`
- Why it matters: delete, pause, and register_with_id only mutate the job row and wake the poller. Execution handles are an unkeyed vector with no per-job cancellation token. An already claimed command or executor continues using its cloned job and can deliver after deletion or rescheduling; completion's payload checks happen only after those side effects. The shell executor also does not enable kill-on-drop or manage a process group.
- Suggested fix: Track executions by job/run revision and cancel them when the applicable mutation invalidates that revision. Revalidate the active revision before delivery and terminate owned subprocesses on cancellation.

### [P1] Resume writes a deadline calculated from a stale expression
- Location: src/cron/scheduler.rs:859
- Evidence: `"UPDATE cron_jobs SET enabled = 1, next_run_at = ?, updated_at = ? WHERE id = ?",`
- Why it matters: resume reads the job, calculates next_run from its expression, then updates by ID only. If register_with_id replaces the expression between the read and write, resume overwrites the replacement's deadline using the old expression. If delete wins instead, the UPDATE affects zero rows but resume still reports true.
- Suggested fix: Compare the observed job revision in the UPDATE and use rows_affected to detect a conflicting edit or deletion; retry against a fresh snapshot only when appropriate.

### [P1] A stopped scheduler cannot be started again
- Location: src/cron/scheduler.rs:675
- Evidence: `if *shutdown.borrow() {`
- Why it matters: shutdown sends true to the watch channel while the running loop is subscribed. start later accepts an empty/finished task slot but subscribes to the same retained true value, so the newly spawned loop exits before polling. No path resets the flag. A normal start/shutdown/start lifecycle silently leaves scheduling stopped.
- Suggested fix: Reset or replace the shutdown channel under the scheduler lifecycle lock before spawning a new polling task, with a defined start-versus-shutdown ordering.

### [P1] Shutdown holds the task mutex across an unbounded join
- Location: src/cron/scheduler.rs:700
- Evidence: `if let Some(task) = self.state.task.lock().await.take() {`
- Why it matters: The temporary MutexGuard in the if-let scrutinee remains alive while task.await executes in the body. The polling loop only observes shutdown after its current synchronization and full due-job sweep finish. During a blocked/long sweep, is_running and start block on this mutex for the entire join. Shutdown subsequently awaits all executions without any deadline, so a nonterminating command can prevent shutdown from returning at all.
- Suggested fix: Take the handle in a separate scoped statement, drop the guard before awaiting it, and implement bounded cancellation/join behavior for both the polling loop and executions.

### [P1] Cancelling wait_idle permanently detaches tracked executions
- Location: src/cron/scheduler.rs:565
- Evidence: `std::mem::take(&mut *guard)`
- Why it matters: wait_idle removes all execution handles from shared state before awaiting them. active_executions_count then reports zero for that work, and concurrent shutdown cannot join it. If the waiting future is cancelled or times out, dropping its JoinHandles detaches rather than aborts the still-running tasks. Those executions and their heartbeats continue with no remaining scheduler-owned handles.
- Suggested fix: Keep execution ownership in shared state until completion; implement waiting as an observation of tracked completion rather than transferring every handle into a cancellable caller future.

### [P1] Delivery obligation IDs collide within a millisecond
- Location: src/cron/scheduler.rs:1696
- Evidence: `let obl_id = format!("obl:cron:{}:{}", job.id, (self.clock)().timestamp_millis());`
- Why it matters: Distinct destinations and run attempts for one job have no identity component beyond a wall-clock millisecond. Two deliveries in the same millisecond, a repeated wall-clock value, or a fixed injected clock produce the same obligation ID. Both attempts then address the same ledger identity rather than independent obligations, so per-destination delivery accounting cannot be correct even though both dispatches may occur.
- Suggested fix: Give each delivery obligation a collision-resistant ID, preferably derived from run ID plus destination identity when retry idempotency is required; do not use wall time as uniqueness.

### [P1] Successful-run output persistence errors are silently discarded
- Location: src/cron/scheduler.rs:1310
- Evidence: `let _ = sqlx::query(`
- Why it matters: The cron_outputs insertion ignores its Result and the transaction is subsequently committed as a successful run. A statement-level failure can leave no predecessor output despite a recorded success, without even a warning. resolve_predecessor_output then returns older output or its unsafe fallback instead of the result just produced.
- Suggested fix: Propagate the insertion error or record an explicit output-persistence failure with a defined recovery path; do not silently commit an incomplete successful-run record.

### [P1] Incident read/write failures silently defeat acknowledgement handling
- Location: src/cron/scheduler.rs:1603
- Evidence: `let _ = sqlx::query(`
- Why it matters: execute_job discards incident INSERT/UPDATE errors; the preceding acknowledgement SELECT converts errors into unacknowledged, and the success path also ignores DELETE errors. A transient database failure can send alerts for an acknowledged incident, leave an alert with no incident row to acknowledge, or retain an old acknowledged incident across a success so the same later failure is incorrectly suppressed. None of these state failures is reported.
- Suggested fix: Handle and log incident persistence/read errors explicitly, preserving a deliberate acknowledgement policy when state is unavailable rather than silently treating database errors as normal state.

### [P1] Full shell commands expose embedded credentials in normal logs
- Location: src/cron/scheduler.rs:436
- Evidence: `tracing::info!(job_id = %job.id, command = %cmd, "Executing cron shell command");`
- Why it matters: A valid scheduled command containing an inline bearer token, password argument, or credential-bearing URL writes that secret verbatim to the normal INFO log before execution. The scheduler's lifecycle validation does not redact command arguments.
- Suggested fix: Log job ID and execution metadata, not the raw command; only log an explicitly redacted command representation if operationally necessary.

### [P2] In-file timezone tests never exercise a DST gap or fold
- Location: src/cron/scheduler.rs:2062
- Evidence: `fn next_run_tz_evaluates_wall_clock_and_tracks_dst_offset() {`
- Why it matters: This test checks Seoul and New York noon in January/July, but never nonexistent spring-forward local times, repeated fall-back local times, or advancing between both representations of an ambiguous time. It cannot establish the gap/fold behavior of Schedule::after with chrono-tz or detect a skipped/duplicated local occurrence at a transition.
- Suggested fix: Add deterministic next-fire cases at the exact spring and fall transition instants, including repeated advancement through the fold, and explicitly assert the chosen skip/duplicate policy without real-time sleeps.

## Strengths
- Ordinary competing claims use a single INSERT ... SELECT with NOT EXISTS for running jobs, rather than relying on a process-local mutex for database exclusion.
- Completion checks run ID and claim token transactionally and treats an already finalized run as idempotent instead of panicking.
- Overdue one-shot retirement compares the observed expression, next_run_at, and payload and checks that no run is active; this is stronger revision protection than the regular completion path.
- The implementation has an injectable clock, a deterministic retirement gate, bounded notification broadcasting, capped failure backoff, and pruning of finished execution handles.

## Notes
- Method: full sequential coverage of all 2842 lines, structural scans for unwrap/expect/panic, blocking calls, TODO/FIXME/HACK, awaits/locks, task spawning, scheduling, and authority predicates, followed by surrounding-code rereads for findings. The production unwrap calls in completion are guarded by matching Some statuses; no unguarded unwrap panic is alleged. No matching std::fs, std::thread::sleep, blocking_, TODO, FIXME, or HACK usage was found in this target.
- Verification approach: static path/race analysis plus four deterministic in-memory SQLite probes was chosen over a repository build/test run because this lane is read-only and may write only this report. The probes confirmed refreshed-lease reclamation, stale-deadline overwrite, paused-job accounting loss, and cross-profile fallback. They are SQL predicate checks, not executions of the Rust gateway. No Rust build, runtime DST experiment, or product test run was performed.
- The gap/fold behavior inside the cron/chrono-tz dependencies is not proved by this file alone. No specific dependency panic or transition algorithm defect is asserted; the timezone-call-site defects and the in-file transition-test gap are separately evidenced above.
- Recurring success deliberately advances from completion time, coalescing overdue occurrences and making intervals completion-relative. One-shots deliberately retire beyond 120 seconds of grace. Without a contrary catch-up contract, ordinary coalescing/grace behavior is not independently classified as a bug; backward-clock replay is a distinct demonstrated path.
- There is no persistent in-memory job map in this file. The unbounded resource findings concern active execution handles/tasks/processes and collected child output, not an invented job-map leak.
- The authority finding proves that the scheduler admits mirror and unknown authorities. The external duplicate-execution consequence is conditional on a separate authoritative executor still running; no deployment topology or external fencing implementation was reviewed. Likewise, obligation collision is proved at ID construction; the downstream ledger's precise conflict policy was not reviewed.
- No SQL-injection finding is made: the dynamic due clause is selected from fixed internal strings and values are bound. The context LIKE issue is unintended matching and profile isolation, not SQL syntax injection.
