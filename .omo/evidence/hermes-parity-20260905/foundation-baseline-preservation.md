# Foundation protected baseline preservation

Reconstructed each bootstrap file in memory from current HEAD plus baseline.patch, with exact context/deletion validation. No working tree resets or file replacement occurred.

- src/agent/omo_config.rs is byte-identical to bootstrap (442 lines).
- src/agent/omo_backend.rs differs only by U12 acknowledgement identity and correlated frame acceptance/terminal handling. Original absolute deadline, stale grace and other user code remain present.
- tests/test_omo_backend.rs retains original user additions; U12 adds correlation regressions, gives existing fake frames actual thread/turn IDs, changes old uncorrelated-idle success to refusal, and replaces approval sleep with exact response. The removed 1100ms tool fixture wait is visible below; later U71 must ensure time-sensitive grace controls remain causal rather than silently losing coverage.

These are foundation-time preservation facts, not a final audit after future changes. All attributable deltas follow.

## src/agent/omo_backend.rs

```diff
*** Update File: src/agent/omo_backend.rs
@@
                 continue;
             };
 
-            if val.get("id").and_then(Value::as_u64) == Some(3) {
+            if val.get("method").is_none()
+                && val.get("id").and_then(Value::as_u64) == Some(3)
+                && !turn_started_ack
+            {
                 if let Some(error) = val.get("error") {
                     return Err(OmonError::Llm(format!("turn/start error: {error}")));
                 }
+                let id = val
+                    .pointer("/result/turn/id")
+                    .and_then(Value::as_str)
+                    .filter(|id| !id.is_empty())
+                    .ok_or_else(|| {
+                        OmonError::Llm("turn/start acknowledgement missing turn id".into())
+                    })?;
+                turn_id = Some(id.to_string());
                 turn_started_ack = true;
-                if let Some(id) = val.pointer("/result/turn/id").and_then(Value::as_str) {
-                    turn_id = Some(id.to_string());
-                }
                 continue;
             }
 
@@
                 }
             }
 
+            // Only the successful start response owns the active turn identity.
+            // Subscription replay (including thread idle without a turn ID) is
+            // not evidence that this turn has produced output or completed.
+            let frame_turn_id = val
+                .pointer("/params/turnId")
+                .or_else(|| val.pointer("/params/turn/id"))
+                .and_then(Value::as_str);
+            let turn_bearing = method.starts_with("turn/")
+                || method.starts_with("item/")
+                || method == "thread/status/changed"
+                || (method == "error"
+                    && (val.pointer("/params/turnId").is_some()
+                        || val.pointer("/params/turn").is_some()));
+            if turn_bearing
+                && (!turn_started_ack
+                    || val.pointer("/params/threadId").and_then(Value::as_str)
+                        != Some(thread_id.as_str())
+                    || frame_turn_id != turn_id.as_deref())
+            {
+                continue;
+            }
+
             match method.as_str() {
                 "turn/started" => {
                     approval_denials = 0;
-                    if let Some(tid) = val.pointer("/params/turnId").and_then(Value::as_str) {
-                        turn_id = Some(tid.to_string());
-                    }
                 }
                 "item/started" | "item/completed" => {
                     approval_denials = 0;
@@
                         }
                     }
                 }
-                "turn/completed" | "thread/status/changed" => {
-                    let is_terminal = if method == "turn/completed" {
-                        let is_for_current_thread = val
-                            .pointer("/params/threadId")
-                            .and_then(Value::as_str)
-                            .map(|id| id == thread_id)
-                            .unwrap_or(true);
-                        if !is_for_current_thread {
-                            continue;
+                "turn/completed" => {
+                    match val.pointer("/params/turn/status").and_then(Value::as_str) {
+                        Some("interrupted") => {
+                            return Err(OmonError::Llm("omo turn interrupted".into()));
                         }
-                        if val.pointer("/params/turn/status").and_then(Value::as_str)
-                            == Some("failed")
-                        {
+                        Some("failed") => {
                             let err_msg = val
                                 .pointer("/params/turn/error")
                                 .and_then(Value::as_str)
@@
                                 .unwrap_or("turn failed");
                             return Err(OmonError::Llm(format!("omo turn failed: {err_msg}")));
                         }
-                        true
-                    } else if method == "thread/status/changed" {
-                        let is_for_current_thread = val
-                            .pointer("/params/threadId")
-                            .and_then(Value::as_str)
-                            .map(|id| id == thread_id)
-                            .unwrap_or(false);
-                        let is_idle = val.pointer("/params/status/type").and_then(Value::as_str)
-                            == Some("idle");
-                        is_for_current_thread && is_idle
-                    } else {
-                        false
-                    };
-
-                    if !is_terminal {
-                        continue;
+                        Some("completed") => {}
+                        _ => continue,
                     }
 
                     let has_content = !full_content.is_empty() || total_tool_calls > 0;
```

