# Lane: discord-commands
## Scope
- `src/discord/commands.rs`: 2,266 LOC, read fully, including poise command checks, slash commands, helper functions, and test cases.
- `src/discord/throttler.rs`: 502 LOC, read fully, including `LiveEditThrottler`, markdown chunking and pagination, split bounding, and test cases.
- `src/discord/table_render/mod.rs`: 464 LOC, read fully, including markdown table parsing, SVG layout generation, resvg/usvg PNG rasterization, and test cases.
- `src/discord/attachments.rs`: 633 LOC, read fully, including `AttachmentDownloader`, WAV PCM decoding, MIME/extension detection, and test cases.
- `src/discord/mod.rs`: 40 LOC, read fully, module declarations and public re-exports.

## Findings
### [P0] TOCTOU in `/compress` causes silent data loss of messages arriving during summarization
- Location: src/discord/commands.rs:1092
- Evidence: `sqlx::query("DELETE FROM messages WHERE session_key = ?")`
- Why it matters: In `/compress`, conversation history is selected at T0 (`SELECT sequence, role, content FROM messages WHERE session_key = ?`). If an LLM client is configured, it streams a summary via `llm.stream(...)`, which typically takes 5 to 30 seconds. While the LLM request is in-flight, new messages from the user or agent can arrive and be inserted into the database. When the LLM stream completes at T1, line 1092 executes an unconditional `DELETE FROM messages WHERE session_key = ?` inside a transaction before inserting the summary. All user and assistant messages inserted between T0 and T1 are permanently wiped from the database without being summarized, causing unrecoverable data loss.
- Suggested fix: Record the maximum `sequence` number fetched at T0 (`max_seq = rows.last().map(|(s, ..)| *s)`), and scope the deletion strictly to processed messages: `DELETE FROM messages WHERE session_key = ? AND sequence <= ?` bound with `max_seq`.

### [P0] Unbounded table dimensions in PNG rasterization allow multi-gigabyte memory allocations and process abort
- Location: src/discord/table_render/mod.rs:313
- Evidence: `let mut pixmap = resvg::tiny_skia::Pixmap::new(pixmap_size.width(), pixmap_size.height())`
- Why it matters: Markdown tables parsed by `extract_markdown_table_spans` have no maximum limit on row count or column count. In `render_table_to_svg`, each column is clamped to a minimum width of 160.0 px (`col_widths = (ratio * target_total_width).clamp(160.0, 480.0)`), and height increases by at least 44 px per row. An agent or tool output generating a large table (e.g., 250 columns and 1,000 rows) produces a canvas of 40,000 x 44,000 pixels. In `svg_to_png`, scaling by 2.0x requests a pixmap of 80,000 x 88,000 pixels (7.04 billion pixels = 28.16 GB of RGBA memory). `tiny_skia::Pixmap::new` attempts this allocation and either fails with an unhandled allocation error or aborts the entire process via an out-of-memory (OOM) panic, creating a reachable denial-of-service vulnerability.
- Suggested fix: Enforce maximum limits on table rows (e.g., max 50 rows) and columns (e.g., max 10 columns), or cap total rendered width (e.g., 1920 px) and height (e.g., 4000 px). If a table exceeds these bounds, truncate rows/columns with an indicator or bypass PNG rendering and keep the raw text.

### [P0] Unrestricted attachment download URL enables SSRF and private network exfiltration
- Location: src/discord/attachments.rs:352
- Evidence: `.get(&attachment.url)`
- Why it matters: `AttachmentDownloader::download` sends an HTTP GET request to `attachment.url` without validating the URL scheme (e.g., forcing HTTPS) or restricting the host to Discord's media CDN (`cdn.discordapp.com`, `media.discordapp.net`). Furthermore, reqwest's client follows HTTP redirects by default. If an attacker supplies an attachment URL pointing to cloud instance metadata (`http://169.254.169.254/latest/meta-data/`) or an internal network address, or if a remote server returns an HTTP 302 redirect to an internal IP, reqwest fetches the sensitive internal data. If `is_text_attachment` evaluates to true (e.g., based on filename or MIME), `hydrate` loads the fetched data directly into `attachment.text_content`, making internal cloud secrets accessible in LLM context and egress responses.
- Suggested fix: Validate that `attachment.url` parses to an HTTPS URL with an allowed hostname (`cdn.discordapp.com` or `media.discordapp.net`). Configure `reqwest::redirect::Policy::none()` on the downloader's HTTP client to prevent redirect-based SSRF.

