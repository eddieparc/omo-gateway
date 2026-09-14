# Lane: agent-workspace

## Scope
- `src/agent/omo_config.rs`: 440 LOC, read fully, including unit tests.
- `src/agent/agent_workspace.rs`: 218 LOC, read fully, including unit tests.
- `src/agent/workspace_migration.rs`: 121 LOC, read fully, including unit tests.
- `src/agent/llm.rs`: 728 LOC, read fully, including unit tests.
- `src/agent/mod.rs`: 18 LOC, read fully.

Total: 1,525 LOC across 5 files.

## Findings

### [P0] Boot-time migration wipes active session thread bindings on every gateway reboot
- Location: src/agent/workspace_migration.rs:15
- Evidence: `        "UPDATE sessions SET state_json = json_remove(state_json, '$.metadata.omo_thread_id') WHERE json_extract(state_json, '$.metadata.omo_thread_id') IS NOT NULL",`
- Why it matters: `wipe_omo_thread_bindings` was created as a one-time upgrade migration to clear legacy global-workspace thread bindings. However, in `src/main.rs:480`, it is called unconditionally on every gateway boot whenever `omo_config.per_agent_workspace` is true (the default). It lacks any migration ledger check, schema version barrier, or distinction between legacy vs per-agent threads. On every gateway restart or crash recovery, it wipes `omo_thread_id` from all active sessions. Subsequent user turns cannot call `thread/resume` and are forced to execute `thread/start`, wiping active conversational context and causing silent session state loss.
- Suggested fix: Gate the execution of `wipe_omo_thread_bindings` behind an idempotent migration tracking table or cutover receipt so it runs exactly once during the initial migration rather than on every boot.

### [P0] Unvalidated workspace slug allows path traversal and arbitrary directory breakout
- Location: src/agent/agent_workspace.rs:79
- Evidence: `    let cwd = base.join("agents").join(slug);`
- Why it matters: `resolve_workspace` is a public function that directly joins `slug` to `base.join("agents")` without verifying that `slug` is a safe, single `Component::Normal`. In standard Rust path semantics, if `slug` is an absolute path (e.g. `/var/log` or `/etc`), `Path::join` discards the base entirely and resolves to the root path. If `slug` contains `..` components, it traverses upward out of `agents`. If `slug` is empty `""`, `cwd` collapses to `base/agents`. Downstream in `omo_backend.rs:311`, the runtime creates directories and writes `.omo/omo.json` into `ws.cwd`, allowing arbitrary directory writes and configuring unauthorized execution roots.
- Suggested fix: Validate that `slug` contains no path separators or parent components (`..`), matches the expected `[a-z0-9_-]` slug format, and verify with `starts_with` that `cwd` remains strictly inside `base.join("agents")`.

### [P1] Multi-byte UTF-8 corruption across stream chunk boundaries
- Location: src/agent/llm.rs:245
- Evidence: `                buffer.push_str(&String::from_utf8_lossy(&bytes));`
- Why it matters: The background stream task decodes each raw byte chunk from `response.bytes_stream()` immediately using `String::from_utf8_lossy(&bytes)` before appending to `buffer`. If a multi-byte UTF-8 character (such as non-ASCII Korean/Japanese/Chinese characters, emojis, or Unicode symbols) is split across network packet or HTTP chunk boundaries, the partial byte sequence at the boundary cannot be decoded and is permanently replaced with `U+FFFD` () replacement characters. The resulting stream received by the user suffers irreversible text corruption.
- Suggested fix: Accumulate raw bytes in a `Vec<u8>` buffer and only slice and decode complete newline-delimited byte lines into UTF-8 strings.

### [P1] Mid-stream SSE errors are silently dropped and reported as successful completions
- Location: src/agent/llm.rs:250
- Evidence: `                    for content in parse_stream_line(provider, &line) {`
- Why it matters: When an upstream LLM provider encounters a mid-generation failure (such as context window exhaustion, token rate limits, or provider outages), it emits a JSON error payload over the SSE connection (e.g. OpenAI `{"error": ...}` or Anthropic `{"type": "error", ...}`). `parse_stream_line` ignores non-text events and returns an empty vector, while `accumulate_stream_tool_calls` ignores error structures. When the network stream terminates, the spawned task sends `StreamChunk { is_final: true, content: String::new(), .. }` wrapped in `Ok`, causing the gateway to treat an aborted generation as an empty success.
- Suggested fix: Check parsed SSE objects for `"error"` keys or error types in `parse_stream_line`, and forward an `Err(OmonError::Llm(...))` to the channel to abort the stream with a typed error.

