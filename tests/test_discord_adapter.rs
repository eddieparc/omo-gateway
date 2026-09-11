use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use omon_gateway::discord::adapter::{message_to_inbound_with_config, InboundFilterConfig};
use omon_gateway::discord::commands::{
    check_slash_admission, is_channel_authorized, is_user_allowed, is_user_authorized,
    CommandAdmissionResult, CommandChannelScope,
};
use omon_gateway::{
    approval_buttons, chunk_markdown, chunk_markdown_paginated, compose_reply_context,
    derive_auto_thread_name, is_authorized_clicker, parse_custom_id, render_user_prompt,
    safe_allowed_mentions, AgentRunner, AllowBotsMode, ApprovalDecision, ApprovalError,
    AttachmentDownloader, ChatMessage, Database, DeliveryLedgerService, DiscordEgress,
    DiscordFileUploader, DiscordMessageTransport, InboundEvent, LiveEditThrottler, LlmClient,
    LlmConfig, LlmProvider, MessageAttachment, OutboundAction, OutboundDispatcher, Result,
    SessionKey, SmartApprovalGuard, DISCORD_ATTACHMENT_MAX_BYTES,
};
use serenity::all::{ChannelId, ChannelType, Message, MessageId, UserId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Call {
    Typing,
    Edit(MessageId, String),
    Send(String),
    Delete(MessageId),
}

struct MockTransport {
    calls: Mutex<Vec<Call>>,
    typing: mpsc::UnboundedSender<()>,
}

#[async_trait]
impl DiscordMessageTransport for MockTransport {
    async fn start_typing(&self, _channel_id: ChannelId) -> Result<()> {
        self.calls.lock().await.push(Call::Typing);
        self.typing.send(()).unwrap();
        Ok(())
    }

    async fn edit_message(
        &self,
        _channel_id: ChannelId,
        message_id: MessageId,
        content: String,
    ) -> Result<()> {
        self.calls
            .lock()
            .await
            .push(Call::Edit(message_id, content));
        Ok(())
    }

    async fn send_message(&self, _channel_id: ChannelId, content: String) -> Result<MessageId> {
        self.calls.lock().await.push(Call::Send(content));
        Ok(MessageId::new(99))
    }

    async fn delete_message(&self, _channel_id: ChannelId, message_id: MessageId) -> Result<()> {
        self.calls.lock().await.push(Call::Delete(message_id));
        Ok(())
    }
}

#[derive(Default)]
struct MockFileUploader {
    calls: Mutex<Vec<(ChannelId, PathBuf)>>,
}

#[async_trait]
impl DiscordFileUploader for MockFileUploader {
    async fn upload(
        &self,
        _http: Arc<serenity::http::Http>,
        channel: ChannelId,
        path: &Path,
    ) -> Result<()> {
        self.calls.lock().await.push((channel, path.to_owned()));
        Ok(())
    }
}

#[test]
fn chunks_markdown_at_discord_limit_and_balances_code_fences() {
    let content = format!(
        "before\n```rust\n{}\n```\nafter",
        "let value = 1;\n".repeat(250)
    );
    let chunks = chunk_markdown(&content, 2_000);

    assert!(chunks.len() > 1);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 2_000));
    assert!(chunks
        .iter()
        .all(|chunk| chunk.matches("```").count() % 2 == 0));
    assert!(chunks[0].starts_with("(1/"));
    assert!(chunks[0].ends_with("\n```"));
    assert!(chunks[1].starts_with("(2/"));
    assert!(chunks[1].contains("```rust\n"));

    let unpaginated = chunk_markdown_paginated(&content, 2_000, false);
    assert!(unpaginated.len() > 1);
    assert!(unpaginated[0].ends_with("\n```"));
    assert!(unpaginated[1].starts_with("```rust\n"));
}

#[tokio::test(start_paused = true)]
async fn live_edits_use_typing_and_debounce_subsequent_updates() {
    let (typing_tx, _typing_rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });
    let throttler = Arc::new(LiveEditThrottler::with_debounce(
        transport.clone(),
        ChannelId::new(7),
        MessageId::new(8),
        Duration::from_millis(800),
    ));

    throttler.update("first", false).await.unwrap();
    let update = {
        let throttler = throttler.clone();
        tokio::spawn(async move { throttler.update("second", false).await })
    };
    tokio::task::yield_now().await;
    assert_eq!(transport.calls.lock().await.len(), 2);

    tokio::time::advance(Duration::from_millis(799)).await;
    tokio::task::yield_now().await;
    assert_eq!(transport.calls.lock().await.len(), 2);
    tokio::time::advance(Duration::from_millis(1)).await;
    update.await.unwrap().unwrap();

    assert_eq!(
        *transport.calls.lock().await,
        vec![
            Call::Typing,
            Call::Edit(MessageId::new(8), "first".into()),
            Call::Typing,
            Call::Edit(MessageId::new(8), "second".into()),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn final_live_edit_preempts_a_sleeping_intermediate_update() {
    let (typing_tx, _typing_rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });
    let throttler = Arc::new(LiveEditThrottler::with_debounce(
        transport.clone(),
        ChannelId::new(7),
        MessageId::new(8),
        Duration::from_millis(800),
    ));

    throttler.update("first", false).await.unwrap();
    let stale = {
        let throttler = throttler.clone();
        tokio::spawn(async move { throttler.update("stale", false).await })
    };
    tokio::task::yield_now().await;

    tokio::time::timeout(Duration::from_millis(1), throttler.update("final", true))
        .await
        .expect("final update must not wait behind the debounce sleeper")
        .unwrap();
    assert_eq!(
        *transport.calls.lock().await,
        vec![
            Call::Typing,
            Call::Edit(MessageId::new(8), "first".into()),
            Call::Typing,
            Call::Edit(MessageId::new(8), "final".into()),
        ]
    );

    tokio::time::advance(Duration::from_millis(800)).await;
    stale.await.unwrap().unwrap();
    assert_eq!(transport.calls.lock().await.len(), 4);
}

#[tokio::test(start_paused = true)]
async fn final_live_edit_deletes_surplus_chunk_messages() {
    let (typing_tx, _typing_rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });
    let throttler = LiveEditThrottler::with_debounce(
        transport.clone(),
        ChannelId::new(7),
        MessageId::new(8),
        Duration::from_millis(800),
    );

    // 1. During streaming, long text (> 2000 chars) maintains a single preview (no Send)
    throttler.update(&"x".repeat(2_100), false).await.unwrap();
    {
        let calls = transport.calls.lock().await;
        assert!(
            !calls.iter().any(|call| matches!(call, Call::Send(_))),
            "Streaming mid-stream must NOT send continuation messages"
        );
        let preview_edits: Vec<_> = calls
            .iter()
            .filter_map(|call| match call {
                Call::Edit(id, text) => Some((*id, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(preview_edits.len(), 1);
        assert_eq!(preview_edits[0].0, MessageId::new(8));
        assert!(preview_edits[0].1.ends_with(" …"));
        assert_eq!(preview_edits[0].1.chars().count(), 2000);
    }

    // 2. On finalize with long text, splits into chunks and sends continuation message
    throttler.update(&"x".repeat(2_100), true).await.unwrap();
    {
        let calls = transport.calls.lock().await;
        assert!(
            calls.iter().any(|call| matches!(call, Call::Send(_))),
            "Finalize must send continuation chunks for oversized content"
        );
    }

    // 3. Subsequent finalize with short text deletes the surplus continuation message
    throttler.update("short", true).await.unwrap();
    {
        let calls = transport.calls.lock().await;
        assert!(
            calls.contains(&Call::Delete(MessageId::new(99))),
            "Finalize with reduced chunks must delete surplus continuation messages"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn single_preview_during_streaming_truncates_without_sending_extra_messages() {
    let (typing_tx, _typing_rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });
    let throttler = LiveEditThrottler::with_debounce(
        transport.clone(),
        ChannelId::new(7),
        MessageId::new(8),
        Duration::from_millis(800),
    );

    // Stream progressive oversized updates
    throttler.update(&"a".repeat(2_500), false).await.unwrap();
    tokio::time::advance(Duration::from_millis(800)).await;
    throttler.update(&"b".repeat(3_000), false).await.unwrap();
    tokio::time::advance(Duration::from_millis(800)).await;
    throttler.update(&"c".repeat(4_000), false).await.unwrap();

    let calls = transport.calls.lock().await;
    // Verify zero Send calls occurred mid-stream
    assert!(
        !calls.iter().any(|call| matches!(call, Call::Send(_))),
        "Must not send any extra messages mid-stream"
    );
    // Verify edits only targeted initial message ID 8 and stayed within limit
    let edit_targets: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Edit(id, text) => Some((*id, text.chars().count())),
            _ => None,
        })
        .collect();
    assert_eq!(edit_targets.len(), 3);
    for (target_id, count) in edit_targets {
        assert_eq!(target_id, MessageId::new(8));
        assert_eq!(count, 2000);
    }
}

#[test]
fn test_derive_auto_thread_name_formatting_and_capping() {
    let bot_id = UserId::new(42);
    assert_eq!(
        derive_auto_thread_name("<@42> help me fix this bug", bot_id),
        "help me fix this bug"
    );
    assert_eq!(
        derive_auto_thread_name("<@!42>   lots   of   spaces   here  ", bot_id),
        "lots of spaces here"
    );
    assert_eq!(derive_auto_thread_name("<@42>", bot_id), "Conversation");
    let long_prompt = format!("<@42> {}", "a".repeat(100));
    let derived = derive_auto_thread_name(&long_prompt, bot_id);
    assert_eq!(derived.chars().count(), 80);
    assert!(derived.ends_with("..."));
}

#[test]
fn test_safe_allowed_mentions() {
    let mentions = safe_allowed_mentions();
    let json = serde_json::to_value(&mentions).unwrap();

    // Verify users allowed and replied_user enabled, but everyone and roles denied
    assert_eq!(json["parse"], serde_json::json!(["users"]));
    assert_eq!(json["replied_user"], serde_json::json!(true));
    assert_eq!(json["roles"], serde_json::json!([]));
    assert_eq!(json["users"], serde_json::json!([]));
}

#[test]
fn test_compose_reply_context() {
    // Normal reply
    assert_eq!(
        compose_reply_context("alice", "Original question?", "Here is the answer"),
        "> [Replying to @alice]: Original question?\n\nHere is the answer"
    );

    // Capping long referenced content at 500 chars
    let long_text = "w".repeat(750);
    let result = compose_reply_context("bob", &long_text, "Follow up");
    let expected = format!("> [Replying to @bob]: {}...\n\nFollow up", "w".repeat(500));
    assert_eq!(result, expected);

    // Empty body
    assert_eq!(
        compose_reply_context("carol", "check logs", ""),
        "> [Replying to @carol]: check logs"
    );

    // Empty referenced content
    assert_eq!(
        compose_reply_context("dave", "", "my feedback"),
        "> [Replying to @dave]\n\nmy feedback"
    );
}

#[test]
fn test_inbound_hydrates_reply_context_and_attachments() {
    let bot_id = UserId::new(42);
    let config = InboundFilterConfig {
        allowed_users: &[10],
        primary_bot_id: Some(42),
        ..Default::default()
    };

    // Reply with parent text and attachment in a DM
    let reply_msg = reply_message_fixture(
        None,
        "what does this error mean?",
        "bob",
        "Error trace attached",
        vec![("trace.log", "text/plain")],
    );
    let event =
        message_to_inbound_with_config(&reply_msg, bot_id, Some(ChannelType::Private), &config)
            .unwrap();
    assert_eq!(
        event.content,
        "> [Replying to @bob]: Error trace attached [Attachment: trace.log]\n\nwhat does this error mean?"
    );

    // Reply with parent multiple attachments only
    let att_only_reply = reply_message_fixture(
        None,
        "can you analyze these?",
        "charlie",
        "",
        vec![
            ("doc1.pdf", "application/pdf"),
            ("doc2.pdf", "application/pdf"),
        ],
    );
    let event2 = message_to_inbound_with_config(
        &att_only_reply,
        bot_id,
        Some(ChannelType::Private),
        &config,
    )
    .unwrap();
    assert_eq!(
        event2.content,
        "> [Replying to @charlie]: [Attachments: doc1.pdf, doc2.pdf]\n\ncan you analyze these?"
    );
}

#[test]
fn test_approval_buttons_and_parse_custom_id() {
    let request_id = uuid::Uuid::new_v4();
    let buttons = approval_buttons(request_id);
    assert_eq!(buttons.len(), 1);

    // Round-trip parse_custom_id
    assert_eq!(
        parse_custom_id(&format!("omon:approval:{request_id}:once")),
        Some((request_id, ApprovalDecision::Once))
    );
    assert_eq!(
        parse_custom_id(&format!("omon:approval:{request_id}:session")),
        Some((request_id, ApprovalDecision::Session))
    );
    assert_eq!(
        parse_custom_id(&format!("omon:approval:{request_id}:always")),
        Some((request_id, ApprovalDecision::Always))
    );
    assert_eq!(
        parse_custom_id(&format!("omon:approval:{request_id}:deny")),
        Some((request_id, ApprovalDecision::Deny { reason: None }))
    );
}

#[test]
fn test_is_authorized_clicker() {
    // Open when allowlist is empty
    assert!(is_authorized_clicker(12345, &[]));
    assert!(is_authorized_clicker(99999, &[]));

    // Enforced when allowlist is non-empty
    let allowed = vec![111, 222, 333];
    assert!(is_authorized_clicker(111, &allowed));
    assert!(is_authorized_clicker(222, &allowed));
    assert!(is_authorized_clicker(333, &allowed));
    assert!(!is_authorized_clicker(444, &allowed));
    assert!(!is_authorized_clicker(99999, &allowed));
}

#[tokio::test(start_paused = true)]
async fn approval_guard_resolves_all_four_buttons_and_times_out() {
    let guard = SmartApprovalGuard::new();

    // 1. Once
    let once_prompt = guard.request().await;
    let once_id = once_prompt.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{once_id}:once"))
            .await
    );
    assert_eq!(
        once_prompt.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Once)
    );

    // 2. Session
    let session_prompt = guard.request().await;
    let session_id = session_prompt.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{session_id}:session"))
            .await
    );
    assert_eq!(
        session_prompt.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Session)
    );

    // 3. Always
    let always_prompt = guard.request().await;
    let always_id = always_prompt.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{always_id}:always"))
            .await
    );
    assert_eq!(
        always_prompt.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Always)
    );

    // 4. Deny
    let deny_prompt = guard.request().await;
    let deny_id = deny_prompt.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{deny_id}:deny"))
            .await
    );
    assert_eq!(
        deny_prompt.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Deny { reason: None })
    );

    // 5. Legacy aliases
    let legacy_app = guard.request().await;
    let legacy_app_id = legacy_app.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{legacy_app_id}:approve"))
            .await
    );
    assert_eq!(
        legacy_app.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Once)
    );

    let legacy_rej = guard.request().await;
    let legacy_rej_id = legacy_rej.request_id;
    assert!(
        guard
            .resolve_custom_id(&format!("omon:approval:{legacy_rej_id}:reject"))
            .await
    );
    assert_eq!(
        legacy_rej.wait(Duration::from_secs(60)).await,
        Ok(ApprovalDecision::Deny { reason: None })
    );

    // 6. Timeout
    let pending = guard.request().await;
    let wait = tokio::spawn(pending.wait(Duration::from_secs(60)));
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(wait.await.unwrap(), Err(ApprovalError::Timeout));
}

#[test]
fn converts_serenity_dm_mentions_threads_and_attachments() {
    let bot_id = UserId::new(42);
    let config = InboundFilterConfig {
        allowed_users: &[10],
        primary_bot_id: Some(42),
        ..Default::default()
    };
    let dm = message_fixture(None, "hello", Vec::new());
    let event =
        message_to_inbound_with_config(&dm, bot_id, Some(ChannelType::Private), &config).unwrap();
    assert_eq!(event.content, "hello");
    assert_eq!(event.session.guild_id, None);
    assert_eq!(event.attachments[0].filename, "main.rs");
    assert_eq!(
        event.attachments[0].content_type.as_deref(),
        Some("text/x-rust")
    );
    assert_eq!(event.attachments[0].local_path, None);

    let ignored = message_fixture(Some(9), "ordinary channel message", Vec::new());
    assert!(
        message_to_inbound_with_config(&ignored, bot_id, Some(ChannelType::Text), &config)
            .is_none()
    );

    let mentioned = message_fixture(Some(9), "<@42> inspect this", vec![42]);
    let event =
        message_to_inbound_with_config(&mentioned, bot_id, Some(ChannelType::Text), &config)
            .unwrap();
    assert_eq!(event.content, "inspect this");
    assert_eq!(event.session.thread_id, None);

    let thread = message_fixture(Some(9), "<@42> thread continuation", vec![42]);
    let event =
        message_to_inbound_with_config(&thread, bot_id, Some(ChannelType::PublicThread), &config)
            .unwrap();
    assert_eq!(event.session.thread_id.as_deref(), Some("7"));
}

