-- Raw rows remain authoritative until OD-01 defines a collision-history policy.
-- This table records deferral only; it is never used to select or alias a history.
CREATE TABLE legacy_guild_session_conflicts (
    raw_key TEXT PRIMARY KEY NOT NULL,
    canonical_key TEXT NOT NULL
);
CREATE INDEX idx_legacy_guild_session_conflicts_canonical
    ON legacy_guild_session_conflicts(canonical_key);

-- A legacy per-user lane can establish an initiator only for the exact pending
-- message present at upgrade, never for later work in the shared canonical lane.
CREATE TABLE legacy_guild_pending_auth (
    message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    message_sequence INTEGER NOT NULL,
    canonical_key TEXT NOT NULL,
    raw_key TEXT NOT NULL,
    author_id TEXT NOT NULL
);

-- Normal ingress must not create a shadow lane, or execute an arbitrarily chosen
-- existing canonical row. Session creation/upsert precedes backend side effects.
-- RAISE(ABORT) also applies to INSERT OR IGNORE / ON CONFLICT DO NOTHING callers.
CREATE TRIGGER defer_legacy_guild_session_insert
BEFORE INSERT ON sessions
WHEN EXISTS (
    SELECT 1 FROM legacy_guild_session_conflicts
    WHERE raw_key = NEW.session_key OR canonical_key = NEW.session_key
)
BEGIN
    SELECT RAISE(ABORT, 'legacy guild session collision deferred: OD-01 unresolved');
END;
