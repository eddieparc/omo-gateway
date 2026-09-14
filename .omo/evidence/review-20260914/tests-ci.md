# Lane: tests-ci

## Scope
The audit inspected the test suites, build definitions, container deployment specifications, and environment configuration:
- Cargo.toml (62 LOC)
- Dockerfile (32 LOC)
- docker-compose.yml (32 LOC)
- .env.example (196 LOC)
- .gitignore (29 LOC)
- tests/test_agent_tools.rs (322 LOC)
- tests/test_cron_authority_upgrade.rs (241 LOC)
- tests/test_cron_boundary_parity.rs (833 LOC)
- tests/test_cron_delivery_parity.rs (188 LOC)
- tests/test_cron_egress_parity.rs (350 LOC)
- tests/test_cron_executor_parity.rs (523 LOC)
- tests/test_cron_schedule_parity.rs (328 LOC)
- tests/test_cron_tool_parity.rs (207 LOC)
- tests/test_discord_adapter.rs (4457 LOC)
- tests/test_e2e_stress.rs (360 LOC)
- tests/test_katok_digest_page.py (66 LOC)
- tests/test_message_context.rs (434 LOC)
- tests/test_migrate.rs (1116 LOC)
- tests/test_multiplexer.rs (1673 LOC)
- tests/test_omo_backend.rs (4852 LOC)
- tests/test_profile_routing.rs (500 LOC)
- tests/test_review_agent.rs (781 LOC)
- tests/test_review_dashboard.rs (844 LOC)
- tests/test_review_discord.rs (792 LOC)
- tests/test_review_security.rs (1012 LOC)
- tests/test_voice_cron.rs (1367 LOC)
- tests/test_wiring_e2e.rs (362 LOC)

Total target scope: 21,959 LOC across 27 files. Target references in `src/` were also audited to verify module test mapping, dead code, and environment variable drift.

## Findings

### [P0] Dockerfile fails to build because binary target is named omo-gateway, not omon-gateway
- Location: Dockerfile:8
- Evidence: RUN cargo build --locked --release --bin omon-gateway
- Why it matters: Cargo.toml defines `[[bin]] name = "omo-gateway"`, but Dockerfile line 8 invokes `cargo build --locked --release --bin omon-gateway`. Executing `docker build` fails unconditionally at the build stage with `error: no bin target named 'omon-gateway' in default-run packages`. Furthermore, line 20 (`COPY --from=builder ... omon-gateway`), line 29 (`grep -aq "omon-gateway" /proc/1/cmdline`), and line 31 (`ENTRYPOINT ["/usr/local/bin/omon-gateway"]`) all refer to the non-existent binary name, rendering the container unbuildable and unrunnable.
- Suggested fix: Replace `omon-gateway` with `omo-gateway` across lines 8, 20, 29, and 31 of `Dockerfile`.

