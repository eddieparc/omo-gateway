# U09: Fail-closed Shared Admission

## Contract & Scope
- Finding: `D.D02`
- Scoped files:
  - `src/discord/commands.rs`
  - `src/discord/adapter.rs`
  - `tests/test_discord_adapter.rs`
  - Evidence artifacts: `.omo/evidence/hermes-parity-20260905/U09-*`
- Invariant: User authorization is fail-closed by default (empty allowlists with `allow_all_users=false` denies access).
- Invariant: Slash command admission (`command_check`) and message ingress (`message_to_inbound_with_config`) share the identical parent-aware channel gate (`is_channel_authorized`).
- Invariant: Explicit `allow_all_users=true` only bypasses user authorization policy, never the channel allow/ignore policy.
- Invariant: Missing or failed required guild metadata fails closed and does not silently authorize.
- Invariant: Authorized roles/users/paired semantics preserved; paired users are not promoted to operators for `/pair`.

## Regression Test Registered Before Production Edits
Exact test: `cargo test --test test_discord_adapter authorization_surface_matrix -- --exact --nocapture`
Location: `tests/test_discord_adapter.rs::authorization_surface_matrix`

The test defines the unified admission matrix across both command and message surfaces:
1. `user 10, lists empty, allow_all=false`: rejected fail-closed (user auth & slash admission).
2. `explicit allow_all=true`: admitted for user policy.
3. `user/role positive and negative`: matching ID/role admitted, non-matching rejected.
4. `ignored parent 7 / child thread 8`: both slash steer and message ingress rejected before runner.
5. `allowed parent 7`: admits non-ignored child thread 8 on both slash controls and message ingress.
6. `child ignore override`: child thread 8 in `ignored_channels` overrides parent 7 in `allowed_channels`.
7. `explicit allow_all only bypasses user policy`: `allow_all_users=true` in ignored parent 7 / thread 8 is rejected.
8. `missing/failed guild metadata`: missing `channel_type` in guild message or failed guild channel lookup fails closed.

## Historical Preproduction RED Chronology
Before production edits, `authorization_surface_matrix` was executed against the baseline code:
- Command: `cargo test --test test_discord_adapter authorization_surface_matrix -- --exact --nocapture`
- Result: Exit code 101, exactly 1 test run, 1 failure.
- **Recorded preproduction failure**: The test panicked and aborted at the first assertion (`assert!(!is_user_authorized(10, &[], &[], &[], false))`), logging `user 10 with empty lists and allow_all=false must fail-closed`.
- **Source-reviewed baseline defects (not reached prior to test abort)**: Because the test panicked on assertion 1, subsequent assertions were not reached during the recorded RED run. Source-code audit independently established the remaining baseline defects:
  - `commands.rs`: `is_user_authorized` contained `if allowed_users.is_empty() && allowed_roles.is_empty() { return true; }`.
  - `commands.rs`: `command_check` checked only user/role authorization and never validated channel allow/ignore lists.
  - `adapter.rs`: `message_to_inbound_with_config` only checked `config.ignored_channels.contains(&channel_id_u64)`, completely ignoring `parent_channel_id`.
  - `adapter.rs`: missing guild channel metadata fell back to `(None, None)` without aborting message ingress.
  All additional cases were confirmed and exercised as GREEN controls upon applying the production fix.
- Output captured in `U09-red.log`:
```
running 1 test
test authorization_surface_matrix ... FAILED

failures:
    authorization_surface_matrix

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 39 filtered out; finished in 0.00s

thread 'authorization_surface_matrix' panicked at tests/test_discord_adapter.rs:1710:5:
user 10 with empty lists and allow_all=false must fail-closed
```

## Interruption and Regression Repair (NOT PASS State Documented)
During the split-turn session compaction and the subsequent API refactor of `check_slash_admission`, `tests/test_discord_adapter.rs` was temporarily reverted to git HEAD:
- **Observed Intermediate Failure**:
  - `cargo test --test test_discord_adapter authorization_surface_matrix -- --exact --nocapture` selected 0 tests (39 filtered).
  - `cargo test --test test_discord_adapter` ran 39 tests: 25 passed, 14 failed (because legacy tests were pinned to fail-open defaults while production code was fail-closed).
  - **Verdict on intermediate state**: **NOT PASS**.
