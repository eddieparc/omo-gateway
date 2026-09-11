# U06 - AP.AP13

## Registered before production edits

Exact test ID: `tools::skills::tests::skill_writes_cannot_escape_or_bypass_staging`.

Literal RED and GREEN command:
`cargo test --lib tools::skills::tests::skill_writes_cannot_escape_or_bypass_staging -- --exact --nocapture`

Payload: action=write, content=u06-sentinel, name=`../outside`, absolute temporary
outside path, or safe-skill with an in-root symlink to outside (also dangling
SKILL.md symlink to outside/SKILL.md). Each exercises real Tool::execute and a
seeded pending row through approve_pending_write with a temporary root.
Expected RED: success and outside/SKILL.md exists. GREEN: error and absent file;
replay row retained. Stage-link must reject before staging.

WRITE_APPROVAL=true without pool: safe-skill RED saved and file exists; GREEN
error and no skill directory. Happy control stages with SQLite, checks no file
before approval, consumes row on apply, and exercises read/list/search.

Private subprocess environment avoids global environment races; no sleeps,
polling or external network. TempDir RAII and process teardown clean failures;
successful scenarios explicitly close pool and temporary directory.

Monitor tool and executable unavailable; available shell runner used. Initial
patch application failed before changing files and produced a zero-test run;
that run is NOT behavioral RED. Registered run below replaces that output.

## Captured RED (before production patch)

U06-red.log: exit 101, outer test 0 passed / 1 failed; all eight direct/replay
escape scenarios reported outside/SKILL.md=true and success. Missing-store
reported status=saved and inside/SKILL.md=true. Stage-link returned staged.
Ten failing scenarios; happy scenario passed. This is behavioral RED, not a
compile failure or zero-test run.

## GREEN and local-surface proof

Identical registered command: U06-green.log, exit 0, outer test 1 passed /
0 failed. All eleven subprocess scenarios exit 0; eight escape cases return
errors and outside/SKILL.md=false. Missing-store returns error with inside and
outside files absent. Stage-link rejects. Happy staging/apply/read/list/search
passes against real SQLite and temporary filesystem, not mocks. This is the
public Tool and production apply local surface; no live daemon agent write
caller is established, and no Discord or production agent was invoked.

Adjacent commands and captured results:

- `cargo test --lib storage::db::tests::test_pending_writes_store_round_trip -- --exact --nocapture`
  -> U06-adjacent.log: exit 0, 1 passed. Existing memory/skill stage, apply,
  reject and pending-row lifecycle remain working.
- `cargo test --lib tools::skills::tests::skill_write_destination_adjacent_controls -- --exact --nocapture`
  -> U06-controls.log: exit 0, 1 passed. Empty/dot/dotdot/nested/slash names
  fail without creating a root; valid dotted/hyphenated skill creates a missing
  root on approval; an existing external SKILL.md symlink target stays unchanged.
- `cargo build` -> U06-build.log: exit 0, dev build successful.
- `git diff --check -- src/tools/skills.rs src/storage/db.rs` -> exit 0.

LSP diagnostics attempted once for each touched file, both unavailable:
`LSP daemon unreachable: LSP daemon did not become reachable at
/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock.` No repair attempted.

## Implementation and scope

src/tools/skills.rs:44 shares a side-effect-free validated destination resolver:
single ASCII alphanumeric/dot/underscore/hyphen name excluding dot/dotdot;
canonical configured root and canonical existing skill/file targets; rejects
escaping or dangling target symlinks. Missing roots resolve via their existing
ancestor. Line 220 fails closed when approval lacks a pool, before filesystem
creation. Line 236 validates before either staging or direct save.
src/storage/db.rs:374 uses the same resolver on replay. Public signatures and
payload shape are unchanged. Tests are in skills.rs:277 and :303.

This fixes the registered static path/symlink payloads. It does not claim
descriptor-relative protection against a concurrent hostile filesystem rename
between validation and write. Exact staged destination persistence and atomic
pending consumption belong to U07/AP14 and were not changed here.

## Cleanup and ownership

All successful fixtures close their SQLite pool and TempDir explicitly.
Failed RED fixtures use TempDir unwinding/process teardown; five temporary
root paths recoverable from RED output were checked and none remained.
No network listeners, real state, .env, dependencies, global config or commits.
Only src/tools/skills.rs, src/storage/db.rs and U06-* artifacts were written by
this child. Other modified files shown by final git status are concurrent
workers or the original protected user edits and were not touched. The
baseline.patch artifact was neither written nor rewritten. Build/test logs
remain intentionally as evidence; no debug code or temporary test skipping.

Shell runner was synchronous because this child exposes no monitor tool or
monitor executable; this execution-method limitation is explicit, not hidden.

## Recovery-session revalidation (st_01a0711d)

The production patch, regression tests, registration and historical RED/GREEN
logs already existed on entry to this session. They were inspected and
preserved, not reverted or recreated. The historical RED log records exit 101
and the ten behavioral failures described above; its pre-edit chronology is
inherited evidence, not a newly observed RED execution in this session.

Every command in U06-commands.json was rerun unchanged, once, against the current
workspace. U06-recheck-green.log records exit 0, one exact selected test and all
eleven successful subprocess scenarios through real Tool::execute and the real
SQLite replay boundary. U06-recheck-controls.log and U06-recheck-adjacent.log
each record exit 0, one test passed. U06-recheck-build.log and
U06-recheck-diff.log record exit 0. Successful fixtures explicitly closed pools
and temporary directories. No production files were edited in this recovery
session, and baseline.patch was not touched.

LSP diagnostics were attempted once per owned production file in this session;
both returned `No diagnostics found`, superseding the earlier infrastructure
limitation for this recheck. No global repair was performed. A monitor tool and
executable remain unavailable; independent shell commands ran concurrently in
a bounded Python executor with subprocess completion waits (600-second bounds),
without polling or sleeps. Output was captured and added with apply_patch.

Current references: shared validator skills.rs:44; fail-closed pool check :220;
validation before staging/save :236; adjacent test :277; registered test :315;
shared replay validation storage/db.rs:374. Public signatures remain unchanged.