### [P1] Fixed-port collision on 127.0.0.1:29998 between test suites prevents concurrent test execution
- Location: tests/test_review_dashboard.rs:308
- Evidence:         let interactive_listener = TcpListener::bind("127.0.0.1:29998").await.map_err(|e| {
- Why it matters: `tests/test_review_dashboard.rs:308` hardcodes binding to port 29998 and fails immediately if binding fails or if the configured backend URL is not `ws://127.0.0.1:29998`. Concurrently, `tests/test_review_agent.rs:644` defines `const INTERACTIVE: &str = "ws://127.0.0.1:29998";` and spawns child test processes pointing to port 29998. Cargo test executes integration test crates concurrently by default; running the test suite in parallel causes address bind collisions (`AddrInUse`), leading to non-deterministic test failures.
- Suggested fix: Dynamically bind to `127.0.0.1:0` in `test_review_dashboard.rs` and inject the ephemeral port into the backend configuration rather than requiring exclusive ownership of fixed port 29998.

### [P1] Docker-compose passes sensitive API keys as plain environment variables without Docker secrets
- Location: docker-compose.yml:21
- Evidence:       OPENAI_API_KEY: ${OPENAI_API_KEY:-}
- Why it matters: Production secrets (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `DISCORD_BOT_TOKEN`, `DISCORD_BOT_TOKENS`) are passed directly via container environment variables in lines 14, 15, 21, and 23. Any process or user with access to `docker inspect`, container logs, or `/proc/*/environ` can extract unencrypted production credentials, violating container security baselines.
- Suggested fix: Adopt Docker Compose `secrets:` mounted under `/run/secrets/` or file-based credential loading within the application rather than populating sensitive keys directly in `environment:`.

### [P1] Docker-compose fails to expose or publish dashboard HTTP port 9119
- Location: docker-compose.yml:2
- Evidence:   omon-gateway:
- Why it matters: The gateway exposes a web dashboard service defaulting to port 9119 (`DASHBOARD_PORT=9119`), but `docker-compose.yml` does not define a `ports:` section and `Dockerfile` lacks `EXPOSE 9119`. Deploying the stack with `docker compose up` leaves the web dashboard inaccessible from the host network without manual intervention.
- Suggested fix: Add `ports: - "127.0.0.1:9119:9119"` under `services.omon-gateway` in `docker-compose.yml` and add `EXPOSE 9119` in `Dockerfile`.

### [P1] Flaky test race in test_wiring_e2e.rs using fixed sleep for async actor execution
- Location: tests/test_wiring_e2e.rs:273
- Evidence:     tokio::time::sleep(std::time::Duration::from_millis(150)).await;
- Why it matters: After dispatching an inbound message via `multiplexer.route(event)`, the test relies on a blind 150ms sleep to wait for the actor task to process the event, invoke the backend, and persist records into SQLite. Under high CPU contention or slow CI runners, async execution often exceeds 150ms, causing immediate test assertion failures on missing SQLite rows.
- Suggested fix: Replace `sleep(150ms)` with a bounded polling loop or subscribe to a completion channel/event that signals when message persistence has completed.

### [P1] Sleep-based polling in test_cron_schedule_parity.rs causes test nondeterminism
- Location: tests/test_cron_schedule_parity.rs:201
- Evidence:     tokio::time::sleep(std::time::Duration::from_millis(50)).await;
- Why it matters: `tests/test_cron_schedule_parity.rs` uses fixed 50ms sleeps at lines 201, 218, and 300 to wait for background execution tasks spawned by `scheduler.run_due_jobs()` to complete and write run records to SQLite. When thread pool scheduling or SQLite writes take longer than 50ms, subsequent assertions fail sporadically.
- Suggested fix: Await the notification channel from `scheduler.subscribe()` or poll the database with a bounded timeout rather than sleeping.

### [P1] test_katok_digest_page.py hardcodes developer home directory, breaking external test runners
- Location: tests/test_katok_digest_page.py:8
- Evidence: SCRIPT = Path.home() / ".omon/workspace/runtime/scripts/katok_group_digest.py"
- Why it matters: The Python test residing in `tests/` imports an external script located at `~/.omon/workspace/runtime/scripts/katok_group_digest.py` on the local developer's machine. Any test runner running `pytest tests/` on CI or another developer's workstation immediately fails with an uncaught `AssertionError`. In addition, `cargo test` does not execute Python files, leaving this test unverified by the Rust build system.
- Suggested fix: Check in the required script into the repository workspace and load it relative to `Path(__file__).parent.parent`, or move the test into a dedicated integration/script directory outside `tests/`.

### [P2] Incomplete .env.example: truncated APPROVAL_MODE and missing operational environment variables
- Location: .env.example:168
- Evidence: # APPROVAL_MODE is enforced by the terminal SmartApprovalGuard. "smart" requests
- Why it matters: Line 168 in `.env.example` ends abruptly mid-sentence and omits the actual variable definition `APPROVAL_MODE=smart`, despite `src/main.rs:222` and `docker-compose.yml:24` actively reading `APPROVAL_MODE`. In addition, multiple operational environment variables parsed by `src/` are omitted from `.env.example`: `APPROVALS_DESTRUCTIVE_SLASH_CONFIRM` (`src/main.rs:136`), `DASHBOARD_INSECURE` (`src/dashboard.rs:85`), `DASHBOARD_WEB_ROOT` (`src/dashboard.rs:84`), `CRON_RUNS_RETENTION_DAYS` (`src/cron/store.rs:43`), `OMON_DISCORD_RECEIVE_WATCHDOG_SECS` (`src/discord/adapter.rs:460`), and OMO backend variables `OMON_OMO_APPSERVER_URL`, `OMON_OMO_CRON_APPSERVER_URL`, and `OMON_PER_AGENT_WORKSPACE` (`src/agent/omo_config.rs:28, 43, 125`). Operators configuring new environments from `.env.example` will miss critical configuration options.
- Suggested fix: Complete the `APPROVAL_MODE=smart` definition in `.env.example` and document all operational, dashboard, and OMO backend configuration variables with sane defaults.

### [P2] Test asserts nothing meaningful in discord_egress_handles_typing_start_and_stop
- Location: tests/test_discord_adapter.rs:1357
- Evidence: async fn discord_egress_handles_typing_start_and_stop() {
- Why it matters: The test initializes `DiscordEgress` with a dummy HTTP token, dispatches typing start and typing stop actions, and asserts nothing. It does not verify that any HTTP request was generated, that throttler state was updated, or that any channel event was emitted. If typing dispatch breaks completely, this test still passes.
- Suggested fix: Use a mock HTTP transport to capture and assert outbound typing requests or inspect the egress actor state.

### [P2] Windows-only test dash_011_012_runtime_and_surface silently passes on non-Windows platforms
- Location: tests/test_review_dashboard.rs:571
- Evidence:         if !cfg!(windows) {
- Why it matters: Lines 571-574 inspect `if !cfg!(windows)` and immediately return `Ok(())` after printing an eprintln message. On macOS and Linux CI environments, this test registers as passed despite evaluating zero assertions.
- Suggested fix: Add `#[cfg(windows)]` attribute to the test function so it is properly marked as ignored/skipped on non-Windows targets rather than pretending to pass.

### [P2] Zero test coverage for voice processing pipeline (src/voice/pipeline.rs and src/voice/mod.rs)
- Location: src/voice/pipeline.rs:1
- Evidence: use std::collections::VecDeque;
- Why it matters: `src/voice/pipeline.rs` (350 LOC) and `src/voice/mod.rs` (66 LOC) define `OpenAiSpeechToText`, `SpeechPipeline`, `VoiceAudioPipeline`, `VoiceLanguageModel`, `TextToSpeech`, and `SongbirdAudioEventListener`. Neither module contains unit tests (`#[cfg(test)]`), and no integration tests exercise the audio pipeline or STT/TTS operations (only `AudioFrame` and `AudioFrameBuffer` struct serialization are touched in `test_voice_cron.rs`). Any regression in voice event handling, audio chunk framing, or STT transcription will go undetected.
- Suggested fix: Add unit tests for `SpeechPipeline`, `VoiceAudioPipeline`, and `OpenAiSpeechToText` using mock audio frames and mock LLM/STT responses.

### [P2] Zero test coverage for browser automation tool (src/tools/browser.rs)
- Location: src/tools/browser.rs:8
- Evidence: pub struct BrowserTool {
- Why it matters: `BrowserTool` (`src/tools/browser.rs`, 137 LOC) implements CDP browser automation (`navigate`, `snapshot`, `eval`, `screenshot`) and is registered in `src/main.rs:649` and `src/dashboard_runtime.rs:107`. The module contains no unit tests and is never invoked in any integration test under `tests/`. URL scheme validation failures or CDP protocol parsing errors are uncovered by tests.
- Suggested fix: Add unit tests validating tool action schema handling, URL restriction checks, and mock CDP responses.

### [P2] Zero test coverage for lazy message context provider (src/tools/message_context_lazy.rs)
- Location: src/tools/message_context_lazy.rs:12
- Evidence: pub struct LazyDiscordMessageContextProvider {
- Why it matters: `LazyDiscordMessageContextProvider` (`src/tools/message_context_lazy.rs`, 69 LOC) lazily initializes the Discord message context provider and pool from environment variables. It has zero unit tests and is not referenced in any test file under `tests/`.
- Suggested fix: Add unit tests checking token parsing from `DISCORD_BOT_TOKEN` and `DISCORD_BOT_TOKENS` and verifying once-initialization mechanics.

### [P2] Dead code without test coverage: DeliveryReceipt in src/models/ledger.rs
- Location: src/models/ledger.rs:17
- Evidence: pub struct DeliveryReceipt {
- Why it matters: `src/models/ledger.rs` (42 LOC) defines `DeliveryReceipt` and `DeliveryStatus`, re-exported in `src/models/mod.rs`. These structs are never used, instantiated, or tested anywhere in `src/` or `tests/`. They constitute dead code that increases cognitive load and maintenance burden.
- Suggested fix: Either integrate `DeliveryReceipt` into `DeliveryLedgerService` with tests or remove `src/models/ledger.rs` and its re-export.

### [P2] Temporary directory and file leak across runs in test_discord_adapter.rs
- Location: tests/test_discord_adapter.rs:1385
- Evidence: fn test_workspace(label: &str) -> PathBuf {
- Why it matters: `test_workspace` creates unmanaged paths under `std::env::temp_dir()` without wrapping them in a `tempfile::TempDir` RAII guard. Tests at lines 1144, 1199, 1243, 1271, and 1333 create directories and write files that persist indefinitely in `/tmp`, leaking disk space over repetitive test executions.
- Suggested fix: Return a `tempfile::TempDir` from `test_workspace` or use RAII cleanup guards so temporary test files are automatically pruned when tests finish.

### [P2] Duplicate directory entry in .gitignore
- Location: .gitignore:21
- Evidence: web/node_modules/
- Why it matters: `web/node_modules/` is listed twice in `.gitignore` (line 15 under `# Web & Node dependencies` and line 21 under `# Apps & packaging`), creating redundant noise.
- Suggested fix: Remove line 21 from `.gitignore`.

## Strengths
- Extensive multi-layered integration tests for multiplexer routing, session actor lifecycle, and cron execution parity (`test_multiplexer.rs`, `test_cron_boundary_parity.rs`, `test_cron_executor_parity.rs`).
- Consistent use of in-memory SQLite (`sqlite::memory:`) running actual SQL migrations for clean, isolated database testing without touching host filesystems.
- Strong negative fault-injection coverage in security and delivery ledger suites (`test_review_agent.rs`, `test_review_security.rs`) asserting that partial write failures never leave delivery claims in uncommitted states.

## Notes
- Several test harness environment variables (`U06_CASE`, `U07_CHILD`, `U34_PNG`, `U62_ISOLATED_TEST`, `U63_DRAIN_DIR`, `U63_DRAIN_ROLE`) are directly referenced in production source files. While they serve internal integration testing, they create undocumented behavioral escape hatches in runtime code.
