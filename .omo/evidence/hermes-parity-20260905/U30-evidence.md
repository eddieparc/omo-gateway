# U30 Evidence: Reference Triggering Message on Final Stream

## Metadata
- Unit: U30 (Lane 1: Discord Ingress/Egress & Streaming Message Threading)
- Findings: D.D05 (streaming placeholder and final response omitted triggering message reference / reply threading)
- Citations:
  - Live: `src/models/events.rs`, `src/agent/omo_backend.rs`, `src/discord/adapter.rs`, `src/discord/throttler.rs`
  - Hermes / Upstream parity: `plugins/platforms/discord/adapter.py:2880-2939`
- Date: 2026-09-07

## Implementation Summary
1. **Stream Envelope Extension (`src/models/events.rs`)**:
   - Extended `StreamChunk` struct with `#[serde(default)] pub reply_to: Option<String>`.
   - Maintained backwards compatibility for all serialization / deserialization flows.
2. **Backend Anchor Emission (`src/agent/omo_backend.rs`)**:
   - In `execute_turn`: captures `reply_to` from `event.platform_message_id` when present.
   - Updated `emit_chunk` to pass `reply_to.clone()` in every emitted `StreamChunk`, ensuring stream consumers have access to the triggering platform message ID.
3. **Egress Transport Reference Support (`src/discord/throttler.rs` & `src/discord/adapter.rs`)**:
   - Added `send_message_with_reference` method to `DiscordMessageTransport` and `SerenityMessageTransport`.
   - Made `LiveEditThrottler<T: DiscordMessageTransport + ?Sized>` accept unsized transports, allowing dynamic dispatch via `Arc<LiveEditThrottler<dyn DiscordMessageTransport>>`.
   - Added `with_message_transport` on `DiscordEgress` to allow mocking or custom transport injection.
   - In `DiscordEgress::stream`: when initializing a new stream, the first placeholder message is created with `reference_message((channel, target_msg_id))` if `chunk.reply_to` is present.
   - If the referenced message was deleted or missing (HTTP 404 / Discord code 10008), it logs a warning and automatically retries sending without reference as a fallback.
   - Subsequent chunks (edits) update the existing message without emitting surplus reference messages.
4. **Regression Test (`tests/test_discord_adapter.rs::final_stream_references_trigger`)**:
   - Turn 1: trigger8 -> verifies first stream message references message_id 8, and further chunks (edits) carry no reference.
   - Turn 2: deleted8 -> verifies that if sending with reference returns 10008, egress retries without reference and succeeds without error.

## Verification
- Captured RED: `U30-red.log`, `U30-red.exit` (exit 101, left: None, right: Some("8"))
- Captured GREEN: `U30-green.log`, `U30-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 48 passed in 1.04s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
