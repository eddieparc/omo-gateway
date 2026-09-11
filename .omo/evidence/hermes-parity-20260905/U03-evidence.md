# U03 AP.AP08

## Registered before production edits
Actual test IDs resolved by cargo --list in U03-list.log. Commands in U03-commands.json are literal exact RED/GREEN commands.

- discord::approval::tests::approval_lifecycle_cleans_all_surfaces_and_isolates_bots: guild= guild, channel=42, thread=43, user=user, bot=A with two FIFO prompts then bot=B. Deny A. Expected remaining [false,true,true]; baseline expected [true,true,false]. Cleanup all UUIDs before assertion.
- discord::approval::tests::approval_lifecycle_drop_cleans_pending: real requester command `rm -rf fixture` (display only, never executed), reason=test, channel42/user7. Subscribe dispatcher before polling request, drop after ApprovalRequest observed. Expected pending_count=0 and terminal ExpireApproval; baseline expected count=1.

No production edits at registration. Initial list invocation hit shared build lock and tool timeout; no RED claimed. Subsequent list exit 0, two actual fully-qualified tests.

## Lead completion2026-09-06

Read current lease/terminal-owner/FIFO implementation, egress terminal path and local dashboard/Discord-REST fixture. Original lane/drop RED logs retained; U03-resume-initial.log distinguishes already-fixed inherited paths. U03-resume-registration.md and U03-resume-red.log register and capture remaining cached-dead-target and content-clobber behavioral RED before those production changes. The interrupted worker applied both fixes before provider failure: dead approval delivery returns Err, terminal edit only removes components without replacing content. Lead did not reimplement them.

Ran all4 literal U03-commands.json entries through monitor mon_7JTQCFS5RVRSY50D: each exact1 passed. FIFO remaining=[false,true,true]; dropped requester pending_count=0; dead target refused=true. Real local HTTP test exercises once/session/always/deny/drop/clear/cancel: one terminal event, guard/dashboard/egress0, duplicate HTTP404 and every real PATCH has components=[] without content. Listener tasks joined, DB closed, temp root removed before assertions. No real Discord transport. Full approval module12 passed, cargo build exit0 in29.55s. Captured U03-lead-green.log.

All3 changed-file LSP attempts before validation reported no diagnostics. Lead corrected only inherited U03 formatting in approval source/test, dead-target test and the newly added dashboard HTTP fixture. Scoped approval/adapter rustfmt and all3 diff-check pass; standalone rustfmt validates the dashboard fixture. Whole dashboard file has preexisting unrelated formatting and remains for final full-repository formatter, not falsely reported clean. Formatting changes are whitespace/import-order only and occurred after runtime proof. No source behavior changed by lead in this acceptance step, no global config/production/.env/commit/push edits.
