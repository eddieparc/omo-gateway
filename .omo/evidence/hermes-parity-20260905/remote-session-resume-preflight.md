# Remote-session phase preflight

Not dispatched; consume settled U12 and admission/storage results before finalizing prompts. All six original unit scenarios remain binding. Read shared-design.md and exact manifest scenarios at dispatch; no speculative API implementation is claimed.

Current interfaces inspected via LSP and read: AgentBackend::run_cancelable selects run versus cancellation and drops run; default cancel returns Ok. SessionActor::run separately drops run before interrupt_turn invokes runner.cancel with self.context, not turn_context. Stop currently logs cancel errors and sends Ok(true) regardless. U13 must retain exact remote identity independent of dropped future, await actual interrupt acknowledgement and propagate failure before reporting stopped, while releasing typing at every terminal.

Current actor persists inbound with log-and-continue, and flush failures are logged before delivery completion. This confirms U18 belongs at the durable acceptance/completion boundary; test with real SQLite triggers, not mocked storage. U16 must persist thread binding without allowing stale actor flush to overwrite it. U17 must retain full bot identity on recovered keys.

Sequence: U13 -> U14 -> U15 -> U16 -> U17 -> U18, with U16 gated on U07/U08 and U17 on U11. Serialize backend and test_omo_backend owners. U18 owns test_multiplexer. A final read-only verifier depends on all six. U14 must retain user 300-second cap and 30-second ACK fail-fast while covering setup/retries; U15 must preserve pre-handshake retry but prohibit accepted or ambiguous resubmission. Exact current predecessor APIs must be read after their producer settles.

Remaining original U19-U85 units stay individually tracked; this document does not remove or collapse them. Final real installed OMO surface, full gates, Discord boundary and cleanup remain unchanged.
