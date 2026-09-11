# U28 Evidence: Final Reasoning and Silence Filter

## Metadata
- Unit: U28 (Lane 1: Discord Ingress/Egress & Streaming Output Preparation)
- Findings: D.D01 (reasoning tag leakage in output, intentional silence handling with badge)
- Citations:
  - Live: `src/agent/omo_backend.rs`, `src/discord/adapter.rs`, `src/models/events.rs`
  - Hermes / Upstream parity: `gateway/stream_consumer.py:406-518`, `gateway/response_filters.py:56-80`
- Date: 2026-09-07

## Implementation Summary
1. **Shared Pure Filter Module (`src/models/events.rs`)**:
   - Implemented `filter_reasoning(text: &str) -> String` using regex `(?is)<\s*(?:think|thought|reasoning)\b[^>]*>.*?(?:<\s*/\s*(?:think|thought|reasoning)\s*>|$)` to scrub inline reasoning tags and unclosed reasoning blocks completely.
   - Re-exported and centralized `is_silence_response`, `is_explicit_silence`, and `SILENCE_SENTINELS`.
2. **Output Scrubbing in Backend (`src/agent/omo_backend.rs`)**:
   - In `OmoBackend::execute_turn`: scrubs reasoning with `filter_reasoning(&full_content)` before evaluating content emptiness and before appending tool execution badges.
   - Explicit silence check (`is_explicit_silence(&scrubbed_content)`): if model returns an intentional silence token (e.g. `NO_REPLY`, `[SILENT]`), the turn succeeds without emitting any stream chunks or SendMessage actions, while still running cron ACK command if configured.
3. **Egress Scrubbing in Adapter (`src/discord/adapter.rs`)**:
   - In `DiscordEgress::stream`: filters reasoning before markdown table transformation and markdown chunking.
   - In `DiscordEgress::dispatch(OutboundAction::SendMessage)` and `OutboundAction::EditMessage`: drops intentional silence output and cleans up active typing guards without dispatching network calls.
4. **Regression Test (`tests/test_discord_adapter.rs::final_output_filters_controls`)**:
   - Verified that fake daemon emitting `<ThInK>PRIVATE</ThInK>answer` produces output containing `answer` with no `PRIVATE` or think tags.
   - Verified that a subsequent turn with tool execution and `NO_REPLY` succeeds with zero sent messages or stream chunks.

## Verification
- Captured RED: `U28-red.log`, `U28-red.exit` (exit 101, output contained `<ThInK>PRIVATE</ThInK>answer`)
- Captured GREEN: `U28-green.log`, `U28-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 46 passed in 1.05s
- Full `test_omo_backend` suite: 34 passed in 30.73s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