### [P1] Missing HTTP client timeouts and retry logic causes task leaks on stalled streams
- Location: src/agent/llm.rs:152
- Evidence: `        let http = reqwest::Client::builder()`
- Why it matters: `reqwest::Client::builder()` does not configure connect, read, or total timeouts. Additionally, the streaming loop in `stream_with_tool_calls` reads from `bytes.next().await` without any bounded timeout or cancellation check. If an LLM provider endpoint hangs, stalls mid-stream, or drops TCP packets without sending FIN/RST, the spawned background task blocks indefinitely. The task never terminates, leaking TCP connections and runtime task handles until the process restarts.
- Suggested fix: Set explicit `.connect_timeout(...)` and `.timeout(...)` on `reqwest::Client::builder()`, and wrap `bytes.next()` with `tokio::time::timeout`.

### [P1] Synchronous file read on Tokio runtime thread blocks async reactor
- Location: src/agent/llm.rs:461
- Evidence: `    let bytes = std::fs::read(path).ok()?;`
- Why it matters: `encoded_image` calls synchronous `std::fs::read(path)` on the local filesystem. This function is called within `build_payload`, which executes synchronously on Tokio worker threads during `stream` and `stream_with_tool_calls`. When reading large image attachments (which can be tens of megabytes on Discord), synchronous disk I/O stalls the Tokio reactor thread, introducing latency spikes and starving other concurrent async tasks on the executor.
- Suggested fix: Use `tokio::fs::read(path)` asynchronously before payload construction or offload the read using `tokio::task::spawn_blocking`.

### [P1] Discarding platform parameter in slug generation causes cross-platform workspace collisions
- Location: src/agent/agent_workspace.rs:48
- Evidence: `        ("user", user_id)`
- Why it matters: `agent_workspace_slug` accepts `platform: &str` but ignores it for all `user` and `bot` sessions (it only checks `platform == "web"`). If users or bots across different platforms (e.g. Discord vs Slack or Telegram) have identical IDs (such as common numeric IDs or bot names), their workspaces resolve to the exact same slug (e.g. `user-1001` or `bot-helper`). Both sessions then share the same cwd, persistent `.omo` memory identity, and agent workspace files, violating cross-platform tenant isolation.
- Suggested fix: Incorporate the platform prefix into the slug category (e.g. `format!("{platform}-user-{sanitized}")` or `format!("{category}-{platform}-{sanitized}")`).

### [P1] Sensitive credentials and API keys leaked in plaintext via derived Debug
- Location: src/agent/llm.rs:28
- Evidence: `    pub api_key: Option<String>,`
- Why it matters: Both `LlmConfig::api_key` (`src/agent/llm.rs:28`) and `OmoBackendConfig::auth_token` (`src/agent/omo_config.rs:41`) are stored as raw `Option<String>` fields on structs that derive `Debug`. When configurations are formatted in tracing log messages, test failure reports, readiness checks, or error displays, secret API tokens and daemon authentication tokens are written to log outputs and console streams in plaintext.
- Suggested fix: Implement custom `Debug` implementations for `LlmConfig` and `OmoBackendConfig` that redact `api_key` and `auth_token` as `"[REDACTED]"`.

### [P2] Parser accepts 0-second timeouts violating positive integer requirement
- Location: src/agent/omo_config.rs:151
- Evidence: `                Duration::from_secs(v.trim().parse::<u64>().map_err(|_| {`
- Why it matters: Parsing for `OMON_OMO_TURN_TIMEOUT_SECS`, `OMON_OMO_TURN_TOTAL_TIMEOUT_SECS`, and `OMON_OMO_CRON_TURN_TOTAL_TIMEOUT_SECS` accepts `"0"` because `0u64` parses successfully. The error message explicitly promises `"expected a positive integer"`, but zero is not rejected. Setting any of these environment variables to `"0"` sets a 0-second duration, immediately failing every turn due to instant deadline expiration.
- Suggested fix: Add `.filter(|&secs| secs > 0)` or validate `if val == 0 { return Err(OmonError::Config(...)); }`.

### [P2] Endpoint builder misidentifies pure numeric path segments as version prefixes
- Location: src/agent/llm.rs:94
- Evidence: `        let digits = segment.strip_prefix('v').unwrap_or(segment);`
- Why it matters: `ends_with_version_segment` falls back to `unwrap_or(segment)`. Any URL whose final path segment consists entirely of ASCII digits (e.g. `http://proxy:8080/models/42` or `http://proxy:8080/deployments/1`) satisfies `digits.bytes().all(is_ascii_digit)` and is incorrectly identified as an API version prefix. As a result, `endpoint()` appends `/chat/completions` instead of `/v1/chat/completions`, producing an invalid 404 URL. Additionally, `base.trim_end_matches('/')` in `endpoint()` fails to trim trailing whitespace, corrupting URLs if environment variables contain whitespace.
- Suggested fix: Require that version prefixes begin with `'v'` or `'V'` followed by digits, and ensure `base_url.trim()` strips whitespace.