#[test]
fn only_primary_bot_owns_unmentioned_threads_and_free_channels() {
    let primary = UserId::new(42);
    let secondary = UserId::new(84);
    let thread = message_fixture(Some(9), "continue", Vec::new());

    // With thread 7 registered as active, primary bot processes it
    assert!(message_to_inbound_with_config(
        &thread,
        primary,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    )
    .is_some());
    // Secondary bot ignores unmentioned thread even if active
    assert!(message_to_inbound_with_config(
        &thread,
        secondary,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    )
    .is_none());
    // If thread is NOT in active_threads, even primary bot ignores it (stops thread spam!)
    assert!(message_to_inbound_with_config(
        &thread,
        primary,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    )
    .is_none());

    let free = message_fixture(Some(9), "hello", Vec::new());
    assert!(message_to_inbound_with_config(
        &free,
        primary,
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            free_response_channels: &[7],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    )
    .is_some());
    assert!(message_to_inbound_with_config(
        &free,
        secondary,
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            free_response_channels: &[7],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    )
    .is_none());
}

#[test]
fn every_bot_answers_its_own_direct_messages_regardless_of_primary_bot() {
    let primary = UserId::new(42);
    let secondary = UserId::new(84);
    let dm = message_fixture(None, "hello", Vec::new());

    let non_primary_event = message_to_inbound_with_config(
        &dm,
        secondary,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    );
    assert!(non_primary_event.is_some());
    assert_eq!(
        non_primary_event
            .as_ref()
            .unwrap()
            .session
            .bot_id
            .as_deref(),
        Some("84")
    );

    let primary_event = message_to_inbound_with_config(
        &dm,
        primary,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(primary.get()),
            ..Default::default()
        },
    );
    assert!(primary_event.is_some());
    assert_eq!(
        primary_event.as_ref().unwrap().session.bot_id.as_deref(),
        Some("42")
    );
}

#[test]
fn every_explicitly_mentioned_bot_owns_exactly_its_target() {
    let message = message_fixture(Some(9), "<@42> <@84> compare", vec![42, 84]);

    assert!(message_to_inbound_with_config(
        &message,
        UserId::new(42),
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());
    assert!(message_to_inbound_with_config(
        &message,
        UserId::new(84),
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());
    assert!(message_to_inbound_with_config(
        &message,
        UserId::new(126),
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_none());
}

#[test]
fn test_allow_bots_policy_modes() {
    let bot_id = UserId::new(42);

    let mut bot_msg = message_fixture(None, "Hello from another bot", Vec::new());
    bot_msg.author.bot = true;
    bot_msg.author.id = UserId::new(999);

    let mut bot_msg_mentioned = message_fixture(None, "<@42> question from bot", vec![42]);
    bot_msg_mentioned.author.bot = true;
    bot_msg_mentioned.author.id = UserId::new(999);

    let mut self_msg = message_fixture(None, "<@42> self ping", vec![42]);
    self_msg.author.bot = true;
    self_msg.author.id = UserId::new(42);

    // 1. None: all bot authors dropped even if mentioned
    assert!(message_to_inbound_with_config(
        &bot_msg,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999],
            allow_bots: AllowBotsMode::None,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_none());

    assert!(message_to_inbound_with_config(
        &bot_msg_mentioned,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999],
            allow_bots: AllowBotsMode::None,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_none());

    // 2. Mentions: dropped if unmentioned, allowed if mentioned
    assert!(message_to_inbound_with_config(
        &bot_msg,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999],
            allow_bots: AllowBotsMode::Mentions,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_none());

    assert!(message_to_inbound_with_config(
        &bot_msg_mentioned,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999],
            allow_bots: AllowBotsMode::Mentions,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_some());

    // 3. All: allowed without mention in DM / free channels
    assert!(message_to_inbound_with_config(
        &bot_msg,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999],
            allow_bots: AllowBotsMode::All,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_some());

    // Self message is always dropped across all modes to prevent loops
    assert!(message_to_inbound_with_config(
        &self_msg,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[999, 42],
            allow_bots: AllowBotsMode::All,
            primary_bot_id: Some(42),
            ..Default::default()
        }
    )
    .is_none());
}

#[test]
fn test_thread_require_mention_option() {
    let bot_id = UserId::new(42);
    let thread_msg_unmentioned =
        message_fixture(Some(9), "random server chatter in thread", Vec::new());
    let thread_msg_mentioned = message_fixture(Some(9), "<@42> please help here", vec![42]);

    // When thread_require_mention is true, even an active thread drops unmentioned messages
    assert!(message_to_inbound_with_config(
        &thread_msg_unmentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(42),
            thread_require_mention: true,
            ..Default::default()
        },
    )
    .is_none());

    // When thread_require_mention is true, explicitly mentioned messages are processed
    assert!(message_to_inbound_with_config(
        &thread_msg_mentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(42),
            thread_require_mention: true,
            ..Default::default()
        },
    )
    .is_some());

    // When thread_require_mention is false, unmentioned messages in active threads are processed
    assert!(message_to_inbound_with_config(
        &thread_msg_unmentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(42),
            thread_require_mention: false,
            ..Default::default()
        },
    )
    .is_some());
}

