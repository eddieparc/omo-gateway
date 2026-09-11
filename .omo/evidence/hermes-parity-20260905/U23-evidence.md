# Hermes Parity Unit U23 Evidence: Recover Source-Time Historical Input

## Defect Summary
- **CR.C05 / D.D15 (Missed-message startup backfill & cursor durability)**:
  - Previously, startup recovery in `src/discord/adapter.rs` used shared channel-only cursors (`discord_channel_cursors`). A secondary bot starting up could inherit a cursor advanced by a primary bot, skipping historical messages intended for the secondary bot.
  - The cursor was advanced before durable admission and delivery claim; a crash during debounce or processing lost recoverability permanently.
  - Missed-message backfill only fetched a single 50-message page, passed empty `user_roles: &[]` (skipping role-only authorized historical messages), and treated channel lookup failures as `ChannelType::Private` (allowing private DM bypass on REST metadata failures).
  - Failed routing attempts did not halt channel scanning and advanced the cursor past unhandled messages, preventing retry.
  - When a replayed turn failed, the uncompleted inbound event remained in `messages`, preventing subsequent retries due to transcript dedup index collisions (`has_platform_message_id`), and failed claims could not be re-claimed in `delivery_ledger`.
- **S.F09 / S.F17 (Source platform timestamp preservation)**:
  - In `message_to_inbound_with_config`, `InboundEvent::message` initialized `received_at` with `Utc::now()`, discarding the Discord message's actual platform timestamp (`message.timestamp`).
  - Historical messages recovered on startup or backfill appeared to have been sent at the gateway restore time instead of their original source date, corrupting prompt formatting and recovery session context.

## Fix Architecture

1. **Per-Bot Durable Monotonic Cursors (`migrations/0018_bot_cursors.sql`, `src/discord/adapter.rs`)**:
   - Added `discord_bot_cursors (bot_id TEXT, channel_id TEXT, last_message_id TEXT, updated_at TEXT, PRIMARY KEY (bot_id, channel_id))`.
   - Added `get_bot_channel_cursor` and `update_bot_channel_cursor` in `src/discord/adapter.rs`. Monotonicity is verified before insertion/update. Legacy `discord_channel_cursors` is maintained for backwards compatibility when `bot_id` is empty.
   - Scans discover all configured targets: `allowed_channels`, `free_response_channels`, active threads, and stored bot cursors, filtering out `ignored_channels`.

2. **Durable Delivery Gating & Failed-Turn Reclaim (`src/ledger/service.rs`, `src/discord/adapter.rs`)**:
   - In `src/ledger/service.rs`: `record_incoming_as` re-claims previously failed deliveries:
     ```sql
     ON CONFLICT(message_id) DO UPDATE SET
        status = 'in_progress',
        updated_at = excluded.updated_at,
        error = NULL
     WHERE delivery_ledger.status = 'failed'
     ```
   - In `src/discord/adapter.rs`:
     - Added `route_claimed_event_awaiting_turn`: routes the event through the multiplexer and awaits the turn outcome. If the turn fails, marks delivery `failed` in the ledger and cleans up the uncompleted message row from SQLite `messages` so subsequent retries are admitted cleanly.
     - `run_missed_message_backfill_with_fetcher`: paginates up to 50 messages per page in a loop.
     - On REST channel lookup failure, logs a warning and fails closed without private/DM bypass.
     - Hydrates member roles via `fetcher.get_member_roles(guild_id, author_id)` so role-authorized messages are properly admitted.
     - Advances the per-bot cursor in SQLite only upon durable delivery completion (`Ok(routed)` where `routed == true`). On route failure, backfill halts immediately for that channel and holds the cursor for future retry.

3. **Session Lane Idle Wait & Turn Outcome Notification (`src/multiplexer/router.rs`)**:
   - Added `SessionMultiplexer::wait_for_session_idle(&self, key: &SessionKey)`: waits for in-flight turn drain via notification (`Notify`), avoiding polling or fixed sleeps.
   - Added `SessionMultiplexer::route_awaiting_turn(&self, event: InboundEvent)`: sends `ActorCommand::EventWithAck` and awaits terminal outcome via oneshot channel.

4. **Source Timestamp Ingress Propagation (`src/discord/adapter.rs`, `src/storage/db.rs`)**:
   - In `message_to_inbound_with_config`, initialized `received_at` by parsing `message.timestamp` (RFC3339) into UTC `DateTime<Utc>`, falling back to `Utc::now()` only if timestamp parsing fails.
   - Updated `inbound_preserves_original_platform_timestamp` test in `src/storage/db.rs` to verify that `received_at` matches the original 2020-01-01 platform timestamp, rendered user prompts contain the source timestamp date, and startup recovery created_at preserves the original timestamp.

