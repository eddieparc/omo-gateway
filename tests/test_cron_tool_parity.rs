use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use omon_gateway::{
    storage::init_pool, tools::Tool, CronJob, CronScheduler, CronTaskExecutor, CronTool, Result,
};
use serde_json::json;

#[derive(Clone, Default)]
struct CountingExecutor {
    runs: Arc<Mutex<usize>>,
}

#[async_trait]
impl CronTaskExecutor for CountingExecutor {
    async fn execute(&self, _job: &CronJob) -> Result<Option<String>> {
        *self.runs.lock().unwrap() += 1;
        Ok(Some("executed".into()))
    }
}

#[tokio::test]
async fn mutations_preserve_contract() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let executor = Arc::new(CountingExecutor::default());
    let scheduler = CronScheduler::new(pool.clone(), executor.clone());
    let tool = CronTool::with_scheduler(pool.clone(), Arc::new(scheduler.clone()));

    let add_res = tool
        .execute(json!({
            "action": "add",
            "id": "contract_job",
            "prompt": "run task",
            "expression": "0 * * * *",
            "enabled": false,
            "repeat": {"times": 2},
            "context_from": ["abc"]
        }))
        .await
        .expect("add must succeed");

    assert_eq!(
        add_res.get("status").and_then(|v| v.as_str()),
        Some("registered")
    );

    let job = scheduler
        .get("contract_job")
        .await
        .unwrap()
        .expect("job must exist");
    assert!(
        !job.enabled,
        "Job must be registered as disabled when enabled:false is specified"
    );

    let payload = job.payload().unwrap();
    assert_eq!(
        payload.get("repeat"),
        Some(&json!({"times": 2})),
        "repeat config must be preserved in payload"
    );
    assert_eq!(
        payload.get("context_from"),
        Some(&json!(["abc"])),
        "context_from must be preserved in payload"
    );

    let initial_runs = *executor.runs.lock().unwrap();
    let trig_res = tool
        .execute(json!({
            "action": "trigger",
            "id": "contract_job"
        }))
        .await
        .expect("trigger on paused job must succeed");

    assert_eq!(
        trig_res.get("status").and_then(|v| v.as_str()),
        Some("triggered")
    );
    for _ in 0..50 {
        if *executor.runs.lock().unwrap() > initial_runs {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        *executor.runs.lock().unwrap(),
        initial_runs + 1,
        "Trigger must execute exactly one manual run even when paused"
    );

    let job_after_trig = scheduler.get("contract_job").await.unwrap().unwrap();
    assert!(
        !job_after_trig.enabled,
        "Job must remain paused after manual trigger"
    );

    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, authority, created_at, updated_at)
         VALUES ('hermes:imported:job1', '0 * * * *', '{}', 1, 'hermes_mirror', datetime('now'), datetime('now'))",
    )
    .execute(&pool)
    .await
    .unwrap();

    let del_imported = tool
        .execute(json!({
            "action": "delete",
            "id": "hermes:imported:job1"
        }))
        .await;

    assert!(
        del_imported.is_err(),
        "Delete on imported read-only job must error, not succeed"
    );

    let original_next_run = chrono::Utc::now() + chrono::Duration::hours(2);
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, authority, created_at, updated_at)
         VALUES ('name_test_job', '0 * * * *', '{\"name\":\"Old Name\"}', 1, ?, 'omon_owned', datetime('now'), datetime('now'))",
    )
    .bind(original_next_run)
    .execute(&pool)
    .await
    .unwrap();

    let _update_res = tool
        .execute(json!({
            "action": "update",
            "id": "name_test_job",
            "name": "New Name"
        }))
        .await
        .expect("name update must succeed");

    let updated_job = scheduler.get("name_test_job").await.unwrap().unwrap();
    assert_eq!(
        updated_job.next_run_at,
        Some(original_next_run),
        "Name-only update must preserve next_run_at timestamp"
    );
}

#[tokio::test]
async fn runs_and_status_are_exposed() {
    let pool = init_pool("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let executor = Arc::new(CountingExecutor::default());
    let scheduler = CronScheduler::new(pool.clone(), executor.clone());
    let tool = CronTool::with_scheduler(pool.clone(), Arc::new(scheduler.clone()));

    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, authority, created_at, updated_at)
         VALUES ('j', '0 * * * *', '{}', 1, 'omon_owned', datetime('now'), datetime('now'))",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO cron_runs (run_id, job_id, claim_token, lease_expires_at, started_at, completed_at, status, attempt, error)
         VALUES ('r1', 'j', 'tok1', datetime('now'), datetime('now'), datetime('now'), 'succeeded', 1, NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();

    let runs_res = tool
        .execute(json!({
            "action": "runs",
            "id": "j"
        }))
        .await
        .expect("action 'runs' must succeed");

    let runs_arr = runs_res
        .get("runs")
        .and_then(|v| v.as_array())
        .expect("runs must be an array");
    assert_eq!(runs_arr.len(), 1, "Expected exactly 1 seeded attempt");
    assert_eq!(
        runs_arr[0].get("run_id").and_then(|v| v.as_str()),
        Some("r1")
    );
    assert_eq!(
        runs_arr[0].get("status").and_then(|v| v.as_str()),
        Some("succeeded")
    );

    let status_res = tool
        .execute(json!({
            "action": "status"
        }))
        .await
        .expect("action 'status' must succeed");

    assert!(
        status_res.get("running").is_some(),
        "status must report running state"
    );
}
