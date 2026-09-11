# Calendar Phase Formatting Evidence: Rust Edition 2021

**Worker Session:** hephaestus (st_01a077d1)  
**Parent Session:** 01a071f6-cc78-7761-86ab-a931f8c15133  
**Target Files:**
1. `src/cron/store.rs`
2. `tests/test_cron_schedule_parity.rs`
3. `tests/test_cron_boundary_parity.rs`

**Strict Scope Constraints:**
- Targeted strictly the three calendar phase files above.
- Did **NOT** touch `src/cron/scheduler.rs` or `tests/test_voice_cron.rs` (actively handled by worker U76).
- Did **NOT** modify `.env`, configuration, dependencies, or create git commits.
- Did **NOT** execute `cargo fmt --all`.
- Made zero code logic, test coverage, or assertion alterations; purely mechanical whitespace and import order corrections to satisfy `rustfmt --edition 2021`.

---

## 1. Root Cause Analysis & Pre-Formatting Discrepancies

The repository's root `Cargo.toml` explicitly declares:
```toml
[package]
name = "omo-gateway"
version = "0.1.0"
edition = "2021"
```

Previous workers formatted files using edition 2024 conventions, resulting in the following `rustfmt --check --edition 2021` discrepancies:

1. **`src/cron/store.rs`**:
   - `effective_timezone` let binding split over 5 lines instead of a single statement.
   - `next_run_at` let binding split across separate lines prior to the `match` block.
   - `created` let binding split across separate lines prior to `{`.
   - `imports_timezone_and_rejects_invalid_schedule` test: unwrapped `tokio::fs::create_dir`, `tokio::fs::write`, `assert_eq!`, and `query_as` lines exceeding edition 2021 width or chaining guidelines.
2. **`tests/test_cron_schedule_parity.rs`**:
   - Multi-line wrapping required for `instant(...)`, `tokio::fs::write`, `HermesStoreSynchronizer::new`, `CapturingExecutor`, `.with_clock(...)`, timeout recv calls, `sqlx::query_as`, and `assert_eq!`.
3. **`tests/test_cron_boundary_parity.rs`**:
   - Import order: `use serde_json::{Value, json};` was non-alphabetical. Edition 2021 requires `use serde_json::{json, Value};`.

---

## 2. Exact Minimal Patch

