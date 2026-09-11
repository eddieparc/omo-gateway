# Pinned upstream delta audit: Discord I/O

Status: COMPLETE - read-only executable source comparison, 2026-09-05. Child hephaestus, task st_01a070d9.

## Result and method

**56/56 assigned paths fetched successfully: 41 changed, 13 added, 2 byte-identical. Two additional applicable findings are confirmed below.** IO-01 affects live output. IO-02 is an additional recovery contract that becomes effective when the already-known live outbox wiring is implemented; it is not presented as a currently populated live outbox failure. Neither finding repeats the six installed-source audits' work lists.

Upstream references below are pinned to `f58fcc8118d9db092ad60d363d4a28520e08ac5a`, fetched from `https://raw.githubusercontent.com/NousResearch/hermes-agent/f58fcc8118d9db092ad60d363d4a28520e08ac5a/<path>`. Installed comparison root: `/Users/indo/.hermes/hermes-agent`; Rust root: `/Users/indo/code/project/omon-gateway`. Installed source, not a clean historical checkout, was compared. All six `approvals/config/cron/discord/runtime/sessions-audit.md` reports were read before deduplication.

Executed bounded HTTP waves (maximum 12 workers), Python AST parsing/symbol comparison and unified diffs; inspected changed Discord/base methods, extracted streaming modules, shared delivery/mirror/readiness behavior and immediate Rust callers/tests. AST differences were NOT treated as behavioral differences: extensive shortening, extracted helpers and plugin-compat reexports account for much of this pin. Non-Discord files received full fetch/parse and role/symbol inventory, not an invented Rust implementation requirement. Local source/caller bodies were inspected with the read tool; ast-grep confirmed the concrete Rust DiscordMessageTransport implementation. Programming skill, Rust README and Tokio reference were read. LSP infrastructure was reported down by the task and all baseline audits; it was not retried and no clean diagnostics are claimed.

Only this report was intentionally written. No source/test changes, commits, .env reads, daemon launches, Discord requests, production operations, cargo builds or cargo test executions. The tool's automatic overflow logs are incidental evidence, not source edits. Proposed regression names below do not exist yet and were not run. Two small in-memory evaluations were executed, explicitly distinguished from compiled-Rust verification below.

## Newly confirmed findings

### IO-01 - No per-response Discord message-count ceiling

**Status: applicable live output gap, P1.** This is a new upstream behavior, not old pagination or PNG attachment batching (Discord D16).

- Upstream `plugins/platforms/discord/adapter.py:996` sets `MAX_SPLIT_MESSAGES = 8`; `:2787-2807` keeps the first seven chunks and replaces the remainder with an explicit truncation notice. `:2809-2895` calls that cap after normal formatting/chunking in `send`. Installed `plugins/platforms/discord/adapter.py:2869-2921` splits and sends every chunk with no corresponding cap.
- Rust `src/discord/throttler.rs:166-180` edits/sends every result from `chunk_markdown`; `:277-350` terminates only when the entire input has been consumed. `src/discord/adapter.rs:1842-1880` routes actual completed OMO output through that throttler; SendMessage independently loops all chunks at `:2005-2045`. `src/agent/omo_backend.rs:627-677` sends full content, with no final message-count ceiling, then persists it.
- Impact: a degenerate 60,698-character final produces at least 31 Discord content messages, potentially more with pagination/fence overhead. Final-only does not prevent this flood: every message belongs to one completed turn. This is different from the already-reported 11-PNG attachment limit. Existing `tests/test_discord_adapter.rs:91-112,194-246` checks per-message length/fences and continuation delivery, not a whole-response ceiling.
- Minimal implementation boundary: a bounded final-delivery chunk plan shared by `DiscordEgress::stream`/LiveEditThrottler and SendMessage (including forum continuation). Keep normal under-limit bytes, balanced fences, safe mentions and PNG rendering. Preserve the full original response in durable history; final delivery must explicitly disclose truncation rather than falsely implying all bytes reached Discord. Do not port upstream table-to-bullets or preview streaming. Do not globally truncate the daemon's authoritative history. Count content messages separately from the D16 PNG upload groups.
- Exact deterministic RED/GREEN regression (proposed): `cargo test --test test_discord_adapter completed_response_has_bounded_message_count -- --exact`. Given a 60,698-character ASCII final and pagination enabled explicitly in fixture configuration, dispatch one final Stream via real DiscordEgress to a loopback REST surrogate that records POST/PATCH bodies and returns unique message IDs. Await dispatch completion with a bounded timeout; no sleeps. RED: more than eight final content messages. GREEN: at most eight, all <=2,000 characters, explicit truncation disposition, durable original content length exactly 60,698. Parameterize the same test fixture for SendMessage and forum destination, plus a short response control and a fenced-code overflow control. The initial zero-width Stream placeholder is not a separate final content message but its final edited body counts. Tests should assert counts, machine-visible truncation status and preserved payload, not fixed notice prose.
- Local surface scenario: loopback OMO peer emits one completed 60,698-character answer through real backend/dispatcher, while an isolated SQLite store and REST surrogate retain original text and emitted messages. Zero interim prose; the final stream respects the ceiling; typing lifecycle remains the already-owned D09/R11 contract. No production Discord.
- Executed corroboration: AST-extracted pinned `_cap_split_chunks` evaluated in memory with 31 chunks and the verified constant 8 produced exactly 8 chunks. This proves the upstream helper outcome only. Rust's unlimited caller loop is source-proven; no Rust runtime result is claimed.

