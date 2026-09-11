# U41 Evidence: Shared Lifecycle Guard on Resolved Scripts Parity

## Metadata
- Unit: U41 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C07 (Import scanned script filename, never file body; executor ran resolved file unchecked; dashboard/REST mutations bypassed guards altogether; overbroad regex blocked unrelated kickstart/systemctl commands)
- Citations:
  - Live: `src/cron/guard.rs`, `src/cron/store.rs`, `src/cron/scheduler.rs`, `src/cron/executor.rs`, `src/dashboard.rs`, `tests/test_cron_boundary_parity.rs`
  - Hermes / Upstream parity: `cron/lifecycle_guard.py:49-66, 76-141`
- Date: 2026-09-07

## Implementation Summary
1. **Target-Specific Lifecycle Regex Narrowing (`src/cron/guard.rs`)**:
   - Refined `GATEWAY_LIFECYCLE_PATTERNS` to specifically target self-restart of the gateway service (`omon`, `hermes`, `omon-gateway`, `hermes-gateway`).
   - Removed blanket matches like `launchctl kickstart` and `systemctl restart`, allowing benign operations for unrelated services (e.g., `launchctl kickstart gui/501/com.example.worker`, `systemctl restart nginx`).
   - Narrowed restart regexes so unrelated prose mentioning "omon" is not falsely flagged.
2. **Central Mutation Lifecycle Validation (`src/cron/scheduler.rs`, `src/dashboard.rs`)**:
   - Implemented `CronScheduler::validate_cron_payload_lifecycle` called directly at registration gates (`register` and `register_with_id`).
   - Validates `prompt`, `script`, `command`, and `ack_command` within the payload JSON before persistent commit.
   - REST dashboard creation and updates automatically inherit this gate and surface `400 Bad Request` on lifecycle violations.
3. **Script Body and Execution Pre-Spawn Verification (`src/cron/store.rs`, `src/cron/executor.rs`)**:
   - Store validation (`HermesJob::validate`): when a job references a script file, scans the actual script file contents from Hermes home scripts directory if present, preventing clean filenames (e.g. `restart.sh`) from concealing self-restart logic.
   - Pre-spawn check (`run_cron_script`, `execute_native_cron`): reads resolved script bytes/string directly from disk before invocation, ensuring that any dynamic or symlinked script modifying gateway lifecycle is rejected with `OmonError::Config` prior to spawning.
   - Checked `ack_command` in both `HermesJob::validate` and `AgentCronExecutor::execute`.
4. **Regression Verification (`tests/test_cron_boundary_parity.rs::lifecycle_guard_covers_rest_and_script_body`)**:
   - Verified benign services (`launchctl kickstart gui/501/com.example.worker`, `systemctl restart nginx`) return `Ok(())`.
   - Verified gateway-targeting kickstart returns `Err`.
   - Verified REST registration with bad script or bad ack_command is rejected.
   - Verified imported `restart.sh` with clean filename but bad script body is rejected before spawn.

## Verification
- Captured RED: `U41-red.log`, `U41-red.exit` (exit 101, panic: launchctl kickstart for unrelated service must be accepted)
- Captured GREEN: `U41-green.log`, `U41-green.exit` (exit 0)
- Full `test_cron_boundary_parity` suite: 2 passed in 0.03s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
