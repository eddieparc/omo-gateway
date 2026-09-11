CREATE TABLE IF NOT EXISTS cron_outputs (
    profile TEXT NOT NULL DEFAULT '',
    job_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    output TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (profile, job_id, run_id)
);

CREATE INDEX IF NOT EXISTS idx_cron_outputs_lookup ON cron_outputs(profile, job_id, created_at DESC);
