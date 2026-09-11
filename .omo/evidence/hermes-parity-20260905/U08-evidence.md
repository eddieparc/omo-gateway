# U08 scoped pending review

Lead resumes after U07 passed its exact replay/claim tests, all current boundaries, diagnostics and build. U08 scope is command pending list/detail/approve/reject and storage scoped claim APIs, with storage/mod.rs export-only addition if needed. No dashboard edit is necessary.

Contract: memory review/mutation is scoped to its existing full session key, not a new requester-only rule. Skill review/mutation is kind-scoped and retains current shared skill policy. Full pending data must be downloadable before approval, not hidden after an80-character preview. Existing slash authorization remains in command_check.

Before production changes, registered exact test: cargo test --lib discord::commands::tests::pending_review_preserves_session_kind_and_full_payload -- --exact --nocapture. Baseline adapter delegates the current command list/get/approve/reject APIs and current display preview so the regression measures existing behavior. GREEN will use the same tested command boundary in actual slash handlers. Expected RED: sessionB sees/applies sessionA memory, skills approval accepts memory kind, and rendered detail omits payload tail. No syntax/zero-test failures count. Real SQLite and actual reply builder, no Discord network.

## Completed lead proof2026-09-06

U08-lead-red.log captures exactly1 test failing on all5 observations: foreign list/detail, wrong-kind approval, foreign rejection and missing full skill download. Actual exit101, after correcting a test-only serde error conversion before compilation. No production scope change preceded that behavioral failure.

Production now exposes typed PendingWriteScope and scoped list/get/approve/reject. Checked immutable payload passes directly into U07 CAS transaction; no second unscoped re-read. Both actual skills and memory slash handlers call the same pending_command exercised by regression. List/detail returns complete JSON attachment including long Unicode content; approval/rejection response text is bounded. Missing/wrong-kind/foreign IDs share one refusal. Existing shared skill policy and exact memory session policy retained. Storage/mod.rs changes are exports only.

U08-lead-green.log: identical exact1 passes, commands module11 passes, storage module18 passes, cargo build exit0. Added postpatch positive owner download/apply/reject control passes1 and checked-payload-replacement CAS control passes1. These are supplemental controls, not retroactively claimed RED. Final scoped rustfmt/diff-check passed with no output after whitespace-only fixes to the two new tests. Full output of controls is U08-lead-controls.log.

Diagnostics were attempted on all3 changed Rust files before build and after final formatting: freshness timeout3000ms, not clean diagnostics. Rust compilation and tests pass. Real surface is the actual command reply builder and scoped storage transaction with real SQLite; no Discord messages or production state. Pools closed after tests, no listeners or temporary daemon fixtures used. U07 regressions retained in full storage target. No baseline backend/config/tests, unrelated source, commit or push edits.
