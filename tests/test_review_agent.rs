//! R-AG-13: real producer and startup-recovery fault injection, without live services.
use std::{
    future::Future,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{FutureExt, SinkExt, StreamExt};
use omon_gateway::{
    recover_pending_delivery_obligations, AgentBackend, CronJob, CronScheduler, CronTaskExecutor,
    Database, DeliveryLedgerService, InboundEvent, OmoBackend, OmoBackendConfig, OmonError,
    OutboundAction, OutboundDispatcher, SessionContext, SessionKey,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{oneshot, watch},
    task::JoinHandle,
};
use tokio_tungstenite::tungstenite::Message;

const BOUND: Duration = Duration::from_secs(20);
const REPORT: &str = "REPORT";
const ACK: &str = "AG13_ACK";
type TestFuture<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
type Row = (String, String, String, i64);

struct Fixture {
    db: Database,
    root: PathBuf,
    stop: watch::Sender<bool>,
    peers: Mutex<Vec<JoinHandle<()>>>,
}

// Every assertion (including setup/RPC assertions) is caught before cleanup.
// A separate owned runtime is dropped before removing the file-backed database.
fn fixture(name: &str, body: impl for<'a> FnOnce(&'a Fixture) -> TestFuture<'a>) {
    let temp = tempfile::tempdir().expect("owned fixture directory");
    let path = temp.path().to_owned();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        runtime.block_on(async {
        let url = format!("sqlite://{}?mode=rwc", path.join("ledger.sqlite").display().to_string().replace('\\', "/"));
        let db = Database::connect(&url).await.expect("migrate owned SQLite");
        let (stop, _) = watch::channel(false);
        let f = Fixture { db, root: path.clone(), stop, peers: Mutex::new(Vec::new()) };
        let result = AssertUnwindSafe(tokio::time::timeout(BOUND, body(&f))).catch_unwind().await;
        // send_replace also works if a peer failed before subscribing/completing.
        f.stop.send_replace(true);
        let peers = std::mem::take(&mut *f.peers.lock());
        let mut peer_errors = Vec::new();
        for mut peer in peers {
            match tokio::time::timeout(BOUND, &mut peer).await {
                Ok(Ok(())) => (),
                Ok(Err(error)) => peer_errors.push(error.to_string()),
                Err(error) => {
                    peer.abort();
                    let joined = peer.await;
                    peer_errors.push(format!("join deadline: {error}; reaped={joined:?}"));
                }
            }
        }
        f.db.close().await;
        eprintln!("R-AG-13 cleanup {name}: peers_joined=true db_closed=true peer_errors={peer_errors:?}");
        assert!(peer_errors.is_empty(), "peer failures: {peer_errors:?}");
        match result {
            Ok(Ok(())) => (),
            Ok(Err(error)) => panic!("fixture completion deadline: {error}"),
            Err(error) => resume_unwind(error),
        }
    })
    }));
    drop(runtime);
    let removed = temp.close();
    eprintln!(
        "R-AG-13 cleanup {name}: runtime_closed=true fixture_removed={}",
        removed.is_ok()
    );
    removed.expect("remove owned fixture after closing runtime and DB");
    assert!(!path.exists());
    if let Err(error) = outcome {
        resume_unwind(error);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    AgInsert,
    SecInsert,
    Attempt,
    Delivered,
    Failed,
    Claim,
    Abandon,
}
impl Fault {
    fn marker(self) -> &'static str {
        match self {
            Self::AgInsert => "outbox blocked",
            Self::SecInsert => "SEC-02",
            _ => "AG13_WRITE_FAULT",
        }
    }
    async fn install(self, f: &Fixture) {
        let sql = match self {
            Self::None => return,
            Self::AgInsert => "CREATE TRIGGER block_outbox BEFORE INSERT ON delivery_obligations BEGIN SELECT RAISE(ABORT,'outbox blocked'); END;".to_string(),
            Self::SecInsert => "CREATE TRIGGER reject_obligation BEFORE INSERT ON delivery_obligations BEGIN SELECT RAISE(ABORT, 'SEC-02'); END;".to_string(),
            other => {
                let condition = match other {
                    Self::Attempt => "NEW.state = 'attempting'",
                    Self::Delivered => "NEW.state = 'delivered'",
                    Self::Failed => "NEW.state = 'failed'",
                    Self::Claim => "NEW.attempts > OLD.attempts",
                    Self::Abandon => "NEW.state = 'abandoned'",
                    _ => unreachable!(),
                };
                format!("CREATE TRIGGER write_fault BEFORE UPDATE ON delivery_obligations WHEN {condition} BEGIN SELECT RAISE(ABORT,'AG13_WRITE_FAULT'); END;")
            }
        };
        sqlx::query(&sql).execute(f.db.pool()).await.unwrap();
    }
    async fn remove(self, f: &Fixture) {
        let name = match self {
            Self::None => return,
            Self::AgInsert => "block_outbox",
            Self::SecInsert => "reject_obligation",
            _ => "write_fault",
        };
        sqlx::query(&format!("DROP TRIGGER {name}"))
            .execute(f.db.pool())
            .await
            .unwrap();
    }
    fn insert(self) -> bool {
        matches!(self, Self::AgInsert | Self::SecInsert)
    }
}

