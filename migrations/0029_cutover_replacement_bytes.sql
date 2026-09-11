-- Keep the exact per-store payload in the pending receipt transaction.
-- NULL identifies legacy receipts: recovery must not invent historical bytes.
ALTER TABLE cron_cutover_receipt_stores ADD COLUMN replacement_bytes BLOB;