```diff
--- a/src/cron/store.rs
+++ b/src/cron/store.rs
@@ -461,5 +461,1 @@
-                let effective_timezone = job
-                    .schedule
-                    .timezone
-                    .as_deref()
-                    .or(timezone.as_deref());
+                let effective_timezone = job.schedule.timezone.as_deref().or(timezone.as_deref());
@@ -494,6 +490,2 @@
-                let next_run_at = match job
-                    .next_run_at
-                    .as_deref()
-                    .map(parse_timestamp)
-                    .transpose()
+                let next_run_at = match job.next_run_at.as_deref().map(parse_timestamp).transpose()
                 {
@@ -506,6 +498,1 @@
-                let created = match job
-                    .created_at
-                    .as_deref()
-                    .map(parse_timestamp)
-                    .transpose()
-                {
+                let created = match job.created_at.as_deref().map(parse_timestamp).transpose() {
@@ -557,2 +544,4 @@
-        tokio::fs::create_dir(root.path().join("cron")).await.unwrap();
+        tokio::fs::create_dir(root.path().join("cron"))
+            .await
+            .unwrap();
@@ -571,3 +560,7 @@
-            }]})).unwrap();
-            tokio::fs::write(sync.stores[0].jobs_path(), &bytes).await.unwrap();
+            }]}))
+            .unwrap();
+            tokio::fs::write(sync.stores[0].jobs_path(), &bytes)
+                .await
+                .unwrap();
@@ -579,4 +572,18 @@
-                    .fetch_all(database.pool()).await.unwrap();
+                    .fetch_all(database.pool())
+                    .await
+                    .unwrap();
             println!("invalid expression with next={next}: result={result:?}, rows={rows:?}");
-            assert_eq!(result.unwrap(), 0, "malformed schedules must be isolated from import, even with a stored next run");
-            assert!(rows.is_empty(), "invalid schedule must never become an enabled NULL row");
-            assert_eq!(tokio::fs::read(sync.stores[0].jobs_path()).await.unwrap(), bytes);
+            assert_eq!(
+                result.unwrap(),
+                0,
+                "malformed schedules must be isolated from import, even with a stored next run"
+            );
+            assert!(
+                rows.is_empty(),
+                "invalid schedule must never become an enabled NULL row"
+            );
+            assert_eq!(
+                tokio::fs::read(sync.stores[0].jobs_path()).await.unwrap(),
+                bytes
+            );
         }
-        tokio::fs::write(sync.stores[0].jobs_path(), serde_json::to_vec(&json!({"jobs": [{
-            "id": "daily", "schedule": {"kind": "cron", "expr": "0 9 * * *"}
-        }]})).unwrap()).await.unwrap();
+        tokio::fs::write(
+            sync.stores[0].jobs_path(),
+            serde_json::to_vec(&json!({"jobs": [{
+                "id": "daily", "schedule": {"kind": "cron", "expr": "0 9 * * *"}
+            }]}))
+            .unwrap(),
+        )
+        .await
+        .unwrap();
         assert_eq!(sync.sync_at(now).await.unwrap(), 1);
-        let (payload, next): (String, DateTime<Utc>) = sqlx::query_as(
-            "SELECT payload_json, next_run_at FROM cron_jobs"
-        ).fetch_one(database.pool()).await.unwrap();
+        let (payload, next): (String, DateTime<Utc>) =
+            sqlx::query_as("SELECT payload_json, next_run_at FROM cron_jobs")
+                .fetch_one(database.pool())
+                .await
+                .unwrap();
         assert_eq!(next, parse_timestamp("2026-09-06T00:00:00Z").unwrap());
-        assert_eq!(serde_json::from_str::<Value>(&payload).unwrap()["schedule"]["timezone"], "Asia/Seoul");
+        assert_eq!(
+            serde_json::from_str::<Value>(&payload).unwrap()["schedule"]["timezone"],
+            "Asia/Seoul"
+        );
--- a/tests/test_cron_schedule_parity.rs
+++ b/tests/test_cron_schedule_parity.rs
@@ -11,3 +11,5 @@
 fn instant(value: &str) -> DateTime<Utc> {
-    DateTime::parse_from_rfc3339(value).unwrap().with_timezone(&Utc)
+    DateTime::parse_from_rfc3339(value)
+        .unwrap()
+        .with_timezone(&Utc)
 }
@@ -44,2 +46,4 @@
-    tokio::fs::write(home.join("config.yaml"), "timezone: Asia/Seoul\n").await.unwrap();
+    tokio::fs::write(home.join("config.yaml"), "timezone: Asia/Seoul\n")
+        .await
+        .unwrap();
@@ -51,3 +55,7 @@
-    }]})).unwrap();
-    tokio::fs::write(home.join("cron/jobs.json"), &bytes).await.unwrap();
+    }]}))
+    .unwrap();
+    tokio::fs::write(home.join("cron/jobs.json"), &bytes)
+        .await
+        .unwrap();
@@ -58,2 +66,5 @@
-    let sync = HermesStoreSynchronizer::new(database.pool().clone(), vec![HermesStore::new("seoul", &home)]);
+    let sync = HermesStoreSynchronizer::new(
+        database.pool().clone(),
+        vec![HermesStore::new("seoul", &home)],
+    );
@@ -64,5 +75,13 @@
-        Arc::new(CapturingExecutor { started, clock: clock.clone(), completed_at: completion }),
+        Arc::new(CapturingExecutor {
+            started,
+            clock: clock.clone(),
+            completed_at: completion,
+        }),
         Arc::new(CapturingDispatcher(delivered)),
-    ).with_clock({ let clock = clock.clone(); move || *clock.lock().unwrap() });
+    )
+    .with_clock({
+        let clock = clock.clone();
+        move || *clock.lock().unwrap()
+    });
@@ -70,7 +89,17 @@
-    let executed = tokio::time::timeout(Duration::from_secs(5), executions.recv()).await.unwrap().unwrap();
+    let executed = tokio::time::timeout(Duration::from_secs(5), executions.recv())
+        .await
+        .unwrap()
+        .unwrap();
     assert_eq!(executed.id, "hermes:seoul:daily");
-    let action = tokio::time::timeout(Duration::from_secs(5), deliveries.recv()).await.unwrap().unwrap();
+    let action = tokio::time::timeout(Duration::from_secs(5), deliveries.recv())
+        .await
+        .unwrap()
+        .unwrap();
     match action {
-        OutboundAction::SendMessage { session, content, .. } => {
+        OutboundAction::SendMessage {
+            session, content, ..
+        } => {
             assert_eq!(session.channel_id, "42");
@@ -79,6 +108,10 @@
-    let notification = tokio::time::timeout(Duration::from_secs(5), notifications.recv()).await.unwrap().unwrap();
+    let notification = tokio::time::timeout(Duration::from_secs(5), notifications.recv())
+        .await
+        .unwrap()
+        .unwrap();
     assert_eq!(notification.triggered_at, completion);
-    tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown()).await.unwrap();
+    tokio::time::timeout(Duration::from_secs(5), scheduler.shutdown())
+        .await
+        .unwrap();
     let updated = scheduler.get(&executed.id).await.unwrap().unwrap();
-    let run: (String, DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
-        "SELECT status, started_at, completed_at FROM cron_runs WHERE job_id = ?"
-    ).bind(&executed.id).fetch_one(database.pool()).await.unwrap();
+    let run: (String, DateTime<Utc>, DateTime<Utc>) =
+        sqlx::query_as("SELECT status, started_at, completed_at FROM cron_runs WHERE job_id = ?")
+            .bind(&executed.id)
+            .fetch_one(database.pool())
+            .await
+            .unwrap();
     assert_eq!(run, ("succeeded".into(), due, completion));
-    println!("import/run/completion: status={}, completed_at={}, next={:?}", run.0, run.2, updated.next_run_at);
+    println!(
+        "import/run/completion: status={}, completed_at={}, next={:?}",
+        run.0, run.2, updated.next_run_at
+    );
     assert_eq!(updated.next_run_at, Some(instant("2026-09-06T00:00:00Z")));
-    assert_eq!(updated.payload().unwrap()["schedule"]["timezone"], "Asia/Seoul");
+    assert_eq!(
+        updated.payload().unwrap()["schedule"]["timezone"],
+        "Asia/Seoul"
+    );
@@ -95,2 +128,5 @@
-    let sync = HermesStoreSynchronizer::new(reopened.pool().clone(), vec![HermesStore::new("seoul", &home)]);
+    let sync = HermesStoreSynchronizer::new(
+        reopened.pool().clone(),
+        vec![HermesStore::new("seoul", &home)],
+    );
@@ -100,2 +136,4 @@
-            .fetch_one(reopened.pool()).await.unwrap();
+            .fetch_one(reopened.pool())
+            .await
+            .unwrap();
@@ -104,2 +142,5 @@
-    assert_eq!(tokio::fs::read(home.join("cron/jobs.json")).await.unwrap(), bytes);
+    assert_eq!(
+        tokio::fs::read(home.join("cron/jobs.json")).await.unwrap(),
+        bytes
+    );
--- a/tests/test_cron_boundary_parity.rs
+++ b/tests/test_cron_boundary_parity.rs
@@ -7,2 +7,2 @@
-use serde_json::{Value, json};
+use serde_json::{json, Value};
```

