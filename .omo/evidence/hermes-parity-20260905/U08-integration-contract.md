# U08 integration boundary

PendingWriteScope is Skills or Memory(exact current session key). A list returns only that scope. Detail/approve/reject use uniform refusal for missing, foreign-session or wrong-kind IDs; no existence oracle.

Approval must authorize the exact immutable PendingWrite then pass that same record into U07 conditional id/kind/payload transaction. A wrapper that authorizes once then calls an unscoped re-read can race payload changes and is rejected. Rejection likewise deletes conditionally using the scoped record identity/payload.

One pending_command helper is shared by actual skills/memory handlers and the regression. List/review send the complete scoped records as a JSON attachment; ordinary response text remains bounded. This preserves full UTF-8 payloads without adding fragile text pagination or a separate dashboard route. Skill policy remains shared-kind scope; memory policy remains exact existing session scope, not requester-only consent.

Current cfg(test) adapter models the existing unscoped APIs and truncated preview for behavioral RED. Replace it with the production command helper only after the monitored exact test produces a real nonzero behavioral failure. Original test payload and assertions remain unchanged. Storage/mod.rs export-only addition is authorized by the lead; no neighboring cleanup.
