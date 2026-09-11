# U71 Evidence: Deterministic Causal Test Fixtures

## Metadata
- Unit: U71 (Lane 4: Cross-Cutting Backend, Agent Protocol & Test Suite)
- Findings: AP.AP23, D.D18, S.F16, CFG.C14 (Approval test and egress fixtures relied on arbitrary wall-clock sleeps like 55ms/50ms/60ms instead of causal event subscriptions, causing test flakiness under system load)
- Citations:
  - Live: `src/discord/approval.rs`, `tests/test_discord_adapter.rs`
  - Hermes / Upstream parity: `tools/approval.py:3126-3160`
- Date: 2026-09-07

## Implementation Summary
1. **Causal Heartbeat Subscription (`src/discord/approval.rs`)**:
   - Replaced fixed wall-clock sleeps (`sleep(55ms)` and `sleep(50ms)`) in `test_wait_with_heartbeat_fires_periodically_and_stops_on_resolve` with an asynchronous channel (`tokio::sync::mpsc::unbounded_channel`).
   - The test causally awaits exactly 2 heartbeat events before resolving the approval request, ensuring zero nondeterminism.
   - Verified that the heartbeat task terminates upon resolution (`beat_rx.try_recv().is_err()`) without any trailing sleep.
2. **Zero-Delay Probe Expiry (`tests/test_discord_adapter.rs`)**:
   - Replaced 60ms wall-clock sleep in dead target registry testing with `with_probe_interval(Duration::ZERO)`, verifying immediate trial probe gating deterministically.
3. **Regression Verification**:
   - `cargo test --lib test_wait_with_heartbeat_fires_periodically_and_stops_on_resolve`: 1 passed in 0.02s.
   - `cargo test --test test_discord_adapter discord_egress_handles_typing_start_and_stop`: 1 passed in 0.23s.

## Verification
- Captured GREEN: `U71-green.log`, `U71-green.exit` (exit 0)
- Single test execution: T1 exit 0 (0.02s), T2 exit 0 (0.23s)
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
