// Compile the real binary modules, including their parent-dependent imports.
// Neither main() nor run_standalone_cli() is called by this fixture.
include!("../src/entry.rs");

mod r_ag_15 {
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::State;
    use axum::routing::get;
    use axum::Router;
    use futures_util::StreamExt;
    use serde_json::{json, Map, Value};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio::process::Command;
    use tokio::sync::{mpsc, oneshot};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_util::sync::CancellationToken;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::Layer;

    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Check<T> = std::result::Result<T, Error>;
    const TEST: &str = "r_ag_15::dash_011_012_runtime_and_surface";
    const URL: &str = "ws://127.0.0.1:29998";
    const IDS: [&str; 4] = [
        "DASH-011",
        "DASH-011-SURFACE",
        "DASH-012",
        "DASH-012-SURFACE",
    ];
    const LIMIT: Duration = Duration::from_secs(15);

    fn error(message: impl Into<String>) -> Error {
        std::io::Error::other(message.into()).into()
    }

    async fn bounded<F: Future>(future: F) -> Check<F::Output> {
        tokio::time::timeout(LIMIT, future)
            .await
            .map_err(|e| error(e.to_string()))
    }

    // Includes timeout cleanup: taskkill is restricted to this owned child's tree.
    // Browser descendants have an additional Job Object exit receipt; this
    // function alone only reaps its direct child.
    async fn owned_output(mut command: Command, limit: Duration) -> Check<std::process::Output> {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let pid = child.id().ok_or_else(|| error("owned child has no PID"))?;
        let mut stdout = child.stdout.take().ok_or_else(|| error("missing stdout"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| error("missing stderr"))?;
        let out = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let err = tokio::spawn(async move {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).await.map(|_| bytes)
        });
        let wait = tokio::time::timeout(limit, child.wait()).await;
        let status = match wait {
            Ok(status) => status?,
            Err(deadline) => {
                let killer = PathBuf::from(std::env::var_os("SystemRoot").ok_or_else(|| {
                    error("Windows SystemRoot is required for owned-tree teardown")
                })?)
                .join("System32/taskkill.exe");
                let killed = bounded(
                    Command::new(killer)
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .output(),
                )
                .await??;
                let reaped = bounded(child.wait()).await?;
                let captured_out = bounded(out).await???;
                let captured_err = bounded(err).await???;
                return Err(error(format!(
                    "owned child {pid} deadline: {deadline}; taskkill={}; reap={reaped:?}; stdout={} stderr={} kill_stderr={}",
                    killed.status, String::from_utf8_lossy(&captured_out),
                    String::from_utf8_lossy(&captured_err), String::from_utf8_lossy(&killed.stderr)
                )));
            }
        };
        Ok(std::process::Output {
            status,
            stdout: bounded(out).await???,
            stderr: bounded(err).await???,
        })
    }

