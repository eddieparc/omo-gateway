use std::any::Any;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use futures_util::{FutureExt, SinkExt, StreamExt};
use omon_gateway::discord::adapter::{
    message_to_inbound_with_config, route_claimed_event, InboundFilterConfig,
};
use omon_gateway::discord::commands::{self, CommandError, PoiseData};
use omon_gateway::{
    AgentBackend, Database, InboundEvent, MultiplexerConfig, OmoBackend, OmoBackendConfig,
    OutboundAction, OutboundDispatcher, SessionContext, SessionKey, SessionMultiplexer,
};
use serde_json::{json, Value};
use serenity::all::{
    Channel, ChannelId, ChannelType, CommandInteraction, GatewayIntents, GuildChannel, GuildId,
    HttpBuilder, Member, Message, MessageId, ShardId, ShardInfo, User, UserId, UserUpdateEvent,
};
use serenity::gateway::{Shard, ShardManager, ShardManagerOptions, ShardMessenger};
use serenity::gateway::{ShardRunner, ShardRunnerOptions};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const EVENT_BOUND: Duration = Duration::from_secs(5);
const CLEANUP_BOUND: Duration = Duration::from_secs(10);
const STARTED: &str = "AG04_WORK_STARTED";
const CALLBACK_PATH: &str = "/api/v10/interactions/500/ag04-fixture/callback";

type RestTrace = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Clone, Copy)]
struct ThreadStopFixture {
    guild_id: u64,
    parent_id: u64,
    thread_id: u64,
    user_id: u64,
    bot_id: u64,
    content: &'static str,
}

const AG04: ThreadStopFixture = ThreadStopFixture {
    guild_id: 100,
    parent_id: 200,
    thread_id: 300,
    user_id: 42,
    bot_id: 84,
    content: "<@84> hold this turn",
};

const DIS001: ThreadStopFixture = ThreadStopFixture {
    guild_id: 9,
    parent_id: 7,
    thread_id: 8,
    user_id: 10,
    bot_id: 42,
    content: "<@42> work",
};

#[derive(Default)]
struct PeerState {
    active: bool,
    starts: usize,
    interrupts: Vec<Value>,
}

struct StartedDispatcher {
    started: Mutex<Option<oneshot::Sender<SessionKey>>>,
}

#[async_trait]
impl OutboundDispatcher for StartedDispatcher {
    async fn dispatch(&self, action: OutboundAction) -> omon_gateway::Result<()> {
        if let OutboundAction::Stream { session, chunk } = action {
            if !chunk.is_final && chunk.content == STARTED {
                self.started
                    .lock()
                    .await
                    .take()
                    .expect("exactly one backend start signal")
                    .send(session)
                    .expect("start subscriber remains installed");
            }
        }
        Ok(())
    }
}

// Forward all execution and cancellation to the production backend. The drop
// receipt lets cleanup await release of both mux and actor ownership.
struct TrackedBackend {
    backend: OmoBackend,
    retired: Option<oneshot::Sender<()>>,
}

#[async_trait]
impl AgentBackend for TrackedBackend {
    async fn run(
        &self,
        session: &mut SessionContext,
        event: InboundEvent,
    ) -> omon_gateway::Result<()> {
        self.backend.run(session, event).await
    }

    async fn cancel(&self, session: &SessionContext) -> omon_gateway::Result<()> {
        self.backend.cancel(session).await
    }
}

impl Drop for TrackedBackend {
    fn drop(&mut self) {
        if let Some(sender) = self.retired.take() {
            if sender.send(()).is_err() {
                eprintln!("AG-04 cleanup: backend retirement subscriber was dropped");
            }
        }
    }
}

