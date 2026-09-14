# Lane: tools-context

> Lane owner note: the delegated worker for this lane failed twice on provider errors
> (`401 Authentication Failed` on the first route, then `503 no available accounts for this model`).
> The review below was performed directly by the orchestrator, reading every target file and
> confirming every cited line with `sed`.

## Scope

| File | LOC | Read |
| --- | --- | --- |
| `src/tools/message_context.rs` | 1205 | full |
| `src/tools/message_context_lazy.rs` | 72 | full |
| `src/tools/cron.rs` | 968 | full |
| `src/tools/skills.rs` | 558 | full |
| `src/tools/web.rs` | 229 | full |

Related file read for cross-checking the FTS5 claim (owned by the `storage-db` lane, not re-reported here):
`src/storage/message_search.rs`.

## Findings

### [P1] web_fetch routes every URL through a third-party proxy, leaking the target and the content
- Location: src/tools/web.rs:159
- Evidence: `let fetch_url = format!("https://r.jina.ai/{}", parsed);`
- Why it matters: after validating that the scheme is http/https, the tool does not fetch the URL
  itself — it concatenates the URL onto `https://r.jina.ai/` and fetches that. Every page the agent
  reads, including internal or authenticated-looking URLs an operator pastes into a Discord thread,
  is disclosed to an external service along with whatever that service chooses to return. The
  returned content is fully attacker-controllable by that third party, and it is fed straight back
  into the model context. There is no opt-out, no allowlist, and no configuration to self-host.
- Suggested fix: fetch the URL directly with the existing `reqwest` client and use the reader
  service only as an explicit, configurable fallback. If the proxy stays, make the base URL a
  config value and document the disclosure.

### [P1] Response bodies are fully buffered before any size limit is applied
- Location: src/tools/web.rs:180
- Evidence: `.text()` — followed at src/tools/web.rs:184 by `let truncated: String = text.chars().take(max_chars).collect();`
- Why it matters: `max_chars` (default 100_000, max 1_000_000) is applied only *after* the whole
  body has been read into memory. A hostile or merely large endpoint returns a multi-gigabyte body
  and the gateway allocates all of it before truncating. The same shape exists in the search path at
  src/tools/web.rs:71, where the DuckDuckGo HTML is buffered whole with no cap at all. The 10s/15s
  request timeouts do not bound body size, only connection time per the reqwest timeout semantics.
- Suggested fix: stream the body with `bytes_stream()` and stop reading once `max_chars` worth of
  bytes have accumulated, or set a hard `Content-Length` rejection threshold before reading.

### [P1] Outbound fetch has no redirect policy, so scheme validation is bypassable
- Location: src/tools/web.rs:161
- Evidence: `let client = reqwest::Client::builder()` — the builder chain sets only `.timeout(...)` and `.user_agent(...)`; no `.redirect(...)` is configured, so reqwest's default of following up to 10 redirects applies.
- Why it matters: the scheme check at src/tools/web.rs:155 constrains only the URL the caller typed.
  Any redirect hop afterwards is followed with no re-validation. Combined with the proxy above this
  is mostly a disclosure issue, but if the direct-fetch fix in the first finding is applied without
  also pinning a redirect policy, it becomes a live SSRF path to loopback and link-local addresses.
- Suggested fix: set an explicit `redirect::Policy` that re-validates every hop's scheme and host,
  or disable redirects and surface the `Location` to the caller.

### [P1] Skill content is read into memory with no size bound
- Location: src/tools/skills.rs:196
- Evidence: `let content = std::fs::read_to_string(path).map_err(|e| {`
- Why it matters: `SKILL.md` files are discovered by scanning directories under `$HOME/.hermes/skills`
  and `$HOME/.omon/skills` (src/tools/skills.rs:20-21) and are read whole with no length check before
  being returned into the model context. A large file — accidental or planted — is loaded entirely
  into the process and then into the prompt. The read is also synchronous `std::fs` inside an async
  tool path, so it blocks the executor thread for the duration.
