# U33 Evidence: Configured Encoded-Audio STT

## Metadata
- Unit: U33 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: D.D11 (attachments.rs:195-231 has optional STT hook; main.rs:1373 constructed downloader without with_stt; voice note gets downloaded marker, no transcript; entire encoded file labeled as one Opus frame including WAV; provider failure fell through to downloaded)
- Citations:
  - Live: `src/discord/attachments.rs`, `src/main.rs`, `src/voice/pipeline.rs`, `src/voice/mod.rs`
  - Hermes / Upstream parity: `H:7235-7244`, `gateway/run.py:16631-16706`
- Date: 2026-09-07

## Implementation Summary
1. **WAV Header Decoding to PCM (`decode_wav_pcm`)**:
   - Implemented standard RIFF/WAVE header decoding in `src/discord/attachments.rs`.
   - Extracts sample rate, channels, and decodes 16-bit PCM samples into `AudioPayload::Pcm(samples)` with matching parameters, avoiding labeling raw WAV containers as Opus frames.
2. **Neutral Failure Metadata Handling**:
   - When STT provider succeeds with non-empty transcript: formats `[Voice message transcription]: {transcript}`.
   - When STT provider succeeds with empty transcript: formats `[Voice message: audio received (empty transcript)]`.
   - When STT provider returns an error: logs warning and formats neutral failure metadata `[Voice message: audio received (transcription unavailable)]`, instead of silently reverting to `(audio downloaded)`.
3. **OpenAI / Whisper STT Implementation (`OpenAiSpeechToText`)**:
   - Implemented `OpenAiSpeechToText` in `src/voice/pipeline.rs` conforming to `SpeechToText`. Supports both PCM (auto-encoded to WAV) and Opus audio multipart uploads to `/audio/transcriptions` using `whisper-1`.
   - Wired `downloader = downloader.with_stt(...)` in `src/main.rs` when `openai_api_key` is configured.
4. **Regression Verification (`tests/test_discord_adapter.rs::voice_note_transcription_is_wired`)**:
   - OGG voice note: provider transcript reaches prompt context.
   - WAV voice note: correctly decoded to `AudioPayload::Pcm` with expected sample count and values (RED panicked when WAV was passed as Opus).
   - Provider failure: surfaces neutral failure metadata `[Voice message: audio received (transcription unavailable)]`.

## Verification
- Captured RED: `U33-red.log`, `U33-red.exit` (exit 101, left: ogg transcription, right: wav samples: 4)
- Captured GREEN: `U33-green.log`, `U33-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 52 passed in 1.65s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
