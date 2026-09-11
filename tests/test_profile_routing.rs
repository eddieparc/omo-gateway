use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::FutureExt;
use omon_gateway::{
    parse_profile_routes, AgentRunner, Database, DiscordEgress, DiscordMessageTransport,
    InboundEvent, MultiplexerConfig, OmonError, OutboundAction, OutboundDispatcher, ProfileRoute,
    ProfileRouter, Result, SessionContext, SessionKey, SessionMultiplexer, SessionState,
};
use serenity::all::{ChannelId, MessageId};
use tokio::sync::{mpsc, Mutex};

const TEST_TIMEOUT: Duration = Duration::from_secs(5);

struct ContextCapturingRunner {
    captured: Mutex<Vec<SessionContext>>,
    // Receiver closure reports that every actor has released this runner.
    _lifetime: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl AgentRunner for ContextCapturingRunner {
    async fn run(&self, session: &mut SessionContext, _event: InboundEvent) -> Result<()> {
        self.captured.lock().await.push(session.clone());
        Ok(())
    }
}

async fn finish_profile_test(
    multiplexer: SessionMultiplexer,
    runner: Arc<ContextCapturingRunner>,
    mut stopped: mpsc::UnboundedReceiver<()>,
    db: Database,
    outcome: std::thread::Result<()>,
) {
    // Close actor mailboxes even when routing or an assertion panicked. The
    // last runner owner is released only after actor shutdown has flushed.
    drop(multiplexer);
    drop(runner);
    let released = tokio::time::timeout(TEST_TIMEOUT, stopped.recv()).await;
    // Attempt database closure before reporting either cleanup failure.
    let closed = tokio::time::timeout(TEST_TIMEOUT, db.close()).await;
    assert!(
        matches!(&released, Ok(None)) && closed.is_ok(),
        "profile cleanup failed: actor release={released:?}, database close={closed:?}"
    );
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

struct SentMessageTransport {
    sent: mpsc::UnboundedSender<(ChannelId, String)>,
}

#[async_trait]
impl DiscordMessageTransport for SentMessageTransport {
    async fn send_message(&self, channel: ChannelId, content: String) -> Result<MessageId> {
        self.sent
            .send((channel, content))
            .map_err(|error| OmonError::Multiplexer(error.to_string()))?;
        Ok(MessageId::new(1))
    }

    async fn start_typing(&self, _channel: ChannelId) -> Result<()> {
        Err(OmonError::Multiplexer(
            "unexpected typing in final send".into(),
        ))
    }

    async fn edit_message(
        &self,
        _channel: ChannelId,
        _message: MessageId,
        _content: String,
    ) -> Result<()> {
        Err(OmonError::Multiplexer(
            "unexpected edit in final send".into(),
        ))
    }

    async fn delete_message(&self, _channel: ChannelId, _message: MessageId) -> Result<()> {
        Err(OmonError::Multiplexer(
            "unexpected delete in final send".into(),
        ))
    }
}

#[tokio::test]
async fn test_profile_router_hierarchical_precedence() {
    let json_config = r#"[
        {
            "guild": 100,
            "model": "guild-model",
            "system_prompt": "guild prompt"
        },
        {
            "guild": 100,
            "channel": 200,
            "model": "channel-model",
            "system_prompt": "channel prompt",
            "toolsets": ["terminal"]
        },
        {
            "guild": 100,
            "channel": 200,
            "thread": 300,
            "model": "thread-model",
            "system_prompt": "thread prompt",
            "toolsets": ["web"]
        }
    ]"#;

    let router = ProfileRouter::from_json(json_config);

    // 1. Thread level: thread_id 300 in channel 200 of guild 100 -> matches thread route
    let matched = router.match_route(Some(100), 200, Some(300)).unwrap();
    assert_eq!(matched.model.as_deref(), Some("thread-model"));
    assert_eq!(matched.system_prompt.as_deref(), Some("thread prompt"));
    assert_eq!(
        matched.enabled_toolsets.as_deref(),
        Some(&["web".to_string()][..])
    );

    // 2. Channel level: other thread 999 in channel 200 of guild 100 -> falls back to channel route
    let matched = router.match_route(Some(100), 200, Some(999)).unwrap();
    assert_eq!(matched.model.as_deref(), Some("channel-model"));
    assert_eq!(matched.system_prompt.as_deref(), Some("channel prompt"));
    assert_eq!(
        matched.enabled_toolsets.as_deref(),
        Some(&["terminal".to_string()][..])
    );

    // 3. Direct channel message: channel 200 in guild 100 (no thread) -> matches channel route
    let matched = router.match_route(Some(100), 200, None).unwrap();
    assert_eq!(matched.model.as_deref(), Some("channel-model"));

    // 4. Guild level: channel 555 in guild 100 (no thread) -> matches guild route
    let matched = router.match_route(Some(100), 555, None).unwrap();
    assert_eq!(matched.model.as_deref(), Some("guild-model"));
    assert_eq!(matched.system_prompt.as_deref(), Some("guild prompt"));

    // 5. Unrelated guild -> returns None
    let matched = router.match_route(Some(999), 555, None);
    assert!(matched.is_none());
}

#[tokio::test]
async fn test_multiplexer_routes_inbound_with_profile_defaults() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let (lifetime, stopped) = mpsc::unbounded_channel();
    let runner = Arc::new(ContextCapturingRunner {
        captured: Mutex::new(Vec::new()),
        _lifetime: lifetime,
    });