- Suggested fix: stat the file first and refuse or truncate above a fixed ceiling, and use
  `tokio::fs` on the async path.

### [P2] Skill discovery silently swallows unreadable directories
- Location: src/tools/skills.rs:96
- Evidence: `let Ok(entries) = std::fs::read_dir(dir) else {`
- Why it matters: a permissions error, a broken mount, or a dangling symlink makes an entire skills
  directory vanish from the listing with no log line. The operator sees "skill not found" with no
  way to distinguish "never existed" from "could not be read".
- Suggested fix: log at `warn!` with the directory and the error before returning.

### [P2] Cron tool payload parsing silently degrades a corrupt payload to an empty object
- Location: src/tools/cron.rs:409
- Evidence: `serde_json::from_str(&job.payload_json).unwrap_or_else(|_| json!({}));`
- Why it matters: if a job row's `payload_json` is malformed, the tool reports the job as if it had
  an empty payload rather than surfacing the corruption. The subsequent update path then writes that
  empty object back, so a display operation can quietly destroy the stored payload.
- Suggested fix: propagate the parse error for read paths, and refuse to write back a payload that
  failed to parse on read.

### [P2] Lazy provider hardcodes a relative database fallback
- Location: src/tools/message_context_lazy.rs:28
- Evidence: `.unwrap_or_else(|_| "sqlite://omon_gateway.db".to_owned()),`
- Why it matters: when `DATABASE_URL` is unset, the provider opens a database path relative to the
  process working directory. A gateway started from a different cwd than the main process silently
  initializes a second, empty database instead of failing, and message-context queries return
  nothing with no error. The default also still carries the pre-rename `omon_` filename.
- Suggested fix: reuse the single canonical database-URL resolution the rest of the process uses,
  and fail loudly when it is absent rather than inventing a relative path.

## Verified-not-a-defect

These were checked specifically because neighbouring lanes flagged similar shapes. They are sound
and are recorded here so the consolidated report does not re-raise them.

- **FTS5 query construction is safe.** `build_fts_query` (src/storage/message_search.rs:220) strips
  every double quote from each whitespace-separated term, re-wraps it as `"term"*`, and the result is
  bound as a parameter to `MATCH ?` at src/storage/message_search.rs:158. There is no FTS5 syntax
  injection and no SQL injection on this path.
- **Result limits are properly clamped.** src/tools/message_context.rs:86 clamps `limit` to
  `1..=100` and src/tools/message_context.rs:91 clamps `scan_limit` to `limit..=2000`, before
  `apply_policy_limits` (src/tools/message_context.rs:128) narrows them further against the policy
  matrix. Discord pagination is additionally capped per page at src/tools/message_context.rs:869.
  There is no unbounded-result path here.
- **Skill write paths are confined.** `validated_write_path` (src/tools/skills.rs:44) rejects any
  name outside `[A-Za-z0-9._-]`, and then walks each component with `symlink_metadata` +
  `canonicalize`, rejecting anything that does not remain under the canonical base
  (src/tools/skills.rs:70). Symlink-based escape is handled, which is more than most of the other
  filesystem paths in this codebase do.

## Strengths

- Limit handling in `message_context.rs` is layered and conservative: caller value, hard constant,
  and policy matrix all clamp independently, and the scan limit can never drop below the result limit.
- `validated_write_path` is genuinely careful about symlinks, and its tests
  (src/tools/skills.rs:373-410) assert the escape cases rather than only the happy path.
- The lazy provider uses `OnceCell::get_or_try_init`, so a failed pool initialization is retried on
  the next call instead of poisoning the provider permanently.

## Notes

- `src/tools/cron.rs` contains a large number of `unwrap()` calls, but they are confined to the
  `#[cfg(test)]` module below line 840; the production paths use `unwrap_or*` with defaults. Only the
  payload-degradation case above is a real defect.
- The `r.jina.ai` dependency is the single most consequential thing in this lane and it is a design
  decision as much as a bug — worth an explicit product call, not just a patch.