#[test]
fn scoped_thread_participation_gates_unmentioned_and_allows_mentioned() {
    let bot_id = UserId::new(42);
    let thread_msg_unmentioned =
        message_fixture(Some(9), "random server chatter in thread", Vec::new());
    let thread_msg_mentioned = message_fixture(Some(9), "<@42> please help here", vec![42]);

    // 1. Thread not in active set and unmentioned -> MUST NOT ROUTE (no spam)
    assert!(message_to_inbound_with_config(
        &thread_msg_unmentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_none());

    // 2. Mentioned in thread -> routes even if not previously active
    assert!(message_to_inbound_with_config(
        &thread_msg_mentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());

    // 3. Thread IS in active set -> routes unmentioned message for primary bot
    assert!(message_to_inbound_with_config(
        &thread_msg_unmentioned,
        bot_id,
        Some(ChannelType::PublicThread),
        &InboundFilterConfig {
            allowed_users: &[10],
            active_threads: &[7],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());
}

#[test]
fn channel_allow_and_ignore_lists() {
    let bot_id = UserId::new(42);
    let mentioned_msg = message_fixture(Some(9), "<@42> do something", vec![42]);
    let dm_msg = message_fixture(None, "hello in dm", Vec::new());

    // Channel 7 is ignored -> blocked even with explicit mention
    assert!(message_to_inbound_with_config(
        &mentioned_msg,
        bot_id,
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            ignored_channels: &[7],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_none());

    // Whitelist active with channel 7 permitted -> allowed
    assert!(message_to_inbound_with_config(
        &mentioned_msg,
        bot_id,
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            allowed_channels: &[7],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());

    // Whitelist active with channel 99 permitted (channel 7 not in whitelist) -> blocked
    assert!(message_to_inbound_with_config(
        &mentioned_msg,
        bot_id,
        Some(ChannelType::Text),
        &InboundFilterConfig {
            allowed_users: &[10],
            allowed_channels: &[99],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_none());

    // DMs are exempt from channel whitelist
    assert!(message_to_inbound_with_config(
        &dm_msg,
        bot_id,
        Some(ChannelType::Private),
        &InboundFilterConfig {
            allowed_users: &[10],
            allowed_channels: &[99],
            primary_bot_id: Some(42),
            ..Default::default()
        },
    )
    .is_some());
}

#[test]
fn test_dm_accepted_when_channel_type_private_regardless_of_guild_id_or_secondary_bot() {
    // Given: A message with guild_id set but channel_type is Private (e.g. from context or edge payload)
    // and secondary bot id where allowed_channels whitelist is active
    let bot_id = UserId::new(42);
    let msg = message_fixture(Some(12345), "hello direct message", Vec::new());
    let config = InboundFilterConfig {
        allowed_users: &[10],
        allowed_channels: &[9999],
        primary_bot_id: Some(999), // different from bot_id (42)
        ..Default::default()
    };

    // When: filtering inbound message with channel_type = Private
    let event = message_to_inbound_with_config(&msg, bot_id, Some(ChannelType::Private), &config);

    // Then: DM is recognized, bypassing channel whitelist and primary bot requirement,
    // and session.guild_id is None
    let event = event.expect("DM should be accepted when channel_type is Private");
    assert_eq!(event.session.guild_id, None);
    assert_eq!(event.content, "hello direct message");
}

#[test]
fn user_authorization_allow_all_and_roles() {
    // 1. Default closed when both allowed_users and allowed_roles are empty
    assert!(!is_user_authorized(10, &[], &[], &[], false));

    // 2. Allow-all bypasses non-empty user and role allowlists
    assert!(is_user_authorized(10, &[], &[20, 30], &[100], true));

    // 3. User allowlist allows matching user ID
    assert!(is_user_authorized(20, &[], &[20, 30], &[], false));
    assert!(!is_user_authorized(99, &[], &[20, 30], &[], false));

    // 4. Role membership allows user possessing an allowed role
    assert!(is_user_authorized(99, &[100, 200], &[], &[100], false));
    assert!(is_user_authorized(99, &[200, 300], &[20], &[300], false));
    assert!(!is_user_authorized(99, &[500], &[20], &[300], false));
}

#[test]
fn test_shared_vs_per_user_thread_sessions() {
    let bot_id = UserId::new(42);
    let mut msg_alice = message_fixture(Some(9), "<@42> msg from alice", vec![42]);
    msg_alice.author.id = UserId::new(100);
    let mut msg_bob = message_fixture(Some(9), "<@42> msg from bob", vec![42]);
    msg_bob.author.id = UserId::new(200);

    // U20: Guild channels and threads are permanent lanes per (bot, channel), independent of sender
    let thread_config = InboundFilterConfig {
        allowed_users: &[100, 200],
        primary_bot_id: Some(42),
        ..Default::default()
    };
    let event_alice_thread = message_to_inbound_with_config(
        &msg_alice,
        bot_id,
        Some(ChannelType::PublicThread),
        &thread_config,
    )
    .unwrap();
    let event_bob_thread = message_to_inbound_with_config(
        &msg_bob,
        bot_id,
        Some(ChannelType::PublicThread),
        &thread_config,
    )
    .unwrap();
    assert_eq!(
        event_alice_thread.session.storage_key(),
        event_bob_thread.session.storage_key()
    );
    assert_eq!(event_alice_thread.session, event_bob_thread.session);

    // Text channels also share the canonical per-bot lane across senders
    let event_alice_text =
        message_to_inbound_with_config(&msg_alice, bot_id, Some(ChannelType::Text), &thread_config)
            .unwrap();
    let event_bob_text =
        message_to_inbound_with_config(&msg_bob, bot_id, Some(ChannelType::Text), &thread_config)
            .unwrap();
    assert_eq!(
        event_alice_text.session.storage_key(),
        event_bob_text.session.storage_key()
    );
    assert_eq!(event_alice_text.session, event_bob_text.session);
}

#[tokio::test]
async fn discord_delivery_claims_deduplicate_per_target_bot() {
    let database = Database::connect("sqlite::memory:").await.unwrap();
    let ledger = DeliveryLedgerService::new(database.pool().clone());
    let session =
        SessionKey::new("discord", Some("9"), "7", None::<String>, "10").with_bot_id("42");
    let event = InboundEvent::message(session, "8", "hello");

    assert!(ledger
        .record_incoming_as(&event, "discord:8")
        .await
        .unwrap());
    assert!(!ledger
        .record_incoming_as(&event, "discord:8")
        .await
        .unwrap());
    assert!(ledger
        .record_incoming_as(&event, "discord:8:84")
        .await
        .unwrap());
}

#[test]
fn renders_complete_attachment_context_for_attachment_only_turns() {
    let event = InboundEvent::message(
        SessionKey::new("discord", Some("9"), "7", None::<String>, "10"),
        "8",
        "",
    )
    .with_attachments(vec![MessageAttachment {
        id: "11".into(),
        filename: "main.rs".into(),
        url: "https://cdn.example/main.rs".into(),
        content_type: Some("text/x-rust".into()),
        size_bytes: Some(24),
        local_path: None,
        text_content: None,
    }]);

    assert_eq!(
        render_user_prompt(&event),
        "[Attachment: main.rs (text/x-rust, 24 bytes) - https://cdn.example/main.rs]"
    );
}

#[test]
fn renders_downloaded_attachment_local_path() {
    let local_path = PathBuf::from("/tmp/omon/main.png");
    let event = InboundEvent::message(
        SessionKey::new("discord", Some("9"), "7", None::<String>, "10"),
        "8",
        "inspect",
    )
    .with_attachments(vec![MessageAttachment {
        id: "11".into(),
        filename: "main.png".into(),
        url: "https://cdn.example/main.png".into(),
        content_type: Some("image/png".into()),
        size_bytes: Some(24),
        local_path: Some(local_path.clone()),
        text_content: None,
    }]);

    let prompt = render_user_prompt(&event);
    assert!(prompt.contains("inspect\n\n[Attachment: main.png"));
    assert!(prompt.contains(&format!("local path: {}", local_path.display())));
}

#[tokio::test]
async fn downloads_discord_attachment_once_and_reuses_cache() {
    let workspace = test_workspace("download-cache");
    std::fs::create_dir_all(&workspace).unwrap();
    let body = b"cached-image-bytes".to_vec();
    let request_count = Arc::new(AtomicUsize::new(0));
    let (url, server) = spawn_single_response_server(body.clone(), request_count.clone()).await;
    let downloader = AttachmentDownloader::new(&workspace).unwrap();
    let attachment = MessageAttachment {
        id: "attachment/11".into(),
        filename: "../capture.png".into(),
        url,
        content_type: Some("image/png".into()),
        size_bytes: Some(body.len() as u64),
        local_path: None,
        text_content: None,
    };

    let first = downloader.download_attachment(&attachment).await.unwrap();
    server.await.unwrap();
    let second = downloader.download_attachment(&attachment).await.unwrap();

    assert_eq!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), body);
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    assert!(first.starts_with(std::fs::canonicalize(&workspace).unwrap()));
    assert_eq!(first.parent(), Some(downloader.attachment_root()));
    assert!(!first.to_string_lossy().contains(".."));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn rejects_discord_attachment_over_size_limit_before_network() {
    let workspace = test_workspace("size-limit");
    let downloader = AttachmentDownloader::new(&workspace).unwrap();
    let attachment = MessageAttachment {
        id: "11".into(),
        filename: "large.bin".into(),
        url: "http://127.0.0.1:1/never-requested".into(),
        content_type: Some("application/octet-stream".into()),
        size_bytes: Some(DISCORD_ATTACHMENT_MAX_BYTES + 1),
        local_path: None,
        text_content: None,
    };

    let error = downloader
        .download_attachment(&attachment)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("25 MB"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn hydrates_and_inlines_small_text_attachment() {
    let workspace = test_workspace("text-inline");
    std::fs::create_dir_all(&workspace).unwrap();
    let file_content = b"fn main() {\n    println!(\"hello world\");\n}\n".to_vec();
    let request_count = Arc::new(AtomicUsize::new(0));
    let (url, server) =
        spawn_single_response_server(file_content.clone(), request_count.clone()).await;
    let downloader = AttachmentDownloader::new(&workspace).unwrap();
    let mut attachment = MessageAttachment {
        id: "attachment/text-1".into(),
        filename: "main.rs".into(),
        url,
        content_type: Some("text/x-rust".into()),
        size_bytes: Some(file_content.len() as u64),
        local_path: None,
        text_content: None,
    };

    downloader.hydrate(&mut attachment).await.unwrap();
    server.await.unwrap();

    assert!(attachment.local_path.is_some());
    assert_eq!(
        attachment.text_content.as_deref(),
        Some("fn main() {\n    println!(\"hello world\");\n}\n")
    );

    let event = InboundEvent::message(
        SessionKey::new("discord", Some("9"), "7", None::<String>, "10"),
        "8",
        "please explain this",
    )
    .with_attachments(vec![attachment]);

    let prompt = render_user_prompt(&event);
    assert!(prompt.contains("please explain this"));
    assert!(prompt.contains("[Attachment: main.rs (text/x-rust, 43 bytes)"));
    assert!(prompt
        .contains("[Content of main.rs]:\n\nfn main() {\n    println!(\"hello world\");\n}\n"));

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn hydrate_does_not_inline_binary_attachment() {
    let workspace = test_workspace("binary-no-inline");
    std::fs::create_dir_all(&workspace).unwrap();
    let binary_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic
    let request_count = Arc::new(AtomicUsize::new(0));
    let (url, server) =
        spawn_single_response_server(binary_bytes.clone(), request_count.clone()).await;
    let downloader = AttachmentDownloader::new(&workspace).unwrap();
    let mut attachment = MessageAttachment {
        id: "attachment/img-1".into(),
        filename: "image.png".into(),
        url,
        content_type: Some("image/png".into()),
        size_bytes: Some(binary_bytes.len() as u64),
        local_path: None,
        text_content: None,
    };

    downloader.hydrate(&mut attachment).await.unwrap();
    server.await.unwrap();

    assert!(attachment.local_path.is_some());
    assert!(attachment.text_content.is_none());

    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn encodes_supported_images_as_openai_and_anthropic_vision_blocks() {
    let workspace = test_workspace("vision");
    std::fs::create_dir_all(&workspace).unwrap();
    let formats = [
        ("png", "image/png", b"png-bytes".as_slice()),
        ("jpg", "image/jpeg", b"jpeg-bytes".as_slice()),
        ("webp", "image/webp", b"webp-bytes".as_slice()),
        ("gif", "image/gif", b"gif-bytes".as_slice()),
    ];
    let mut attachments = Vec::new();
    for (index, (extension, media_type, bytes)) in formats.iter().enumerate() {
        let path = workspace.join(format!("image-{index}.{extension}"));
        std::fs::write(&path, bytes).unwrap();
        attachments.push(MessageAttachment {
            id: index.to_string(),
            filename: path.file_name().unwrap().to_string_lossy().into_owned(),
            url: format!("https://cdn.example/image-{index}.{extension}"),
            content_type: Some((*media_type).into()),
            size_bytes: Some(bytes.len() as u64),
            local_path: Some(path),
            text_content: None,
        });
    }
    let message = ChatMessage::new("user", "inspect these").with_attachments(attachments.clone());

    let openai = LlmClient::new(LlmConfig::new(LlmProvider::OpenAi, "gpt-test")).unwrap();
    let openai_payload = openai.build_payload(std::slice::from_ref(&message), &[]);
    let openai_content = openai_payload["messages"][0]["content"].as_array().unwrap();
    assert_eq!(openai_content[0]["type"], "text");
    assert_eq!(openai_content.len(), formats.len() + 1);
    for (index, (_, media_type, bytes)) in formats.iter().enumerate() {
        assert_eq!(openai_content[index + 1]["type"], "image_url");
        assert_eq!(
            openai_content[index + 1]["image_url"]["url"],
            format!("data:{media_type};base64,{}", BASE64_STANDARD.encode(bytes))
        );
    }

    let anthropic = LlmClient::new(LlmConfig::new(LlmProvider::Anthropic, "claude-test")).unwrap();
    let anthropic_payload = anthropic.build_payload(&[message], &[]);
    let anthropic_content = anthropic_payload["messages"][0]["content"]
        .as_array()
        .unwrap();
    assert_eq!(anthropic_content.len(), formats.len() + 1);
    for (index, (_, media_type, bytes)) in formats.iter().enumerate() {
        assert_eq!(anthropic_content[index]["type"], "image");
        assert_eq!(anthropic_content[index]["source"]["type"], "base64");
        assert_eq!(
            anthropic_content[index]["source"]["media_type"],
            *media_type
        );
        assert_eq!(
            anthropic_content[index]["source"]["data"],
            BASE64_STANDARD.encode(bytes)
        );
    }
    assert_eq!(anthropic_content[formats.len()]["type"], "text");

    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn discord_egress_dispatches_upload_file_to_target_channel() {
    let workspace = test_workspace("upload");
    std::fs::create_dir_all(&workspace).unwrap();
    let path = workspace.join("report.txt");
    std::fs::write(&path, b"report").unwrap();
    let uploader = Arc::new(MockFileUploader::default());
    let egress = DiscordEgress::new(Arc::new(serenity::http::Http::new("test-token")))
        .with_file_uploader(uploader.clone());
    let session = SessionKey::new("discord", Some("9"), "7", None::<String>, "10");

    egress
        .dispatch(OutboundAction::UploadFile {
            session,
            path: path.clone(),
        })
        .await
        .unwrap();

    assert_eq!(
        *uploader.calls.lock().await,
        vec![(ChannelId::new(7), path)]
    );
    let _ = std::fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn discord_egress_handles_typing_start_and_stop() {
    let egress = DiscordEgress::new(Arc::new(serenity::http::Http::new("test-token")));
    let session = SessionKey::new("discord", Some("9"), "7", None::<String>, "10");

    egress
        .dispatch(OutboundAction::Typing {
            session: session.clone(),
            active: true,
        })
        .await
        .unwrap();

    egress
        .dispatch(OutboundAction::Typing {
            session,
            active: false,
        })
        .await
        .unwrap();
}

#[test]
fn slash_authorization_defaults_open_and_enforces_allowlist() {
    assert!(!is_user_allowed(&[], 10));
    assert!(is_user_allowed(&[10, 11], 10));
    assert!(!is_user_allowed(&[10, 11], 12));
}

fn test_workspace(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("omon-discord-{label}-{}", uuid::Uuid::new_v4()))
}

async fn spawn_single_response_server(
    body: Vec<u8>,
    request_count: Arc<AtomicUsize>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 2048];
        let _ = socket.read(&mut request).await.unwrap();
        request_count.fetch_add(1, Ordering::SeqCst);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (format!("http://{address}/attachment"), handle)
}

fn message_fixture(guild_id: Option<u64>, content: &str, mentions: Vec<u64>) -> Message {
    let mentions = mentions
        .into_iter()
        .map(|id| {
            serde_json::json!({
                "id": id.to_string(), "username": "omon", "discriminator": "0001",
                "avatar": null, "bot": true, "system": false, "mfa_enabled": false,
                "banner": null, "accent_color": null, "locale": null, "verified": false,
                "email": null, "flags": 0, "premium_type": 0, "public_flags": 0,
                "global_name": null, "avatar_decoration_data": null, "collectibles": null,
                "primary_guild": null
            })
        })
        .collect::<Vec<_>>();
    serde_json::from_value(serde_json::json!({
        "id": "8", "channel_id": "7", "guild_id": guild_id.map(|id| id.to_string()),
        "author": {
            "id": "10", "username": "alice", "discriminator": "0001", "avatar": null,
            "bot": false, "system": false, "mfa_enabled": false, "banner": null,
            "accent_color": null, "locale": null, "verified": false, "email": null,
            "flags": 0, "premium_type": 0, "public_flags": 0, "global_name": null,
            "avatar_decoration_data": null, "collectibles": null, "primary_guild": null
        },
        "content": content, "timestamp": "2026-08-14T00:00:00Z", "edited_timestamp": null,
        "tts": false, "mention_everyone": false, "mentions": mentions, "mention_roles": [],
        "mention_channels": [],
        "attachments": [{
            "id": "11", "filename": "main.rs", "description": null, "height": null,
            "width": null, "proxy_url": "https://cdn.example/main.rs", "size": 24,
            "url": "https://cdn.example/main.rs", "content_type": "text/x-rust",
            "ephemeral": false, "duration_secs": null, "waveform": null
        }],
        "embeds": [], "reactions": [], "nonce": null, "pinned": false, "webhook_id": null,
        "type": 0, "activity": null, "application": null, "application_id": null,
        "message_reference": null, "flags": null, "referenced_message": null,
        "message_snapshots": [], "interaction": null, "interaction_metadata": null,
        "thread": null, "components": [], "sticker_items": [], "position": null,
        "role_subscription_data": null, "member": null, "poll": null
    }))
    .unwrap()
}

fn reply_message_fixture(
    guild_id: Option<u64>,
    content: &str,
    ref_author: &str,
    ref_content: &str,
    ref_attachments: Vec<(&str, &str)>,
) -> Message {
    let parent_attachments = ref_attachments
        .into_iter()
        .enumerate()
        .map(|(i, (name, ct))| {
            serde_json::json!({
                "id": (100 + i).to_string(), "filename": name, "description": null, "height": null,
                "width": null, "proxy_url": format!("https://cdn.example/{name}"), "size": 100,
                "url": format!("https://cdn.example/{name}"), "content_type": ct,
                "ephemeral": false, "duration_secs": null, "waveform": null
            })
        })
        .collect::<Vec<_>>();

    let referenced_message = serde_json::json!({
        "id": "1", "channel_id": "7", "guild_id": guild_id.map(|id| id.to_string()),
        "author": {
            "id": "20", "username": ref_author, "discriminator": "0001", "avatar": null,
            "bot": false, "system": false, "mfa_enabled": false, "banner": null,
            "accent_color": null, "locale": null, "verified": false, "email": null,
            "flags": 0, "premium_type": 0, "public_flags": 0, "global_name": null,
            "avatar_decoration_data": null, "collectibles": null, "primary_guild": null
        },
        "content": ref_content, "timestamp": "2026-08-14T00:00:00Z", "edited_timestamp": null,
        "tts": false, "mention_everyone": false, "mentions": [], "mention_roles": [],
        "mention_channels": [], "attachments": parent_attachments,
        "embeds": [], "reactions": [], "nonce": null, "pinned": false, "webhook_id": null,
        "type": 0, "activity": null, "application": null, "application_id": null,
        "message_reference": null, "flags": null, "referenced_message": null,
        "message_snapshots": [], "interaction": null, "interaction_metadata": null,
        "thread": null, "components": [], "sticker_items": [], "position": null,
        "role_subscription_data": null, "member": null, "poll": null
    });

    serde_json::from_value(serde_json::json!({
        "id": "8", "channel_id": "7", "guild_id": guild_id.map(|id| id.to_string()),
        "author": {
            "id": "10", "username": "alice", "discriminator": "0001", "avatar": null,
            "bot": false, "system": false, "mfa_enabled": false, "banner": null,
            "accent_color": null, "locale": null, "verified": false, "email": null,
            "flags": 0, "premium_type": 0, "public_flags": 0, "global_name": null,
            "avatar_decoration_data": null, "collectibles": null, "primary_guild": null
        },
        "content": content, "timestamp": "2026-08-14T00:00:00Z", "edited_timestamp": null,
        "tts": false, "mention_everyone": false, "mentions": [], "mention_roles": [],
        "mention_channels": [],
        "attachments": [],
        "embeds": [], "reactions": [], "nonce": null, "pinned": false, "webhook_id": null,
        "type": 19, "activity": null, "application": null, "application_id": null,
        "message_reference": null, "flags": null, "referenced_message": referenced_message,
        "message_snapshots": [], "interaction": null, "interaction_metadata": null,
        "thread": null, "components": [], "sticker_items": [], "position": null,
        "role_subscription_data": null, "member": null, "poll": null
    }))
    .unwrap()
}

#[tokio::test]
async fn coalesces_split_messages_and_unions_attachments() {
    let session = SessionKey::new(
        "discord",
        Some("guild"),
        "channel",
        None::<String>,
        "author",
    );
    let mut event1 = InboundEvent::message(session.clone(), "msg-chunk-1", "Hello from part 1");
    event1.attachments.push(MessageAttachment {
        id: "att-1".into(),
        filename: "file1.txt".into(),
        url: "https://example.com/file1.txt".into(),
        content_type: Some("text/plain".into()),
        size_bytes: Some(123),
        local_path: None,
        text_content: None,
    });

    let mut event2 =
        InboundEvent::message(session.clone(), "msg-chunk-2", "and continuation part 2");
    event2.delivery_id = Some("discord:msg-chunk-2".into());
    event2.attachments.push(MessageAttachment {
        id: "att-2".into(),
        filename: "file2.png".into(),
        url: "https://example.com/file2.png".into(),
        content_type: Some("image/png".into()),
        size_bytes: Some(456),
        local_path: None,
        text_content: None,
    });

    let coalesced = omon_gateway::coalesce_inbound_events(vec![event1, event2]).unwrap();
    assert_eq!(
        coalesced.content,
        "Hello from part 1\nand continuation part 2"
    );
    assert_eq!(coalesced.platform_message_id, "msg-chunk-2");
    assert_eq!(
        coalesced.delivery_id.as_deref(),
        Some("discord:msg-chunk-2")
    );
    assert_eq!(coalesced.attachments.len(), 2);
    assert_eq!(coalesced.attachments[0].id, "att-1");
    assert_eq!(coalesced.attachments[1].id, "att-2");
}

#[test]
fn test_format_channel_context_ordering_and_truncation() {
    let history = vec![
        ("alice", "first message from earlier"),
        ("bob", "second message asking for info"),
        ("charlie", "third message with more details"),
    ];

    let formatted = omon_gateway::format_channel_context(&history);
    let expected = "[Recent channel context]\nalice: first message from earlier\nbob: second message asking for info\ncharlie: third message with more details";
    assert_eq!(formatted, expected);
}

#[test]
fn test_format_channel_context_empty_and_skip_empty_lines() {
    let empty: Vec<(&str, &str)> = Vec::new();
    assert_eq!(omon_gateway::format_channel_context(&empty), "");

    let with_blanks = vec![
        ("alice", "   \n\t  "),
        ("", "message with no author"),
        ("bob", "valid message"),
    ];
    assert_eq!(
        omon_gateway::format_channel_context(&with_blanks),
        "[Recent channel context]\nbob: valid message"
    );
}

#[test]
fn test_forwarded_message_snapshots_routing() {
    let bot_id = UserId::new(42);

    let raw_str = r#"{
        "id": "500",
        "channel_id": "7",
        "guild_id": null,
        "author": {
            "id": "100", "username": "alice", "discriminator": "0001", "avatar": null,
            "bot": false, "system": false, "mfa_enabled": false, "banner": null,
            "accent_color": null, "locale": null, "verified": false, "email": null,
            "flags": 0, "premium_type": 0, "public_flags": 0
        },
        "content": "",
        "timestamp": "2026-08-16T12:00:00Z",
        "edited_timestamp": null,
        "tts": false,
        "mention_everyone": false,
        "mentions": [],
        "mention_roles": [],
        "attachments": [],
        "embeds": [],
        "reactions": [],
        "nonce": null,
        "pinned": false,
        "webhook_id": null,
        "type": 0,
        "activity": null,
        "application": null,
        "application_id": null,
        "message_reference": null,
        "flags": null,
        "referenced_message": null,
        "message_snapshots": [
            {
                "message": {
                    "content": "This was originally sent in another channel",
                    "timestamp": "2026-08-16T11:55:00Z",
                    "edited_timestamp": null,
                    "mentions": [],
                    "mention_roles": [],
                    "attachments": [
                        {
                            "id": "999", "filename": "plan.md", "description": null,
                            "height": null, "width": null, "proxy_url": "https://cdn.example/plan.md",
                            "size": 500, "url": "https://cdn.example/plan.md",
                            "content_type": "text/markdown", "ephemeral": false,
                            "duration_secs": null, "waveform": null
                        }
                    ],
                    "embeds": [],
                    "type": 0,
                    "flags": null,
                    "components": [],
                    "sticker_items": []
                }
            }
        ],
        "interaction": null,
        "interaction_metadata": null,
        "thread": null,
        "components": [],
        "sticker_items": [],
        "position": null,
        "role_subscription_data": null,
        "member": null,
        "poll": null
    }"#;

    let msg: Message = serde_json::from_str(raw_str).unwrap();
    let config = InboundFilterConfig {
        allowed_users: &[100],
        primary_bot_id: Some(42),
        ..Default::default()
    };
    let event = message_to_inbound_with_config(&msg, bot_id, Some(ChannelType::Private), &config);
    assert!(event.is_some(), "Forwarded message should not be dropped");
    let event = event.unwrap();
    assert_eq!(
        event.content,
        "[Forwarded]\nThis was originally sent in another channel [Attachment: plan.md]"
    );
    assert_eq!(event.attachments.len(), 1);
    assert_eq!(event.attachments[0].filename, "plan.md");
}

#[tokio::test]
async fn test_dead_target_short_circuit_in_egress() {
    use omon_gateway::{
        DeadTargetRegistry, DiscordEgress, OutboundAction, OutboundDispatcher, SessionKey,
    };

    let dead_targets = std::sync::Arc::new(DeadTargetRegistry::new());
    // Pre-mark channel 98765 as dead (simulating a prior 404/403)
    dead_targets.mark_dead(98765, "HTTP 404: Unknown Channel");

    let client = std::sync::Arc::new(serenity::all::Http::new("fake-token"));
    let egress = DiscordEgress::new(client).with_dead_targets(dead_targets.clone());

    let session = SessionKey::new("discord", None::<String>, "98765", None::<String>, "user-1");

    // Attempt to dispatch a message to the dead channel
    // It should short-circuit and return Err without claiming delivered
    let result = egress
        .dispatch(OutboundAction::SendMessage {
            session: session.clone(),
            content: "hello dead channel".into(),
            reply_to: None,
        })
        .await;

    assert!(
        result.is_err(),
        "Short-circuited dead target should return Err"
    );

    // Also verify EditMessage short-circuits
    let edit_result = egress
        .dispatch(OutboundAction::EditMessage {
            session: session.clone(),
            platform_message_id: "123".into(),
            content: "edited content".into(),
        })
        .await;
    assert!(edit_result.is_err());

    // Once cleared (e.g. self-healed or user re-adds bot), is_dead returns false
    dead_targets.clear(98765);
    assert!(!dead_targets.is_dead(98765));
}

#[test]
fn authorization_surface_matrix() {
    let bot_id = UserId::new(42);

    // 1. user10 / default-deny: user 10 with empty allowlists and allow_all=false must be rejected
    assert!(
        !is_user_authorized(10, &[], &[], &[], false),
        "user 10 with empty lists and allow_all=false must fail-closed"
    );
    assert!(
        !is_user_allowed(&[], 10),
        "is_user_allowed must fail-closed for empty allowlist"
    );
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &InboundFilterConfig::default(),
        ),
        CommandAdmissionResult::UnauthorizedUser,
        "Slash command must reject unlisted user when allow_all=false"
    );

    // 2. Explicit allow_all true positive
    assert!(
        is_user_authorized(10, &[], &[], &[], true),
        "allow_all=true must authorize user"
    );
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &InboundFilterConfig {
                allow_all_users: true,
                ..Default::default()
            },
        ),
        CommandAdmissionResult::Allowed,
        "Slash command must admit user when allow_all=true"
    );

    // 3. User / role positive and negative
    assert!(
        is_user_authorized(10, &[], &[10], &[], false),
        "matching user ID in allowed_users must be admitted"
    );
    assert!(
        !is_user_authorized(10, &[], &[20], &[], false),
        "non-matching user ID must be rejected"
    );
    assert!(
        is_user_authorized(10, &[100], &[], &[100], false),
        "user possessing matching allowed role must be admitted"
    );
    assert!(
        !is_user_authorized(10, &[200], &[], &[100], false),
        "user without matching allowed role must be rejected"
    );

    // Setup channel fixture: message in child thread 8 under parent channel 7
    let mut thread_msg = message_fixture(Some(1), "<@42> steer guidance", vec![42]);
    thread_msg.channel_id = ChannelId::new(8);
    thread_msg.author.id = UserId::new(10);

    // 4. Ignored parent 7 / child thread 8: both slash steer and message ingress rejected before runner
    assert!(
        !is_channel_authorized(8, Some(7), &[], &[7], false),
        "is_channel_authorized must reject child thread 8 when parent 7 is ignored"
    );
    let ignored_parent_config = InboundFilterConfig {
        allowed_users: &[10],
        ignored_channels: &[7],
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &ignored_parent_config,
        ),
        CommandAdmissionResult::UnauthorizedChannel,
        "Slash steer in thread 8 under ignored parent 7 must be rejected before runner"
    );
    assert!(
        message_to_inbound_with_config(
            &thread_msg,
            bot_id,
            Some(ChannelType::PublicThread),
            &ignored_parent_config,
        )
        .is_none(),
        "Message ingress in thread 8 under ignored parent 7 must be rejected before runner"
    );

    // 5. Allowed parent 7 admits its nonignored child thread 8
    assert!(
        is_channel_authorized(8, Some(7), &[7], &[], false),
        "is_channel_authorized must admit child thread 8 when parent 7 is in allowed_channels"
    );
    let allowed_parent_config = InboundFilterConfig {
        allowed_users: &[10],
        allowed_channels: &[7],
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &allowed_parent_config,
        ),
        CommandAdmissionResult::Allowed,
        "Slash steer in thread 8 must be admitted when parent 7 is in allowed_channels"
    );
    assert!(
        message_to_inbound_with_config(
            &thread_msg,
            bot_id,
            Some(ChannelType::PublicThread),
            &allowed_parent_config,
        )
        .is_some(),
        "Message ingress in thread 8 must be admitted when parent 7 is in allowed_channels"
    );

    // 6. Child ignore override: allowed parent 7, but child thread 8 is in ignored_channels
    assert!(
        !is_channel_authorized(8, Some(7), &[7], &[8], false),
        "is_channel_authorized must reject when child thread 8 is explicitly ignored"
    );
    let child_ignored_config = InboundFilterConfig {
        allowed_users: &[10],
        allowed_channels: &[7],
        ignored_channels: &[8],
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &child_ignored_config,
        ),
        CommandAdmissionResult::UnauthorizedChannel,
        "Slash steer in thread 8 must be rejected when child thread 8 is in ignored_channels"
    );
    assert!(
        message_to_inbound_with_config(
            &thread_msg,
            bot_id,
            Some(ChannelType::PublicThread),
            &child_ignored_config,
        )
        .is_none(),
        "Message ingress in thread 8 must be rejected when child thread 8 is in ignored_channels"
    );

    // 7. Explicit allow_all only bypasses user policy, NOT ignored-channel policy
    let allow_all_ignored_channel_config = InboundFilterConfig {
        allow_all_users: true,
        ignored_channels: &[7],
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::guild(8, Some(7)),
            &allow_all_ignored_channel_config,
        ),
        CommandAdmissionResult::UnauthorizedChannel,
        "Explicit allow_all_users must not bypass ignored parent 7 in slash check"
    );
    assert!(
        message_to_inbound_with_config(
            &thread_msg,
            bot_id,
            Some(ChannelType::PublicThread),
            &allow_all_ignored_channel_config,
        )
        .is_none(),
        "Explicit allow_all_users must not bypass ignored parent 7 in message ingress"
    );

    // 8. Missing/failed required guild metadata must not silently authorize
    assert_eq!(
        check_slash_admission(
            10,
            false,
            CommandChannelScope::missing_guild_metadata(8),
            &allowed_parent_config,
        ),
        CommandAdmissionResult::MissingGuildMetadata,
        "Slash check with failed guild metadata must fail closed"
    );
    assert!(
        message_to_inbound_with_config(&thread_msg, bot_id, None, &allowed_parent_config,)
            .is_none(),
        "Message ingress with missing channel_type in guild must fail closed"
    );
}

#[tokio::test]
async fn dedicated_thread_owner_and_channel_identity() {
    let bot84 = UserId::new(84);
    let bot42 = UserId::new(42);

    // 1. Guild channel conversations are one permanent lane per (bot, channel), NOT per (bot, channel, user).
    // Alice and Bob in the same guild text channel must share the exact same permanent lane key.
    let mut msg_alice_text = message_fixture(Some(9), "<@84> text from alice", vec![84]);
    msg_alice_text.channel_id = ChannelId::new(7);
    msg_alice_text.author.id = UserId::new(100);

    let mut msg_bob_text = message_fixture(Some(9), "<@84> text from bob", vec![84]);
    msg_bob_text.channel_id = ChannelId::new(7);
    msg_bob_text.author.id = UserId::new(200);

    let config_text = InboundFilterConfig {
        allowed_users: &[100, 200],
        primary_bot_id: Some(42),
        ..Default::default()
    };
    let event_alice_text = message_to_inbound_with_config(
        &msg_alice_text,
        bot84,
        Some(ChannelType::Text),
        &config_text,
    )
    .expect("alice text message should be accepted");
    let event_bob_text =
        message_to_inbound_with_config(&msg_bob_text, bot84, Some(ChannelType::Text), &config_text)
            .expect("bob text message should be accepted");

    assert_eq!(
        event_alice_text.session.storage_key(),
        event_bob_text.session.storage_key(),
        "Guild text channel lane must be shared per bot, NOT per author"
    );
    assert_eq!(
        event_alice_text.session, event_bob_text.session,
        "Guild text channel session keys must be canonical and independent of sender"
    );

    // 2. Thread ownership: bot84 engaged thread8; Alice/Bob unmentioned followups route to bot84
    let mut msg_alice_thread = message_fixture(Some(9), "unmentioned followup from alice", vec![]);
    msg_alice_thread.channel_id = ChannelId::new(8);
    msg_alice_thread.author.id = UserId::new(100);

    let mut msg_bob_thread = message_fixture(Some(9), "unmentioned followup from bob", vec![]);
    msg_bob_thread.channel_id = ChannelId::new(8);
    msg_bob_thread.author.id = UserId::new(200);

    let database = Database::connect("sqlite::memory:").await.unwrap();
    let pool = database.pool().clone();

    // Record engagement of thread8 by bot84 in persistent SQLite storage
    Database::record_thread_owner(&pool, 8, 84).await.unwrap();

    let owner_record = Database::get_thread_owner(&pool, 8).await.unwrap();
    assert_eq!(
        owner_record,
        Some(84),
        "Thread 8 ownership must be durably stored in SQLite"
    );

    let config_thread = InboundFilterConfig {
        allowed_users: &[100, 200],
        active_threads: &[8],
        thread_owners: &[(8, 84)],
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };

    // Bot 84 must receive unmentioned followup in its engaged thread
    let event_alice_thread = message_to_inbound_with_config(
        &msg_alice_thread,
        bot84,
        Some(ChannelType::PublicThread),
        &config_thread,
    );
    assert!(
        event_alice_thread.is_some(),
        "bot84 engaged thread8 so it must receive unmentioned followup"
    );

    let event_bob_thread = message_to_inbound_with_config(
        &msg_bob_thread,
        bot84,
        Some(ChannelType::PublicThread),
        &config_thread,
    );
    assert!(
        event_bob_thread.is_some(),
        "bot84 engaged thread8 so it must receive bob's unmentioned followup"
    );

    assert_eq!(
        event_alice_thread.as_ref().unwrap().session.storage_key(),
        event_bob_thread.as_ref().unwrap().session.storage_key(),
        "Thread lane storage keys must match across different senders"
    );

    // Bot 42 (primary) must NOT take over thread 8
    let event_bot42_thread = message_to_inbound_with_config(
        &msg_alice_thread,
        bot42,
        Some(ChannelType::PublicThread),
        &config_thread,
    );
    assert!(
        event_bot42_thread.is_none(),
        "bot42 must NOT take over thread 8 engaged by bot 84"
    );

    // Simulated restart: re-read durable ownership from SQLite
    let reloaded_owner = Database::get_thread_owner(&pool, 8).await.unwrap().unwrap();
    let reloaded_owners = [(8, reloaded_owner)];
    let config_restarted = InboundFilterConfig {
        allowed_users: &[100, 200],
        active_threads: &[8],
        thread_owners: &reloaded_owners,
        parent_channel_id: Some(7),
        primary_bot_id: Some(42),
        ..Default::default()
    };
    assert!(
        message_to_inbound_with_config(
            &msg_alice_thread,
            bot84,
            Some(ChannelType::PublicThread),
            &config_restarted,
        )
        .is_some(),
        "bot84 must still receive unmentioned followup after restart"
    );
    assert!(
        message_to_inbound_with_config(
            &msg_alice_thread,
            bot42,
            Some(ChannelType::PublicThread),
            &config_restarted,
        )
        .is_none(),
        "bot42 must still not take over thread8 after restart"
    );

    // 3. Reply in parent 7 must NOT create a thread
    let reply_msg = reply_message_fixture(
        Some(9),
        "<@84> inline reply in parent",
        "charlie",
        "original message in parent",
        vec![],
    );
    let is_reply = reply_msg.kind == serenity::model::channel::MessageType::InlineReply
        || reply_msg.referenced_message.is_some();
    assert!(
        !omon_gateway::discord::adapter::should_auto_create_thread(
            true,  // auto_thread enabled
            true,  // is_guild_text
            true,  // is_explicit_mention
            false, // is_free_channel
            is_reply,
        ),
        "Reply in parent 7 must not create a thread"
    );

    // 4. Channels already free of thread-forcing must not create a thread
    assert!(
        !omon_gateway::discord::adapter::should_auto_create_thread(
            true,  // auto_thread enabled
            true,  // is_guild_text
            true,  // is_explicit_mention
            true,  // is_free_channel
            false, // is_reply
        ),
        "Channels already free of thread-forcing must not trigger auto-thread"
    );
}

#[tokio::test]
async fn split_batch_replay_and_generation() {
    use omon_gateway::{
        AgentRunner, Database, DeliveryLedgerService, MultiplexerConfig, PoiseData, SessionContext,
        SessionKey, SessionMultiplexer, SplitMessageDebouncer,
    };

    struct CollectingRunner {
        events: Mutex<Vec<InboundEvent>>,
        routed_tx: mpsc::UnboundedSender<()>,
    }

    #[async_trait]
    impl AgentRunner for CollectingRunner {
        async fn run(&self, _session: &mut SessionContext, event: InboundEvent) -> Result<()> {
            self.events.lock().await.push(event);
            let _ = self.routed_tx.send(());
            Ok(())
        }
    }

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let (routed_tx, mut routed_rx) = mpsc::unbounded_channel();
    let runner = Arc::new(CollectingRunner {
        events: Mutex::new(Vec::new()),
        routed_tx,
    });
    let multiplexer = SessionMultiplexer::new(
        db.pool().clone(),
        runner.clone(),
        MultiplexerConfig::default(),
    );
    let data = PoiseData::new(multiplexer, db.pool().clone());
    let debouncer = SplitMessageDebouncer::new(Duration::from_millis(600));

    // --- Scenario Part 1: constituent IDs 8, 9, 9 in one batch ---
    let session = SessionKey::new(
        "discord",
        Some("guild-1"),
        "channel-1",
        None::<String>,
        "user-1",
    );
    tokio::time::pause();
    let msg8 = InboundEvent::message(session.clone(), "8", "chunk-8");
    let msg9a = InboundEvent::message(session.clone(), "9", "chunk-9");
    let msg9b = InboundEvent::message(session.clone(), "9", "chunk-9");

    debouncer.enqueue(msg8, data.clone()).await;
    debouncer.enqueue(msg9a, data.clone()).await;
    debouncer.enqueue(msg9b, data.clone()).await;

    // Advance simulated time past the debounce duration (600ms)
    tokio::time::advance(Duration::from_millis(600)).await;
    tokio::time::resume();

    // Wait for the runner to complete the batch
    routed_rx.recv().await.expect("turn 1 should be executed");

    // Wait for delivery completion in the ledger
    let ledger = DeliveryLedgerService::new(db.pool().clone());
    for _ in 0..100 {
        let entry_9 = ledger.get("discord:9").await.ok().flatten();
        let entry_8 = ledger.get("discord:8").await.ok().flatten();
        if entry_9.as_ref().map(|e| e.status.as_str()) == Some("delivered")
            && entry_8.as_ref().map(|e| e.status.as_str()) == Some("delivered")
        {
            break;
        }
        tokio::task::yield_now().await;
    }

    // Check completed runner outcome: constituent 9 must not duplicate its content in the merged body
    {
        let runs = runner.events.lock().await;
        assert_eq!(runs.len(), 1, "batch should execute exactly one turn");
        assert_eq!(
            runs[0].content, "chunk-8\nchunk-9",
            "constituent IDs 8, 9, 9 must be deduplicated before merge, producing chunk-8\\nchunk-9 without duplicating chunk-9"
        );
    }

    // Check durable constituent claim rows:
    // Constituent 8 must be durably recorded and marked delivered
    let claim_8 = ledger.get("discord:8").await.unwrap();
    assert!(
        claim_8.is_some(),
        "constituent ID 8 must be durably recorded in delivery_ledger"
    );
    assert_eq!(
        claim_8.unwrap().status,
        "delivered",
        "constituent ID 8 claim row must be marked delivered upon batch completion"
    );

    let claim_9 = ledger.get("discord:9").await.unwrap();
    assert!(
        claim_9.is_some(),
        "constituent ID 9 must be durably recorded in delivery_ledger"
    );
    assert_eq!(
        claim_9.unwrap().status,
        "delivered",
        "constituent ID 9 claim row must be marked delivered upon batch completion"
    );

    // --- Scenario Part 2: replay of constituent ID 8 after batch completion ---
    tokio::time::pause();
    let msg8_replay = InboundEvent::message(session.clone(), "8", "chunk-8");
    debouncer.enqueue(msg8_replay, data.clone()).await;
    tokio::time::advance(Duration::from_millis(600)).await;
    tokio::time::resume();

    // Must NOT re-run the turn
    assert!(
        routed_rx.try_recv().is_err(),
        "replaying already-completed constituent ID 8 must NOT execute a new turn"
    );
    assert_eq!(
        runner.events.lock().await.len(),
        1,
        "runner executions must remain 1 after replay of constituent ID 8"
    );

    // --- Scenario Part 3: cancelling old batch and re-enqueuing equal-size new batch must not cause early flush ---
    let session_cancel = SessionKey::new(
        "discord",
        Some("guild-1"),
        "channel-1",
        None::<String>,
        "user-2",
    );

    tokio::time::pause();
    let old_batch_msg = InboundEvent::message(session_cancel.clone(), "old-1", "old content");
    debouncer.enqueue(old_batch_msg, data.clone()).await;

    // Advance 200ms into the 600ms debounce window
    tokio::time::advance(Duration::from_millis(200)).await;

    // Cancel old batch
    let cancelled = debouncer.cancel(&session_cancel).await;
    assert!(cancelled.is_some(), "old batch should be cancelled");

    // Re-enqueue an equal-size new batch (1 message)
    let new_batch_msg = InboundEvent::message(session_cancel.clone(), "new-1", "new content");
    debouncer.enqueue(new_batch_msg, data.clone()).await;

    // Advance 400ms: total elapsed since T=0 is 600ms (when old sleeper would wake),
    // but only 400ms elapsed since new batch was enqueued (which requires 600ms)
    tokio::time::advance(Duration::from_millis(400)).await;

    // Assert no early flush has occurred: runner must not have received new_batch_msg yet
    assert_eq!(
        runner.events.lock().await.len(),
        1,
        "equal-size replacement batch must not be prematurely flushed by older sleeper"
    );
    assert!(
        routed_rx.try_recv().is_err(),
        "runner channel must have no turn queued prematurely"
    );

    // Now advance the remaining 200ms to reach full 600ms debounce for the new batch
    tokio::time::advance(Duration::from_millis(200)).await;
    tokio::time::resume();

    routed_rx
        .recv()
        .await
        .expect("new batch should flush after its full debounce duration");

    // Verify the new batch was executed
    {
        let runs = runner.events.lock().await;
        assert_eq!(runs.len(), 2, "new batch should execute exactly once");
        assert_eq!(runs[1].content, "new content");
        assert_eq!(runs[1].platform_message_id, "new-1");
    }

    // Verify claim row for new batch in ledger
    for _ in 0..100 {
        if let Ok(Some(entry)) = ledger.get("discord:new-1").await {
            if entry.status == "delivered" {
                break;
            }
        }
        tokio::task::yield_now().await;
    }
    let claim_new = ledger.get("discord:new-1").await.unwrap();
    assert!(claim_new.is_some());
    assert_eq!(claim_new.unwrap().status, "delivered");
}

#[tokio::test]
async fn startup_recovery_preserves_work_and_source_time() {
    use omon_gateway::discord::adapter::{
        get_bot_channel_cursor, run_missed_message_backfill_with_fetcher, DiscordHistoryFetcher,
    };
    use omon_gateway::{
        AgentRunner, Database, MultiplexerConfig, OmonError, PoiseData, SessionContext,
        SessionMultiplexer,
    };
    use serenity::all::{Channel, GuildId};
    use std::collections::HashMap;

    struct RecordingRunner {
        events: Mutex<Vec<InboundEvent>>,
        fail_channel_id: Mutex<Option<String>>,
        ran_tx: tokio::sync::mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl AgentRunner for RecordingRunner {
        async fn run(&self, _session: &mut SessionContext, event: InboundEvent) -> Result<()> {
            if let Some(ref fail_id) = *self.fail_channel_id.lock().await {
                if event.session.channel_id == *fail_id {
                    return Err(OmonError::Multiplexer(format!(
                        "simulated routing failure for channel {fail_id}"
                    )));
                }
            }
            self.events.lock().await.push(event.clone());
            let _ = self.ran_tx.send(event.platform_message_id.clone());
            Ok(())
        }
    }

    let (ran_tx, mut ran_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let runner = Arc::new(RecordingRunner {
        events: Mutex::new(Vec::new()),
        fail_channel_id: Mutex::new(None),
        ran_tx,
    });

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let pool = db.pool().clone();
    let multiplexer =
        SessionMultiplexer::new(pool.clone(), runner.clone(), MultiplexerConfig::default());

    let mut data = PoiseData::new(multiplexer, pool.clone());
    data.allowed_roles = vec![777]; // Role-only authorization for user alice
    data.allowed_channels = vec![100, 200];
    data.primary_bot_id = Some(42);
    data.missed_backfill = true;

    fn make_test_msg(
        id: u64,
        channel_id: u64,
        guild_id: Option<u64>,
        author_id: u64,
        content: &str,
        timestamp_str: &str,
        mentions: Vec<u64>,
    ) -> Message {
        let mentions_json: Vec<serde_json::Value> = mentions
            .into_iter()
            .map(|m_id| {
                serde_json::json!({
                    "id": m_id.to_string(), "username": format!("bot_{m_id}"), "discriminator": "0001",
                    "avatar": null, "bot": true, "system": false, "mfa_enabled": false,
                    "banner": null, "accent_color": null, "locale": null, "verified": false,
                    "email": null, "flags": 0, "premium_type": 0, "public_flags": 0,
                    "global_name": null, "avatar_decoration_data": null, "collectibles": null,
                    "primary_guild": null
                })
            })
            .collect();

        serde_json::from_value(serde_json::json!({
            "id": id.to_string(),
            "channel_id": channel_id.to_string(),
            "guild_id": guild_id.map(|g| g.to_string()),
            "author": {
                "id": author_id.to_string(), "username": "alice", "discriminator": "0001", "avatar": null,
                "bot": false, "system": false, "mfa_enabled": false, "banner": null,
                "accent_color": null, "locale": null, "verified": false, "email": null,
                "flags": 0, "premium_type": 0, "public_flags": 0, "global_name": null,
                "avatar_decoration_data": null, "collectibles": null, "primary_guild": null
            },
            "content": content,
            "timestamp": timestamp_str,
            "edited_timestamp": null,
            "tts": false,
            "mention_everyone": false,
            "mentions": mentions_json,
            "mention_roles": [],
            "mention_channels": [],
            "attachments": [],
            "embeds": [], "reactions": [], "nonce": null, "pinned": false, "webhook_id": null,
            "type": 0, "activity": null, "application": null, "application_id": null,
            "message_reference": null, "flags": null, "referenced_message": null,
            "message_snapshots": [], "interaction": null, "interaction_metadata": null,
            "thread": null, "components": [], "sticker_items": [], "position": null,
            "role_subscription_data": null, "member": null, "poll": null
        }))
        .unwrap()
    }

    fn make_guild_channel(id: u64, guild_id: u64, kind: u8) -> Channel {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string(),
            "type": kind,
            "guild_id": guild_id.to_string(),
            "name": format!("chan_{id}"),
            "position": 0,
            "permission_overwrites": [],
            "rate_limit_per_user": 0,
            "nsfw": false
        }))
        .unwrap()
    }

    struct MockHistoryFetcher {
        messages: Mutex<HashMap<u64, Vec<Message>>>,
        channels: Mutex<HashMap<u64, std::result::Result<Channel, String>>>,
        member_roles: Mutex<HashMap<(u64, u64), Vec<u64>>>,
    }

    #[async_trait]
    impl DiscordHistoryFetcher for MockHistoryFetcher {
        async fn fetch_messages(
            &self,
            channel_id: ChannelId,
            after: Option<MessageId>,
            limit: u8,
        ) -> Result<Vec<Message>> {
            let store = self.messages.lock().await;
            let msgs = store.get(&channel_id.get()).cloned().unwrap_or_default();
            let mut filtered: Vec<Message> = if let Some(after_id) = after {
                msgs.into_iter()
                    .filter(|m| m.id.get() > after_id.get())
                    .collect()
            } else {
                msgs
            };
            filtered.sort_by_key(|m| m.id.get());
            Ok(filtered.into_iter().take(limit as usize).collect())
        }

        async fn get_channel(&self, channel_id: ChannelId) -> Result<Option<Channel>> {
            let store = self.channels.lock().await;
            if let Some(res) = store.get(&channel_id.get()) {
                match res {
                    Ok(ch) => Ok(Some(ch.clone())),
                    Err(err) => Err(OmonError::Config(err.clone())),
                }
            } else {
                Ok(None)
            }
        }

        async fn get_member_roles(&self, guild_id: GuildId, user_id: UserId) -> Result<Vec<u64>> {
            let store = self.member_roles.lock().await;
            Ok(store
                .get(&(guild_id.get(), user_id.get()))
                .cloned()
                .unwrap_or_default())
        }
    }

    let fetcher = Arc::new(MockHistoryFetcher {
        messages: Mutex::new(HashMap::new()),
        channels: Mutex::new(HashMap::new()),
        member_roles: Mutex::new(HashMap::new()),
    });

    fetcher
        .channels
        .lock()
        .await
        .insert(100, Ok(make_guild_channel(100, 999, 0)));
    fetcher
        .member_roles
        .lock()
        .await
        .insert((999, 10), vec![777]);

    // --- Scenario 1: Aug14 role-authorized msg8, crash before debounce flush, restore Sep5 ---
    let msg8 = make_test_msg(
        8,
        100,
        Some(999),
        10,
        "<@42> please execute work",
        "2026-08-14T12:00:00Z",
        vec![42],
    );
    fetcher.messages.lock().await.insert(100, vec![msg8]);

    let initial_cursor = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        initial_cursor, None,
        "crash before debounce flush must not leave an advanced cursor"
    );

    let backfilled_count =
        run_missed_message_backfill_with_fetcher(&pool, fetcher.as_ref(), &data, UserId::new(42))
            .await
            .unwrap();

    assert_eq!(
        backfilled_count, 1,
        "Aug14 role-authorized msg8 must be backfilled on restore"
    );

    {
        let dispatched = tokio::time::timeout(std::time::Duration::from_secs(5), ran_rx.recv())
            .await
            .expect("msg8 dispatch must be observed within 5s");
        assert_eq!(
            dispatched.as_deref(),
            Some("8"),
            "msg8 must be dispatched exactly once"
        );
        let evs = runner.events.lock().await;
        assert_eq!(evs.len(), 1, "msg8 must be dispatched exactly once");
        let expected_aug14 = chrono::DateTime::parse_from_rfc3339("2026-08-14T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            evs[0].received_at, expected_aug14,
            "msg8 must preserve Aug14 source timestamp, not Sep5 restore time"
        );
        assert_eq!(evs[0].platform_message_id, "8");
    }

    let cursor_after_8 = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        cursor_after_8.as_deref(),
        Some("8"),
        "Bot 42 cursor must advance to 8 after successful claim"
    );

    // --- Scenario 2: msg9 where REST metadata lookup fails must NOT be admitted as DM ---
    let msg9 = make_test_msg(
        9,
        100,
        Some(999),
        10,
        "<@42> message with metadata fail",
        "2026-08-14T12:05:00Z",
        vec![42],
    );
    {
        let mut msg_map = fetcher.messages.lock().await;
        msg_map.get_mut(&100).unwrap().push(msg9);
    }
    fetcher.channels.lock().await.insert(
        100,
        Err("Discord REST API 500 internal server error".to_string()),
    );

    let _ =
        run_missed_message_backfill_with_fetcher(&pool, fetcher.as_ref(), &data, UserId::new(42))
            .await;

    {
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(300), ran_rx.recv(),)
                .await
                .is_err(),
            "msg9 must NOT be dispatched when channel metadata fails"
        );
        let evs = runner.events.lock().await;
        assert_eq!(
            evs.len(),
            1,
            "msg9 must NOT be admitted or dispatched when channel metadata fails"
        );
        assert!(
            evs.iter().all(|e| e.session.guild_id.is_some()),
            "msg9 must NOT be admitted as Private/DM"
        );
    }
    let cursor_after_9_fail = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        cursor_after_9_fail.as_deref(),
        Some("8"),
        "cursor must NOT advance on metadata failure"
    );

    fetcher
        .channels
        .lock()
        .await
        .insert(100, Ok(make_guild_channel(100, 999, 0)));

    // --- Scenario 3: Two bots scanning the same channel: no cross-bot cursor skip ---
    let cursor_bot84 = get_bot_channel_cursor(&pool, "84", "100").await.unwrap();
    assert_eq!(
        cursor_bot84, None,
        "Bot 84 must not inherit Bot 42's cursor"
    );

    let msg9_for_bot84 = make_test_msg(
        9,
        100,
        Some(999),
        10,
        "<@84> task for bot 84",
        "2026-08-14T12:06:00Z",
        vec![84],
    );
    {
        let mut msg_map = fetcher.messages.lock().await;
        let m8 = msg_map.get(&100).unwrap()[0].clone();
        msg_map.insert(100, vec![m8, msg9_for_bot84]);
    }

    let mut data_bot84 = data.clone();
    data_bot84.primary_bot_id = Some(84);

    let count_bot84 = run_missed_message_backfill_with_fetcher(
        &pool,
        fetcher.as_ref(),
        &data_bot84,
        UserId::new(84),
    )
    .await
    .unwrap();

    assert_eq!(
        count_bot84, 1,
        "Bot 84 must scan from its own start and process its message"
    );
    {
        let dispatched = tokio::time::timeout(std::time::Duration::from_secs(5), ran_rx.recv())
            .await
            .expect("bot 84 msg9 dispatch must be observed within 5s");
        assert_eq!(
            dispatched.as_deref(),
            Some("9"),
            "bot 84 msg9 must be dispatched exactly once"
        );
    }
    let cursor_bot84_after = get_bot_channel_cursor(&pool, "84", "100").await.unwrap();
    assert_eq!(
        cursor_bot84_after.as_deref(),
        Some("9"),
        "Bot 84 cursor must be 9"
    );
    let cursor_bot42_still_8 = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        cursor_bot42_still_8.as_deref(),
        Some("8"),
        "Bot 42 cursor must remain unaffected by Bot 84"
    );

    // --- Scenario 4: Failed-route retry ---
    let msg10 = make_test_msg(
        10,
        100,
        Some(999),
        10,
        "<@42> task that fails initially",
        "2026-08-14T12:10:00Z",
        vec![42],
    );
    {
        let mut msg_map = fetcher.messages.lock().await;
        msg_map.get_mut(&100).unwrap().push(msg10);
    }
    *runner.fail_channel_id.lock().await = Some("100".to_string());

    let _ =
        run_missed_message_backfill_with_fetcher(&pool, fetcher.as_ref(), &data, UserId::new(42))
            .await;

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), ran_rx.recv(),)
            .await
            .is_err(),
        "msg10 must NOT reach the runner while the route is failing"
    );

    let cursor_retry = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        cursor_retry.as_deref(),
        Some("8"),
        "cursor must not advance on route failure"
    );

    *runner.fail_channel_id.lock().await = None;
    let retry_count =
        run_missed_message_backfill_with_fetcher(&pool, fetcher.as_ref(), &data, UserId::new(42))
            .await
            .unwrap();

    assert_eq!(
        retry_count, 1,
        "failed route message 10 must be retried and succeed"
    );
    {
        let dispatched = tokio::time::timeout(std::time::Duration::from_secs(5), ran_rx.recv())
            .await
            .expect("msg10 retry dispatch must be observed within 5s");
        assert_eq!(
            dispatched.as_deref(),
            Some("10"),
            "msg10 must be dispatched exactly once on retry"
        );
    }
    let cursor_retry_done = get_bot_channel_cursor(&pool, "42", "100").await.unwrap();
    assert_eq!(
        cursor_retry_done.as_deref(),
        Some("10"),
        "cursor must advance to 10 after successful retry"
    );

    // --- Scenario 5: >50 messages pagination coverage ---
    runner.events.lock().await.clear();
    fetcher
        .channels
        .lock()
        .await
        .insert(200, Ok(make_guild_channel(200, 999, 0)));
    let mut msgs_200 = Vec::new();
    for id in 101..=175 {
        msgs_200.push(make_test_msg(
            id,
            200,
            Some(999),
            10,
            "<@42> paginated batch item",
            "2026-08-14T13:00:00Z",
            vec![42],
        ));
    }
    fetcher.messages.lock().await.insert(200, msgs_200);

    let count_paginated =
        run_missed_message_backfill_with_fetcher(&pool, fetcher.as_ref(), &data, UserId::new(42))
            .await
            .unwrap();

    assert_eq!(
        count_paginated, 75,
        "all 75 messages across multiple pages must be backfilled"
    );
    for _ in 0..75 {
        tokio::time::timeout(std::time::Duration::from_secs(5), ran_rx.recv())
            .await
            .expect("paginated dispatch must be observed within 5s per message");
    }
    {
        let evs = runner.events.lock().await;
        let ids: Vec<String> = evs.iter().map(|e| e.platform_message_id.clone()).collect();
        assert_eq!(
            evs.len(),
            75,
            "all 75 paginated messages must reach the runner; got ids: {ids:?}"
        );
    }
    let cursor_200 = get_bot_channel_cursor(&pool, "42", "200").await.unwrap();
    assert_eq!(
        cursor_200.as_deref(),
        Some("175"),
        "cursor must advance to 175 covering all paginated pages"
    );
}