## tests/test_omo_backend.rs

```diff
*** Update File: tests/test_omo_backend.rs
@@
 use tokio::net::TcpListener;
 use tokio_tungstenite::tungstenite::Message;
 
+// Public backend entry point over a real loopback WebSocket. Each script is a
+// new subscription to r1, so completed t1 frames can replay while t2 is active.
+async fn run_correlated_frames(frames: Vec<Value>) -> (omon_gateway::Result<()>, Vec<StreamChunk>) {
+    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
+    let address = listener.local_addr().unwrap();
+    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
+    let peer = tokio::spawn(async move {
+        let (socket, _) = listener.accept().await.unwrap();
+        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
+        for method in ["initialize", "thread/resume", "turn/start"] {
+            let message = ws.next().await.unwrap().unwrap();
+            let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
+            assert_eq!(request["method"], method);
+            if method != "initialize" {
+                assert_eq!(request["params"]["threadId"], "r1");
+            }
+            if method != "turn/start" {
+                ws.send(Message::text(
+                    json!({"jsonrpc":"2.0", "id":request["id"],
+                    "result":{"thread":{"id":"r1"}}})
+                    .to_string(),
+                ))
+                .await
+                .unwrap();
+            }
+        }
+        for frame in frames {
+            ws.feed(Message::text(frame.to_string())).await.unwrap();
+        }
+        ws.flush().await.unwrap();
+        stop_rx.await.unwrap();
+    });
+    let dispatcher = Arc::new(CapturingDispatcher::new());
+    let backend = OmoBackend::new(
+        OmoBackendConfig::new(format!("ws://{address}")),
+        dispatcher.clone(),
+    );
+    let key = SessionKey::new("local", None::<String>, "u12", None::<String>, "user");
+    let mut session = SessionContext::new(key.clone());
+    session
+        .state
+        .metadata
+        .insert("omo_thread_id".into(), json!("r1"));
+    let result = tokio::time::timeout(
+        std::time::Duration::from_secs(5),
+        backend.run(&mut session, InboundEvent::message(key, "u12", "probe")),
+    )
+    .await;
+    stop_tx.send(()).unwrap();
+    tokio::time::timeout(std::time::Duration::from_secs(5), peer)
+        .await
+        .unwrap()
+        .unwrap();
+    (
+        result.expect("bounded backend completion"),
+        dispatcher.stream_chunks(),
+    )
+}
+
+fn correlation_ack() -> Value {
+    json!({"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"t2","status":"inProgress"}}})
+}
+
+fn correlation_delta(thread: &str, turn: &str, text: &str) -> Value {
+    json!({"jsonrpc":"2.0","method":"item/agentMessage/delta",
+        "params":{"threadId":thread,"turnId":turn,"itemId":"m1","delta":text}})
+}
+
+fn correlation_terminal(turn: &str, status: &str) -> Value {
+    json!({"jsonrpc":"2.0","method":"turn/completed",
+        "params":{"threadId":"r1","turn":{"id":turn,"status":status,"error":{"message":"failure-sentinel"}}}})
+}
+
+#[tokio::test]
+async fn rejects_foreign_and_stale_turn_frames() {
+    let (result, chunks) = run_correlated_frames(vec![
+        correlation_ack(),
+        correlation_delta("r9", "t9", "SECRET"),
+        correlation_terminal("t1", "completed"),
+        correlation_delta("r1", "t2", "OK"),
+        correlation_terminal("t2", "completed"),
+    ])
+    .await;
+    assert!(result.is_ok(), "{result:?}");
+    assert_eq!(
+        chunks
+            .iter()
+            .filter(|c| c.is_final)
+            .map(|c| c.content.as_str())
+            .collect::<Vec<_>>(),
+        vec!["OK"]
+    );
+    assert!(chunks.iter().all(|c| !c.content.contains("SECRET")));
+}
+
+#[tokio::test]
+async fn correlation_requires_ack_and_ignores_subscription_replay() {
+    let replay = vec![
+        correlation_delta("r1", "t2", "SECRET"),
+        correlation_terminal("t2", "completed"),
+        correlation_ack(),
+        correlation_delta("r1", "t2", "O"),
+        json!({"method":"thread/status/changed","params":{"threadId":"r1","status":{"type":"idle"}}}),
+        json!({"method":"turn/started","params":{"threadId":"r1","turnId":"t1"}}),
+        correlation_delta("r1", "t1", "SECRET"),
+        json!({"method":"item/started","params":{"threadId":"r9","turnId":"t9","item":{"id":"tool","type":"commandExecution","command":"SECRET"}}}),
+        json!({"method":"item/completed","params":{"threadId":"r1","turnId":"t1","item":{"type":"agentMessage","text":"SECRET"}}}),
+        json!({"method":"turn/error","params":{"threadId":"r9","turnId":"t9","message":"SECRET"}}),
+        json!({"method":"error","params":{"threadId":"r1","turn":{"id":"t1"},"message":"SECRET"}}),
+        json!({"method":"item/agentMessage/delta","params":{"delta":"SECRET"}}),
+        json!({"method":"turn/completed","params":{"turn":{"id":"t2","status":"completed"}}}),
+        correlation_delta("r1", "t2", "K"),
+        correlation_terminal("t2", "completed"),
+    ];
+    let (result, chunks) = run_correlated_frames(replay).await;
+    assert!(result.is_ok(), "{result:?}");
+    assert_eq!(
+        chunks
+            .iter()
+            .filter(|c| c.is_final)
+            .map(|c| c.content.as_str())
+            .collect::<Vec<_>>(),
+        vec!["OK"]
+    );
+    assert!(chunks.iter().all(|c| !c.content.contains("SECRET")));
+}
+
+#[tokio::test]
+async fn correlation_distinguishes_interrupted_and_failed() {
+    for status in ["interrupted", "failed"] {
+        let (result, chunks) = run_correlated_frames(vec![
+            correlation_ack(),
+            correlation_delta("r1", "t2", "partial"),
+            correlation_terminal("t2", status),
+        ])
+        .await;
+        let error = result.expect_err(status);
+        assert!(error.to_string().contains(status), "{error}");
+        assert!(!chunks.iter().any(|c| c.is_final));
+    }
+}
+
 struct CapturingDispatcher {
     actions: ParkingMutex<Vec<OutboundAction>>,
 }
@@
                                         .send(Message::text(approval_req.to_string()))
                                         .await;
 
-                                    // Small delay to allow client to process approval request before ending turn
-                                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
+                                    // Observe the exact denial before publishing completion.
+                                    let response = tokio::time::timeout(
+                                        std::time::Duration::from_secs(5),
+                                        ws_stream.next(),
+                                    )
+                                    .await
+                                    .unwrap()
+                                    .unwrap()
+                                    .unwrap();
+                                    let response: Value =
+                                        serde_json::from_str(response.to_text().unwrap()).unwrap();
+                                    assert_eq!(response["id"], 999);
+                                    ar.lock().push(response);
 
                                     // Stream item/agentMessage/delta chunks
                                     let emit_activity = ea.load(Ordering::SeqCst);
@@
                     let _ = ws.send(Message::text(response.to_string())).await;
                     let intent = json!({
                         "jsonrpc":"2.0","method":"item/completed",
-                        "params":{"item":{"type":"agentMessage","id":"intent","text":"I read this as the digest task."}}
+                        "params":{"threadId":"thread-tool-sequence","turnId":"turn-tool-sequence","item":{"type":"agentMessage","id":"intent","text":"I read this as the digest task."}}
                     });
                     let _ = ws.send(Message::text(intent.to_string())).await;
                     let tool_started = json!({
                         "jsonrpc":"2.0","method":"item/started",
-                        "params":{"item":{"type":"commandExecution","id":"tool-1","command":"write digest","status":"inProgress"}}
+                        "params":{"threadId":"thread-tool-sequence","turnId":"turn-tool-sequence","item":{"type":"commandExecution","id":"tool-1","command":"write digest","status":"inProgress"}}
                     });
                     let _ = ws.send(Message::text(tool_started.to_string())).await;
 
-                    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
-
                     let tool_completed = json!({
                         "jsonrpc":"2.0","method":"item/completed",
-                        "params":{"item":{"type":"commandExecution","id":"tool-1","command":"write digest","status":"completed"}}
+                        "params":{"threadId":"thread-tool-sequence","turnId":"turn-tool-sequence","item":{"type":"commandExecution","id":"tool-1","command":"write digest","status":"completed"}}
                     });
                     let _ = ws.send(Message::text(tool_completed.to_string())).await;
                     let digest = json!({
                         "jsonrpc":"2.0","method":"item/completed",
-                        "params":{"item":{"type":"agentMessage","id":"digest","text":"## Actual digest body"}}
+                        "params":{"threadId":"thread-tool-sequence","turnId":"turn-tool-sequence","item":{"type":"agentMessage","id":"digest","text":"## Actual digest body"}}
                     });
                     let _ = ws.send(Message::text(digest.to_string())).await;
                     let idle = json!({
-                        "jsonrpc":"2.0","method":"thread/status/changed",
-                        "params":{"threadId":"thread-tool-sequence","status":{"type":"idle"}}
+                        "jsonrpc":"2.0","method":"turn/completed",
+                        "params":{"threadId":"thread-tool-sequence","turn":{"id":"turn-tool-sequence","status":"completed"}}
                     });
                     let _ = ws.send(Message::text(idle.to_string())).await;
                 }
@@
 }
 
 #[tokio::test]
-async fn test_omo_backend_completes_from_final_agent_message_when_turn_terminal_is_missing() {
+async fn test_omo_backend_requires_turn_terminal_even_after_final_agent_message() {
     let server = FakeAppServer::spawn_without_turn_completed().await;
     let config = OmoBackendConfig::new(format!("ws://127.0.0.1:{}", server.port))
         .with_request_timeout(std::time::Duration::from_millis(800));
@@
         .await;
 
     assert!(
-        result.is_ok(),
-        "missing terminal fallback failed: {result:?}"
+        result.is_err(),
+        "uncorrelated idle must not finalize: {result:?}"
     );
     let chunks = dispatcher.stream_chunks();
-    let final_chunk = chunks.last().expect("final fallback chunk");
-    assert!(final_chunk.is_final);
-    assert_eq!(final_chunk.content, "Hello World!");
+    assert!(!chunks.iter().any(|chunk| chunk.is_final));
 }
 
 #[tokio::test]
```