5. **Comprehensive Multi-Scenario Regression Test (`tests/test_discord_adapter.rs`)**:
   - `startup_recovery_preserves_work_and_source_time` covers 5 critical scenarios:
     1. **Aug14 role-authorized msg8**: Crash before debounce flush; restores on Sep5; verifies msg8 is backfilled, preserves Aug14 timestamp, dispatches exactly once, and bot 42 cursor advances to 8.
     2. **Channel REST metadata failure (msg9)**: Simulates 500 error on metadata lookup; verifies fail-closed behavior (no dispatch, no DM bypass, cursor held at 8).
     3. **Multi-bot channel scanning**: Verifies Bot 84 scans independently from its own start, processes msg9 for Bot 84, advances its cursor to 9, while Bot 42's cursor remains unaffected at 8.
     4. **Failed-route retry (msg10)**: Simulates route failure for msg10; verifies cursor is held at 8; heals route and re-runs backfill; verifies msg10 is re-claimed and dispatched, and cursor advances to 10.
     5. **Pagination coverage**: Generates 75 messages (IDs 101..=175) across multiple 50-message pages on channel 200; verifies all 75 messages reach the runner and cursor advances to 175.

## RED Phase Verification

Original failures captured in `.omo/evidence/hermes-parity-20260905/U23-red.log` (exit code 101):

```
=== cargo test --test test_discord_adapter startup_recovery_preserves_work_and_source_time -- --exact --nocapture ===
thread 'startup_recovery_preserves_work_and_source_time' panicked at tests/test_discord_adapter.rs:2569:5:
assertion `left == right` failed: Aug14 role-authorized msg8 must be backfilled on restore
  left: 0
 right: 1
test startup_recovery_preserves_work_and_source_time ... FAILED
exit: 101

=== cargo test --lib storage::db::tests::inbound_preserves_original_platform_timestamp -- --exact --nocapture ===
thread 'storage::db::tests::inbound_preserves_original_platform_timestamp' panicked at src/storage/db.rs:1829:9:
assertion `left == right` failed: InboundEvent::received_at must preserve source platform timestamp (2020-01-01T00:00:00Z), not ingestion time
  left: 2026-09-07T12:35:24.205729Z
 right: 2020-01-01T00:00:00Z
test storage::db::tests::inbound_preserves_original_platform_timestamp ... FAILED
exit: 101
```

## GREEN Phase Verification

Executed after source implementation and test fixes; passed with exit code 0 (recorded in `.omo/evidence/hermes-parity-20260905/U23-green.log` and `.omo/evidence/hermes-parity-20260905/U23-green.exit`):

- **Command 1**: `cargo test --test test_discord_adapter startup_recovery_preserves_work_and_source_time -- --exact --nocapture`
  ```
  running 1 test
  test startup_recovery_preserves_work_and_source_time ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 43 filtered out; finished in 0.75s
  ```

- **Command 2**: `cargo test --lib storage::db::tests::inbound_preserves_original_platform_timestamp -- --exact --nocapture`
  ```
  running 1 test
  test storage::db::tests::inbound_preserves_original_platform_timestamp ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 361 filtered out; finished in 0.02s
  ```

- **Command 3**: `cargo test --lib inbound_preserves_original_platform_timestamp -- --nocapture`
  ```
  running 1 test
  test storage::db::tests::inbound_preserves_original_platform_timestamp ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 361 filtered out; finished in 0.09s
  ```

## Adjacent Suite Results

- **`cargo test --test test_discord_adapter`**: 44 passed, 0 failed (`U23-suite.log`)
- **`cargo test --test test_multiplexer`**: 15 passed, 0 failed (`U23-adjacent-mux.log`)
- **`cargo test --lib storage::db::tests`**: 25 passed, 0 failed (`U23-adjacent-db.log`)
- **`cargo build`**: exit code 0 (`U23-build.log`, `U23-build.exit`)

## Formatting and Diff Verification

- **Format Check (`rustfmt --check --edition 2021 src/discord/adapter.rs src/ledger/service.rs src/multiplexer/router.rs tests/test_discord_adapter.rs`)**: exit code 0 (`U23-format.log`, `U23-format.exit`)
- **Diff Check (`git diff --check src/discord/adapter.rs src/ledger/service.rs src/multiplexer/router.rs tests/test_discord_adapter.rs`)**: exit code 0, zero whitespace errors or trailing diff issues (`U23-diff.log`, `U23-diff.exit`)
