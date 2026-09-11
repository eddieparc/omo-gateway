# State and policy implementation phase draft

Superseded dispatch plan: see admission-storage-plan.md. Ten units whose foundation predecessors passed run independently of the U12 repair; the six remote-session units remain pending their actual prerequisites. Do not dispatch the obsolete full16-unit draft. This separation does not waive any unit verification or final goal gate.

Scope: 16 units read from current implementation-units.json and upstream-implementation-units.json. U03/U04 consent lifetime and display; U07/U08 exact staged replay and scoped review; U09/U10/U11 admission, pairing and yolo; U13-U18 remote cancellation, whole-turn deadline, submission ambiguity, durable binding, recovered identity and durable ingress; U51/U55 migration operations and dotenv; U75 drain age. The later installed policy/provider bridge is U58, consent U59, default model U60, footer U61.

Every semantic dependency and earlier physical file overlap has an ordering edge. U17 owns canonical identity in session.rs/db.rs/main.rs, and depends on U11 and U16; it is not the live-sequence unit. File lists below are generated from manifests, not recalled titles.

- U03 Cancellation-safe approval lifetime <- foundation only; src/discord/approval.rs, src/discord/adapter.rs, src/dashboard.rs
- U07 Atomic exact-destination replay <- foundation only; src/tools/skills.rs, src/storage/db.rs, src/memory/store.rs
- U13 Interrupt actual remote work <- foundation only; src/agent/backend.rs, src/agent/omo_backend.rs, src/multiplexer/actor.rs, tests/test_omo_backend.rs
- U51 Verified safe launchd retirement <- foundation only; src/migrate/gateway_down.rs, src/migrate/sys.rs
- U55 Dotenv value fidelity <- foundation only; src/migrate/config_import.rs
- U75 Expire stale drain requests by age <- foundation only; src/drain_control.rs
- U04 Redact approval display copies <- U03; src/discord/approval.rs, src/security/mod.rs, src/security/approval_display.rs
- U08 Session-scoped full pending review <- U07; src/discord/commands.rs, src/storage/db.rs
- U14 Absolute interactive deadline <- U13; src/agent/omo_backend.rs, tests/test_omo_backend.rs
- U09 Fail-closed shared admission <- U03, U08; src/discord/commands.rs, src/discord/adapter.rs, tests/test_discord_adapter.rs
- U15 No accepted-turn resubmission <- U14, U13; src/agent/omo_backend.rs, tests/test_omo_backend.rs
- U10 Pairing notification and lockout state <- U09, U03; src/discord/pairing.rs, src/discord/adapter.rs
- U11 Coherent yolo lifecycle <- U09, U08; src/discord/commands.rs, src/main.rs
- U16 Durable remote conversation binding <- U08, U13, U15, U07, U14; src/agent/omo_backend.rs, src/multiplexer/actor.rs, src/storage/db.rs, tests/test_omo_backend.rs
- U17 Canonical recovery bot identity <- U11, U16, U07, U08; src/models/session.rs, src/storage/db.rs, src/main.rs
- U18 Fail before undurable side effects <- U16, U17, U07, U13, U08; src/multiplexer/actor.rs, src/storage/db.rs, tests/test_multiplexer.rs

Route: preserve the existing logical category/reason from each manifest; the proven explicit hephaestus/ocx route is provider recovery from configured category 503s. Five resident slots naturally serialize ready nodes; one verification node depends on all 16 producers and independently runs their exact nonzero regression commands. No producer handles broad unrelated cleanup.

Shared-design.md governs actor ownership and installed peer APIs. U13 must interrupt actual remote work and never resume stale connections for cancel. U14 includes setup/retry in the same 300-second budget. U15 must not resubmit accepted or ambiguous work. U16 checkpoints remote binding before side effects and never persists a stale actor copy over it. U17 retains bot identity. U18 refuses work before persistence failures can cause backend effects. U07 must replay the exact staged destination once with durable receipt. U09 gates command/mention routes; U11 clears yolo on reset and completed stop.

Preserve all baseline user hunks, foundation changes, final-only replies, immediate continuous typing, PNG tables, isolated workspaces, one daemon, permanent sessions and interactive fail-fast. No .env, production/Discord writes, commits or push. Register RED before production; use real local surface and event-gated fixtures, then identical GREEN and adjacent checks, cleanup and diagnostics limitation evidence.
