# Pinned upstream configuration/security delta audit

Status: COMPLETE (read-only source comparison; not runtime certification).

Pinned source: `https://raw.githubusercontent.com/NousResearch/hermes-agent/f58fcc8118d9db092ad60d363d4a28520e08ac5a/<path>`. `U:` below denotes that exact revision; `H:` denotes `/Users/indo/.hermes/hermes-agent`; Rust paths are repository-relative. Existing working-tree edits were preserved.

## Result and method

All **73 assigned paths** were fetched successfully and compared against installed bytes: **44 added, 26 changed, 3 identical; 0 fetch failures**. All **70 Python files parsed successfully with Python AST**. Function inventories and module roles were inspected across the entire assignment; applicable configuration, authorization, approval, lifecycle and Discord branches received focused body/caller comparisons. This is exhaustive path/role coverage, not a claim that every statement in independent products was behaviorally tested.

**Three additional confirmed findings**, absent from the six installed-source audit finding lists: DC01 recovery authorization (existing reference behavior missed previously), DC02 low-disk classification (new upstream helper), and DC03 destructive-command confirmation (existing optional reference behavior missed previously). DC01 and DC03 are explicitly **not newly introduced upstream functionality**. Splitting old `run.py` and `slash_commands.py` into mixins does not make their existing behavior new.

Read config/approvals/discord/runtime/cron audits and the sessions inventory/contracts, and consulted lead-audit.md for active daemon boundaries. Deduplication uses lane-qualified IDs. Programming skill and Rust reference were read. Rust AST search located actual recovery functions; textual caller search plus direct reads established paths. LSP infrastructure is down as specified by the task; no LSP-clean claim. No cargo tests/build, production subprocess, migration, .env read/write, daemon operation or Discord action was performed. Only this report was written.

The executable comparison used in-memory HTTP downloads, byte equality/SHA-256, Python AST inventories and moved-symbol lookup against installed monoliths. A bounded pure evaluation of the pinned disk classifier returned `(free_mb=200,total_mb=1000) -> critical`, `(50000,1000000) -> ok`, and `(None,1000) -> unknown`. Rust execution was not performed; Rust results below are source-derived. Every proposed regression is NEW and was NOT executed; a zero-test filtered command must not be accepted as verification.

## Newly confirmed findings

### DC01 - Startup replay bypasses current authorization

**Status: FIX, security boundary; existing reference behavior, newly identified audit omission.**

- Upstream: `U:gateway/run_startup.py:423-436` fails closed on current owner authorization; `:465-466` checks it before scheduling. Installed counterpart `H:gateway/run.py:7246-7265` already performs the same check. Thus this is not a feature invented by the file split.
- Rust: `src/main.rs:370-422` receives only pool/multiplexer, clears a pending marker and routes its reconstructed event at `:409`; no current ACL is available. `src/multiplexer/router.rs:234-249` queues without ingress authorization. Normal command admission is separate (`src/discord/commands.rs:179-205`). Live startup calls recovery at `src/main.rs:1314`.
- Impact: a previously admitted user removed from current policy can have unfinished work replayed on restart without renewed admission. Important qualification: current bot-scoped rows are usually masked by the already-known reconstruction bug (`src/storage/db.rs:135-166`, sessions F01/runtime R03). Legacy bot-less rows, also used by the existing real recovery test at `src/main.rs:1763-1836`, reach the bypass now. Fixing canonical identity alone exposes the same missing gate to normal bot rows. No claim of a currently demonstrated remote exploit or live Discord send.
- Not duplicate: sessions F01-F03/runtime R03-R04 cover key loss, premature marker clear and duplicate history, not revocation/current-policy replay admission. Config C03 covers profile-policy association, not recovery checking it.
- Minimal boundary: inject an effective bot/profile recovery admission service into this startup function and check before consuming eligibility or dispatching. Use stored real initiator/provenance, not a synthetic shared-session user as authority. Unknown identity or failed role/policy lookup must defer/deny rather than borrow default-bot authority. Reuse ingress policy; do not put a Discord-specific ACL into all multiplexer traffic or implement a second daemon.
- Exact proposed test: `cargo test --bin omo-gateway recovery_rechecks_current_owner_authorization`. Seed temporary SQLite with an unfinished legacy Discord session for user 42, a pending marker, and current allowed-users `[7]`; construct the real multiplexer with a recording runner. Await recovery's returned admission count, then an explicit actor/barrier completion if a route was admitted. **RED:** recovered count 1 and runner observes user42. **GREEN:** recovered count 0, no actor admitted, no user42 runner invocation, marker retained/deferred with a machine-readable denial reason. Separate allowed-user7 control must admit exactly once. After identity repair, repeat with bot84 whose ACL differs from bot42. No sleep/negative timing window.
- Local surface scenario: call the production startup recovery entry against file-backed temp SQLite and a loopback recording daemon; restart construction with a narrower ACL, then inspect actual `turn/start` requests after the recovery completion barrier. No request for the revoked owner; authorized control still reaches its exact bot/agent workspace. This test does not need real Discord credentials.

