# U17: Canonical Recovery Bot Identity

## 1. Summary

- **Unit**: U17 ("Canonical recovery bot identity")
- **Findings & Defect Diagnosis**:
  - In unpatched code (`src/storage/db.rs`), `fetch_resume_pending_session_keys` selected `session_key, platform, guild_id, channel_id, thread_id, user_id FROM sessions WHERE resume_pending = 1`, but ignored `row.session_key` (marking it `#[allow(dead_code)]`) and constructed `crate::SessionKey::new(row.platform, row.guild_id, row.channel_id, row.thread_id, row.user_id)` without `bot_id`.
  - In Discord multi-bot or application-ingress scenarios, two distinct sessions may share identical DM coordinates (`guild_id = None`, `channel_id = "dm-100"`, `user_id = "user-200"`), distinguished solely by `bot_id` (e.g. Bot 42 vs Bot 84). The canonical storage key encodes `bot_id` as the 6th length-prefixed component (`...|2:42` vs `...|2:84`).
  - When daemon recovery executed (`recover_resume_pending_sessions`), `session_key.storage_key()` computed the storage key of the botless/default key (`...|user-200` without `bot_id`). This stripped key failed to match either session in SQLite:
    1. `clear_session_resume_pending` returned `Ok(false)` (0 rows updated), skipping the session entirely.
    2. `find_last_unfinished_user_turn` searched `messages` using the botless key and returned `None`.
    3. The agent runner executed 0 runs, dropping both unfinished turns and losing bot identity.
  - In `recover_pending_delivery_obligations` (`src/main.rs`), undelivered outbox messages similarly reconstructed session keys by selecting columns without `bot_id`, causing recovered egress deliveries to route through default bot credentials instead of the original receiving bot identity.
  - `SessionKey` in `src/models/session.rs` lacked an inverse parser (`from_storage_key`) capable of reconstructing full `SessionKey` instances from canonical length-prefixed storage keys.
- **Architectural Resolution**:
  - **Strict Inverse Parser (`src/models/session.rs`)**:
    - Implemented `SessionKey::from_storage_key(&str) -> Result<SessionKey, SessionKeyParseError>`, along with `FromStr` and `TryFrom<&str>` implementations.
    - Strictly parses length-prefixed components (`{len}:{value}` or `-`), verifies character boundaries, requires separators (`|`), validates mandatory fields (`platform`, `channel_id`, `user_id`), allows optional fields (`guild_id`, `thread_id`), and parses the optional 6th component (`bot_id`).
    - Successfully round-trips legacy 5-component storage keys (reconstructing `bot_id: None`) and identity-aware 6-component storage keys (reconstructing `bot_id: Some(...)`), while rejecting malformed, truncated, or trailing-data inputs.
  - **Durable Load Recovery (`src/storage/db.rs`)**:
    - Patched `fetch_resume_pending_session_keys` to reconstruct session keys via `crate::SessionKey::from_storage_key(&row.session_key)`, falling back to `SessionKey::new(...)` only if legacy unparseable records exist.
    - Implemented `recover_resume_pending_sessions(pool, multiplexer)` in `src/storage/db.rs` to allow full recovery verification against real SQLite and `SessionMultiplexer`.
  - **Outbox Reconstruction (`src/main.rs`)**:
    - Patched `recover_pending_delivery_obligations` to parse `obligation.session_key` via `SessionKey::from_storage_key(&obligation.session_key)`, preserving `bot_id` across dead-process delivery recovery sweeps.
  - **Deterministic Mux & Binding Verification**:
    - Implemented `resume_pending_preserves_bot_identity` in `src/storage/db.rs` testing twin bots (Bot-A = 42 and Bot-B = 84) on identical DM coordinates:
      - Bot-A row has `resume_pending = 0` (not pending).
      - Bot-B row has `resume_pending = 1` (pending).
      - Under unpatched recovery, the returned key lacks B (`bot_id: None`).
      - Under patched recovery, exact B key (`SessionKey` with `bot_id: Some("84")`) is returned.
      - Real SQLite recovery sweep executes through `SessionMultiplexer`, dispatching only bot 84's pending turn with bot identity intact; bot-A is never dispatched.
      - Only bot-B's `resume_pending` flag transitions to 0; bot-A remains 0 (no blanket reset of other rows).
      - Outbound-ledger owner identity included: unrecovered obligation for bot-B from a dead PID recovers with owner session key parsed via `from_storage_key`, retaining bot identity `84`.

