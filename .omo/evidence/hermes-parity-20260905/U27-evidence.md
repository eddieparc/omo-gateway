# U27 Evidence: Unicode-Safe Chunked Slash Replies

## Metadata
- Unit: U27 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: D.D13 (commands.rs /skill read slices UTF-8 at byte 1800 causing panics on multi-byte characters like Korean '한'; other long replies use ctx.say directly bypassing chunker)
- Citations:
  - Live: `src/discord/commands.rs`, `src/discord/mod.rs`
  - Hermes / Upstream parity: `H:4870-4928`, `H:2877`
- Date: 2026-09-07

## Implementation Summary
1. **Character-Safe Preview Truncation (`skill_read_preview`)**:
   - Replaced raw byte slicing `&content[..1800]` with character-boundary-aware truncation.
   - Searches backwards from byte index 1800 until a valid UTF-8 character boundary (`is_char_boundary`) is found, preventing panics on multibyte characters.
2. **Chunked Slash Replies (`chunk_slash_reply`)**:
   - Introduced `chunk_slash_reply(content, limit)` leveraging markdown paginated chunking.
   - Wired `skills` list dispatch to split long lists across multiple safe chunks (<= 2000 chars) instead of a single overflowing `ctx.say`.
3. **Regression Verification (`tests/test_discord_adapter.rs::slash_skill_read_unicode_and_long_output`)**:
   - Verified that 1799 ASCII bytes + 3-byte Korean '한' + tail panicked on the old code at byte index 1800 (RED exit 101).
   - Verified that with character-safe preview, it completes safely with length <= 1800 and clean boundary.
   - Verified that >2000-char list is cleanly chunked into multiple valid bodies <= 2000 chars.

## Verification
- Captured RED: `U27-red.log`, `U27-red.exit` (exit 101, panic: end byte index 1800 is not a char boundary; it is inside '한')
- Captured GREEN: `U27-green.log`, `U27-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 53 passed in 1.67s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
