# U46 - Validated timezone-aware schedules (CR.C15, CFG.C09)

Resumed from interrupted session `01a070bb-40f8-7c3e-a02e-e802d250b826`. The prior
worker had registered both fully-qualified tests, captured real behavioral RED, and
authored the test bodies, but had not landed any production change. This resume added
the minimal production behavior and drove both constituents to GREEN. All unrelated
dirty handoff changes in the workspace were preserved untouched.

## Scope and constraints honored

- WRITE ONLY: `src/cron/store.rs`, `src/cron/scheduler.rs`, `Cargo.toml`, `Cargo.lock`,
  `tests/test_cron_schedule_parity.rs`, and `U46-*` evidence. No other source, test,
  migration, dashboard, storage/db, main, or tools file was edited.
- Timezone dependency: `chrono-tz = "0.10"` added. It was genuinely absent - only the
  transitive `iana-time-zone` (a chrono dependency for local-zone detection) was present,
  which cannot resolve arbitrary IANA names. No other dependency was added.
- Serde compatibility: the new `HermesSchedule::timezone` field is
  `#[serde(default, skip_serializing_if = "Option::is_none")]`, so existing Hermes
  fixtures round-trip byte-identically when no timezone is configured.
- No fixed sleeps or polling in tests; time is injected at the existing scheduling
  boundary (`with_clock`) and the executor mutates the shared clock to model completion.

## Binding scenarios

### CR.C15 - persist/import timezone, evaluate wall-clock cron there, store UTC
Profile `Asia/Seoul`, `0 9 * * *`, imported `next_run_at=2026-09-05T00:00:00Z`,
completion at `2026-09-05T00:01:00Z` via injected fixed clock.

- Command: `cargo test --test test_cron_schedule_parity imported_cron_retains_timezone -- --exact --nocapture`
- RED (`U46-resume-red-c15.log`, exit 101): after completion the scheduler recomputed
  `next=Some(2026-09-05T09:00:00Z)` (naive UTC), asserted against `2026-09-06T00:00:00Z`.
- GREEN (`U46-resume-green-c15.log`, exit 0): `next=Some(2026-09-06T00:00:00Z)`,
  `payload.schedule.timezone=Asia/Seoul`, `cron_runs` row `(succeeded, due, completion)`,
  and two reopen/resync cycles keep `next=2026-09-06T00:00:00Z` with `jobs.json` unchanged.

### CFG.C09 - validate full expression before insertion, persist source timezone
Payload expr `garbage` (with both null and a stored `next_run_at`), plus a valid
`0 9 * * *` Asia/Seoul job evaluated at `2026-09-05T00:00:00Z`.

- Command: `cargo test --lib cron::store::tests::imports_timezone_and_rejects_invalid_schedule -- --exact --nocapture`
- RED (`U46-resume-red-c09.log`, exit 101): `garbage` imported as an enabled row with
  `next=null` (`result=Ok(1), rows=[(true, None)]`) - malformed schedule silently persisted.
- GREEN (`U46-resume-green-c09.log`, exit 0): `garbage` is rejected before any row is
  written (`result=Err(Config(...)), rows=[]`) for both the null and stored-next payloads;
  the valid job imports with `next=2026-09-06T00:00:00Z` and `schedule.timezone=Asia/Seoul`.

### DST supplementary (America/New_York)
`cargo test --lib cron::scheduler::tests::next_run_tz_evaluates_wall_clock_and_tracks_dst_offset`
(`U46-resume-green-dst.log`, exit 0) asserts `0 12 * * *` resolves to `17:00Z` in EST
(`2026-01-10`) and `16:00Z` in EDT (`2026-07-10`), proving DST-aware offset selection;
Asia/Seoul (no DST), `None` timezone, and `interval:1h` all fall back to the UTC path, and
`garbage` is rejected. This exercises a brand-new function, so no RED phase applies.

## Implementation (minimal scoped patch)

- `src/cron/scheduler.rs`
  - Added `pub fn next_run_tz(expression, after, timezone)`: for a present IANA timezone
    and a cron expression it parses the schedule, evaluates `.after()` in that zone via a
    `DateTime<chrono_tz::Tz>`, and converts the next instant back to UTC. Interval and
    one-shot schedules are absolute instants and delegate to `next_run`. Invalid cron or
    invalid timezone returns `OmonError::Config`.
  - `complete_success` now reads `schedule.timezone` from the (post-increment) payload and
    advances the recurring schedule via `next_run_tz`, so wall-clock jobs re-arm in-zone.
  - Added unit test `next_run_tz_evaluates_wall_clock_and_tracks_dst_offset`.
- `src/cron/store.rs`
  - Added `HermesSchedule::timezone: Option<String>` (serde default + skip-if-none).
  - Added `HermesStore::timezone()` reading `<home>/config.yaml` (`timezone:` key) via serde_yaml.
  - `sync_at` now (1) validates the full expression up front through `next_run_tz`,
    returning `Err` before any row is written for malformed cron (one-shot-in-the-past stays
    tolerated as before), (2) stamps the source timezone into each job's schedule, and
    (3) computes the fallback `next_run_at` in-zone. Dropped the now-unused `next_run` import.
- `Cargo.toml` / `Cargo.lock`: added `chrono-tz = "0.10"` (0.10.4 pinned in lock).

## Verification chronology (this session)

1. Read handoff (RED logs, commands.json, plan, checkpoint), current store/scheduler,
   manifests, and the parity test.
2. `lsp_diagnostics` (error) on both production files: none.
3. `cargo build`: Finished, exit 0 (`U46-resume-build.log`).
4. GREEN both registered tests (C15, C09) and the DST unit test - logs above.
5. Adjacent regression `cargo test --lib cron::`: 39 passed, 0 failed
   (`U46-resume-adjacent-cron.log`).
6. Full parity test file `cargo test --test test_cron_schedule_parity`: 1 passed.
7. Scoped rustfmt: scheduler.rs changed regions have zero diffs; store.rs changed region
   clean after aligning the new match block; residual store.rs diffs (lines 517/531/539)
   are the preserved handoff RED test body; remaining whole-crate diffs belong to unrelated
   pre-existing dirty files and were left untouched.

## Changed files

- `Cargo.toml`
- `Cargo.lock`
- `src/cron/store.rs`
- `src/cron/scheduler.rs`
- `tests/test_cron_schedule_parity.rs` (handoff RED test; production change makes it GREEN)

## Exact commands

- `cargo test --test test_cron_schedule_parity imported_cron_retains_timezone -- --exact --nocapture`
- `cargo test --lib cron::store::tests::imports_timezone_and_rejects_invalid_schedule -- --exact --nocapture`
- `cargo test --lib cron::scheduler::tests::next_run_tz_evaluates_wall_clock_and_tracks_dst_offset -- --exact --nocapture`
- `cargo test --lib cron::`
- `cargo build`
- `cargo test --test test_cron_schedule_parity`