### DC02 - Disk safety reports healthy with critically low absolute headroom

**Status: FIX, operational classification; added upstream helper.**

- Upstream: `U:gateway/disk_status.py:19-39` combines absolute free-space floors and percentage/headroom, and returns unknown on invalid samples. `:43-60` exposes the resulting pressure. The assigned file is absent from the install.
- Rust: `src/readiness.rs:66-83` uses percentage alone and calls zero total healthy; `:97-123` feeds that result into real disk probes. `src/readiness.rs:190` includes the probe in runtime readiness. `src/dashboard.rs:697-720` exposes raw capacity, but no pressure classification. HTTP `/readyz` independently tests only path metadata (`:661-685`), already covered generally by runtime R09.
- Impact: a 1000 MiB data filesystem with 200 MiB free is only 80% used, so the 90%-threshold Rust disk probe says ok although pinned upstream says critical. Conversely, 95% used with 50000 MiB free is degraded in Rust but not pressure-critical/elevated upstream. Zero-size sample is incorrectly positive. This is a disk-classifier defect, not absence of all disk monitoring or a claim that SQLite will necessarily fail at 200 MiB.
- Not duplicate: runtime R09's concrete cases cover daemon/credentials/connection truthfulness and metadata-only HTTP readiness; none of the six reports identify absolute headroom or zero-capacity classification. Coordinate with R09's owner for the shared readiness surface rather than open a second HTTP rewrite.
- Minimal boundary: update the existing disk classifier to incorporate the upstream absolute floors and represent unknown/degraded for unusable capacity, retaining byte metrics. Feed that one classification to exposed status/readiness where disk health is represented. No Linux cgroup, heartbeat file, full OOM-forensics stack or dashboard redesign is required.
- Exact proposed test: `cargo test --lib readiness::tests::disk_pressure_uses_absolute_headroom`. Inputs in bytes: total `1000*1048576`, free `200*1048576`; total `1000000*1048576`, free `50000*1048576`; total/free 0. **RED:** first is ok, second degraded, third ok. **GREEN:** first degraded with critical pressure, second ok, third nonhealthy/unknown. Pure injected values; no disk filling or wall clock. Extend machine-valued existing tests, not assertions on human-readable detail text.
- Local surface scenario: construct actual readiness/status HTTP service with a narrow disk-sampling seam returning the above samples and real temp SQLite/workspace. Issue loopback GET to the exposed readiness/status routes; assert serialized pressure/capacity and the selected nonhealthy policy, while `/health` stays live. The sampler may be fake; classifier and HTTP serialization must be production code.

### DC03 - Destructive Discord commands have no confirmation gate

**Status: OPTIONAL-DISCORD parity, confirmed missing feature; existing reference behavior, not new upstream.**

