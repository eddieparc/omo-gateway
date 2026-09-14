# Lane: cron-store-exec

## Scope
- `src/cron/store.rs` (1,552 LOC): Persistence model, store synchronizer, notepad storage, retention pruning, authority tracking, and destination resolution.
- `src/cron/executor.rs` (1,000 LOC): Native and Hermes cron execution, script timeouts, monitor script/URL state tracking, prompt assembly, and skill resolution.
- `src/cron/ack.rs` (104 LOC): Post-delivery ack command execution, process spawning, and timeout killing.
- `src/cron/guard.rs` (93 LOC): Gateway lifecycle self-restart and process kill pattern guards.
- `src/cron/mod.rs` (30 LOC): Module layout and public exports.
Total: 2,779 LOC.

## Findings

### [P0] Validation failure during synchronization causes silent permanent deletion of existing cron jobs
- Location: src/cron/store.rs:942
- Evidence:
```rust
                    Err(reason) => {
                        tracing::warn!(job_id = %job.id, %reason, "skipping invalid Hermes job");
                        continue;
```
- Why it matters: In `HermesStoreSynchronizer::sync_at`, when a job in `jobs.json` fails `job.validate()` (e.g. due to an invalid schedule expression, unparseable timezone, or temporary guard match), the loop logs a warning and executes `continue`. This skips registering the job ID into `live`. Subsequently, the orphan cleanup query (`DELETE FROM cron_jobs WHERE id = ? AND authority = 'hermes_mirror'` at line 988) deletes every stored job whose ID is absent from `live`. Consequently, a transient validation failure or syntax error on a single job in `jobs.json` permanently drops that job and its entire historical runtime state (`repeat.completed`, `last_status`, `last_run_at`, etc.) from the database instead of preserving the previous valid configuration.
- Suggested fix: Distinguish between parse/validation errors and deliberate job deletions: only delete jobs if all jobs in the source file parsed and validated cleanly, or retain unvalidated existing IDs in `live` with an error flag so they are not pruned.

### [P0] Premature commit of monitor hash drops state change alerts on downstream execution or delivery failure
- Location: src/cron/executor.rs:164
- Evidence:
```rust
                "INSERT INTO cron_monitor_states (job_id, last_hash, last_snapshot, updated_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(job_id) DO UPDATE SET last_hash = excluded.last_hash,
```
- Why it matters: `cron_monitor_states` is updated with `current_hash` and `m_output` before the prompt is assembled, before `self.backend.run()` executes, and before delivery obligations are dispatched. If the downstream agent run fails, times out, or delivery fails, `cron_monitor_states` has already committed the new hash. On retry or the next scheduled poll, `prev_hash == current_hash` (line 156) will be true, causing the executor to log `"monitor state unchanged for job {}, skipping agent execution"` and return `Ok(None)`. The change notification is permanently lost.
- Suggested fix: Record the new hash in session metadata or return it alongside the output, and only commit the new hash to `cron_monitor_states` in `CronScheduler::complete_success` after execution and delivery have succeeded.

### [P0] Gateway lifecycle guard regexes bypassed by multiline commands and line continuations
- Location: src/cron/guard.rs:14
- Evidence:
```rust
            r"(?i)\bsystemctl\s+(?:-\S+\s+)*(?:restart|stop|start)\b[^\n]*\b(?:omon|hermes)",
```
- Why it matters: The regexes in `GATEWAY_LIFECYCLE_PATTERNS` (lines 9–19) use `[^\n]*` between the action verb (`restart`, `stop`, `kickstart`, `pkill`) and the target process/service name (`omon`, `hermes`). In shell scripts and command invocations, arguments can be split across lines using backslash line continuation (`\ \n`) or normal newlines in scripts (e.g. `systemctl restart \\\nomon-gateway` or `pkill -9 \\\nomon-gateway`). Because `[^\n]*` stops at `\n`, the regex fails to match and the command is permitted, bypassing the guard and terminating the gateway.
- Suggested fix: Replace `[^\n]*` with `(?:\s|\\\n|[^\s])*` or match over any whitespace, or parse command argv tokens rather than relying on single-line string regexes.

### [P0] Modern macOS launchctl subcommands (`bootout`, `kill`) bypass gateway lifecycle guard
- Location: src/cron/guard.rs:10
- Evidence:
```rust
            r"(?i)\blaunchctl\s+(?:kickstart|unload|load|stop|restart)\b[^\n]*\b(?:omon|hermes)",
```
- Why it matters: Under modern macOS launchd, services are stopped or terminated via `launchctl bootout <domain/service>` or `launchctl kill <signal> <domain/service>`. The guard pattern only checks `(?:kickstart|unload|load|stop|restart)`. A script executing `launchctl bootout gui/501/ai.hermes.gateway` or `launchctl kill SIGTERM gui/501/ai.hermes.gateway` bypasses the guard completely, causing the gateway daemon to be killed or unloaded without restriction.
- Suggested fix: Include `bootout`, `kill`, `bootstrap`, and `reboot` in the launchctl subcommands alternation list.

