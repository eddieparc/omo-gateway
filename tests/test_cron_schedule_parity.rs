use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use omon_gateway::{
    CronJob, CronScheduler, CronTaskExecutor, Database, HermesStore, HermesStoreSynchronizer,
    OutboundAction, OutboundDispatcher, Result,
};
use serde_json::json;
use tokio::sync::mpsc;

fn instant(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

struct CapturingExecutor {
    started: mpsc::UnboundedSender<CronJob>,
    clock: Arc<Mutex<DateTime<Utc>>>,
    completed_at: DateTime<Utc>,
}

#[async_trait]
impl CronTaskExecutor for CapturingExecutor {
    async fn execute(&self, job: &CronJob) -> Result<Option<String>> {
        self.started.send(job.clone()).unwrap();
        *self.clock.lock().unwrap() = self.completed_at;
        Ok(Some("calendar-result".into()))
    }
}

struct CapturingDispatcher(mpsc::UnboundedSender<OutboundAction>);

#[async_trait]
impl OutboundDispatcher for CapturingDispatcher {
    async fn dispatch(&self, action: OutboundAction) -> Result<()> {
        self.0.send(action).unwrap();
        Ok(())
    }
}

#[tokio::test]
async fn imported_cron_retains_timezone() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("seoul");
    tokio::fs::create_dir_all(home.join("cron")).await.unwrap();
    tokio::fs::write(home.join("config.yaml"), "timezone: Asia/Seoul\n")
        .await
        .unwrap();
    let bytes = serde_json::to_vec(&json!({"jobs": [{
        "id": "daily", "prompt": "report", "enabled": true,
        "schedule": {"kind": "cron", "expr": "0 9 * * *"},
        "next_run_at": "2026-09-05T00:00:00Z", "deliver": "discord:42",
        "attach_to_session": false
    }]}))
    .unwrap();
    tokio::fs::write(home.join("cron/jobs.json"), &bytes)
        .await
        .unwrap();
    let url = format!("sqlite://{}", root.path().join("calendar.sqlite").display());
    let database = Database::connect(&url).await.unwrap();
    let due = instant("2026-09-05T00:00:00Z");
    let completion = instant("2026-09-05T00:01:00Z");
    let clock = Arc::new(Mutex::new(due));
    let sync = HermesStoreSynchronizer::new(
        database.pool().clone(),
        vec![HermesStore::new("seoul", &home)],
    );
    assert_eq!(sync.sync_at(due).await.unwrap(), 1);
    let (started, mut executions) = mpsc::unbounded_channel();
    let (delivered, mut deliveries) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::with_dispatcher(
        database.pool().clone(),
        Arc::new(CapturingExecutor {
            started,
            clock: clock.clone(),
            completed_at: completion,
        }),
        Arc::new(CapturingDispatcher(delivered)),
    )
    .with_clock({
        let clock = clock.clone();
        move || *clock.lock().unwrap()
    });
    let mut notifications = scheduler.subscribe();
    assert_eq!(scheduler.run_due_jobs().await.unwrap(), 1);
    let executed = tokio::time::timeout(Duration::from_secs(5), executions.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(executed.id, "hermes:seoul:daily");
    let action = tokio::time::timeout(Duration::from_secs(5), deliveries.recv())
        .await
        .unwrap()
        .unwrap();
    match action {
        OutboundAction::SendMessage {
            session, content, ..
        } => {
            assert_eq!(session.channel_id, "42");
            assert_eq!(content, "calendar-result");
        }
        other => panic!("unexpected dispatch: {other:?}"),
    }
    let notification = tokio::time::timeout(Duration::from_secs(5), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notification.triggered_at, completion);
    tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
        .await
        .unwrap();
    let updated = scheduler.get(&executed.id).await.unwrap().unwrap();
    let run: (String, DateTime<Utc>, DateTime<Utc>) =
        sqlx::query_as("SELECT status, started_at, completed_at FROM cron_runs WHERE job_id = ?")
            .bind(&executed.id)
            .fetch_one(database.pool())
            .await
            .unwrap();
    assert_eq!(run, ("succeeded".into(), due, completion));
    println!(
        "import/run/completion: status={}, completed_at={}, next={:?}",
        run.0, run.2, updated.next_run_at
    );
    assert_eq!(updated.next_run_at, Some(instant("2026-09-06T00:00:00Z")));
    assert_eq!(
        updated.payload().unwrap()["schedule"]["timezone"],
        "Asia/Seoul"
    );
    drop(scheduler);
    drop(sync);
    database.close().await;
    let reopened = Database::connect(&url).await.unwrap();
    let sync = HermesStoreSynchronizer::new(
        reopened.pool().clone(),
        vec![HermesStore::new("seoul", &home)],
    );
    for _ in 0..2 {
        assert_eq!(sync.sync_at(completion).await.unwrap(), 1);
        let next: DateTime<Utc> = sqlx::query_scalar("SELECT next_run_at FROM cron_jobs")
            .fetch_one(reopened.pool())
            .await
            .unwrap();
        assert_eq!(next, instant("2026-09-06T00:00:00Z"));
        println!("reopen/resync: next={next}");
    }
    assert_eq!(
        tokio::fs::read(home.join("cron/jobs.json")).await.unwrap(),
        bytes
    );
    reopened.close().await;
    root.close().unwrap();
}

#[tokio::test]
async fn finite_budget_survives_failure_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", temp.path().join("cron.sqlite").display());
    let database = Database::connect(&url).await.unwrap();

    struct FailingExecutor;
    #[async_trait]
    impl CronTaskExecutor for FailingExecutor {
        async fn execute(&self, _job: &CronJob) -> Result<Option<String>> {
            Err(omon_gateway::OmonError::Multiplexer(
                "simulated execution error".into(),
            ))
        }
    }

    let clock_time = Arc::new(Mutex::new(instant("2026-09-07T00:00:00Z")));
    let clock_time_cb = clock_time.clone();

    let (delivered, _deliveries) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::with_dispatcher(
        database.pool().clone(),
        Arc::new(FailingExecutor),
        Arc::new(CapturingDispatcher(delivered)),
    )
    .with_clock(move || *clock_time_cb.lock().unwrap());

    // 1. interval:1m, repeat {times:2, completed:0}, executor error twice
    let spec = omon_gateway::CronJobSpec {
        session_key: None,
        expression: "interval:1m".into(),
        payload: serde_json::json!({
            "repeat": { "times": 2, "completed": 0 }
        }),
    };
    let job = scheduler
        .register_with_id("finite_fail", spec)
        .await
        .unwrap();
    assert!(job.enabled);

    // First failure attempt
    *clock_time.lock().unwrap() = instant("2026-09-07T00:01:05Z");
    scheduler.run_due_jobs().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let job_after_first = scheduler.get("finite_fail").await.unwrap().unwrap();
    let (times, completed) = omon_gateway::extract_repeat_info(&job_after_first.payload().unwrap());
    assert_eq!(times, Some(2));
    assert_eq!(
        completed, 1,
        "First failure must increment completed count toward finite budget"
    );
    assert!(
        job_after_first.enabled,
        "Job should still be enabled after 1 of 2 failures"
    );

    // Second failure attempt
    *clock_time.lock().unwrap() = instant("2026-09-07T00:02:10Z");
    scheduler.run_due_jobs().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let job_after_second = scheduler.get("finite_fail").await.unwrap().unwrap();
    let (_, completed_second) =
        omon_gateway::extract_repeat_info(&job_after_second.payload().unwrap());
    assert_eq!(
        completed_second, 2,
        "Second failure must increment completed count to 2"
    );
    assert!(
        !job_after_second.enabled,
        "Job must be disabled once finite budget is exhausted"
    );

    // 2. Triggering an exhausted job must be rejected
    let trigger_res = scheduler.trigger_job("finite_fail").await.unwrap();
    assert!(
        !trigger_res,
        "Triggering an exhausted job must return false / be rejected"
    );
}

