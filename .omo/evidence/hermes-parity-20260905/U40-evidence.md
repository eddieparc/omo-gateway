# U40 Evidence: Skill-Only Jobs and Slash Bundles Parity

## Metadata
- Unit: U40 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C06 (Promptless skill-only jobs rejected before loading any skill; missing skills with real prompt warning vs empty prompt precheck; /bundle rejected as absolute path rather than normalizing slash command)
- Citations:
  - Live: `src/cron/executor.rs`, `src/cron/mod.rs`, `src/main.rs`, `tests/test_cron_executor_parity.rs`
  - Hermes / Upstream parity: `cron/scheduler.py:2339-2352, 2451-2540`, `agent/skill_bundles.py:1-38`
- Date: 2026-09-07

## Implementation Summary
1. **Modular Cron Task Execution (`src/cron/executor.rs`)**:
   - Extracted `AgentCronExecutor` and core skill/bundle helpers (`load_cron_skills`, `resolve_skill_bundle`, `find_skill_file`, `run_cron_script`, `resolve_workspace_instructions`) to dedicated library module `src/cron/executor.rs`, exported via `omon_gateway::cron::executor` for integration testing and runtime modularity.
2. **Slash Command Normalization for Skill Bundles (`resolve_skill_bundle`, `find_skill_file`)**:
   - Normalized input bundle and skill names via `name.trim_start_matches('/')`.
   - Prevented false rejection of slash commands like `"/bundle"` as absolute root paths while preserving strict rejection of directory traversal (`..`).
   - Bundle manifests (`bundle.yaml`, `bundle.json`, etc.) are resolved using normalized identifiers across Hermes home and skill roots.
3. **Skill-Only Job Task Resolution**:
   - In `AgentCronExecutor::execute`: permitted promptless jobs when skills are declared (`!hermes.skills.is_empty() || hermes.skill.is_some()`).
   - Validated effective task post-resolution: if prompt is empty and no skills resolve, rejected with actionable configuration error.
   - When a prompt is present, missing optional skills emit a warning banner (`⚠️ Skill(s) not found and skipped: ...`) and the turn continues with the normal task.
4. **Regression Verification (`tests/test_cron_executor_parity.rs::skill_only_bundle_runs`)**:
   - Scenario 1: `prompt = ""`, `skills = ["/bundle"]`, bundle manifest specifies `["member"]`, `member/SKILL.md` exists.
     - RED captured: rejected before loading skills with panic: `skill-only job with /bundle should execute successfully: Config("Hermes job job_bundle_test has neither prompt nor executable script")` (exit 101).
     - GREEN verified: successfully executed 1 backend turn with resolved `[Skill: member]` and content `Execute member instructions.` (exit 0).
   - Scenario 2: Normal prompt with missing optional skill (`non_existent_skill`) executed second backend turn, preserving prompt and emitting skip warning.

## Verification
- Captured RED: `U40-red.log`, `U40-red.exit` (exit 101)
- Captured GREEN: `U40-green.log`, `U40-green.exit` (exit 0)
- Full `test_cron_executor_parity` suite: 1 passed in 0.01s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
