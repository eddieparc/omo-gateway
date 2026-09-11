# U32 Evidence: Valid Native Voice-Note Transport

## Metadata
- Unit: U32 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: D.D08 (uploader sets flags=8192 for every .ogg/.opus but CreateAttachment has no duration/waveform and no fallback; metadata builder was unused; ordinary OGG files forced into voice mode; forum starter incorrectly flagged)
- Citations:
  - Live: `src/discord/adapter.rs`, `src/models/events.rs`, `src/discord/mod.rs`
  - Hermes / Upstream parity: `plugins/platforms/discord/adapter.py:3450-3539`
- Date: 2026-09-07

## Implementation Summary
1. **Explicit Voice Intent (`is_voice_audio_file`)**:
   - Audio files are only treated as native Discord voice notes when explicit voice intent is present (filename contains `voice-message`, `voice_message`, `voice-note`, `voice_note`, `.voice.ogg`, `.voice.opus`, or content-type explicitly specifies voice / `audio/ogg; codecs=opus`).
   - Ordinary audio files (`ordinary.ogg`, `speech.opus`, `soundtrack.ogg`) remain ordinary document attachments without voice mode flags.
   - Mirrored explicit voice intent in `src/models/events.rs` for inbound prompt attachment labeling.
2. **Wire Attachment JSON Construction (`DiscordUploadTransport` & `DefaultDiscordUploadTransport`)**:
   - Introduced modular `DiscordUploadTransport` trait and wired `SerenityFileUploader::with_transport`.
   - Production implementation `DefaultDiscordUploadTransport::send_voice_file` formats Discord multipart `payload_json` with `flags: 8192` and attachments containing `duration_secs` and sampled `waveform` from `build_voice_metadata`.
3. **Graceful Ordinary File Fallback**:
   - If native voice upload fails or is rejected by Discord API (e.g. channel doesn't allow voice messages, or 400 Bad Request), `SerenityFileUploader::upload` logs a warning and gracefully falls back to `send_ordinary_file` (ordinary attachment, flags 0).
4. **Forum Channel Attachment Safety**:
   - Forum post starter creation stays an ordinary attachment (`flags: 0`) without setting `DISCORD_VOICE_MESSAGE_FLAG` (which is rejected by Discord for forum posts).
5. **Regression Verification (`tests/test_discord_adapter.rs::native_voice_wire_metadata_and_fallback`)**:
   - Verified that surrogate rejects flagged payload lacking metadata, panicking in RED.
   - Verified that with metadata present, voice note is accepted with flags=8192, duration_secs > 0, waveform non-empty.
   - Verified that injected native rejection cleanly falls back to ordinary file send.
   - Verified that ordinary .ogg without voice intent remains document.

## Verification
- Captured RED: `U32-red.log`, `U32-red.exit` (exit 101, panic on missing metadata)
- Captured GREEN: `U32-green.log`, `U32-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 50 passed in 1.04s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