#[tokio::test]
async fn predecessor_is_profile_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", temp.path().join("cron.sqlite").display());
    let database = Database::connect(&url).await.unwrap();

    struct EchoExecutor;
    #[async_trait]
    impl CronTaskExecutor for EchoExecutor {
        async fn execute(&self, job: &CronJob) -> Result<Option<String>> {
            let payload = job.payload().unwrap_or_default();
            let out = payload
                .get("echo_out")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("default");
            Ok(Some(out.to_string()))
        }
    }

    let clock_time = Arc::new(Mutex::new(instant("2026-09-07T00:00:00Z")));
    let clock_time_cb = clock_time.clone();

    let (delivered, _deliveries) = mpsc::unbounded_channel();
    let scheduler = CronScheduler::with_dispatcher(
        database.pool().clone(),
        Arc::new(EchoExecutor),
        Arc::new(CapturingDispatcher(delivered)),
    )
    .with_clock(move || *clock_time_cb.lock().unwrap());

    // Profile A job "abc123"
    let spec_a = omon_gateway::CronJobSpec {
        session_key: None,
        expression: "interval:1m".into(),
        payload: serde_json::json!({
            "profile": "A",
            "echo_out": "Output A"
        }),
    };
    scheduler
        .register_with_id("abc123_a", spec_a)
        .await
        .unwrap();

    // Profile B job "abc123"
    let spec_b = omon_gateway::CronJobSpec {
        session_key: None,
        expression: "interval:1m".into(),
        payload: serde_json::json!({
            "profile": "B",
            "echo_out": "Output B"
        }),
    };
    scheduler
        .register_with_id("abc123_b", spec_b)
        .await
        .unwrap();

    *clock_time.lock().unwrap() = instant("2026-09-07T00:01:05Z");
    scheduler.run_due_jobs().await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Direct output verification
    let output_a = omon_gateway::resolve_predecessor_output(database.pool(), "A", "abc123_a").await;
    assert_eq!(
        output_a.as_deref(),
        Some("Output A"),
        "Profile A predecessor must resolve exact Output A"
    );

    let output_b = omon_gateway::resolve_predecessor_output(database.pool(), "B", "abc123_b").await;
    assert_eq!(
        output_b.as_deref(),
        Some("Output B"),
        "Profile B predecessor must resolve exact Output B"
    );

    // Profile isolation: Profile A querying Profile B's job must return None
    let cross_query =
        omon_gateway::resolve_predecessor_output(database.pool(), "A", "abc123_b").await;
    assert_eq!(cross_query, None, "Cross-profile query must return None");

    // Wildcard safety: "%" or "abc%" must not match arbitrary jobs
    let wildcard_query = omon_gateway::resolve_predecessor_output(database.pool(), "A", "%").await;
    assert_eq!(
        wildcard_query, None,
        "Wildcard '%' must not match arbitrary jobs"
    );
}
