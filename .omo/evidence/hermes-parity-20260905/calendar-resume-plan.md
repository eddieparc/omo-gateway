# Calendar phase resumed ahead of unrelated consumers

Lead read current src/cron/store.rs import/sync and src/cron/scheduler.rs claim/next_run paths. U46 original U39/U43/U45 edges serialize shared files, but timezone computation does not consume future output provenance, target resolution or reminder identity. U47 can use current run state and current source sync; U76 consumes repaired U47 rearm semantics. No active worker owns calendar source, Cargo manifests or these targets.

Graph U46 -> U47 -> U76 -> verify-calendar. One producer at a time; three original units remain independent tracked tasks, each change paired with its proof. Deep routing is required for backend/logic work. Runs concurrently with the two current recovery DAGs and daemon-only repair, without raising capacity. U46 owns Cargo dependency only if IANA timezone support requires it; no other producer edits Cargo.

Later U38-U49/U52/U78+ writers must consume and preserve this verified calendar work. Original semantic contracts and shared-design.md remain binding. Completion means all timezone/DST, malformed store isolation/rearm and stale one-shot boundary scenarios pass through real SQLite/scheduler/import seams. Final global tests/build/real OMO QA remain deferred until all work completes.

