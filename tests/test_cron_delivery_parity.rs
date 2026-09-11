use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use omon_gateway::{
    extract_media_directives, storage::init_pool, CronJob, CronScheduler, CronTaskExecutor,
    OmonError, OutboundAction, OutboundDispatcher, Result,
};
use serde_json::json;

#[derive(Clone, Default)]
struct MockDispatcher {
    dispatched: Arc<Mutex<Vec<OutboundAction>>>,
    fail_channels: Arc<Mutex<Vec<String>>>,
    fail_media: Arc<Mutex<bool>>,
    uploaded_media: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl OutboundDispatcher for MockDispatcher {
    async fn dispatch(&self, action: OutboundAction) -> Result<()> {
        if let OutboundAction::SendMessage {
            ref session,
            ref content,
            ..
        } = action
        {
            let channel = session.channel_id.clone();
            if self.fail_channels.lock().unwrap().contains(&channel) {
                return Err(OmonError::Multiplexer(format!(
                    "simulated failure for {channel}"
                )));
            }
            if content.contains("MEDIA:") {
                let (_, media_paths) = extract_media_directives(content);
                for path in media_paths {
                    if *self.fail_media.lock().unwrap() {
                        return Err(OmonError::ToolExecution(format!(
                            "media upload failed for {path}"
                        )));
                    }
                    self.uploaded_media.lock().unwrap().push(path);
                }
            }
        }
        self.dispatched.lock().unwrap().push(action);
        Ok(())
    }
}

struct TestCronExecutor {
    fail: bool,
    content: Option<String>,
}

#[async_trait]
impl CronTaskExecutor for TestCronExecutor {
    async fn execute(&self, _job: &CronJob) -> Result<Option<String>> {
        if self.fail {
            Err(OmonError::Config("execution exploded".into()))
        } else {
            Ok(self.content.clone())
        }
    }
}

#[tokio::test]
async fn ack_requires_complete_output_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let ack_file = temp.path().join("ack.log");

    let pool = init_pool("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let dispatcher = Arc::new(MockDispatcher::default());
    let executor = Arc::new(TestCronExecutor {
        fail: false,
        content: Some("Report content".into()),
    });

    let scheduler = CronScheduler::with_dispatcher(pool.clone(), executor, dispatcher.clone());

    let ack_cmd = format!(
        "printf 'ack\\n' >> '{}'",
        ack_file.to_string_lossy().replace('\\', "/")
    );

    let job_json = json!({
        "id": "ack_test_job",
        "name": "Ack Test",
        "prompt": "run report",
        "deliver": ["discord:42", "discord:43"],
        "ack_command": ack_cmd,
        "schedule": {"kind": "cron", "expr": "0 * * * *"}
    });

    let cron_job = CronJob {
        id: "ack_test_job".into(),
        session_key: None,
        expression: "0 * * * *".into(),
        payload_json: serde_json::to_string(&job_json).unwrap(),
        enabled: true,
        next_run_at: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        authority: "omon_owned".into(),
    };

    dispatcher.fail_channels.lock().unwrap().push("43".into());
    let _ = scheduler.execute_job(&cron_job).await;

    let acks_case1 = std::fs::read_to_string(&ack_file).unwrap_or_default();
    let ack_count_case1 = acks_case1.lines().count();
    assert_eq!(
        ack_count_case1, 0,
        "ACK must NOT run when one or more destinations fail delivery"
    );

    dispatcher.fail_channels.lock().unwrap().clear();
    let res_succ = scheduler.execute_job(&cron_job).await;
    assert!(
        res_succ.is_ok(),
        "Run must succeed when all destinations succeed"
    );

    let acks_case2 = std::fs::read_to_string(&ack_file).unwrap_or_default();
    let ack_count_case2 = acks_case2.lines().count();
    assert_eq!(
        ack_count_case2, 1,
        "ACK must run exactly ONCE when all destinations succeed"
    );

    let err_executor = Arc::new(TestCronExecutor {
        fail: true,
        content: None,
    });
    let err_scheduler =
        CronScheduler::with_dispatcher(pool.clone(), err_executor, dispatcher.clone());

    let _ = err_scheduler.execute_job(&cron_job).await;
    let acks_case3 = std::fs::read_to_string(&ack_file).unwrap_or_default();
    let ack_count_case3 = acks_case3.lines().count();
    assert_eq!(
        ack_count_case3, 1,
        "Executor error alert send must NOT execute ACK (count remains 1)"
    );

    let media_file = temp.path().join("chart.png");
    std::fs::write(&media_file, b"fake png bytes").unwrap();

    let media_executor = Arc::new(TestCronExecutor {
        fail: false,
        content: Some(format!("Report summary\nMEDIA:{}", media_file.display())),
    });
    let media_scheduler =
        CronScheduler::with_dispatcher(pool.clone(), media_executor, dispatcher.clone());

    *dispatcher.fail_media.lock().unwrap() = true;
    let res_media_fail = media_scheduler.execute_job(&cron_job).await;
    assert!(
        res_media_fail.is_err(),
        "Job execution must fail when media upload fails"
    );
    let acks_case4a = std::fs::read_to_string(&ack_file).unwrap_or_default();
    assert_eq!(
        acks_case4a.lines().count(),
        1,
        "Failed media upload must NOT trigger ACK (count remains 1)"
    );

    *dispatcher.fail_media.lock().unwrap() = false;
    let res_media_ok = media_scheduler.execute_job(&cron_job).await;
    assert!(
        res_media_ok.is_ok(),
        "Job execution must succeed when media upload succeeds"
    );
    let acks_case4b = std::fs::read_to_string(&ack_file).unwrap_or_default();
    assert_eq!(
        acks_case4b.lines().count(),
        2,
        "Successful media upload and delivery must trigger ACK (count becomes 2)"
    );
    assert!(dispatcher
        .uploaded_media
        .lock()
        .unwrap()
        .contains(&media_file.to_str().unwrap().to_string()));
}
