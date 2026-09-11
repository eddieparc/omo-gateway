# U84: Disk Pressure Classification Using Absolute Headroom

## Registration Before Production Edits

Exact integration test id: `readiness::tests::disk_pressure_uses_absolute_headroom` (crate `omon_gateway`, module `readiness::tests`).
Literal RED and GREEN command: `cargo test --lib readiness::tests::disk_pressure_uses_absolute_headroom -- --exact --nocapture`

Authoritative upstream specification (`gateway/disk_status.py:19-39`):
- Convert byte values to integer MiB via floor division by 1,048,576 (`// 1048576`).
- Missing/malformed free space (`free > total` or unreadable) or zero/malformed total capacity (`total == 0` or `total_mb == 0`) classifies as `"unknown"`.
- Usage percentage: `used_percent = (1 - free / total) * 100`.
- Worst-first pressure classification:
  - `critical`: `free < 256 MiB` OR (`used >= 95%` AND `free < 1024 MiB`)
  - `elevated`: `free < 512 MiB` OR (`used >= 85%` AND `free < 4096 MiB`)
  - `ok`: otherwise
- Status surfaces expose byte metrics, integer MiB metrics, rounded usage percentage (1 decimal place), and pressure classification.
- Unusable or unreadable filesystems must be reported as `"unknown"` pressure and nonhealthy (`"degraded"`), never `"ok"`.

## Captured Behavioral RED (Before Production Edits)

Literal execution:
`cargo test --lib readiness::tests::disk_pressure_uses_absolute_headroom -- --exact --nocapture`

Pre-production failure captured in `U84-red.log` and `U84-red.exit` (exit code `101`):
```text
running 1 test
case=1 total_mb=1000 free_mb=200 status=ok pct=80 pressure=ok
case=2 total_mb=1000000 free_mb=50000 status=degraded pct=95 pressure=degraded
case=3 total_mb=0 free_mb=0 status=ok pct=0 pressure=ok
test readiness::tests::disk_pressure_uses_absolute_headroom ... FAILED

failures:

failures:
    readiness::tests::disk_pressure_uses_absolute_headroom

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 350 filtered out; finished in 0.07s

thread 'readiness::tests::disk_pressure_uses_absolute_headroom' (10437984) panicked at src/readiness.rs:240:9:
assertion `left == right` failed: sample 1 must be degraded due to critical headroom
  left: "ok"
 right: "degraded"
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
error: test failed, to rerun pass `--lib`
```

Defect analysis confirmed by RED output:
1. Sample 1 (1000 MiB total, 200 MiB free): 80% used was classified as `"ok"` under percentage-only threshold (<90%), ignoring critical headroom floor (<256 MiB).
2. Sample 2 (1,000,000 MiB total, 50,000 MiB free): 95% used was classified as `"degraded"` under percentage-only threshold, ignoring abundant 50 GB free headroom.
3. Sample 3 (0 total, 0 free): Zero capacity was treated as healthy `"ok"` fallback rather than nonhealthy / unknown.

## Production Implementation

### 1. `src/readiness.rs`
- Defined canonical threshold constants:
  - `DISK_BYTES_PER_MB`: `1024 * 1024` (1,048,576 bytes)
  - `DISK_CRITICAL_FREE_FLOOR_MB`: `256`
  - `DISK_CRITICAL_PERCENT_FLOOR`: `95.0`
  - `DISK_CRITICAL_HEADROOM_MB`: `1024`
  - `DISK_ELEVATED_FREE_FLOOR_MB`: `512`
  - `DISK_ELEVATED_PERCENT_FLOOR`: `85.0`
  - `DISK_ELEVATED_HEADROOM_MB`: `4096`
- Implemented `classify_disk_pressure_opt(total_bytes: Option<u64>, free_bytes: Option<u64>) -> &'static str`:
  - Returns `"unknown"` for `None`, `total == 0`, `free > total`, or `total_mb == 0`.
  - Computes integer MiB via integer division by `DISK_BYTES_PER_MB`.
  - Calculates `used_percent = (1.0 - (free_mb as f64 / total_mb as f64)) * 100.0`.
  - Evaluates worst-first: critical, then elevated, else ok.
- Implemented `classify_disk_pressure(total_bytes: u64, free_bytes: u64) -> &'static str`:
  - Wraps `classify_disk_pressure_opt` for standard non-optional byte inputs.
- Updated `calculate_disk_headroom(total_bytes: u64, free_bytes: u64, threshold_pct: f64) -> (String, f64, String)`:
  - Returns `(status, rounded_pct, pressure)`.
  - Degraded whenever pressure != `"ok"`.
  - Returns `("degraded", 0.0, "unknown")` for zero or malformed samples.
- Updated `probe_disk`:
  - Probes total and available space via `fs2`.
  - Exports metrics: `total_bytes`, `free_bytes`, `total_mb`, `free_mb`, `used_percent`, and `pressure`.

### 2. `src/dashboard.rs`
- Introduced narrow injectable seam on `DashboardState`:
  - `pub type DiskSampler = Arc<dyn Fn(&Path) -> (Option<u64>, Option<u64>) + Send + Sync>`
  - Added `pub disk_sampler: Option<DiskSampler>` field.
  - Added `with_disk_sampler` builder method and `sample_disk(&self) -> (Option<u64>, Option<u64>)`.
  - Falls back to `fs2::total_space` and `fs2::available_space` in production.
