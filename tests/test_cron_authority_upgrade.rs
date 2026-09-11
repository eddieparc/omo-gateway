use omon_gateway::cron::{HermesStore, HermesStoreSynchronizer};
use sqlx::sqlite::SqlitePoolOptions;
use std::fs;
use std::path::Path;

#[tokio::test]
async fn test_migration_0022_backfill_legacy_imported_rows() {
    let pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();

    // 1. Apply pre-upgrade schema: migrations 0001 through 0016
    for migration_file in &[
        "0001_initial.sql",
        "0002_delivery_tracking.sql",
        "0003_message_sequence.sql",
        "0004_cron_leases.sql",
        "0005_cron_runs_indexes.sql",
        "0006_delivery_obligations.sql",
        "0007_approval_allowlist.sql",
        "0008_resume_pending.sql",
        "0009_messages_platform_id.sql",
        "0010_pending_writes.sql",
        "0011_pairing_codes.sql",
        "0012_discord_channel_cursors.sql",
        "0013_cron_runs_owner_pid.sql",
        "0014_bot_profiles.sql",
        "0015_message_search_fts.sql",
        "0016_messenger_policy.sql",
    ] {
        let path = Path::new("migrations").join(migration_file);
        let sql = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        sqlx::raw_sql(&sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("failed to execute {}: {e}", path.display()));
    }

    // Verify pre-upgrade cron_jobs schema has NO authority column
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as("PRAGMA table_info(cron_jobs)")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(
        !columns.iter().any(|col| col.1 == "authority"),
        "old schema must not have authority column"
    );

    // Setup isolated temp Hermes store directory for live synchronizer validation
    let temp_dir = tempfile::tempdir().unwrap();
    let root = temp_dir.path();
    fs::create_dir_all(root.join("cron")).unwrap();
    let jobs_path = root.join("cron/jobs.json");
    let jobs_path_str = jobs_path.to_string_lossy().to_string();

    // 2. Insert pre-upgrade rows into old-schema cron_jobs:
    // a) Legacy imported row with source provenance pointing to our isolated jobs.json
    let legacy_imported_payload = serde_json::json!({
        "id": "daily",
        "prompt": "status",
        "_omon_hermes_source": &jobs_path_str,
        "_omon_hermes_profile": "default",
        "schedule": {
            "kind": "cron",
            "expr": "0 9 * * *"
        }
    })
    .to_string();

    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled) VALUES (?, '0 9 * * *', ?, 1)",
    )
    .bind("hermes:default:daily")
    .bind(&legacy_imported_payload)
    .execute(&pool)
    .await
    .unwrap();

    // b) Native row without source provenance
    let native_payload = r#"{"id":"native_status","prompt":"native system status"}"#;
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled) VALUES (?, '0 12 * * *', ?, 1)",
    )
    .bind("native_status")
    .bind(native_payload)
    .execute(&pool)
    .await
    .unwrap();

    // 3. Apply migration 0022 (actual schema execution)
    let migration_22_sql = fs::read_to_string("migrations/0022_cron_authority.sql").unwrap();
    sqlx::raw_sql(&migration_22_sql)
        .execute(&pool)
        .await
        .unwrap();

    // 4. Inspect post-upgrade authority of both rows
    let legacy_authority: String =
        sqlx::query_scalar("SELECT authority FROM cron_jobs WHERE id = 'hermes:default:daily'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let native_authority: String =
        sqlx::query_scalar("SELECT authority FROM cron_jobs WHERE id = 'native_status'")
            .fetch_one(&pool)
            .await
            .unwrap();

    println!(
        "Upgrade check: legacy_authority={legacy_authority}, native_authority={native_authority}"
    );

    // Native jobs must be omon_owned
    assert_eq!(
        native_authority, "omon_owned",
        "native jobs must be omon_owned after upgrade"
    );

    // Legacy imported rows must be hermes_mirror, NOT omon_owned!
    assert_eq!(
        legacy_authority, "hermes_mirror",
        "legacy imported rows with source provenance must be backfilled to hermes_mirror on upgrade"
    );

    // Source provenance must be preserved intact
    let post_upgrade_payload: String =
        sqlx::query_scalar("SELECT payload_json FROM cron_jobs WHERE id = 'hermes:default:daily'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let parsed_payload: serde_json::Value = serde_json::from_str(&post_upgrade_payload).unwrap();
    assert_eq!(
        parsed_payload["_omon_hermes_source"].as_str().unwrap(),
        jobs_path_str,
        "source provenance _omon_hermes_source must be preserved"
    );

    // 5. Verify live synchronizer interaction with the backfilled post-upgrade database:
    // Because legacy row is hermes_mirror (not falsely omon_owned), live sync continues to update it!
    fs::write(
        &jobs_path,
        serde_json::to_vec(&serde_json::json!({
            "jobs": [{
                "id": "daily",
                "prompt": "updated_status_via_hermes",
                "schedule": {
                    "kind": "cron",
                    "expr": "0 9 * * *"
                }
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let synchronizer =
        HermesStoreSynchronizer::new(pool.clone(), vec![HermesStore::new("default", root)]);

    let updated_count = synchronizer.sync().await.unwrap();
    assert_eq!(
        updated_count, 1,
        "synchronizer must update hermes_mirror row"
    );

    let updated_payload: String =
        sqlx::query_scalar("SELECT payload_json FROM cron_jobs WHERE id = 'hermes:default:daily'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        updated_payload.contains("updated_status_via_hermes"),
        "hermes_mirror row must receive live updates from source"
    );

    // Native row is omon_owned and must not be affected by live sync
    let native_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_jobs WHERE id = 'native_status'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(native_count, 1, "native row must remain untouched by sync");

    // If source deletes the job before cutover, hermes_mirror job is pruned
    fs::write(
        &jobs_path,
        serde_json::to_vec(&serde_json::json!({"jobs": []})).unwrap(),
    )
    .unwrap();

    let prune_sync = synchronizer.sync().await.unwrap();
    assert_eq!(prune_sync, 0);
    let legacy_count_after_prune: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_jobs WHERE id = 'hermes:default:daily'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        legacy_count_after_prune, 0,
        "un-cutover hermes_mirror job must be pruned when removed from live source"
    );
    let native_count_after_prune: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_jobs WHERE id = 'native_status'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        native_count_after_prune, 1,
        "native omon_owned job must never be deleted by source synchronizer"
    );

    // 6. Verify that real cutover ownership (omon_owned) is not blanket-reset
    // Re-insert cutover job as omon_owned
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, authority) VALUES (?, '0 9 * * *', ?, 1, 'omon_owned')",
    )
    .bind("hermes:default:daily")
    .bind(&legacy_imported_payload)
    .execute(&pool)
    .await
    .unwrap();

    // Re-sync with empty store: omon_owned cutover job survives!
    synchronizer.sync().await.unwrap();
    let cutover_survived_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cron_jobs WHERE id = 'hermes:default:daily'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        cutover_survived_count, 1,
        "cutover omon_owned job must survive empty store sync"
    );

    pool.close().await;
    temp_dir.close().unwrap();
}