struct DeltaGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}
#[derive(Default)]
struct Recorder {
    attempts: Mutex<Vec<(SessionKey, String)>>,
    successes: AtomicUsize,
    fail: bool,
    delta_gate: Mutex<Option<DeltaGate>>,
}
#[async_trait]
impl OutboundDispatcher for Recorder {
    async fn dispatch(&self, action: OutboundAction) -> omon_gateway::Result<()> {
        let final_output = match action {
            OutboundAction::Stream { session, chunk } if chunk.is_final => {
                Some((session, chunk.content))
            }
            OutboundAction::SendMessage {
                session, content, ..
            } => Some((session, content)),
            OutboundAction::Stream { chunk, .. } if chunk.content == REPORT => {
                let gate = self.delta_gate.lock().take();
                if let Some(gate) = gate {
                    gate.entered.send(()).expect("subscribed delta observer");
                    gate.release.await.expect("deadline driver releases delta");
                }
                None
            }
            _ => None,
        };
        if let Some(output) = final_output {
            self.attempts.lock().push(output);
            if self.fail {
                return Err(OmonError::Llm("fixture transport refused".into()));
            }
            self.successes.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[derive(Default)]
struct PeerCounts {
    starts: AtomicUsize,
    interrupts: AtomicUsize,
    terminals: AtomicUsize,
}
impl Fixture {
    async fn peer(&self, deadline: bool) -> (String, Arc<PeerCounts>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut stop = self.stop.subscribe();
        let counts = Arc::new(PeerCounts::default());
        let seen = counts.clone();
        let task = tokio::spawn(async move {
            tokio::select! {
                changed = stop.changed() => { changed.expect("owned shutdown sender"); }
                () = async {
                    let (socket, _) = listener.accept().await.unwrap();
                    let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                    for (method, id) in [("initialize", 1), ("thread/resume", 2), ("turn/start", 3)] {
                        let frame = ws.next().await.unwrap().unwrap();
                        let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                        assert_eq!(request["jsonrpc"], "2.0");
                        assert_eq!(request["method"], method);
                        assert_eq!(request["id"], id);
                        if method != "initialize" { assert_eq!(request["params"]["threadId"], "r1"); }
                        let result = match method {
                            "initialize" => json!({"userAgent":"r-ag-13-fixture"}),
                            "thread/resume" => json!({"thread":{"id":"r1"}}),
                            _ => {
                                seen.starts.fetch_add(1, Ordering::SeqCst);
                                json!({"turn":{"id":"t1","status":"inProgress"}})
                            }
                        };
                        ws.send(Message::text(json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string())).await.unwrap();
                    }
                    ws.send(Message::text(json!({"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"threadId":"r1","turnId":"t1","itemId":"m1","delta":REPORT}}).to_string())).await.unwrap();
                    if deadline {
                        let frame = ws.next().await.unwrap().unwrap();
                        let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                        assert_eq!(request["method"], "turn/interrupt");
                        assert_eq!(request["id"], 9001);
                        assert_eq!(request["params"]["threadId"], "r1");
                        assert_eq!(request["params"]["turnId"], "t1");
                        seen.interrupts.fetch_add(1, Ordering::SeqCst);
                        // The matching completed event races ahead of the interrupt ACK,
                        // as allowed by the appserver. It must use the cleanup finalizer.
                    }
                    ws.send(Message::text(json!({"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"r1","turn":{"id":"t1","status":"completed"}}}).to_string())).await.unwrap();
                    seen.terminals.fetch_add(1, Ordering::SeqCst);
                    // Keep the socket/listener owned until the test requests cleanup.
                    std::future::pending::<()>().await;
                } => unreachable!("peer exits only through owned shutdown"),
            }
        });
        self.peers.lock().push(task);
        (format!("ws://{address}"), counts)
    }
    async fn rows(&self) -> Vec<Row> {
        sqlx::query_as("SELECT id, content, state, attempts FROM delivery_obligations ORDER BY id")
            .fetch_all(self.db.pool())
            .await
            .unwrap()
    }
    fn key(&self) -> SessionKey {
        SessionKey::new("discord", None::<String>, "42", None::<String>, "cron").with_bot_id("84")
    }
    fn ack_path(&self) -> PathBuf {
        self.root.join("ack-ran")
    }
    fn ack_command(&self) -> String {
        let path = self.ack_path().display().to_string().replace('\\', "/");
        // Only shell builtins, with an absolute owned target; no touch, scripts,
        // provider command, repository dotenv, or caller-supplied shell content.
        if cfg!(windows) && !std::path::Path::new("C:\\Program Files\\Git\\bin\\bash.exe").exists()
        {
            assert!(!path.contains(['"', '%', '!']));
            format!("echo {ACK}>\"{path}\"")
        } else {
            format!("echo {ACK} > '{}'", path.replace('\'', "'\\''"))
        }
    }
    fn acked(&self) -> bool {
        match std::fs::read_to_string(self.ack_path()) {
            Ok(value) => {
                assert_eq!(value.trim(), ACK);
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("read owned ACK sentinel: {error}"),
        }
    }
    async fn dead_owner(&self) {
        // A previous incarnation of this PID: deterministic, no process probing
        // or killing an arbitrary PID. This is the ledger's native restart seam.
        sqlx::query("UPDATE delivery_obligations SET owner_pid = ?, owner_started_at = '1970-01-01T00:00:00Z'")
            .bind(std::process::id() as i64).execute(self.db.pool()).await.unwrap();
    }
}

fn require_fault<T: std::fmt::Debug>(result: &omon_gateway::Result<T>, fault: Fault) {
    let error = result
        .as_ref()
        .expect_err("durability failure must reach the real caller");
    assert!(
        error.to_string().contains(fault.marker()),
        "lost persistence cause: {error}"
    );
}

// Real startup reconciliation, never a mock/helper replacement for recovery.
// Called after recording all producer outcomes, so RED assertions cannot hide
// recovery results. It must not start a second agent/executor turn or run ACK.
async fn reconcile(
    f: &Fixture,
    fault: Fault,
    before: &[Row],
) -> Option<(omon_gateway::Result<usize>, Arc<Recorder>, Vec<Row>)> {
    if fault == Fault::None || fault.insert() {
        return None;
    }
    fault.remove(f).await;
    f.dead_owner().await;
    let recorder = Arc::new(Recorder::default());
    let result = recover_pending_delivery_obligations(f.db.pool(), recorder.clone()).await;
    let rows = f.rows().await;
    eprintln!("R-AG-13 checkpoint recovery: before={before:?} result={result:?} attempts={} successes={} after={rows:?}", recorder.attempts.lock().len(), recorder.successes.load(Ordering::SeqCst));
    Some((result, recorder, rows))
}

fn check_reconciled(
    recovery: Option<(omon_gateway::Result<usize>, Arc<Recorder>, Vec<Row>)>,
    before: &[Row],
) {
    if let Some((result, recorder, rows)) = recovery {
        assert_eq!(result.unwrap(), 1);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].1, REPORT);
        assert_ne!(
            before[0].2, "delivered",
            "failed checkpoint falsely durable"
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, before[0].0);
        assert_eq!(rows[0].1, REPORT);
        assert_eq!(rows[0].2, "delivered");
        assert_eq!(rows[0].3, before[0].3 + 1);
        assert_eq!(recorder.successes.load(Ordering::SeqCst), 1);
        let outputs = recorder.attempts.lock();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.bot_id.as_deref(), Some("84"));
        assert!(outputs[0].1.ends_with(REPORT));
    }
}

async fn backend_case(f: &Fixture, fault: Fault, deadline: bool) {
    fault.install(f).await;
    let (url, peer) = f.peer(deadline).await;
    // Subscribe before backend.run or any peer frame can trigger the gate.
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let recorder = Arc::new(Recorder {
        fail: fault == Fault::Failed,
        delta_gate: Mutex::new(deadline.then_some(DeltaGate {
            entered: entered_tx,
            release: release_rx,
        })),
        ..Default::default()
    });
    let mut config = OmoBackendConfig::new(url);
    config.per_agent_workspace = false;
    config.total_timeout = if deadline {
        Duration::from_secs(10)
    } else {
        BOUND
    };
    let backend = OmoBackend::new(config, recorder.clone()).with_pool(f.db.pool().clone());
    let key = f.key();
    let mut session = SessionContext::new(key.clone());
    session
        .state
        .metadata
        .insert("omo_thread_id".into(), json!("r1"));
    session
        .state
        .metadata
        .insert("cron_ack_command".into(), json!(f.ack_command()));
    let driver = async {
        if deadline {
            entered_rx
                .await
                .expect("own correlated REPORT delta observed");
            // Time is under test only here. No waiting for a wall-clock deadline,
            // no auto-advance while SQLite or the socket is doing real I/O.
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(6)).await;
            tokio::time::resume();
            release_tx
                .send(())
                .expect("backend delta gate remains owned");
        }
    };
    let (result, ()) = tokio::join!(
        backend.run(&mut session, InboundEvent::message(key, "msg", "digest")),
        driver
    );
    let rows = f.rows().await;
    let sends = recorder.attempts.lock().len();
    let successes = recorder.successes.load(Ordering::SeqCst);
    let acked = f.acked();
    eprintln!("R-AG-13 backend deadline={deadline} fault={fault:?}: result={result:?} final_send_count={sends} successes={successes} acked={acked} rows={rows:?}");
    let recovery = reconcile(f, fault, &rows).await;
    let acked_after_recovery = f.acked();
    assert_eq!(
        peer.starts.load(Ordering::SeqCst),
        1,
        "recovery must not rerun agent"
    );
    assert_eq!(
        peer.interrupts.load(Ordering::SeqCst),
        usize::from(deadline)
    );
    assert_eq!(peer.terminals.load(Ordering::SeqCst), 1);
    if fault == Fault::None {
        result.unwrap();
        assert_eq!(sends, 1);
        assert_eq!(successes, 1);
        assert!(acked, "real ACK command control must execute");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "delivered");
    } else {
        require_fault(&result, fault);
        assert_eq!(
            sends,
            usize::from(matches!(fault, Fault::Delivered | Fault::Failed))
        );
        assert_eq!(successes, usize::from(fault == Fault::Delivered));
        assert!(!acked, "ACK must not precede required durable writes");
        assert!(
            !acked_after_recovery,
            "startup ledger recovery must not execute producer ACK"
        );
        if fault.insert() {
            assert!(rows.is_empty());
        }
    }
    check_reconciled(recovery, &rows);
}

struct ReportExecutor(AtomicUsize);
#[async_trait]
impl CronTaskExecutor for ReportExecutor {
    async fn execute(&self, _: &CronJob) -> omon_gateway::Result<Option<String>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Some(REPORT.into()))
    }
}
async fn cron_case(f: &Fixture, fault: Fault) {
    fault.install(f).await;
    let executor = Arc::new(ReportExecutor(AtomicUsize::new(0)));
    let recorder = Arc::new(Recorder {
        fail: fault == Fault::Failed,
        ..Default::default()
    });
    let now: chrono::DateTime<chrono::Utc> = "2026-09-05T00:01:00Z".parse().unwrap();
    let scheduler =
        CronScheduler::with_dispatcher(f.db.pool().clone(), executor.clone(), recorder.clone())
            .with_clock(move || now);
    // execute_job is the public producer entry; do not start its polling loop.
    let job = CronJob {
        id: "r-ag-13".into(), session_key: Some(f.key().storage_key()),
        expression: "0 * * * *".into(), enabled: true, next_run_at: None,
        created_at: now, updated_at: now, authority: "omon_owned".into(),
        payload_json: json!({"id":"r-ag-13","prompt":"digest","deliver":"discord:42","attach_to_session":false,"ack_command":f.ack_command(),"schedule":{"kind":"cron","expr":"0 * * * *"}}).to_string(),
    };
    let result = scheduler.execute_job(&job).await;
    let rows = f.rows().await;
    let sends = recorder.attempts.lock().len();
    let acked = f.acked();
    let successes = recorder.successes.load(Ordering::SeqCst);
    eprintln!("R-AG-13 cron fault={fault:?}: result={result:?} final_send_count={sends} successes={successes} acked={acked} rows={rows:?}");
    let recovery = reconcile(f, fault, &rows).await;
    let acked_after_recovery = f.acked();
    scheduler.shutdown().await;
    assert_eq!(
        executor.0.load(Ordering::SeqCst),
        1,
        "recovery must not rerun cron executor"
    );
    if fault == Fault::None {
        assert_eq!(result.unwrap().as_deref(), Some(REPORT));
        assert_eq!(sends, 1);
        assert!(acked, "real scheduler ACK control must execute");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "delivered");
    } else {
        require_fault(&result, fault);
        assert_eq!(
            sends,
            usize::from(matches!(fault, Fault::Delivered | Fault::Failed))
        );
        assert_eq!(successes, usize::from(fault == Fault::Delivered));
        assert!(!acked);
        assert!(!acked_after_recovery);
        if fault.insert() {
            assert!(rows.is_empty());
        }
    }
    check_reconciled(recovery, &rows);
}

async fn recovery_case(f: &Fixture, fault: Fault, transport_fails: bool) {
    let ledger = DeliveryLedgerService::new(f.db.pool().clone());
    ledger
        .record_obligation("obl:review:dead-owner", &f.key(), REPORT)
        .await
        .unwrap();
    f.dead_owner().await;
    if fault == Fault::Abandon {
        sqlx::query("UPDATE delivery_obligations SET attempts = 3")
            .execute(f.db.pool())
            .await
            .unwrap();
    }
    fault.install(f).await;
    let recorder = Arc::new(Recorder {
        fail: transport_fails,
        ..Default::default()
    });
    // This is exactly SEC-02's declared exported startup entry.
    let result = recover_pending_delivery_obligations(f.db.pool(), recorder.clone()).await;
    let rows = f.rows().await;
    let attempts = recorder.attempts.lock().len();
    let successes = recorder.successes.load(Ordering::SeqCst);
    eprintln!("R-AG-13 recovery fault={fault:?} transport_fails={transport_fails}: result={result:?} attempted={attempts} successfully_dispatched={successes} rows={rows:?}");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, REPORT);
    if fault != Fault::None {
        require_fault(&result, fault);
        assert_ne!(rows[0].2, "delivered");
        assert_eq!(successes, usize::from(fault == Fault::Delivered));
        assert_eq!(
            attempts,
            usize::from(matches!(fault, Fault::Delivered | Fault::Failed))
        );
        if matches!(fault, Fault::Claim | Fault::Abandon) {
            assert_eq!(rows[0].3, if fault == Fault::Abandon { 3 } else { 0 });
        } else {
            assert_eq!(rows[0].3, 1);
        }
    } else if transport_fails {
        assert!(
            result.is_err() || matches!(result, Ok(0)),
            "attempted is not successfully recovered: {result:?}"
        );
        assert_eq!(rows[0].2, "failed");
        assert_eq!(attempts, 1);
        assert_eq!(successes, 0);
    } else {
        assert_eq!(result.unwrap(), 1);
        assert_eq!(rows[0].2, "delivered");
        assert_eq!(rows[0].3, 1);
        assert_eq!(attempts, 1);
        assert_eq!(successes, 1);
    }
}

// Continue after a RED subcase, but never skip/delete its assertion. The exact
// native selection fails if any subcase fails; each has its own cleanup receipt.
fn cases(entries: &[(Fault, bool)], body: fn(&Fixture, Fault, bool) -> TestFuture<'_>) {
    let mut failures = Vec::new();
    for &(fault, flag) in entries {
        let name = format!("{fault:?}/{flag}");
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            fixture(&name, |f| body(f, fault, flag))
        }));
        if outcome.is_err() {
            failures.push(name);
        }
    }
    assert!(failures.is_empty(), "R-AG-13 failed subcases: {failures:?}");
}

