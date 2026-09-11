# U65: Dashboard Host Origin and Auth Boundary

## Registration Before Production Edits

Exact integration test id: `legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin` (binary `omo-gateway`, module `legacy::dashboard::tests`).
Literal RED and GREEN command: `cargo test --bin omo-gateway dashboard_rejects_untrusted_host_and_ws_origin -- --nocapture`

Authoritative specification (Hermes parity R07 / `implementation-units.json` / `matrix.md`):
- Enforce accepted Host on HTTP and Host+same-origin on WebSocket.
- Require auth or reject public bind rather than interpreting `--insecure` as authentication. Simplest allowed public-bind policy is reject unauthenticated non-loopback even when `--insecure` was supplied.
- Malicious/rebound Host on HTTP (`GET /api/sessions` with `Host: evil.test`) must be rejected with 400/403 before any session data is returned.
- Untrusted/cross-origin WebSocket (`WS /api/sessions/probe/ws` with `Origin: https://evil.test`) must be rejected with 400/403 during handshake, before any data frame or admitted runtime event.
- Valid local same-origin HTTP and WS requests over real loopback must succeed.
- Synthetic test fixtures must include required Host headers rather than disabling the security boundary.

## Captured Behavioral RED (Before Production Edits)

Literal execution:
`cargo test --bin omo-gateway dashboard_rejects_untrusted_host_and_ws_origin -- --nocapture`

Pre-production failure captured in `U65-red.log` and `U65-red.exit` (exit code `101`):
```text
running 1 test
U65 public bind with insecure=true rejected=false
U65 HTTP GET /api/sessions Host=evil.test status_line=HTTP/1.1 200 OK
U65 WS /api/sessions/probe/ws Origin=https://evil.test connected status=101 Switching Protocols
U65 WS evil origin admitted event="probe"
U65 local HTTP GET /api/sessions Host=127.0.0.1:56255 status_line=HTTP/1.1 200 OK
U65 local WS connected status=101 Switching Protocols
U65 local WS admitted event="probe"

thread 'legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin' (130316) panicked at src/dashboard.rs:2960:9:
public bind must be refused even when insecure=true
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin ... FAILED

failures:

failures:
    legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 32 filtered out; finished in 0.03s

error: test failed, to rerun pass `--bin omo-gateway`
```

Defect analysis confirmed by RED output:
1. Public bind check permitted binding to `0.0.0.0` when `insecure: true`, exposing unauthenticated endpoints on public interfaces.
2. HTTP requests with untrusted Host header (`Host: evil.test`) were accepted with `HTTP/1.1 200 OK` and returned session data.
3. WebSocket upgrade with cross-origin header (`Origin: https://evil.test`) was upgraded with `101 Switching Protocols`, and subsequent message frames were admitted as runtime events to the agent multiplexer.

## Production Implementation

### `src/dashboard.rs`
1. **Public Bind Enforcement (`DashboardSettings::validate`)**:
   - Refuses non-loopback host bindings unconditionally when transport authentication is not configured:
     `refusing to expose the unauthenticated dashboard on non-loopback host {}; public bind requires transport authentication which is not configured`.
   - `--insecure` no longer bypasses this check to expose unauthenticated endpoints publicly.

2. **Host & Port Parsing (`extract_host`, `extract_port`, `is_loopback_host`)**:
   - Parses host and port from `Host` header values and URI authorities, correctly handling bracketed IPv6 (`[::1]:9119`), single-port hostnames (`localhost:9119`, `127.0.0.1:9119`), and portless hosts (`localhost`, `127.0.0.1`).
   - Verifies whether host is a loopback address (`127.0.0.0/8`, `::1`) or `localhost`.

3. **WebSocket Same-Origin Validation (`is_same_origin`)**:
   - Parses `Origin` header and checks authority equality against `Host`.
   - Verifies matching hostname and normalized effective ports (e.g., http/ws default 80, https/wss default 443).

4. **Axum Middleware (`validate_host_and_origin`)**:
   - Added `validate_host_and_origin` layer to `Router::new()` wrapping all HTTP routes, fallback, and WebSocket upgrades.
   - Extracts and verifies `Host` header on all incoming requests; missing host yields `400 Bad Request`, untrusted host yields `403 Forbidden`.
   - On WebSocket upgrade requests (`Upgrade: websocket`), verifies presence and same-origin conformance of `Origin` header against `Host`. Untrusted or missing origins are rejected with `403 Forbidden` prior to upgrade or route execution.

5. **Synthetic Request Fixtures**:
   - Updated synthetic test fixtures in `src/dashboard.rs` unit tests to include `.header(header::HOST, "127.0.0.1")`.

## Exact Proof and Verification

### 1. Exact GREEN Regression
Command:
`cargo test --bin omo-gateway dashboard_rejects_untrusted_host_and_ws_origin -- --nocapture`
- Exit code: 0 (`U65-green.exit`)
- Result: 1 passed, 0 failed (`U65-green.log`)
- Output:
  ```text
  running 1 test
  U65 public bind with insecure=true rejected=true
  U65 HTTP GET /api/sessions Host=evil.test status_line=HTTP/1.1 403 Forbidden
  U65 WS /api/sessions/probe/ws Origin=https://evil.test rejected: HTTP error: 403 Forbidden
  U65 local HTTP GET /api/sessions Host=127.0.0.1:57883 status_line=HTTP/1.1 200 OK
  U65 local WS connected status=101 Switching Protocols
  U65 local WS admitted event="probe"
  test legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 32 filtered out; finished in 0.02s
  ```

### 2. Adjacent Dashboard Tests
Command:
`cargo test --bin omo-gateway dashboard -- --nocapture`
- Exit code: 0 (`U65-adjacent-dashboard.exit`)
- Result: 11 passed, 0 failed (`U65-adjacent-dashboard.log`)
- Suite passed:
  - `legacy::dashboard_runtime::tests::split_csv_trims_and_drops_empty_values`
  - `legacy::dashboard::tests::canonical_session_key_parser_handles_embedded_separators_and_bot_identity`
  - `legacy::dashboard_runtime::tests::dashboard_config_exposes_message_context_policy_matrix`
  - `tests::cli_accepts_dashboard_and_serve_alias`
  - `legacy::dashboard::tests::config_endpoint_never_exposes_provider_secret_values`
  - `legacy::dashboard::tests::health_and_status_endpoints_report_runtime_state`
  - `legacy::dashboard::tests::cron_crud_routes_use_scheduler_semantics`
  - `legacy::dashboard::tests::sessions_list_and_transcript_are_paginated`
  - `legacy::dashboard::tests::disk_pressure_real_loopback_http_surface` (U84)
  - `legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin` (U65)
  - `legacy::dashboard::tests::approval_lifecycle_local_http_surface` (U03)

### 3. Binary Build
Command:
`cargo build --bin omo-gateway`
- Exit code: 0 (`U65-build.exit`)
- Log: `U65-build.log`

### 4. Scoped Format and Diff Cleanliness
- `rustfmt --edition 2021 --check src/dashboard.rs`: exit code 0 (`U65-format.exit`, `U65-format.log`)
- `git diff --check src/dashboard.rs`: exit code 0 (`U65-diff.exit`, `U65-diff.log`)

### 5. Invariant Preservation
- All existing dirty working tree hunks preserved.
- U03 local HTTP approval lifecycle test preserved and green.
- U84 disk pressure headroom test preserved and green.
- File scope restricted strictly to `src/dashboard.rs` and `U65-*` evidence files.