async fn discord_rest(
    State((trace, fixture, case)): State<(RestTrace, ThreadStopFixture, &'static str)>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let payload = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).expect("Discord REST JSON")
    };
    let request = format!("{method} {}", uri.path());
    eprintln!("{case} REST {request} {payload}");
    trace.lock().await.push((request, payload.clone()));

    match (method.as_str(), uri.path()) {
        ("GET", path) if path == format!("/api/v10/channels/{}", fixture.thread_id) => {
            let mut channel = GuildChannel::default();
            channel.id = ChannelId::new(fixture.thread_id);
            channel.guild_id = GuildId::new(fixture.guild_id);
            channel.parent_id = Some(ChannelId::new(fixture.parent_id));
            channel.kind = ChannelType::PublicThread;
            channel.name = "ag04-thread".into();
            Json(channel).into_response()
        }
        ("GET", path) if path == format!("/api/v10/channels/{}", fixture.parent_id) => {
            let mut channel = GuildChannel::default();
            channel.id = ChannelId::new(fixture.parent_id);
            channel.guild_id = GuildId::new(fixture.guild_id);
            channel.kind = ChannelType::Text;
            channel.name = "ag04-parent".into();
            Json(channel).into_response()
        }
        ("POST", path) if path == format!("/api/v10/channels/{}/threads", fixture.parent_id) => {
            let mut channel = GuildChannel::default();
            channel.id = ChannelId::new(fixture.thread_id);
            channel.guild_id = GuildId::new(fixture.guild_id);
            channel.parent_id = Some(ChannelId::new(fixture.parent_id));
            // CreateThread leaves type unset; Discord's standalone default is private.
            channel.kind = ChannelType::PrivateThread;
            channel.name = payload["name"]
                .as_str()
                .expect("create-thread name")
                .to_owned();
            (StatusCode::CREATED, Json(channel)).into_response()
        }
        ("POST", CALLBACK_PATH) => StatusCode::NO_CONTENT.into_response(),
        _ => (
            StatusCode::BAD_REQUEST,
            Json(json!({"code": 0, "message": "unexpected fixture REST request"})),
        )
            .into_response(),
    }
}

async fn appserver_peer(
    listener: TcpListener,
    state: Arc<Mutex<PeerState>>,
    expected_prompt: String,
    case: &'static str,
    create_thread: bool,
) {
    loop {
        let (socket, _) = listener.accept().await.expect("appserver accept");
        let mut ws = tokio_tungstenite::accept_async(socket)
            .await
            .expect("appserver WebSocket handshake");
        while let Some(frame) = ws.next().await {
            let frame = match frame {
                Ok(frame) => frame,
                Err(error) => {
                    // Dropping the run may reset TCP without a close handshake.
                    // The accepted work survives this connection.
                    eprintln!("{case} appserver transport closed: {error}");
                    break;
                }
            };
            let text = match frame {
                WsMessage::Text(text) => text,
                WsMessage::Close(_) => break,
                WsMessage::Ping(bytes) => {
                    ws.send(WsMessage::Pong(bytes)).await.expect("pong");
                    continue;
                }
                other => panic!("unexpected appserver frame: {other:?}"),
            };
            let request: Value = serde_json::from_str(&text).expect("appserver JSON");
            eprintln!("{case} RPC {request}");
            let result = match request["method"].as_str() {
                Some("initialize") => json!({}),
                Some("thread/start") => json!({"thread": {"id": "r1"}}),
                Some("thread/resume") => {
                    assert_eq!(request["params"]["threadId"], "r1");
                    json!({"thread": {"id": "r1"}})
                }
                Some("turn/start") => {
                    assert_eq!(request["params"]["threadId"], "r1");
                    if create_thread {
                        // The callback creates its own timestamped InboundEvent.
                        // Check the supplied starter without depending on wall-clock time.
                        let input = request["params"]["input"].as_array().expect("turn input");
                        assert_eq!(input.len(), 1);
                        assert_eq!(input[0]["type"], "text");
                        assert_eq!(
                            omon_gateway::models::strip_leading_message_timestamps(
                                input[0]["text"].as_str().expect("starter text")
                            ),
                            expected_prompt
                        );
                    } else {
                        assert_eq!(
                            request["params"]["input"],
                            json!([{"type": "text", "text": expected_prompt}])
                        );
                    }
                    {
                        let mut state = state.lock().await;
                        state.starts += 1;
                        assert_eq!(state.starts, 1, "accepted work must not be resubmitted");
                        state.active = true;
                    }
                    ws.send(WsMessage::text(
                        json!({
                            "jsonrpc": "2.0", "id": request["id"],
                            "result": {"turn": {"id": "t1", "status": "inProgress"}}
                        })
                        .to_string(),
                    ))
                    .await
                    .expect("start acknowledgement");
                    ws.send(WsMessage::text(
                        json!({
                            "jsonrpc": "2.0", "method": "item/agentMessage/delta",
                            "params": {
                                "threadId": "r1", "turnId": "t1",
                                "itemId": "m1", "delta": STARTED
                            }
                        })
                        .to_string(),
                    ))
                    .await
                    .expect("post-ACK start signal");
                    // No terminal output is produced while work is held.
                    continue;
                }
                Some("turn/interrupt") => {
                    assert_eq!(request["params"], json!({"threadId": "r1", "turnId": "t1"}));
                    let mut state = state.lock().await;
                    assert!(state.active, "interrupt must address the held turn");
                    state.interrupts.push(request["params"].clone());
                    state.active = false;
                    json!({})
                }
                _ => panic!("unexpected appserver request: {request}"),
            };
            ws.send(WsMessage::text(
                json!({"jsonrpc": "2.0", "id": request["id"], "result": result}).to_string(),
            ))
            .await
            .expect("appserver acknowledgement");
        }
    }
}