### [P0] Script body lifecycle check in store validation is dead code and bypassed due to missing `_omon_hermes_home`
- Location: src/cron/store.rs:342
- Evidence:
```rust
            if let Some(home) = self.extra.get("_omon_hermes_home").and_then(Value::as_str) {
                let script_path = PathBuf::from(home).join("scripts").join(script);
                if let Ok(content) = std::fs::read_to_string(&script_path) {
```
- Why it matters: In `HermesJob::validate()`, script contents on disk are scanned for lifecycle violations, but only if `self.extra.get("_omon_hermes_home")` is present. During store sync (`HermesStoreSynchronizer::sync_at`, line 940), `job.validate()` is called on the freshly parsed `HermesJob` before `_omon_hermes_home` is injected into `payload` (line 958). Because `extra` does not contain `_omon_hermes_home` during sync, this block is dead code and never executes during synchronization. Malicious or accidental lifecycle commands inside script files on disk are never checked upon import.
- Suggested fix: Pass `store.home()` into `job.validate()` as an explicit parameter instead of expecting it inside `self.extra`.

### [P1] Subprocess leak on ack command timeout due to missing process group and process tree termination
- Location: src/cron/ack.rs:38
- Evidence:
```rust
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;
```
- Why it matters: Unlike `execute_native_cron` (`src/cron/executor.rs:407`) and `run_cron_script` (`src/cron/executor.rs:856`), `run_ack_command` neither sets `command.process_group(0)` nor invokes `libc::kill(-pid, SIGKILL)` when the timeout fires. On Unix, `kill_on_drop(true)` only signals the immediate `sh` process. Any child processes, subprocesses, or pipelines spawned by `sh` are not signaled; they become orphaned, get reparented to PID 1, and continue executing indefinitely, leaking system resources and holding locks.
- Suggested fix: On Unix, configure `command.process_group(0)` before spawning, extract `child.id()`, and send `SIGKILL` to `-(pid as i32)` in the timeout branch.

### [P1] Ack command executed without working directory or augmented environment PATH
- Location: src/cron/ack.rs:31
- Evidence:
```rust
    let child = tokio::process::Command::new(shell)
```
- Why it matters: `run_ack_command` configures neither `current_dir` nor `PATH`. It executes in whatever working directory the gateway daemon happens to occupy. Ack commands (such as git checkpoint commits or local commit scripts) that use relative paths (e.g. `./scripts/checkpoint.sh` or `git commit`) will execute in the gateway process root instead of the job's workspace or Hermes home directory. Furthermore, binaries in `/opt/homebrew/bin`, `~/.cargo/bin`, or `~/.local/bin` are not in `PATH` if the daemon runs with a minimal environment.
- Suggested fix: Accept an optional `workdir: Option<&Path>` and call `.current_dir(workdir)` and `command.env("PATH", &augmented_path_from_environment())`.

### [P1] Cross-profile collision on `cron_monitor_states` primary key
- Location: src/cron/executor.rs:145
- Evidence:
```rust
                sqlx::query_as("SELECT last_hash FROM cron_monitor_states WHERE job_id = ?")
```
- Why it matters: The `cron_monitor_states` table schema defines `job_id TEXT PRIMARY KEY NOT NULL` without a `profile` column. The executor binds `&hermes.id` (unscoped, e.g. `"weather"`) instead of `job.id` (scoped, e.g. `"hermes:personal:weather"`). If two Hermes profiles define a job with the same name, or if a native job and Hermes job share an ID, they overwrite each other's hash in `cron_monitor_states`, causing alternating false-positive state changes and dropped alerts.
- Suggested fix: Bind `&job.id` (the globally unique scoped identifier) instead of `&hermes.id`.

### [P1] Monitor output discarded and omitted from agent prompt
- Location: src/cron/executor.rs:252
- Evidence:
```rust
        if let Some(output) = script_output.filter(|output| !output.trim().is_empty()) {
            prompt.push_str("\n\n[Script output]\n");
            prompt.push_str(&output);
```
- Why it matters: When a job defines `monitor_script` or `monitor_url`, the output is retrieved into `monitor_output` (line 94) and hashed. If the hash changes, agent execution proceeds. However, during prompt construction (lines 190–255), only `script_output` (from `hermes.script`) is added to `prompt`. `monitor_output` is never appended to `prompt`. The agent is invoked to handle the state change but is provided zero content or diff regarding what changed.
- Suggested fix: Append `monitor_output` under a `\n\n[Monitor output]\n` section in `prompt` alongside `script_output`.

