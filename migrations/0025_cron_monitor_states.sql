CREATE TABLE IF NOT EXISTS cron_monitor_states (
    job_id TEXT PRIMARY KEY NOT NULL,
    last_hash TEXT NOT NULL,
    last_snapshot TEXT,
    updated_at TEXT NOT NULL
);