#[test]
fn ag04_registered_thread_stop_interrupts_message_lane() {
    exercise_registered_thread_stop("AG-04", AG04);
}

#[test]
fn ag04_registered_thread_stop_surface() {
    exercise_registered_thread_stop("AG-04-SURFACE", AG04);
}

#[test]
fn dis001_registered_thread_stop_interrupts_message_lane() {
    exercise_registered_thread_stop("DIS-001", DIS001);
}

#[test]
fn dis001_registered_thread_stop_surface() {
    exercise_registered_thread_stop("DIS-001-SURFACE", DIS001);
}

fn exercise_registered_thread_stop(case: &'static str, fixture: ThreadStopFixture) {
    exercise_registered_thread_command(case, fixture, false);
}

#[test]
fn ag04_registered_thread_starter_uses_parent_lane() {
    exercise_registered_thread_command(
        "AG-04-THREAD-STARTER",
        ThreadStopFixture {
            content: "THREAD_STARTER_SENTINEL",
            ..AG04
        },
        true,
    );
}

fn exercise_registered_thread_command(
    case: &'static str,
    fixture: ThreadStopFixture,
    create_thread: bool,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("isolated fixture runtime");
    let (outcome, retired, peer_errors, closed) = runtime.block_on(async {
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("isolated migrated database");
        let mut tasks = JoinSet::new();
        let mut owned_mux = None;
        let (retired_tx, retired_rx) = oneshot::channel();

        let outcome = AssertUnwindSafe(async {
            let trace: RestTrace = Arc::new(Mutex::new(Vec::new()));
            let rest_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let rest_address = rest_listener.local_addr().unwrap();
            let app = Router::new()
                .fallback(discord_rest)
                .with_state((trace.clone(), fixture, case));
            tasks.spawn(async move {
                axum::serve(rest_listener, app).await.expect("loopback REST server");
            });
            let http = Arc::new(
                HttpBuilder::new("ag04-not-a-real-token")
                    .client(
                        reqwest::Client::builder()
                            .no_proxy()
                            .redirect(reqwest::redirect::Policy::none())
                            .timeout(EVENT_BOUND)
                            .build()
                            .unwrap(),
                    )
                    .proxy(format!("http://{rest_address}"))
                    .ratelimiter_disabled(true)
                    .build(),
            );

            let mut bot = User::default();
            bot.id = UserId::new(fixture.bot_id);
            bot.name = "ag04-bot".into();
            bot.bot = true;
            let mut author = User::default();
            author.id = UserId::new(fixture.user_id);
            author.name = "ag04-user".into();
            let mut message = Message::default();
            message.id = MessageId::new(400);
            message.channel_id = ChannelId::new(fixture.thread_id);
            message.guild_id = Some(GuildId::new(fixture.guild_id));
            message.author = author.clone();
            message.content = fixture.content.into();
            message.mentions = vec![bot.clone()];

            let event = if create_thread {
                // Only the registered /thread callback may create and route
                // this case's starter. Do not prefetch or enter the child lane.
                None
            } else {
                // Use the real ingress conversion with metadata fetched through
                // Serenity, then the real claim/admission/multiplexer path.
                let Channel::Guild(channel) = timeout(
                    EVENT_BOUND,
                    message.channel_id.to_channel(&http),
                )
                .await
                .expect("bounded ingress metadata")
                .expect("ingress thread metadata")
                else {
                    panic!("expected guild thread metadata");
                };
                let event = message_to_inbound_with_config(
                    &message,
                    bot.id,
                    Some(channel.kind),
                    &InboundFilterConfig {
                        allowed_users: &[fixture.user_id],
                        allowed_channels: &[fixture.parent_id],
                        primary_bot_id: Some(fixture.bot_id),
                        parent_channel_id: channel.parent_id.map(|id| id.get()),
                        ..Default::default()
                    },
                )
                .expect("authorized thread message admitted");
                let key = event.session.clone();
                assert_eq!(key.guild_id, Some(fixture.guild_id.to_string()));
                assert_eq!(key.channel_id, fixture.parent_id.to_string());
                assert_eq!(key.thread_id, Some(fixture.thread_id.to_string()));
                assert_eq!(key.bot_id, Some(fixture.bot_id.to_string()));
                Some(event)
            };

            let peer_state = Arc::new(Mutex::new(PeerState::default()));
            let backend_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let backend_address = backend_listener.local_addr().unwrap();
            tasks.spawn(appserver_peer(
                backend_listener,
                peer_state.clone(),
                event
                    .as_ref()
                    .map(omon_gateway::render_user_prompt)
                    .unwrap_or_else(|| fixture.content.to_owned()),
                case,
                create_thread,
            ));
            // Subscribe before ingress. A backend-delivered delta, not peer
            // receipt alone, proves the acknowledged turn is now running.
            let (started_tx, started_rx) = oneshot::channel();
            let dispatcher = Arc::new(StartedDispatcher {
                started: Mutex::new(Some(started_tx)),
            });
            let backend = TrackedBackend {
                backend: OmoBackend::new(
                    OmoBackendConfig::new(format!("ws://{backend_address}"))
                        .with_connect_timeout(EVENT_BOUND)
                        .with_request_timeout(EVENT_BOUND)
                        .with_per_agent_workspace(false),
                    dispatcher.clone(),
                )
                .with_pool(database.pool().clone()),
                retired: Some(retired_tx),
            };
            let mux = SessionMultiplexer::with_dispatcher(
                database.pool().clone(),
                Arc::new(backend),
                Some(dispatcher),
                MultiplexerConfig::default(),
            );
            owned_mux = Some(mux.clone());
            let mut data = PoiseData::new(mux, database.pool().clone());
            data.allowed_users = vec![fixture.user_id];
            data.allowed_channels = vec![fixture.parent_id];
            data.primary_bot_id = Some(fixture.bot_id);

            // Context requires a ShardMessenger. Construct its public runner
            // against a passive loopback socket, without starting a gateway
            // event loop or sending Discord IDENTIFY/registration requests.
            let gateway_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let gateway_url = format!("ws://{}", gateway_listener.local_addr().unwrap());
            tasks.spawn(async move {
                let (socket, _) = gateway_listener.accept().await.unwrap();
                let ws = tokio_tungstenite::accept_async(socket).await.unwrap();
                std::future::pending::<()>().await;
                drop(ws);
            });
            let ws_url = Arc::new(Mutex::new(gateway_url));
            let cache = Arc::new(serenity::cache::Cache::new());
            let mut user_update: UserUpdateEvent =
                serde_json::from_value(serde_json::to_value(&bot).unwrap()).unwrap();
            let _previous_user = cache.update(&mut user_update);
            let typemap = Arc::new(RwLock::new(serenity::prelude::TypeMap::new()));
            let (manager, _manager_result) = ShardManager::new(ShardManagerOptions {
                data: typemap.clone(),
                event_handlers: Vec::new(),
                raw_event_handlers: Vec::new(),
                framework: Arc::new(std::sync::OnceLock::new()),
                shard_index: 0,
                shard_init: 0,
                shard_total: 1,
                voice_manager: None,
                ws_url: ws_url.clone(),
                cache: cache.clone(),
                http: http.clone(),
                intents: GatewayIntents::empty(),
                presence: None,
            });
            let shard = timeout(
                EVENT_BOUND,
                Shard::new(
                    ws_url,
                    "ag04-not-a-real-token",
                    ShardInfo { id: ShardId(0), total: 1 },
                    GatewayIntents::empty(),
                    None,
                ),
            )
            .await
            .expect("bounded loopback shard construction")
            .expect("loopback shard");
            let runner = ShardRunner::new(ShardRunnerOptions {
                data: typemap.clone(),
                event_handlers: Vec::new(),
                raw_event_handlers: Vec::new(),
                framework: None,
                manager: manager.clone(),
                shard,
                voice_manager: None,
                cache: cache.clone(),
                http: http.clone(),
            });
            let context = serenity::client::Context {
                data: typemap,
                shard: ShardMessenger::new(&runner),
                shard_id: ShardId(0),
                http,
                cache,
            };
            let options = poise::FrameworkOptions::<PoiseData, CommandError> {
                commands: commands::all(),
                command_check: Some(|ctx| Box::pin(commands::command_check(ctx))),
                ..Default::default()
            };
            if create_thread {
                assert_eq!(
                    options.commands.iter().filter(|command| command.name == "thread").count(),
                    1
                );
                let mut member = Member::default();
                member.user = author;
                member.guild_id = GuildId::new(fixture.guild_id);
                let interaction: CommandInteraction = serde_json::from_value(json!({
                    "id": "500", "application_id": fixture.bot_id.to_string(), "type": 2,
                    "data": {
                        "id": "501", "name": "thread", "type": 1,
                        "options": [
                            {"name": "name", "type": 3, "value": "ag04-created"},
                            {"name": "message", "type": 3, "value": fixture.content}
                        ]
                    },
                    "guild_id": fixture.guild_id.to_string(),
                    "channel_id": fixture.parent_id.to_string(), "member": member,
                    "token": "ag04-fixture", "version": 1, "locale": "en-US",
                    "guild_locale": "en-US", "app_permissions": "0",
                    "entitlements": [], "attachment_size_limit": 10485760
                }))
                .expect("thread creation interaction");
                assert_eq!(interaction.user.id, UserId::new(fixture.user_id));

                let framework = poise::FrameworkContext {
                    bot_id: bot.id,
                    options: &options,
                    user_data: &data,
                    shard_manager: &manager,
                };
                let responded = AtomicBool::new(false);
                let invocation_data: Mutex<Box<dyn Any + Send + Sync>> =
                    Mutex::new(Box::new(()));
                let arguments = interaction.data.options();
                let mut parents = Vec::new();
                timeout(
                    EVENT_BOUND,
                    poise::dispatch::dispatch_interaction(
                        framework,
                        &context,
                        &interaction,
                        &responded,
                        &invocation_data,
                        &arguments,
                        &mut parents,
                    ),
                )
                .await
                .expect("bounded registered thread dispatch")
                .unwrap_or_else(|error| panic!("registered thread callback failed: {error}"));

                let started_key = timeout(EVENT_BOUND, started_rx)
                    .await
                    .expect("bounded starter backend event")
                    .expect("starter reached production backend");
                let state = peer_state.lock().await;
                eprintln!(
                    "{case} starter_key={started_key} expected_parent={} expected_child={} starts={} interrupts={:?} remote_active={}",
                    fixture.parent_id, fixture.thread_id,
                    state.starts, state.interrupts, state.active
                );
                assert_eq!(state.starts, 1);
                assert!(state.active);
                assert!(state.interrupts.is_empty());
                drop(state);

                let requests = trace.lock().await;
                let create_path =
                    format!("POST /api/v10/channels/{}/threads", fixture.parent_id);
                let creations: Vec<_> = requests
                    .iter()
                    .enumerate()
                    .filter(|(_, (path, _))| path == &create_path)
                    .collect();
                assert_eq!(creations.len(), 1);
                assert_eq!(creations[0].1.1["name"], "ag04-created");
                let callbacks: Vec<_> = requests
                    .iter()
                    .enumerate()
                    .filter(|(_, (path, _))| path == &format!("POST {CALLBACK_PATH}"))
                    .collect();
                assert_eq!(callbacks.len(), 1);
                assert_eq!(callbacks[0].1.1["type"], 4);
                assert!(creations[0].0 < callbacks[0].0);
                drop(requests);

                // Assert the actual backend lane before any cleanup cancellation.
                assert_eq!(started_key.platform, "discord");
                assert_eq!(started_key.guild_id, Some(fixture.guild_id.to_string()));
                assert_eq!(
                    started_key.channel_id,
                    fixture.parent_id.to_string(),
                    "registered /thread starter must retain the creation parent"
                );
                assert_eq!(started_key.thread_id, Some(fixture.thread_id.to_string()));
                assert_eq!(started_key.bot_id, Some(fixture.bot_id.to_string()));
                assert!(started_key.user_id.is_empty());
                assert!(data.multiplexer.contains_session(&started_key));
                let stored: Vec<String> =
                    sqlx::query_scalar("SELECT session_key FROM sessions ORDER BY session_key")
                        .fetch_all(database.pool())
                        .await
                        .unwrap();
                assert_eq!(stored, vec![started_key.storage_key()]);
                return;
            }

            let event = event.expect("stop case has an admitted message");
            let key = event.session.clone();
            assert_eq!(
                options.commands.iter().filter(|command| command.name == "stop").count(),
                1
            );
            let mut member = Member::default();
            member.user = author;
            member.guild_id = GuildId::new(fixture.guild_id);
            let interaction: CommandInteraction = serde_json::from_value(json!({
                "id": "500", "application_id": fixture.bot_id.to_string(), "type": 2,
                "data": {"id": "501", "name": "stop", "type": 1, "options": []},
                "guild_id": fixture.guild_id.to_string(),
                "channel_id": fixture.thread_id.to_string(), "member": member,
                "token": "ag04-fixture", "version": 1, "locale": "en-US",
                "guild_locale": "en-US", "app_permissions": "0",
                "entitlements": [], "attachment_size_limit": 10485760
            }))
            .expect("slash interaction");
            assert_eq!(interaction.user.id, UserId::new(fixture.user_id));

            assert!(
                timeout(EVENT_BOUND, route_claimed_event(&data, event))
                    .await
                    .expect("bounded ingress")
                    .expect("claim and route")
            );
            assert_eq!(
                timeout(EVENT_BOUND, started_rx)
                    .await
                    .expect("bounded backend start event")
                    .expect("backend start event"),
                key
            );
            assert!(peer_state.lock().await.active);
            assert!(data.multiplexer.contains_session(&key));
            // Subsequent REST evidence must come from slash dispatch itself.
            trace.lock().await.clear();

            let framework = poise::FrameworkContext {
                bot_id: bot.id,
                options: &options,
                user_data: &data,
                shard_manager: &manager,
            };
            let responded = AtomicBool::new(false);
            let invocation_data: Mutex<Box<dyn Any + Send + Sync>> =
                Mutex::new(Box::new(()));
            let arguments = interaction.data.options();
            let mut parents = Vec::new();
            timeout(
                EVENT_BOUND,
                poise::dispatch::dispatch_interaction(
                    framework,
                    &context,
                    &interaction,
                    &responded,
                    &invocation_data,
                    &arguments,
                    &mut parents,
                ),
            )
            .await
            .expect("bounded registered slash dispatch")
            .unwrap_or_else(|error| panic!("registered slash callback failed: {error}"));

            // Observe immediately after callback completion, before dropping
            // the mux. Cleanup cancellation must not satisfy this oracle.
            let state = peer_state.lock().await;
            eprintln!(
                "{case} key={key} starts={} interrupts={:?} remote_active={}",
                state.starts, state.interrupts, state.active
            );
            assert_eq!(state.starts, 1);
            assert_eq!(
                state.interrupts,
                vec![json!({"threadId": "r1", "turnId": "t1"})],
                "registered /stop must interrupt the lane entered by message ingress"
            );
            assert!(!state.active, "message lane remains active after /stop");
            drop(state);

            let requests = trace.lock().await;
            let thread_request = format!("GET /api/v10/channels/{}", fixture.thread_id);
            assert!(requests.iter().any(|(path, _)| path == &thread_request));
            let callbacks: Vec<_> = requests
                .iter()
                .filter(|(path, _)| path == &format!("POST {CALLBACK_PATH}"))
                .collect();
            assert_eq!(callbacks.len(), 1);
            assert_eq!(callbacks[0].1["type"], 4);
            assert_eq!(callbacks[0].1["data"]["flags"], 64);
            drop(requests);

            let stored: Vec<String> =
                sqlx::query_scalar("SELECT session_key FROM sessions ORDER BY session_key")
                    .fetch_all(database.pool())
                    .await
                    .unwrap();
            assert_eq!(stored, vec![key.storage_key()], "slash must not create a second lane");
            let state_json: String =
                sqlx::query_scalar("SELECT state_json FROM sessions WHERE session_key = ?")
                    .bind(key.storage_key())
                    .fetch_one(database.pool())
                    .await
                    .unwrap();
            let saved: omon_gateway::SessionState = serde_json::from_str(&state_json).unwrap();
            assert!(saved.suspended);
            assert_eq!(saved.metadata.get("omo_thread_id"), Some(&json!("r1")));
        })
        .catch_unwind()
        .await;

        // This also runs after a failing assertion. Closing all mux senders
        // gracefully cancels held work while its loopback peer is still alive.
        drop(owned_mux.take());
        let retired = timeout(CLEANUP_BOUND, retired_rx).await;
        tasks.abort_all();
        let mut peer_errors = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            if let Err(error) = joined {
                if !error.is_cancelled() {
                    peer_errors.push(error.to_string());
                }
            }
        }
        let closed = timeout(CLEANUP_BOUND, database.close()).await;
        eprintln!(
            "{case} cleanup backend_retired={retired:?} peer_errors={peer_errors:?} database_closed={closed:?}"
        );
        (outcome, retired, peer_errors, closed)
    });
    // Serenity's internal shard-queuer task has no public join handle. This
    // fixture owns its entire runtime; that task cannot survive either case.
    drop(runtime);
    eprintln!("{case} cleanup runtime_dropped=true");
    assert!(
        peer_errors.is_empty(),
        "fixture peer failures: {peer_errors:?}"
    );
    closed.expect("database pool closed");
    retired
        .expect("bounded actor/backend retirement")
        .expect("actor/backend ownership released");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
