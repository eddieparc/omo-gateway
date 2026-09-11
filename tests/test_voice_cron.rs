use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use omon_gateway::{
    AudioFrame, AudioFrameBuffer, CronJob, CronJobSpec, CronScheduler, CronTaskExecutor, Database,
    OmonError,
};
use serde_json::json;
use tokio::sync::{mpsc, Notify};

struct RecordingExecutor(mpsc::UnboundedSender<String>);

#[async_trait]
impl CronTaskExecutor for RecordingExecutor {
    async fn execute(&self, job: &CronJob) -> Result<Option<String>, OmonError> {
        self.0.send(job.id.clone()).unwrap();
        Ok(Some("finished".into()))
    }
}

struct FailingExecutor;

#[async_trait]
impl CronTaskExecutor for FailingExecutor {
    async fn execute(&self, _job: &CronJob) -> Result<Option<String>, OmonError> {
        Err(OmonError::ToolExecution("intentional failure".into()))
    }
}

struct BlockingExecutor {
    started: mpsc::UnboundedSender<String>,
    release: Arc<Notify>,
}

#[async_trait]
impl CronTaskExecutor for BlockingExecutor {
    async fn execute(&self, job: &CronJob) -> Result<Option<String>, OmonError> {
        self.started.send(job.id.clone()).unwrap();
        self.release.notified().await;
        Ok(Some("finished".into()))
    }
}

#[tokio::test]
async fn cron_scheduler_registers_triggers_and_manages_jobs() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(RecordingExecutor(tx)));
    let job = scheduler
        .register(CronJobSpec::new(
            "interval:1s",
            json!({"channel_id": "42", "content": "run"}),
        ))
        .await
        .unwrap();
    let scheduled = job.next_run_at;

    assert!(job.enabled);
    assert_eq!(scheduler.list_active().await.unwrap().len(), 1);
    assert!(scheduler.trigger(&job.id).await.unwrap());
    assert_eq!(rx.recv().await.unwrap(), job.id);
    scheduler.shutdown().await;
    assert_eq!(
        scheduler.get(&job.id).await.unwrap().unwrap().next_run_at,
        scheduled
    );
    assert!(scheduler.pause(&job.id).await.unwrap());
    assert!(scheduler.list_active().await.unwrap().is_empty());
    assert!(scheduler.resume(&job.id).await.unwrap());
    assert!(scheduler.delete(&job.id).await.unwrap());
    assert!(scheduler.get(&job.id).await.unwrap().is_none());
}

#[tokio::test]
async fn background_scheduler_executes_due_interval_and_notifies_channel() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (tx, mut executions) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::with_poll_interval(
        database.pool().clone(),
        Arc::new(RecordingExecutor(tx)),
        Duration::from_secs(60),
    );
    let mut notifications = scheduler.subscribe();
    let job = scheduler
        .register_job(
            "interval:1ms",
            json!({"channel_id": 123456, "notification": "done"}),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(chrono::Utc::now() - chrono::TimeDelta::seconds(1))
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    scheduler.start().await;
    scheduler.run_due_jobs().await.unwrap();
    let executed = tokio::time::timeout(Duration::from_secs(2), executions.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(executed, job.id);
    let event = tokio::time::timeout(Duration::from_secs(2), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.channel_id, 123456);
    assert_eq!(event.content, "finished");
    scheduler.shutdown().await;
}

#[tokio::test]
async fn failed_one_shot_due_run_disables_job_and_records_failed_lease() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(FailingExecutor));
    let job = scheduler
        .register(CronJobSpec::new(
            "once:2999-01-01T00:00:00Z",
            json!({"content": "run"}),
        ))
        .await
        .unwrap();
    let due = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(due)
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    scheduler.shutdown().await;

    let updated = scheduler.get(&job.id).await.unwrap().unwrap();
    assert!(!updated.enabled);
    assert!(updated.next_run_at.is_none());
    let (status, error): (String, Option<String>) = sqlx::query_as(
        "SELECT status, error FROM cron_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(&job.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(status, "failed");
    assert!(error.is_some_and(|value| value.contains("intentional failure")));
}

#[tokio::test]
async fn failed_interval_advances_next_run_and_applies_backoff() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(FailingExecutor));
    let job = scheduler
        .register(CronJobSpec::new("interval:1m", json!({"content": "run"})))
        .await
        .unwrap();

    let before_run = chrono::Utc::now();
    let due = before_run - chrono::TimeDelta::seconds(1);
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(due)
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    scheduler.shutdown().await;

    let updated = scheduler.get(&job.id).await.unwrap().unwrap();
    assert!(updated.enabled);
    let next_run = updated.next_run_at.expect("next_run_at should be set");
    // Next run must be strictly in the future (at least 60s from execution time)
    assert!(next_run >= before_run + chrono::TimeDelta::seconds(55));

    // Calling run_due_jobs immediately claims nothing (no tight loop!)
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 0);
}

