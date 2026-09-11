CREATE TABLE IF NOT EXISTS cron_notepads (
    profile TEXT NOT NULL,
    job_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (profile, job_id, key)
);

CREATE INDEX IF NOT EXISTS idx_cron_notepads_lookup ON cron_notepads(profile, job_id);
