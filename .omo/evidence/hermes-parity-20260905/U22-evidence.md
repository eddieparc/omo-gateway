# Hermes Parity Unit U22 Evidence: Durable Split-Message Constituent Dedup

## Defect Summary
- **D.D03 (P0 #6; P1 transcript dedup)**:
  - In `src/discord/adapter.rs` (~R:492-611 and live path ~R:938-945), rapid split messages were debounced, but `coalesce_inbound_events` did not deduplicate constituent IDs before concatenation. A duplicate of the last chunk within the same batch caused duplicate body content in the merged turn (`chunk 8\nchunk 9\nchunk 9`).
  - `coalesce_inbound_events` retained only the last constituent message ID (`last.platform_message_id` and `last.delivery_id`), and ingress ledger claims occurred later at `route_claimed_event` (~R:1075-1102). Consequently, earlier constituents (e.g. chunk 8) were never claimed in `delivery_ledger`. Replaying an already-completed first chunk (e.g. chunk 8) bypassed duplicate detection and re-ran the entire turn.
  - In `SplitMessageDebouncer`, batch generations were tracked with a simple counter restarting at zero on removal/cancellation (`DebounceBatch.generation`). If a batch was cancelled and an equal-size new batch was immediately enqueued for the same session, an older sleeping task could wake up with an equal generation (`batch.generation == generation`) and prematurely flush the replacement batch before its debounce window had elapsed.
  - Upstream Hermes (`plugins/platforms/discord/adapter.py:7558-7641; 7529-7549`) enforces unique batch tokens, constituent deduplication before merge, and constituent visibility across the claim boundary while deliberately excluding recovered backfill events from batching.

## Fix Architecture

1. **Unique Lifetime Batch Token per Debounce Generation (`src/discord/adapter.rs`)**:
   - Replaced per-batch counter with a global atomic generation token generator (`static NEXT_BATCH_TOKEN: AtomicU64`).
   - Each `enqueue` assigns a monotonically increasing unique token to `DebounceBatch.token`.
   - The spawned sleeper captures its specific `token`. When waking, it compares `batch.token == token`.
   - Cancelling or removing a batch and re-enqueuing an equal-size replacement gives the new batch a strictly higher lifetime token, preventing older sleepers from ever matching or prematurely flushing new batches.

2. **Constituent Deduplication Before Merge (`src/discord/adapter.rs`)**:
   - In `coalesce_inbound_events`, constituent events are deduplicated by constituent ID (`platform_message_id` or `delivery_id`) in arrival order before merging content or unioning attachments.
   - A batch containing constituent IDs [8, 9, 9] preserves [8, 9], producing `chunk-8\nchunk-9` without duplicate content.

3. **Durable Constituent Claims Across Ingress Boundary (`src/storage/db.rs`, `src/ledger/service.rs`, `src/discord/adapter.rs`)**:
   - Added durable mapping table `delivery_ledger_constituents (parent_delivery_id, constituent_id PRIMARY KEY)` and index on `parent_delivery_id` in SQLite.
   - Added `DeliveryLedgerService::record_incoming_with_constituents(&self, event, delivery_id, constituent_ids)`:
     - Checks duplicate status against both `delivery_id` and all constituent IDs before claiming.
     - Claims the primary delivery in `delivery_ledger`.
     - Records each constituent ID in `delivery_ledger` and associates it with `parent_delivery_id` in `delivery_ledger_constituents`.
   - In `DeliveryLedgerService::complete`:
     - When the actor completes delivery (`mark_delivered` or `mark_failed`), updates the primary delivery row and propagates status, error, and completion latency to all linked constituents in `delivery_ledger`.
   - In `DeliveryLedgerService::is_duplicate` and `get`:
     - Resolves messages by exact `message_id`, `platform_message_id`, or with/without `"discord:"` transport prefix.
   - In `SplitMessageDebouncer`:
     - Collects constituent delivery IDs from buffered events and invokes `route_claimed_event_with_constituents(&data, coalesced, &constituent_ids)`.
     - When an already-completed constituent (e.g. chunk 8) is replayed, `delivery_ledger` recognizes the existing delivered constituent row and rejects the claim; the turn does NOT re-run.

4. **Deterministic Regression Test (`tests/test_discord_adapter.rs`)**:
   - Implemented `split_batch_replay_and_generation`:
     - Scenario 1: Enqueues constituent IDs 8, 9, 9 in one batch, advances simulated time past debounce window, verifies runner executes exactly one turn with deduplicated body `"chunk-8\nchunk-9"`, and confirms durable claim rows for both 8 and 9 are marked `delivered`.
     - Scenario 2: Replays constituent ID 8 after turn completion, advances simulated time, and confirms runner execution count remains 1 (replayed turn does not execute).
     - Scenario 3: Enqueues old batch for session 2, advances 200ms into 600ms debounce window, cancels old batch, enqueues equal-size new batch, advances 400ms (T=600ms from start, T=400ms for new batch), asserts no early flush occurs at T=400ms, then advances remaining 200ms to T=600ms and asserts successful execution and delivery claim.
     - Zero wall-clock sleeps; asserts exclusively against completed runner events and durable ledger claim rows.

## RED Phase Verification

Executed before source implementation; failed non-zero (`exit: 101`):

- **Command**: `cargo test --test test_discord_adapter split_batch_replay_and_generation -- --exact --nocapture`
- **Exit Code**: `101`
- **Failed Assertion**: `tests/test_discord_adapter.rs:2193:9`
  ```
  thread 'split_batch_replay_and_generation' (1584358) panicked at tests/test_discord_adapter.rs:2193:9:
  assertion `left == right` failed: constituent IDs 8, 9, 9 must be deduplicated before merge, producing chunk-8\nchunk-9 without duplicating chunk-9
    left: "chunk-8\nchunk-9\nchunk-9"
   right: "chunk-8\nchunk-9"
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test split_batch_replay_and_generation ... FAILED
  ```
- Recorded in `.omo/evidence/hermes-parity-20260905/U22-red.exit` and `.omo/evidence/hermes-parity-20260905/U22-red.log`.

## GREEN Phase Verification

Executed after source implementation; passed with exit code 0:

- **Command**: `cargo test --test test_discord_adapter split_batch_replay_and_generation -- --exact --nocapture`
- **Exit Code**: `0`
- **Output**:
  ```
  running 1 test
  test split_batch_replay_and_generation ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 1.04s
  ```
- Recorded in `.omo/evidence/hermes-parity-20260905/U22-green.exit` and `.omo/evidence/hermes-parity-20260905/U22-green.log`.

## Adjacent Suite Results

- **`cargo test --test test_discord_adapter`**: 42 passed, 0 failed (`U22-suite.log`)
- **`cargo test --test test_multiplexer`**: 14 passed, 0 failed (`U22-mux.log`)
- **`cargo test --lib storage::db`**: 23 passed, 0 failed (`U22-db.log`)
- **`cargo build`**: exit code 0 (`U22-build.log`)
- **`cargo test --bin omo-gateway`**: 35 passed, 0 failed (`U22-main.log`)

## Formatting and Diff Verification

- **Format Check (`rustfmt --check --edition 2024`)**: exit code 0 (`U22-format.log`)
- **Diff Check (`git diff --check`)**: exit code 0, clean whitespace and formatting (`U22-diff.log`)