### IO-02 - No same-process, bot-scoped retry of definitively failed transport delivery

**Status: applicable recovery extension, P1; dependent on cron C01 / sessions F13 live outbox wiring.** Existing reports cover missing obligation producers, boot recovery, false ACK, owner start-stamps, retention and lost bot identity. They do not cover recovery when the process stays alive and only the Discord transport reconnects. Implement this as an extension of their outbox owner, not a second recovery subsystem.

- Upstream `plugins/platforms/discord/adapter.py:217-249` distinguishes connection-shaped transport failures from timeout ambiguity and ordinary HTTP rejections. `:2809-2895` returns `send_path_degraded` plus retryable status for a dead connection instead of generic failure. `gateway/delivery_ledger.py:274-320` adds `sweep_failed_for_runtime`: exact process-start ownership, failed state, allowlisted error, exact adapter profile, attempt/staleness limits and atomic attempting-state claim. `:189-207` releases a claim without consuming an attempt when no send started. `:228-271` additionally scopes boot claims to connected `(platform, adapter_profile)` pairs. `gateway/platforms/base.py:3644-3665` records failure and calls runtime redelivery if a replacement adapter is already live, closing the reconnect-before-failure-record race. Installed `gateway/delivery_ledger.py:205-273` has only the dead-owner sweep.
- Rust `src/ledger/service.rs:291-314` skips every row with a live owner PID, including this process's failed rows. There is no runtime failed-row API or typed transport-retry classification. `src/main.rs:301-365,1297` invokes only boot recovery; `src/discord/adapter.rs:1040-1063` Ready handling starts inbound backfill only. Concrete transport methods in `src/discord/throttler.rs:41-80` propagate SDK errors without a delivery disposition. Source caller search confirmed `sweep_recoverable` is called only by boot recovery/tests and `record_obligation` has no live producer.
- Impact: fixing only baseline outbox creation and ACK propagation still leaves a response stranded after a transient, definitively rejected transport send until a process restart. Recovering bot A must not spend bot B's budget or send through A. A reconnect/ready event can precede recording the failure, so a Ready-only callback is insufficient. A timeout must not become a silent retry; earlier chunks may already be visible, so recovered output must retain explicit at-least-once ambiguity rather than promise exactly-once delivery.
- Minimal implementation boundary: extend existing delivery ledger with exact owner-instance/bot identity and compare-and-set failed->attempting runtime claims, narrow typed retryable error disposition at Discord egress, and one bot-scoped reconnect/replacement event consumer. Also retry scheduling after a late failure record when the transport is already restored. Share attempt/expiry policy and baseline outbox producer/receipt implementation. Release unsent claims without consuming budget; leave permissions, malformed requests and ambiguous timeout errors out of automatic unmarked retries. Never rerun the agent, create another daemon or reset a sticky conversation.
- Exact deterministic RED/GREEN regression (proposed): `cargo test --test test_discord_adapter reconnect_replays_only_owned_transport_failures -- --exact`. Seed real temporary SQLite with current-owner bot A failed/connection-refused row, A timeout row, A forbidden row, bot B connection-refused row, and another process-instance row (same PID/different start stamp). Use an explicit local transport recovery event, subscribed before trigger; join two simultaneous sweep requests at a barrier. RED: A failed row remains failed with no delivery (current live-owner skip). GREEN: exactly A's definitive transport row is claimed/sent once and then delivered; all other rows and budgets unchanged. The REST surrogate validates bot client, channel, content and returned ACK; a structured recovered/ambiguous disposition is asserted instead of pinning recovery notice prose. Separate table-driven cases cover stale/exhausted abandonment, claim release before send, and disconnected-bot boot eligibility.
- Local surface scenario: the same gateway runtime and SQLite pool survive a local connection-refused send. Restore the loopback transport, trigger the actual adapter lifecycle hook, and observe redelivery without another turn/start. A second case restores transport before the original failure is persisted and confirms the completion-side catch-up schedules exactly one claim. Use event gates, no fixed sleeps or polling; isolated bot IDs and no Discord network calls.
- Executed corroboration: an in-memory SQLite/OS-liveness evaluation of the Rust source predicate found one current-owner failed row and zero eligible dead-owner rows. This was a source-predicate model, NOT execution of DeliveryLedgerService. Pinned source supplies the positive runtime-claim contract; its imported production module was not executed against the real Hermes home.

