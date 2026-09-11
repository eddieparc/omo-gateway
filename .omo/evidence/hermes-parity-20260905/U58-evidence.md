# U58 Evidence: Real Per-Agent Policy and Provider Bridge

## Metadata
- Unit: U58 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: CFG.C11, CR.C13, S.F11 (Model and initial system prompt reach daemon; enabled_toolsets and per-job provider/base_url did not; imported provider overrides were dropped; profile tool restrictions were not enforced at daemon boundary)
- Citations:
  - Live: `src/main.rs`, `src/agent/omo_backend.rs`, `src/agent/omo_protocol.rs`, `src/agent/agent_workspace.rs`, `src/cron/executor.rs`, `src/migrate/config_import.rs`
  - Hermes / Upstream parity: `gateway/run.py:14803-14844`, `cron/jobs.py:1017-1070`
- Date: 2026-09-07

## Implementation Summary
1. **Daemon Boundary Tool Allowlist Forwarding (`src/agent/omo_protocol.rs`, `src/agent/omo_backend.rs`)**:
   - Extended `thread_start_request` to serialize `enabledToolsets` param when present on session state.
   - Wired `OmoBackend::resolve_thread_id` to forward `session.state.enabled_toolsets.as_deref()` to `thread/start`.
2. **Profile Tool Restriction Enforcement (`src/agent/omo_backend.rs`)**:
   - In `execute_turn`, incoming tool execution approval requests are gated against `session.state.enabled_toolsets`. If the session restricts toolsets (e.g. `["web"]`) and the requested tool is outside the allowed list (e.g. `commandExecution`), the request is immediately declined by policy, even when YOLO mode is enabled.
3. **Provider Override Preservation in Migration (`src/migrate/config_import.rs`)**:
   - Activated `HermesModel::provider` and mapped it to `LLM_PROVIDER` in generated environment settings rather than discarding it.
4. **Cron Per-Job Provider/Base_Url Overrides & Validation (`src/cron/executor.rs`)**:
   - Preflights per-job `provider` and `base_url`: rejects unsupported provider overrides lacking `base_url` with an explicit configuration error, while honoring valid custom endpoints by populating session metadata.

## Verification
- Captured RED: `U58-red.log`, `U58-red.exit`
  - `test_profile_routing::routed_tools_reach_daemon` (compile error, `omo_protocol` private)
  - `test_migrate::provider_override_is_not_silently_dropped` (assertion failed, left: None, right: Some("custom:my_provider"))
  - `test_cron_executor_parity::overrides_are_honored_or_rejected` (panic: unsupported provider without base_url must be rejected)
  - `test_omo_backend::profile_tool_restrictions_are_enforced` (assertion failed: left: "accept", right: "decline")
- Captured GREEN: `U58-green.log`, `U58-green.exit` (all 4 tests passed, exit 0)
- Single test executions:
  - `test_profile_routing::routed_tools_reach_daemon`: ok in 0.00s
  - `test_migrate::provider_override_is_not_silently_dropped`: ok in 0.00s
  - `test_cron_executor_parity::overrides_are_honored_or_rejected`: ok in 0.05s
  - `test_omo_backend::profile_tool_restrictions_are_enforced`: ok in 0.05s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
