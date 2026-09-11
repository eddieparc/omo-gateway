CREATE TABLE IF NOT EXISTS dead_targets (
    bot_id TEXT NOT NULL,
    channel_id INTEGER NOT NULL,
    status_code INTEGER NOT NULL,
    error_message TEXT NOT NULL,
    dead_since TEXT NOT NULL,
    probed_at TEXT,
    PRIMARY KEY (bot_id, channel_id)
);
