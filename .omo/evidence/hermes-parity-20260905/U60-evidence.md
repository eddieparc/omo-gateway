# U60 Evidence: Unified Effective Default Model

## Metadata
- Unit: U60 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: CFG.C12 (README gpt-4o default was actually a required DEFAULT_MODEL causing gateway startup panic when absent or when only OMON_DEFAULT_MODEL was set; dashboard and gateway surfaces had inconsistent default model resolution)
- Citations:
  - Live: `src/main.rs`, `src/dashboard_runtime.rs`, `src/agent/omo_config.rs`
  - Hermes / Upstream parity: `hermes_cli/config.py:6875-6957, 7150-7164`
- Date: 2026-09-07

## Implementation Summary
1. **Unified Resolution Hierarchy (`src/main.rs`)**:
   - Implemented `Config::resolve_default_model() -> String` with strict precedence:
     1. `OMON_DEFAULT_MODEL` (daemon/backend override)
     2. `DEFAULT_MODEL` (standard env configuration)
     3. Documented fallback: `"gpt-4o"`.
   - Replaced `required_env("DEFAULT_MODEL")?` in `Config::from_env()` with `Self::resolve_default_model()`.
2. **Dashboard Synchronization (`src/dashboard_runtime.rs`)**:
   - Implemented `dashboard_runtime::effective_default_model() -> String` delegating directly to `Config::resolve_default_model()`.
   - Updated `dashboard_config_view` to report `effective_default_model()`.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::default_model_precedence_matches_dashboard`)**:
   - Case 1: `OMON_DEFAULT_MODEL=model_a`, `DEFAULT_MODEL=model_b` -> both resolve `model_a`.
     - RED captured: gateway resolved `model_b` ignoring `OMON_DEFAULT_MODEL` (`assertion left == right failed: left: "model_b", right: "model_a"`, exit 101).
     - GREEN verified: both resolve `model_a` (exit 0).
   - Case 2: Only `DEFAULT_MODEL=model_b` -> both resolve `model_b`.
   - Case 3: Both absent -> both resolve to documented fallback `"gpt-4o"` without startup error.

## Verification
- Captured RED: `U60-red.log`, `U60-red.exit` (exit 101, panic: `assertion left == right failed: left: "model_b", right: "model_a"`)
- Captured GREEN: `U60-green.log`, `U60-green.exit` (exit 0)
- Single test execution: 1 passed in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
