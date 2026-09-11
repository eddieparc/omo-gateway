CREATE TABLE IF NOT EXISTS cron_incidents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL,
    error_signature TEXT NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(job_id, error_signature)
);

CREATE INDEX IF NOT EXISTS idx_cron_incidents_lookup ON cron_incidents(job_id, error_signature);
