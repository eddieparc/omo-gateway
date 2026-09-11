# U31 Evidence: Safe Final MEDIA Uploads

## Metadata
- Unit: U31 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: D.D07 (MEDIA: extractor was never called in production egress paths, printing directives literally; regex truncated quoted paths with spaces; no path authorization validation)
- Citations:
  - Live: `src/discord/adapter.rs`
  - Hermes / Upstream parity: `gateway/platforms/base.py:5134-5155`, `plugins/platforms/discord/adapter.py:3291-3325`
- Date: 2026-09-07

## Implementation Summary
1. **Quoted-Path-Aware Directive Grammar (`MEDIA_DIRECTIVE_RE` & `extract_media_directives`)**:
   - Updated regex to accurately capture quoted paths with spaces (e.g. `MEDIA:"/tmp/report one.png"`) without truncating at whitespace.
   - Strips the directive completely from the final delivered message text so users never see raw `MEDIA:...` directives.
2. **Path Authorization Validation (`validate_media_path`)**:
   - Canonicalizes input path and validates existence.
   - Enforces an authorized-root security policy: files must reside within the system temp directories (`std::env::temp_dir()`, `/tmp`, etc.) or the active working directory / repository.
   - Explicitly rejects sensitive system paths (`/etc`, `/proc`, `/sys`, `.ssh`, `.gnupg`, etc.) and paths outside authorized roots, returning `Err(OmonError::Config(...))` without leaking files.
3. **Production Wiring into Egress**:
   - In `DiscordEgress::stream`: on final chunk, runs `extract_media_directives`, validates each path via `validate_media_path`, and uploads via `self.file_uploader.upload(http, channel, &valid_path)`.
   - In `DiscordEgress::dispatch(OutboundAction::SendMessage)`: runs `extract_media_directives` and validates/uploads attachments before dispatching message text.
4. **Regression Test (`tests/test_discord_adapter.rs::final_media_directive_uploads`)**:
   - Verifies that `MEDIA:"<path with spaces/report one.png>"` in temp dir triggers `self.file_uploader.upload` exactly once with the file, and strips the directive from the delivered message.
   - Verifies that unauthorized path (`/etc/passwd`) triggers an explicit error and produces zero uploads.

## Verification
- Captured RED: `U31-red.log`, `U31-red.exit` (exit 101, uploader called 0 times instead of 1)
- Captured GREEN: `U31-green.log`, `U31-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 49 passed in 1.06s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
