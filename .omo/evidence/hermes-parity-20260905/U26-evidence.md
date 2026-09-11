# U26 Evidence: Authoritative Serialized Slash Controls

## Metadata
- Unit: U26 (Lane 2: Cron, Multiplexer, Storage & Egress Reliability)
- Findings: D.D12, S.F09 (Slash commands updated SQLite or local state without mutating authoritative multiplexer/backend state; live actor retained stale session state; reset/model/steer did not serialize through multiplexer actor)
- Citations:
  - Live: `src/discord/commands.rs`, `src/multiplexer/actor.rs`, `src/multiplexer/router.rs`, `src/agent/backend.rs`, `src/agent/omo_backend.rs`
  - Hermes / Upstream parity: `gateway/run.py:5001-5030`
- Date: 2026-09-07

## Implementation Summary
1. **Multiplexer Actor Control Serialization (`src/multiplexer/actor.rs`)**:
   - Added `SetModel`, `Reset`, and `GetContext` variants to `ActorCommand`.
   - In `SessionActor::run`, handled these control commands synchronously across both idle state and in-flight turn loops.
   - `SetModel`: mutates active model in live actor context, marks actor dirty, and flushes to storage.
   - `Reset`: clears remote thread binding (`omo_thread_id`) and resets session state to default, shutting down current turn if in-flight.
   - `GetContext`: returns snapshot of authoritative actor session context.
2. **Session Multiplexer API Extensions (`src/multiplexer/router.rs`)**:
   - Added `multiplexer.set_model(key, model)`, `multiplexer.reset(key)`, and `multiplexer.session_context(key)`.
   - Wired live handles to send control commands directly into running session actors while keeping SQLite updated.
3. **Slash Command Handlers Integration (`src/discord/commands.rs`)**:
   - Implemented `execute_model_command(data, key, name)`: routes model change through `data.multiplexer.set_model`.
   - In `execute_reset_command`: after resetting SQLite state, calls `data.multiplexer.reset(key)` so active actor state and remote thread caches are evicted.

## Verification
- Captured RED: `U26-red.log`, `U26-red.exit`
  - `test_discord_adapter::slash_controls_change_authoritative_session` (compile error, `execute_model_command` missing)
  - `test_multiplexer::live_controls_change_authoritative_session` (compile error, `set_model` / `reset` / `session_context` missing)
- Captured GREEN: `U26-green.log`, `U26-green.exit` (all tests passed, exit 0)
- Single test executions:
  - `test_discord_adapter::slash_controls_change_authoritative_session`: ok in 0.02s
  - `test_multiplexer::live_controls_change_authoritative_session`: ok in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