- Upstream: `U:gateway/run_busy.py:1037-1076` defaults `approvals.destructive_slash_confirm` to true, runs only after consent, and supports operator opt-out; `:1122-1155` registers the request before sending buttons/fallback. Installed `H:gateway/run.py:15617-15677` already has this gate.
- Rust: `src/discord/commands.rs:572-589` immediately deletes messages AND memories on `/reset`; `:719-744` immediately calls undo. General `command_check` (`:179-205`) establishes admission only. `src/migrate/config_import.rs:42-45` parses approval mode only, with no destructive-confirm preference.
- Impact: a valid but accidental reset/undo irreversibly changes local data immediately. This is not an authorization bypass, and does not imply the remote conversation reset is correct (already D12/sessions F09). Sticky conversations rule out automatic resets, not confirmation of an explicitly requested destructive command. Optional consent UI is an exception for a user command, not restoration of intermediate agent chatter.
- Not duplicate: AP02/AP08/AP22 concern agent-tool approvals and their lifecycle/yolo; D12/F09 concern authoritative session control. None specifies confirmation before destructive slash mutation. Config C02 owns generic unsupported-config reporting; coordinate rather than duplicate its implementation package.
- Minimal boundary if selected: one gateway command-confirmation lease for reset/undo, exact bot/channel/session and requesting interaction binding, no mutation before authenticated decision. Cancel/expiry does nothing. Apply the confirmed command via the serialized authoritative-control boundary from F09; do not build a parallel reset implementation. Preserve explicit operator opt-out. Persistence failure of an Always preference must not claim it was saved.
- Exact proposed test: `cargo test --test test_discord_adapter destructive_slash_waits_for_confirmation`. Seed real SQLite with one exchange and memory. Drive a real local interaction handler through a REST capture transport. Subscribe to confirmation dispatch before invoking reset; inspect DB while dispatch is held: **RED:** rows already deleted/no confirmation. **GREEN:** rows unchanged until matching Once, then one serialized reset; Cancel/expiry retains bytes. A component from another bot/session cannot resolve it. Use explicit dispatch/resolution barriers and paused time for expiry; no sleeps.
- Local surface scenario: loopback Discord interaction/REST fixture plus real command service/multiplexer and stateful fake daemon. Observe button payload, cancel first request, accept second, verify local and remote control once only. This remains optional work requiring feature selection, not a prerequisite to claim an exploit fixed.

## Exhaustive assigned-path coverage

Each path appears once below. A/C/S = added/changed/same bytes against the install. MAPPED means applicable behavior is already captured in named existing findings, not certified correct. NOT-APPLICABLE is role-based, not a blanket claim that all relay or voice code is non-Discord. OPTIONAL-DISCORD retains actual optional Discord surfaces explicitly. Mixed-role files list the exclusions beside their applicable mapping.

### Configuration, permissions and process safety (22 paths)