#[tokio::test]
async fn reply_parent_attachment_reaches_prompt() {
    let bot_id = UserId::new(42);
    let config = InboundFilterConfig {
        allowed_users: &[10],
        primary_bot_id: Some(42),
        ..Default::default()
    };

    let reply_msg = reply_message_fixture(
        None,
        "analyze this",
        "bob",
        "Here is the chart",
        vec![("chart.png", "image/png")],
    );

    let event =
        message_to_inbound_with_config(&reply_msg, bot_id, Some(ChannelType::Private), &config)
            .expect("reply message should convert to inbound event");

    // D04 assertion: event.attachments must include the parent message's referenced attachments
    assert!(
        event.attachments.iter().any(|a| a.id == "100" && a.filename == "chart.png"),
        "parent message attachment 'chart.png' (id 100) must be included in event.attachments, got: {:?}",
        event.attachments
    );
}

#[tokio::test]
async fn processing_lifecycle_cleans_up() {
    let egress = DiscordEgress::new(std::sync::Arc::new(serenity::all::Http::new("local-test")));
    let session = SessionKey::new("discord", Some("guild-1"), "7", None::<String>, "user-1");

    // 1. Start typing: guard is active
    egress
        .dispatch(OutboundAction::Typing {
            session: session.clone(),
            active: true,
        })
        .await
        .unwrap();
    assert_eq!(
        egress.active_typing_count().await,
        1,
        "typing guard must be active"
    );

    // 2. Terminal Typing(false) cleans up active guard
    egress
        .dispatch(OutboundAction::Typing {
            session: session.clone(),
            active: false,
        })
        .await
        .unwrap();
    assert_eq!(
        egress.active_typing_count().await,
        0,
        "terminal typing false must release active guard"
    );

    // 3. React action targets original parent channel_id (not thread_id)
    let thread_session = SessionKey::new(
        "discord",
        Some("guild-1"),
        "100",
        Some("200".to_string()),
        "",
    );
    let react_action = OutboundAction::React {
        session: thread_session,
        message_id: "999".to_string(),
        emoji: "✅".to_string(),
        remove_others: true,
    };
    let _ = egress.dispatch(react_action).await;
}