### [P2] Missing default workspace root in config silently disables per-agent workspaces
- Location: src/agent/omo_config.rs:201
- Evidence: `        let workspace_root = std::env::var_os("OMON_WORKSPACE_ROOT").map(PathBuf::from);`
- Why it matters: `OmoBackendConfig::from_env()` defaults `per_agent_workspace` to `true`, but reads `OMON_WORKSPACE_ROOT` without falling back to the standard application default (`~/.omon/workspace`, as configured in `main.rs:150`). If `OMON_WORKSPACE_ROOT` is unset, `workspace_root` remains `None`. In `omo_backend.rs:305`, `self.config.workspace_root` being `None` causes the backend to silently skip per-agent workspace provisioning and fall back to the global workspace without warning or error.
- Suggested fix: Default `workspace_root` in `OmoBackendConfig::from_env()` to `~/.omon/workspace` when `OMON_WORKSPACE_ROOT` is absent.

### [P2] Trailing hyphen and collision on empty cron job identifier in slug generation
- Location: src/agent/agent_workspace.rs:54
- Evidence: `    let initial_slug = format!("{category}-{sanitized}");`
- Why it matters: When `user_id` is `"cron:"` (a prefix without a job ID), `strip_prefix("cron:")` returns `Some("")`. `sanitized` and `raw_lower` are both `""`. Because `sanitized == raw_lower` evaluates to true, the hash-suffix fallback is bypassed, outputting `"cron-"` with an illegal trailing hyphen. Any cron jobs with empty job IDs collide on this invalid slug.
- Suggested fix: Add a check for `if raw_value.is_empty()` and fall back to `"cron-default"` or append the hash suffix.

### [P2] Cron lane total timeout can invert below inherited request gap timeout
- Location: src/agent/omo_config.rs:233
- Evidence: `        config.total_timeout = match std::env::var("OMON_OMO_CRON_TURN_TOTAL_TIMEOUT_SECS") {`
- Why it matters: In `cron_from_env()`, `total_timeout` defaults to 600s or is overridden by `OMON_OMO_CRON_TURN_TOTAL_TIMEOUT_SECS`. However, `request_timeout` is inherited from `from_env()` without modification (default 600s, or larger if `OMON_OMO_TURN_TIMEOUT_SECS` was configured). If `OMON_OMO_CRON_TURN_TOTAL_TIMEOUT_SECS` is set to e.g. 180s, `config.request_timeout` (600s) exceeds `config.total_timeout` (180s), causing an inverted timeout configuration where the single-event gap tolerance exceeds the entire turn budget.
- Suggested fix: In `cron_from_env()`, clamp `config.request_timeout = config.request_timeout.min(config.total_timeout)`.

## Strengths
- Fast-fail backend validation: `validate_agent_backend_value` proactively rejects deprecated direct-LLM backends (`llm`, `hermes`, `direct`) at startup with informative error messages, preventing ambiguous partial configurations.
- Deterministic slug length bounding: `agent_workspace_slug` strictly constrains directory slug length to <= 48 characters using category overhead calculation and gracefully truncates while appending a SHA-256 hash tail to prevent namespace collisions.
- Unified streaming abstraction: `LlmClient::stream_with_tool_calls` provides a cohesive SSE line framing and tool call parsing interface across heterogeneous providers (Anthropic, OpenAI, DeepSeek, Ollama).
- Non-destructive metadata updates: `wipe_omo_thread_bindings` uses SQLite JSON1 `json_remove` to strip only the `omo_thread_id` key without mutating adjacent session metadata fields.

## Notes
- Residual `LlmClient` lifecycle: While `validate_agent_backend_value` strictly mandates the `omo` app-server backend for primary agent turns, `LlmClient` remains in `poise_data.llm` (optionally initialized from `main.rs:804`) and is used for non-agent fallbacks. Its lack of timeouts and UTF-8 handling defects present residual risk whenever this client path is triggered.
- Workspace root wiring: In `src/main.rs` and `src/dashboard_runtime.rs`, callers invoke `.with_workspace_root(...)` immediately after `OmoBackendConfig::from_env()`, which compensates for `from_env()` leaving `workspace_root` as `None`. However, any secondary caller using `from_env()` directly will silently lose per-agent workspace isolation.