## Exhaustive assigned-path coverage

Delta: C=changed, A=added, S=byte-identical. Dispositions are per-file roles; MIXED means all applicable changed behavior is either IO-01/02 or mapped below, with remaining paths explicitly excluded. Counts: **17 mapped/shared applicable files, 1 mixed Discord adapter, 5 intentional/optional presentation or voice files, 33 role exclusions = 56.** Optional voice status is retained, not mislabeled as a broken supported STT path.

| # | Assigned upstream path | Delta | Role / disposition |
|---|---|---|---|
| 1 | gateway/channel_directory.py | C | MAPPED: Discord text/forum/DM directory and target resolution; numeric/home/name target limitations remain config C10 / cron C10. Slack workspace/name enhancements excluded. No additional Discord delta. |
| 2 | gateway/dead_targets.py | C | MAPPED: classifier extraction, persistence simplification; Discord D06 / cron C12. |
| 3 | gateway/delivery.py | C | MAPPED: Discord send result/target handling cron C10-C11; Relay/native selection excluded. New exemption preserves terse cron artifacts rather than applying narration filtering; Rust does not have that narration filter, so no new defect. |
| 4 | gateway/delivery_ledger.py | C | MAPPED + IO-02: same-process failed-send claim is new; producer/boot/start-stamp/retention and identity defects remain cron C01/C14, sessions F01/F13. |
| 5 | gateway/mirror.py | C | MAPPED: explicit session_id now bypasses discovery; Rust mirror_to_session already takes exact key. Lookup ambiguity remains runtime R12 / cron C14. Registry acquire/release is Python resource management, not a new Rust gap. |
| 6 | gateway/platforms/ADDING_A_PLATFORM.md | C | NOT-APPLICABLE: plugin author documentation/extension contracts; no Rust plugin SDK requested. |
| 7 | gateway/platforms/__init__.py | C | NOT-APPLICABLE: Python package reexport/compat decomposition. |
| 8 | gateway/platforms/_http_client_limits.py | C | NOT-APPLICABLE: httpx client pool factory; Rust Discord uses Serenity, not httpx. No equivalent socket leak demonstrated. |
| 9 | gateway/platforms/_shared.py | A | MAPPED: per-profile secret read/YAML bridge extraction, config C03; Rust client selection already keys explicit bot identity. No Python global secret-scope port. |
| 10 | gateway/platforms/api_server.py | C | NOT-APPLICABLE: separate OpenAI-compatible Hermes platform on 8642, not this gateway's Discord transport/dashboard. Dashboard boundaries remain runtime R07-R09, not evidence for porting this server. |
| 11 | gateway/platforms/api_server_openai_routes.py | A | NOT-APPLICABLE: OpenAI completions/responses and SSE extraction; independent agent/frontend API. |
| 12 | gateway/platforms/api_server_room_dispatch.py | A | NOT-APPLICABLE: RoomLink hidden member-agent session ownership. |
| 13 | gateway/platforms/api_server_room_grants.py | A | NOT-APPLICABLE: RoomLink grants/capability endpoint. |
| 14 | gateway/platforms/api_server_run_idempotency.py | A | NOT-APPLICABLE: /v1/runs admission store; not Discord egress idempotency or OMO protocol. |
| 15 | gateway/platforms/api_server_runs.py | A | NOT-APPLICABLE: independent /v1/runs lifecycle and agent execution. |
| 16 | gateway/platforms/base.py | C | MAPPED + IO-02: final receipt/failure coordination; media/reasoning/silence/typing remain D01/D07/D09, approval AP02; Docker media remapping and independent runner session internals excluded. Async cache extraction does not establish new behavior gap. |
| 17 | gateway/platforms/bluebubbles.py | C | NOT-APPLICABLE: iMessage transport. |
| 18 | gateway/platforms/helpers.py | C | MAPPED: shared chunk/fence helpers extracted; Rust has Unicode chunk/fence handling. Thread participation remains D14, table fence behavior D17; table-to-bullets intentionally replaced by PNG. |
| 19 | gateway/platforms/media_cache.py | A | MAPPED: MIME/extension union preserves adapter historical outputs; Discord's existing downloader/text inlining D21 and optional STT D11. No new MIME behavior required merely by extraction. |
| 20 | gateway/platforms/msgraph_webhook.py | C | NOT-APPLICABLE: Microsoft Graph inbound transport. |
| 21 | gateway/platforms/qqbot/__init__.py | C | NOT-APPLICABLE: QQ package/registration. |
| 22 | gateway/platforms/qqbot/adapter.py | C | NOT-APPLICABLE: QQ transport. |
| 23 | gateway/platforms/qqbot/chunked_upload.py | C | NOT-APPLICABLE: QQ-specific upload protocol. |
| 24 | gateway/platforms/qqbot/constants.py | C | NOT-APPLICABLE: QQ protocol constants. |
| 25 | gateway/platforms/qqbot/crypto.py | C | NOT-APPLICABLE: QQ binding crypto. |
| 26 | gateway/platforms/qqbot/keyboards.py | C | NOT-APPLICABLE: QQ interactive keyboard wire schema. |
| 27 | gateway/platforms/qqbot/onboard.py | C | NOT-APPLICABLE: QQ account binding. |
| 28 | gateway/platforms/qqbot/utils.py | C | NOT-APPLICABLE: QQ HTTP headers/client helpers. |
| 29 | gateway/platforms/signal.py | C | NOT-APPLICABLE: Signal transport. |
| 30 | gateway/platforms/signal_format.py | C | NOT-APPLICABLE: Signal formatting/UTF-16 spans, not Discord wire limits. |
| 31 | gateway/platforms/signal_rate_limit.py | C | NOT-APPLICABLE: Signal attachment scheduler. |
| 32 | gateway/platforms/webhook.py | C | NOT-APPLICABLE: inbound HMAC/Svix webhook platform. |
| 33 | gateway/platforms/webhook_filters.py | C | NOT-APPLICABLE: webhook route filtering/scripts. |
| 34 | gateway/platforms/weixin.py | C | NOT-APPLICABLE: WeChat transport, AES/CDN/context tokens. |
| 35 | gateway/platforms/whatsapp_cloud.py | C | NOT-APPLICABLE: WhatsApp Graph transport. |
| 36 | gateway/platforms/whatsapp_common.py | C | NOT-APPLICABLE: WhatsApp rendering/bridge. |
| 37 | gateway/platforms/yuanbao.py | C | NOT-APPLICABLE: Yuanbao transport/middleware. |
| 38 | gateway/platforms/yuanbao_media.py | C | NOT-APPLICABLE: Yuanbao COS/media protocol. |
| 39 | gateway/platforms/yuanbao_proto.py | C | NOT-APPLICABLE: Yuanbao protobuf codec. |
| 40 | gateway/platforms/yuanbao_sticker.py | C | NOT-APPLICABLE: Yuanbao sticker catalog/lookup. |
| 41 | gateway/readiness.py | C | MAPPED: new active-session-store state probe maps runtime R09 (actual runtime state vs independent health proxy); sqlite closing fixes Python connection lifetime, Rust sqlx pool already owns connections. |
| 42 | gateway/response_filters.py | C | MAPPED: new autonomous matcher first/last token and [SILENT] prefix already present in Rust scheduler.rs:213-249; live integration remains cron C02/C05 and Discord D01. |
| 43 | gateway/stream_consumer.py | C | MAPPED: final-payload reconciliation and extracted state reset remain delivery ACK sessions F13 / cron C01/C11; no permission to emit tool/interim prose. |
| 44 | gateway/stream_consumer_fallback.py | A | MAPPED: extracted final continuation/receipt behavior; missing tail ACK maps cron C11 / sessions F13. Telegram empty-preview special retry excluded; final-only deliberately has no long-lived preview. |
| 45 | gateway/stream_consumer_fences.py | A | MAPPED: closing/escaping helpers extracted from installed consumer, not a new pin behavior. Rust chunk fences inspected; no new extraction-only finding. |
| 46 | gateway/stream_consumer_think.py | A | MAPPED: block-boundary case-insensitive scrubber extraction; Discord D01 already owns inline reasoning suppression, including preserving ordinary tag discussion. |
| 47 | gateway/stream_consumer_transport.py | A | MAPPED: explicit final-vs-interim delivery and fallback receipt tracking map F13/C11; draft/native relay transport, fresh-preview replacement excluded by final-only. |
| 48 | gateway/stream_dispatch.py | C | INTENTIONAL: adapter-driven tool chrome/notice dispatch; final-only output and separate tool badge already D20/D28. |
| 49 | gateway/stream_events.py | C | INTENTIONAL: dataclass contract simplification; remote OMO typed event correlation remains sessions F06, not Hermes agent event-engine port. |
| 50 | gateway/streaming_tts_consumer.py | A | OPTIONAL DISCORD PARITY, not enabled: incremental PCM synthesis. Retain explicit opt-in VC/TTS status from Discord D30; independent synthesis engine excluded. |
| 51 | plugins/platforms/discord/__init__.py | S | NOT-APPLICABLE: identical three-line Python package marker. |
| 52 | plugins/platforms/discord/adapter.py | C | MIXED: IO-01/02 new; channel/user profile gates config C03 and D02; response/media/mention/thread/backfill/approval deltas map existing D/AP IDs. Optional VC playback remains optional. Plugin platform edit/delete/thread event bus is an unsupported plugin hook, not a required new turn trigger under sticky conversations. |
| 53 | plugins/platforms/discord/ffmpeg_utils.py | A | OPTIONAL DISCORD PARITY: FFmpeg discovery for VC/TTS; no active Rust VC/TTS producer established. D08 voice uploader / D11 STT remain separately labeled optional/latent. |
| 54 | plugins/platforms/discord/plugin.yaml | S | NOT-APPLICABLE: identical plugin manifest; Rust has no Python registration layer. |
| 55 | plugins/platforms/discord/recovery.py | C | MAPPED: WAL fallback/schema compaction; actual missed-input recovery remains D15. Python database fallback alone does not prove a Rust sqlx/WAL failure. |
| 56 | plugins/platforms/discord/voice_mixer.py | C | OPTIONAL DISCORD PARITY: ambient PCM/voice mixer shortening; unsupported opt-in VC/TTS, not inbound note transcription. |

