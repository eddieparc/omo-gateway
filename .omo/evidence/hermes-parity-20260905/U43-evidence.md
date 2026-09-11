# U43 Evidence: Canonical Discord Home and Fanout Targets Parity

## Metadata
- Unit: U43 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C10, CFG.C10 (`all`/bare `discord` used origin only without configured home fallback; `discord:42:43` remained unparsed `42:43` and failed numeric parse; uppercase platform `Discord:` ignored; imported array `deliver` failed whole-document deserialization despite native array support)
- Citations:
  - Live: `src/cron/store.rs`, `src/cron/scheduler.rs`, `tests/test_cron_boundary_parity.rs`
  - Hermes / Upstream parity: `cron/scheduler.py:1136-1297`, `gateway/delivery.py:161-205`, `gateway/config.py:1756-1763`
- Date: 2026-09-07

## Implementation Summary
1. **Flexible `deliver` Deserializer (`deserialize_deliver`)**:
   - Replaced rigid `Option<String>` with `Option<Vec<String>>` via untagged enum deserialization accepting single string (with comma splitting) or sequence of strings.
   - Preserved document-level deserialization for jobs configured with array targets like `["Discord:42:43", "all"]`.
2. **Channel and Thread Reference Normalization**:
   - Case-insensitive platform prefix matching (e.g. `Discord:`, `discord:`).
   - Parses composite references `channel:thread` (e.g. `"Discord:42:43"` -> `chat_id: "42"`, `thread_id: Some("43")`), eliminating downstream numeric parse errors on Discord delivery.
   - Supports raw composite strings `"42:43"` and standard channel IDs `"42"`.
3. **Configured Discord Home Expansion and Fan-out Dedup**:
   - `HermesJob::configured_discord_home`: checks profile `config.yaml` (`discord.channel`, `home_channel`, `thread_id`) and payload extra (`home_channel`, `discord_home_channel`, etc.).
   - `origin`, `all`, and bare `discord` target specs fall back to the configured profile home channel and thread when `origin` is absent.
   - Deduplicates targets in-memory using `(chat_id, thread_id)` sets so `["discord", "all"]` yields exactly one delivery target.
4. **Regression Verification**:
   - `tests/test_cron_boundary_parity.rs::target_forms_resolve`:
     - RED captured: deserialization failure on sequence deliver: `Error("invalid type: sequence, expected a string")` (exit 101).
     - GREEN verified: resolved into `(42, Some("43"))` and `(44, None)` (exit 0).
   - `src/cron/store.rs::tests::discord_delivery_uses_profile_home`:
     - RED captured: deserialization failure on array deliver (exit 101).
     - GREEN verified: resolved into single deduped target `(42, Some("43"))` (exit 0).

## Verification
- Captured RED: `U43-red.log`, `U43-red.exit` (exit 101, panic: HermesJob must deserialize with array deliver)
- Captured GREEN: `U43-green.log`, `U43-green.exit` (exit 0)
- Full `test_cron_boundary_parity` suite: 4 passed in 0.23s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