### [P1] Synchronous system font loading blocks async Tokio worker thread on every table render
- Location: src/discord/table_render/mod.rs:304
- Evidence: `opt.fontdb_mut().load_system_fonts();`
- Why it matters: `svg_to_png` instantiates a new `usvg::Options` and calls `opt.fontdb_mut().load_system_fonts()` synchronously every time a markdown table is rasterized to PNG. Loading system fonts performs synchronous filesystem traversal and parses all installed TTF/OTF font files from disk. Because `transform_markdown_tables_to_images` is invoked directly in the async message pipeline (`adapter.rs:2661`, `2743`) without `tokio::task::spawn_blocking`, this disk-heavy operation blocks the calling Tokio runtime thread for 50-300ms per table, causing latency spikes and thread starvation under concurrent message traffic.
- Suggested fix: Initialize the `usvg::fontdb::Database` once using a `LazyLock<Arc<usvg::fontdb::Database>>` or global cache, and share it across calls. Offload `svg_to_png` rendering to `tokio::task::spawn_blocking`.

### [P1] Byte-count comparison against character limit in `bound_split_messages` prematurely truncates multi-byte UTF-8 messages
- Location: src/discord/throttler.rs:335
- Evidence: `if last.len() > budget {`
- Why it matters: Discord enforces a 2,000 Unicode character limit per message. `bound_split_messages` calculates `budget` as `DISCORD_MESSAGE_LIMIT.saturating_sub(TRUNCATION_NOTICE.len())` (2,000 - 68 = 1,932 bytes), but checks `last.len() > budget` using string byte length (`last.len()`). In multi-byte languages such as Korean, Japanese, Chinese, or messages containing emoji, characters occupy 3 to 4 bytes each. A completely valid 1,000-character message in Korean is ~3,000 bytes. The check evaluates true and forcibly truncates the final chunk down to 1,932 bytes (~644 characters), unnecessarily discarding over 350 valid characters.
- Suggested fix: Compare character counts instead of byte lengths: `if last.chars().count() > budget_chars` where `budget_chars = DISCORD_MESSAGE_LIMIT.saturating_sub(TRUNCATION_NOTICE.chars().count())`, and truncate using character boundary indices rather than byte offsets.

### [P1] `/skills action:search` response is sent without chunking and exceeds Discord 2,000-character limit
- Location: src/discord/commands.rs:558
- Evidence: `ctx.say(format!("🔍 **Skill Search Results for `{query_val}`** ({count} matches):\n- {list_str}")).await?;`
- Why it matters: While `/skills action:list` explicitly paginates replies using `chunk_slash_reply(&reply_text, 2000)`, the `search` handler joins all search match strings with `\n- ` and directly passes the formatted string to `ctx.say`. If a query matches many skills or skills with long names/descriptions, the output easily exceeds 2,000 characters. Discord's API rejects messages over 2,000 characters with HTTP 400 Bad Request (`Invalid Form Body: BASE_TYPE_MAX_LENGTH`), causing the slash command to fail with an unhandled error.
- Suggested fix: Wrap the output of `search` in `chunk_slash_reply` and iterate over chunks with `ctx.say`, identical to the `list` handler.

