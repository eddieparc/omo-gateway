# U34 / D.D17

## Registered before production edits

Scenario: a backtick-fenced pipe example identical to a real table, followed by END.
Payload constructed exactly as: cell = "한" repeated 100; raw = "| Value |\n|---|\n| {cell} |";
payload = "```markdown\n{raw}\n```\n\n{raw}\nEND".

Fully qualified test: `discord::table_render::tests::fences_and_unbroken_cells_preserve_content`.
Literal RED and GREEN command:
`cargo test --lib discord::table_render::tests::fences_and_unbroken_cells_preserve_content -- --exact --nocapture`

Expected behavioral RED: fence_preserved=false and/or glyphs_inside=false,
one executed failing test and exit 101 (not zero tests or compiler failure).
Test measures actual usvg shaped text bounding boxes against the padded canvas,
counts all 100 Korean characters in the SVG, and calls the production transform
entry point used by both completed Stream and SendMessage in adapter.rs.
GREEN must preserve the literal example, render exactly one image, retain original
table bytes, keep all shaped text inside bounds, and decode the emitted PNG.

Baseline patch SHA256: dd23d51f825104cd4b4192256a8524370de9dfe171aceafa24d93d7eba3f7964.
No monitor tool or executable is exposed in this child. Shell jobs are launched
asynchronously and awaited using shell wait, without sleeps or polling.

## Captured RED (before production edits)

`U34-red.txt`: exit 101, 1 executed / 1 failed. Actual output:
```text
text bounds: Rect { left: 16.0, top: 57.4, right: 1226.9989, bottom: 74.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(89.0)) }
fence_preserved=false; glyphs_inside=false; images=2
test discord::table_render::tests::fences_and_unbroken_cells_preserve_content ... FAILED
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 286 filtered out; finished in 0.57s
EXIT_CODE=101
```

## Production patch and identical GREEN

Only `src/discord/table_render/mod.rs` changed in production/test code.
Public MarkdownTable/RenderedTable and public function signatures are unchanged.
Internal extraction carries UTF-8 byte spans, retaining CRLF, and skips backtick
and tilde fences until a matching marker of sufficient length closes them.
Transformation replaces only those spans, not identical examples elsewhere.
Wrapping breaks oversized words by character with a full-em budget for CJK.

The identical registered command ran successfully (`U34-green.txt`):
```text
fence_preserved=true; glyphs_inside=true; images=1
test discord::table_render::tests::fences_and_unbroken_cells_preserve_content ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 288 filtered out; finished in 0.43s
EXIT_CODE=0
```
Actual text bounds for the three full CJK lines end at x=403.5199, within the
480px canvas's padded right boundary x=464. The fourth ends at x=64.44.
Last text bottom y=134.2 is within height 149. SVG retains exactly 100 Korean
characters; the emitted production PNG decodes at double resolution, 960x298.

## Local production surface and adjacent controls

Command (captured in `U34-local.txt`):
`U34_PNG=.omo/evidence/hermes-parity-20260905/U34-local.png cargo test --lib discord::table_render::tests -- --nocapture`

This exercises the real `transform_markdown_tables_to_images` entry called by
adapter completed Stream and SendMessage, including real system-font shaping,
SVG rasterization and PNG decode; no mocked renderer or Discord transport.
The PNG was read through the image tool and visually inspected: the header and
all four lines (32/32/32/4 Korean characters) are visible, with right padding and
no clipping. This proves the rendering surface, not Discord attachment delivery
(D16 belongs to another unit).

All four tests passed, exit 0 (0 failed, 0 ignored):
- `discord::table_render::tests::fences_and_unbroken_cells_preserve_content`
- `discord::table_render::tests::table_source_span_and_fence_controls`
- `discord::table_render::tests::test_extract_markdown_tables`
- `discord::table_render::tests::test_render_table_to_svg_and_png`

Controls cover CRLF spans, repeated identical real tables with distinct image
references, matching/longer tilde and indented backtick closes, shorter/mismatched
and nonterminal closes, unclosed fences, plain input, header-only input, unbroken
ASCII content, multiline cells and ordinary two-column PNG output. Tests use no
timers, network, sleeps, polling, or async scheduling assumptions.

## Diagnostics, cleanup and limitations

One attempted LSP diagnostics call on the touched file failed because the daemon
was unreachable at `/Users/indo/.omo/lsp-daemon/v0.1.0/daemon.sock`; no repair was
attempted. Cargo compiled the library tests successfully. V2 single-domain
verification used the real rendering entry point rather than starting the live
gateway. No whole-repository build or suite claim is made.

Initial file-only rustfmt check found two formatting differences (`U34-format.txt`,
exit 1); both were corrected with apply_patch. Final
`rustfmt --check --edition 2021 src/discord/table_render/mod.rs` passed
(`U34-format-green.txt`, exit 0). `git diff --check -- src/discord/table_render/mod.rs`
also passed. Only formatting changed after the successful tests.

All spawned shell jobs were joined with wait. No services, temporary directories,
network requests, production state, environment/config files, dependencies or
commits were created. Retained PNG/logs are intentional U34 evidence. All writes
were restricted to the owned module and U34-* evidence. Other concurrent/user
changes remain untouched. The baseline.patch SHA256 was rechecked after edits
and remains exactly dd23d51f825104cd4b4192256a8524370de9dfe171aceafa24d93d7eba3f7964.
Programming/debugging skill files were not exposed at the searched standard
locations; no claim of reading unavailable skills or using unavailable monitor
infrastructure is made.

## Recovery-child revalidation (st_01a0711f)

This child found the production patch, tests, and preceding RED/GREEN artifacts
already present. It preserved them; the historical RED above was read from
`U34-red.txt`, not recreated or claimed as a new pre-edit execution. No further
production edits were needed. The exact registered selected command and the
registered local-surface command were executed again in an asynchronous shell
job joined with `wait` (monitor remains unavailable). Actual tool output:

```text
fence_preserved=true; glyphs_inside=true; images=1
test discord::table_render::tests::fences_and_unbroken_cells_preserve_content ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 300 filtered out; finished in 0.70s

test discord::table_render::tests::test_extract_markdown_tables ... ok
test discord::table_render::tests::test_render_table_to_svg_and_png ... ok
test discord::table_render::tests::fences_and_unbroken_cells_preserve_content ... ok
test discord::table_render::tests::table_source_span_and_fence_controls ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 297 filtered out; finished in 1.00s
SELECTED=0 ADJACENT=0 FORMAT=0 DIFF=0
```

The generated PNG was visually inspected again: all four text lines fit inside
the canvas. Both current adapter callers were read. File-only rustfmt and diff
checks passed. One LSP attempt in this child timed out waiting for fresh
diagnostics within 3000ms; no infrastructure repair was attempted. The baseline
patch hash still matches the value above. No background job remains from this
verification, and no other worker's files were edited.