## Delta decisions that must not become duplicate findings

- Redirect-guard image URL fetch is newly extracted/hardened in Discord adapter `:273-297`, called by optional outbound URL-image handling. Rust's active inbound downloader consumes Discord attachment metadata, not agent-supplied outbound image URLs, and OMO currently has no MEDIA producer. No new live SSRF finding is proven by comparing these different surfaces. If optional URL-media parity is selected, include redirect validation in D07's existing safe media boundary.
- Empty-send rejection added in upstream send belongs D01/cron C02's explicit silence-vs-empty-result contract; do not count detached zero-width placeholders as an additional user-visible empty daemon failure without following the backend, which has an existing no-content error/Done decision.
- Profile-scoped authorization snapshots are important upstream changes but exactly config C03 / D02, not a new global-ACL finding. Bot-scoped delivery identity belongs cron C14 / sessions F01; IO-02 only adds live reconnect eligibility and atomic retry scheduling.
- Added streaming files are decompositions, not six missing Rust modules. The meaningful gateway guarantees are final-only text, reasoning/control filtering, actual receipt reconciliation and cleanup, already mapped above. No interim commentary, preview churn, streaming TTS or second daemon is required for parity.
- No new test-reliability row is added: existing adapter timing-luck tests are already D18 and existing backend/multiplexer ones sessions F16. Proposed new scenarios use causal event/barrier completion, never wall-clock luck.

## Verification boundary

This report proves source-level applicability and inventories every assigned path. It does not claim runtime certification, clean LSP, passing cargo tests, or an implemented fix. In-memory pinned cap evaluation and live-owner predicate evaluation completed; integration RED/GREEN and local-surface scenarios remain implementation acceptance contracts. User edits and all runtime invariants are preserved. The task's read-only stop condition is met: all 56 paths have dispositions, new confirmed work is isolated to IO-01/02, and the remaining applicable deltas map to existing findings or intentional behavior.