### [P1] `/tools` response concatenates unbounded tool/endpoint lists and exceeds Discord 2,000-character limit
- Location: src/discord/commands.rs:887
- Evidence: `ctx.say(format!("Tools: {tools}\nMCP endpoints:\n{endpoints}"))`
- Why it matters: In `/tools`, all configured tool names (`ctx.data().tools`) and MCP server endpoints (`ctx.data().mcp_endpoints`) are joined into comma- and newline-separated strings and sent in a single `ctx.say(...)` call. In deployments with dozens of tools and long MCP URLs or headers, the concatenated string exceeds Discord's 2,000-character limit, resulting in HTTP 400 rejection and slash command failure.
- Suggested fix: Chunk the formatted tools and endpoints text using `chunk_slash_reply` or truncate to 2,000 characters before sending.

### [P1] `/steer` embeds untruncated user guidance into ephemeral reply exceeding Discord 2,000-character limit
- Location: src/discord/commands.rs:961
- Evidence: `.content(format!("🎯 Steering guidance queued: `{text}`"))`
- Why it matters: Discord slash command string options permit inputs up to 6,000 characters. In `/steer`, the user-provided `text` argument is interpolated directly into `format!("🎯 Steering guidance queued: `{text}`")`. If `text` exceeds 1,968 characters, the ephemeral interaction reply exceeds 2,000 characters, causing Discord to reject the interaction response with HTTP 400 Bad Request.
- Suggested fix: Use `preview_text(&text, 100)` (which is already implemented and used in `undo` and `retry`) to truncate the echoed text in the confirmation message.

### [P1] Discord thread name length validation missing in `/title` and `/thread` commands
- Location: src/discord/commands.rs:1126
- Evidence: `let builder = serenity::EditThread::new().name(text.trim());`
- Why it matters: Discord's REST API strictly requires thread names to be between 1 and 100 characters in length. In `/title` (line 1126) and `/thread` (line 1167: `CreateThread::new(name.trim())`), neither command validates `name.trim().is_empty()` or `name.trim().chars().count() > 100`. If a user passes an empty string, whitespace only, or a title longer than 100 characters, Serenity's API call fails with HTTP 400 Bad Request (`Invalid Form Body: name: Must be between 1 and 100 in length`), surfacing an unhandled error to the user.
- Suggested fix: Validate that trimmed thread names are between 1 and 100 characters before calling `EditThread` or `CreateThread`. If invalid, return an informative ephemeral error message.

### [P1] Temporary `.part` download files leak indefinitely on cancelled futures
- Location: src/discord/attachments.rs:375
- Evidence: `.join(format!(".{}.part", Uuid::new_v4()));`
- Why it matters: `AttachmentDownloader::download` creates temporary download files named `.{uuid}.part` in `.discord-attachments`. The cleanup call `tokio::fs::remove_file(&temp)` is only invoked when `stream_to_file` returns `Err`. If the download future is cancelled (e.g., when the caller times out or the client connection is dropped mid-stream), Tokio drops the future at an `.await` boundary, bypassing the error branch. Because there is no startup purge or background garbage collection of `.part` files, orphaned temporary files accumulate indefinitely on disk, causing a permanent storage leak.
- Suggested fix: Implement a drop-guard (RAII temp-file wrapper) that removes the partial file when dropped unless marked committed, and scan/remove `.part` files in `AttachmentDownloader::new`.

### [P1] `decode_wav_pcm` parses audio data as 16-bit PCM without validating format tag or bit depth
- Location: src/discord/attachments.rs:35
- Evidence: `if chunk_id == b"fmt " && chunk_data.len() >= 16 {`
- Why it matters: `decode_wav_pcm` reads `channels` (offset 2..4) and `sample_rate` (offset 4..8) from the `fmt ` chunk, but fails to check `wFormatTag` (offset 0..2) and `wBitsPerSample` (offset 14..16). It unconditionally assumes the `data` chunk contains 16-bit linear PCM (`as_chunks::<2>()`). If an inbound WAV file is 8-bit PCM, 24-bit PCM, 32-bit float, or compressed (e.g., mu-law or ADPCM), the decoder misaligns chunk bytes and interprets raw non-16-bit samples as `i16` integers. This delivers completely corrupted audio frames / screeching static to the Speech-to-Text transcriber, causing transcription failure or hallucinated text.
- Suggested fix: In `decode_wav_pcm`, verify that `wFormatTag == 1` (WAVE_FORMAT_PCM) and `wBitsPerSample == 16`. Return `None` for non-16-bit or non-PCM audio so it falls back to the Opus/raw audio container pipeline.

