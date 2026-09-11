# U77 Evidence: Terminate Owned Cron Script Descendant Trees Parity

## Metadata
- Unit: U77 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS04 (Cron script timeout killed only the direct child process; background child processes, subshells, and grandchildren remained alive as orphaned processes, leaking side effects across tasks)
- Citations:
  - Live: `src/cron/executor.rs`, `src/main.rs`
  - Hermes / Upstream parity: `cron/scheduler_script.py:188-212, 338-378`, `cron/scheduler.py:2197-2243`
- Date: 2026-09-07

## Implementation Summary
1. **Process Group Ownership (`src/cron/executor.rs`)**:
   - In `run_cron_script` and `execute_native_cron`: sets `command.process_group(0)` on Unix systems prior to spawn so the child becomes a process group leader (`pgid == pid`).
   - Descendant processes (background tasks, subshells, grandchildren) inherit this isolated process group.
2. **Descendant Tree Termination on Timeout**:
   - When `tokio::time::timeout` expires on the script, retrieves `child.id()`.
   - Sends `SIGKILL` to the entire process group via `libc::kill(-(pid as i32), libc::SIGKILL)`, ensuring all spawned descendants are promptly terminated rather than left orphaned.
3. **Regression Verification (`src/main.rs::legacy::runner_tests::cron_script_timeout_reaps_descendants`)**:
   - Executed script `leaker.sh` which spawned a background `sleep 30` process and recorded its PID.
   - Script configured with a 1-second timeout.
   - RED captured: direct child died on timeout but descendant process remained alive (`assertion left != right failed: Descendant process ... must NOT be running after script timeout, left: 0, right: 0`, exit 101).
   - GREEN verified: after timeout expiration, the descendant PID was confirmed dead (`libc::kill(pid, 0) != 0`, exit 0).

## Verification
- Captured RED: `U77-red.log`, `U77-red.exit` (exit 101, panic: Descendant process must NOT be running after script timeout)
- Captured GREEN: `U77-green.log`, `U77-green.exit` (exit 0)
- Full bin test suite: 1 passed in 1.11s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
