# U85 Evidence: Confirm Destructive Scoped Discord Session Commands

## Metadata
- Unit: U85 (Lane 1: Multiplexer, Router & Chat Protocol)
- Findings: UP.DC03 (Destructive Discord commands like `/reset` had no confirmation gate; immediately purged messages and memories without consent check; `destructive_slash_confirm` was neither imported nor honored)
- Citations:
  - Live: `src/discord/commands.rs`, `src/migrate/config_import.rs`, `src/main.rs`, `tests/test_discord_adapter.rs`
  - Hermes / Upstream parity: `gateway/run_busy.py:1037-1076, 1122-1155`, `gateway/run.py:15617-15677`
- Date: 2026-09-07

## Implementation Summary
1. **Destructive Command Gate (`src/discord/commands.rs`)**:
   - Added `destructive_slash_confirm: bool` (default `true`) to `PoiseData`.
   - Implemented `execute_reset_command(data: &PoiseData, key: &SessionKey, confirm: Option<bool>) -> Result<ResetCommandResult, CommandError>`.
   - When `destructive_slash_confirm` is true and `confirm != Some(true)`, blocks deletion and returns `ResetCommandResult::NeedsConfirmation` prompting the user to re-run with `confirm:true`.
   - When `confirm == Some(true)` or confirmation is disabled, invokes `reset_session` to clear messages, memories, and session state.
2. **Config Import & Runtime Mapping (`src/migrate/config_import.rs`, `src/main.rs`)**:
   - Added `destructive_slash_confirm: Option<bool>` to `HermesApprovals`.
   - Mapped `APPROVALS_DESTRUCTIVE_SLASH_CONFIRM` into `SCALAR_ENV_KEYS` and `Config::from_env`.
   - Wired `poise_data.destructive_slash_confirm = config.destructive_slash_confirm`.
3. **Regression Verification (`tests/test_discord_adapter.rs::destructive_slash_waits_for_confirmation`)**:
   - Initial call without confirmation:
     - RED captured: deleted session immediately without confirmation (`assertion left == right failed: left: Executed, right: NeedsConfirmation`, exit 101).
     - GREEN verified: blocks execution, messages remain intact (`count == 1`, exit 0).
   - Second call with `confirm: Some(true)`: executes purge, messages deleted (`count == 0`, exit 0).

## Verification
- Captured RED: `U85-red.log`, `U85-red.exit` (exit 101, panic: `assertion left == right failed: left: Executed, right: NeedsConfirmation`)
- Captured GREEN: `U85-green.log`, `U85-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
