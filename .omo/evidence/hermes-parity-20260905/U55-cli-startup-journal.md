# Isolated CLI startup journal

Fixture root /tmp/omon-U55-cli-SYbdpq was created by lead only; source .env/config and target .env contain synthetic sentinel values, no real credentials. All created files and verifier binary beneath this root must be removed after proof. Retain only U55-* evidence source/logs in repository. No real project .env was read or edited.

First real target/debug/omo-gateway migrate --no-cutover with env -i, private HOME/HERMES_HOME/DATABASE_URL timed out at180s with no output. Fixture target remained unchanged and no SQLite file existed; process listing confirmed no surviving fixture child. This is NOT a dotenv failure or successful CLI validation.

Diagnostic run mon_4CNW4Y0X9G9T5T2R uses same isolated inputs with RUST_LOG=debug and samples the owned PID62508 once. Captured849 samples solely in _dyld_start, before Rust main. This refutes app-level import/database deadlock as the observed stopping point; loader/signature environment is the current hypothesis. Signature/xattr/unified-log inspection is read-only. No security settings or signing metadata will be changed merely to make QA pass.

The second CLI monitor is still the live exit channel; do not start a duplicate normal invocation or delete its fixture while it is active. The real dotenvy verifier has not run yet because the first command never completed.