### [P1] TOCTOU race condition and unbounded key growth in `set_cron_notepad`
- Location: src/cron/store.rs:1022
- Evidence:
```rust
    let current_entries = get_cron_notepads(pool, profile, job_id).await?;
```
- Why it matters: In `set_cron_notepad`, the total byte size check against `MAX_NOTEPAD_TOTAL_BYTES` (64 KiB) is performed outside a database transaction prior to `INSERT ... ON CONFLICT`. Concurrent calls for different keys on the same job race between the fetch and the insert, allowing the total stored size to exceed the 64 KiB ceiling. Additionally, there is no cap on the number of keys. An agent or script can store an unbounded number of keys, all of which are formatted into the prompt in `executor.rs:242` without limit, leading to prompt bloat and token exhaustion.
- Suggested fix: Enforce size checks inside a transaction using an immediate transaction lock, and add a cap on the maximum entry count (e.g. 50 entries).

### [P1] Non-transactional store synchronization risks inconsistent state and scheduler races
- Location: src/cron/store.rs:974
- Evidence:
```rust
                let insert_result = sqlx::query(
```
- Why it matters: `HermesStoreSynchronizer::sync_at` runs in auto-commit mode without wrapping its operations in a transaction (`pool.begin()`). Job upserts and orphan deletions execute individually. If an error occurs midway through sync (e.g. database busy, disk full, or bad JSON serialization), a partial set of jobs is left updated while others remain stale or orphaned. Furthermore, the concurrent scheduler loop can observe and claim partially imported or inconsistent jobs during the synchronization sweep.
- Suggested fix: Wrap the entire synchronization loop per store inside a single database transaction (`let mut tx = self.pool.begin().await?`) and commit only upon full success.

### [P1] Blocking filesystem I/O inside asynchronous execution paths
- Location: src/cron/executor.rs:808
- Evidence:
```rust
    let script_content = std::fs::read_to_string(&path).map_err(|error| {
```
- Why it matters: `run_cron_script` and `AgentCronExecutor::execute` are asynchronous functions running on Tokio worker threads. Synchronous filesystem calls (`std::fs::read_to_string`, `std::fs::canonicalize`, and recursive `std::fs::read_dir` in `find_skill_file` and `resolve_skill_bundle`) block the executor thread. Under slow I/O or network mounts, blocking reactor threads can starve time-critical background tasks such as `refresh_lease` in `scheduler.rs:1145`, leading to spurious lease expirations and duplicate executions.
- Suggested fix: Use `tokio::fs` or offload synchronous file traversal to `tokio::task::spawn_blocking`.

### [P1] Unbounded agent backend execution duration lacking timeout enforcement
- Location: src/cron/executor.rs:336
- Evidence:
```rust
        self.backend.run(&mut session, event).await?;
```
- Why it matters: While shell scripts have explicit timeout enforcement via `resolve_cron_script_timeout`, `self.backend.run(&mut session, event).await` is invoked without any timeout wrapper. If the agent backend hangs on network connections, deadlocks on internal mutexes, or enters an infinite loop, the task future remains blocked indefinitely.
- Suggested fix: Wrap `self.backend.run` in `tokio::time::timeout(agent_timeout, ...)`.

### [P1] Potential process suicide on `libc::kill(-(pid as i32), SIGKILL)` when pid is zero
- Location: src/cron/executor.rs:421
- Evidence:
```rust
                            libc::kill(-(pid as i32), libc::SIGKILL);
```
- Why it matters: If `child.id()` returns 0 or if `pid as i32 == 0`, `-(pid as i32)` evaluates to 0. In POSIX libc, calling `kill(0, sig)` sends the signal to every process in the process group of the caller. Consequently, `libc::kill(0, SIGKILL)` terminates the entire gateway process and its child threads rather than the timed-out script. A similar pattern exists at line 893.
- Suggested fix: Guard with `if pid > 0 { unsafe { libc::kill(-(pid as i32), libc::SIGKILL); } }`.

### [P1] Reachable panic on large retention days in `prune_terminal_cron_runs`
- Location: src/cron/store.rs:127
- Evidence:
```rust
    let cutoff = now - chrono::TimeDelta::days(retention_days);
```
- Why it matters: `chrono::TimeDelta::days(retention_days)` internally invokes `try_days(retention_days).expect("out of range")`. In `cron_runs_retention_days_from_environment`, `retention_days` is parsed from `CRON_RUNS_RETENTION_DAYS` as an `i64` and only checked for `>= 0`. If a user supplies a large value (e.g. `999999999999`), `TimeDelta::days` panics, crashing the gateway on startup during `prune_terminal_cron_runs` (invoked from `main.rs:725`).
- Suggested fix: Use `chrono::TimeDelta::try_days(retention_days).ok_or_else(...)` and clamp or return a configuration error.