#[tokio::test]
async fn final_output_filters_controls() {
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    struct CapturingDispatcher {
        actions: Arc<Mutex<Vec<OutboundAction>>>,
    }
    #[async_trait]
    impl OutboundDispatcher for CapturingDispatcher {
        async fn dispatch(&self, action: OutboundAction) -> Result<(), omon_gateway::OmonError> {
            self.actions.lock().await.push(action);
            Ok(())
        }
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let peer_handle = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let mut ws = match tokio_tungstenite::accept_async(socket).await {
                Ok(ws) => ws,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                while let Some(Ok(Message::Text(text))) = ws.next().await {
                    let Ok(req) = serde_json::from_str::<Value>(&text) else {
                        continue;
                    };
                    let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
                    let method = req.get("method").and_then(Value::as_str).unwrap_or("");

                    match method {
                        "initialize" => {
                            let resp = json!({"jsonrpc":"2.0","id":id,"result":{}});
                            let _ = ws.send(Message::text(resp.to_string())).await;
                        }
                        "thread/resume" | "thread/start" => {
                            let thread_id = req["params"]["threadId"].as_str().unwrap_or("th-1");
                            let resp = json!({"jsonrpc":"2.0","id":id,"result":{"thread":{"id":thread_id}}});
                            let _ = ws.send(Message::text(resp.to_string())).await;
                        }
                        "turn/start" => {
                            let thread_id = req["params"]["threadId"].as_str().unwrap_or("th-1");
                            let resp = json!({"jsonrpc":"2.0","id":id,"result":{"turn":{"id":"turn-1","status":"inProgress"}}});
                            let _ = ws.send(Message::text(resp.to_string())).await;

                            let started = json!({
                                "jsonrpc": "2.0",
                                "method": "turn/started",
                                "params": { "threadId": thread_id, "turnId": "turn-1" }
                            });
                            let _ = ws.send(Message::text(started.to_string())).await;

                            if thread_id == "th-think" {
                                let item_started = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/started",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": { "id": "m1", "type": "agentMessage" }
                                    }
                                });
                                let _ = ws.send(Message::text(item_started.to_string())).await;

                                let item_completed = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/completed",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": {
                                            "id": "m1",
                                            "type": "agentMessage",
                                            "text": "<ThInK>PRIVATE</ThInK>answer"
                                        }
                                    }
                                });
                                let _ = ws.send(Message::text(item_completed.to_string())).await;

                                let completed = json!({
                                    "jsonrpc": "2.0",
                                    "method": "turn/completed",
                                    "params": {
                                        "threadId": thread_id,
                                        "turn": { "id": "turn-1", "status": "completed" }
                                    }
                                });
                                let _ = ws.send(Message::text(completed.to_string())).await;
                            } else if thread_id == "th-silence" {
                                let tool_started = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/started",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": { "id": "t1", "type": "toolCall", "tool": "terminal" }
                                    }
                                });
                                let _ = ws.send(Message::text(tool_started.to_string())).await;
                                let tool_completed = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/completed",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": { "id": "t1", "type": "toolCall", "tool": "terminal" }
                                    }
                                });
                                let _ = ws.send(Message::text(tool_completed.to_string())).await;

                                let item_started = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/started",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": { "id": "m2", "type": "agentMessage" }
                                    }
                                });
                                let _ = ws.send(Message::text(item_started.to_string())).await;

                                let item_completed = json!({
                                    "jsonrpc": "2.0",
                                    "method": "item/completed",
                                    "params": {
                                        "threadId": thread_id,
                                        "turnId": "turn-1",
                                        "item": {
                                            "id": "m2",
                                            "type": "agentMessage",
                                            "text": "NO_REPLY"
                                        }
                                    }
                                });
                                let _ = ws.send(Message::text(item_completed.to_string())).await;

                                let completed = json!({
                                    "jsonrpc": "2.0",
                                    "method": "turn/completed",
                                    "params": {
                                        "threadId": thread_id,
                                        "turn": { "id": "turn-1", "status": "completed" }
                                    }
                                });
                                let _ = ws.send(Message::text(completed.to_string())).await;
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
    });

    let config = omon_gateway::OmoBackendConfig::new(format!("ws://127.0.0.1:{port}"))
        .with_request_timeout(std::time::Duration::from_secs(5))
        .with_total_timeout(std::time::Duration::from_secs(10));

    let actions = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = Arc::new(CapturingDispatcher {
        actions: actions.clone(),
    });

    let backend = omon_gateway::OmoBackend::new(config, dispatcher);

    // 1. Turn 1: <ThInK>PRIVATE</ThInK>answer -> only "answer" emitted, NO "PRIVATE"
    let session_key1 = SessionKey::new("discord", Some("g1"), "c1", None::<String>, "u1");
    let mut session1 = omon_gateway::SessionContext::new(session_key1.clone());
    session1.state.metadata.insert(
        "omo_thread_id".to_string(),
        serde_json::Value::String("th-think".to_string()),
    );
    let ev1 = InboundEvent::message(session_key1, "1", "hello");

    backend.run(&mut session1, ev1).await.unwrap();

    let acts1 = actions.lock().await.clone();
    assert!(!acts1.is_empty(), "Turn 1 must emit output");
    let mut full_output = String::new();
    for act in &acts1 {
        if let OutboundAction::Stream { chunk, .. } = act {
            if chunk.is_final {
                full_output = chunk.content.clone();
            }
        }
    }
    assert!(
        !full_output.contains("PRIVATE"),
        "output must not contain PRIVATE reasoning, got: {full_output}"
    );
    assert!(
        !full_output.to_lowercase().contains("think"),
        "output must not contain think tags, got: {full_output}"
    );
    assert!(
        full_output.contains("answer"),
        "output must contain answer, got: {full_output}"
    );

    // Clear actions for Turn 2
    actions.lock().await.clear();

    // 2. Turn 2: tool event + NO_REPLY -> success, but ZERO sends (no stream chunks, no SendMessage)
    let session_key2 = SessionKey::new("discord", Some("g1"), "c2", None::<String>, "u2");
    let mut session2 = omon_gateway::SessionContext::new(session_key2.clone());
    session2.state.metadata.insert(
        "omo_thread_id".to_string(),
        serde_json::Value::String("th-silence".to_string()),
    );
    let ev2 = InboundEvent::message(session_key2, "2", "silent please");

    let res2 = backend.run(&mut session2, ev2).await;
    assert!(res2.is_ok(), "silence turn must succeed");

    let acts2 = actions.lock().await.clone();
    let sent_count = acts2
        .iter()
        .filter(|a| {
            matches!(
                a,
                OutboundAction::SendMessage { .. } | OutboundAction::Stream { .. }
            )
        })
        .count();
    assert_eq!(
        sent_count, 0,
        "silence turn must result in zero sends, got: {acts2:?}"
    );

    peer_handle.abort();
}

