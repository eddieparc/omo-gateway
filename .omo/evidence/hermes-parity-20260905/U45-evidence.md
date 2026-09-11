# U45 Evidence: Exact Reminder and Mirror Identity Parity

## Metadata
- Unit: U45 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C14, R.R12 (Created reminders previously dropped bot/thread delivery coordinates; all newly built delivery/recovery keys had bot_id None; explicit source session was reused for every fanout mirror regardless of destination, causing fanout messages to mirror into source session twice; fallback broadened explicit thread targets to arbitrary channel sessions; ambiguous bot candidates picked newest row)
- Citations:
  - Live: `src/mirror.rs`, `src/tools/cron.rs`, `src/cron/scheduler.rs`, `src/cron/store.rs`, `tests/test_cron_egress_parity.rs`
  - Hermes / Upstream parity: `cron/scheduler.py:656-757`, `gateway/delivery.py:161-205`
- Date: 2026-09-07

## Implementation Summary
1. **Reminder Delivery Coordinates Preservation (`src/tools/cron.rs`)**:
   - In `CronTool::execute_with_context`: when adding a reminder from a Discord session, format `deliver` as `discord:<channel_id>:<thread_id>` whenever `thread_id` is present, rather than truncating to channel only.
   - Preserves source `bot_id` in `args["bot_id"]` and saves it in the cron job payload.
2. **Strict Thread Matching and Ambiguity Refusal (`src/mirror.rs`)**:
   - Updated `find_session_by_origin` to accept `bot_id: Option<&str>`.
   - Explicit thread targets: queries strictly matching the specified `thread_id`. If no matching row exists, returns `Ok(None)` without broadening to the channel.
   - Ambiguous bot resolution: if `bot_id` is not specified and multiple candidate sessions exist with different bots (e.g. Bot A and Bot B), returns `Ok(None)` instead of guessing the newest row.
3. **Destination-Specific Mirror Routing (`src/cron/scheduler.rs`)**:
   - In `mirror_cron_delivery_to_session`: checks whether `job_session_key` matches the destination channel, thread, and bot.
   - If the destination is a fanout target (different channel/thread), skips reusing the source session and queries a dedicated session for that destination.
   - Prevents second fanout destinations from duplicating messages into the source session.
4. **Regression Verification**:
   - `tests/test_cron_egress_parity.rs::reminder_retains_bot_and_thread`:
     - Reminder created from Bot B channel 42, thread 43 correctly retains `deliver: "discord:42:43"` and `bot_id: "bot-b"`.
     - Mirrored destination 1 to source session; fanout destination 2 (channel 99) rejected from mirroring into source session (exactly 1 message recorded in source).
   - `src/mirror.rs::tests::mirror_refuses_wrong_thread_and_ambiguous_bot`:
     - Missing explicit thread `c1/t3` returns `None`.
     - Ambiguous bot candidates on `c1` returns `None`.
     - Explicit bot `bot-a` on `c1/t1` matches exact session.

## Verification
- Captured RED: `U45-red.log`, `U45-red.exit` (exit 101, panic: reminder_retains_bot_and_thread, missing 'id' / ambiguous bot resolution)
- Captured GREEN: `U45-green.log`, `U45-green.exit` (exit 0)
- Full `test_cron_egress_parity` suite: 2 passed in 0.47s
- Unit test `mirror_refuses_wrong_thread_and_ambiguous_bot`: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