    #[derive(Default)]
    struct Fields(Map<String, Value>);
    impl Visit for Fields {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().into(), json!(format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().into(), json!(value));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().into(), json!(value));
        }
    }

    struct ConstructionTrace {
        listening: Mutex<Option<oneshot::Sender<String>>>,
        configs: Arc<Mutex<Vec<Value>>>,
    }
    impl<S: tracing::Subscriber> Layer<S> for ConstructionTrace {
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            let target = event.metadata().target();
            if target.ends_with("::dashboard") {
                if let Some(address) = fields.0.get("address").and_then(Value::as_str) {
                    if let Some(sender) = self.listening.lock().unwrap().take() {
                        // Receiver closure is a fixture failure reported by the awaiting task.
                        if sender.send(address.to_owned()).is_err() {
                            eprintln!("fixture listening receiver closed");
                        }
                    }
                }
            }
            if target.ends_with("::dashboard_runtime") && fields.0.contains_key("appserver_url") {
                fields.0.remove("message");
                self.configs.lock().unwrap().push(Value::Object(fields.0));
            }
        }
    }

    #[derive(Clone)]
    struct Peer {
        endpoint: String,
        probes: Arc<AtomicUsize>,
        captures: mpsc::UnboundedSender<std::result::Result<Value, String>>,
        shutdown: CancellationToken,
    }
    async fn ready(State(peer): State<Peer>) -> &'static str {
        peer.probes.fetch_add(1, Ordering::SeqCst);
        "ready"
    }
    async fn upgrade(State(peer): State<Peer>, ws: WebSocketUpgrade) -> axum::response::Response {
        ws.on_upgrade(move |socket| async move {
            if let Err(failure) = daemon_socket(socket, &peer).await {
                if peer.captures.send(Err(failure.to_string())).is_err() {
                    eprintln!("fake daemon failure after capture receiver closed: {failure}");
                }
            }
        })
    }
    async fn daemon_socket(mut socket: WebSocket, peer: &Peer) -> Check<()> {
        let mut thread_frame = None;
        let thread_id = uuid::Uuid::new_v4().to_string();
        loop {
            let incoming = tokio::select! {
                _ = peer.shutdown.cancelled() => return Ok(()),
                incoming = socket.recv() => incoming,
            };
            let Some(incoming) = incoming else {
                return Ok(());
            };
            match incoming? {
                Message::Text(text) => {
                    let frame: Value = serde_json::from_str(text.as_str())?;
                    match frame["method"].as_str() {
                        Some("initialize") => {
                            socket
                                .send(Message::Text(
                                    json!({
                                        "jsonrpc":"2.0", "id":frame["id"], "result":{}
                                    })
                                    .to_string()
                                    .into(),
                                ))
                                .await?
                        }
                        Some("thread/start") => {
                            if thread_frame.is_some() {
                                return Err(error("duplicate thread/start"));
                            }
                            socket.send(Message::Text(json!({
                                "jsonrpc":"2.0", "id":frame["id"], "result":{"thread":{"id":thread_id}}
                            }).to_string().into())).await?;
                            thread_frame = Some(frame);
                        }
                        Some("turn/start") => {
                            let thread = thread_frame
                                .take()
                                .ok_or_else(|| error("turn without fresh thread/start"))?;
                            if frame["params"]["threadId"] != thread_id {
                                return Err(error("turn/thread identity mismatch"));
                            }
                            // Capture the causal prompt too, then reject before inference.
                            // An explicit turn/start error is terminal, not a retryable transport failure.
                            socket
                                .send(Message::Text(
                                    json!({
                                        "jsonrpc":"2.0", "id":frame["id"],
                                        "error":{"code":-32000,"message":"R_AG_15_CAPTURE_ONLY"}
                                    })
                                    .to_string()
                                    .into(),
                                ))
                                .await?;
                            peer.captures
                                .send(Ok(json!({
                                    "endpoint":peer.endpoint, "frame":thread, "turnFrame":frame
                                })))
                                .map_err(|e| error(e.to_string()))?;
                            return Ok(());
                        }
                        other => {
                            return Err(error(format!("unexpected fake-daemon method {other:?}")))
                        }
                    }
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                Message::Close(_) => return Ok(()),
                _ => return Err(error("unexpected fake-daemon frame")),
            }
        }
    }
    async fn capture(
        rx: &mut mpsc::UnboundedReceiver<std::result::Result<Value, String>>,
        sentinel: &str,
    ) -> Check<Value> {
        let capture = bounded(rx.recv())
            .await?
            .ok_or_else(|| error("capture channel closed"))?
            .map_err(error)?;
        let prompt = capture
            .pointer("/turnFrame/params/input/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| error("captured turn has no input text"))?;
        if !prompt.contains(sentinel) {
            return Err(error(format!("capture is not caused by {sentinel}")));
        }
        Ok(capture)
    }

    async fn http(request: reqwest::RequestBuilder, status: u16) -> Check<Value> {
        let response = bounded(request.send()).await??;
        let actual = response.status().as_u16();
        let headers = format!("{:?}", response.headers());
        let body = bounded(response.text()).await??;
        if actual != status {
            return Err(error(format!(
                "HTTP expected {status}, got {actual}; {headers}; {body}"
            )));
        }
        Ok(
            json!({"status":actual,"headers":headers,"body":serde_json::from_str::<Value>(&body)?,"rawBody":body}),
        )
    }

    async fn scenario(root: &Path, evidence: &Path) -> Check<Value> {
        for name in [
            "OMON_DEFAULT_MODEL",
            "DEFAULT_MODEL",
            "OMON_OMO_CRON_APPSERVER_URL",
            "DISCORD_BOT_TOKEN",
            "DISCORD_BOT_TOKENS",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
        ] {
            if std::env::var_os(name).is_some() {
                return Err(error(format!("{name} must be absent")));
            }
        }
        if std::env::current_dir()? != root
            || std::env::var_os("HOME").as_deref() != Some(root.as_os_str())
        {
            return Err(error("fixture cwd/HOME identity mismatch"));
        }
        if root.join("gateway.db").exists() {
            return Err(error("fixture database was not fresh"));
        }
        // Resolve both lanes using the same APIs and workspace binding as
        // run_standalone. This exact-test child never mutates its sanitized env.
        // Reject any endpoint outside this literal fixture BEFORE runtime/spawn.
        let workspace = PathBuf::from(
            std::env::var_os("OMON_WORKSPACE_ROOT")
                .ok_or_else(|| error("missing isolated workspace"))?,
        );
        omon_gateway::validate_agent_backend_env()?;
        let interactive_config =
            omon_gateway::OmoBackendConfig::from_env()?.with_workspace_root(workspace.clone());
        let cron_config =
            omon_gateway::OmoBackendConfig::cron_from_env()?.with_workspace_root(workspace);
        if interactive_config.appserver_url != URL || cron_config.appserver_url != URL {
            return Err(error(format!(
                "endpoint preflight blocked: interactive={}, cron={}; only {URL} may be owned; runtime/contact/spawn not started",
                interactive_config.appserver_url, cron_config.appserver_url
            )));
        }
        let interactive_listener = TcpListener::bind("127.0.0.1:29998").await.map_err(|e| {
            error(format!(
                "environment limitation: cannot own 29998 ({e}); runtime/contact/spawn not started"
            ))
        })?;
        let owned_url = format!("ws://{}", interactive_listener.local_addr()?);
        if interactive_config.appserver_url != owned_url || cron_config.appserver_url != owned_url {
            return Err(error(
                "effective endpoint is not owned; runtime/contact/spawn not started",
            ));
        }
        let (tx, mut rx) = mpsc::unbounded_channel();
        let peers_shutdown = CancellationToken::new();
        let probes = Arc::new(AtomicUsize::new(0));
        let mut peer_tasks = Vec::new();
        let peer = Peer {
            endpoint: format!("ws://{}", interactive_listener.local_addr()?),
            probes: probes.clone(),
            captures: tx.clone(),
            shutdown: peers_shutdown.clone(),
        };
        let app = Router::new()
            .route("/readyz", get(ready))
            .route("/", get(upgrade))
            .with_state(peer);
        let stop = peers_shutdown.clone();
        peer_tasks.push(tokio::spawn(async move {
            axum::serve(interactive_listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
        }));
        let (listening, heard) = oneshot::channel();
        let configs = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry()
                .with(ConstructionTrace {
                    listening: Mutex::new(Some(listening)),
                    configs: configs.clone(),
                })
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr)),
        )?;
        // Settings reject port 0. Release one owned ephemeral reservation once;
        // a bind collision is a setup failure, never a retry or an unowned HTTP request.
        let reservation = TcpListener::bind("127.0.0.1:0").await?;
        let address = reservation.local_addr()?;
        drop(reservation);
        let settings = crate::legacy::dashboard::DashboardSettings {
            enabled: true,
            host: "127.0.0.1".into(),
            port: address.port(),
            insecure: false,
            web_root: PathBuf::from(
                std::env::var_os("R_AG_15_WEB_ROOT").ok_or_else(|| error("missing web root"))?,
            ),
        };
        let shutdown = CancellationToken::new();
        let mut runtime = tokio::spawn(crate::legacy::dashboard_runtime::run_standalone(
            settings,
            shutdown.clone(),
            false,
        ));
        let outcome: Check<Value> = async {
            let bound = bounded(heard).await??;
            if bound != address.to_string() { return Err(error("runtime bound an unexpected address")); }
            let startup_probes = probes.load(Ordering::SeqCst);
            let base = format!("http://{address}");
            let client = reqwest::Client::builder().no_proxy().timeout(LIMIT).build()?;
            let config = http(client.get(format!("{base}/api/config")), 200).await?;
            let curl_exe = PathBuf::from(std::env::var_os("SystemRoot").ok_or_else(|| error("missing SystemRoot"))?)
                .join("System32/curl.exe");
            let mut curl = Command::new(curl_exe);
            curl.args(["--noproxy", "*", "--silent", "--show-error", "--max-time", "15", "-i"])
                .arg(format!("{base}/api/config"));
            let curl = owned_output(curl, Duration::from_secs(20)).await?;
            if !curl.status.success() { return Err(error(format!("curl config capture failed: {}", String::from_utf8_lossy(&curl.stderr)))); }
            let mut request = format!("ws://{address}/api/sessions/r-ag-15-interactive/ws").into_client_request()?;
            request.headers_mut().insert("Origin", base.parse()?);
            let (mut ws, _) = bounded(tokio_tungstenite::connect_async(request)).await??;
            let ready = bounded(ws.next()).await?.ok_or_else(|| error("session WS closed"))??;
            let ready: Value = serde_json::from_str(ready.to_text()?)?;
            if ready["type"] != "ready" { return Err(error("missing session subscription barrier")); }
            let chat = http(client.post(format!("{base}/api/sessions/r-ag-15-interactive/chat"))
                .json(&json!({"message":"R_AG_15_INTERACTIVE"})), 202).await?;
            let interactive = capture(&mut rx, "R_AG_15_INTERACTIVE").await?;
            // Exact completion event from the actual actor, subscribed before POST.
            bounded(async {
                loop {
                    let frame = ws.next().await.ok_or_else(|| error("session closed before completion"))??;
                    if frame.is_text() {
                        let frame: Value = serde_json::from_str(frame.to_text()?)?;
                        if frame["type"] == "event" && frame["event"]["type"] == "typing"
                            && frame["event"]["active"] == false { break; }
                    }
                }
                Check::Ok(())
            }).await??;
            bounded(ws.close(None)).await??;
            drop(ws);
            let messages = http(client.get(format!("{base}/api/sessions/r-ag-15-interactive/messages")), 200).await?;
            let created = http(client.post(format!("{base}/api/cron/jobs")).json(&json!({
                "id":"r-ag-15-cron", "expression":"0 * * * *", "enabled":false,
                "payload":{"prompt":"R_AG_15_CRON"}
            })), 201).await?;
            let trigger = http(client.post(format!("{base}/api/cron/jobs/r-ag-15-cron/trigger")), 200).await?;
            let cron = capture(&mut rx, "R_AG_15_CRON").await?;
            // OmoBackend sends its selected model on both thread/start and
            // turn/start. These optional protocol fields must be populated in
            // this default-model scenario; a turn override must not go unchecked.
            let dashboard_model = config.pointer("/body/model").and_then(Value::as_str)
                .filter(|model| !model.trim().is_empty());
            let model_comparisons: Vec<Value> = [("interactive", &interactive), ("cron", &cron)]
                .into_iter().map(|(lane, capture)| {
                    let thread = capture.pointer("/frame/params/model").and_then(Value::as_str);
                    let turn = capture.pointer("/turnFrame/params/model").and_then(Value::as_str);
                    json!({"lane":lane,"threadModel":thread,"turnModel":turn,
                        "dashboardModel":dashboard_model,
                        "matches":dashboard_model.is_some() && thread == dashboard_model && turn == dashboard_model})
                }).collect();
            let model_ok = model_comparisons.iter().all(|comparison| comparison["matches"] == true);
            let mut report = json!({
                "base":base,"owner":root.file_name().and_then(|s|s.to_str()),
                "modelVariablesAbsent":true,"cronOverrideAbsent":true,"dotenvEntryCalled":false,
                "config":config,"configCurlHttp":String::from_utf8_lossy(&curl.stdout),
                "chat":chat,"messages":messages,"createdCron":created,"trigger":trigger,
                "interactive":interactive,"cron":cron,
                "modelComparisons":model_comparisons,
                "endpointPreflight":{"interactive":interactive_config.appserver_url,
                    "cron":cron_config.appserver_url,"owned":owned_url},
                "constructionConfigs":configs.lock().unwrap().clone(),
                "startupReadyProbes":{"endpoint":URL,"count":startup_probes},
                "fallbackReadyProbes":{"endpoint":"ws://127.0.0.1:19742","count":null,
                    "status":"unobserved-not-selected-not-owned"},
                "daemonContract":"JSON-RPC initialize/thread/start succeed; captured turn/start is explicitly rejected before inference"
            });
            std::fs::write(root.join("wire.json"), serde_json::to_vec_pretty(&report)?)?;
            let script = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(".omo/evidence/review-20260908/implementation/DASH-011/browser.mjs");
            let supervisor = script.with_file_name("browser-job.ps1");
            let powershell = PathBuf::from(std::env::var_os("SystemRoot")
                .ok_or_else(|| error("missing SystemRoot"))?)
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let mut browser = Command::new(powershell);
            browser.env("R_AG_15_BROWSER_SCRIPT", &script);
            browser.env("BUN_CHROME_PATH", PathBuf::from(
                std::env::var_os("ProgramFiles(x86)")
                    .ok_or_else(|| error("missing ProgramFiles(x86) for fixture Edge"))?
            ).join("Microsoft/Edge/Application/msedge.exe"));
            browser.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(supervisor);
            // Preserve native verdicts if browser setup fails. A missing/invalid browser
            // observation is BLOCKED for the surface, never a synthetic product RED.
            match owned_output(browser, Duration::from_secs(90)).await {
                Ok(output) => {
                    report["browserSupervisorReaped"] = json!(true);
                    report["browserSupervisorExit"] = json!(output.status.code());
                    report["browserStdout"] = json!(String::from_utf8_lossy(&output.stdout));
                    report["browserStderr"] = json!(String::from_utf8_lossy(&output.stderr));
                    match std::fs::read(root.join("browser.json")).map_err(|e| error(e.to_string()))
                        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| error(e.to_string()))) {
                        Ok(observation) => report["browser"] = observation,
                        Err(failure) => report["browserSetupError"] = json!(failure.to_string()),
                    }
                }
                Err(failure) => {
                    report["browserSupervisorReaped"] = json!(false);
                    report["browserSetupError"] = json!(failure.to_string());
                }
            }
            let job_path = evidence.join("DASH-011-SURFACE/browser-job.json");
            match std::fs::read(&job_path).map_err(|e| error(e.to_string()))
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| error(e.to_string()))) {
                Ok(job) => report["browserJob"] = job,
                Err(failure) => report["browserJobReceiptError"] = json!(failure.to_string()),
            }
            let job = &report["browserJob"];
            let tree_exited = report["browserSupervisorReaped"] == true
                && job["ownerRoot"] == json!(root)
                && job["assignedBeforeResume"] == true
                && job["activeZeroObserved"] == true && job["activeCountZero"] == true
                && job["primaryReaped"] == true && job["treeExited"] == true;
            report["browserTreeExited"] = json!(tree_exited);
            report["browserProcessReaped"] = report["browserJob"]["primaryReaped"].clone();
            report["browserExit"] = report["browserJob"]["bunExit"].clone();
            report["finalReadyProbes"] = json!({"endpoint":URL,"count":probes.load(Ordering::SeqCst)});
            // This counts real /readyz requests on the only selected endpoint.
            // A second ensure to that endpoint produces a second request. No
            // assertion claims zero traffic at the unowned historical fallback.
            let one_ensure = startup_probes == 1 && report["finalReadyProbes"]["count"] == 1;
            let construction = report["constructionConfigs"].as_array().ok_or_else(|| error("missing construction events"))?;
            let endpoints_ok = construction.len() == 2 && construction.iter().all(|cfg| cfg["appserver_url"] == URL);
            report["verdicts"] = json!({
                "DASH-011":model_ok,
                "DASH-011-SURFACE":if tree_exited
                    && report["browserJob"]["errors"] == json!([])
                    && report["browserJob"]["deadline"] == false
                    && matches!(report["browserSupervisorExit"].as_i64(), Some(0 | 1)) {
                    report["browser"]["verdict"].clone()
                } else { Value::Null },
                "DASH-012":endpoints_ok && one_ensure,
                "DASH-012-SURFACE":interactive["endpoint"] == URL && cron["endpoint"] == URL && one_ensure
            });
            Ok(report)
        }.await;
        // Always clean up before returning either assertion data or an infrastructure error.
        shutdown.cancel();
        let runtime_cleanup = match tokio::time::timeout(LIMIT, &mut runtime).await {
            Ok(joined) => joined
                .map_err(|e| error(e.to_string()))
                .and_then(|r| r.map_err(|e| error(e.to_string()))),
            Err(failure) => {
                runtime.abort();
                let reaped = runtime.await;
                Err(error(format!(
                    "runtime shutdown deadline: {failure}; aborted join={reaped:?}"
                )))
            }
        };
        peers_shutdown.cancel();
        let mut peer_errors = Vec::new();
        for mut task in peer_tasks {
            match tokio::time::timeout(LIMIT, &mut task).await {
                Ok(Ok(Ok(()))) => {}
                Ok(other) => peer_errors.push(format!("peer join: {other:?}")),
                Err(failure) => {
                    task.abort();
                    peer_errors.push(format!("peer deadline: {failure}; join={:?}", task.await));
                }
            }
        }
        runtime_cleanup?;
        if !peer_errors.is_empty() {
            return Err(error(peer_errors.join("; ")));
        }
        let mut report = outcome?;
        match rx.try_recv() {
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {}
            Ok(extra) => {
                return Err(error(format!(
                    "unexpected extra daemon capture/failure: {extra:?}"
                )))
            }
        }
        report["runtimeAndPeersJoined"] = json!(true);
        Ok(report)
    }

    #[tokio::test]
    async fn dash_011_012_runtime_and_surface() -> Check<()> {
        if let Some(root) = std::env::var_os("R_AG_15_CHILD") {
            let evidence = PathBuf::from(
                std::env::var_os("R_AG_15_EVIDENCE")
                    .ok_or_else(|| error("missing evidence path"))?,
            );
            let report = scenario(Path::new(&root), &evidence).await?;
            println!("R_AG_15_RESULT={}", report);
            return if IDS.iter().all(|id| report["verdicts"][*id] == true) {
                Ok(())
            } else {
                Err(error(
                    "one or more R-AG-15 criteria failed; see separate verdicts",
                ))
            };
        }
        if !cfg!(windows) {
            eprintln!("skipping Windows-only lifecycle driver on non-Windows platform");
            return Ok(());
        }
        let bun = std::fs::canonicalize(
            std::env::var_os("R_AG_15_BUN").ok_or_else(|| error("set absolute R_AG_15_BUN"))?,
        )?;
        let web =
            std::fs::canonicalize(std::env::var_os("R_AG_15_WEB_ROOT").ok_or_else(|| {
                error("set R_AG_15_WEB_ROOT to the fingerprinted prebuilt web root")
            })?)?;
        if !web.join("index.html").is_file() {
            return Err(error("built mounted App index is missing"));
        }
        let evidence = PathBuf::from(
            std::env::var_os("R_AG_15_EVIDENCE")
                .ok_or_else(|| error("set a fresh absolute R_AG_15_EVIDENCE directory"))?,
        );
        if !evidence.is_absolute() {
            return Err(error("evidence path must be absolute"));
        }
        std::fs::create_dir(&evidence)?; // Refuse to overwrite another run.
        for id in IDS {
            std::fs::create_dir(evidence.join(id))?;
        }
        let root = tempfile::Builder::new().prefix("r-ag-15-").tempdir()?;
        for dir in [
            "tmp",
            "appdata",
            "localappdata",
            "config",
            "cache",
            "data",
            "hermes",
            "workspace",
            ".omo",
        ] {
            std::fs::create_dir(root.path().join(dir))?;
        }
        std::fs::write(root.path().join(".omo/omo.json"), b"{}\n")?;
        // Disable TempDir's implicit deletion BEFORE any child can run. Every
        // return path without an owned-tree exit receipt must retain this root.
        let root_path = root.keep();
        let mut child = Command::new(std::env::current_exe()?);
        child
            .arg(TEST)
            .args(["--exact", "--nocapture", "--test-threads=1"])
            .env_clear()
            .current_dir(&root_path);
        for name in [
            "SystemRoot",
            "WINDIR",
            "ComSpec",
            "PATHEXT",
            "SystemDrive",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
        ] {
            if let Some(value) = std::env::var_os(name) {
                child.env(name, value);
            }
        }
        for name in ["HOME", "USERPROFILE", "OMON_RUNTIME_LOCK_DIR"] {
            child.env(name, &root_path);
        }
        for (name, dir) in [
            ("TEMP", "tmp"),
            ("TMP", "tmp"),
            ("APPDATA", "appdata"),
            ("LOCALAPPDATA", "localappdata"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_DATA_HOME", "data"),
            ("HERMES_HOME", "hermes"),
            ("OMON_WORKSPACE_ROOT", "workspace"),
        ] {
            child.env(name, root_path.join(dir));
        }
        child
            .env("R_AG_15_CHILD", &root_path)
            .env("R_AG_15_BUN", bun)
            .env("R_AG_15_WEB_ROOT", web)
            .env("R_AG_15_EVIDENCE", &evidence)
            .env(
                "DATABASE_URL",
                format!("sqlite://{}", root_path.join("gateway.db").display()).replace('\\', "/"),
            )
            .env("OMON_AGENT_BACKEND", "omo")
            .env("OMON_OMO_APPSERVER_URL", URL)
            .env("OMON_OMO_AUTOSPAWN", "on")
            .env(
                "OMON_OMO_BIN",
                root_path.join("never-spawn-a-real-daemon.exe"),
            );
        let output = owned_output(child, Duration::from_secs(150)).await;
        let child_reaped = output.is_ok();
        // Read the supervisor receipt independently of the runtime report. A
        // runtime error must not erase a successfully observed browser teardown.
        let job_receipt: Check<Value> =
            std::fs::read(evidence.join("DASH-011-SURFACE/browser-job.json"))
                .map_err(|e| error(e.to_string()))
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| error(e.to_string())));
        let browser_cleanup = child_reaped
            && job_receipt.as_ref().is_ok_and(|job| {
                job["ownerRoot"] == json!(&root_path)
                    && job["treeExited"] == true
                    && ((job["assignedBeforeResume"] == true
                        && job["activeZeroObserved"] == true
                        && job["activeCountZero"] == true
                        && job["primaryReaped"] == true)
                        || job["processCreated"] == false
                        || (job["assignedBeforeResume"] == false
                            && job["resumed"] == false
                            && job["primaryReaped"] == true))
            });
        let removed: Check<()> = if browser_cleanup {
            std::fs::remove_dir_all(&root_path).map_err(|e| error(e.to_string()))
        } else {
            Err(error(format!(
                "owned root retained: no certified browser-tree exit; receipt error={:?}",
                job_receipt.as_ref().err().map(ToString::to_string)
            )))
        };
        let root_removed = removed.is_ok() && !root_path.exists();
        let (status, transcript) = match output {
            Ok(output) => (
                output.status.success(),
                format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ),
            ),
            Err(failure) => (false, failure.to_string()),
        };
        let report = transcript
            .lines()
            .find_map(|line| line.split_once("R_AG_15_RESULT=").map(|(_, json)| json))
            .map(serde_json::from_str::<Value>)
            .transpose()?;
        // Bind-only release checks: never issue traffic or act on a listener that
        // may have acquired the address after this owned process terminated.
        let mut release_checks = Vec::new();
        if let Some(report) = &report {
            let address = report["base"]
                .as_str()
                .and_then(|base| base.strip_prefix("http://"))
                .ok_or_else(|| error("missing owned runtime address"))?;
            for address in ["127.0.0.1:29998", address] {
                let bound = std::net::TcpListener::bind(address);
                release_checks.push(json!({"address":address,"released":bound.is_ok(),
                    "error":bound.as_ref().err().map(ToString::to_string)}));
            }
        }
        let browser_observation_valid = browser_cleanup
            && report.as_ref().is_some_and(|r| {
                r["browserTreeExited"] == true && r["verdicts"]["DASH-011-SURFACE"].is_boolean()
            });
        let runtime_cleanup = child_reaped
            && root_removed
            && report
                .as_ref()
                .is_some_and(|r| r["runtimeAndPeersJoined"] == true)
            && release_checks.iter().all(|check| check["released"] == true);
        let cleanup = runtime_cleanup && browser_cleanup;
        for id in IDS {
            let pass = cleanup && status;
            // A different criterion's RED exit must not erase a successful independent observation.
            let observed = report.as_ref().and_then(|r| r["verdicts"][id].as_bool());
            let verdict = if !cleanup || observed.is_none() {
                "blocked"
            } else if observed == Some(true) {
                "pass"
            } else {
                "fail"
            };
            let receipt = json!({
                "criterionId":id,"ownerUnitId":"R-AG-15","verdict":verdict,
                "allCriteriaCommandSucceeded":pass,"fixtureCommandSucceeded":status,
                "cleanup":{"childReaped":child_reaped,"ownedRootRemoved":root_removed,
                    "root":root_path,"runtimeLifecycleCertified":runtime_cleanup,
                    "browserLifecycleCertified":browser_cleanup,"bindOnlyReleaseChecks":release_checks},
                "browserObservationValid":browser_observation_valid,
                "report":report,"cleanupError":removed.as_ref().err().map(ToString::to_string),
                "browserJob":job_receipt.as_ref().ok(),
                "browserJobReceiptError":job_receipt.as_ref().err().map(ToString::to_string)
            });
            std::fs::write(
                evidence.join(id).join("result.json"),
                serde_json::to_vec_pretty(&receipt)?,
            )?;
            std::fs::write(evidence.join(id).join("transcript.txt"), &transcript)?;
        }
        println!("{transcript}\nR_AG_15_EVIDENCE={}", evidence.display());
        removed?;
        if !cleanup {
            return Err(error(
                "R-AG-15 cleanup could not be certified; see receipts",
            ));
        }
        if !status {
            return Err(error(
                "R-AG-15 fixture failed; inspect per-criterion receipts",
            ));
        }
        if report.is_none() {
            return Err(error("child exited without an observation report"));
        }
        Ok(())
    }
}