### [P1] Incomplete two-pass convergence in `chunk_markdown_paginated` causes mislabeled `(i/N)` headers
- Location: src/discord/throttler.rs:307
- Evidence: `if chunks.len() != total {`
- Why it matters: When pagination headers `(i/N)\n` are prepended to chunks, the available character budget per chunk decreases. This reduction can push text across chunk boundaries and increase the total number of chunks from N to N+1. The convergence logic only checks `if chunks.len() != total` once. If the second pass splits into additional chunks (e.g., expanding from 3 to 4 chunks), the function does not loop to recompute `total = 4`. As a result, chunks are sent to Discord with invalid headers such as `(1/3)`, `(2/3)`, `(3/3)`, `(4/3)`.
- Suggested fix: Replace the single `if` statement with a bounded loop (e.g., up to 4 iterations) until `chunks.len() == total`, or conservatively pre-budget `(i/N)\n` based on maximum chunk count.

### [P1] `html_escape` retains XML 1.0 illegal control characters causing `usvg::Tree::from_str` parser failure
- Location: src/discord/table_render/mod.rs:291
- Evidence: `fn html_escape(text: &str) -> String {`
- Why it matters: `html_escape` substitutes `&`, `<`, `>`, `"`, and `'`, but does not strip or replace ASCII control characters (`0x00 - 0x08`, `0x0B - 0x0C`, `0x0E - 0x1F`). In the XML 1.0 specification (which SVG adheres to), these control characters are well-formedness violations and cannot be represented even as numeric character entities. When markdown table cells contain command output, logs, or terminal escape sequences containing control characters, `usvg::Tree::from_str` fails with an XML parsing error, and rasterization fails.
- Suggested fix: Filter out ASCII control characters (except `\t` and `\n`) in `html_escape`: `text.chars().filter(|&c| c >= ' ' || c == '\t' || c == '\n')`.

### [P1] `LiveEditThrottler` holds mutex lock across network I/O and issues redundant `start_typing` on final update
- Location: src/discord/throttler.rs:200
- Evidence: `self.transport.start_typing(self.channel_id).await?;`
- Why it matters: In `LiveEditThrottler::update`, `state = self.state.lock().await` is held while performing multiple remote Discord network operations (`start_typing`, `edit_message`, `send_message`, `delete_message`). If the Discord REST API experiences latency or rate-limiting, all other threads awaiting debounce in `wait_for_debounce` block on the mutex. Furthermore, `start_typing` is invoked unconditionally on line 200 before checking `if !is_final`. When a streaming response completes and `is_final` is true, a typing indicator is broadcast to Discord, causing the bot to show "typing..." for 10 seconds after the message has already finalized.
- Suggested fix: Only call `start_typing` when `!is_final`. Release or minimize the duration of the mutex lock around individual transport network calls.

### [P2] `/undo` and `/retry` mutate session messages without stopping active turns in multiplexer
- Location: src/discord/commands.rs:974
- Evidence: `match undo_last_exchange(&ctx.data().pool, &storage_key).await? {`
- Why it matters: Unlike `/stop` and `/reset` (which call `multiplexer.stop` and `multiplexer.reset`), `/undo` and `/retry` delete messages directly from SQLite while an agent turn may be actively executing in the multiplexer actor. If a user runs `/undo` while the bot is generating an answer, the actor's subsequent write will interleave with deleted rows, causing message sequence inconsistency and stale LLM history.
- Suggested fix: Call `ctx.data().multiplexer.stop(&key).await` in `undo` and `retry` before executing the database deletion.

