# U61 Evidence: Wire Final Footer and Truthful Config Docs

## Metadata
- Unit: U61 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: CFG.C13, D.D10 (`runtime_footer` parsed but neither copied to `PoiseData` nor wired into active egress; dashboard reported default approval `ask` though actual default is `smart`; README advertised `omon-gateway migrate` instead of cargo target `omo-gateway migrate` and claimed unmapped keys were removed during merge)
- Citations:
  - Live: `src/main.rs`, `src/dashboard_runtime.rs`, `src/discord/adapter.rs`, `README.md`, `tests/test_profile_routing.rs`, `tests/test_discord_adapter.rs`
  - Hermes / Upstream parity: `gateway/display_config.py:182-241`
- Date: 2026-09-07

## Implementation Summary
1. **Runtime Footer Egress Wiring (`src/discord/adapter.rs`)**:
   - Added `pub runtime_footer: bool`, `default_model: Option<String>`, and `workspace_root: Option<PathBuf>` to `DiscordEgress`.
   - In `DiscordEgress::dispatch`: when `self.runtime_footer` is active, decorates the final assistant message content with `append_runtime_footer(&content, model, None, cwd)`.
   - Wired `self.message_transport` delegation for direct outbound testing and mock transports.
2. **Main Configuration & Poise Wiring (`src/main.rs`)**:
   - Wired `poise_data.runtime_footer = config.runtime_footer;`.
   - Configured `discord_egress` with `.with_runtime_footer(config.runtime_footer)`, `.with_default_model(config.default_model.clone())`, and `.with_workspace_root(config.workspace_root.clone())`.
3. **Dashboard Truthful Policy Representation (`src/dashboard_runtime.rs`)**:
   - Corrected dashboard approval policy default from `"ask"` to `"smart"` to match `ApprovalPolicy::parse`.
4. **Documentation Accuracy (`README.md`)**:
   - Updated CLI invocation references from `omon-gateway migrate` to `omo-gateway migrate`.
   - Updated config mapping tables and noted that unmapped target keys in an existing gateway `.env` are preserved during merge.
5. **Regression Verification**:
   - `tests/test_discord_adapter.rs::runtime_footer_toggle_is_wired`:
     - RED captured: egress failed without message transport delegation / footer wiring (`status_code: 401 Unauthorized`, exit 101).
     - GREEN verified: footer enabled appends `_model_x · /tmp/test_workspace_`, footer disabled leaves message unmodified (exit 0).
   - `tests/test_profile_routing.rs::runtime_footer_flag_reaches_final_egress`:
     - GREEN verified: footer flag decoration matches expected metadata (exit 0).

## Verification
- Captured RED: `U61-red.log`, `U61-red.exit` (exit 101)
- Captured GREEN: `U61-green.log`, `U61-green.exit` (exit 0)
- Single test runs: T1 exit 0 in 0.00s, T2 exit 0 in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
