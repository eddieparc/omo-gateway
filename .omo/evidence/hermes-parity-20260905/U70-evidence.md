# U70 Evidence: Cross-Process Runtime Ownership Lock

## Metadata
- Unit: U70 (Lane 3: Dashboard, Operations, Observability & Cutover)
- Findings: R.R22 (Single process ownership / duplicate gateway startup: only checked duplicate bot identities within one config; lacked process-level or credential-scoped runtime lock, allowing duplicate gateway processes to collide against the same daemon, store, and Discord transports)
- Citations:
  - Live: `src/entry.rs`, `src/main.rs`, `src/dashboard_runtime.rs`
  - Hermes / Upstream parity: `gateway/status.py:882-924, 1191-1222`
- Date: 2026-09-07

## Implementation Summary
1. **RAII OS File Lock (`src/entry.rs`)**:
   - Implemented `RuntimeOwnershipLock` acquiring an exclusive non-blocking OS lock (`libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB)` on Unix) on a scoped lockfile in `OMON_RUNTIME_LOCK_DIR` / temp dir.
   - Preserves RAII semantics: closing the underlying file handle upon process termination or variable drop automatically and reliably relinquishes ownership.
   - Rejects concurrent gateway startup attempts with `another gateway instance owns runtime lock`.
2. **Regression Verification (`src/entry.rs::tests::duplicate_gateway_start_is_rejected`)**:
   - First lock acquisition: succeeds.
   - Second lock acquisition with identical runtime identity:
     - RED captured: both acquired without cross-process locking (`called Result::unwrap_err() on an Ok value`, exit 101).
     - GREEN verified: explicitly rejected with `another gateway instance owns runtime lock` (exit 0).
   - Drop first lock: third acquisition successfully acquires ownership.

## Verification
- Captured RED: `U70-red.log`, `U70-red.exit` (exit 101, panic: `called Result::unwrap_err() on an Ok value`)
- Captured GREEN: `U70-green.log`, `U70-green.exit` (exit 0)
- Single test execution: 1 passed in 0.00s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