#[tokio::test]
async fn dead_target_is_identity_and_resource_scoped() {
    use omon_gateway::DeadTargetRegistry;

    let registry = DeadTargetRegistry::new();
    registry.mark_dead_for_bot("bot-a", 7, 403, "HTTP 403: Missing Permissions");

    assert!(
        registry.is_dead_for_bot("bot-a", 7),
        "bot-a must be dead on channel 7"
    );
    assert!(
        !registry.is_dead_for_bot("bot-b", 7),
        "bot-b must NOT be dead on channel 7 when only bot-a was marked dead"
    );

    let probe_registry = DeadTargetRegistry::new().with_probe_interval(std::time::Duration::ZERO);
    probe_registry.mark_dead_for_bot("bot-a", 99, 404, "HTTP 404: Unknown Channel");
    assert!(
        !probe_registry.is_dead_for_bot("bot-a", 99),
        "virtual expiry probe must permit one trial probe send"
    );
    assert!(
        probe_registry.is_dead_for_bot("bot-a", 99),
        "probe send must lock target again until trial outcome"
    );

    let client = Arc::new(serenity::all::Http::new("token-a"));
    let egress = DiscordEgress::new(client).with_dead_targets(Arc::new(registry));
    let session = SessionKey::new("discord", None::<String>, "7", None::<String>, "user");
    let send_res = egress
        .dispatch(OutboundAction::SendMessage {
            session,
            content: "hello".into(),
            reply_to: None,
        })
        .await;
    assert!(
        send_res.is_err(),
        "short-circuited dead target must return Err without claiming delivered"
    );
}

#[tokio::test]
async fn final_stream_references_trigger() {
    #[derive(Clone)]
    struct ReferenceCapturingTransport {
        sent_refs: Arc<Mutex<Vec<Option<MessageId>>>>,
        fail_references: Arc<Mutex<Vec<MessageId>>>,
    }

    #[async_trait]
    impl DiscordMessageTransport for ReferenceCapturingTransport {
        async fn start_typing(&self, _channel_id: ChannelId) -> Result<()> {
            Ok(())
        }
        async fn edit_message(
            &self,
            _channel_id: ChannelId,
            _message_id: MessageId,
            _content: String,
        ) -> Result<()> {
            Ok(())
        }
        async fn send_message(
            &self,
            _channel_id: ChannelId,
            _content: String,
        ) -> Result<MessageId> {
            self.sent_refs.lock().await.push(None);
            Ok(MessageId::new(100))
        }
        async fn send_message_with_reference(
            &self,
            channel_id: ChannelId,
            content: String,
            reference: Option<MessageId>,
        ) -> Result<MessageId> {
            self.sent_refs.lock().await.push(reference);
            if let Some(ref_id) = reference {
                if self.fail_references.lock().await.contains(&ref_id) {
                    return self.send_message(channel_id, content).await;
                }
            }
            Ok(MessageId::new(100))
        }
        async fn delete_message(
            &self,
            _channel_id: ChannelId,
            _message_id: MessageId,
        ) -> Result<()> {
            Ok(())
        }
    }

    let sent_refs = Arc::new(Mutex::new(Vec::new()));
    let fail_references = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(ReferenceCapturingTransport {
        sent_refs: sent_refs.clone(),
        fail_references: fail_references.clone(),
    });

    let http = Arc::new(serenity::all::Http::new("test-token"));
    let egress = DiscordEgress::new(http).with_message_transport(transport);

    // 1. Turn 1: trigger8 -> first request must have message_reference.message_id = 8, no reference on further chunks
    let session1 = SessionKey::new("discord", Some("g1"), "7", None::<String>, "u1");
    let stream_id1 = uuid::Uuid::new_v4();

    let chunk1 = omon_gateway::StreamChunk {
        stream_id: stream_id1,
        sequence: 0,
        content: "intermediate".into(),
        is_final: false,
        reply_to: Some("8".into()),
    };
    egress
        .dispatch(OutboundAction::Stream {
            session: session1.clone(),
            chunk: chunk1,
        })
        .await
        .unwrap();

    let chunk1_final = omon_gateway::StreamChunk {
        stream_id: stream_id1,
        sequence: 1,
        content: "completed answer".into(),
        is_final: true,
        reply_to: Some("8".into()),
    };
    egress
        .dispatch(OutboundAction::Stream {
            session: session1,
            chunk: chunk1_final,
        })
        .await
        .unwrap();

    let refs = sent_refs.lock().await.clone();
    assert_eq!(
        refs,
        vec![Some(MessageId::new(8))],
        "first message must reference triggering message_id 8, and no reference on further chunks"
    );

    // Clear recorded references for Turn 2
    sent_refs.lock().await.clear();

    // 2. Turn 2: deleted8 -> first attempt returns 404 (code 10008), retries without anchor
    fail_references.lock().await.push(MessageId::new(888));
    let session2 = SessionKey::new("discord", Some("g1"), "7", None::<String>, "u2");
    let stream_id2 = uuid::Uuid::new_v4();

    let chunk2 = omon_gateway::StreamChunk {
        stream_id: stream_id2,
        sequence: 0,
        content: "answer for deleted trigger".into(),
        is_final: true,
        reply_to: Some("888".into()),
    };
    let res2 = egress
        .dispatch(OutboundAction::Stream {
            session: session2,
            chunk: chunk2,
        })
        .await;
    assert!(
        res2.is_ok(),
        "deleted8 turn must succeed via retry fallback"
    );

    let refs2 = sent_refs.lock().await.clone();
    assert_eq!(
        refs2,
        vec![Some(MessageId::new(888)), None],
        "deleted8 must first attempt with reference, then retry without anchor"
    );
}

