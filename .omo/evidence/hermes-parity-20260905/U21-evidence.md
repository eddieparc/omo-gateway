# Hermes Parity Unit U21 Evidence: Parent-Aware Profile Routing

## Defect Summary
- **S.F10**:
  - In `src/discord/adapter.rs`, Discord ingress received thread messages with both `message.channel_id` (the thread ID) and `config.parent_channel_id` (the parent channel ID). However, `message_to_inbound_with_config` stored the thread ID into both `session.channel_id` and `session.thread_id`.
  - Similarly, in auto-thread creation (`src/discord/adapter.rs:968`), `event.session.channel_id` was overwritten with `thread_channel.id.to_string()`, erasing the parent channel context (`new_message.channel_id`).
  - In `src/multiplexer/profile_routing.rs:121-139, 239-246`, the profile router matched routes against `session.channel_id` literally without checking the thread's parent channel. Consequently, profile routes or per-channel prompts configured on a parent channel (e.g. channel 200) never applied inside its threads (e.g. thread 300).
  - Hermes upstream `gateway/profile_routing.py:76-104` explicitly matches `parent_chat_id` when direct thread routes miss. Pre-existing tests in `profile_routing.rs:344-398` and `tests/test_profile_routing.rs:125` supplied parent as `channel_id` artificially in test fixtures, concealing the ingress defect.

## Fix Architecture

1. **Parent Chat/Channel Accessors on Inbound Events (`src/models/events.rs`)**:
   - Added `InboundEvent::parent_chat_id(&self) -> Option<&str>`: returns the parent channel ID when the event belongs to a thread (`self.session.thread_id.is_some()`).
   - Added `InboundEvent::parent_channel_id(&self) -> Option<&str>` alias.
   - Added `InboundEvent::with_parent_chat_id(mut self, parent_chat_id: impl Into<String>) -> Self`.

2. **Parent-Aware Ingress & Auto-Thread Preservation (`src/discord/adapter.rs`)**:
   - In `message_to_inbound_with_config`:
     - When `is_thread` is true and `config.parent_channel_id` is present, `session.channel_id` is set to the parent channel ID (`parent_id.to_string()`), while `session.thread_id` is set to `Some(message.channel_id.to_string())`.
     - When `is_thread` is true without a known parent, falls back to `message.channel_id.to_string()`.
   - In `handle_event` auto-thread handling:
     - Preserves the parent channel ID (`new_message.channel_id.to_string()`) in `event.session.channel_id`.
     - Sets `event.session.thread_id` to the newly created thread (`Some(thread_channel.id.to_string())`).
   - Egress invariant preserved: `DiscordEgress::target` uses `session.thread_id.as_ref().unwrap_or(&session.channel_id)`, ensuring replies target thread 300 without sending to parent 200.

3. **Parent-Aware Profile Route Matching & Precedence (`src/multiplexer/profile_routing.rs`)**:
   - Added `ProfileRouter::match_route_with_parent(guild_id, channel_id, thread_id, parent_chat_id)`:
     1. **Thread-Specific Match**: Checks routes targeting `thread == Some(tid)` or direct route with `channel == Some(tid)` (thread treated as channel).
     2. **Direct Channel Match**: Checks routes configured for the direct channel.
     3. **Parent Channel Match**: When direct channel/thread routes miss, falls back to matching `channel == Some(parent_id)`.
     4. **Guild Fallback**: Matches guild-level route if configured.
   - Added `ProfileRouter::match_event(&self, event: &InboundEvent)`: resolves matching route from inbound event metadata including parent channel context.
   - Added unit test `test_parent_channel_fallback_when_thread_route_misses` verifying thread route > parent channel route > guild route precedence.

4. **Integration Regression Test (`tests/test_multiplexer.rs`)**:
   - Implemented `thread_inherits_parent_profile`: converts real Discord message in thread 300 with parent 200 and guild 100 via `message_to_inbound_with_config`, routes through `SessionMultiplexer::with_profile_router` configured with a route on parent channel 200 selecting model `"model-x"`.
   - Asserts model `"model-x"` is inherited while egress destination remains thread 300.

## RED Phase Verification

Executed before source implementation; failed non-zero (`exit: 101`):

- **Command**: `cargo test --test test_multiplexer thread_inherits_parent_profile -- --exact --nocapture`
- **Exit Code**: `101`
- **Failed Assertion**: `tests/test_multiplexer.rs:1177:5`
  ```
  thread 'thread_inherits_parent_profile' panicked at tests/test_multiplexer.rs:1177:5:
  assertion `left == right` failed: thread 300 must inherit profile route model from parent channel 200
    left: None
   right: Some("model-x")
  ```
- Recorded in `.omo/evidence/hermes-parity-20260905/U21-red.exit` and `.omo/evidence/hermes-parity-20260905/U21-red.log`.

## GREEN Phase Verification

Executed after source implementation; passed with exit code 0:

- **Command**: `cargo test --test test_multiplexer thread_inherits_parent_profile -- --exact --nocapture`
- **Exit Code**: `0`
- **Output**:
  ```
  running 1 test
  test thread_inherits_parent_profile ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out; finished in 0.09s
  ```
- Recorded in `.omo/evidence/hermes-parity-20260905/U21-green.exit` and `.omo/evidence/hermes-parity-20260905/U21-green.log`.

## Adjacent Suite Results

All required adjacent suites pass cleanly:

1. `cargo test --test test_multiplexer`: **14 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U21-mux.log`)
2. `cargo test --test test_discord_adapter`: **41 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U21-suite.log`)
3. `cargo build`: **Finished `dev` profile (exit 0)** (`.omo/evidence/hermes-parity-20260905/U21-build.log`)
4. `cargo test --bin omo-gateway`: **35 passed; 0 failed** (`.omo/evidence/hermes-parity-20260905/U21-main.log`)
5. `cargo fmt --all -- --check`: **Clean formatting (exit 0)** (`.omo/evidence/hermes-parity-20260905/U21-format.log`)
6. `git diff --check` on owned files: **No whitespace/diff issues (exit 0)** (`.omo/evidence/hermes-parity-20260905/U21-diff.log`)