### [P2] `failure_deliver = []` inverts user intent by falling back to origin delivery
- Location: src/cron/store.rs:478
- Evidence:
```rust
        if let Some(ref failure_deliver) = self.failure_deliver {
            let mut clone = self.clone();
            clone.deliver = Some(failure_deliver.clone());
            clone.discord_destinations()
```
- Why it matters: When a user explicitly specifies `failure_deliver: []` (or an empty string) to suppress failure notifications, `clone.deliver` is `Some(vec![])`. In `discord_destinations()`, `match &self.deliver` tests `Some(list) if !list.is_empty()`. Because `list` is empty, it falls through to the default `vec!["origin".to_string()]`. Instead of delivering to zero destinations, failure notifications are dispatched to origin, violating the user's intent.
- Suggested fix: Check `if let Some(list) = &self.failure_deliver { if list.is_empty() { return Ok(Vec::new()); } }`.

### [P2] Script retry loop on transient loader error multiplies effective execution timeout
- Location: src/cron/executor.rs:409
- Evidence:
```rust
            let res = tokio::time::timeout(timeout, child.wait_with_output()).await;
```
- Why it matters: In `execute_native_cron` and `run_cron_script`, when a transient process initialization error occurs, the loop sleeps and retries up to 3 times. Each attempt resets the full `timeout` duration rather than deducting elapsed time. Under repeated transient errors, script execution can take up to 3x `timeout` plus backoff sleep, violating the configured execution budget.
- Suggested fix: Track an overall deadline (`let deadline = tokio::time::Instant::now() + timeout`) and pass `deadline - Instant::now()` to `tokio::time::timeout`.

### [P2] Native cron execution omits gateway lifecycle check on assembled prompt
- Location: src/cron/executor.rs:477
- Evidence:
```rust
    let threats = crate::security::scan_assembled_cron_prompt(&task);
```
- Why it matters: `execute_native_cron` validates `check_gateway_lifecycle` on `payload["script"]` (line 371), but fails to invoke `check_gateway_lifecycle(&task)` on the assembled prompt. `scan_assembled_cron_prompt` only checks prompt injection patterns, allowing native cron prompts to request gateway restart or termination commands.
- Suggested fix: Add `check_gateway_lifecycle(&task)?;` before dispatching the inbound message.

### [P2] Delivery destination parser silently drops targets with non-numeric thread IDs
- Location: src/cron/store.rs:661
- Evidence:
```rust
                    if left_trimmed.chars().all(|c| c.is_ascii_digit())
```
- Why it matters: When destination targets are formatted as `channel_id:thread_id`, lines 661–662 enforce that both `left_trimmed` and `right_trimmed` consist exclusively of ASCII digits. If a platform or custom adapter uses alphanumeric thread identifiers, the condition evaluates to false, falls through to the raw digit branch (which also fails due to `:`), and silently drops the target without warning or error.
- Suggested fix: Only enforce ASCII digits on the channel ID or accept alphanumeric characters for thread identifiers.

## Strengths
- **Defense in depth on prompt injection and lifecycle guards**: The codebase scans prompts, scripts, and ack commands at both definition time and execution time, and tests against forbidden patterns.
- **Robust repeat and one-shot schedule idempotence in SQL**: The ON CONFLICT upsert query in `store.rs` carefully prevents completed one-shots and repeat-times jobs from being re-armed during store sync.
- **Process group isolation during script execution**: `execute_native_cron` and `run_cron_script` create distinct process groups (`command.process_group(0)`) to ensure child process trees can be cleanly targeted with `SIGKILL` on timeout.
- **Strict path sandboxing**: `run_cron_script` and skill resolution strictly enforce path canonicalization and directory boundary checks to prevent directory traversal outside authorized Hermes roots.

## Notes
- `scheduler.rs` was inspected as context to understand how `store.rs`, `executor.rs`, and `ack.rs` interact with the scheduler run loop and delivery obligations, but findings and citations were strictly limited to the in-scope files (`store.rs`, `executor.rs`, `ack.rs`, `guard.rs`, `mod.rs`).
- `ack.rs`'s intended behavior is non-blocking fire-and-forget logging (`run_ack_logged`), which correctly isolates delivery obligations from commit failures; the process-group leak on timeout is the primary reliability risk in that module.
