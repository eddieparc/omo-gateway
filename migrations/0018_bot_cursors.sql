-- Per-bot Discord channel cursors for crash recovery and missed-message backfill
CREATE TABLE IF NOT EXISTS discord_bot_cursors (
    bot_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    last_message_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (bot_id, channel_id)
);

CREATE INDEX IF NOT EXISTS idx_discord_bot_cursors_bot_channel
    ON discord_bot_cursors (bot_id, channel_id);

-- Migrate legacy un-scoped channel cursors if present (with empty bot_id)
INSERT OR IGNORE INTO discord_bot_cursors (bot_id, channel_id, last_message_id, updated_at)
SELECT '' AS bot_id, channel_id, last_message_id, updated_at FROM discord_channel_cursors;
