-- Migration 0023: Quiesced receipt-verified all-store cutover journal.
-- Tracks durable pending/completed cutover receipts, store progress, hashes, and backups.
-- Migrations 0017-0021 are reserved by shared-design; 0022 is cron_authority; 0024 is pairing_state.
-- Applied 0001-0016, 0022, and 0024 are never modified.

CREATE TABLE IF NOT EXISTS cron_cutover_receipts (
    operation_id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('pending', 'committed', 'failed', 'rolled_back')),
    store_count INTEGER NOT NULL,
    policy_digest TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cron_cutover_receipt_stores (
    operation_id TEXT NOT NULL,
    profile TEXT NOT NULL,
    store_path TEXT NOT NULL,
    original_hash TEXT NOT NULL,
    replacement_hash TEXT NOT NULL,
    backup_path TEXT NOT NULL,
    job_digests_json TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('prepared', 'backed_up', 'replaced', 'committed')),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (operation_id, store_path),
    FOREIGN KEY (operation_id) REFERENCES cron_cutover_receipts(operation_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_cron_cutover_receipts_status
    ON cron_cutover_receipts(status);
