# Lane: discord-auth
## Scope
- `src/discord/pairing.rs`: 1,069 LOC, read fully, including tests.
- `src/discord/approval.rs`: 1,204 LOC, read fully, including tests.

## Findings
### [P0] Approval resolution does not bind the clicker to the requesting session
- Location: src/discord/approval.rs:559
- Evidence: `pub async fn resolve_custom_id(&self, custom_id: &str) -> bool {`
- Why it matters: The resolver accepts only the public button custom ID, removes its pending entry, and sends the decision without checking the entry's stored session against an actor, guild, channel, or bot. The Discord interaction caller does check general bot access, but explicitly admits any paired user as well as any allowlisted user/role; it does not compare the clicker with the request owner. Consequently, paired user B who can see user A's guild approval message can approve, deny, or permanently allow A's pending operation. This is not a claim that an entirely unpaired, unallowlisted clicker bypasses the caller's check. The `Always` path also caches the resulting grant globally, not just for A or that guild.
- Suggested fix: Pass the authenticated actor and interaction context into resolution, compare them with the stored request session before removing it, and permit an override only for an explicitly configured approval-operator policy. Do not equate ordinary paired access with authority over every other user's approvals.

### [P0] Unauthorized DM notification records have no retention bound
- Location: src/discord/pairing.rs:370
- Evidence:
  ```rust
            "INSERT INTO pairing_notifications (user_id, last_notified_at)
             VALUES (?, ?)
             ON CONFLICT(user_id) DO UPDATE SET last_notified_at = excluded.last_notified_at
  ```
- Why it matters: With the default no-explicit-allowlist pairing policy, each new non-bot Discord user who sends an unauthorized DM creates a durable notification row. Code cleanup and the 100-code capacity limit operate only on `pairing_codes`; neither expiry, eviction, successful pairing, nor any repository call site deletes notification rows. Thus N distinct unsolicited senders leave N persistent rows forever, even if every corresponding code expires or is evicted. An unauthenticated population can cause unbounded database growth despite the advertised pending-code bound.
- Suggested fix: Prune notification rows whose timestamps are older than the rate-limit window during issuance/cleanup, and remove a user's notification row after successful pairing. Keep recent throttle reservations intact.

### [P0] Remembered session approvals grow without eviction
- Location: src/discord/approval.rs:468
- Evidence:
  ```rust
            .entry(session.clone())
            .or_default()
            .insert(pattern_key.to_string());
  ```
- Why it matters: Every distinct Session grant is retained in a process-wide map with no capacity, TTL, or automatic session-lifecycle eviction. The scanner-scoped path constructs keys from the entire command and reason, so distinct command arguments create distinct, potentially large entries even within a single session. Across sessions, map entries also accumulate. Only an explicit `clear_session` removes them; ordinary completion and prompt timeout do not. A long-running deployment with continuing Session approvals therefore has unbounded resident memory growth, independently of the pending-prompt lease cleanup.
- Suggested fix: Bound the remembered session cache by entry count/bytes and expire it with the session lifecycle. Evicting a remembered grant is safe because the next matching operation can prompt again; use a digest for exact command/reason identity rather than retaining entire raw strings.

### [P1] A resolved approval can resurrect grants after session clear
- Location: src/discord/approval.rs:388
- Evidence: `self.guard.approve_session(session, pattern_key).await;`
- Why it matters: A concrete interleaving is: resolve_custom_id removes an entry and sends Session; before its waiting requester resumes, clear_session runs to completion, finds no pending entry, and clears that session's grant cache; the old requester then resumes and inserts the Session grant here. Clear has returned successfully, but the old grant is restored and the old requester still returns approval. The Once decision similarly remains usable after removal from pending, and an Always decision can still be persisted after clear. There is no session generation or cancellation check between receipt, caching, the awaited UI-expiry delivery, and returning the grant. Clearing pending entries therefore does not revoke an already-resolved-but-not-yet-consumed authorization.
- Suggested fix: Associate requests and grants with a session generation. Increment it on clear and atomically reject stale generations when publishing a grant; carry that generation to the execution boundary so a clear between approval return and execution cannot use the old authorization.

### [P1] Always approval reports success even when persistence fails
- Location: src/discord/approval.rs:489
- Evidence: `tracing::warn!(%error, pattern = %pattern, "failed to persist always-allow approval");`
- Why it matters: approve_always inserts the pattern into always_cache before executing SQL, returns no Result, and only logs an INSERT failure. A full disk, read-only database, or closed pool therefore leaves a globally active in-memory grant while the requester returns Always as though the durable grant succeeded. Restart loses that allegedly permanent permission. Cancellation while the SQL is pending can produce the same cache/database divergence. Concurrent requests can observe the global grant even before persistence has completed.
- Suggested fix: Return a persistence Result, persist before publishing the global cache entry, and propagate failure to the requester rather than reporting a permanent grant. If persistence is unavailable, explicitly downgrade to Once only with a visible non-persistent outcome.

### [P1] Pairing can consume the code without publishing authorization to the cache
- Location: src/discord/pairing.rs:513
- Evidence: `self.paired_cache.write().await.insert(user_id);`
- Why it matters: The transaction has already committed the paired_users row and deleted the one-time code before this cancellable lock acquisition. If the approval future is cancelled after commit and before insertion, the live cache continues to reject the user indefinitely. Retrying the code cannot repair it because it was consumed; notification issuance also refuses to offer a replacement because it checks the database and sees that the user is already paired. Only cache initialization/restart or a separate new successful pairing repairs the divergence. Lock contention makes this an explicit await window rather than merely a theoretical instruction boundary.
- Suggested fix: Make committed cache publication cancellation-safe, for example by giving a separately owned operation responsibility for both commit and publication, or add database reconciliation on cache misses so durable paired state cannot remain invisible. Preserve the current rule against granting access before a successful commit.

