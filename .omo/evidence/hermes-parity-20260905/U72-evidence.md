# U72 Evidence: Bound Final Response Message Count

## Metadata
- Unit: U72 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: UP.IO-01 (No per-response Discord message-count ceiling; runaway/oversized outputs could flood channel with unbounded messages; upstream Hermes adapter bounds split messages to 8 with truncation notice)
- Citations:
  - Live: `src/discord/throttler.rs`, `src/discord/adapter.rs`, `src/discord/mod.rs`
  - Hermes / Upstream parity: `plugins/platforms/discord/adapter.py:996`, `2787-2807`, `2809-2895`
- Date: 2026-09-07

## Implementation Summary
1. **Per-Response Split Message Ceiling (`MAX_SPLIT_MESSAGES` & `bound_split_messages`)**:
   - Defined `MAX_SPLIT_MESSAGES = 8` and truncation notice `"

… [Response truncated: output exceeded Discord 8-message limit]"`.
   - Implemented `bound_split_messages(chunks, max_messages)`: if chunk count exceeds `max_messages`, keeps the first `max_messages` chunks, and truncates the final kept chunk to make room for the truncation notice without exceeding the 2000-character Discord limit.
2. **Wiring into Streaming and Outbound Dispatch**:
   - In `LiveEditThrottler::flush`: passes split chunks through `bound_split_messages`. Stale intermediate message IDs exceeding the ceiling are cleanly pruned via `delete_message`.
   - In `DiscordEgress::dispatch(OutboundAction::SendMessage)`: bounds split chunks through `bound_split_messages` before transmission.
3. **Regression Verification (`tests/test_discord_adapter.rs::completed_response_has_bounded_message_count`)**:
   - Tested 23KB input splitting into 12 chunks.
   - RED captured: throttler emitted 11 sent messages (total 12) without bounding (RED exit 101).
   - GREEN verified: total messages strictly capped to 8 (`MAX_SPLIT_MESSAGES`), and the 8th message contains the explicit truncation notice.

## Verification
- Captured RED: `U72-red.log`, `U72-red.exit` (exit 101, panic: Total sent messages must not exceed MAX_SPLIT_MESSAGES (8), got 11)
- Captured GREEN: `U72-green.log`, `U72-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 54 passed in 1.68s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
