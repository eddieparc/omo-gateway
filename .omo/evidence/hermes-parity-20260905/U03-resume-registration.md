# Resumption registration, before remaining production edits

The inherited lease, FIFO matching, clear_session and delivery-owner task were partially applied before resumption. Their original two RED logs are retained. Resumption's initial exact runs already pass those two scenarios; not a fresh RED implementation.

Listed by cargo, each with 1 test, 0 benchmarks:
- `discord::adapter::tests::approval_dead_target_refuses_delivery`
- `legacy::dashboard::tests::approval_lifecycle_local_http_surface` (binary target, dashboard is not in the library)

Literal invocations are now in U03-commands.json before their registered RED runs.

Dead payload: cached channel 42, ApprovalRequest command=display-only, user=7. Expect Err immediately; baseline Ok. No HTTP allowed/needed.

Local surface: real dashboard router on ephemeral loopback, in-memory migrated SQLite, temporary workspace/web roots; real CompositeDispatcher -> real DiscordEgress using Serenity's explicit loopback HTTP endpoint, no forwarding. Real POST /api/approvals/{UUID}/resolve payloads {decision:once|session|always|deny}, requester abort, clear_session and cancel. Display-only rm -rf fixture is never executed. Expect exactly one ExpireApproval per request, dashboard items=[], guard=0, egress=0, serialized PATCH components=[] with no content field (must not overwrite a button resolver's decision), duplicate resolution HTTP404. Baseline inherited task already emits terminal events but adapter overwrites content with expired text. Cleanup precedes that binary assertion.

No new external API or file ownership is needed. No eval/tool_schema/monitor tool is exposed in this session; bounded synchronous bash commands are the available execution path.
