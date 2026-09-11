# U83 Evidence: Recheck Current Authorization Before Startup Replay

## Metadata
- Unit: U83 (Lane 1: Multiplexer, Router & Chat Protocol)
- Findings: UP.DC01 (Startup recovery replay bypassed current authorization policies; in-flight sessions from previous runs were re-dispatched without checking if their owner user or channel was revoked while the gateway was offline)
- Citations:
  - Live: `src/main.rs`, `src/discord/adapter.rs`, `src/multiplexer/router.rs`
  - Hermes / Upstream parity: `gateway/run_startup.py:423-436, 465-466`, `gateway/run.py:7246-7265`
- Date: 2026-09-07

## Implementation Summary
1. **Startup Authorization Gating (`src/main.rs`)**:
   - Implemented `StartupAuthorization` checking current configured allowed users, allowed channels, and ignored channels.
   - Implemented `recover_resume_pending_sessions_with_auth(pool, multiplexer, auth)` which validates each recovered `SessionKey` against `StartupAuthorization` before re-dispatching unfinished user turns.
   - If a session belongs to a revoked user or ignored channel, recovery skips re-dispatch and logs a security audit warning.
   - Wired live boot recovery in `run_gateway` to construct and enforce `StartupAuthorization` using active `Config` authorization settings.
2. **Regression Verification (`src/main.rs::legacy::runner_tests::recovery_rechecks_current_owner_authorization`)**:
   - In-flight turn recorded for user 42.
   - Current configuration revokes user 42 (`allowed_users = [99]`, `allow_all_users = false`).
   - RED captured: re-dispatched turn without checking current authorization (`assertion left == right failed: left: 1, right: 0`, exit 101).
   - GREEN verified: recovery skips revoked session (`resumed_count == 0`, no event sent, exit 0).

## Verification
- Captured RED: `U83-red.log`, `U83-red.exit` (exit 101, panic: `Revoked user session must NOT be recovered`)
- Captured GREEN: `U83-green.log`, `U83-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
