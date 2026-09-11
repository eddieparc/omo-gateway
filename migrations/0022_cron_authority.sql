-- Migration 0022: Durable cron job authority boundaries.
-- Distinguishes hermes_mirror, cutover_pending, and omon_owned.
-- Preserves source provenance while protecting cutover jobs from live-source deletion.
-- Migrations 0017-0021 are reserved by shared-design; actual directory ends at 0016.
-- Applied 0001-0016 and 0024 are never modified.

ALTER TABLE cron_jobs ADD COLUMN authority TEXT NOT NULL DEFAULT 'omon_owned' CHECK (authority IN ('hermes_mirror', 'cutover_pending', 'omon_owned'));

-- Backfill legacy pre-upgrade imported rows: rows with source provenance are hermes_mirror until cutover
UPDATE cron_jobs
SET authority = 'hermes_mirror'
WHERE json_extract(payload_json, '$._omon_hermes_source') IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_cron_jobs_authority
    ON cron_jobs(authority);
