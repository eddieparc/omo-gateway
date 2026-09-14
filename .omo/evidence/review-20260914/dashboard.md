# Lane: dashboard
## Scope
- `src/dashboard.rs`: 3,278 LOC
- `src/dashboard_runtime.rs`: 393 LOC

## Findings
### [P0] Partial updates in update_bot silently wipe unprovided fields to NULL
- Location: src/dashboard.rs:1927
- Evidence:         "UPDATE bot_profiles SET
- Why it matters: `UpdateBotPayload` defines `model`, `system_prompt`, and `enabled_toolsets` as `Option<T>`. When a client performs a partial update via `PUT /api/bots/{id}` (e.g. updating only `name`), unsupplied fields deserialize as `None`. Unlike `name` and `custom_settings` which fall back to existing database values via `unwrap_or`, `model`, `system_prompt`, and `enabled_toolsets` are bound directly (`&payload.model`, etc.), writing SQL `NULL` over existing configurations in the database and causing silent data loss.
- Suggested fix: Preserve existing values for omitted fields prior to executing the SQL update (e.g. `let model = payload.model.or(existing.model);`).

### [P0] Dashboard lacks authentication on high-privilege execution and administrative endpoints
- Location: src/dashboard.rs:698
- Evidence: pub fn router(state: DashboardState) -> Router {
- Why it matters: The entire dashboard HTTP and WebSocket API is unauthenticated. Any process or local network entity with access to the port can approve arbitrary shell commands via `POST /api/approvals/{id}/resolve`, execute arbitrary agent instructions via `POST /api/sessions/{id}/chat` or session WebSockets, create or trigger arbitrary cron jobs, modify bot personas and tool configurations, or exfiltrate full message histories and server logs.
- Suggested fix: Add authentication middleware requiring a secret bearer token or cookie on all `/api/` endpoints except health check routes (`/api/health`, `/api/readiness`).

### [P0] State-changing HTTP mutation endpoints lack Origin/CSRF validation
- Location: src/dashboard.rs:229
- Evidence:     if is_ws {
- Why it matters: The `validate_host_and_origin` middleware validates the `Origin` header only when `Upgrade: websocket` is present. Standard HTTP requests only verify that the `Host` header matches loopback. Browsers connecting to localhost automatically provide `Host: 127.0.0.1:9119`. Consequently, any external website loaded in a developer's browser can perform cross-site request forgery (CSRF) via simple POST requests to trigger cron jobs, delete sessions, or stop active agent runs without user confirmation.
- Suggested fix: Extend origin verification to all state-mutating HTTP methods (`POST`, `PUT`, `DELETE`), rejecting requests where `Origin` is present and does not match the server host, or require a custom header like `X-Requested-With` or an anti-CSRF token.

### [P0] Premature server termination in run_standalone causes indefinite hang and zombie process
- Location: src/dashboard_runtime.rs:206
- Evidence:     shutdown.cancelled().await;
- Why it matters: `run_standalone` spawns the Axum server task and immediately awaits `shutdown.cancelled()`. If `axum::serve` exits prematurely (e.g. due to port binding errors, listener failures, or internal server panics), the server task completes with an error, but `shutdown` is never signalled. As a result, `shutdown.cancelled().await` hangs indefinitely, leaving the process running as a zombie with an unresponsive port that never shuts down cleanly.
- Suggested fix: Use `tokio::select!` to await either `shutdown.cancelled()` or `server`, and trigger cancellation if the server task finishes unexpectedly.

### [P1] Race condition in create_cron_job allows initially paused jobs to trigger execution
- Location: src/dashboard.rs:1449
- Evidence:     if input.enabled == Some(false) {
- Why it matters: `scheduler.register` and `register_with_id` insert records with hardcoded `enabled = 1` and immediately invoke `self.wake.notify_one()` to signal the scheduler task. If a client creates a job with `"enabled": false`, there is an active race window between job insertion and the subsequent `scheduler.pause(&job.id).await` call where an immediate cron trigger will claim and execute the job despite the caller's request for it to start paused.
- Suggested fix: Pass the initial `enabled` state into the scheduler registration methods and insert the record with `enabled = 0` when `input.enabled == Some(false)`.

### [P1] OutboundAction::ExpireApproval is never forwarded to session WebSocket subscribers
- Location: src/dashboard.rs:518
- Evidence:         OutboundAction::ExpireApproval { .. } => None,
- Why it matters: In `handle_session_socket`, outbound events are routed to client WebSockets only if `action_session(&action)` matches the active `storage_id`. Because `action_session` returns `None` for `OutboundAction::ExpireApproval`, expiration and resolution events are never sent to the session socket. Connected web clients remain unaware that an approval has been resolved or expired until the user reloads the page.
- Suggested fix: Track the mapping from `request_id` to `SessionKey` in `WebDashboardDispatcher` and inspect this mapping to provide the target session for `ExpireApproval` events.

### [P1] ApiError leaks internal database error details and schema structure to callers
- Location: src/dashboard.rs:772
- Evidence: impl From<sqlx::Error> for ApiError {
- Why it matters: `From<sqlx::Error>` converts raw SQL errors into response messages via `error.to_string()`. In the event of SQLite constraint failures, disk I/O errors, or syntax errors, internal database table names, column structures, constraint names, and query details are returned verbatim in HTTP 500 JSON responses.
- Suggested fix: Log detailed error messages internally via `tracing::error!` and return a generic error message (e.g. `"internal database error"`) in the client JSON response.

### [P1] list_cron_runs exposes internal lease claim_token and owner process IDs
- Location: src/dashboard.rs:1572
- Evidence:     claim_token: String,
- Why it matters: `CronRunRow` serializes `claim_token` and `owner_pid` directly into JSON. In the cron scheduling architecture, `claim_token` is the secret lease token proving ownership of an execution lock. Exposing this token allows unauthorized actors to view secret lease credentials and discover internal host process IDs.
- Suggested fix: Exclude `claim_token` from serialized responses (e.g. via `#[serde(skip)]`) and omit or sanitize `owner_pid`.

### [P1] Synchronous fs2 disk space metrics block async worker threads in status and readiness handlers
- Location: src/dashboard.rs:583
- Evidence:     pub fn sample_disk(&self) -> (Option<u64>, Option<u64>) {
- Why it matters: `sample_disk` calls `fs2::total_space` and `fs2::available_space`, which execute synchronous `statvfs` system calls directly on Tokio worker threads during `GET /api/status` and `GET /api/readiness`. If the workspace path is mounted on a slow or non-responsive disk (e.g. NFS/SMB/FUSE), the system call blocks the Tokio worker thread, creating latency spikes and thread starvation.
- Suggested fix: Offload disk sampling to `tokio::task::spawn_blocking` or cache sampled values in a background polling loop.

### [P1] Unbounded SQL queries and response payloads in cron, bot, and allowlist listings
- Location: src/dashboard.rs:1418
- Evidence: async fn list_cron_jobs(State(state): State<DashboardState>) -> Result<Json<Value>, ApiError> {
- Why it matters: `list_cron_jobs`, `list_bots`, and `list_approval_allowlist` perform unrestricted `SELECT *` database queries without `LIMIT` or pagination. Over time as records accumulate, querying these endpoints consumes unbounded memory on the server and client, risking out-of-memory crashes and request timeouts.
- Suggested fix: Apply standard pagination (`PageQuery`) with maximum limit clamping to all listing endpoints.

### [P1] Missing incoming frame/message limits and heartbeat timeouts on WebSocket connections
- Location: src/dashboard.rs:1209
- Evidence: async fn session_ws(
- Why it matters: Neither `session_ws` nor `logs_ws` configures `.max_message_size()` on `WebSocketUpgrade`. Furthermore, connection loops lack idle read timeouts or ping/pong intervals. Orphaned client connections from disconnected networks remain open indefinitely, leaking file descriptors and server memory.
- Suggested fix: Configure `.max_message_size(64 * 1024)` on `WebSocketUpgrade` and add an idle timeout or periodic ping interval.

### [P1] Slow log consumers trigger broadcast channel lag and cascading warning stalls
- Location: src/dashboard.rs:2152
- Evidence:                     Err(broadcast::error::RecvError::Lagged(skipped)) => {
- Why it matters: `handle_logs_socket` reads from a 512-item broadcast channel. If a client socket is slow to drain, `socket.send().await` blocks. Once the broadcast receiver falls behind, it receives `RecvError::Lagged` and immediately attempts to send another text warning over the congested socket, compounding socket buffer saturation and discarding subsequent log entries.
- Suggested fix: Decouple broadcast reception from network transmission using a bounded per-connection buffer or drop lag warnings if the socket is not immediately writable.

### [P1] Tool root defaults authorize the entire home directory to unauthenticated callers
- Location: src/dashboard_runtime.rs:228
- Evidence: fn dashboard_tool_roots() -> Vec<PathBuf> {
- Why it matters: When `OMON_TOOL_ROOTS` is not configured, `dashboard_tool_roots` falls back to `vec![env::var_os("HOME")]`. This grants `TerminalTool` and `FileTool` full authorized read and write permissions across the entire user home directory (including `~/.ssh`, `~/.aws`, `~/.bashrc`). Because chat and session execution endpoints are unauthenticated, any client reaching the dashboard can prompt agents to inspect and modify sensitive files across `$HOME`.
- Suggested fix: Restrict default tool roots to `workspace_root` rather than the user's entire `$HOME` directory unless explicitly overridden.

### [P1] safe_relative_path permits access to dotfiles and does not resolve symlinks outside web root
- Location: src/dashboard.rs:2201
- Evidence: fn safe_relative_path(path: &Path) -> bool {
- Why it matters: `safe_relative_path` validates that path components are `Normal` or `CurDir`, but allows hidden dotfiles (e.g. `.env`, `.git`) because they are valid normal components. If `web_root` contains configuration files or if symlinks point outside `web_root`, `tokio::fs::read` serves the target files without checking if canonical paths remain within the designated web root.
- Suggested fix: Canonicalize the requested path and verify `canonical.starts_with(&state.web_root)`, and reject path components starting with `.`.

### [P2] Dead --insecure CLI and environment option is ignored during host validation
- Location: src/dashboard.rs:100
- Evidence:         if !is_loopback_host(&self.host) {
- Why it matters: `DashboardArgs` and `DashboardSettings` declare an `insecure` flag documented as allowing public non-loopback binding. However, `DashboardSettings::validate` unconditionally errors on non-loopback hosts, ignoring `self.insecure`. Passing `--insecure` or `DASHBOARD_INSECURE=true` has no effect.
- Suggested fix: Either respect `self.insecure` (`if !self.insecure && !is_loopback_host(&self.host)`) or remove the flag and associated documentation.

### [P2] update_cron_job lacks pre-validation for empty cron expression strings
- Location: src/dashboard.rs:1480
- Evidence:     let spec = CronJobSpec {
- Why it matters: Unlike `create_cron_job` which validates `if input.expression.trim().is_empty()` and returns a clean 400 Bad Request, `update_cron_job` passes empty expressions directly to the scheduler, causing a downstream 500 error instead of a 400 validation error.
- Suggested fix: Add `if input.expression.trim().is_empty()` validation to `update_cron_job`.

### [P2] serve_static returns HTTP 200 with HTML for /api requests without trailing slash
- Location: src/dashboard.rs:2175
- Evidence:     if uri.path().starts_with("/api/") {
- Why it matters: `serve_static` only intercepts paths starting with `"/api/"`. A request sent to `"/api"` bypasses this guard and serves `index.html` with HTTP 200 OK rather than returning a 404 JSON response.
- Suggested fix: Check `if uri.path() == "/api" || uri.path().starts_with("/api/")`.

## Strengths
- Strict loopback binding defense: `DashboardSettings::validate` enforces loopback host addresses, and `validate_host_and_origin` validates the HTTP `Host` header against loopback IP addresses, guarding against basic DNS rebinding.
- Same-origin WebSocket enforcement: `validate_host_and_origin` requires the `Origin` header on WebSocket upgrades and checks exact scheme, host, and port parity against the validated `Host` header before accepting connections.
- Parameterized SQL and input pagination: Database queries throughout the module use parameterized bindings (`?`), and session/message listing endpoints clamp pagination parameters via `MAX_PAGE_SIZE = 200` to prevent oversized fetches.
- Bounded in-memory log buffer: `DashboardLogStore` caps in-memory log entries using a fixed-capacity ring buffer (`LOG_CAPACITY = 2000`), preventing unbounded memory growth during continuous tracing.

## Notes
- Single-user local workstation model: The dashboard design assumes a single local developer on a workstation. In this context, loopback binding provides baseline isolation, but local processes and browser CSRF vectors still represent viable attack surfaces.
- Duplicated storage key parsing: `parse_storage_key` is implemented locally in `src/dashboard.rs` rather than reusing the canonical parser in `src/models/session.rs`, introducing maintenance overhead if the serialization format evolves.