mod r_dash_001 {
    use crate::legacy::dashboard::ChatRequest;

    #[tokio::test]
    async fn dash_001_chat_protocol_ingress_and_contract() {
        // 1. Verify Rust-side ChatRequest accepts both "message" and "content" alias
        let req1: ChatRequest = serde_json::from_str(r#"{"message":"hello"}"#).unwrap();
        assert_eq!(req1.message, "hello");
        let req2: ChatRequest = serde_json::from_str(r#"{"content":"hello"}"#).unwrap();
        assert_eq!(req2.message, "hello");

        // 2. Run handlerProbeSource machine contract assertions
        let output = std::process::Command::new("node")
            .args(["-e", "eval(JSON.parse(require('fs').readFileSync('.omo/evidence/review-20260908/dashboard.json','utf8')).handlerProbeSource)"])
            .current_dir(std::env::current_dir().unwrap())
            .output()
            .expect("failed to execute handlerProbeSource node command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("handlerProbeSource stdout:\n{stdout}");
        assert!(
            stdout.contains("DASH-001-ws-input GREEN"),
            "DASH-001-ws-input must be GREEN"
        );
        assert!(
            stdout.contains("DASH-001-ws-final GREEN"),
            "DASH-001-ws-final must be GREEN"
        );
        assert!(
            stdout.contains("DASH-001-http GREEN"),
            "DASH-001-http must be GREEN"
        );
    }
}

mod r_dash_002 {
    #[tokio::test]
    async fn dash_002_live_logs_envelope_decoding_contract() {
        let output = std::process::Command::new("node")
            .args(["-e", "eval(JSON.parse(require('fs').readFileSync('.omo/evidence/review-20260908/dashboard.json','utf8')).handlerProbeSource)"])
            .current_dir(std::env::current_dir().unwrap())
            .output()
            .expect("failed to execute handlerProbeSource node command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("handlerProbeSource stdout:\n{stdout}");
        assert!(stdout.contains("DASH-002 GREEN"), "DASH-002 must be GREEN");
    }
}

mod r_dash_003 {
    #[tokio::test]
    async fn dash_003_session_search_parameter_alignment_contract() {
        let output = std::process::Command::new("node")
            .args(["-e", "eval(JSON.parse(require('fs').readFileSync('.omo/evidence/review-20260908/dashboard.json','utf8')).handlerProbeSource)"])
            .current_dir(std::env::current_dir().unwrap())
            .output()
            .expect("failed to execute handlerProbeSource node command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        println!("handlerProbeSource stdout:\n{stdout}");
        assert!(stdout.contains("DASH-003 GREEN"), "DASH-003 must be GREEN");
    }
}