- **Repair**:
  - Re-applied the exact named regression `authorization_surface_matrix` with all 8 original assertions adapted to the clean 4-argument `(user_id, is_paired, CommandChannelScope, &InboundFilterConfig)` signature.
  - Re-applied intentional access configurations (`allowed_users: &[10]`, `allowed_roles`, or `allow_all_users: true`) across the 14 legacy adapter tests without deleting assertions.
  - Re-verified both exact selection (1 test, 1 passed) and full integration suite (40 tests, 40 passed).

## Production Implementation & Lint Conformance
1. `src/discord/commands.rs`:
   - `is_user_authorized`: removed fail-open branch `if allowed_users.is_empty() && allowed_roles.is_empty() { return true; }`. Returns `false` unless explicitly authorized by user ID, role, or `allow_all_users=true`.
   - `is_channel_authorized`: evaluates channel and parent channel against `ignored_channels` first (blacklist), then evaluates guild whitelist (`allowed_channels`) allowing child thread if either channel or parent is allowed. DMs exempt from whitelist.
   - `CommandChannelScope`: domain struct encapsulating `channel_id: u64`, `parent_channel_id: Option<u64>`, `is_dm: bool`, and `guild_metadata_available: bool`, with constructor helpers `guild(...)`, `dm(...)`, and `missing_guild_metadata(...)`.
   - `check_slash_admission`: shared evaluator boundary with signature:
     ```rust
     pub fn check_slash_admission(
         user_id: u64,
         is_paired: bool,
         channel: CommandChannelScope,
         config: &super::adapter::InboundFilterConfig<'_>,
     ) -> CommandAdmissionResult
     ```
     This reduces the parameter count to 4 (well within `clippy::too_many_arguments` limit of 7) by leveraging the domain `CommandChannelScope` and the existing `InboundFilterConfig`, with **zero** `#[allow(...)]` suppressions and no throwaway tuple packing.
   - `command_check`: invokes `check_slash_admission`, handling metadata failures, unauthorized users, and unauthorized channels with dedicated ephemeral replies.
2. `src/discord/adapter.rs`:
   - Re-exports `pub use super::commands::is_channel_authorized;`.
   - `message_to_inbound_with_config`: enforces `guild_id.is_some() && channel_type.is_none()` fail-closed check and delegates channel gating to `is_channel_authorized`.
   - `handle_event`: drops guild message fail-closed if guild channel metadata lookup fails.
3. `tests/test_discord_adapter.rs`:
   - Restored `authorization_surface_matrix` exercising all 8 facets with the 4-arg API.
   - Reconciled 14 legacy tests that pinned fail-open defaults by configuring intentional access, preserving positive/negative assertions without deleting tests.

## GREEN Verification
1. Exact command:
   `cargo test --test test_discord_adapter authorization_surface_matrix -- --exact --nocapture`
   Output:
   ```
   running 1 test
   test authorization_surface_matrix ... ok

   test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 39 filtered out; finished in 0.00s
   ```
   Exit code: 0
   Captured in: `U09-green.log`

2. Adapter integration suite:
   `cargo test --test test_discord_adapter`
   Output:
   ```
   running 40 tests
   test final_live_edit_deletes_surplus_chunk_messages ... ok
   ...
   test authorization_surface_matrix ... ok
   ...
   test result: ok. 40 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
   ```
   Exit code: 0
   Captured in: `U09-adjacent.log`

3. Commands unit tests:
   `cargo test --lib discord::commands`
   Output: `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 324 filtered out; finished in 0.35s`
   Exit code: 0 (preserves U08 pending review/owner download tests).

4. Adapter unit tests:
   `cargo test --lib discord::adapter`
   Output: `test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 293 filtered out; finished in 0.51s`
   Exit code: 0 (preserves U03 dead target tests).

5. Lint & Style Checks:
   - `rustfmt --check --edition 2021 src/discord/commands.rs src/discord/adapter.rs tests/test_discord_adapter.rs`: Exit 0.
   - `git diff --check src/discord/commands.rs src/discord/adapter.rs tests/test_discord_adapter.rs`: Exit 0 (zero whitespace errors).
   - Zero `clippy::too_many_arguments` or other warnings attributable to `commands.rs`, `adapter.rs`, or `test_discord_adapter.rs`.

## Scoped Cleanup
- Modifications strictly restricted to `src/discord/commands.rs`, `src/discord/adapter.rs`, `tests/test_discord_adapter.rs`, and evidence artifacts.
- No changes to daemon, storage, remote backend, cron/calendar, global config, or dependencies.
- No git commits or pushes created.
