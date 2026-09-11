# U34 Lint Verification Evidence

## Overview

- **Task**: Fix exactly two attributable strict-clippy errors in `src/discord/table_render/mod.rs`.
- **Target File**: `src/discord/table_render/mod.rs`
- **Scope Limit**: Only `src/discord/table_render/mod.rs` and `U34-lint-*` evidence files. No edits to other worker files, config, dependencies, or git commits.

## Pre-Fix Attributable Clippy Diagnostics

Running `cargo clippy --lib -- -D warnings` reported two attributable style defects in `src/discord/table_render/mod.rs`:

1. `clippy::collapsible_if` at line 165:
   ```text
   error: this `if` statement can be collapsed
      --> src/discord/table_render/mod.rs:165:17
       |
   165 | /                 if current.chars().count() + word.chars().count() + 1 > max_line_chars {
   166 | |                     if !current.is_empty() {
   167 | |                         wrapped.push(current.clone());
   168 | |                         current.clear();
   169 | |                     }
   170 | |                 }
       | |_________________^
       = help: collapse nested if block
   ```

2. `clippy::field_reassign_with_default` at lines 300-301:
   ```text
   error: field assignment outside of initializer for an instance created with Default::default()
      --> src/discord/table_render/mod.rs:301:5
       |
   301 |     opt.font_family = "sans-serif".to_string();
       |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   note: consider initializing the variable with `Options::<'_> { font_family: "sans-serif".to_string(), ..Default::default() }` and removing relevant reassignments
      --> src/discord/table_render/mod.rs:300:5
       |
   300 |     let mut opt = usvg::Options::default();
       |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   ```

## Minimal Applied Patch

Both defects were resolved with minimal, behavior-preserving changes:
- **Defect 1 (`collapsible_if`)**: Collapsed nested `if` statements into a single condition with `&& !current.is_empty()`. The predicate evaluation order and short-circuit semantics remain identical.
- **Defect 2 (`field_reassign_with_default`)**: Initialized `usvg::Options` directly via struct literal syntax with `font_family: "sans-serif".to_string()` and `..Default::default()`. All options and system font loading logic remain identical.
- **No Allow Attributes**: No `#[allow(...)]` annotations were added.
- **Preservation of U34 Invariants**: All wide table columns, backtick/tilde code-fence span extraction, full-em CJK word-wrapping budget, and 2x lossless PNG rasterization logic are preserved without modification.

### Diff Patch
```diff
--- a/src/discord/table_render/mod.rs
+++ b/src/discord/table_render/mod.rs
@@ -162,11 +162,10 @@ pub fn render_table_to_svg(table: &MarkdownTable) -> String {
             let max_line_chars = ((width - cell_padding_x * 2.0) / font_size).max(1.0) as usize;
             let mut current = String::new();
             for word in paragraph.split_whitespace() {
-                if current.chars().count() + word.chars().count() + 1 > max_line_chars {
-                    if !current.is_empty() {
-                        wrapped.push(current.clone());
-                        current.clear();
-                    }
+                if current.chars().count() + word.chars().count() + 1 > max_line_chars
+                    && !current.is_empty()
+                {
+                    wrapped.push(current.clone());
+                    current.clear();
                 }
                 if !current.is_empty() {
                     current.push(' ');
@@ -297,8 +296,10 @@ fn html_escape(text: &str) -> String {
 }
 
 pub fn svg_to_png(svg_str: &str, scale: f32) -> Result<Vec<u8>, String> {
-    let mut opt = usvg::Options::default();
-    opt.font_family = "sans-serif".to_string();
+    let mut opt = usvg::Options {
+        font_family: "sans-serif".to_string(),
+        ..Default::default()
+    };
     opt.fontdb_mut().load_system_fonts();
 
     let tree = usvg::Tree::from_str(svg_str, &opt).map_err(|e| e.to_string())?;
```

## Validation Evidence

### 1. LSP Diagnostics
LSP language server diagnostics for `src/discord/table_render/mod.rs` report zero errors, warnings, or hints:
```text
No diagnostics found
```

### 2. Scoped Formatting and Diff Check
- `rustfmt --check --edition 2021 src/discord/table_render/mod.rs`: Exit code 0 (no format discrepancies).
- `git diff --check -- src/discord/table_render/mod.rs`: Exit code 0 (clean diff, no whitespace or conflict errors).

### 3. Clippy Diagnostics Audit
Running `cargo clippy --lib --message-format=json` confirms:
- Attributable errors in `src/discord/table_render/mod.rs`: **0**
- Attributable warnings in `src/discord/table_render/mod.rs`: **0**

Exact non-attributable compiler/clippy findings in other worker files (untouched per scope requirements):
- `src/migrate/sys.rs:225` (`unused_mut`)
- `src/migrate/sys.rs:226` (`unused_mut`)
- `src/agent/omo_backend.rs:595` (`clippy::single_match`)
- `src/agent/omo_backend.rs:827` (`clippy::collapsible_if`)

### 4. Existing Table Render Unit Tests
Command: `cargo test --lib discord::table_render::tests -- --nocapture`

Output:
```text
running 4 tests
test discord::table_render::tests::test_extract_markdown_tables ... ok
test discord::table_render::tests::test_render_table_to_svg_and_png ... ok
text bounds: Rect { left: 16.0, top: 13.4, right: 50.3, bottom: 30.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(149.0)) }
text bounds: Rect { left: 16.0, top: 57.4, right: 403.5199, bottom: 74.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(149.0)) }
text bounds: Rect { left: 16.0, top: 77.4, right: 403.5199, bottom: 94.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(149.0)) }
text bounds: Rect { left: 16.0, top: 97.4, right: 403.5199, bottom: 114.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(149.0)) }
text bounds: Rect { left: 16.0, top: 117.4, right: 64.44, bottom: 134.2 }; canvas: Size { width: NonZeroPositiveF32(FiniteF32(480.0)), height: NonZeroPositiveF32(FiniteF32(149.0)) }
fence_preserved=true; glyphs_inside=true; images=1
test discord::table_render::tests::fences_and_unbroken_cells_preserve_content ... ok
test discord::table_render::tests::table_source_span_and_fence_controls ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 331 filtered out; finished in 1.22s
```

### 5. Asset Preservation
No pre-existing or shipped assets were regenerated or modified (e.g., `U34-local.png` was untouched). No new tests were added for pure lint fixes.
