# U68 Evidence: Live Backend and Connection Readiness

## Metadata
- Unit: U68 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: R.R09 (HTTP readiness was satisfied by SQLite SELECT 1 and workspace metadata even when the agent backend daemon was dead or socket was refused; operational callers received healthy status while no agent work could run)
- Citations:
  - Live: `src/readiness.rs`, `src/dashboard.rs`, `src/main.rs`, `src/dashboard_runtime.rs`
  - Hermes / Upstream parity: `gateway/readiness.py:26-115`, `gateway/status.py:1044-1119`
- Date: 2026-09-07

## Implementation Summary
1. **Live Backend Probe (`src/readiness.rs`)**:
   - Implemented `probe_backend(appserver_url: Option<&str>) -> CheckResult` testing backend daemon `/health` with bounded timeout (800ms).
   - Reports OK on success and degraded on connection refused, unreachable, or non-200 responses.
   - Wired backend check into `collect_runtime_readiness`.
2. **Dashboard Readiness Gating (`src/dashboard.rs`)**:
   - In `api_readiness`: added `readiness::probe_backend` into readiness calculation.
   - Requires `backend_ok` alongside `db_ok`, `workspace_ok`, and `disk_ok` before returning HTTP 200.
   - Added `/api/ready` route alias for Kubernetes and Hermes readiness conventions.
   - When backend is unavailable, `/api/ready` returns HTTP 503 (SERVICE_UNAVAILABLE) with `"backend": false` and `"status": "degraded"`, while `/api/health` remains HTTP 200.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::readiness_degrades_when_backend_unavailable`)**:
   - Refused daemon socket target (`http://127.0.0.1:59999`).
   - Health check: HTTP 200 OK.
   - Readiness check:
     - RED captured: returned HTTP 200 ready despite dead backend (`assertion left == right failed: left: 200, right: 503`, exit 101).
     - GREEN verified: returned HTTP 503 Service Unavailable with `checks.backend == false` (exit 0).

## Verification
- Captured RED: `U68-red.log`, `U68-red.exit` (exit 101, panic: `Readiness must degrade to 503 when backend is unreachable`)
- Captured GREEN: `U68-green.log`, `U68-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
