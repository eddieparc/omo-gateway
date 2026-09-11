# U25 Evidence: Lifecycle Cleanup and Balanced Typing Release

## Metadata
- Unit: U25 (Lane 1: Discord Ingress/Egress & Multiplexer Lifecycle)
- Findings: D.D09 (ingress accepted-event typing hook, reaction lifecycle cleanup for start/finish/cancel), S.F14 (balanced typing release across all terminal paths)
- Citations:
  - Live: `src/discord/adapter.rs`, `src/multiplexer/actor.rs`
  - Hermes / Upstream parity: H:2784-2815, HB:5451-5533
- Date: 2026-09-07

## Implementation Summary
1. **Typing Lifecycle**:
   - In `DiscordEgress::dispatch(OutboundAction::SendMessage)`: cleans up `self.typing` guard upon final message dispatch to ensure zero leftover typing guards.
   - In `DiscordEgress::dispatch(OutboundAction::Typing { active: false })`: cleans up `self.typing` guard.
   - Added `DiscordEgress::active_typing_count()` to inspect live typing guards.
2. **Reaction Lifecycle**:
   - In `src/multiplexer/actor.rs`:
     - On `TurnOutcome::Completed(result)`: emits `OutboundAction::React` with `PROCESSING_SUCCESS_EMOJI` or `PROCESSING_FAILURE_EMOJI` replacing start processing emoji (`PROCESSING_START_EMOJI`).
     - On `TurnOutcome::Stopped`: emits `OutboundAction::React` with `PROCESSING_FAILURE_EMOJI` and clears typing.
     - On `TurnOutcome::Shutdown`: emits `OutboundAction::React` with `PROCESSING_FAILURE_EMOJI` and clears typing.
   - In `src/discord/adapter.rs`:
     - Reactions target `session.channel_id` (where the message was originally posted) rather than auto-created `session.thread_id` to ensure reactions hit the actual user message.
3. **Regression Tests**:
   - `tests/test_discord_adapter.rs::processing_lifecycle_cleans_up`: verifies active typing guard creation and terminal release to 0, plus reaction targeting.
   - `tests/test_multiplexer.rs::terminal_outcomes_release_typing`: verifies balanced typing emission across success, error, and concurrent multi-session isolation paths.

## Verification
- `cargo test --test test_discord_adapter processing_lifecycle_cleans_up`: EXIT 0 (GREEN)
- `cargo test --test test_multiplexer terminal_outcomes_release_typing`: EXIT 0 (GREEN)
- Full `test_discord_adapter` suite: 45 passed in 1.04s
- Full `test_multiplexer` suite: 15 passed in 0.41s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