#[tokio::test]
async fn repeated_failures_scale_backoff_deterministically() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(FailingExecutor));
    let job = scheduler
        .register(CronJobSpec::new("interval:1s", json!({"content": "run"})))
        .await
        .unwrap();

    let wait_for_failure = |attempt: i64| {
        let pool = database.pool().clone();
        let job_id = job.id.clone();
        async move {
            for _ in 0..200 {
                let status: Option<String> = sqlx::query_scalar(
                    "SELECT status FROM cron_runs WHERE job_id = ? AND attempt = ? AND status = 'failed'",
                )
                .bind(&job_id)
                .bind(attempt)
                .fetch_optional(&pool)
                .await
                .unwrap();
                if status.is_some() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            panic!("timed out waiting for attempt {attempt} to record failure");
        }
    };

    // 1st failure: backoff of 10s (exceeds 1s interval)
    let t1 = chrono::Utc::now();
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(t1 - chrono::TimeDelta::seconds(1))
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    wait_for_failure(1).await;

    let job_after_1 = scheduler.get(&job.id).await.unwrap().unwrap();
    let next1 = job_after_1.next_run_at.unwrap();
    assert!(next1 >= t1 + chrono::TimeDelta::seconds(9));
    assert!(next1 <= t1 + chrono::TimeDelta::seconds(15));

    // 2nd failure: backoff of 20s
    let t2 = chrono::Utc::now();
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(t2 - chrono::TimeDelta::seconds(1))
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    wait_for_failure(2).await;

    let job_after_2 = scheduler.get(&job.id).await.unwrap().unwrap();
    let next2 = job_after_2.next_run_at.unwrap();
    assert!(next2 >= t2 + chrono::TimeDelta::seconds(19));
    assert!(next2 <= t2 + chrono::TimeDelta::seconds(25));

    // 3rd failure: backoff of 40s
    let t3 = chrono::Utc::now();
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(t3 - chrono::TimeDelta::seconds(1))
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    wait_for_failure(3).await;

    let job_after_3 = scheduler.get(&job.id).await.unwrap().unwrap();
    let next3 = job_after_3.next_run_at.unwrap();
    assert!(next3 >= t3 + chrono::TimeDelta::seconds(39));
    assert!(next3 <= t3 + chrono::TimeDelta::seconds(45));

    // Immediate run_due_jobs claims 0 because next_run_at is far in the future
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 0);

    scheduler.shutdown().await;
}

#[tokio::test]
async fn active_lease_blocks_duplicate_claims_until_success_commits_one_shot() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let release = Arc::new(Notify::new());
    let scheduler = CronScheduler::new(
        database.pool().clone(),
        Arc::new(BlockingExecutor {
            started: started_tx,
            release: release.clone(),
        }),
    );
    let job = scheduler
        .register(CronJobSpec::new(
            "once:2999-01-01T00:00:00Z",
            json!({"content": "run"}),
        ))
        .await
        .unwrap();
    let due = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(due)
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    assert_eq!(started_rx.recv().await.unwrap(), job.id);
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 0);
    assert!(!scheduler.trigger(&job.id).await.unwrap());

    let during = scheduler.get(&job.id).await.unwrap().unwrap();
    assert!(during.enabled);
    assert!(during.next_run_at.is_some());
    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cron_runs WHERE job_id = ? AND status = 'running'",
    )
    .bind(&job.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(running, 1);

    release.notify_waiters();
    scheduler.shutdown().await;

    let completed = scheduler.get(&job.id).await.unwrap().unwrap();
    assert!(!completed.enabled);
    assert!(completed.next_run_at.is_none());
    let (status, completed_at): (String, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT status, completed_at FROM cron_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(&job.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(status, "succeeded");
    assert!(completed_at.is_some());
}

#[tokio::test]
async fn live_process_expired_lease_is_not_reclaimed_and_completes_successfully() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let release = Arc::new(Notify::new());
    let scheduler = CronScheduler::new(
        database.pool().clone(),
        Arc::new(BlockingExecutor {
            started: started_tx,
            release: release.clone(),
        }),
    );
    let job = scheduler
        .register(CronJobSpec::new(
            "once:2999-01-01T00:00:00Z",
            json!({"content": "run"}),
        ))
        .await
        .unwrap();
    let due = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(due)
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    assert_eq!(started_rx.recv().await.unwrap(), job.id);

    // Verify owner_pid is set to current process
    let (owner_pid, status): (Option<i64>, String) = sqlx::query_as(
        "SELECT owner_pid, status FROM cron_runs WHERE job_id = ? AND status = 'running'",
    )
    .bind(&job.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(owner_pid, Some(std::process::id() as i64));
    assert_eq!(status, "running");

    // Manually expire the lease
    let expired_at = chrono::Utc::now() - chrono::TimeDelta::seconds(10);
    sqlx::query(
        "UPDATE cron_runs SET lease_expires_at = ? WHERE job_id = ? AND status = 'running'",
    )
    .bind(expired_at)
    .bind(&job.id)
    .execute(database.pool())
    .await
    .unwrap();

    // Another scheduler sweep should NOT reclaim since the owner process is alive
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 0);

    let status_still_running: String =
        sqlx::query_scalar("SELECT status FROM cron_runs WHERE job_id = ? AND status = 'running'")
            .bind(&job.id)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(status_still_running, "running");

    // Release the executor to allow it to finish and complete_success
    release.notify_waiters();
    scheduler.shutdown().await;

    let final_status: String = sqlx::query_scalar(
        "SELECT status FROM cron_runs WHERE job_id = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(&job.id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(final_status, "succeeded");
}

#[tokio::test]
async fn dead_process_expired_lease_is_reclaimed_by_scheduler() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (tx, mut executions) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(RecordingExecutor(tx)));
    let job = scheduler
        .register(CronJobSpec::new("interval:1m", json!({"content": "run"})))
        .await
        .unwrap();

    // Simulate an expired running run owned by a dead PID
    let dead_pid = 4_194_304i64;
    let old_run_id = uuid::Uuid::new_v4().to_string();
    let old_token = uuid::Uuid::new_v4().to_string();
    let expired_time = chrono::Utc::now() - chrono::TimeDelta::minutes(5);
    sqlx::query(
        "INSERT INTO cron_runs
         (run_id, job_id, claim_token, lease_expires_at, started_at, completed_at, status, attempt, error, owner_pid)
         VALUES (?, ?, ?, ?, ?, NULL, 'running', 1, NULL, ?)",
    )
    .bind(&old_run_id)
    .bind(&job.id)
    .bind(&old_token)
    .bind(expired_time)
    .bind(expired_time)
    .bind(dead_pid)
    .execute(database.pool())
    .await
    .unwrap();

    // Set job next_run_at to past
    sqlx::query("UPDATE cron_jobs SET next_run_at = ? WHERE id = ?")
        .bind(expired_time)
        .bind(&job.id)
        .execute(database.pool())
        .await
        .unwrap();

    // Reclaim should happen and new run should execute
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    assert_eq!(executions.recv().await.unwrap(), job.id);
    scheduler.shutdown().await;

    // Check old run was marked failed with expired error
    let (old_status, old_error): (String, Option<String>) =
        sqlx::query_as("SELECT status, error FROM cron_runs WHERE run_id = ?")
            .bind(&old_run_id)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(old_status, "failed");
    assert_eq!(
        old_error.as_deref(),
        Some("lease expired before completion")
    );
}

