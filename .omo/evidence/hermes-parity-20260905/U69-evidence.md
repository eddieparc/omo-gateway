# U69: Truthful Bot Profile CRUD (R.R13)

## Registration Before Production Edits

Exact integration test id: `legacy::dashboard::tests::bot_profile_delete_stays_deleted` (binary `omo-gateway`, module `legacy::dashboard::tests`).
Literal RED and GREEN command: `cargo test --bin omo-gateway bot_profile_delete_stays_deleted -- --nocapture`

Authoritative specification (Hermes parity R.R13 / `implementation-units.json` / `matrix.md` / `runtime-audit.md`):
- Remove synthetic fallback profiles from bot profile CRUD.
- Propagate DB errors instead of swallowing failures into healthy synthetic lists.
- Unknown bot profile detail requests (`GET /api/bots/{id}`) must return `404 Not Found`, not fabricated success.
- If configured-but-unsaved identities must appear, represent separately as runtime connections, not deletable profile rows.
- Deleting a bot profile (`DELETE /api/bots/{id}`) must ensure the bot profile stays deleted: subsequent `GET /api/bots` reload sequence must NOT contain it, and `GET /api/bots/{id}` must return `404 Not Found`.
- When the database is unavailable, `GET /api/bots` must return an error (`500 Internal Server Error`), not invented inventory.
- Real loopback HTTP API CRUD and BotsPage reload sequence fixture verifying truthful persistence lifecycle.

## Captured Behavioral RED (Before Production Edits)

Literal execution:
`cargo test --bin omo-gateway bot_profile_delete_stays_deleted -- --nocapture`

Pre-production failure captured in `U69-red.log` and `U69-red.exit` (exit code `101`):
```text
running 1 test
U69 POST /api/bots status=200 OK body={"bot_id":"1465631383862120451","name":"wawabot-saved","status":"created"}
U69 GET /api/bots after create: created_in_list=true
U69 GET /api/bots/1465631383862120451 status=200 OK name=String("wawabot-saved")
U69 DELETE /api/bots/1465631383862120451 status=200 OK body={"bot_id":"1465631383862120451","status":"deleted"}
U69 GET /api/bots after delete status=200 OK deleted_in_list=true (RED: synthetic row returns; GREEN: row absent)
U69 GET /api/bots/1465631383862120451 after delete status=200 OK body={"bot_id":"1465631383862120451","created_at":"2026-09-07T15:02:24.120951+00:00","custom_settings":{},"enabled_toolsets":null,"model":null,"name":"wawabot","system_prompt":null,"updated_at":"2026-09-07T15:02:24.120953+00:00"} (RED: 200 fabricated; GREEN: 404)
U69 GET /api/bots/unknown-bot-999 status=200 OK body={"bot_id":"unknown-bot-999","created_at":"2026-09-07T15:02:24.121393+00:00","custom_settings":{},"enabled_toolsets":null,"model":null,"name":"Discord Bot","system_prompt":null,"updated_at":"2026-09-07T15:02:24.121397+00:00"} (RED: 200 fabricated; GREEN: 404)
U69 DB unavailable GET /api/bots status=200 OK invented_inventory=true (RED: 200 with fake rows; GREEN: error)

thread 'legacy::dashboard::tests::bot_profile_delete_stays_deleted' (2555435) panicked at src/dashboard.rs:3254:9:
deleted bot 1465631383862120451 must NOT appear in bot list, but synthetic row was returned: {"items":[{"bot_id":"1465631383862120451","created_at":"2026-09-07T15:02:24.120579+00:00","custom_settings":{},"enabled_toolsets":null,"model":null,"name":"wawabot","system_prompt":null,"updated_at":"2026-09-07T15:02:24.120581+00:00"},{"bot_id":"1529539440589013182","created_at":"2026-09-07T15:02:24.120584+00:00","custom_settings":{},"enabled_toolsets":null,"model":null,"name":"실피","system_prompt":null,"updated_at":"2026-09-07T15:02:24.120585+00:00"},{"bot_id":"1529738312833830984","created_at":"2026-09-07T15:02:24.120585+00:00","custom_settings":{},"enabled_toolsets":null,"model":null,"name":"에리스","system_prompt":null,"updated_at":"2026-09-07T15:02:24.120586+00:00"}],"total":3}
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
test legacy::dashboard::tests::bot_profile_delete_stays_deleted ... FAILED

failures:

failures:
    legacy::dashboard::tests::bot_profile_delete_stays_deleted

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 35 filtered out; finished in 0.02s
```

Defect analysis confirmed by RED output:
1. `DELETE /api/bots/1465631383862120451` removed the bot from SQLite, but the subsequent BotsPage reload sequence `GET /api/bots` reinjected a synthetic fallback row with `"name": "wawabot"` because `1465631383862120451` was hardcoded in `known_bots`.
2. `GET /api/bots/1465631383862120451` after deletion returned `200 OK` with fabricated bot profile data rather than `404 Not Found`.
3. `GET /api/bots/unknown-bot-999` for non-existent bot IDs returned `200 OK` with fabricated bot profile data (`"name": "Discord Bot"`) rather than `404 Not Found`.
4. When the database was unavailable (closed pool), `list_bots` swallowed the error via `.unwrap_or_default()` and returned `200 OK` with 3 invented inventory rows (`wawabot`, `실피`, `에리스`) instead of propagating the error.

