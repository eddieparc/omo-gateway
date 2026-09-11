# U57 Evidence: Per-Bot Named Profile Policy

## Metadata
- Unit: U57 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: CFG.C03 (All profile tokens shared one global ACL and first-profile scalar fallback; Hermes route `profile: work` was not represented by Rust inline routes; work profile user 2 was denied if root only allowed user 1, and root user 1 got unauthorized work-bot access)
- Citations:
  - Live: `src/migrate/config_import.rs`, `src/multiplexer/profile_routing.rs`, `src/discord/adapter.rs`, `src/main.rs`
  - Hermes / Upstream parity: `gateway/profile_routing.py:51-64`, `gateway/run.py:18860-18928`, `hermes_cli/profiles.py:949-987`
- Date: 2026-09-07

## Implementation Summary
1. **Profile Route Extensions (`src/multiplexer/profile_routing.rs`)**:
   - Added `profile: Option<String>` and `allowed_users: Option<Vec<u64>>` to `ProfileRoute`.
   - In `ProfileRoute::apply_to_session`: records `profile` into session state metadata when specified on the matched route.
   - Implemented `ProfileRouter::is_user_allowed_for_bot_or_route(&self, bot_id, channel_id, user_id, global_allowed_users) -> bool`:
     - Checks channel route and bot-matching route: if the matched route specifies its own `allowed_users`, enforces that route's user policy instead of leaking the root global ACL.
     - Falls back to global allowed users when no route-specific ACL is defined.
2. **Regression Verification (`tests/test_profile_routing.rs::imported_profiles_preserve_bot_policy`)**:
   - Route for channel 20 targets profile "work" with `allowed_users: [2]`. Global allowed users: `[1]`.
   - User 1 on channel 20 (profile work):
     - RED captured: evaluated under global policy and allowed (`assertion left == right failed: User 1 must be denied on profile work`, exit 101).
     - GREEN verified: denied under profile work policy (exit 0).
   - User 2 on channel 20 (profile work): accepted under profile work policy.
   - Session application: successfully applies profile `"work"` into session state metadata.

## Verification
- Captured RED: `U57-red.log`, `U57-red.exit` (exit 101, panic: `User 1 must be denied on profile work`)
- Captured GREEN: `U57-green.log`, `U57-green.exit` (exit 0)
- Single test execution: 1 passed in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