#[test]
fn audio_frame_buffer_serializes_without_losing_pcm_samples() {
    let first = AudioFrame::pcm(99, Some(7), 1, vec![-32_768, -1, 0, 1, 32_767]);
    let second = AudioFrame::pcm(99, Some(8), 2, vec![10, 20]);
    let mut buffer = AudioFrameBuffer::new(2);
    assert!(buffer.push(first.clone()).is_none());
    assert!(buffer.push(second.clone()).is_none());

    let encoded = buffer.serialized().unwrap();
    let mut decoded = AudioFrameBuffer::deserialize(&encoded, 2).unwrap();
    assert_eq!(decoded.pop(), Some(first));
    assert_eq!(decoded.pop(), Some(second));
    assert!(decoded.is_empty());
}

#[tokio::test]
async fn persisted_oneshot_past_grace_is_retired() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let t0 = chrono::DateTime::parse_from_rfc3339("2026-09-06T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let clock_time = Arc::new(std::sync::Mutex::new(t0));
    let scheduler = CronScheduler::new(database.pool().clone(), Arc::new(RecordingExecutor(tx)))
        .with_clock({
            let c = clock_time.clone();
            move || *c.lock().unwrap()
        });

    // --- PHASE 1: DIRECT DB SEEDING AND DUE SCAN AT T ---
    // Seed real DB rows directly:
    // 1. One-shot past grace: next_run = T - 121s (more than 120s late)
    let overdue_id = "test-overdue-oneshot";
    let overdue_next = t0 - chrono::TimeDelta::seconds(121);
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
         VALUES (?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(overdue_id)
    .bind(format!("once:{}", overdue_next.to_rfc3339()))
    .bind(json!({"content": "overdue once"}).to_string())
    .bind(overdue_next)
    .bind(overdue_next)
    .bind(overdue_next)
    .execute(database.pool())
    .await
    .unwrap();

    // 2. Boundary one-shot: next_run = T - 120s (exactly at 120s grace boundary)
    let boundary_id = "test-boundary-oneshot";
    let boundary_next = t0 - chrono::TimeDelta::seconds(120);
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
         VALUES (?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(boundary_id)
    .bind(format!("once:{}", boundary_next.to_rfc3339()))
    .bind(json!({"content": "boundary once"}).to_string())
    .bind(boundary_next)
    .bind(boundary_next)
    .bind(boundary_next)
    .execute(database.pool())
    .await
    .unwrap();

    // 3. Overdue recurring job: next_run = T - 300s (recurring jobs remain eligible)
    let recurring_id = "test-overdue-recurring";
    let recurring_next = t0 - chrono::TimeDelta::seconds(300);
    sqlx::query(
        "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
         VALUES (?, ?, ?, 1, ?, ?, ?)",
    )
    .bind(recurring_id)
    .bind("interval:60s")
    .bind(json!({"content": "recurring"}).to_string())
    .bind(recurring_next)
    .bind(recurring_next)
    .bind(recurring_next)
    .execute(database.pool())
    .await
    .unwrap();

    // Run due scan at T:
    // RED: claims all 3 jobs (claim count = 3); overdue one-shot executes
    // GREEN: claims 2 jobs (boundary + recurring); overdue one-shot claim count = 0
    let claimed = scheduler.run_due_jobs().await.unwrap();
    assert_eq!(
        claimed, 2,
        "overdue one-shot past grace (T-121s) must not be claimed; boundary (T-120s) and recurring must be claimed"
    );

    // Collect executions for the claimed jobs:
    let mut executed_ids = Vec::new();
    for _ in 0..claimed {
        let id = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("must receive claimed execution within timeout")
            .expect("channel should not close");
        executed_ids.push(id);
    }
    assert!(
        !executed_ids.contains(&overdue_id.to_string()),
        "overdue one-shot past grace must not execute"
    );
    assert!(
        executed_ids.contains(&boundary_id.to_string()),
        "boundary one-shot (T-120s) must execute once"
    );
    assert!(
        executed_ids.contains(&recurring_id.to_string()),
        "overdue recurring job must execute"
    );

    // Overdue one-shot must be retired: disabled, no next run, and retained missed record
    let overdue_job = scheduler.get(overdue_id).await.unwrap().unwrap();
    assert!(!overdue_job.enabled, "overdue once job must be disabled");
    assert!(
        overdue_job.next_run_at.is_none(),
        "overdue once job must have next_run_at = None"
    );
    assert_eq!(
        overdue_job.last_status().as_deref(),
        Some("missed"),
        "overdue once job must have last_status = 'missed'"
    );
    assert!(
        overdue_job
            .last_error()
            .unwrap_or_default()
            .contains("missed"),
        "overdue once job must retain missed error diagnostic"
    );

    // Verify manual control: manual trigger still executes the retired job
    assert!(scheduler.trigger(overdue_id).await.unwrap());
    let triggered_id = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("manual trigger must execute")
        .expect("channel open");
    assert_eq!(triggered_id, overdue_id);

    // Verify explicit rearm: resume with future schedule re-enables the job
    let future_t = t0 + chrono::TimeDelta::minutes(10);
    *clock_time.lock().unwrap() = future_t;
    // Updating next_run to future and resuming:
    sqlx::query("UPDATE cron_jobs SET expression = ?, next_run_at = ? WHERE id = ?")
        .bind(format!("once:{}", future_t.to_rfc3339()))
        .bind(future_t)
        .bind(overdue_id)
        .execute(database.pool())
        .await
        .unwrap();
    assert!(scheduler.resume(overdue_id).await.unwrap());
    let rearmed_job = scheduler.get(overdue_id).await.unwrap().unwrap();
    assert!(rearmed_job.enabled, "rearmed job must be enabled");
    assert!(
        rearmed_job.next_run_at.is_some(),
        "rearmed job must have next_run_at"
    );

    // --- PHASE 2: LOCAL SURFACE (HERMES STORE IMPORT & DISPATCH PASS) ---
    let root = tempfile::tempdir().unwrap();
    let profile_home = root.path().join("surface");
    tokio::fs::create_dir_all(profile_home.join("cron"))
        .await
        .unwrap();

    let old_imported_time = "2026-09-06T11:57:00Z"; // T - 180s (past grace)
    let fresh_imported_time = "2026-09-06T12:10:00Z";
    let jobs_json = json!({
        "jobs": [
            {
                "id": "old-once",
                "prompt": "stale one-shot reminder",
                "enabled": true,
                "next_run_at": old_imported_time,
                "schedule": {"kind": "once", "run_at": old_imported_time},
                "deliver": "local"
            },
            {
                "id": "fresh-once",
                "prompt": "fresh one-shot",
                "enabled": true,
                "next_run_at": fresh_imported_time,
                "schedule": {"kind": "once", "run_at": fresh_imported_time},
                "deliver": "local"
            }
        ]
    });
    tokio::fs::write(
        profile_home.join("cron/jobs.json"),
        serde_json::to_vec(&jobs_json).unwrap(),
    )
    .await
    .unwrap();

    let sync = omon_gateway::HermesStoreSynchronizer::new(
        database.pool().clone(),
        vec![omon_gateway::HermesStore::new("surface", &profile_home)],
    );
    let imported_count = sync.sync_at(t0).await.unwrap();
    assert_eq!(imported_count, 2, "must import both jobs from temp store");

    // Reset clock back to t0
    *clock_time.lock().unwrap() = t0;

    // Run actual scheduler dispatch pass
    let surface_claimed = scheduler.run_due_jobs().await.unwrap();
    assert_eq!(
        surface_claimed, 0,
        "old-once is past grace and fresh-once is in the future; 0 claims"
    );

    // Capturing executor must receive zero calls for old-once
    assert!(
        rx.try_recv().is_err(),
        "capturing executor must receive zero calls for expired once job"
    );

    // Dashboard-visible job/run state must distinguish missed from succeeded:
    let old_job = scheduler
        .get("hermes:surface:old-once")
        .await
        .unwrap()
        .expect("job exists");
    assert!(!old_job.enabled, "expired imported once must be disabled");
    assert!(
        old_job.next_run_at.is_none(),
        "expired imported once must have NULL next_run_at"
    );
    assert_eq!(
        old_job.last_status().as_deref(),
        Some("missed"),
        "dashboard-visible status must be 'missed'"
    );

    // Trigger fresh-once manually to verify it succeeds and distinguishes from missed
    assert!(scheduler
        .trigger("hermes:surface:fresh-once")
        .await
        .unwrap());
    let fresh_executed = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh_executed, "hermes:surface:fresh-once");

    // Allow completion to persist
    tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
        .await
        .expect("scheduler shutdown timeout");

    let fresh_job = scheduler
        .get("hermes:surface:fresh-once")
        .await
        .unwrap()
        .expect("fresh job exists");
    assert_eq!(
        fresh_job.last_status().as_deref(),
        Some("succeeded"),
        "dashboard-visible status for succeeded job must be 'succeeded'"
    );

    tokio::time::timeout(Duration::from_secs(5), database.close())
        .await
        .expect("database close timeout");
    root.close().unwrap();
}

struct NotifyOnDrop(Arc<Notify>);
impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_waiters();
    }
}

