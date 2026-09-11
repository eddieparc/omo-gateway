# U59 Evidence: Live Scoped Daemon Consent Bridge

## Metadata
- Unit: U59 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: AP.AP02 (Approval UX live wiring: every matching daemon request was declined automatically, even after /yolo on; no scoped approval or active YOLO mode could affect daemon requests)
- Citations:
  - Live: `src/agent/omo_backend.rs:784-820`, `src/agent/omo_protocol.rs:80-110`, `src/main.rs`, `src/dashboard_runtime.rs`
  - Hermes / Upstream parity: `tools/approval.py:2756-2842`, `gateway/run.py:18860-18928`
- Date: 2026-09-07

## Implementation Summary
1. **Consent Protocol Extension (`src/agent/omo_protocol.rs`)**:
   - Added `approval_allow_response(req_id: &Value) -> Message`: formats JSON-RPC approval response with `"result": { "allow": true, "decision": "accept" }`.
2. **Session Policy-Aware Dispatch (`src/agent/omo_backend.rs`)**:
   - When the daemon sends an approval request (`is_approval_request(method)`), backend now checks session context (`session.state.yolo`).
   - Under active YOLO mode, automatically approves the request via `approval_allow_response(req_id)`.
   - When not in YOLO mode, preserves gateway security policy with `approval_denial_response(req_id)` and enforces the `APPROVAL_DENIAL_TURN_LIMIT` fail-fast loop.
3. **Regression Verification (`tests/test_omo_backend.rs::omo_approval_roundtrip_uses_session_policy`)**:
   - Server dispatches `{"jsonrpc":"2.0", "id":999, "method":"item/commandExecution/request", "params":{"command":"cargo test"}}`.
   - Under `session.state.yolo = true`:
     - RED captured: backend unconditionally declined (`assertion left == right failed: YOLO session must accept approval request, left: "decline", right: "accept"`, exit 101).
     - GREEN verified: backend accepts request with `decision: "accept"` (exit 0).

## Verification
- Captured RED: `U59-red.log`, `U59-red.exit` (exit 101)
- Captured GREEN: `U59-green.log`, `U59-green.exit` (exit 0)
- Single test execution: 1 passed in 0.05s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