#[tokio::test]
async fn final_media_directive_uploads() {
    let temp_dir = tempfile::tempdir().unwrap();
    let valid_media_path = temp_dir.path().join("report one.png");
    std::fs::write(&valid_media_path, b"fake png content").unwrap();

    let uploader = Arc::new(MockFileUploader::default());
    let (typing_tx, _typing_rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });

    let egress = DiscordEgress::new(Arc::new(serenity::all::Http::new("test-token")))
        .with_file_uploader(uploader.clone())
        .with_message_transport(transport.clone());

    let session = SessionKey::new("discord", Some("g1"), "7", None::<String>, "u1");
    let stream_id = uuid::Uuid::new_v4();

    // 1. Valid temp file with spaces in path inside quotes
    let directive = format!(
        "Here is your report:\nMEDIA:\"{}\"",
        valid_media_path.display()
    );
    let chunk = omon_gateway::StreamChunk {
        stream_id,
        sequence: 0,
        content: directive,
        is_final: true,
        reply_to: None,
    };

    let result = egress
        .dispatch(OutboundAction::Stream {
            session: session.clone(),
            chunk,
        })
        .await;
    assert!(result.is_ok(), "valid media upload must succeed");

    // Check uploader was called exactly once with the valid path
    let upload_calls = uploader.calls.lock().await.clone();
    assert_eq!(
        upload_calls.len(),
        1,
        "uploader must be called exactly once"
    );
    let upload_call_path =
        std::fs::canonicalize(&upload_calls[0].1).unwrap_or_else(|_| upload_calls[0].1.clone());
    let expected_media_path =
        std::fs::canonicalize(&valid_media_path).unwrap_or_else(|_| valid_media_path.clone());
    assert_eq!(upload_call_path, expected_media_path);

    // Check transport received edited text with the MEDIA directive stripped (no literal directive)
    let calls = transport.calls.lock().await.clone();
    let sent_or_edited_texts: Vec<String> = calls
        .iter()
        .filter_map(|c| match c {
            Call::Edit(_, text) => Some(text.clone()),
            Call::Send(text) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        sent_or_edited_texts
            .iter()
            .any(|t| t.contains("Here is your report:") && !t.contains("MEDIA:")),
        "delivered text must have MEDIA directive stripped, got: {:?}",
        sent_or_edited_texts
    );

    // 2. Invalid / unauthorized root path produces explicit delivery error and NO secret upload
    let unauthorized_path = "/etc/passwd";
    let bad_directive = format!("Steal this:\nMEDIA:\"{unauthorized_path}\"");
    let bad_chunk = omon_gateway::StreamChunk {
        stream_id: uuid::Uuid::new_v4(),
        sequence: 0,
        content: bad_directive,
        is_final: true,
        reply_to: None,
    };

    let bad_result = egress
        .dispatch(OutboundAction::Stream {
            session,
            chunk: bad_chunk,
        })
        .await;

    assert!(
        bad_result.is_err(),
        "unauthorized media path must produce explicit delivery error"
    );
    // Uploader call count must still be 1 (no new secret upload)
    assert_eq!(uploader.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn native_voice_wire_metadata_and_fallback() {
    use omon_gateway::{
        DiscordFileUploader, DiscordUploadTransport, SerenityFileUploader, VoiceMetadata,
        DISCORD_VOICE_MESSAGE_FLAG,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone, Debug, PartialEq)]
    enum CapturedWireCall {
        Voice {
            filename: String,
            flags: u64,
            duration_secs: f64,
            waveform: String,
        },
        Ordinary {
            filename: String,
        },
        Forum {
            filename: String,
            is_voice: bool,
            flags: u64,
        },
    }

    struct SurrogateTransport {
        calls: Mutex<Vec<CapturedWireCall>>,
        reject_native_voice: AtomicBool,
    }

    #[async_trait]
    impl DiscordUploadTransport for SurrogateTransport {
        async fn send_voice_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            filename: &str,
            _bytes: Vec<u8>,
            meta: &VoiceMetadata,
        ) -> omon_gateway::Result<()> {
            if self.reject_native_voice.load(Ordering::SeqCst) {
                return Err(omon_gateway::OmonError::Multiplexer(
                    "Discord API 400 Bad Request: Voice notes disabled in channel".into(),
                ));
            }
            // Surrogate rejects flagged payload lacking metadata
            if meta.flags != DISCORD_VOICE_MESSAGE_FLAG
                || meta.duration_secs <= 0.0
                || meta.waveform.is_empty()
            {
                return Err(omon_gateway::OmonError::Multiplexer(
                    "Discord API 400 Bad Request: Voice message requires duration_secs and waveform metadata".into(),
                ));
            }
            self.calls.lock().await.push(CapturedWireCall::Voice {
                filename: filename.to_owned(),
                flags: meta.flags,
                duration_secs: meta.duration_secs,
                waveform: meta.waveform.clone(),
            });
            Ok(())
        }

        async fn send_ordinary_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            filename: &str,
            _bytes: Vec<u8>,
        ) -> omon_gateway::Result<()> {
            self.calls.lock().await.push(CapturedWireCall::Ordinary {
                filename: filename.to_owned(),
            });
            Ok(())
        }

        async fn send_forum_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            filename: &str,
            _bytes: Vec<u8>,
            is_voice: bool,
        ) -> omon_gateway::Result<()> {
            self.calls.lock().await.push(CapturedWireCall::Forum {
                filename: filename.to_owned(),
                is_voice,
                flags: 0,
            });
            Ok(())
        }

        async fn send_attachments(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            _attachments: Vec<serenity::all::CreateAttachment>,
        ) -> omon_gateway::Result<()> {
            Ok(())
        }
    }

    let surrogate = Arc::new(SurrogateTransport {
        calls: Mutex::new(Vec::new()),
        reject_native_voice: AtomicBool::new(false),
    });

    let uploader = SerenityFileUploader::new().with_transport(surrogate.clone());
    let http = Arc::new(serenity::all::Http::new("test-token"));
    let channel = ChannelId::new(12345);

    let temp_dir = tempfile::tempdir().unwrap();

    // 1. Valid OggOpus with explicit voice intent -> native voice metadata present and accepted
    let voice_path = temp_dir.path().join("my-voice-note.voice.ogg");
    tokio::fs::write(&voice_path, vec![1, 2, 3, 4, 5, 6, 7, 8])
        .await
        .unwrap();
    uploader
        .upload(http.clone(), channel, &voice_path)
        .await
        .expect("voice note should upload with valid metadata");

    let calls = surrogate.calls.lock().await.clone();
    assert_eq!(calls.len(), 1);
    match &calls[0] {
        CapturedWireCall::Voice {
            flags,
            duration_secs,
            waveform,
            ..
        } => {
            assert_eq!(*flags, DISCORD_VOICE_MESSAGE_FLAG);
            assert!(*duration_secs > 0.0);
            assert!(!waveform.is_empty());
        }
        _ => panic!("expected Voice wire call, got {:?}", calls[0]),
    }

    // 2. Injected native rejection falls back to ordinary file
    surrogate.reject_native_voice.store(true, Ordering::SeqCst);
    let rejected_voice_path = temp_dir.path().join("another-voice-message.ogg");
    tokio::fs::write(&rejected_voice_path, vec![10, 20, 30])
        .await
        .unwrap();
    uploader
        .upload(http.clone(), channel, &rejected_voice_path)
        .await
        .expect("native rejection should fall back to ordinary file");

    let calls = surrogate.calls.lock().await.clone();
    assert_eq!(calls.len(), 2);
    match &calls[1] {
        CapturedWireCall::Ordinary { filename } => {
            assert_eq!(filename, "another-voice-message.ogg");
        }
        _ => panic!("expected Ordinary fallback wire call, got {:?}", calls[1]),
    }

    // 3. Ordinary .ogg without voice intent remains document (never flagged as voice)
    surrogate.reject_native_voice.store(false, Ordering::SeqCst);
    let ordinary_path = temp_dir.path().join("soundtrack.ogg");
    tokio::fs::write(&ordinary_path, vec![99, 100, 101])
        .await
        .unwrap();
    uploader
        .upload(http.clone(), channel, &ordinary_path)
        .await
        .expect("ordinary ogg should upload as document");

    let calls = surrogate.calls.lock().await.clone();
    assert_eq!(calls.len(), 3);
    match &calls[2] {
        CapturedWireCall::Ordinary { filename } => {
            assert_eq!(filename, "soundtrack.ogg");
        }
        _ => panic!(
            "expected Ordinary wire call for ordinary .ogg, got {:?}",
            calls[2]
        ),
    }
}

#[tokio::test]
async fn tables_survive_forum_and_attachment_limits() {
    use omon_gateway::{
        DiscordEgress, DiscordUploadTransport, OutboundAction, OutboundDispatcher, SessionKey,
        VoiceMetadata, DISCORD_ATTACHMENT_LIMIT,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MockTableTransport {
        uploaded_batches: Mutex<Vec<Vec<String>>>,
        fail_upload: AtomicBool,
    }

    #[async_trait]
    impl DiscordUploadTransport for MockTableTransport {
        async fn send_voice_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            _filename: &str,
            _bytes: Vec<u8>,
            _meta: &VoiceMetadata,
        ) -> omon_gateway::Result<()> {
            Ok(())
        }

        async fn send_ordinary_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            _filename: &str,
            _bytes: Vec<u8>,
        ) -> omon_gateway::Result<()> {
            Ok(())
        }

        async fn send_forum_file(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            _filename: &str,
            _bytes: Vec<u8>,
            _is_voice: bool,
        ) -> omon_gateway::Result<()> {
            Ok(())
        }

        async fn send_attachments(
            &self,
            _http: &serenity::all::Http,
            _channel: ChannelId,
            attachments: Vec<serenity::all::CreateAttachment>,
        ) -> omon_gateway::Result<()> {
            if self.fail_upload.load(Ordering::SeqCst) {
                return Err(omon_gateway::OmonError::Multiplexer(
                    "Simulated Discord attachment upload failure".into(),
                ));
            }
            // Discord rejects requests exceeding 10 attachments
            if attachments.len() > DISCORD_ATTACHMENT_LIMIT {
                return Err(omon_gateway::OmonError::Multiplexer(format!(
                    "Discord 400 Bad Request: Request has {} attachments, limit is {}",
                    attachments.len(),
                    DISCORD_ATTACHMENT_LIMIT
                )));
            }
            let names: Vec<String> = attachments.iter().map(|a| a.filename.clone()).collect();
            self.uploaded_batches.lock().await.push(names);
            Ok(())
        }
    }

    let mock_transport = Arc::new(MockTableTransport {
        uploaded_batches: Mutex::new(Vec::new()),
        fail_upload: AtomicBool::new(false),
    });

    let http = Arc::new(serenity::all::Http::new("test-token"));
    let (typing_tx, _typing_rx) = tokio::sync::mpsc::unbounded_channel();
    let msg_transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: typing_tx,
    });
    let egress = DiscordEgress::new(http)
        .with_upload_transport(mock_transport.clone())
        .with_message_transport(msg_transport);

    // 1. Build markdown content containing 11 distinct tables
    let mut eleven_tables = String::new();
    for i in 1..=11 {
        eleven_tables.push_str(&format!(
            "\n### Table {i}\n| Column A | Column B |\n| --- | --- |\n| Value {i}A | Value {i}B |\n"
        ));
    }

    let session = SessionKey::new("discord", Some("g1"), "7", None::<String>, "u1");
    let stream_id = uuid::Uuid::new_v4();

    // 2. Test Stream delivery of 11 tables
    let stream_res = egress
        .dispatch(OutboundAction::Stream {
            session: session.clone(),
            chunk: omon_gateway::StreamChunk {
                stream_id,
                sequence: 0,
                content: eleven_tables.clone(),
                is_final: true,
                reply_to: None,
            },
        })
        .await;

    // Delivery must succeed AND all 11 tables must be received across valid batches (<= 10)
    assert!(
        stream_res.is_ok(),
        "11 table stream should succeed with batching: {:?}",
        stream_res.err()
    );
    let batches = mock_transport.uploaded_batches.lock().await.clone();
    assert!(
        batches.len() >= 2,
        "11 tables must be split across at least 2 batches of <= 10, got {} batches: {:?}",
        batches.len(),
        batches
    );
    let total_uploaded: usize = batches.iter().map(|b| b.len()).sum();
    assert_eq!(total_uploaded, 11, "All 11 table images must be uploaded");
    assert!(
        batches.iter().all(|b| b.len() <= DISCORD_ATTACHMENT_LIMIT),
        "Every batch must be <= 10 attachments"
    );

    // 3. Test forced upload failure must NOT be acknowledged as fully delivered
    mock_transport.fail_upload.store(true, Ordering::SeqCst);
    let fail_res = egress
        .dispatch(OutboundAction::Stream {
            session: session.clone(),
            chunk: omon_gateway::StreamChunk {
                stream_id: uuid::Uuid::new_v4(),
                sequence: 0,
                content: "| Col 1 | Col 2 |\n| --- | --- |\n| X | Y |\n".into(),
                is_final: true,
                reply_to: None,
            },
        })
        .await;

    assert!(
        fail_res.is_err(),
        "Forced upload failure must be propagated, not swallowed as success"
    );
}

#[tokio::test]
async fn voice_note_transcription_is_wired() {
    use omon_gateway::{
        AttachmentDownloader, AudioFrame, AudioPayload, MessageAttachment, SpeechToText,
    };

    struct MockSttProvider {
        received_payloads: Mutex<Vec<AudioPayload>>,
        fail_next: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl SpeechToText for MockSttProvider {
        async fn transcribe(&self, frames: &[AudioFrame]) -> omon_gateway::Result<String> {
            if self.fail_next.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(omon_gateway::OmonError::Multiplexer(
                    "STT provider temporary failure".into(),
                ));
            }
            if let Some(frame) = frames.first() {
                self.received_payloads
                    .lock()
                    .await
                    .push(frame.payload.clone());
                match &frame.payload {
                    AudioPayload::Pcm(samples) => Ok(format!("wav samples: {}", samples.len())),
                    AudioPayload::Opus(_) => Ok("ogg transcription successful".to_string()),
                }
            } else {
                Ok(String::new())
            }
        }
    }

    let temp_dir = tempfile::tempdir().unwrap();
    let mock_stt = Arc::new(MockSttProvider {
        received_payloads: Mutex::new(Vec::new()),
        fail_next: std::sync::atomic::AtomicBool::new(false),
    });

    let downloader = AttachmentDownloader::new(temp_dir.path())
        .unwrap()
        .with_stt(mock_stt.clone());

    // 1. OGG voice note
    let ogg_bytes = vec![0x4f, 0x67, 0x67, 0x53, 1, 2, 3, 4];
    let ogg_path = downloader
        .attachment_root()
        .join("att_1-my-voice-note.voice.ogg");
    tokio::fs::write(&ogg_path, &ogg_bytes).await.unwrap();
    let mut att_ogg = MessageAttachment {
        id: "att_1".into(),
        filename: "my-voice-note.voice.ogg".into(),
        url: "https://cdn.discordapp.com/voice.ogg".into(),
        content_type: Some("audio/ogg".into()),
        size_bytes: Some(ogg_bytes.len() as u64),
        local_path: None,
        text_content: None,
    };

    downloader.hydrate(&mut att_ogg).await.unwrap();
    assert_eq!(
        att_ogg.text_content.as_deref(),
        Some("[Voice message transcription]: ogg transcription successful")
    );

    // 2. WAV voice note - must decode to PCM
    // Build a minimal valid 16-bit mono 16000Hz PCM WAV header + 4 samples
    let mut wav_bytes = Vec::new();
    wav_bytes.extend_from_slice(b"RIFF");
    wav_bytes.extend_from_slice(&44u32.to_le_bytes()); // size
    wav_bytes.extend_from_slice(b"WAVE");
    wav_bytes.extend_from_slice(b"fmt ");
    wav_bytes.extend_from_slice(&16u32.to_le_bytes()); // subchunk1size
    wav_bytes.extend_from_slice(&1u16.to_le_bytes()); // audio format = 1 (PCM)
    wav_bytes.extend_from_slice(&1u16.to_le_bytes()); // num channels = 1
    wav_bytes.extend_from_slice(&16000u32.to_le_bytes()); // sample rate
    wav_bytes.extend_from_slice(&32000u32.to_le_bytes()); // byte rate
    wav_bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav_bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav_bytes.extend_from_slice(b"data");
    wav_bytes.extend_from_slice(&8u32.to_le_bytes()); // data size (8 bytes = 4 samples)
    wav_bytes.extend_from_slice(&100i16.to_le_bytes());
    wav_bytes.extend_from_slice(&200i16.to_le_bytes());
    wav_bytes.extend_from_slice(&(-300i16).to_le_bytes());
    wav_bytes.extend_from_slice(&400i16.to_le_bytes());

    let wav_path = downloader
        .attachment_root()
        .join("att_2-recording.voice.wav");
    tokio::fs::write(&wav_path, &wav_bytes).await.unwrap();
    let mut att_wav = MessageAttachment {
        id: "att_2".into(),
        filename: "recording.voice.wav".into(),
        url: "https://cdn.discordapp.com/recording.wav".into(),
        content_type: Some("audio/wav".into()),
        size_bytes: Some(wav_bytes.len() as u64),
        local_path: None,
        text_content: None,
    };

    downloader.hydrate(&mut att_wav).await.unwrap();
    assert_eq!(
        att_wav.text_content.as_deref(),
        Some("[Voice message transcription]: wav samples: 4")
    );

    // Verify mock received AudioPayload::Pcm, not Opus!
    let payloads = mock_stt.received_payloads.lock().await.clone();
    assert_eq!(payloads.len(), 2);
    match &payloads[1] {
        AudioPayload::Pcm(samples) => {
            assert_eq!(samples.len(), 4);
            assert_eq!(samples[0], 100);
            assert_eq!(samples[1], 200);
            assert_eq!(samples[2], -300);
            assert_eq!(samples[3], 400);
        }
        _ => panic!("Expected AudioPayload::Pcm for WAV, got {:?}", payloads[1]),
    }

    // 3. Provider failure surfaces neutral failure metadata
    mock_stt
        .fail_next
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let failed_path = downloader
        .attachment_root()
        .join("att_3-failed-voice.voice.ogg");
    tokio::fs::write(&failed_path, &ogg_bytes).await.unwrap();
    let mut att_failed = MessageAttachment {
        id: "att_3".into(),
        filename: "failed-voice.voice.ogg".into(),
        url: "https://cdn.discordapp.com/failed.ogg".into(),
        content_type: Some("audio/ogg".into()),
        size_bytes: Some(ogg_bytes.len() as u64),
        local_path: None,
        text_content: None,
    };

    downloader.hydrate(&mut att_failed).await.unwrap();
    assert_eq!(
        att_failed.text_content.as_deref(),
        Some("[Voice message: audio received (transcription unavailable)]")
    );
}

