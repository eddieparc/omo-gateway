# U35 Evidence: Forum and Batched PNG Delivery

## Metadata
- Unit: U35 (Lane 1: Discord Ingress/Egress & Media Directive Pipeline)
- Findings: D.D16 (Table PNG delivery: forum SendMessage returned before attachment branch so tables replaced by placeholders but PNGs never uploaded; all tables sent in 1 request exceeding Discord's 10-file limit; upload errors swallowed leaving delivery acknowledged)
- Citations:
  - Live: `src/discord/adapter.rs`, `src/discord/mod.rs`
  - Hermes / Upstream parity: `plugins/platforms/discord/adapter.py:3007-3063`, `3336-3380`
- Date: 2026-09-07

## Implementation Summary
1. **Batched PNG Attachment Delivery (`DISCORD_ATTACHMENT_LIMIT` & `dispatch_rendered_tables`)**:
   - Defined `DISCORD_ATTACHMENT_LIMIT = 10` per Discord API attachment cap.
   - Implemented `dispatch_rendered_tables` which divides `rendered_tables` into chunks of at most 10 attachments (`chunks(DISCORD_ATTACHMENT_LIMIT)`).
   - Extended `DiscordUploadTransport` trait with `send_attachments` and wired `DiscordEgress::with_upload_transport`.
2. **Forum Channel Attachment Preservation**:
   - In `DiscordEgress::dispatch(OutboundAction::SendMessage)`:
     - When target is a forum channel, after creating the forum thread and dispatching any chunk continuations, calls `self.dispatch_rendered_tables(&http, post_channel.id, rendered_tables).await?`.
     - Preserves all rendered table PNGs in the newly created forum thread rather than dropping them.
3. **Error Propagation**:
   - Replaced `if let Err(error) = ... { tracing::error!(...); }` with `?` error propagation in both `DiscordEgress::stream` and `DiscordEgress::dispatch(SendMessage)`.
   - Any attachment upload failure fails the turn cleanly rather than falsely reporting delivery success.
4. **Regression Verification (`tests/test_discord_adapter.rs::tables_survive_forum_and_attachment_limits`)**:
   - Tested 11 distinct tables in final Stream:
     - RED captured: 11 attachments sent in single batch exceeding limit 10, rejected by mock, swallowed by old code leaving 0 batches uploaded (exit 101).
     - GREEN: 11 tables split into 2 batches (10 + 1), total 11 uploaded, each batch <= 10.
     - Forced upload failure propagates `Err` rather than swallowing.

## Verification
- Captured RED: `U35-red.log`, `U35-red.exit` (exit 101, panic: got 0 batches)
- Captured GREEN: `U35-green.log`, `U35-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 51 passed in 1.65s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
