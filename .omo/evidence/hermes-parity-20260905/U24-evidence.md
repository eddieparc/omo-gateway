# U24 — Hydrate referenced attachments

## Findings
- **D.D04**: When replying to a parent message with attachments, referenced attachments were converted to filenames only in the quote prose, while `event.attachments` only included direct attachments and forwarded snapshots. The downloader and vision/prompt pipelines never received the referenced parent attachments.

## Regressions
- `tests/test_discord_adapter.rs::reply_parent_attachment_reaches_prompt`
  - **RED**: `event.attachments` was empty (`[]`), failing assertion that parent attachment id 100 (`chart.png`) is present. Captured in `U24-red.log` (exit 101).
  - **GREEN**: `event.attachments` includes parent attachment id 100 (`chart.png`). Captured in `U24-green.log` (exit 0).

## Changes
- `src/discord/adapter.rs`: In `message_to_inbound_with_config`, union `parent.attachments` into `attachments` by unique attachment ID before the empty-message check. This ensures image and document replies reach the attachment downloader and prompt renderer.

## Verification
- `cargo test --test test_discord_adapter reply_parent_attachment_reaches_prompt -- --exact` -> exit 0
- `cargo test --test test_discord_adapter` -> 44 passed, 0 failed
- `cargo fmt --check` -> clean (exit 0)
- `cargo clippy --lib --tests -- -D warnings` -> clean (exit 0)
