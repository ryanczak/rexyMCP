# Phase 07: Executor tooling improvements

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** review
**Depends on:** none (additive changes to existing tools; independent of phases 03–06)
**Estimated diff:** ~450 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

Bring the executor's tool surface to parity with a capable coding harness.
Seven improvements spread across existing tools and three new tools, all
additive. No new `Cargo.toml` dependency — all changes use crates and
`std` APIs already in the tree.

### What the audit found is already good (do not re-implement)

Before touching anything, read `patch.rs` — it already has:
- `fuzzy_hint` (closest-window hint on 0-match mismatch, `patch.rs:194-233`)
- unified diff in the success output (`patch.rs:162-167`)

These are done. The spec below does not touch them.

## Architecture references

- `executor/src/tools/mod.rs` — pub re-exports; add new tools here.
- `mcp/src/runner.rs:134-168` (`build_registry`) — the only production
  tool-registration site; new tools must be added to the `tools: vec![…]`.
- `executor/src/tools/router.rs` (`categorize`) — maps tool names to
  router categories; new tools must be added here.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo build` to confirm the tree is clean before starting.

## Current state

### File to touch — `executor/src/tools/write_file.rs`

Current `WriteFileArgs` (`write_file.rs:16-19`):
```rust
#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}
```

Current success output (`write_file.rs:112-117`):
```rust
Ok(ToolResult {
    output: format!("wrote {bytes} bytes to {}", abs_path.to_string_lossy()),
    error: None,
    metadata: Some(metadata),
})
```

### File to touch — `executor/src/tools/search.rs`

Current `SearchArgs` (`search.rs:21-26`):
```rust
#[derive(Deserialize)]
struct SearchArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
    case_insensitive: Option<bool>,
}
```

Current `MatchHit` (`search.rs:28-33`):
```rust
struct MatchHit {
    path: String,
    line: usize,
    col: usize,
    line_content: String,
}
```

Current inner loop (`search.rs:173-186`):
```rust
for (line_idx, line) in content.lines().enumerate() {
    for m in re.find_iter(line) {
        hits.push(MatchHit {
            path: rel_path.clone(),
            line: line_idx + 1,
            col: m.start() + 1,
            line_content: line.to_string(),
        });
        if hits.len() >= max_results {
            let truncated = hits.len() == max_results;
            return Ok(format_output(&hits, &abs_root, &parsed.pattern, truncated));
        }
    }
}
```

The same shape runs in `execute_single_file` (`search.rs:225-236`). Both must
be updated identically.

### File to touch — `executor/src/tools/find_files.rs`

Current `FindFilesArgs` (`find_files.rs:19-23`):
```rust
#[derive(Deserialize)]
struct FindFilesArgs {
    pattern: String,
    path: Option<String>,
    max_results: Option<usize>,
}
```

Current `Walk` construction (`find_files.rs:140`):
```rust
for entry in Walk::new(&abs_root) {
```
`Walk::new` has no depth control. Replace with `WalkBuilder`. The `ignore`
crate already in the workspace (`executor/Cargo.toml`: `ignore.workspace = true`)
exposes `WalkBuilder`:
```rust
use ignore::WalkBuilder;
// ...
for entry in WalkBuilder::new(&abs_root)
    .max_depth(parsed.depth)  // None means unbounded — same as today
    .build()
{
```
`WalkBuilder::max_depth` takes `Option<usize>` directly; no conditionals needed.
You can drop `use ignore::Walk;` once `Walk::new` is replaced.

### Tool registration pattern (for the three new tools)

**`executor/src/tools/mod.rs`** — current bottom (`mod.rs:14-24`):
```rust
mod update_task;

pub use bash::{Bash, bash, bash_with_filter, is_allowed_env_key};
pub use find_files::{FindFiles, find_files};
pub use patch::{Patch, patch};
pub use read_file::{ReadFile, read_file};
pub use registry::{Tool, ToolRegistry, ToolResult};
pub use router::{Category, categorize};
pub use search::{Search, search};
pub use symbols::{Symbols, symbols};
pub use update_task::{UpdateTask, update_task};
pub use write_file::{WriteFile, write_file};
```
Add three `mod` declarations (alphabetical in the mod block) and three `pub use`
lines following the same pattern.

**`mcp/src/runner.rs:142-150`** — the production tool vec (read the full
function at `runner.rs:134-168`; add the three new tools here):
```rust
let mut tools: Vec<Arc<dyn tools::Tool>> = vec![
    tools::read_file(scope.clone()),
    tools::write_file(scope.clone()),
    tools::patch(scope.clone()),
    tools::find_files(scope.clone()),
    tools::search(scope.clone()),
    tools::symbols(scope.clone()),
    tools::bash_with_filter(scope.clone(), bash_timeout_secs, filter_output),
    // add delete_file, move_file, patch_lines here
];
```

**`executor/src/tools/router.rs`** — `categorize` function (`router.rs:14-23`):
```rust
pub fn categorize(tool_name: &str) -> Option<Category> {
    Some(match tool_name {
        "read_file" | "symbols" => Category::Read,
        "write_file" | "patch" => Category::Write,
        "search" | "find_files" => Category::Search,
        "bash" => Category::Run,
        "update_task" => Category::Meta,
        _ => return None,
    })
}
```
Add `"patch_lines" | "delete_file" | "move_file"` to the `Category::Write` arm.
Also add all three names to the `built_ins` array in the
`all_built_in_tools_categorize_to_some` test (`router.rs:76-85`).

## Spec

### 1. `write_file` — append mode + line count

Add `append: Option<bool>` to `WriteFileArgs` (default false when absent).
When `append == Some(true)`, open with `std::fs::OpenOptions` in append mode
instead of `std::fs::write`:
```rust
use std::io::Write as _;
// ...
if parsed.append.unwrap_or(false) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| /* return ToolResult error */)?;
    f.write_all(parsed.content.as_bytes())
        .map_err(|e| /* return ToolResult error */)?;
} else {
    std::fs::write(&path, &parsed.content)
        .map_err(|e| /* return ToolResult error */)?;
}
```
Do NOT use `?` here — errors must return `Ok(ToolResult { error: Some(…) })`,
matching the existing write-failure arm at `write_file.rs:94-100`. Use
`match … { Ok(()) => {}, Err(e) => return Ok(ToolResult { error: … }) }`.

Update the success output to include the line count:
```rust
let lines = parsed.content.lines().count();
// output:
format!("wrote {lines} lines ({bytes} bytes) to {}", abs_path.to_string_lossy())
```
Add `lines_written` to the metadata JSON alongside `bytes_written`.

Update the description: `"Create, overwrite, or append a file. Use append: true to add to an existing file. Prefer patch for edits."`.

Update the schema to document the `append` field as an optional boolean.

The parent-directory-must-exist guard (`write_file.rs:75-87`) must apply in
both modes. Scope confinement (`scope.resolve`) must apply in both modes.

### 2. `search` — `context_lines`

Add `context_lines: Option<usize>` to `SearchArgs`. Cap internally at 5
(`let ctx = parsed.context_lines.unwrap_or(0).min(5)`).

Add `context_before` and `context_after` to `MatchHit`:
```rust
struct MatchHit {
    path: String,
    line: usize,
    col: usize,
    line_content: String,
    context_before: Vec<(usize, String)>,  // (1-indexed line number, content)
    context_after: Vec<(usize, String)>,
}
```

When `ctx == 0`, both vecs are empty — the hot path is unchanged. When `ctx > 0`,
collect all file lines first, then slice:
```rust
let all_lines: Vec<&str> = content.lines().collect();
// ... inside the outer loop, replace the existing for (line_idx, line) loop:
for (line_idx, line) in all_lines.iter().enumerate() {
    for m in re.find_iter(line) {
        let before_start = line_idx.saturating_sub(ctx);
        let context_before = all_lines[before_start..line_idx]
            .iter()
            .enumerate()
            .map(|(i, l)| (before_start + i + 1, l.to_string()))
            .collect();
        let after_end = (line_idx + 1 + ctx).min(all_lines.len());
        let context_after = all_lines[line_idx + 1..after_end]
            .iter()
            .enumerate()
            .map(|(i, l)| (line_idx + 2 + i, l.to_string()))
            .collect();
        hits.push(MatchHit {
            path: rel_path.clone(),
            line: line_idx + 1,
            col: m.start() + 1,
            line_content: line.to_string(),
            context_before,
            context_after,
        });
        if hits.len() >= max_results { … }
    }
}
```
Apply the same change to `execute_single_file` (pass `ctx` as a parameter or
re-derive it; the function must accept it).

Update `format_output` to render context when present. Output format:
```
path/to/file.rs:
   40  line before
   41  line before
