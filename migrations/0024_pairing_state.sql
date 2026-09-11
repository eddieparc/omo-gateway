-- Migration 0024: Persistent pairing platform lockout and notification throttling state.
-- Migrations 0017-0023 are reserved by shared-design; actual directory ends at 0016.
-- Applied 0011_pairing_codes.sql is never modified.

CREATE TABLE IF NOT EXISTS pairing_platform_lockout (
    platform_key TEXT PRIMARY KEY,
    locked_until TEXT NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS pairing_notifications (
    user_id TEXT PRIMARY KEY,
    last_notified_at TEXT NOT NULL
);