---

## 2. Captured RED Before Production Edits

### Constituent 1: Recovery Bot Identity (`storage::db::tests::resume_pending_preserves_bot_identity`)
- **Command**: `cargo test --lib storage::db::tests::resume_pending_preserves_bot_identity -- --exact --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-red.log`
- **Captured Output**:
  ```text
     Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
      Finished `test` profile [unoptimized + debuginfo] target(s) in 17.93s
       Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

  running 1 test

  thread 'storage::db::tests::resume_pending_preserves_bot_identity' (3301233) panicked at src/storage/db.rs:1669:9:
  assertion `left == right` failed: returned key must retain bot identity '84'
    left: None
   right: Some("84")
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test storage::db::tests::resume_pending_preserves_bot_identity ... FAILED

  failures:

  failures:
      storage::db::tests::resume_pending_preserves_bot_identity

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 361 filtered out; finished in 0.02s

  error: test failed, to rerun pass `--lib`
  ```

### Constituent 2: Outbound Ledger Identity (`legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity`)
- **Command**: `cargo test --bin omo-gateway test_recover_pending_delivery_obligations_preserves_bot_identity -- --nocapture`
- **Exit Code**: `101`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-red-outbox.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 2.15s
       Running unittests src/entry.rs (target/debug/deps/omo_gateway-890c8f6763a989f1)

  running 1 test

  thread 'legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity' (3313038) panicked at src/main.rs:1807:17:
  assertion `left == right` failed
    left: None
   right: Some("42")
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
  test legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity ... FAILED

  failures:

  failures:
      legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity

  test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 36 filtered out; finished in 0.05s

  error: test failed, to rerun pass `--bin omo-gateway`
  ```

---

## 3. Implementation Details

### 1. `SessionKey::from_storage_key` in `src/models/session.rs`
```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionKeyParseError {
    #[error("invalid storage key: input is empty")]
    EmptyInput,
    #[error("invalid storage key: malformed component at byte offset {0}: {1}")]
    MalformedComponent(usize, String),
    #[error("invalid storage key: expected '|' separator at byte offset {0}")]
    ExpectedSeparator(usize),
    #[error("invalid storage key: unexpected trailing data at byte offset {0}")]
    TrailingData(usize),
    #[error("invalid storage key: missing required field '{0}'")]
    MissingRequiredField(&'static str),
}

impl SessionKey {
    /// Strict inverse parser that reconstructs the full `SessionKey` from a canonical storage key,
    /// preserving the exact `bot_id` and all routing dimensions.
    pub fn from_storage_key(raw: &str) -> Result<Self, SessionKeyParseError> {
        if raw.is_empty() {
            return Err(SessionKeyParseError::EmptyInput);
        }

        let mut rem = raw;
        let mut offset = 0;

        let parse_component =
            |rem: &mut &str, offset: &mut usize| -> Result<Option<String>, SessionKeyParseError> {
                if rem.starts_with('-') {
                    *rem = &rem[1..];
                    *offset += 1;
                    return Ok(None);
                }

                let colon_idx = rem.find(':').ok_or_else(|| {
                    SessionKeyParseError::MalformedComponent(
                        *offset,
                        "missing ':' length separator".to_owned(),
                    )
                })?;

                let len_str = &rem[..colon_idx];
                let len: usize = len_str.parse().map_err(|_| {
                    SessionKeyParseError::MalformedComponent(
                        *offset,
                        format!("invalid length prefix '{len_str}'"),
                    )
                })?;

                let after_colon = &rem[colon_idx + 1..];
                let prefix_len = colon_idx + 1;

                if after_colon.len() < len {
                    return Err(SessionKeyParseError::MalformedComponent(
                        *offset,
                        format!("expected {len} bytes, found {}", after_colon.len()),
                    ));
                }

                if !after_colon.is_char_boundary(len) {
                    return Err(SessionKeyParseError::MalformedComponent(
                        *offset,
                        "byte length does not align with UTF-8 character boundary".to_owned(),
                    ));
                }

                let val = &after_colon[..len];
                *rem = &after_colon[len..];
                *offset += prefix_len + len;

                Ok(Some(val.to_owned()))
            };

        // Component 0: platform (required)
        let platform = parse_component(&mut rem, &mut offset)?
            .ok_or(SessionKeyParseError::MissingRequiredField("platform"))?;

        if !rem.starts_with('|') {
            return Err(SessionKeyParseError::ExpectedSeparator(offset));
        }
        rem = &rem[1..];
        offset += 1;

        // Component 1: guild_id (optional)
        let guild_id = parse_component(&mut rem, &mut offset)?;

        if !rem.starts_with('|') {
            return Err(SessionKeyParseError::ExpectedSeparator(offset));
        }
        rem = &rem[1..];
        offset += 1;

        // Component 2: channel_id (required)
        let channel_id = parse_component(&mut rem, &mut offset)?
            .ok_or(SessionKeyParseError::MissingRequiredField("channel_id"))?;

        if !rem.starts_with('|') {
            return Err(SessionKeyParseError::ExpectedSeparator(offset));
        }
        rem = &rem[1..];
        offset += 1;

        // Component 3: thread_id (optional)
        let thread_id = parse_component(&mut rem, &mut offset)?;

        if !rem.starts_with('|') {
            return Err(SessionKeyParseError::ExpectedSeparator(offset));
        }
        rem = &rem[1..];
        offset += 1;

        // Component 4: user_id (required for DM/guild-less, excluded for guild sessions)
        let user_id = parse_component(&mut rem, &mut offset)?;
        let user_id = match user_id {
            Some(uid) => {
                if guild_id.is_some() {
                    String::new()
                } else {
                    uid
                }
            }
            None if guild_id.is_some() => String::new(),
            None => return Err(SessionKeyParseError::MissingRequiredField("user_id")),
        };

        // Component 5: optional bot_id
        let bot_id = if rem.is_empty() {
            None
        } else if rem.starts_with('|') {
            rem = &rem[1..];
            offset += 1;
            let bot = parse_component(&mut rem, &mut offset)?;
            if !rem.is_empty() {
                return Err(SessionKeyParseError::TrailingData(offset));
            }
            bot
        } else {
            return Err(SessionKeyParseError::ExpectedSeparator(offset));
        };

        Ok(Self {
            platform,
            guild_id,
            channel_id,
            thread_id,
            user_id,
            bot_id,
        })
    }
}
```

### 2. `fetch_resume_pending_session_keys` in `src/storage/db.rs`
```rust
pub async fn fetch_resume_pending_session_keys(
    pool: &SqlitePool,
) -> Result<Vec<crate::SessionKey>> {
    let rows: Vec<ResumePendingSessionRow> = sqlx::query_as(
        "SELECT session_key, platform, guild_id, channel_id, thread_id, user_id FROM sessions WHERE resume_pending = 1 ORDER BY updated_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            crate::SessionKey::from_storage_key(&row.session_key).unwrap_or_else(|_| {
                crate::SessionKey::new(
                    row.platform,
                    row.guild_id,
                    row.channel_id,
                    row.thread_id,
                    row.user_id,
                )
            })
        })
        .collect())
}
```

### 3. Outbox Reconstruction in `src/main.rs`
```rust
    for obligation in recoverable {
        let session_key: SessionKey = if let Ok(key) =
            SessionKey::from_storage_key(&obligation.session_key)
        {
            key
        } else if let Ok(Some((stored_key, platform, guild_id, channel_id, thread_id, user_id))) =
            sqlx::query_as::<_, (String, String, Option<String>, String, Option<String>, String)>(
                "SELECT session_key, platform, guild_id, channel_id, thread_id, user_id FROM sessions WHERE session_key = ?",
            )
            .bind(&obligation.session_key)
            .fetch_optional(pool)
            .await
        {
            SessionKey::from_storage_key(&stored_key).unwrap_or_else(|_| {
                SessionKey::new(platform, guild_id, channel_id, thread_id, user_id)
            })
        } else {
            SessionKey::new(
                "discord",
                None::<String>,
                &obligation.channel_id,
                obligation.thread_id.as_deref(),
                "recovered-delivery",
            )
        };
```

---

## 4. Captured GREEN Verification

### Constituent 1: Recovery Bot Identity (`storage::db::tests::resume_pending_preserves_bot_identity`)
- **Command**: `cargo test --lib storage::db::tests::resume_pending_preserves_bot_identity -- --exact --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-green.log`
- **Captured Output**:
  ```text
     Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
      Finished `test` profile [unoptimized + debuginfo] target(s) in 1m 13s
       Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

  running 1 test
  test storage::db::tests::resume_pending_preserves_bot_identity ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 361 filtered out; finished in 0.02s
  ```

### Short-Form Filter Execution
- **Command**: `cargo test --lib resume_pending_preserves_bot_identity -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-green-short.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.63s
       Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

  running 1 test
  test storage::db::tests::resume_pending_preserves_bot_identity ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 361 filtered out; finished in 0.02s
  ```

### Constituent 2: Outbox Delivery Recovery Bot Identity
- **Command**: `cargo test --bin omo-gateway test_recover_pending_delivery_obligations_preserves_bot_identity -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-outbox-green.log`
- **Captured Output**:
  ```text
     Compiling omo-gateway v0.1.0 (/Users/indo/code/project/omon-gateway)
      Finished `test` profile [unoptimized + debuginfo] target(s) in 24.03s
       Running unittests src/entry.rs (target/debug/deps/omo_gateway-890c8f6763a989f1)

  running 1 test
  test legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 36 filtered out; finished in 0.02s
  ```

---

## 5. Suite & Adjacent Verifications

### 1. `models::session` Unit Suite
- **Command**: `cargo test --lib models::session -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-models-session.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.61s
       Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

  running 8 tests
  test models::session::tests::derives_stable_key_from_every_routing_dimension ... ok
  test models::session::tests::storage_key_parser_rejects_malformed_inputs ... ok
  test models::session::tests::dm_session_key_includes_user_and_guild_session_key_excludes_user ... ok
  test models::session::tests::bot_identity_partitions_discord_sessions_without_breaking_legacy_keys ... ok
  test models::session::tests::differentiates_absent_values_and_separator_like_content ... ok
  test models::session::tests::deserializes_pre_identity_session_keys ... ok
  test models::session::tests::storage_key_strict_inverse_round_trip ... ok
  test models::session::tests::supports_hash_based_session_lookup_and_serde_round_trip ... ok

  test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 354 filtered out; finished in 0.01s
  ```

### 2. `storage::db` Unit Suite
- **Command**: `cargo test --lib storage::db -- --nocapture`
- **Exit Code**: `0`
- **Log File**: `.omo/evidence/hermes-parity-20260905/U17-storage-db.log`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 0.17s
       Running unittests src/lib.rs (target/debug/deps/omon_gateway-e129d555675071bf)

  running 25 tests
  test storage::db::tests::migration_creates_discord_channel_cursors_table ... ok
  test storage::db::tests::migration_creates_cron_runs_owner_pid_column ... ok
  test storage::db::tests::migration_creates_discord_bot_cursors_table ... ok
  test storage::db::tests::migration_creates_pending_writes_table_and_indexes ... ok
  test storage::db::tests::applies_all_migrations_to_an_in_memory_database ... ok
  test storage::db::tests::migration_creates_cron_runs_indexes ... ok
  test storage::db::tests::migration_creates_pairing_tables_and_indexes ... ok
  test storage::db::tests::migration_creates_resume_pending_column_and_index ... ok
  test storage::db::tests::enforces_foreign_keys_after_migration ... ok
  test storage::db::tests::migrations_are_idempotent_for_repeated_connections ... ok
  test storage::db::tests::migration_creates_messages_platform_id_column_and_index ... ok
  test storage::db::tests::mark_session_suspended_fails_on_malformed_state_without_overwriting ... ok
  test storage::db::tests::durable_thread_ownership_persists_across_restart ... ok
  test storage::db::tests::file_database_serializes_concurrent_writers_without_lock_errors ... ok
  test storage::db::tests::inbound_preserves_original_platform_timestamp ... ok
  test storage::db::tests::message_sequence_preserves_causality_and_recent_history_window ... ok
  test storage::db::tests::scoped_approval_does_not_apply_replaced_payload ... ok
  test storage::db::tests::test_persist_session_binding_persists_durable_binding ... ok
  test storage::db::tests::test_has_platform_message_id_dedup_query ... ok
  test storage::db::tests::staged_memory_claim_is_atomic ... ok
  test storage::db::tests::test_find_last_unfinished_user_turn ... ok
  test storage::db::tests::test_pending_writes_store_round_trip ... ok
  test storage::db::tests::staged_memory_failure_retains_pending_without_fabricating_session ... ok
  test storage::db::tests::test_resume_pending_flag_lifecycle_and_queries ... ok
  test storage::db::tests::resume_pending_preserves_bot_identity ... ok

  test result: ok. 25 passed; 0 failed; 0 ignored; 0 measured; 337 filtered out; finished in 0.32s
  ```

### 3. Binary Entry Recovery Suite
- **Command**: `cargo test --bin omo-gateway test_recover_ -- --nocapture`
- **Exit Code**: `0`
- **Captured Output**:
  ```text
      Finished `test` profile [unoptimized + debuginfo] target(s) in 21.14s
       Running unittests src/entry.rs (target/debug/deps/omo_gateway-890c8f6763a989f1)

  running 4 tests
  test legacy::runner_tests::test_recover_pending_delivery_obligations_preserves_bot_identity ... ok
  test legacy::runner_tests::test_recover_pending_delivery_obligations_redispatches_dead_process_rows ... ok
  test legacy::runner_tests::test_recover_resume_pending_sessions_redispatches_unfinished_turn ... ok
  test legacy::runner_tests::test_recover_resume_pending_sessions_preserves_bot_identity ... ok

  test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 33 filtered out; finished in 0.05s
  ```

### 4. Actor Multiplexer Suite
- **Command**: `cargo test --test test_multiplexer -- --nocapture`
- **Exit Code**: `0`
- **Captured Output**:
  ```text
     Running tests/test_multiplexer.rs (target/debug/deps/test_multiplexer-35045673f4a5ed6a)

  running 15 tests
  test route_reports_actor_startup_failure_instead_of_acknowledging_a_lost_event ... ok
  test gc_does_not_evict_an_actor_with_an_active_turn ... ok
  test scale_to_zero_evicts_and_flushes_idle_sessions ... ok
  test stop_cancels_active_turn_and_clears_queued_events ... ok
  test delivery_ledger_deduplicates_concurrent_claims_and_records_latency ... ok
  test stop_immediately_cancels_the_active_turn ... ok
  test dropping_multiplexer_releases_actor_cycle_and_flushes_dirty_state ... ok
  test thread_inherits_parent_profile ... ok
  test guild_lane_is_shared_per_bot_not_author ... ok
  test events_arriving_during_running_turn_are_queued_and_processed_in_order ... ok
  test terminal_outcomes_release_typing ... ok
  test routes_multiple_sessions_in_parallel ... ok
  test flush_failure_replays_completed_turn_on_restart ... ok
  test handles_events_sequentially_within_one_session ... ok
  test transcript_level_inbound_dedup_skips_duplicate_platform_message_id ... ok

  test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.47s
  ```

### 5. Backend Suite
- **Command**: `cargo test --test test_omo_backend -- --nocapture`
- **Exit Code**: `0`
- **Captured Output**: 34 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 30.61s

---

## 6. Build, Formatting, Diagnostics & Cleanup

- **LSP Diagnostics**:
  - `lsp_diagnostics` query for `src/models/session.rs`, `src/storage/db.rs`, `src/main.rs` reported daemon unreachable (`LSP daemon did not become reachable at /Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`).
  - Compiler checks (`cargo check`, `cargo build`, `cargo test`) executed cleanly with zero errors or warnings on touched code.
- **Cargo Build**:
  - `cargo build`: **PASS** (exit code 0).
  - Log: `.omo/evidence/hermes-parity-20260905/U17-build.log`
- **Scoped Format Check**:
  - `rustfmt --edition 2021 --check src/models/session.rs src/storage/db.rs src/main.rs`: **PASS** (clean, exit code 0).
  - Log: `.omo/evidence/hermes-parity-20260905/U17-format.log`
- **Scoped Diff Check**:
  - `git --no-pager diff --check src/models/session.rs src/storage/db.rs src/main.rs`: **PASS** (clean, exit code 0).
  - Log: `.omo/evidence/hermes-parity-20260905/U17-diff.log`
- **Scope Compliance**:
  - Strictly modified ONLY `src/models/session.rs`, `src/storage/db.rs`, `src/main.rs`, and generated `U17-*` evidence files.
  - No database migration was required because SQLite schema `sessions.session_key` and `delivery_obligations.session_key` already store the length-prefixed canonical storage key, which preserves the optional 6th component `bot_id`.
  - Zero mock sleeps, polling loops, or unverified claims.
