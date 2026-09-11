# U55 / CFG.C04 - resumed dotenv fidelity repair

## Chronology and scope

Resumed child st_01a07205 from interrupted session 01a070bb-40f8-7c3e-a02e-e802d250b826. The owned file already contained U50 atomic/unique backup changes and a partially applied U55 dotenvy parser plus the registered regression. No production edits have been made by this resumed child at this registration point. Earlier logs are retained verbatim, not recreated.

The original U55-list.log resolves `migrate::config_import::tests::dotenv_round_trip_special_values` with one test. U55-commands.json already registers the exact command. U55-red.log and U55-red.exit record exit 101, one behavioral failure: exported DISCORD_BOT_TOKEN was None instead of Some("x"). U55-green.log contains only a build-lock wait; it is NOT a GREEN result. The resumed run of the identical registered regression fails later with Some("a") rather than Some("a # b"); see U55-resume-red.log. This is runtime evidence against the inherited parser-only state, not a claim of a fresh original-parser RED.

Owned source is only src/migrate/config_import.rs. Preserve all U50 hunks, other workers' dirty files and protected baseline.patch; no public API, dependency, schema, installed configuration or service changes. Current manifests, matching config audit, state-policy-plan, shared-design and predecessor U50 evidence were read. Upstream units introduce no additional U55 contract.

## Regression registration and payload

Exact ID: `migrate::config_import::tests::dotenv_round_trip_special_values`.

Literal RED/GREEN command: `cargo test --lib migrate::config_import::tests::dotenv_round_trip_special_values -- --exact --nocapture`.

Unchanged original payload: source `export DISCORD_BOT_TOKEN='x' # comment`; YAML API keys `a # b`, `$HOME ${HOME}`, ` both ' and " quotes \`, `plain`. Each is imported to both new and existing target documents. Existing raw bytes contain CRLF comments, `export KEEP='literal $HOME # value' # keep`, and a quoted physical multiline assignment. Expected binary observable: dotenvy::from_path_iter on a temporary rendered file yields token exactly x and API key exactly the YAML input; unrelated raw prefix remains byte-identical. No global environment load/export is used by the test.

Additional bounded controls before resumed production changes: exact raw-document round trip, and replacement of existing exported assignments with escaped source/YAML secrets. Literal invocations will be recorded in U55-commands.json after resolving their names.

## Artifact / resource journal

- Retain U55-* evidence only; create new command outputs as U55-resume-*.
- Unit-test disk fixtures use tempfile RAII. Importer fake paths are in-memory only.
- Real CLI fixture uses migrate --no-cutover with private temporary HOME, cwd, Hermes source, workspace, logs and SQLite; output is parsed with the actual dotenvy iterator without exporting variables into the parent.
- No monitor/eval/tool_schema API is exposed in this child's tool inventory; command lookup found no tool_schema executable. Use available bounded command execution, no sleeps, polling or orphan background commands. This is an execution-tool limitation, not a claim that requested monitor ran.
- Initial apply_patch via PATH timed out before writing any file; subsequent reads confirmed no artifacts or test additions. Use the same apply_patch program with /usr/bin/python3 to avoid the environment Python launcher. No global tool changes.

Verification results and cleanup follow after execution.

## Lead completion2026-09-06

Read current byte-preserving dotenvy parser, value renderer and merge path, plus actual original/resumed RED and GREEN files. Original export parser RED, resumed special-value/comment RED, raw-document and replacement RED each ran nonzero behavioral tests before their respective production changes. Current3 exact GREEN logs pass1 each, adjacent importer14 pass; lead independently reran importer module14 pass0fail. U08 current full cargo build includes this unchanged source and passed. Scoped rustfmt and diff check pass.

Actual compiled binary invoked with env -i/private HOME/HERMES_HOME/DATABASE_URL and cwd under /tmp/omon-U55-cli-SYbdpq, migrate --no-cutover. First invocation timed out180s before writing anything; diagnostic rerun sampled849 frames in _dyld_start (before Rust main), then completed exit0 without changing code/signing/environment settings. This is an explicit first-attempt runtime-launch limitation, not a hidden successful first run or evidence against the dotenv algorithm. Read-only codesign validation passed; xattr reported provenance; no security configuration changed.

The resulting target was parsed by U55-cli-verify.rs using the real linked dotenvy::from_path_iter: token=true, secret=true, raw_prefix=true, exit0. It verifies exported token/comment, YAML a # b $HOME and unrelated raw assignment equality. Full CLI/loader/reload output: U55-lead-cli.log. Runtime fixture, SQLite, generated environment/backups and verifier binary were removed with exact-root cleanup after monitor exits. No project .env, production HOME or real Discord operations. Only synthetic fixture credentials existed.

Source/verify-script LSP initially reported no diagnostics; final source retry timed out3000ms. Fixture YAML LSP unavailable and .env has no configured server; actual parser/CLI compilation/runtime validation used instead, no dependency/global config installation. Final source formatting passes. U55 accepted at its bounded import-value contract; broader config policies and migration cutover remain separate open units.