- `gateway/AGENTS.md` A - Documentation/inventory, no runtime module. Profile fail-closed reads and credential locks map config C03/runtime R22; no new product requirement from prose.
- `gateway/__init__.py` C - Package exports/documentation, no independent behavior gap.
- `gateway/agent_cache_pressure.py` A - NOT-APPLICABLE: resident Hermes AIAgent transcript eviction; Rust delegates that cache to OMO, not gateway SQLite actor GC.
- `gateway/authz_mixin.py` C - MAPPED config C03, Discord D02, approvals AP18-19: transport-owner/profile-aware ACL and pairing. Nostr normalization/chat grants are NOT-APPLICABLE to Discord sender IDs.
- `gateway/cgroup_cleanup.py` C - NOT-APPLICABLE: Linux systemd ExecStopPost cgroup reaper; macOS launchd migration is config C06, not this implementation.
- `gateway/code_skew.py` C - NOT-APPLICABLE: Python lazy-import hot-checkout skew; compiled Rust binary has no matching module-load surface.
- `gateway/config.py` C - MAPPED config C02/C03/C10-C13 and intentional display/session exceptions; hosted RoomLink/systemd/non-Discord entries excluded by role.
- `gateway/config_env.py` A - MAPPED config C02/C04: environment precedence, explicit disable, Discord token/home/reply bridges; mostly extraction of old config.py. Relay-exclusive enablement is optional transport selection, not required architecture.
- `gateway/config_loader.py` A - MAPPED config C02: raw YAML/legacy merge, key-presence precedence and shared policy bridging; no formatting-only finding.
- `gateway/control_socket.py` A - MAPPED config C06/runtime R22/R09: positive process identity/liveness and bounded local update coordination. A Unix socket is one upstream mechanism, not a required new Rust control architecture. Windows pipe branch NOT-APPLICABLE.
- `gateway/cwd_placeholder.py` C - NOT-APPLICABLE: Hermes terminal container placeholder mapping. Applicable actual OMO cwd/config wiring already config C11/cron C13; per-agent workspace invariant retained.
- `gateway/disk_status.py` A - NEW DC02; existing Rust disk metrics are present, low-headroom classification differs.
- `gateway/display_config.py` C - MAPPED config C13/Discord D10 for final footer; preview/tool/status churn INTENTIONAL exclusion under final-only and continuous typing.
- `gateway/media_policy.py` A - OPTIONAL-DISCORD, MAPPED Discord D07/config C02: shared strict/allow-dir bridge belongs with already-known missing MEDIA delivery and path policy, not a new literal-file exfiltration claim.
- `gateway/media_repair.py` A - NOT-APPLICABLE agent-protocol repair: uses Hermes computer_use tool transcript/call IDs to fix model-mangled Windows paths. Do not infer equivalent provenance from arbitrary OMO text. Optional MEDIA remains D07.
- `gateway/memory_monitor.py` C - NOT-APPLICABLE Python daemon-thread RSS logging; no required logging-format parity. Rust status already has memory metric.
- `gateway/memory_status.py` A - NOT-APPLICABLE hosted lifecycle-sentinel/heartbeat OOM rollup; existing Rust status has process memory, not these Hermes files. No demand to reconstruct independent sentinel infrastructure.
- `gateway/pairing.py` C - MAPPED AP18/AP19/config C03: separate rate/lockout, scoped stores, revocation/list/hash feature inventory already reported. Live allowlist purge belongs existing revocation capability, not duplicate new finding.
- `gateway/platform_registry.py` C - NOT-APPLICABLE Python plugin discovery/cancellation and profile-scoped registry; native Discord Rust construction does not use it.
- `gateway/systemd_notify.py` C - NOT-APPLICABLE Linux service watchdog; shared local teardown/readiness remain runtime R01/R02/R09.
- `tools/approval.py` C - MAPPED AP02/AP04/AP06/AP08/AP10/AP12/AP22; now delegates detection/context/floors/wait logic to leaves. Kernel/computer-use permission-mode resource release is independent agent internals, not a demand for gateway kernels.
- `tools/write_approval.py` C - MAPPED AP13-15; exact replay, write gate and review remain covered. Inline interactive CLI memory prompt is NOT-APPLICABLE to Discord staging.

### Independent hosted-room/browser/kanban products (15 paths)

- `gateway/browser_control_artifacts.py` A - NOT-APPLICABLE browser controller upload/artifact capability protocol, not Discord attachment hydration.
- `gateway/browser_control_broker.py` A - NOT-APPLICABLE browser controller ticket/ownership broker; daemon-owned browser intelligence is not a gateway parity commitment.
- `gateway/hosted_room_discussion.py` A - NOT-APPLICABLE autonomous hosted group-chat discussion planning.
- `gateway/hosted_room_driver.py` A - NOT-APPLICABLE hosted-room lease/task execution state machine.
- `gateway/hosted_room_execution_policy.py` A - NOT-APPLICABLE RoomLink target-issued scoped execution authority.
- `gateway/hosted_room_links.py` A - NOT-APPLICABLE private negotiated cross-gateway link storage.
- `gateway/hosted_room_peer.py` A - NOT-APPLICABLE cross-gateway room grant/endpoint negotiation.
- `gateway/hosted_room_policy_checkpoint.py` A - NOT-APPLICABLE hosted discussion policy projection.
- `gateway/hosted_room_replicas.py` A - NOT-APPLICABLE cross-gateway replica takeover/fencing, not one local OMO daemon.
- `gateway/hosted_rooms.py` A - NOT-APPLICABLE hosted Bot Mode append-only room service, not ordinary Discord channels.
- `gateway/hosted_rooms_common.py` A - NOT-APPLICABLE leaf validators/storage for that product.
- `gateway/kanban_watchers.py` C - NOT-APPLICABLE independent board dispatcher/notifier; not gateway cron schedules.
- `gateway/kanban_watchers_common.py` A - NOT-APPLICABLE board enumeration/settings/singleton lock extraction.
- `gateway/kanban_watchers_dispatcher.py` A - NOT-APPLICABLE autonomous board task decomposition/dispatch.
- `gateway/kanban_watchers_notifier.py` A - NOT-APPLICABLE board subscription notification product. Its possible Discord destination does not make an unconfigured kanban engine required.

