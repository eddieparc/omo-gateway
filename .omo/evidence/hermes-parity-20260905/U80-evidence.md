# U80 Evidence: Suppress Acknowledged Repeated Cron Failure Incidents

## Metadata
- Unit: U80 (Lane 2: Cron Job & Egress Pipeline)
- Findings: UP.DS07 (Acknowledged cron incidents could not suppress repeated failure pings; scheduler blasted failure pings on every execution even when an incident was acknowledged, and no acknowledge operation existed in `CronTool`)
- Citations:
  - Live: `migrations/0026_cron_incidents.sql`, `src/cron/scheduler.rs`, `src/tools/cron.rs`, `tests/test_voice_cron.rs`
  - Hermes / Upstream parity: `cron/incidents.py:134-203`, `cron/scheduler.py:2621-2628`
- Date: 2026-09-07

## Implementation Summary
1. **Incident State Persistence (`migrations/0026_cron_incidents.sql`)**:
   - Created table `cron_incidents (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, error_signature TEXT NOT NULL, acknowledged INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, UNIQUE(job_id, error_signature))`.
   - Created index `idx_cron_incidents_lookup ON cron_incidents(job_id, error_signature)`.
2. **Scheduler Incident Acknowledgment & Gating (`src/cron/scheduler.rs`)**:
   - Added `CronScheduler::acknowledge_incident(&self, job_id: &str, signature: Option<&str>) -> Result<bool>` to mark incidents acknowledged in the database.
   - In `CronScheduler::execute_job`:
     - On failure: computes error signature. Checks if incident exists with `acknowledged != 0`. If acknowledged, logs and suppresses repeated delivery notification. If new or unacknowledged, records/updates the incident and delivers the notification.
     - On success: clears outstanding failure incidents for the job (`DELETE FROM cron_incidents WHERE job_id = ?`).
3. **Cron Tool Operations (`src/tools/cron.rs`)**:
   - Added `ack`, `acknowledge`, and `ack_incident` actions taking `id` / `job_id` and optional `signature`.
4. **Regression Verification (`tests/test_voice_cron.rs::acknowledged_failure_signature_stays_silent`)**:
   - Initial failure: generates alert notification (`messages.len() == 1`).
   - Acknowledgment: `scheduler.acknowledge_incident("failing_job_incidents", None)`.
   - Repeated failure with identical signature: stays silent.
     - RED captured: sent second alert without checking incident state (`assertion left == right failed: left: 2, right: 1`, exit 101).
     - GREEN verified: stays silent on identical error signature (`messages.len() == 1`, exit 0).
   - Subsequent failure with modified error signature: triggers fresh notification (`messages.len() == 2`).

## Verification
- Captured RED: `U80-red.log`, `U80-red.exit` (exit 101, panic: `Acknowledged failure signature must stay silent`)
- Captured GREEN: `U80-green.log`, `U80-green.exit` (exit 0)
- Single test execution: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