## Production Implementation

### `src/dashboard.rs`
1. **Truthful Profile Listing (`list_bots`)**:
   - Removed hardcoded `known_bots` vector and synthetic fallback row injection.
   - Replaced `.unwrap_or_default()` with `?` error propagation: database query failures immediately convert to `ApiError` (`500 Internal Server Error`).
   - The returned `items` array now contains strictly truthful rows from `bot_profiles`.
   - Represented runtime connections separately as `runtime_connections: state.bot_connections` rather than deletable profile rows.

2. **Truthful Detail Lookup (`get_bot`)**:
   - Removed fabricated fallback object branch (`None => { let default_name = match ... }`).
   - Returns `ApiError::not_found("bot profile")` (`404 Not Found`) when the profile does not exist in `bot_profiles`.

3. **Truthful Profile Update (`update_bot`)**:
   - Removed hardcoded `known_bots` name fallbacks.
   - Queries `existing` bot profile from `bot_profiles` first; returns `ApiError::not_found("bot profile")` (`404 Not Found`) if the profile does not exist.
   - Validates that the resulting name is not empty or whitespace-only (`StatusCode::BAD_REQUEST`).
   - Updates the existing database record and returns the updated profile via truthful `get_bot`.

## Exact Proof and Verification

### 1. Exact GREEN Regression
Command:
`cargo test --bin omo-gateway bot_profile_delete_stays_deleted -- --nocapture`
- Exit code: 0 (`U69-green.exit`)
- Result: 1 passed, 0 failed (`U69-green.log`)
- Output:
  ```text
  running 1 test
  U69 POST /api/bots status=200 OK body={"bot_id":"1465631383862120451","name":"wawabot-saved","status":"created"}
  U69 GET /api/bots after create: created_in_list=true
  U69 GET /api/bots/1465631383862120451 status=200 OK name=String("wawabot-saved")
  U69 DELETE /api/bots/1465631383862120451 status=200 OK body={"bot_id":"1465631383862120451","status":"deleted"}
  U69 GET /api/bots after delete status=200 OK deleted_in_list=false (RED: synthetic row returns; GREEN: row absent)
  U69 GET /api/bots/1465631383862120451 after delete status=404 Not Found body={"error":"bot profile not found","status":404} (RED: 200 fabricated; GREEN: 404)
  U69 GET /api/bots/unknown-bot-999 status=404 Not Found body={"error":"bot profile not found","status":404} (RED: 200 fabricated; GREEN: 404)
  U69 DB unavailable GET /api/bots status=500 Internal Server Error invented_inventory=false (RED: 200 with fake rows; GREEN: error)
  test legacy::dashboard::tests::bot_profile_delete_stays_deleted ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 35 filtered out; finished in 0.08s
  ```

### 2. Adjacent Dashboard Tests
Command:
`cargo test --bin omo-gateway dashboard -- --nocapture`
- Exit code: 0 (`U69-adjacent-dashboard.exit`)
- Result: 12 passed, 0 failed (`U69-adjacent-dashboard.log`)
- Suite passed:
  - `legacy::dashboard_runtime::tests::split_csv_trims_and_drops_empty_values`
  - `legacy::dashboard::tests::canonical_session_key_parser_handles_embedded_separators_and_bot_identity`
  - `legacy::dashboard_runtime::tests::dashboard_config_exposes_message_context_policy_matrix`
  - `tests::cli_accepts_dashboard_and_serve_alias`
  - `legacy::dashboard::tests::config_endpoint_never_exposes_provider_secret_values`
  - `legacy::dashboard::tests::health_and_status_endpoints_report_runtime_state`
  - `legacy::dashboard::tests::sessions_list_and_transcript_are_paginated`
  - `legacy::dashboard::tests::cron_crud_routes_use_scheduler_semantics`
  - `legacy::dashboard::tests::bot_profile_delete_stays_deleted` (U69)
  - `legacy::dashboard::tests::disk_pressure_real_loopback_http_surface` (U84)
  - `legacy::dashboard::tests::dashboard_rejects_untrusted_host_and_ws_origin` (U65)
  - `legacy::dashboard::tests::approval_lifecycle_local_http_surface` (U03)

### 3. Binary Build
Command:
`cargo build --bin omo-gateway`
- Exit code: 0 (`U69-build.exit`)
- Log: `U69-build.log`

### 4. Scoped Format and Diff Cleanliness
- `rustfmt --edition 2021 --check src/dashboard.rs`: exit code 0 (`U69-format.exit`, `U69-format.log`)
- `git diff --check src/dashboard.rs`: exit code 0 (`U69-diff.exit`, `U69-diff.log`)

### 5. Invariant Preservation
- Verified context preserved: U65 Host/Origin boundary middleware and tests preserved and green.
- U84 disk classification and loopback tests preserved and green.
- U03 approval HTTP fixture preserved and green (`approval_lifecycle_local_http_surface`).
- File scope restricted strictly to `src/dashboard.rs` and `U69-*` evidence files.
- No sleeps/polling/yield tests introduced.