### Optional relay and other-platform assets (14 paths)

- `gateway/assets/status_phrases.yaml` S - INTENTIONAL no status chatter; identical bytes.
- `gateway/assets/telegram-botfather-threads-settings.jpg` S - NOT-APPLICABLE Telegram setup illustration; byte-equal binary, no image-content parity claim needed.
- `gateway/builtin_hooks/__init__.py` S - Empty built-in hook package/docstring, no new runtime behavior.
- `gateway/relay/__init__.py` C - OPTIONAL-DISCORD transport replacement, not current native deployment: connector provisioning/policy sync. Other fronted platforms NOT-APPLICABLE.
- `gateway/relay/adapter.py` C - OPTIONAL-DISCORD relay media, approval/prompt passthrough, typing/reactions, threads. Equivalent native gaps already D05-D09/D14 and AP02/AP08. Do not install another connector for native parity. Slack cumulative streams excluded.
- `gateway/relay/auth.py` C - OPTIONAL-DISCORD relay bearer protocol; no connector credentials or upgrade surface in native Poise runtime. No inferred native auth defect.
- `gateway/relay/command_manifest.py` A - OPTIONAL-DISCORD registration manifest mirroring native command tree; missing command inventory already Discord audit feature section. Not an independent command implementation requirement.
- `gateway/relay/descriptor.py` C - OPTIONAL-DISCORD relay capability negotiation; native transport uses Serenity types rather than connector descriptor.
- `gateway/relay/media.py` A - OPTIONAL-DISCORD connector-hosted reference uploads/downloads; native media gaps D07/D11, no requirement to add remote media plane.
- `gateway/relay/transport.py` C - OPTIONAL-DISCORD connector transport contract; interrupt/confirmed delivery invariants map sessions F04/F13 when comparing native runtime.
- `gateway/relay/ws_transport.py` C - OPTIONAL-DISCORD relay reconnect/ACK/deadline machinery. Actual OMO WS is a different protocol; native acceptance/correlation/deadline gaps already sessions F05-F07, not proof to copy relay frames.
- `gateway/rich_sent_store.py` C - NOT-APPLICABLE Telegram Bot API rich-message reply-content cache.
- `gateway/sticker_cache.py` C - NOT-APPLICABLE Telegram vision-description cache.
- `gateway/whatsapp_identity.py` C - NOT-APPLICABLE WhatsApp LID/JID normalization; Discord snowflakes are not aliases of these IDs.

### Gateway runner and command decomposition (22 paths)

Added files here largely move old monolith code. Symbol-name absence was not treated as semantic novelty; recovery authorization, destructive confirmation and semantic thread rename were checked against installed run.py.