>  42:5  matching line content
   43  line after
---
```
The `>` prefix marks the match line. Context lines use `"  {line:>4}  {content}\n"`;
match lines use `"> {line:>4}:{col}  {content}\n"`. A `---` separator appears
between hits from the **same file** when their context windows do not overlap
(i.e. the after-context of one hit ends before the before-context of the next
begins). When `ctx == 0`, the format is byte-identical to today (`"  {}:{}  {}\n"`
per hit). **Pin this: the no-context path must produce the same output as before.**

### 3. `find_files` — `depth` parameter

Add `depth: Option<usize>` to `FindFilesArgs`. Switch from `Walk::new` to
`WalkBuilder`:
```rust
use ignore::WalkBuilder;

for entry in WalkBuilder::new(&abs_root)
    .max_depth(parsed.depth)
    .build()
{
```
`None` → unbounded (same as today). `Some(1)` → immediate children only (the
directory itself is depth 0, so depth 1 = its direct children). Update the
schema to document `depth`.

### 4. New tool — `patch_lines`

New file `executor/src/tools/patch_lines.rs`. Tool name `"patch_lines"`.
Category `Write`.

Schema: `{path: String, start_line: usize, end_line: usize, new_content: String}`.
All four fields required.

Description: `"Replace a line range in a file. start_line and end_line are 1-indexed and inclusive. Use for large block replacements where patch old_str matching is fragile."`.

Implementation:
1. Scope.resolve(path).
2. `read_to_string` → `content`.
3. `let lines: Vec<&str> = content.lines().collect();`
4. Validate: `start_line >= 1`, `end_line >= start_line`,
   `end_line <= lines.len()` — return advisory errors for each violation.
5. Build replacement:
   ```rust
   let mut result_lines: Vec<&str> = lines[..start_line - 1].to_vec();
   let new_lines: Vec<&str> = new_content.lines().collect();
   result_lines.extend(new_lines.iter().copied());
   result_lines.extend_from_slice(&lines[end_line..]);
   let new_content_str = result_lines.join("\n");
   // Preserve trailing newline if the original had one:
   let new_content_str = if content.ends_with('\n') {
       format!("{new_content_str}\n")
   } else {
       new_content_str
   };
   ```
6. `std::fs::write(&path, &new_content_str)`.
7. Return `"patched {path}:{start_line}-{end_line} ({n_old} lines → {n_new} lines)"`.
   Metadata: `{path, start_line, end_line, lines_removed, lines_added}`.

Gotcha: `new_content` may be empty (deletion — all lines in the range removed).
This is valid. Do **not** error on empty `new_content`.

### 5. New tool — `delete_file`

New file `executor/src/tools/delete_file.rs`. Tool name `"delete_file"`.
Category `Write`.

Schema: `{path: String}` (required).

Description: `"Delete a file. Directories are not permitted; use bash for recursive removal."`.

Implementation:
1. Scope.resolve(path).
2. If `path.is_dir()`: return advisory error `"path is a directory: {path} — use bash for directory removal"`.
3. If `!path.exists()`: return advisory error `"file not found: {path}"`.
4. `std::fs::remove_file(&path)`.
5. Return `"deleted {abs_path}"`.

### 6. New tool — `move_file`

New file `executor/src/tools/move_file.rs`. Tool name `"move_file"`.
Category `Write`.

Schema: `{from: String, to: String}` (both required).

Description: `"Move or rename a file within the project root. Both paths are confined to the project root."`.

Implementation:
1. Scope.resolve(from) and Scope.resolve(to) — **both** must be within scope.
2. If `from` does not exist: advisory error.
3. If `to` already exists: advisory error `"destination already exists: {to} — delete it first if you want to overwrite"`.
4. If to's parent directory does not exist: advisory error (same guard as `write_file`).
5. `std::fs::rename(&from_path, &to_path)`.
6. Return `"moved {from} → {to}"`.

## Acceptance criteria

- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      and `cargo fmt --all --check` are clean.
- [ ] `cargo test` passes (existing + new).
- [ ] `write_file` with `append: true` appends to an existing file; `append: false`
      (or absent) overwrites — byte-identical to today.
- [ ] `write_file` success output includes line count: `"wrote N lines (B bytes) to …"`.
- [ ] `search` with `context_lines: 0` (or absent) produces byte-identical output
      to today.
- [ ] `search` with `context_lines: 2` emits 2 before/after lines per hit with the
      `>` match prefix and `---` separators between non-adjacent hits.
- [ ] `find_files` with `depth: 1` returns only immediate children of the search root.
- [ ] `patch_lines` replaces a line range; empty `new_content` deletes the range;
      trailing newline is preserved.
- [ ] `delete_file` removes a file; errors on directory; errors on missing file.
- [ ] `move_file` renames a file within scope; errors when destination exists; errors
      when either path escapes scope.
- [ ] All three new tools appear in `categorize()` as `Category::Write` and in
      `build_registry`'s tool vec.
- [ ] The `all_built_in_tools_categorize_to_some` test array includes all three
      new tool names.

## Test plan

Hermetic tests (`TempDir`, `Scope`, `tokio::test`) in each file's
`#[cfg(test)] mod tests` block. Follow the pattern in `write_file.rs:125-230`
(see the existing tests for the `make_scope`/`write_file` call shape — copy it
for each new tool's tests).

**`write_file.rs` new tests:**
- `appends_to_existing_file` — write "line 1\n", then append "line 2\n";
  assert full content is "line 1\nline 2\n".
- `append_creates_file_if_missing` — append to a nonexistent path; file is
  created. (Mutation-resistant: if `create(true)` is missing, this fails.)
- `append_false_overwrites` — ensure existing `append: false` behaviour unchanged.
- `success_output_includes_line_count` — `assert!(output.contains("2 lines"))` for
  a 2-line write.

**`search.rs` new tests:**
- `context_lines_zero_output_matches_no_context` — same file, same pattern;
  `context_lines: 0` and absent both produce the same output. (Pinned identity.)
- `context_lines_emits_before_and_after` — 5-line file, match on line 3,
  `context_lines: 2`; assert lines 1-2 in output as context and lines 4-5 in
  output as context, with `>` on line 3.
- `context_lines_capped_at_five` — `context_lines: 99`; a 3-line file with a
  match on line 2 only shows 1 before and 1 after (bounded by file edges, not
  the cap), confirming no panic.
- `context_lines_separator_between_nonadjacent_hits` — two matches far apart;
  `---` separator appears between them. (Pinned negative: no separator when
  context windows touch.)

**`find_files.rs` new tests:**
- `depth_one_returns_only_immediate_children` — create `a.rs` and `sub/b.rs`;
  `depth: 1` with `**/*.rs` returns only `a.rs`. (Pinned negative: `sub/b.rs`
  absent.)
- `depth_none_returns_nested_files` — same fixture; no `depth` returns both.

**`patch_lines.rs` tests:**
- `replaces_middle_lines` — 5-line file, replace lines 2-3; check result.
- `empty_new_content_deletes_range` — replace lines 2-3 with `""`; lines 2-3
  gone, lines 1,4,5 remain.
- `preserves_trailing_newline` — file ending with `\n`; result also ends with `\n`.
- `rejects_out_of_range` — `start_line: 10` on a 5-line file → advisory error.
- `rejects_end_before_start` — `start_line: 3, end_line: 2` → advisory error.

**`delete_file.rs` tests:**
- `deletes_existing_file` — file gone after call.
- `errors_on_directory` — pass a dir path; error contains "directory".
- `errors_on_missing_file` — advisory error on nonexistent path.
- `rejects_path_outside_scope` — error contains "escapes".

**`move_file.rs` tests:**
- `moves_file_to_new_name` — file at new path, old path gone.
- `errors_when_destination_exists` — advisory error.
- `errors_when_from_missing` — advisory error.
- `rejects_from_outside_scope` — error contains "escapes".
- `rejects_to_outside_scope` — error contains "escapes".

## End-to-end verification

Run against the real binary (`rexymcp`) for a quick sanity check (no test
harness replacement — the hermetic unit tests are the gate):

```
cd /home/matt/src/rexyMCP
echo 'fn main() {}' > /tmp/e2e_test.rs
cargo run -p rexymcp -- health --config rexymcp.toml
```

This phase has no CLI surface — the E2E is the build passing and tests green.
State this in the Update Log; do not fabricate a CLI transcript.

## Authorizations

- Modify `executor/src/tools/write_file.rs`, `search.rs`, `find_files.rs`,
  `router.rs`, `mod.rs`.
- New files: `executor/src/tools/patch_lines.rs`, `delete_file.rs`,
  `move_file.rs`.
- Modify `mcp/src/runner.rs` (add three tool registrations to `build_registry`).
- No new `Cargo.toml` dependency. No edits to `architecture.md`, `STANDARDS.md`,
  `WORKFLOW.md`, or any other phase doc.

## Out of scope

- Modifying `patch.rs` — it already has fuzzy mismatch hints and unified-diff
  success output; leave it unchanged.
- `read_file` offset/limit or any other `read_file` change.
- `bash` changes.
- `symbols` changes.
- Any MCP-server or CLI surface change.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-15 18:07 (started)

**Executor:** Claude (direct)

Implementing all 7 spec tasks: write_file append+line-count, search context_lines, find_files depth, patch_lines, delete_file, move_file, plus router/registry wiring.

### Review bounce — 2026-06-15 (reviewer: Claude Code architect)

Bounced. Executor returned `complete` (53 turns, after a 1st-dispatch infra blip:
SSE stream stalled 240s) but the phase is ~half done and three DoD gates fail. See
[bug-07-1](bugs/bug-07-1.md) (blocker). Summary of what's wrong:

- **F1 (blocker):** `move_file` (task 6) never created; `mod.rs` / `router.rs` /
  `runner.rs` all **unmodified**, so `delete_file.rs` + `patch_lines.rs` are
  undeclared orphan files that never compiled — their tests never ran. The
  "766 passed" excluded all new-tool tests. Nothing is registered or routed.
- **F2 (major, false_completion):** `cargo fmt --all --check` exits 1
  (`find_files.rs`/`search.rs`/`write_file.rs`).
- **F3 (major, prod_unwrap):** `search.rs:337,357` use `.unwrap()` in
  `format_output` (production). 3rd occurrence of the M12 watch-item.

**Notes for executor (read on re-dispatch):** tasks 1–3 (`write_file` append,
`search` context_lines, `find_files` depth) and the bodies of `delete_file.rs` /
`patch_lines.rs` are already on disk and look correct — do **not** redo them. The
remaining work is exactly bug-07-1 § "How to fix": (1) create `move_file.rs`;
(2) wire all three new tools into `mod.rs` + `router.rs` `categorize`/test-array +
`runner.rs` `build_registry`; (3) replace the two `search.rs` `unwrap`s with
`map_or`; (4) `rustfmt` the touched files (NOT `cargo fmt --all`); (5) re-run all
four gates separately and confirm the test count rises above 766 (proof the new
tools compiled in). The wiring step (mod.rs/router.rs/runner.rs) is the one that
was skipped entirely — it is what makes the new tools real.

### Update — 2026-06-15 21:11 (complete)

**Summary:** Fixed all three bugs from bug-07-1 review bounce: (1) created
`move_file.rs` per spec; (2) wired all three new tools into `mod.rs` (mod + pub
use), `router.rs` (categorize + test array), and `runner.rs` (build_registry +
test counts); (3) replaced two `.unwrap()` calls in `search.rs` `format_output`
with `.map_or()`; (4) fixed clippy `if_same_then_else` in `patch_lines.rs`
trailing-newline logic; (5) removed unused imports from test modules. All four
gates pass: 780 tests (up from 766 — 14 new tests from the three new tools now
compile in).

**Acceptance criteria:** all ticked above.

**Commands:**

```
$ cargo fmt --all --check
(exit 0)

$ cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.17s

$ cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.30s

$ cargo test 2>&1 | tail -30
test result: ok. 780 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The E2E is the build
passing and tests green (780 passed, up from 766).

**Files changed:**
- `executor/src/tools/move_file.rs` — new file: move/rename tool with scope confinement, 5 tests
- `executor/src/tools/mod.rs` — added `mod` + `pub use` for delete_file, move_file, patch_lines
- `executor/src/tools/router.rs` — added three new tools to `categorize()` Write arm + test array
- `mcp/src/runner.rs` — added three new tools to `build_registry` vec + updated test counts (7→10, 8→11)
- `executor/src/tools/search.rs` — replaced two `.unwrap()` with `.map_or()` in `format_output`
- `executor/src/tools/patch_lines.rs` — fixed clippy `if_same_then_else` on trailing-newline logic, removed unused import
- `executor/src/tools/delete_file.rs` — removed unused import
- `docs/dev/milestones/M18-capability-adaptation/phase-07-tooling-improvements.md` — status → review, completion entry
- `docs/dev/milestones/M18-capability-adaptation/README.md` — phase table status → review

**New tests:**
- `moves_file_to_new_name` in `move_file.rs`
- `errors_when_destination_exists` in `move_file.rs`
- `errors_when_from_missing` in `move_file.rs`
- `rejects_from_outside_scope` in `move_file.rs`
- `rejects_to_outside_scope` in `move_file.rs`

**Commits:**
- pending (one commit for all changes)

**Notes for review:** Test count rose from 766 to 780 (14 new tests: 5 move_file, 5 patch_lines, 4 delete_file — the delete_file/patch_lines tests were previously dead code). The `runner.rs` test assertions were updated from 7→10 base tools and 8→11 with update_task to reflect the three new tools. No `.unwrap()` remains in any production path of the touched files.
