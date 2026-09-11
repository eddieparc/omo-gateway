# Hermes Parity Unit U20 Evidence: Permanent Per-Bot Conversation Lanes

## Defect Summary
- **D.D14**:
  - In `src/discord/adapter.rs`, auto-thread creation fired indiscriminately even for `InlineReply` or channels already free of thread forcing (`free_response_channels`). On failure to create a thread, it silently fell back to an in-channel reply in the public/parent channel rather than failing visibly and halting parent agent invocation.
  - `active_threads` in `PoiseData` was an in-memory set that vanished across restarts. Additionally, unmentioned guild traffic was unconditionally routed only to the primary bot (`config.primary_bot_id`), meaning a secondary bot explicitly engaged in a thread could not receive unmentioned continuations and the primary bot would take over.
  - Text-channel and thread sessions included author `user_id` in `SessionKey` and `SessionKey::storage_key()` (`src/models/session.rs:59-64`), violating the per-bot channel identity invariant by partitioning conversation lanes per user rather than maintaining one canonical permanent lane per (bot, channel).
- **S.F12**:
  - Session key for guild channel/thread traffic must be a canonical guild channel/thread key independent of the sender (sender identity preserved separately in message transcript/context); slash command routing must follow the exact same rule.

## Fix Architecture

1. **Sender-Independent Guild Lane Identity (`src/models/session.rs`)**:
   - Updated `SessionKey::new` so that whenever `guild_id.is_some()`, `user_id` is normalized to `String::new()`, guaranteeing that all messages and slash commands in a guild channel or thread produce the identical canonical `SessionKey` regardless of author.
   - Updated `SessionKey::storage_key()` so that when `guild_id.is_some()`, the user component is serialized as `Some("")` (`0:`), completely excluding author identifiers from the canonical storage key while remaining fully roundtrip-compatible with downstream parsers.
   - Updated `SessionKey::from_storage_key` to reconstruct guild session keys with canonical sender-independent `user_id` (`String::new()`).

2. **Durable Thread Ownership (`src/storage/db.rs`, `src/discord/commands.rs`)**:
   - Added SQLite table `discord_thread_owners (thread_id TEXT PRIMARY KEY, bot_id TEXT NOT NULL, updated_at TEXT NOT NULL)` created during database initialization.
   - Added storage helper methods:
     - `Database::record_thread_owner(pool, thread_id, bot_id)`
     - `Database::get_thread_owner(pool, thread_id)`
     - `Database::load_all_thread_owners(pool)`
   - Added in-memory caching and durable query support to `PoiseData`:
     - `thread_owners: Arc<RwLock<HashMap<u64, u64>>>`
     - `PoiseData::mark_thread_owner` (persists asynchronously to SQLite and updates in-memory map)
     - `PoiseData::get_thread_owner_durable` (loads from SQLite on cache miss)
     - `PoiseData::get_thread_owner_cached`

3. **Bot-Specific Thread Routing & Auto-Thread Safety (`src/discord/adapter.rs`)**:
   - Added `thread_owners: &'a [(u64, u64)]` to `InboundFilterConfig`.
   - Updated `message_to_inbound_with_config`:
     - For unmentioned messages in threads, checks `thread_owner`. If the thread is engaged by a specific bot, only that bot (`owner == bot_user_id.get()`) responds; other bots (including primary bot) return `None` and do not take over.
     - Normalized `user_id` for guild inbound events to `String::new()` (preserving author identity in transcript and channel context).
   - Added `should_auto_create_thread(auto_thread_enabled, is_guild_text, is_explicit_mention, is_free_channel, is_reply)`. Auto-thread creation is suppressed for inline replies and free-response channels.
   - Forced thread creation failure handling: when thread creation fails, logs a warning, emits visible failure emoji reaction `❌`, sends a visible channel error message, and aborts before queueing or routing to the agent (no agent invocation in the parent).
   - Records thread ownership upon auto-thread creation, explicit thread mention, or `/thread` creation.

4. **Slash Command Routing (`src/discord/commands.rs`)**:
   - Updated `session_key` helper to set `user_id` to `String::new()` for all guild channels/threads, ensuring slash commands operate on the canonical conversation lane.
   - Updated `/thread` slash command to record bot ownership of newly created threads.

## RED Phase Verification

Both regression tests were executed before source implementation and failed non-zero (`exit: 101`):

- **Command**: `cargo test --test test_discord_adapter dedicated_thread_owner_and_channel_identity -- --exact --nocapture`
  - **Exit Code**: `101`
  - **Failed Assertion**: `tests/test_discord_adapter.rs:2012:5`
    ```
    assertion `left == right` failed: Guild text channel lane must be shared per bot, NOT per author
      left: "7:discord|1:9|1:7|-|3:100|2:84"
     right: "7:discord|1:9|1:7|-|3:200|2:84"
    ```
- **Command**: `cargo test --test test_multiplexer guild_lane_is_shared_per_bot_not_author -- --exact --nocapture`
  - **Exit Code**: `101`
  - **Failed Assertion**: `tests/test_multiplexer.rs:885:5`
    ```
    assertion `left == right` failed: Alice and Bob in the same guild lane must produce the exact same storage key
      left: "7:discord|1:9|1:8|1:8|3:100|2:84"
     right: "7:discord|1:9|1:8|1:8|3:200|2:84"
    ```
- Recorded in `.omo/evidence/hermes-parity-20260905/U20-red.exit` and `.omo/evidence/hermes-parity-20260905/U20-red.log`.

## GREEN Phase Verification

Both regression tests pass cleanly with exit code 0:

- **Command**: `cargo test --test test_discord_adapter dedicated_thread_owner_and_channel_identity -- --exact --nocapture`
  - **Exit Code**: `0`
  - **Output**: `test dedicated_thread_owner_and_channel_identity ... ok` (1 passed, 0 failed)
- **Command**: `cargo test --test test_multiplexer guild_lane_is_shared_per_bot_not_author -- --exact --nocapture`
  - **Exit Code**: `0`
  - **Output**: `test guild_lane_is_shared_per_bot_not_author ... ok` (1 passed, 0 failed)
- Recorded in `.omo/evidence/hermes-parity-20260905/U20-green.exit` and `.omo/evidence/hermes-parity-20260905/U20-green.log`.

## Adjacent Suite Results

All adjacent test suites and build targets pass cleanly:

1. `cargo test --test test_discord_adapter`: **41 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U20-suite.log`)
2. `cargo test --test test_multiplexer`: **13 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U20-mux.log`)
3. `cargo test --lib storage::db`: **23 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U20-db.log`)
4. `cargo build`: **Finished `dev` profile (exit 0)** (`.omo/evidence/hermes-parity-20260905/U20-build.log`)
5. `cargo test --bin omo-gateway`: **35 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U20-main.log`)
6. `cargo fmt --all -- --check`: **Clean formatting (exit 0)** (`.omo/evidence/hermes-parity-20260905/U20-format.log`)
7. `git diff --check`: **No whitespace errors or diff issues on owned files (exit 0)** (`.omo/evidence/hermes-parity-20260905/U20-diff.log`)