- `gateway/run_adapters.py` A - MAPPED config C03/runtime R01/R06/R22 and AP18: profile ownership, bounded connection/teardown, restart and credential claims. Other adapters excluded; independent relay/kanban services excluded.
- `gateway/run_agent_cache.py` A - MAPPED sessions F04/F06/F09/AP22 for shared cancellation/generation/reset contracts. Resident Hermes AIAgent cache-pressure eviction/finalization NOT-APPLICABLE.
- `gateway/run_busy.py` A - NEW optional DC03; existing queue/steer/control differences MAPPED D12/F09 and runtime R02. Global emergency pause is covered at reversible-admission boundary R02, not a second daemon-wide control service.
- `gateway/run_common.py` A - Leaf sentinel constant extraction, no independent feature.
- `gateway/run_config_loaders.py` A - MAPPED config C02/C11-C13, sessions F05/F09, runtime R02: timeout, profile busy-mode and model/settings wiring. Hermes provider fallback/reasoning intelligence excluded.
- `gateway/run_goals.py` A - NOT-APPLICABLE autonomous goal/heartbeat/loop continuation product; do not turn sticky conversations into new autonomous sessions.
- `gateway/run_inbound.py` A - MAPPED config C03, D02/D04/D11/D15, sessions F02/F03/F17: fail-closed routes/admission, durable markers, media and timestamps. Python contextvars and agent image-mode inference excluded; pause admission maps R02.
- `gateway/run_notifications.py` A - MAPPED cron C01/C02/C11 and sessions F13 for shared completion receipts; optional MEDIA D07. Independent Hermes background-process/delegation/update watchers NOT-APPLICABLE to OMO-owned work. No raw-progress restoration.
- `gateway/run_shutdown.py` A - MAPPED runtime R01/R02/R10/R11, sessions F02/F04/F14 for bounded drain, interrupt and cleanup. Fly dormancy/Linux watcher/independent worker cleanup excluded.
- `gateway/run_startup.py` A - NEW DC01; other startup recovery/claim/readiness/profile differences MAPPED sessions F01-F03/F13, runtime R01/R03/R09/R22, config C03. Hosted-room warmup excluded.
- `gateway/run_topics.py` A - OPTIONAL-DISCORD semantic rename callback, already present in installed run.py; manual Rust /title and auto-thread gaps already Discord feature inventory/D14. Agent-generated title producer is independent engine behavior, not a new mandatory gateway gap. Telegram topics NOT-APPLICABLE.
- `gateway/run_turn.py` A - MAPPED sessions F04-F09/F11/F13-F15 and D01/D07/D10 for applicable turn controls/output/config. Hermes context hygiene/compaction/provider routing is independent agent internals; preview/reasoning intentionally excluded.
- `gateway/run_turn_runner.py` A - MAPPED AP02/AP08, sessions F04/F06/F14 for callback consent/lifetime. Hermes resident-agent build/history/vision internals excluded; task cards/intermediate tool prose intentionally excluded.
- `gateway/run_voice.py` A - OPTIONAL-DISCORD VC join/TTS mode product, already explicitly inventoried D30/feature section; not classified as another-platform-only. Gateway voice-note STT remains D11. No new confirmed voice defect.
- `gateway/run_watchers.py` A - MAPPED sessions F05/runtime R06 for bounded live work; agent catalog refresh/internal stall reporting excluded, automatic session expiry INTENTIONAL exclusion under stickiness.
- `gateway/scale_to_zero.py` C - NOT-APPLICABLE Fly/NAS machine suspend and relay wake routing; perpetual local macOS gateway is not hosted autoscaling.
- `gateway/slash_access.py` C - MAPPED config C02 additional admin/group command ACL inventory; installed policy and pinned policy retain same admin/command floor. No duplicate finding for whitespace/helper extraction.
- `gateway/slash_commands.py` C - MAPPED D12/D13, AP08/AP15/AP22, config C13; broader command omissions already Discord feature inventory. Administrative updater/background/kanban commands are not automatically authorized parity additions.
- `gateway/slash_commands_goals.py` A - NOT-APPLICABLE autonomous goal/subgoal/heartbeat/refine/review/loop intelligence.
- `gateway/slash_commands_model.py` A - MAPPED D12/sessions F09/config C11/C12 for real model routing; provider picker/cost/reasoning/personality internals excluded rather than reproduced in gateway.
- `gateway/slash_commands_session.py` A - MAPPED sessions F09/Discord D12 for authoritative history; optional confirmation DC03. Named resume/branch/new routing intentionally not restored against sticky permanent lanes. Scope checks of a nonexistent resume surface are not a current exploit.
- `gateway/slash_commands_status.py` A - MAPPED runtime R09 and existing Discord help/usage/model-inspection feature inventory. Hermes compressor/billing/autonomous-agent introspection is independent intelligence; no fake gateway metrics proposed.

## Closure

Coverage groups total **22 + 15 + 14 + 22 = 73**. Three findings have upstream and Rust coordinates, explicit applicability/provenance, minimal implementation boundary and deterministic proposed RED/GREEN plus local-surface scenarios. Everything else is mapped to an existing finding, intentional invariant, optional transport/product, or role-based exclusion. This report does not certify existing findings fixed and does not authorize optional feature implementation. No new architecture, subagents, production changes or commits were created.