    let routes = vec![
        ProfileRoute {
            name: Some("dev-channel".into()),
            guild: Some(10),
            channel: Some(20),
            thread: None,
            enabled: true,
            model: Some("dev-llm-model".into()),
            system_prompt: Some("You are a dev assistant".into()),
            enabled_toolsets: Some(vec!["terminal".into(), "file".into()]),
            profile: None,
            allowed_users: None,
        },
        ProfileRoute {
            name: Some("special-thread".into()),
            guild: Some(10),
            channel: Some(20),
            thread: Some(30),
            enabled: true,
            model: Some("thread-llm-model".into()),
            system_prompt: Some("You are a thread assistant".into()),
            enabled_toolsets: Some(vec!["web".into()]),
            profile: None,
            allowed_users: None,
        },
    ];
    let profile_router = ProfileRouter::new(routes);

    let multiplexer = SessionMultiplexer::with_profile_router(
        db.pool().clone(),
        runner.clone(),
        None,
        MultiplexerConfig::default(),
        profile_router,
    );

    let outcome = AssertUnwindSafe(async {
        tokio::time::timeout(TEST_TIMEOUT, async {
            // Send event in channel 20 (no thread) -> should get dev-channel profile
            let session_key_channel =
                SessionKey::new("discord", Some("10"), "20", None::<String>, "user-channel");
            let event1 =
                InboundEvent::message(session_key_channel.clone(), "msg-1", "Hello channel");

            // Send event in thread 30 -> should get special-thread profile
            let session_key_thread =
                SessionKey::new("discord", Some("10"), "20", Some("30"), "user-thread");
            let event2 = InboundEvent::message(session_key_thread.clone(), "msg-2", "Hello thread");
            // Each route subscribes its terminal acknowledgement before enqueueing.
            tokio::try_join!(
                multiplexer.route_awaiting_turn(event1),
                multiplexer.route_awaiting_turn(event2),
            )
            .unwrap();

            let captured = runner.captured.lock().await.clone();
            assert_eq!(captured.len(), 2);

            let cap_channel = captured
                .iter()
                .find(|c| c.key == session_key_channel)
                .unwrap();
            assert_eq!(
                cap_channel.state.active_model.as_deref(),
                Some("dev-llm-model")
            );
            assert_eq!(
                cap_channel.state.system_prompt.as_deref(),
                Some("You are a dev assistant")
            );
            assert_eq!(
                cap_channel.state.enabled_toolsets.as_deref(),
                Some(&["terminal".to_string(), "file".to_string()][..])
            );

            let cap_thread = captured
                .iter()
                .find(|c| c.key == session_key_thread)
                .unwrap();
            assert_eq!(
                cap_thread.state.active_model.as_deref(),
                Some("thread-llm-model")
            );
            assert_eq!(
                cap_thread.state.system_prompt.as_deref(),
                Some("You are a thread assistant")
            );
            assert_eq!(
                cap_thread.state.enabled_toolsets.as_deref(),
                Some(&["web".to_string()][..])
            );
        })
        .await
        .expect("profile routing scenario timed out");
    })
    .catch_unwind()
    .await;

