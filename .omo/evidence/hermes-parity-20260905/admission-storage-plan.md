# Admission, storage and migration phase

Replan: the original foundation run settled with seven PASS and two qualified FAILs (U12 quiet-tool coverage, U62 exact-payload chronology). Current U12 follow-up owns only tests/test_omo_backend.rs. Its dependency is needed by remote-session units, not the ten units below. U62 is not a predecessor of any unit here. All foundation predecessors for this subset have independently passed: U02/U05 grant boundary, U06 path containment, U50 writes, U63 epoch. Therefore dispatch this fresh dependency-ordered phase while U12 repair continues; do not claim the full foundation or goal complete.

- U03: Cancellation-safe approval lifetime; after verified foundation predecessors; src/discord/approval.rs, src/discord/adapter.rs, src/dashboard.rs
- U07: Atomic exact-destination replay; after verified foundation predecessors; src/tools/skills.rs, src/storage/db.rs, src/memory/store.rs
- U51: Verified safe launchd retirement; after verified foundation predecessors; src/migrate/gateway_down.rs, src/migrate/sys.rs
- U55: Dotenv value fidelity; after verified foundation predecessors; src/migrate/config_import.rs
- U75: Expire stale drain requests by age; after verified foundation predecessors; src/drain_control.rs
- U04: Redact approval display copies; after u03; src/discord/approval.rs, src/security/mod.rs, src/security/approval_display.rs
- U08: Session-scoped full pending review; after u07; src/discord/commands.rs, src/storage/db.rs
- U09: Fail-closed shared admission; after u03, u08; src/discord/commands.rs, src/discord/adapter.rs, tests/test_discord_adapter.rs
- U10: Pairing notification and lockout state; after u09, u03; src/discord/pairing.rs, src/discord/adapter.rs
- U11: Coherent yolo lifecycle; after u09, u08; src/discord/commands.rs, src/main.rs

Ten producers plus one all-dependent verification node. Shared-file edges retained from current manifests. No overlap with live U12 writer. Natural slot cap5, independent readiness rolls in without config changes. Logical routing: U51/U55/U75 bounded value/lifecycle predicates; U03/U04 approval lifetime/display; U07/U08 exact transactional replay/review; U09-U11 authorization state integration. Proven hephaestus/ocx route retained due earlier configured-provider503. Each producer owns patch and preproduction RED/identical GREEN plus real-surface proof; verifier owns no code.

Deferred remote-session phase (not abandoned): U13/U14/U15/U16/U17/U18 will start after U12 and their storage/admission predecessors. Recompute edges from actual merged APIs; do not send old16-unit definition. Keep all explicit user constraints and shared-design.md. No global formatter with live writers.