struct SpawnGuard<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> SpawnGuard<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    async fn join(&mut self, timeout: Duration) -> Result<T, String> {
        let handle = self
            .handle
            .as_mut()
            .ok_or_else(|| "task already joined".to_string())?;
        match tokio::time::timeout(timeout, handle).await {
            Ok(Ok(val)) => {
                self.handle.take();
                Ok(val)
            }
            Ok(Err(join_err)) => {
                self.handle.take();
                Err(format!("task panicked: {join_err}"))
            }
            Err(_) => {
                if let Some(h) = self.handle.take() {
                    h.abort();
                    let _ = h.await;
                }
                Err(format!("task timed out after {timeout:?}"))
            }
        }
    }
}

impl<T> Drop for SpawnGuard<T> {
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

#[tokio::test]
async fn overdue_oneshot_retirement_is_atomic_against_concurrent_claim_and_rearm() {
    let t0 = chrono::DateTime::parse_from_rfc3339("2026-09-06T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let overdue_next = t0 - chrono::TimeDelta::seconds(125);

    // --- CASE 1: COMPETING MANUAL CLAIM WINS OVER RETIREMENT ---
    // Proves CAS condition: NOT EXISTS (SELECT 1 FROM cron_runs WHERE job_id = ? AND status = 'running')
    {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        let job_id = "job-competing-manual";
        let payload =
            json!({"content": "competing manual", "preserved_metadata": "keep-me"}).to_string();
        sqlx::query(
            "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(job_id)
        .bind(format!("once:{}", overdue_next.to_rfc3339()))
        .bind(&payload)
        .bind(overdue_next)
        .bind(overdue_next)
        .bind(overdue_next)
        .execute(database.pool())
        .await
        .unwrap();

        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let release_manual = Arc::new(Notify::new());
        let _manual_guard = NotifyOnDrop(release_manual.clone());
        let entered = Arc::new(Notify::new());
        let release_gate = Arc::new(Notify::new());
        let _gate_guard = NotifyOnDrop(release_gate.clone());

        let scheduler = CronScheduler::new(
            database.pool().clone(),
            Arc::new(BlockingExecutor {
                started: started_tx,
                release: release_manual.clone(),
            }),
        )
        .with_clock(move || t0)
        .with_pre_retire_gate(entered.clone(), release_gate.clone());

        // Subscribe to pre-retire gate before triggering background due scan:
        let entered_wait = entered.notified();
        tokio::pin!(entered_wait);

        let s = scheduler.clone();
        let mut run_task = SpawnGuard::new(tokio::spawn(async move { s.run_due_jobs().await }));

        // Wait until scheduler enters pre-retire gate (paused before retirement CAS update):
        tokio::time::timeout(Duration::from_secs(5), &mut entered_wait)
            .await
            .expect("scheduler must reach pre-retire gate within timeout");

        // Competing manual trigger executes while retirement is paused at gate:
        let triggered = scheduler.trigger(job_id).await.unwrap();
        assert!(triggered, "competing manual trigger must succeed");
        let executed_id = tokio::time::timeout(Duration::from_secs(5), started_rx.recv())
            .await
            .expect("manual executor must start within timeout")
            .expect("channel open");
        assert_eq!(executed_id, job_id);

        // Verify lease is actively running in SQLite:
        let running_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cron_runs WHERE job_id = ? AND status = 'running'",
        )
        .bind(job_id)
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(
            running_count, 1,
            "manual claim must be actively running in SQLite"
        );

        // Release the pre-retire gate to allow retirement update to attempt CAS:
        release_gate.notify_one();
        let due_claims = run_task
            .join(Duration::from_secs(5))
            .await
            .expect("due task join")
            .unwrap();
        assert_eq!(
            due_claims, 0,
            "due scan must claim 0 after manual trigger took ownership"
        );

        // Verify retirement CAS failed and job is still enabled while manual run is active:
        let job_mid_run = scheduler.get(job_id).await.unwrap().unwrap();
        assert!(
            job_mid_run.enabled,
            "job must remain enabled while manual run is active"
        );
        assert_ne!(
            job_mid_run.last_status().as_deref(),
            Some("missed"),
            "job must not be marked missed while manual run is active"
        );

        // Release manual executor and wait for scheduler shutdown:
        release_manual.notify_one();
        tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
            .await
            .expect("scheduler shutdown timeout");

        let job_after = scheduler.get(job_id).await.unwrap().unwrap();
        assert_eq!(
            job_after.last_status().as_deref(),
            Some("succeeded"),
            "job was executed manually, must have status 'succeeded'"
        );

        tokio::time::timeout(Duration::from_secs(5), database.close())
            .await
            .expect("database close timeout");
    }

    // --- CASE 2: COMPETING FULL REARM WINS OVER RETIREMENT ---
    // Proves CAS condition: expression = ? + next_run_at = ? + payload_json = ? (full schedule change)
    {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let job_id = "job-competing-rearm";
        let payload = json!({"content": "competing rearm", "original_key": "v1"}).to_string();
        sqlx::query(
            "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(job_id)
        .bind(format!("once:{}", overdue_next.to_rfc3339()))
        .bind(&payload)
        .bind(overdue_next)
        .bind(overdue_next)
        .bind(overdue_next)
        .execute(database.pool())
        .await
        .unwrap();

        let entered = Arc::new(Notify::new());
        let release_gate = Arc::new(Notify::new());
        let _gate_guard = NotifyOnDrop(release_gate.clone());

        let scheduler = CronScheduler::new(
            database.pool().clone(),
            Arc::new(RecordingExecutor(tx.clone())),
        )
        .with_clock(move || t0)
        .with_pre_retire_gate(entered.clone(), release_gate.clone());

        // Subscribe before triggering background due scan:
        let entered_wait = entered.notified();
        tokio::pin!(entered_wait);

        let s = scheduler.clone();
        let mut run_task = SpawnGuard::new(tokio::spawn(async move { s.run_due_jobs().await }));

        // Wait until scheduler enters pre-retire gate:
        tokio::time::timeout(Duration::from_secs(5), &mut entered_wait)
            .await
            .expect("scheduler must reach pre-retire gate within timeout");

        // Competing rearm by operator while paused:
        let rearmed_time = t0 + chrono::TimeDelta::hours(1);
        let rearmed_payload =
            json!({"content": "competing rearm", "rearmed_key": "v2_updated"}).to_string();
        sqlx::query(
            "UPDATE cron_jobs SET expression = ?, next_run_at = ?, payload_json = ?, updated_at = ? WHERE id = ?",
        )
        .bind(format!("once:{}", rearmed_time.to_rfc3339()))
        .bind(rearmed_time)
        .bind(&rearmed_payload)
        .bind(t0)
        .bind(job_id)
        .execute(database.pool())
        .await
        .unwrap();

        // Release the pre-retire gate:
        release_gate.notify_one();
        let due_claims = run_task
            .join(Duration::from_secs(5))
            .await
            .expect("due task join")
            .unwrap();
        assert_eq!(due_claims, 0);

        tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
            .await
            .expect("scheduler shutdown timeout");

        // Verify rearmed job was NOT clobbered by the stale retirement:
        let job_after = scheduler.get(job_id).await.unwrap().unwrap();
        assert!(job_after.enabled, "rearmed job must remain enabled");
        assert_eq!(
            job_after.next_run_at,
            Some(rearmed_time),
            "rearmed next_run_at must not be cleared to NULL by stale retirement"
        );
        assert_eq!(
            job_after.payload_json, rearmed_payload,
            "rearmed payload must not be overwritten with old metadata by stale retirement"
        );
        assert_ne!(
            job_after.last_status().as_deref(),
            Some("missed"),
            "rearmed job must not have last_status 'missed'"
        );

        tokio::time::timeout(Duration::from_secs(5), database.close())
            .await
            .expect("database close timeout");
    }

    // --- CASE 3: COMPETING SAME-EXPRESSION NEXT_RUN_AT-ONLY REARM WINS OVER RETIREMENT ---
    // Proves CAS condition: next_run_at = ? predicate rejects retirement when expression & payload are unchanged
    {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let job_id = "job-competing-reschedule-time-only";
        let original_expression = format!("once:{}", overdue_next.to_rfc3339());
        let original_payload =
            json!({"content": "reschedule time only", "key": "retain-this"}).to_string();
        sqlx::query(
            "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(job_id)
        .bind(&original_expression)
        .bind(&original_payload)
        .bind(overdue_next)
        .bind(overdue_next)
        .bind(overdue_next)
        .execute(database.pool())
        .await
        .unwrap();

        let entered = Arc::new(Notify::new());
        let release_gate = Arc::new(Notify::new());
        let _gate_guard = NotifyOnDrop(release_gate.clone());

        let scheduler = CronScheduler::new(
            database.pool().clone(),
            Arc::new(RecordingExecutor(tx.clone())),
        )
        .with_clock(move || t0)
        .with_pre_retire_gate(entered.clone(), release_gate.clone());

        // Subscribe before triggering background due scan:
        let entered_wait = entered.notified();
        tokio::pin!(entered_wait);

        let s = scheduler.clone();
        let mut run_task = SpawnGuard::new(tokio::spawn(async move { s.run_due_jobs().await }));

        // Wait until scheduler enters pre-retire gate:
        tokio::time::timeout(Duration::from_secs(5), &mut entered_wait)
            .await
            .expect("scheduler must reach pre-retire gate within timeout");

        // Operator reschedules to future time: ONLY next_run_at and updated_at change!
        // expression and payload_json remain IDENTICAL to the observed row.
        let rescheduled_time = t0 + chrono::TimeDelta::hours(2);
        sqlx::query("UPDATE cron_jobs SET next_run_at = ?, updated_at = ? WHERE id = ?")
            .bind(rescheduled_time)
            .bind(t0)
            .bind(job_id)
            .execute(database.pool())
            .await
            .unwrap();

        // Release the pre-retire gate:
        release_gate.notify_one();
        let due_claims = run_task
            .join(Duration::from_secs(5))
            .await
            .expect("due task join")
            .unwrap();
        assert_eq!(due_claims, 0);

        tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
            .await
            .expect("scheduler shutdown timeout");

        // Verify: CAS rejected retirement because next_run_at changed.
        // If CAS only checked expression = ?, it would have wrongly succeeded and set next_run_at to NULL!
        let job_after = scheduler.get(job_id).await.unwrap().unwrap();
        assert!(
            job_after.enabled,
            "job with rescheduled next_run_at must remain enabled"
        );
        assert_eq!(
            job_after.next_run_at,
            Some(rescheduled_time),
            "rescheduled next_run_at must be preserved (not cleared to NULL by stale retirement)"
        );
        assert_eq!(
            job_after.expression, original_expression,
            "expression must remain unchanged"
        );
        assert_eq!(
            job_after.payload_json, original_payload,
            "payload must remain unchanged"
        );
        assert_ne!(
            job_after.last_status().as_deref(),
            Some("missed"),
            "job must not have last_status 'missed'"
        );

        tokio::time::timeout(Duration::from_secs(5), database.close())
            .await
            .expect("database close timeout");
    }

    // --- CASE 4: COMPETING SAME-EXPRESSION PAYLOAD-ONLY UPDATE WINS OVER RETIREMENT ---
    // Proves CAS condition: payload_json = ? predicate rejects retirement when payload changed, preventing metadata clobber
    {
        let database = Database::connect("sqlite::memory:").await.unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let job_id = "job-competing-payload-only";
        let original_expression = format!("once:{}", overdue_next.to_rfc3339());
        let initial_payload =
            json!({"content": "payload only", "version": 1, "metadata": "initial"}).to_string();
        sqlx::query(
            "INSERT INTO cron_jobs (id, expression, payload_json, enabled, next_run_at, created_at, updated_at)
             VALUES (?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(job_id)
        .bind(&original_expression)
        .bind(&initial_payload)
        .bind(overdue_next)
        .bind(overdue_next)
        .bind(overdue_next)
        .execute(database.pool())
        .await
        .unwrap();

        let entered = Arc::new(Notify::new());
        let release_gate = Arc::new(Notify::new());
        let _gate_guard = NotifyOnDrop(release_gate.clone());

        let scheduler = CronScheduler::new(
            database.pool().clone(),
            Arc::new(RecordingExecutor(tx.clone())),
        )
        .with_clock(move || t0)
        .with_pre_retire_gate(entered.clone(), release_gate.clone());

        // Subscribe before triggering background due scan:
        let entered_wait = entered.notified();
        tokio::pin!(entered_wait);

        let s = scheduler.clone();
        let mut run_task = SpawnGuard::new(tokio::spawn(async move { s.run_due_jobs().await }));

        // Wait until scheduler enters pre-retire gate:
        tokio::time::timeout(Duration::from_secs(5), &mut entered_wait)
            .await
            .expect("scheduler must reach pre-retire gate within timeout");

        // Concurrent update modifies payload: ONLY payload_json and updated_at change!
        // expression and next_run_at remain IDENTICAL to the observed row.
        let updated_payload =
            json!({"content": "payload only", "version": 2, "metadata": "concurrent-operator-edit"})
                .to_string();
        sqlx::query("UPDATE cron_jobs SET payload_json = ?, updated_at = ? WHERE id = ?")
            .bind(&updated_payload)
            .bind(t0)
            .bind(job_id)
            .execute(database.pool())
            .await
            .unwrap();

        // Release the pre-retire gate:
        release_gate.notify_one();
        let due_claims = run_task
            .join(Duration::from_secs(5))
            .await
            .expect("due task join")
            .unwrap();
        assert_eq!(due_claims, 0);

        tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
            .await
            .expect("scheduler shutdown timeout");

        // Verify: CAS rejected retirement because payload changed.
        // If CAS only checked expression = ? or next_run_at = ?, it would have wrongly overwritten payload_json!
        let job_after = scheduler.get(job_id).await.unwrap().unwrap();
        assert!(
            job_after.enabled,
            "job with concurrently updated payload must remain enabled"
        );
        assert_eq!(
            job_after.next_run_at,
            Some(overdue_next),
            "next_run_at must remain unchanged"
        );
        assert_eq!(
            job_after.payload_json, updated_payload,
            "concurrently updated payload must NOT be overwritten by stale retirement payload"
        );
        assert_ne!(
            job_after.last_status().as_deref(),
            Some("missed"),
            "job must not have last_status 'missed'"
        );

        tokio::time::timeout(Duration::from_secs(5), database.close())
            .await
            .expect("database close timeout");
    }
}

#[derive(Clone, Default)]
struct FailureRoutingDispatcher {
    messages: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

#[async_trait]
impl omon_gateway::OutboundDispatcher for FailureRoutingDispatcher {
    async fn dispatch(&self, action: omon_gateway::OutboundAction) -> Result<(), OmonError> {
        if let omon_gateway::OutboundAction::SendMessage {
            ref session,
            ref content,
            ..
        } = action
        {
            self.messages
                .lock()
                .unwrap()
                .push((session.channel_id.clone(), content.clone()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn failure_delivery_uses_its_own_lane() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let dispatcher = Arc::new(FailureRoutingDispatcher::default());
    let executor = Arc::new(FailingExecutor);

    let scheduler =
        CronScheduler::with_dispatcher(database.pool().clone(), executor, dispatcher.clone());

    let job1_json = json!({
        "id": "suppressed_fail_job",
        "name": "Suppressed Fail",
        "prompt": "run",
        "deliver": "discord:42",
        "failure_deliver": "local",
        "schedule": {"kind": "cron", "expr": "0 * * * *"}
    });

    let cron_job1 = CronJob {
        id: "suppressed_fail_job".into(),
        session_key: None,
        expression: "0 * * * *".into(),
        payload_json: serde_json::to_string(&job1_json).unwrap(),
        enabled: true,
        next_run_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        authority: "omon_owned".into(),
    };

    let _ = scheduler.execute_job(&cron_job1).await;
    {
        let msgs = dispatcher.messages.lock().unwrap();
        assert_eq!(
            msgs.len(),
            0,
            "failure_deliver:local must suppress error notification to discord:42"
        );
    }

    let job2_json = json!({
        "id": "routed_fail_job",
        "name": "Routed Fail",
        "prompt": "run",
        "deliver": "local",
        "failure_deliver": "discord:43",
        "schedule": {"kind": "cron", "expr": "0 * * * *"}
    });

    let cron_job2 = CronJob {
        id: "routed_fail_job".into(),
        session_key: None,
        expression: "0 * * * *".into(),
        payload_json: serde_json::to_string(&job2_json).unwrap(),
        enabled: true,
        next_run_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        authority: "omon_owned".into(),
    };

    let _ = scheduler.execute_job(&cron_job2).await;
    {
        let msgs = dispatcher.messages.lock().unwrap();
        assert_eq!(
            msgs.len(),
            1,
            "failure_deliver:discord:43 must route failure alert to channel 43"
        );
        assert_eq!(msgs[0].0, "43");
        assert!(msgs[0].1.contains("intentional failure"));
    }
}

struct MutableFailingExecutor {
    error_message: Arc<std::sync::Mutex<String>>,
}

#[async_trait]
impl CronTaskExecutor for MutableFailingExecutor {
    async fn execute(&self, _job: &CronJob) -> Result<Option<String>, OmonError> {
        let msg = self.error_message.lock().unwrap().clone();
        Err(OmonError::ToolExecution(msg))
    }
}

#[tokio::test]
async fn acknowledged_failure_signature_stays_silent() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let dispatcher = Arc::new(FailureRoutingDispatcher::default());
    let err_holder = Arc::new(std::sync::Mutex::new("connection refused".to_string()));
    let executor = Arc::new(MutableFailingExecutor {
        error_message: err_holder.clone(),
    });

    let scheduler =
        CronScheduler::with_dispatcher(database.pool().clone(), executor, dispatcher.clone());

    let job_json = json!({
        "id": "failing_job_incidents",
        "name": "Failing Job Incidents",
        "prompt": "run",
        "deliver": "discord:42",
        "schedule": {"kind": "cron", "expr": "0 * * * *"}
    });

    let cron_job = CronJob {
        id: "failing_job_incidents".into(),
        session_key: None,
        expression: "0 * * * *".into(),
        payload_json: serde_json::to_string(&job_json).unwrap(),
        enabled: true,
        next_run_at: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        authority: "omon_owned".into(),
    };

    // First failure: alerts
    let _ = scheduler.execute_job(&cron_job).await;
    assert_eq!(dispatcher.messages.lock().unwrap().len(), 1);

    // Operator acknowledges the incident
    scheduler
        .acknowledge_incident("failing_job_incidents", None)
        .await
        .unwrap();

    // Second failure with SAME error: must stay silent (acknowledged)
    let _ = scheduler.execute_job(&cron_job).await;
    assert_eq!(
        dispatcher.messages.lock().unwrap().len(),
        1,
        "Acknowledged failure signature must stay silent"
    );

    // Third failure with DIFFERENT error signature: must alert again
    *err_holder.lock().unwrap() = "out of disk space".to_string();
    let _ = scheduler.execute_job(&cron_job).await;
    assert_eq!(
        dispatcher.messages.lock().unwrap().len(),
        2,
        "New failure signature must generate a fresh alert"
    );
}