### [P1] Pairing expiry is evaluated against stale request-start time
- Location: src/discord/pairing.rs:462
- Evidence: `if now > row.expires_at {`
- Why it matters: approve_code samples Utc::now before approve_code_at waits for the pool and performs its SQL reads. A valid code submitted just before its deadline can queue behind other work, expire, and still be accepted because this comparison uses the earlier timestamp. There is also an exact-boundary inconsistency: approval accepts now == expires_at while cleanup deletes expires_at <= now. Thus the one-hour expiry is not enforced at the actual claim boundary, and the result at equality depends on whether cleanup ran first.
- Suggested fix: Reject now >= expires_at and sample the production clock at the claim/check boundary after waiting for the connection. Make the consuming DELETE conditional on the same expiry check; preserve deterministic injected-clock support for tests without reusing a pre-queue production timestamp.

### [P1] Heartbeat wait can accept an approval after its deadline
- Location: src/discord/approval.rs:99
- Evidence:
  ```rust
                biased;
                res = &mut receiver => {
                    return match res {
  ```
- Why it matters: The pending entry has no approval deadline and resolve_custom_id never checks elapsed time. If the waiter is not polled across its deadline, a click can still find the pending entry and send a decision afterward. When the waiter resumes, both the receiver and deadline are ready; this biased select always chooses the decision first and returns approval rather than Timeout. Under executor contention, the configured timeout therefore does not reliably expire authority. The deadline starts only after prompt delivery as well, so request creation time is not an implicit substitute for this missing check.
- Suggested fix: Store the effective deadline with the pending request and check it when resolving under the pending lock. In the heartbeat waiter, reject an elapsed deadline before consuming a newly received approval, or include the decision's resolution timestamp if pre-deadline decisions must remain valid after delayed polling.

### [P2] Heartbeat tests still depend on scheduler timing
- Location: src/discord/approval.rs:1051
- Evidence: `beat_rx.try_recv().is_err(),`
- Why it matters: The test consumes two heartbeat events but does not prevent a third from being queued before it resolves the approval. After joining the correctly stopped producer, an already-buffered third event makes this assertion fail even though no heartbeat occurred after resolution. A sufficiently delayed test task can also hit the real 200 ms approval timeout before sending its decision. The adjacent requester-heartbeat test similarly assumes a callback must execute before a real 300 ms deadline; a delayed executor can legitimately select timeout first. These are timing-dependent test outcomes, not proof of a production heartbeat defect.
- Suggested fix: Use Tokio's controlled clock to advance to explicit heartbeat events, resolve before deliberately advancing to the deadline, and verify producer termination separately from already-buffered events. Bound event waits; drain/count pre-resolution events rather than requiring the buffer to be empty.

## Strengths
- Pairing uses parameterized SQL, transactionally consumes a code together with recording paired_users, and checks the DELETE row count. No demonstrated double-success replay exists; the production pool has one connection, so the read/modify/write transactions are serialized there.
- Pairing codes originate from UUID v4 randomness, are normalized for operator entry, have a one-hour expiry, and are protected by a persisted five-failure platform lockout. Active code capacity is explicitly bounded.
- Approval prompts use random UUIDs and one-shot senders; PendingLease removes pending state on timeout or dropped callers, and the delivery owner attempts to retire the Discord UI after terminal completion.
- Approval display values are redacted without replacing the raw executable arguments, and scanner-specific grants cap scope before caching rather than accidentally persisting a user-selected Always decision.

## Notes
- Review method: full source review plus explicit interleaving analysis was chosen over running Cargo tests because this lane may write only the evidence file; Cargo could create build artifacts or change dependency state. No tests, builds, or runtime Discord interactions were executed. Structural panic/unwrap, blocking-I/O, and TODO/FIXME/HACK scans were run on both complete targets. The initial optional caller scan mentioned a nonexistent handler.rs; actual call context was obtained from adapter.rs and commands.rs instead.
- Other files were read only to establish the target modules' callers, production SQLite pool configuration, schema, and permission policy. No findings are assigned to those files and no product file was modified.
- Pairing-store operator_id is ignored, but the actual /pair caller checks configured user/role authorization or allow_all before calling it. This review therefore does not claim unauthenticated self-pairing merely from the ignored parameter. The store itself offers no operator-policy enforcement or actor audit trail.
- Pairing authorization is global by Discord user ID, not limited to a DM or guild. The message and component callers treat paired membership as sufficient user authorization in either context, while channel admission is separate. Global Always grants likewise deliberately cross sessions (there is an explicit test for this). These scope choices are not counted again as standalone defects; if every paired user is intentionally a trusted global approval operator, the first finding becomes a documented delegation policy rather than a requester-isolation guarantee.
- The alphabet contains 32 symbols. UUID version bits constrain the seventh code character to 16 choices, leaving 39 random bits rather than an ideal 40. With the implemented online lockout, this alone does not prove a practical guessing attack, so it is not elevated to a finding.
- Public format_code can panic on an eight-byte Unicode string whose byte offset four is not a character boundary, but production callers format generated ASCII or stored codes, not raw operator input. Likewise, zero heartbeat intervals and concurrent cloned builder setters can violate public API preconditions, but current production construction uses a nonzero default and startup-only setters. No production-reachable panic is claimed from these cases.
- The persisted loaders merge into their caches instead of replacing them, and pairing cache reload can race with updates if called during service. Current production callers load these caches at startup; a live-reload/revocation bug was not asserted without a live-reload caller.