    finish_profile_test(multiplexer, runner, stopped, db, outcome).await;
}

#[tokio::test]
async fn test_multiplexer_does_not_clobber_existing_explicit_model() {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let (lifetime, stopped) = mpsc::unbounded_channel();
    let runner = Arc::new(ContextCapturingRunner {
        captured: Mutex::new(Vec::new()),
        _lifetime: lifetime,
    });

    let routes = vec![ProfileRoute {
        name: Some("default-guild".into()),
        guild: Some(50),
        channel: None,
        thread: None,
        enabled: true,
        model: Some("profile-default-model".into()),
        system_prompt: Some("Profile system prompt".into()),
        enabled_toolsets: None,
        profile: None,
        allowed_users: None,
    }];
    let profile_router = ProfileRouter::new(routes);

    let multiplexer = SessionMultiplexer::with_profile_router(
        db.pool().clone(),
        runner.clone(),
        None,
        MultiplexerConfig::default(),
        profile_router,
    );

    let outcome = AssertUnwindSafe(async {
        tokio::time::timeout(TEST_TIMEOUT, async {
            // Pre-populate session in DB with an explicit model (e.g. set by /model)
            let session_key = SessionKey::new("discord", Some("50"), "60", None::<String>, "user-explicit");
            let explicit_state = SessionState {
                active_model: Some("explicit-user-chosen-model".into()),
                ..Default::default()
            };
            let explicit_json = serde_json::to_string(&explicit_state).unwrap();

            sqlx::query(
                "INSERT INTO sessions (session_key, platform, guild_id, channel_id, user_id, state_json) VALUES (?, 'discord', '50', '60', 'user-explicit', ?)"
            )
            .bind(session_key.storage_key())
            .bind(&explicit_json)
            .execute(db.pool())
            .await
            .unwrap();

            let event = InboundEvent::message(
                session_key.clone(),
                "msg-explicit",
                "Hello with explicit model",
            );
            // The terminal acknowledgement is subscribed before the event is enqueued.
            multiplexer.route_awaiting_turn(event).await.unwrap();

            let captured = runner.captured.lock().await.clone();
            assert_eq!(captured.len(), 1);
            let session = &captured[0];
            // Explicit model is preserved, NOT clobbered by profile default
            assert_eq!(
                session.state.active_model.as_deref(),
                Some("explicit-user-chosen-model")
            );
            // Unset system prompt gets populated from profile
            assert_eq!(
                session.state.system_prompt.as_deref(),
                Some("Profile system prompt")
            );
        })
        .await
        .expect("profile routing scenario timed out");
    })
    .catch_unwind()
    .await;

    finish_profile_test(multiplexer, runner, stopped, db, outcome).await;
}

#[test]
fn test_json_parsing_edge_cases() {
    assert_eq!(parse_profile_routes("").len(), 0);
    assert_eq!(parse_profile_routes("   \n\t  ").len(), 0);
    assert_eq!(parse_profile_routes("null").len(), 0);
    assert_eq!(parse_profile_routes("invalid json").len(), 0);
    assert_eq!(parse_profile_routes("{\"not_an_array\": 1}").len(), 0);

    let json = r#"[{"guild":123,"channel":456,"thread":null,"model":"gpt-x","system_prompt":"...","toolsets":["terminal","web"]}]"#;
    let routes = parse_profile_routes(json);
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].guild, Some(123));
    assert_eq!(routes[0].channel, Some(456));
    assert_eq!(routes[0].thread, None);
    assert_eq!(routes[0].model.as_deref(), Some("gpt-x"));
    assert_eq!(routes[0].system_prompt.as_deref(), Some("..."));
    assert_eq!(
        routes[0].enabled_toolsets.as_deref(),
        Some(&["terminal".to_string(), "web".to_string()][..])
    );
}

