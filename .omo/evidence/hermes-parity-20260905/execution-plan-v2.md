# Hermes parity — regenerated execution plan (v2)

Replaces the stale `completion-checklist.md` accounting (written at the 16-unit mark) and the
deleted `.omo/senpi-task/dag` state. Numbers below are derived from evidence files on disk,
not from worker claims.

## 1. Honest state (evidence-derived)

| Bucket | Count | Units |
|---|---|---|
| GREEN / evidence on disk | 38 | U01-U22, U34, U46, U47, U50-U55, U62, U63, U65, U74, U75, U76, U84 |
| RED captured, implementation in flight | 1 | U23 |
| No evidence at all | 46 | U24-U33, U35-U45, U48, U49, U56-U61, U64, U66-U73, U77-U83, U85 |

A previously reported "49/85" was inflated; the disk-backed number is 38 complete.

## 2. Why the previous approach stalled

- Both Gemini 3.8 Flash routes (`mahoquot`, `quotio`) returned HTTP 503 "no available accounts",
  killing two U23 workers mid-implementation.
- Worker output required lead repair anyway (compile error, racy test assertions, a wrong
  format assertion), so per-unit wall-clock stayed high while the delegation policy assumed
  workers were cheap and reliable.
- Sequential one-unit-at-a-time dispatch ignored that most remaining units touch a small set of
  hot files, so parallelism was never the real bottleneck — serialization on those files is.

## 3. File contention (drives the new batching)

| Contended file | Units |
|---|---|
| `src/main.rs` | 27 units |
| `src/discord/adapter.rs` | 17 units |
| `src/cron/scheduler.rs` | 15 units |
| `src/dashboard_runtime.rs` | 10 units |
| `src/agent/omo_backend.rs` | 7 units |
| `src/tools/cron.rs` | 7 units |
| `src/cron/store.rs` | 6 units |

Units sharing a hot file cannot run concurrently without conflicting edits. The plan therefore
groups by **owning file**, runs groups in parallel, and serializes within a group.

## 4. Regenerated DAG (lanes run in parallel; units inside a lane run in order)

- **L0 (blocking, lead):** U23 finish — cursor durability + failed-delivery reclaim. Everything
  touching `discord/adapter.rs` waits on this.
- **L1 adapter lane** (after L0): U24, U25, U28, U29, U30, U31, U32, U35, U44, U71, U72, U73, U83
- **L2 cron lane:** U38, U39, U41, U42, U43, U45, U49, U78, U79, U80, U81, U82
- **L3 dashboard/runtime lane:** U48, U56, U58, U59, U60, U61, U64, U66, U67, U68, U69, U70
- **L4 backend/agent lane:** U26, U27, U33, U36, U37, U57, U77, U85
- **L5 (final, lead):** C3 dashboard smoke + C4 fmt/clippy/build/test + baseline-preservation audit

## 5. Execution rules for this phase

1. Lead implements directly whenever a Flash route is 503; no waiting on dead providers.
2. Per unit: RED capture -> minimal production change -> GREEN with the identical invocation.
   No sleeps, no polling, no weakened or skipped assertions.
3. Async assertions are event-gated (channel signal + bounded timeout), never timing luck.
4. Verification is batched per lane, not per unit: one `cargo test` sweep closes a lane.
5. No `.env`, production, real-Discord, commit, or push operations at any point.

## 6. Fixes landed in this session (U23)

- `src/discord/adapter.rs`: backfill waits for the session lane to drain, then advances the
  per-bot cursor only when the ledger reports `Delivered`; otherwise it holds the cursor and
  stops the channel scan so the message is retried.
- `src/ledger/service.rs`: `record_incoming_as` now re-claims a `failed` delivery instead of
  silently discarding it via `ON CONFLICT DO NOTHING`. Without this a failed turn was lost
  permanently — a production data-loss bug, not just a test obstacle.
- `src/multiplexer/router.rs`: added `wait_for_session_idle`, a notification-driven lane drain.
- `src/storage/db.rs`: source-timestamp test made event-gated instead of racing the actor.
