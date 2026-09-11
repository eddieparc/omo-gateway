// U16-U18 remote durable phase definition. Lead-authored dispatch data; Flash workers edit source.
const EV = '/Users/indo/code/project/omon-gateway/.omo/evidence/hermes-parity-20260905';
const ROOT = '/Users/indo/code/project/omon-gateway';

const units = {
  U16: {
    title: 'Durable remote conversation binding',
    files: ['src/agent/omo_backend.rs', 'src/multiplexer/actor.rs', 'src/storage/db.rs'],
    tests: ['tests/test_omo_backend.rs'],
    redCommand: 'cargo test --test test_omo_backend failed_first_turn_keeps_durable_binding -- --exact --nocapture',
    scenario: 'thread/start r1; turn fails; reconstruct backend/mux from file DB. RED second thread/start; GREEN resume r1. Missing-rollout subcase: RED silent fresh history, GREEN explicit continuity error or restored exchange. QA wire transcript verifies binding across process recreation.',
    note: 'Binding must persist before turn/start and survive backend/mux recreation from the same SQLite file; missing rollout is continuity_error, never silent fresh history. Stale actor copy must not overwrite durable binding. Preserve U14/U15 deadline and no-resubmit semantics.'
  },
  U17: {
    title: 'Canonical recovery bot identity',
    files: ['src/models/session.rs', 'src/storage/db.rs', 'src/main.rs'],
    tests: ['src/models/session.rs'],
    redCommand: 'cargo test --lib resume_pending_preserves_bot_identity -- --exact --nocapture',
    scenario: 'otherwise identical bot-A and bot-B rows, only B pending. RED: returned key lacks B; GREEN: exact B key, only B recovery eligibility changed. Real SQLite recovery sweep and recording dispatch identity; include outbound-ledger identity case with its owner.',
    note: 'Full SessionKey incl bot_id must survive recovery; use the existing length-prefixed storage_key format, share parsing rather than inventing a parallel scheme. Twin-bot same-DM rows must recover exact keys with unchanged bot identity; outbound-ledger owner identity included. No blanket reset of other rows. If persistence is needed, reserve the next free migration number after checking the actual migrations directory; never modify applied migrations.',
    extra: ['migrations/0025_session_identity.sql']
  },
  U18: {
    title: 'Fail before undurable side effects',
    files: ['src/multiplexer/actor.rs', 'src/storage/db.rs'],
    tests: ['tests/test_multiplexer.rs'],
    redCommand: 'cargo test --test test_multiplexer persistence_failure_never_acknowledges_turn -- --exact --nocapture',
    scenario: "SQLite trigger RAISE(ABORT,'write blocked') on user insert; second variant on session update. RED runner called/delivered; GREEN no run for failed insert, no delivered for failed flush. QA real SQLite triggers and mux, no mocked DB.",
    note: 'Real SQLite RAISE(ABORT) triggers on user insert and separately on session update; no engine run for failed insert, no delivered acknowledgment for failed completion flush. Actor must surface the durable failure, not swallow it. Queue replay journaling is reserved for U19 - do not implement it here.',
    extra: ['tests/test_multiplexer.rs']
  }
};

const ids = ['U16', 'U17', 'U18'];
const common = `
VERIFIED PREREQUISITES: U07/U08/U11/U13/U14/U15 all accepted with lead verification.
remote-control-verification.md documents the current setup_turn/execute_turn split, one absolute
entry deadline, typed result plumbing and interrupt semantics - consume these APIs, do not redesign
them. Accepted storage APIs: scoped pending writes, authority-guarded cron tables, atomic claim,
session-scoped review. U23-U25 are future units, not yours. Other workers own cron cutover,
dashboard and readiness files; no overlap.

MUST DO:
1. Read shared-design.md section2 (durable execution state machine, control transaction, canonical
   lane rules), your manifest scenario, and the current actor/backend/storage/session code. Exact
   repo paths only; never search the home directory.
2. Register exact regression IDs and capture nonzero behavioral RED before new production changes;
   a separate RED per constituent, not one combined run.
3. Minimal fix, identical GREEN, the full related target once (tests/test_omo_backend.rs for backend
   aspects, tests/test_multiplexer.rs for actor, lib session/storage tests for identity), LSP
   diagnostics before build, cargo build, scoped edition2021 format and diff check, real loopback
   WS/SQLite surfaces, event-gated bounded fixtures, no sleeps/polling/yield loops, joined cleanup.

STOP WHEN: Every constituent and adjacent proof passes with captured evidence, resources cleaned,
and no unowned edits remain. Do not claim the overall objective complete.
`;

const nodes = ids.map((id, index) => {
  const u = units[id];
  const files = [...new Set([...u.files, ...(u.extra || [])])];
  return {
    id: id.toLowerCase(),
    subagent_type: 'hephaestus',
    model: 'quotio/gemini-3.8-flash-high',
    dependsOn: index ? [ids[index - 1].toLowerCase()] : [],
    task_summary: ('3.8 Flash: ' + id + ' ' + u.title).slice(0, 80),
    prompt: [
      'TASK: Implement and prove ' + id + ': ' + u.title + ', covering only its registered findings.',
      'DELIVERABLE: Minimal scoped patch and ' + EV + '/' + id + '-evidence.md plus ' + id + '-commands.json with actual RED/GREEN/adjacent/build/cleanup outputs.',
      'SCOPE: ' + ROOT + '. WRITE ONLY ' + JSON.stringify(files) + ' and ' + id + '-* evidence. No other source, .env, production, Discord, global config, dependency or commit/push changes. Lead makes no source edits.',
      'VERIFY BINDING SCENARIO: ' + u.scenario,
      'Registered command to resolve exactly: ' + u.redCommand,
      'UNIT NOTES: ' + u.note,
      common
    ].join('\n')
  };
});

nodes.push({
  id: 'verify-remote-durable',
  subagent_type: 'hephaestus',
  model: 'quotio/gemini-3.8-flash-high',
  dependsOn: ['u16', 'u17', 'u18'],
  task_summary: '3.8 Flash: 바인딩·신원·부작용 안전 독립 검증',
  prompt: [
    'TASK: Independently verify U16/U17/U18 against full manifest scenarios and current code.',
    'DELIVERABLE: ' + EV + '/remote-durable-verification.md and captured logs with per-unit PASS/FAIL; no code edits.',
    'SCOPE: Read shared-design.md section2, all three manifests/evidence/commands, and the changed actor/backend/storage/session/migration files. Verified prior phases: remote-control-verification.md (U13-U15) and accepted U07/U08/U11 storage/approval APIs.',
    'VERIFY: Run each unique exact regression once plus the full backend and multiplexer targets once. Confirm binding persists across real recreation BEFORE turn submission, continuity error on missing rollout, stale actor copy never overwrites durable binding, twin-bot recovery preserves exact bot identity including ledger owner, and real SQLite triggers refuse engine execution and delivered acknowledgment on persistence failure. Inspect RED chronology; no fabricated chronology. Build, scoped edition2021 format, resource cleanup. No production/.env/Discord/commits.',
    'STOP WHEN: All three verdicts have actual evidence; missing proof stays FAIL. Final C3/C4 gates remain separate.'
  ].join('\n')
});

export default {
  key: 'hermes-parity-remote-durable-20260907',
  name: 'Durable binding identity and side-effect safety',
  nodes
};
