# U29 Evidence: Bot-Scoped Recoverable Dead Targets

## Metadata
- Unit: U29 (Lane 1: Discord Ingress/Egress & Error Classification)
- Findings: D.D06 (403/404 whole-channel poisoning, missing message 10008, un-scoped registry), CR.C12 (dead targets persistence, non-delivered skip receipt, recovery probe)
- Citations:
  - Live: `src/discord/adapter.rs`, `src/storage/db.rs`, `migrations/0019_dead_targets.sql`
  - Hermes / Upstream parity: `plugins/platforms/discord/adapter.py:2900-2940`, `gateway/dead_targets.py:1-20,58-88,97-132`, `gateway/delivery.py:127-141`
- Date: 2026-09-07

## Implementation Summary
1. **Database Persistence Migration (`migrations/0019_dead_targets.sql`)**:
   - Created table `dead_targets (bot_id TEXT NOT NULL, channel_id INTEGER NOT NULL, status_code INTEGER NOT NULL, error_message TEXT NOT NULL, dead_since TEXT NOT NULL, probed_at TEXT, PRIMARY KEY (bot_id, channel_id))`.
   - Added persistence helper functions in `src/storage/db.rs`: `persist_dead_target`, `remove_dead_target`, `remove_dead_targets_for_channel`, `load_dead_targets`.
2. **Bot-Scoped Recoverable Registry (`DeadTargetRegistry` in `src/discord/adapter.rs`)**:
   - Keyed by `(bot_id: String, channel_id: u64)`, ensuring permission issues or kicks on Bot A do not disable delivery for Bot B.
   - Added `with_probe_interval` to support self-healing probes: after the probe interval has elapsed since the last failure/probe, one trial send is permitted to probe whether permissions were restored.
   - Added `load_from_db` to restore known dead targets across gateway restarts or egress re-creations.
   - Implemented `mark_dead_and_persist` and `clear_and_persist` for immediate durable synchronization without race conditions.
3. **Discord JSON Error Code Classification (`is_discord_dead_target_error`)**:
   - Code 10008 ("Unknown Message") is a message-specific error (deleted referenced message), NOT channel death: returns `None`.
   - In `SendMessage`, failing with 10008 now cleanly proceeds to the reference-less retry fallback without poisoning the channel.
   - Code 10003 ("Unknown Channel") or generic 404 is classified as dead channel: returns `Some((404, message))`.
   - Code 403 (e.g. 50001 Missing Access, 50013 Missing Permissions) is classified as dead channel for the sending bot: returns `Some((403, message))`.
4. **Structured Non-Delivered Result on Short-Circuit**:
   - When a target channel is dead for a given bot, `dispatch` returns `Err(OmonError::Multiplexer("dead target short-circuited..."))` instead of claiming success.
   - Prevents cron schedulers and delivery obligations from erroneously considering dropped messages as delivered.
   - Applied dead target short-circuiting to `stream`, `SendMessage`, `EditMessage`, `DeleteMessage`, `React`, and `UploadFile`.
5. **Regression Tests**:
   - `tests/test_discord_adapter.rs::dead_target_is_identity_and_resource_scoped`:
     - Verifies bot isolation (Bot A dead on channel 7, Bot B active on channel 7).
     - Verifies probe interval allows one trial send and locks again until trial outcome.
     - Verifies short-circuit returns `Err` without claiming delivery.
   - `tests/test_cron_egress_parity.rs::dead_targets_are_scoped_and_recoverable`:
     - Verifies bot scoping under multiple bot clients.
     - Verifies unknown channel 10003 persists in SQLite across egress recreation and subsequent send fails with `Err`.
     - Verifies recovery on successful send/clear.

## Verification
- Captured RED: `U29-red.log`, `U29-red.exit` (exit 101)
- Captured GREEN: `U29-green.log`, `U29-green.exit` (exit 0)
- Full `test_discord_adapter` suite: 47 passed in 1.04s
- Full `test_cron_egress_parity` suite: 1 passed in 0.48s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