#[test]
fn ag13_outbox_insert_failure_prevents_final_send() {
    cases(
        &[(Fault::AgInsert, false), (Fault::AgInsert, true)],
        |f, fault, deadline| Box::pin(backend_case(f, fault, deadline)),
    );
}

#[test]
fn sec02_outbox_write_faults() {
    let mut entries = Vec::new();
    for deadline in [false, true] {
        for fault in [
            Fault::None,
            Fault::SecInsert,
            Fault::Attempt,
            Fault::Delivered,
            Fault::Failed,
        ] {
            entries.push((fault, deadline));
        }
    }
    cases(&entries, |f, fault, deadline| {
        Box::pin(backend_case(f, fault, deadline))
    });
}

#[test]
fn ag13_cron_outbox_write_faults() {
    cases(
        &[
            (Fault::None, false),
            (Fault::AgInsert, false),
            (Fault::Attempt, false),
            (Fault::Delivered, false),
            (Fault::Failed, false),
        ],
        |f, fault, _| Box::pin(cron_case(f, fault)),
    );
}

#[test]
fn sec02_recovery_write_faults() {
    cases(
        &[
            (Fault::None, false),
            (Fault::Claim, false),
            (Fault::Attempt, false),
            (Fault::Delivered, false),
            (Fault::Failed, true),
            (Fault::Abandon, false),
            (Fault::None, true),
        ],
        |f, fault, transport_fails| Box::pin(recovery_case(f, fault, transport_fails)),
    );
}

