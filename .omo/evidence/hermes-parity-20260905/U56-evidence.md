# U56 Evidence: Effective Config Import and Policy Reporting

## Metadata
- Unit: U56 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: CFG.C02 (Import schema admitted only model/approval-mode/two Discord token fields and four scalar env keys; dropped allowed/ignored channels, roles, YAML allow_from, model aliases like `model.name`, and activated disabled profile tokens into active pool)
- Citations:
  - Live: `src/migrate/config_import.rs`, `src/main.rs`
  - Hermes / Upstream parity: `gateway/config.py:1187-1216, 1373-1558, 1698-1720`, `plugins/platforms/discord/adapter.py:9294-9408`
- Date: 2026-09-07

## Implementation Summary
1. **Extended Importer Schema (`src/migrate/config_import.rs`)**:
   - Expanded `HermesModel` to support `name` and `model` alongside `default`, resolving via `effective_default()`.
   - Expanded `HermesDiscord` to deserialize `enabled`, `allowed_users`, `allowed_channels`, `ignored_channels`, `allowed_roles`, `free_response_channels`, and `home_channel`.
   - Expanded `SCALAR_ENV_KEYS` to include all 10 scalar policy keys: `DISCORD_ALLOWED_USERS`, `DISCORD_ALLOWED_CHANNELS`, `DISCORD_IGNORED_CHANNELS`, `DISCORD_ALLOWED_ROLES`, `DISCORD_ALLOW_ALL_USERS`, `DISCORD_THREAD_SESSIONS_PER_USER`, `DISCORD_THREAD_REQUIRE_MENTION`, `DISCORD_FREE_RESPONSE_CHANNELS`, `DISCORD_HOME_CHANNEL`, `APPROVAL_MODE`.
2. **Policy Mapping & Disabled Profile Gating**:
   - Mapped YAML Discord policy lists (`allowed_users`, `ignored_channels`, etc.) into comma-separated env values.
   - Gated profile bot tokens against `config.discord.enabled`: profiles with `enabled: false` are excluded from `DISCORD_BOT_TOKENS`.
3. **Regression Verification (`src/migrate/config_import.rs::tests::imports_effective_discord_policy`)**:
   - Payload token x, allowed user 42, ignored channel 99, disabled profile bot with `enabled: false`, model alias `name: gpt-4o`.
   - RED captured: lost keys and model alias (`assertion left == right failed: left: None, right: Some("gpt-4o")`, exit 101).
   - GREEN verified: mapped policy preserved, disabled token omitted from active tokens, exit 0.

## Verification
- Captured RED: `U56-red.log`, `U56-red.exit` (exit 101, panic: `assertion left == right failed: left: None, right: Some("gpt-4o")`)
- Captured GREEN: `U56-green.log`, `U56-green.exit` (exit 0)
- Single test execution: 1 passed in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