---

## 3. Verification Commands & Results

### 1. Scoped `rustfmt --check --edition 2021`
```bash
rustfmt --check --edition 2021 src/cron/store.rs tests/test_cron_schedule_parity.rs tests/test_cron_boundary_parity.rs
```
**Output:** (empty)  
**Exit code:** 0 (PASS)

Individual checks:
- `rustfmt --check --edition 2021 src/cron/store.rs`: Exit 0 (PASS)
- `rustfmt --check --edition 2021 tests/test_cron_schedule_parity.rs`: Exit 0 (PASS)
- `rustfmt --check --edition 2021 tests/test_cron_boundary_parity.rs`: Exit 0 (PASS)

### 2. Scoped `git --no-pager diff --check`
```bash
git --no-pager diff --check src/cron/store.rs tests/test_cron_schedule_parity.rs tests/test_cron_boundary_parity.rs
```
**Output:** (empty)  
**Exit code:** 0 (PASS)

### 3. LSP Diagnostics
- `src/cron/store.rs`: No diagnostics found (clean)
- `tests/test_cron_schedule_parity.rs`: No diagnostics found (clean)
- `tests/test_cron_boundary_parity.rs`: No diagnostics found (clean)

---

## 4. Scope & Work Invariant Verification
- Only `src/cron/store.rs`, `tests/test_cron_schedule_parity.rs`, and `tests/test_cron_boundary_parity.rs` were modified.
- `src/cron/scheduler.rs` and `tests/test_voice_cron.rs` remained completely untouched.
- No overall calendar acceptance is inferred beyond the mechanical formatting compliance of these three files under edition 2021.