### [P2] Unrecognized `mode` argument in `/yolo` unexpectedly toggles YOLO mode instead of returning validation error
- Location: src/discord/commands.rs:1265
- Evidence: `_ => !effective,`
- Why it matters: In `yolo_toggle`, the `match` on `mode` treats any unrecognized string (such as `status`, `check`, or `help`) as the wildcard arm `_ => !effective`, toggling YOLO mode instead of validating the argument or returning a usage error. A user querying `/yolo mode:check` unexpectedly flips safety approval bypass on or off.
- Suggested fix: Match `None => !effective`, and return an error for unrecognized `Some(unknown)` strings: "Invalid mode. Use 'on' or 'off'".

### [P2] `thread_sessions_per_user` configuration field is populated in `PoiseData` but unused in routing
- Location: src/discord/commands.rs:29
- Evidence: `pub thread_sessions_per_user: bool,`
- Why it matters: `pub thread_sessions_per_user: bool` is stored in `PoiseData` and passed to `InboundFilterConfig`, but no code path in `commands.rs` or `adapter.rs` reads or branches on this setting. Thread session routing always derives `user_id` based solely on whether the channel is a DM.
- Suggested fix: Either implement user-isolated thread session keys when `thread_sessions_per_user` is enabled, or remove the unused configuration field.

### [P2] Starter message in `/thread` is routed to agent multiplexer but never posted to Discord thread
- Location: src/discord/commands.rs:1192
- Evidence: `let _ = ctx.data().multiplexer.route(event).await;`
- Why it matters: In `/thread`, if `message` is provided, `starter_msg` is routed directly to the multiplexer as an event for the new thread, but it is never sent as a message to the Discord thread. The created thread appears completely empty on Discord until the agent responds to the invisible prompt.
- Suggested fix: Call `created_thread.id.send_message(...)` with `starter_msg` before or when routing the event to the multiplexer.

### [P2] `is_voice_attachment` fallback omits `.wav` extension when Content-Type is missing
- Location: src/discord/attachments.rs:90
- Evidence: `|| lower.ends_with(".opus")`
- Why it matters: When `content_type` is `None`, line 90 only checks `.ends_with(".ogg") || .ends_with(".opus")`. A `.wav` file uploaded without a MIME header is ignored by voice detection unless its filename contains "voice-message".
- Suggested fix: Add `|| lower.ends_with(".wav")` to the filename extension fallback check.

### [P2] `scan_fences` in `throttler.rs` mishandles 4+ backtick fences and fails to track tilde (`~~~`) code blocks
- Location: src/discord/throttler.rs:431
- Evidence: `if let Some(after_fence) = trimmed.strip_prefix("```") {`
- Why it matters: `trimmed.strip_prefix("```")` strips only 3 backticks, causing 4+ backtick blocks (e.g. ````rust) to capture leading backticks in the language tag (`reopen` becomes ````\n`), and tilde code fences (`~~~`) are entirely ignored.
- Suggested fix: Count fence markers and match opening/closing fence lengths dynamically, supporting both `` ` `` and `~`.

## Strengths
- Slash command admission logic (`check_slash_admission`) cleanly separates user authentication, role allowlists, pairing store records, and channel blacklists/whitelists with explicit fail-closed semantics.
- Attachment path sandboxing in `AttachmentDownloader` performs rigorous path sanitization, symlink inspection, and canonicalization checks against workspace escaping.
- Code fence tracking in `chunk_markdown` preserves syntax validity across message boundaries by closing and reopening active language fences in subsequent chunks.
- YOLO state toggling and destructive command confirmation strictly persist changes in SQLite before invoking cache updates, preventing lock inversions on single-connection pools.

## Notes
- `command_check` handles slash commands globally, but prefix command execution (`!stop`, `!pair`) receives the same check; however, bot-account filtering (`allow_bots`) is not checked in poise command checks, relying on Discord's native slash-command bot restrictions.