#[test]
fn slash_skill_read_unicode_and_long_output() {
    use omon_gateway::{chunk_slash_reply, skill_read_preview};

    // 1. 1799 ASCII bytes + 3-byte '한' + tail
    let mut input = "a".repeat(1799);
    input.push('한');
    input.push_str(" additional text tail");

    let preview = skill_read_preview(&input);
    assert!(preview.len() <= 1800);
    assert_eq!(preview, &"a".repeat(1799));

    // 2. >2000-char list chunking
    let long_list = (1..=100)
        .map(|i| {
            format!(
                "- Skill number {i:03}: detailed capability description for testing chunked output"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(long_list.len() > 2000);

    let chunks = chunk_slash_reply(&long_list, 2000);
    assert!(chunks.len() >= 2);
    assert!(chunks.iter().all(|c| c.len() <= 2000));
}

#[tokio::test]
async fn completed_response_has_bounded_message_count() {
    use omon_gateway::{
        DiscordMessageTransport, LiveEditThrottler, MAX_SPLIT_MESSAGES, TRUNCATION_NOTICE,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Clone, Default)]
    struct MockThrottlerTransport {
        sent_messages: Arc<Mutex<Vec<(ChannelId, String)>>>,
        edited_messages: Arc<Mutex<Vec<(ChannelId, MessageId, String)>>>,
        deleted_messages: Arc<Mutex<Vec<(ChannelId, MessageId)>>>,
        next_id: Arc<AtomicU64>,
    }

    #[async_trait]
    impl DiscordMessageTransport for MockThrottlerTransport {
        async fn send_message(
            &self,
            channel: ChannelId,
            content: String,
        ) -> omon_gateway::Result<MessageId> {
            let id = MessageId::new(self.next_id.fetch_add(1, Ordering::SeqCst) + 1);
            self.sent_messages.lock().await.push((channel, content));
            Ok(id)
        }

        async fn edit_message(
            &self,
            channel: ChannelId,
            message_id: MessageId,
            content: String,
        ) -> omon_gateway::Result<()> {
            self.edited_messages
                .lock()
                .await
                .push((channel, message_id, content));
            Ok(())
        }

        async fn delete_message(
            &self,
            channel: ChannelId,
            message_id: MessageId,
        ) -> omon_gateway::Result<()> {
            self.deleted_messages
                .lock()
                .await
                .push((channel, message_id));
            Ok(())
        }

        async fn start_typing(&self, _channel: ChannelId) -> omon_gateway::Result<()> {
            Ok(())
        }

        async fn send_message_with_reference(
            &self,
            channel: ChannelId,
            content: String,
            _reference_message_id: Option<MessageId>,
        ) -> omon_gateway::Result<MessageId> {
            self.send_message(channel, content).await
        }
    }

    let transport = Arc::new(MockThrottlerTransport::default());
    let throttler =
        LiveEditThrottler::new(transport.clone(), ChannelId::new(999), MessageId::new(100));

    // Build oversized content that splits into 12 distinct chunks (> 8)
    let paragraph = "This is a detailed analysis section of the system architecture.\n".repeat(30);
    let huge_content = (1..=12)
        .map(|i| format!("\n\n# Section {i}\n{paragraph}"))
        .collect::<Vec<_>>()
        .join("");

    throttler.update(&huge_content, true).await.unwrap();

    let sent = transport.sent_messages.lock().await.clone();
    let edited = transport.edited_messages.lock().await.clone();
    let total_messages = edited.len() + sent.len();
    assert_eq!(
        total_messages, MAX_SPLIT_MESSAGES,
        "Total response messages must be bounded to MAX_SPLIT_MESSAGES ({MAX_SPLIT_MESSAGES}), got {total_messages}"
    );
    let last_chunk = &sent.last().unwrap().1;
    assert!(
        last_chunk.contains(TRUNCATION_NOTICE),
        "Last chunk must contain truncation notice, got: {last_chunk}"
    );
}

#[tokio::test]
async fn runtime_footer_toggle_is_wired() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: tx,
    });
    let http = Arc::new(serenity::all::Http::new("token"));

    // Case 1: with_runtime_footer(true)
    let egress_on = DiscordEgress::new(http.clone())
        .with_message_transport(transport.clone())
        .with_runtime_footer(true)
        .with_default_model("model_x".into())
        .with_workspace_root(PathBuf::from("/tmp/test_workspace"));

    let session = SessionKey::new("discord", Some("9"), "7", None::<String>, "10");

    egress_on
        .dispatch(OutboundAction::SendMessage {
            session: session.clone(),
            content: "Hello world".into(),
            reply_to: None,
        })
        .await
        .unwrap();

    let calls = transport.calls.lock().await.clone();
    let sent_on = calls
        .iter()
        .find_map(|c| match c {
            Call::Send(s) => Some(s.clone()),
            _ => None,
        })
        .expect("Message must be sent");
    assert!(sent_on.contains("Hello world"));
    assert!(sent_on.contains("_model_x · /tmp/test_workspace_"));

    // Case 2: with_runtime_footer(false)
    let (tx2, _rx2) = mpsc::unbounded_channel();
    let transport2 = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: tx2,
    });
    let egress_off = DiscordEgress::new(http)
        .with_message_transport(transport2.clone())
        .with_runtime_footer(false);

    egress_off
        .dispatch(OutboundAction::SendMessage {
            session,
            content: "Hello world".into(),
            reply_to: None,
        })
        .await
        .unwrap();

    let calls2 = transport2.calls.lock().await.clone();
    let sent_off = calls2
        .iter()
        .find_map(|c| match c {
            Call::Send(s) => Some(s.clone()),
            _ => None,
        })
        .expect("Message must be sent");
    assert_eq!(sent_off, "Hello world");
}

#[tokio::test]
async fn destructive_slash_waits_for_confirmation() {
    use omon_gateway::{MultiplexerConfig, PoiseData, SessionMultiplexer};

    struct NoopRunner;
    #[async_trait]
    impl omon_gateway::AgentRunner for NoopRunner {
        async fn run(
            &self,
            _session: &mut omon_gateway::SessionContext,
            _event: omon_gateway::InboundEvent,
        ) -> Result<(), omon_gateway::OmonError> {
            Ok(())
        }
    }

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let runner = Arc::new(NoopRunner);
    let multiplexer =
        SessionMultiplexer::new(db.pool().clone(), runner, MultiplexerConfig::default());
    let mut data = PoiseData::new(multiplexer, db.pool().clone());
    data.destructive_slash_confirm = true;

    let session = SessionKey::new("discord", Some("9"), "7", None::<String>, "10");

    // Insert dummy session and message to be cleared
    sqlx::query("INSERT INTO sessions (session_key, platform, channel_id, user_id, state_json) VALUES (?, 'discord', '7', '10', '{}')")
        .bind(session.storage_key())
        .execute(db.pool())
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO messages (id, session_key, role, content) VALUES (?, ?, 'user', 'hello')",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(session.storage_key())
    .execute(db.pool())
    .await
    .unwrap();

    // 1. Without confirmation: must NOT delete and must return NeedsConfirmation
    let res1 = omon_gateway::discord::commands::execute_reset_command(&data, &session, None)
        .await
        .unwrap();
    assert_eq!(
        res1,
        omon_gateway::discord::commands::ResetCommandResult::NeedsConfirmation,
        "Must require confirmation before executing destructive slash command"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_key = ?")
        .bind(session.storage_key())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(
        count, 1,
        "Messages must NOT be deleted without confirmation"
    );

    // 2. With confirmation: must delete and return Executed
    let res2 = omon_gateway::discord::commands::execute_reset_command(&data, &session, Some(true))
        .await
        .unwrap();
    assert_eq!(
        res2,
        omon_gateway::discord::commands::ResetCommandResult::Executed
    );
    let count2: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE session_key = ?")
        .bind(session.storage_key())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count2, 0, "Messages must be deleted after confirmation");
}

#[tokio::test]
async fn slash_controls_change_authoritative_session() {
    use omon_gateway::discord::commands::execute_model_command;
    use omon_gateway::discord::PoiseData;
    use omon_gateway::{
        AgentRunner, Database, InboundEvent, MultiplexerConfig, OmonError, SessionContext,
        SessionKey, SessionMultiplexer,
    };

    type RunRecord = (Option<String>, Option<String>);
    struct CapturingRunner {
        runs: Arc<tokio::sync::Mutex<Vec<RunRecord>>>,
    }

    #[async_trait]
    impl AgentRunner for CapturingRunner {
        async fn run(
            &self,
            session: &mut SessionContext,
            _event: InboundEvent,
        ) -> Result<(), OmonError> {
            let model = session.state.active_model.clone();
            let thread_id = session
                .state
                .metadata
                .get("omo_thread_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            self.runs.lock().await.push((model, thread_id));
            session
                .state
                .metadata
                .insert("omo_thread_id".into(), serde_json::json!("t1"));
            Ok(())
        }
    }

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let runs = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let runner = Arc::new(CapturingRunner { runs: runs.clone() });
    let multiplexer =
        SessionMultiplexer::new(db.pool().clone(), runner, MultiplexerConfig::default());
    let data = PoiseData::new(multiplexer.clone(), db.pool().clone());
    let session = SessionKey::new("discord", Some("9"), "7", None::<String>, "10");

    let ev1 = InboundEvent::message(session.clone(), "m1", "turn 1");
    multiplexer.route_awaiting_turn(ev1).await.unwrap();

    execute_model_command(&data, &session, "model-2")
        .await
        .unwrap();

    let ev2 = InboundEvent::message(session.clone(), "m2", "turn 2");
    multiplexer.route_awaiting_turn(ev2).await.unwrap();

    let runs_snapshot = runs.lock().await.clone();
    assert_eq!(
        runs_snapshot[1].0.as_deref(),
        Some("model-2"),
        "Turn 2 must observe updated active model"
    );
    assert_eq!(
        runs_snapshot[1].1.as_deref(),
        Some("t1"),
        "Turn 2 reuses thread t1 before reset"
    );

    omon_gateway::discord::commands::execute_reset_command(&data, &session, Some(true))
        .await
        .unwrap();

    let ev3 = InboundEvent::message(session.clone(), "m3", "turn 3");
    multiplexer.route_awaiting_turn(ev3).await.unwrap();

    let runs_final = runs.lock().await.clone();
    assert_eq!(
        runs_final[2].1, None,
        "Turn 3 must have a fresh thread after authoritative reset"
    );
}

#[tokio::test]
async fn reconnect_replays_only_owned_transport_failures() {
    let pool = omon_gateway::storage::init_pool("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let bot_a = "bot-a";
    let bot_b = "bot-b";

    let session_a =
        SessionKey::new("discord", None::<String>, "101", None::<String>, "u1").with_bot_id(bot_a);
    let session_b =
        SessionKey::new("discord", None::<String>, "102", None::<String>, "u2").with_bot_id(bot_b);

    let ledger = DeliveryLedgerService::new(pool.clone());

    // 1. Bot A failed row: retryable connection refused (current owner)
    let obl_a_conn = "obl:test:a-conn";
    ledger
        .record_obligation(obl_a_conn, &session_a, "Retryable payload A")
        .await
        .unwrap();
    ledger
        .mark_obligation_failed(obl_a_conn, "connection refused: 127.0.0.1:443")
        .await
        .unwrap();

    // 2. Bot A failed row: non-retryable timeout (current owner)
    let obl_a_timeout = "obl:test:a-timeout";
    ledger
        .record_obligation(obl_a_timeout, &session_a, "Timeout payload A")
        .await
        .unwrap();
    ledger
        .mark_obligation_failed(obl_a_timeout, "request timeout after 30s")
        .await
        .unwrap();

    // 3. Bot A failed row: non-retryable 403 Forbidden (current owner)
    let obl_a_forbidden = "obl:test:a-forbidden";
    ledger
        .record_obligation(obl_a_forbidden, &session_a, "Forbidden payload A")
        .await
        .unwrap();
    ledger
        .mark_obligation_failed(obl_a_forbidden, "HTTP 403: Missing Permissions")
        .await
        .unwrap();

    // 4. Bot B failed row: retryable connection refused (current owner, but bot B)
    let obl_b_conn = "obl:test:b-conn";
    ledger
        .record_obligation(obl_b_conn, &session_b, "Retryable payload B")
        .await
        .unwrap();
    ledger
        .mark_obligation_failed(obl_b_conn, "connection refused: 127.0.0.1:443")
        .await
        .unwrap();

    // 5. Another process instance row (different owner_started_at)
    let obl_other_proc = "obl:test:other-proc";
    ledger
        .record_obligation(obl_other_proc, &session_a, "Other proc payload")
        .await
        .unwrap();
    ledger
        .mark_obligation_failed(obl_other_proc, "connection refused: 127.0.0.1:443")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE delivery_obligations SET owner_started_at = '2026-01-01T00:00:00Z' WHERE id = ?",
    )
    .bind(obl_other_proc)
    .execute(&pool)
    .await
    .unwrap();

    // Set up mock Discord egress
    let (tx, _rx) = mpsc::unbounded_channel();
    let mock_transport = Arc::new(MockTransport {
        calls: Mutex::new(Vec::new()),
        typing: tx,
    });
    let client_a = Arc::new(serenity::all::Http::new("token-a"));
    let client_b = Arc::new(serenity::all::Http::new("token-b"));
    let mut clients = std::collections::HashMap::new();
    clients.insert(bot_a.to_string(), client_a);
    clients.insert(bot_b.to_string(), client_b);

    let egress = DiscordEgress::with_bot_clients(bot_a, clients)
        .unwrap()
        .with_message_transport(mock_transport.clone());

    // Trigger replay for bot A
    let replayed = egress
        .replay_failed_transport_obligations(bot_a, &pool)
        .await
        .expect("replay must succeed");

    assert_eq!(
        replayed, 1,
        "Exactly 1 obligation (bot A connection-refused) must be replayed"
    );

    // Verify DB state of all 5 rows:
    // 1. Bot A connection refused: now 'delivered'
    let row_a_conn = ledger.get_obligation(obl_a_conn).await.unwrap().unwrap();
    assert_eq!(
        row_a_conn.state, "delivered",
        "Bot A connection-refused must be marked delivered"
    );

    // 2. Bot A timeout: still 'failed'
    let row_a_timeout = ledger.get_obligation(obl_a_timeout).await.unwrap().unwrap();
    assert_eq!(
        row_a_timeout.state, "failed",
        "Bot A timeout must remain failed"
    );

    // 3. Bot A forbidden: still 'failed'
    let row_a_forbidden = ledger
        .get_obligation(obl_a_forbidden)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row_a_forbidden.state, "failed",
        "Bot A 403 must remain failed"
    );

    // 4. Bot B connection refused: still 'failed' (bot A sweep must not touch bot B)
    let row_b_conn = ledger.get_obligation(obl_b_conn).await.unwrap().unwrap();
    assert_eq!(
        row_b_conn.state, "failed",
        "Bot B must remain untouched by bot A replay"
    );

    // 5. Other process instance: still 'failed'
    let row_other = ledger
        .get_obligation(obl_other_proc)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row_other.state, "failed",
        "Other process instance must remain untouched"
    );

    // Verify mock transport received the message
    let calls = mock_transport.calls.lock().await;
    let sent_calls: Vec<_> = calls
        .iter()
        .filter_map(|c| {
            if let Call::Send(content) = c {
                Some(content.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        sent_calls.len(),
        1,
        "Mock transport must have received exactly 1 message"
    );
    assert_eq!(sent_calls[0], "Retryable payload A");
}

#[tokio::test]
async fn test_dis006_approval_scoped_dead_targets_and_persistence() {
    use omon_gateway::{
        storage::init_pool, DeadTargetRegistry, DiscordEgress, OutboundAction, OutboundDispatcher,
        SessionKey,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    let pool = init_pool("sqlite::memory:").await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    let registry = Arc::new(
        DeadTargetRegistry::new()
            .with_pool(pool.clone())
            .with_probe_interval(std::time::Duration::from_secs(60)),
    );

    // Bot A has marked dead target on channel 7
    registry.mark_dead_for_bot("bot-a", 7, 403, "HTTP 403: Forbidden");

    let http_a = Arc::new(serenity::all::Http::new("token-a"));
    let http_b = Arc::new(serenity::all::Http::new("token-b"));
    let clients = HashMap::from([("bot-a".to_string(), http_a), ("bot-b".to_string(), http_b)]);

    let egress = DiscordEgress::with_bot_clients("bot-a".to_string(), clients)
        .unwrap()
        .with_dead_targets(registry.clone());

    let session_a = SessionKey::new("discord", None::<String>, "7", None::<String>, "user-1")
        .with_bot_id("bot-a");
    let session_b = SessionKey::new("discord", None::<String>, "7", None::<String>, "user-2")
        .with_bot_id("bot-b");

    // 1. ApprovalRequest for bot-a: must be short-circuited because bot-a is dead on channel 7
    let err_a = egress
        .dispatch(OutboundAction::ApprovalRequest {
            session: session_a.clone(),
            request_id: uuid::Uuid::new_v4(),
            command: "cmd-a".into(),
            reason: "reason-a".into(),
        })
        .await;
    assert!(
        err_a.is_err(),
        "bot-a approval request must be short-circuited"
    );
    let err_msg = err_a.unwrap_err().to_string();
    assert!(
        err_msg.contains("approval target 7 is unavailable"),
        "Expected dead target error for bot-a, got: {err_msg}"
    );

    // 2. ApprovalRequest for bot-b: must NOT be short-circuited by bot-a's dead state!
    let res_b = egress
        .dispatch(OutboundAction::ApprovalRequest {
            session: session_b.clone(),
            request_id: uuid::Uuid::new_v4(),
            command: "cmd-b".into(),
            reason: "reason-b".into(),
        })
        .await;
    let err_b_msg = res_b.unwrap_err().to_string();
    assert!(
        !err_b_msg.contains("approval target 7 is unavailable"),
        "bot-b must not be suppressed by bot-a's dead state on channel 7; got {err_b_msg}"
    );

    // 3. Persistence: Recreate registry and egress over the SAME DB, call load_from_db
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let new_registry = Arc::new(
        DeadTargetRegistry::new()
            .with_pool(pool.clone())
            .with_probe_interval(std::time::Duration::from_secs(60)),
    );
    new_registry.load_from_db(&pool).await.unwrap();
    assert!(
        new_registry.is_dead_for_bot("bot-a", 7),
        "bot-a dead state on channel 7 must survive recreation over same DB"
    );
    assert!(
        !new_registry.is_dead_for_bot("bot-b", 7),
        "bot-b must remain not dead after recreation"
    );
}
