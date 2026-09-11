# U44 Evidence: Aggregate Delivery Receipts Before ACK Parity

## Metadata
- Unit: U44 (Lane 2: Cron Job & Egress Pipeline)
- Findings: CR.C11 (ACK previously executed inside individual `deliver` calls per destination, causing first good destination to trigger ACK before later failures; failure alert messages triggered unwanted ACK commands; partial fanout failure aborted remaining destinations)
- Citations:
  - Live: `src/cron/scheduler.rs`, `src/discord/adapter.rs`, `tests/test_cron_delivery_parity.rs`
  - Hermes / Upstream parity: `cron/scheduler.py:1368-1387, 1530-1532, 2011-2013`, `gateway/delivery.py:266-313`
- Date: 2026-09-07

## Implementation Summary
1. **Per-Target Delivery Receipt Aggregation (`src/cron/scheduler.rs`)**:
   - In `CronScheduler::execute_job`: attempts all fan-out destinations sequentially without early abort on the first failure, collecting any errors in `delivery_errors`.
   - If any destination fails, returns `OmonError::Config` with aggregate failure count and error detail, allowing failure tracking and preventing false completion.
2. **Strict Aggregate ACK Gating**:
   - Removed `run_ack_logged` invocation from individual destination `deliver` calls.
   - Guarded ACK execution: runs `ack_command` only at the top-level aggregate after ALL fan-out destinations succeed without error.
   - Error alert deliveries and silent/empty responses (`[SILENT]`, `is_cron_silence_response`) explicitly bypass ACK execution.
3. **Media Upload and Delivery Chain Integration**:
   - Media directive extraction (`MEDIA:<path>`) and upload failure propagation in `DiscordEgress` and dispatcher ensures that failed media uploads fail the delivery action, thereby blocking ACK execution.
4. **Regression Verification (`tests/test_cron_delivery_parity.rs::ack_requires_complete_output_delivery`)**:
   - Case 1: Partial delivery failure (dest 42 succeeds, 43 fails) -> RED captured ACK count = 1 before failure -> GREEN verified ACK count = 0.
   - Case 2: Complete fanout success (42, 43 both succeed) -> GREEN verified ACK count = 1.
   - Case 3: Executor error alert dispatch -> GREEN verified ACK count remains 1 (0 additional ACKs).
   - Case 4: Media upload handling:
     - 4a: Failed media upload fails delivery -> ACK count remains 1 (0 additional ACKs).
     - 4b: Successful media upload and message delivery -> ACK count becomes 2 (exactly 1 additional ACK).

## Verification
- Captured RED: `U44-red.log`, `U44-red.exit` (exit 101, panic: ACK must NOT run when one or more destinations fail delivery, left: 1, right: 0)
- Captured GREEN: `U44-green.log`, `U44-green.exit` (exit 0)
- Full `test_cron_delivery_parity` suite: 1 passed in 0.02s
- `cargo clippy --lib --tests -- -D warnings`: EXIT 0 (0 warnings)
- `cargo fmt --check`: EXIT 0