- Wired `/api/status` (`api_status`):
  - Uses `sample_disk()` and `readiness::calculate_disk_headroom()`.
  - Sets top-level `"status"` to `"ok"` when disk pressure is ok, or `"degraded"` when elevated/critical/unknown.
  - Exposes under `"disk"`:
    - `"status"`: `"ok"` or `"degraded"`
    - `"workspace_total_bytes"`, `"total_bytes"`: byte capacity
    - `"workspace_available_bytes"`, `"available_bytes"`: available bytes
    - `"total_mb"`, `"available_mb"`, `"free_mb"`: integer MiB capacity
    - `"used_percent"`: rounded percentage (1 decimal place)
    - `"pressure"`: `"ok"`, `"elevated"`, `"critical"`, or `"unknown"`
- Wired `/api/readiness` (`api_readiness`):
  - Evaluates database query, workspace metadata, and `disk_ok = workspace_ok && disk_pressure == "ok"`.
  - Returns HTTP 200 `StatusCode::OK` with `"status": "ready"` when ready.
  - Returns HTTP 503 `StatusCode::SERVICE_UNAVAILABLE` with `"status": "degraded"` when disk pressure is elevated, critical, or unknown.
  - Exposes `"checks"` (`database`, `workspace`, `disk`, `chat_runtime`) and `"disk"` details.
- Preserved `/api/health` (`api_health`):
  - Retains live HTTP 200 OK regardless of degraded disk pressure.

## Exact Proof and Verification

### 1. Exact GREEN Regression
Command:
`cargo test --lib readiness::tests::disk_pressure_uses_absolute_headroom -- --exact --nocapture`
- Exit code: 0 (`U84-green.exit`)
- Result: 1 passed, 0 failed (`U84-green.log`)
- Output:
  ```text
  case=1 total_mb=1000 free_mb=200 status=degraded pct=80 pressure=critical
  case=2 total_mb=1000000 free_mb=50000 status=ok pct=95 pressure=ok
  case=3 total_mb=0 free_mb=0 status=degraded pct=0 pressure=unknown
  test readiness::tests::disk_pressure_uses_absolute_headroom ... ok
  ```

### 2. Real Loopback HTTP Surface Proof
Command:
`cargo test --bin omo-gateway legacy::dashboard::tests::disk_pressure_real_loopback_http_surface -- --exact --nocapture`
- Exit code: 0 (`U84-loopback.exit`)
- Result: 1 passed, 0 failed (`U84-loopback.log`)
- Verified across live loopback TCP listener and real Axum router:
  1. Critical headroom (1000 MiB total, 200 MiB free): `/api/status` returns 200 with `status="degraded"`, `pressure="critical"`, `used_percent=80.0`, `total_mb=1000`, `available_mb=200`; `/api/readiness` returns 503 `SERVICE_UNAVAILABLE` with `status="degraded"` and `checks.disk=false`; `/api/health` returns 200 `status="ok"`.
  2. Large capacity ok (1,000,000 MiB total, 50,000 MiB free): `/api/status` returns 200 with `status="ok"`, `pressure="ok"`, `used_percent=95.0`, `total_mb=1000000`, `available_mb=50000`; `/api/readiness` returns 200 `status="ready"` with `checks.disk=true`; `/api/health` returns 200 `status="ok"`.
  3. Zero capacity (0 total, 0 free): `/api/status` returns 200 with `status="degraded"`, `pressure="unknown"`; `/api/readiness` returns 503 `status="degraded"` with `checks.disk=false`; `/api/health` returns 200 `status="ok"`.
  4. Unreadable filesystem (`None, None`): `/api/status` returns 200 with `status="degraded"`, `pressure="unknown"`, `workspace_total_bytes=null`; `/api/readiness` returns 503 `status="degraded"`; `/api/health` returns 200 `status="ok"`.
  5. Elevated boundary (4000 MiB total, 400 MiB free): `/api/status` returns 200 with `status="degraded"`, `pressure="elevated"`, `used_percent=90.0`; `/api/readiness` returns 503 `status="degraded"`; `/api/health` returns 200 `status="ok"`.

### 3. Adjacent Test Suites
- Readiness unit suite (`cargo test --lib readiness::tests -- --nocapture`):
  - Exit code: 0 (`U84-adjacent-readiness.exit`)
  - Result: 6 passed, 0 failed (`U84-adjacent-readiness.log`)
- Dashboard test suite (`cargo test --bin omo-gateway legacy::dashboard::tests -- --nocapture`):
  - Exit code: 0 (`U84-adjacent-dashboard.exit`)
  - Result: 7 passed, 0 failed (`U84-adjacent-dashboard.log`)
  - Verified: Preserved existing U03 approval fixture `approval_lifecycle_local_http_surface` and session/config/cron routes.

### 4. Build, Diagnostics, Format, and Diff
- Build: `cargo build` exit 0 (`U84-build.exit`, `U84-build.log`).
- Compiler diagnostics: `cargo check --all-targets` exit 0, clean.
- Format: `rustfmt --edition 2021 --check src/readiness.rs src/dashboard.rs` exit 0 (`U84-format.exit`, `U84-format.log`).
- Diff: `git diff --check src/readiness.rs src/dashboard.rs` exit 0 (`U84-diff.exit`, `U84-diff.log`).

## Invariant and Boundary Preservation
- Touched ONLY `src/readiness.rs`, `src/dashboard.rs`, and `U84-*` evidence in `.omo/evidence/hermes-parity-20260905/`.
- No modifications to other source files, `.env`, configuration, dependencies, or migrations.
- No git commits or pushes.