// AG-15 / DASH-012: public configuration API only. Runtime ensure/spawn counts
// remain the separate dashboard runtime fixture's obligation.
mod ag15_endpoint_fallback {
    use omon_gateway::OmoBackendConfig;
    use std::{
        panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
        process::Stdio,
        time::Duration,
    };

    const INTERACTIVE: &str = "ws://127.0.0.1:29998";
    const OVERRIDE: &str = "ws://127.0.0.1:29999";
    const CHILD_CASE: &str = "OMON_REVIEW_AG15_CHILD_CASE";
    const COMPLETE_FILE: &str = "ag15-config-child-complete";
    const COMPLETE: &[u8] = b"AG15_CONFIG_CHILD_COMPLETE\n";
    const COMPLETION_BOUND: Duration = Duration::from_secs(20);
    const REAP_BOUND: Duration = Duration::from_secs(5);

    // Re-execute exactly this test, never Cargo or the gateway CLI. Child env
    // and cwd are owned; public config construction does not load dotenv or
    // connect to either URL. Inherited stdio avoids pipe-drain deadlocks.
    fn isolated(
        selector: &str,
        cron_override: Option<&str>,
        check: impl FnOnce(OmoBackendConfig, OmoBackendConfig),
    ) {
        if let Some(child_case) = std::env::var_os(CHILD_CASE) {
            assert_eq!(child_case, std::ffi::OsStr::new(selector));
            assert_eq!(
                std::env::var("OMON_OMO_APPSERVER_URL").unwrap(),
                INTERACTIVE
            );
            assert_eq!(
                std::env::var_os("OMON_OMO_CRON_APPSERVER_URL"),
                cron_override.map(std::ffi::OsString::from),
            );
            let interactive = OmoBackendConfig::from_env().expect("public interactive config");
            let cron = OmoBackendConfig::cron_from_env().expect("public cron config");
            eprintln!(
                "{}",
                serde_json::json!({
                    "case": selector,
                    "interactive_url": interactive.appserver_url,
                    "cron_url": cron.appserver_url,
                    "endpoint_equal": interactive.appserver_url == cron.appserver_url,
                })
            );
            check(interactive, cron);
            // A successful zero-test child invocation cannot satisfy this.
            std::fs::write(COMPLETE_FILE, COMPLETE).expect("write child completion sentinel");
            return;
        }

        let root = tempfile::tempdir().expect("owned config subprocess cwd");
        let root_path = root.path().to_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("config subprocess runtime");
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            runtime.block_on(async {
                let mut command = tokio::process::Command::new(
                    std::env::current_exe().expect("current native test executable"),
                );
                command
                    .args([selector, "--exact", "--nocapture", "--test-threads=1"])
                    .env_clear()
                    .env(CHILD_CASE, selector)
                    .env("OMON_OMO_APPSERVER_URL", INTERACTIVE)
                    .current_dir(&root_path)
                    .stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit())
                    .kill_on_drop(true);
                // Only Windows loader metadata survives the environment reset;
                // no host credentials, model/timeouts, HOME, PATH or dotenv.
                for name in ["SystemRoot", "WINDIR"] {
                    if let Some(value) = std::env::var_os(name) {
                        command.env(name, value);
                    }
                }
                if let Some(url) = cron_override {
                    command.env("OMON_OMO_CRON_APPSERVER_URL", url);
                }
                let mut child = command.spawn().expect("spawn owned config subprocess");
                let waited = tokio::time::timeout(COMPLETION_BOUND, child.wait()).await;
                match waited {
                    Ok(Ok(status)) => {
                        // wait reaps even a child that panics in a config assertion.
                        eprintln!("AG15 cleanup case={selector} child_reaped=true status={status}");
                        assert!(status.success(), "config subprocess failed: {status}");
                        assert_eq!(
                            std::fs::read(root_path.join(COMPLETE_FILE))
                                .expect("selected child completed its assertions"),
                            COMPLETE,
                        );
                    }
                    failure => {
                        // No assertion precedes this owned-child cleanup attempt.
                        // Kill errors and reap errors are reported, never discarded.
                        let killed = child.start_kill();
                        let reaped = tokio::time::timeout(REAP_BOUND, child.wait()).await;
                        panic!(
                            "config subprocess completion failed: {failure:?}; kill={killed:?}; reap={reaped:?}"
                        );
                    }
                }
            });
        }));
        drop(runtime);
        let removed = root.close();
        eprintln!(
            "AG15 cleanup case={selector} runtime_closed=true fixture_removed={}",
            removed.is_ok(),
        );
        removed.expect("remove config fixture after child wait and runtime shutdown");
        assert!(!root_path.exists());
        if let Err(error) = outcome {
            resume_unwind(error);
        }
    }

    #[test]
    fn custom_interactive_endpoint_is_inherited_without_cron_override() {
        isolated(
            "ag15_endpoint_fallback::custom_interactive_endpoint_is_inherited_without_cron_override",
            None,
            |interactive, cron| {
                assert_eq!(interactive.appserver_url, INTERACTIVE);
                assert_eq!(cron.appserver_url, interactive.appserver_url);
                assert_eq!(cron.appserver_url, INTERACTIVE);
            },
        );
    }

    #[test]
    fn explicit_supported_cron_endpoint_override_is_preserved() {
        isolated(
            "ag15_endpoint_fallback::explicit_supported_cron_endpoint_override_is_preserved",
            Some(OVERRIDE),
            |interactive, cron| {
                // Existing public API support, not a new runtime divergence policy.
                assert_eq!(interactive.appserver_url, INTERACTIVE);
                assert_eq!(cron.appserver_url, OVERRIDE);
            },
        );
    }
}
