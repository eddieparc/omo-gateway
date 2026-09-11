# Next phase readiness draft

Do not dispatch until foundation-verification.md is read and failed/missing items corrected. Shared design is read and current installed peer contracts independently checked.

Candidate immediately-ready units after current foundation: U03 Cancellation-safe approval lifetime; U07 Atomic exact-destination replay; U13 Interrupt actual remote work; U51 Verified safe launchd retirement; U55 Dotenv value fidelity; U75 Expire stale drain requests by age. This is dependency readiness only, not a permission to run shared files concurrently.

Use shared-design.md contracts: cancellation-safe approval lifetime; backend result ownership via outbox rather than direct channel dispatch; whole setup+turn+retry deadline; stable bot-scoped identities; durable accepted inbox without automatic replay of ambiguous turns; actual extension tool_call policy; safe migration receipts.

Explicit installed peer limitation: no thread/rollback; keep unimplementable remote history mutation as honest explicit error until supported equivalent exists. Never fake semantic completion by local SQLite mutation. Every behavior change still requires registered exact nonzero RED before patch, same GREEN and faithful wire/SQLite/CLI.

All current completed candidates require lead result checks. Next graph must be a NEW phase run, include verification node, serialize every overlapping source/test file.