#[tokio::test]
async fn runtime_footer_flag_reaches_final_egress() {
    let content = "Final response";
    let model = "ag16-model";
    // A relative sentinel avoids platform-specific roots and HOME abbreviation.
    let workspace = "ag16-workspace";
    let channel = ChannelId::new(20);
    let session = SessionKey::new("discord", Some("10"), "20", None::<String>, "2");

    for enabled in [true, false] {
        // Subscribe at the final transport boundary before dispatching. Both
        // configurations carry identical metadata; only the flag differs.
        let (tx, mut sent) = mpsc::unbounded_channel();
        let egress = DiscordEgress::new(Arc::new(serenity::all::Http::new("unused-test-token")))
            .with_message_transport(Arc::new(SentMessageTransport { sent: tx }))
            .with_runtime_footer(enabled)
            .with_default_model(model.to_owned())
            .with_workspace_root(std::path::PathBuf::from(workspace));

        let dispatched = tokio::time::timeout(
            TEST_TIMEOUT,
            egress.dispatch(OutboundAction::SendMessage {
                session: session.clone(),
                content: content.to_owned(),
                reply_to: None,
            }),
        )
        .await;
        // No task is spawned by this fixture or the awaited final-send path.
        // Dropping egress closes the transport signal before any assertions.
        drop(egress);
        let first = tokio::time::timeout(TEST_TIMEOUT, sent.recv()).await;
        let end = tokio::time::timeout(TEST_TIMEOUT, sent.recv()).await;
        drop(sent);

        dispatched.expect("final dispatch timed out").unwrap();
        let (target, output) = first
            .expect("final send signal timed out")
            .expect("egress did not send a message");
        assert!(
            end.expect("transport did not close").is_none(),
            "unexpected extra send"
        );
        assert_eq!(target, channel);
        if enabled {
            let footer = output.strip_prefix(content).expect("response copy changed");
            assert!(footer.contains(model), "model did not reach final egress");
            assert!(
                footer.contains(workspace),
                "workspace did not reach final egress"
            );
        } else {
            assert_eq!(output, content);
        }
    }
}

#[tokio::test]
async fn imported_profiles_preserve_bot_policy() {
    let route_json = r#"[
        {
            "name": "work",
            "profile": "work",
            "channel": 20,
            "allowed_users": [2],
            "model": "claude-3-5-sonnet"
        }
    ]"#;
    let router = ProfileRouter::from_json(route_json);
    let global_allowed = vec![1];

    // User 1 on channel 20 (profile work) -> must be denied (only user 2 is allowed)
    assert!(
        !router.is_user_allowed_for_bot_or_route(Some("work_bot"), 20, 1, &global_allowed),
        "User 1 must be denied on profile work"
    );

    // User 2 on channel 20 (profile work) -> must be accepted
    assert!(
        router.is_user_allowed_for_bot_or_route(Some("work_bot"), 20, 2, &global_allowed),
        "User 2 must be allowed on profile work"
    );

    // Session application sets profile/agent
    let mut session = SessionContext::new(SessionKey::new(
        "discord",
        Some("work_bot"),
        "20",
        None::<String>,
        "2",
    ));
    if let Some(route) = router.match_route(None, 20, None) {
        route.apply_to_session(&mut session);
    }
    assert_eq!(
        session
            .state
            .metadata
            .get("profile")
            .and_then(serde_json::Value::as_str),
        Some("work")
    );
}

#[test]
fn routed_tools_reach_daemon() {
    use omon_gateway::agent::omo_protocol::thread_start_request;

    let route = ProfileRoute {
        name: Some("web-only".into()),
        guild: Some(100),
        channel: Some(200),
        thread: None,
        enabled: true,
        model: Some("gpt-4o".into()),
        system_prompt: Some("web prompt".into()),
        enabled_toolsets: Some(vec!["web".into()]),
        ..Default::default()
    };

    let key = SessionKey::new("discord", Some("100"), "200", None::<String>, "u1");
    let mut session = SessionContext::new(key);
    route.apply_to_session(&mut session);

    let msg = thread_start_request(
        session.state.system_prompt.as_deref(),
        session.state.active_model.as_deref(),
        None,
        session.state.enabled_toolsets.as_deref(),
    );
    let val: serde_json::Value =
        serde_json::from_str(msg.to_text().expect("must be text")).unwrap();
    assert_eq!(
        val["params"]["enabledToolsets"],
        serde_json::json!(["web"]),
        "routed enabled_toolsets must be serialized into thread/start params"
    );
}
