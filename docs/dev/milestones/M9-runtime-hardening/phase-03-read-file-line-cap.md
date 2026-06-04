# Phase 03: read_file output cap

**Milestone:** M9 — Executor runtime hardening
**Status:** done
**Depends on:** M2 (the `read_file` tool exists at `executor/src/tools/read_file.rs`).
**Estimated diff:** ~80 lines (cap logic + notice + updated description + tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Prevent `RunawayOutput` hard-fails caused by whole-file reads of large files. When
`read_file` would return more than `MAX_OUTPUT_LINES` (500) lines, it truncates the
output and appends a notice telling the model the file's total line count and how to
re-read a specific range with `start_line`/`end_line`. This closes the second half
of the M9 recurring-bounce root cause: phase-01 and phase-02 each had a dispatch
hard-fail on the first turn because the executor read `executor/src/agent/mod.rs`
(~3 000–4 000 lines) whole and the output exceeded the 100 KB `RunawayOutput`
threshold.

## Architecture references

Read before starting:

- `executor/src/governor/hard_fail.rs` — `RUNAWAY_OUTPUT_BYTES = 100 * 1024`. The
  hard-fail detector checks `content.len()` against this after every tool dispatch.
  A `read_file` returning more than ~100 KB trips `RunawayOutput` before any editing
  can occur.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.
4. Read `executor/src/tools/read_file.rs` in full — it is ~200 lines, safe to read
   whole. Understand the existing `start_line`/`end_line` slice logic and the
   `ToolResult { output, error, metadata }` shape before editing.

## Current state

`executor/src/tools/read_file.rs` already supports `start_line`/`end_line` — the
args struct, schema, and slice logic are all there. When neither is supplied,
`end = total_lines`, so the full file is returned as `output`. A 4 000-line
file at ~50 chars/line returns ~200 KB, doubling the `RUNAWAY_OUTPUT_BYTES`
threshold.

The `start_line`/`end_line` params in the schema have descriptions but the tool
description says only "Read a file's contents, optionally a line range." — nothing
warns that large files are capped or that ranges are necessary for large files.

There are no existing tests for truncation (since it doesn't exist yet). The 9
existing tests cover: whole-file read, not-found, directory, line-range, clamped
end, start>end, non-UTF-8, malformed args, path-outside-root, no-token-count.
None write a file large enough to trigger a cap. **These tests must all keep
passing.**

## Spec

All edits are in `executor/src/tools/read_file.rs`. Do **not** modify any other
file.

### 1. Add the line-cap constant

Near the top of the file, add:

```rust
/// Maximum lines returned in a single read_file call. Callers needing more must
/// use start_line/end_line to read in slices. Keeps output well below the
/// governor's RUNAWAY_OUTPUT_BYTES (100 KB) threshold for typical source files.
const MAX_OUTPUT_LINES: usize = 500;
```

### 2. Apply the cap and emit a truncation notice

After computing `end_clamped` and `slice`, if `slice.len() > MAX_OUTPUT_LINES`:

1. Trim `slice` to `slice[..MAX_OUTPUT_LINES]`.
2. Compute `shown_end = start + MAX_OUTPUT_LINES - 1` (the last line number shown,
   1-indexed).
3. Append a truncation notice to the output string **as a trailing newline + comment
   line** that the model can read but that does not look like file content:

   ```
   \n[read_file: truncated — file has {total_lines} lines; showing lines {start}–{shown_end}. Re-read with start_line/end_line for other ranges.]
   ```

4. Set `lines_read` in the metadata to `MAX_OUTPUT_LINES` (the actual lines
   returned, not the full slice before trimming).
5. Add a `"truncated": true` key to the metadata JSON so the notice is
   machine-readable (used by future tooling; benign today).

**When the cap does NOT apply** (output ≤ 500 lines): behavior is identical to
today. No notice, no `"truncated"` key, `lines_read` == actual slice length.

### 3. Update the tool description

Change `description()` to:

```rust
"Read a file's contents. Returns at most 500 lines; use start_line/end_line to \
 read specific ranges of larger files."
```

This ensures the executor contract (which embeds the tool description in the model's
system prompt) advertises the cap and the range API.

### 4. Error model

No change to the error path. Truncation is a **normal, successful** `ToolResult`
with `error: None`. The cap is not an error — it is a deliberate output bound.

## Acceptance criteria

- [ ] A whole-file read of a 600-line file returns exactly 500 lines plus the
      truncation notice; `metadata["lines_read"] == 500` and
      `metadata["truncated"] == true`.
- [ ] The truncation notice contains the file's **total line count** (600) and the
      range shown (`1–500`) (**positive**).
- [ ] A whole-file read of a 499-line file returns all 499 lines with **no
      truncation notice** and **no** `"truncated"` key in metadata (**negative**).
- [ ] An explicit `start_line=501, end_line=600` read of the 600-line file returns
      exactly 100 lines (no cap triggered, since 100 < 500) (**positive**).
- [ ] An explicit `start_line=1, end_line=600` read of the 600-line file returns 500
      lines + notice (explicit range larger than cap is still capped) (**positive**).
- [ ] The notice for the explicit `start_line=1, end_line=600` case shows
      `1–500` (not `1–600`) as the range shown.
- [ ] `tool.description()` contains the string `"500 lines"`.
- [ ] All 9 existing tests continue to pass unchanged.
- [ ] No new dependency; no other file modified; no `tracing`; no `unsafe`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

All tests in `executor/src/tools/read_file.rs`'s `#[cfg(test)]` module. Use
`tempfile::TempDir` + `std::fs::write`. A 600-line file can be created with
`(1..=600).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n")`.

- `truncates_whole_file_read_at_max_lines` — 600-line file, no range args; assert
  `output` contains `"line1"`, does not contain `"line501"`, contains the truncation
  notice with `"600"` and `"500"`; `metadata["lines_read"] == 500`;
  `metadata["truncated"] == true`.
- `truncation_notice_shows_correct_range` — same 600-line file, no range;
  assert notice contains `"1–500"` or `"1-500"` (the range shown).
- `small_file_not_truncated` (negative) — 499-line file, no range; assert no
  `"truncated"` key, `metadata["lines_read"] == 499`, output does not contain
  `"[read_file:"`.
- `explicit_range_within_cap_not_truncated` (negative) — 600-line file,
  `start_line=501, end_line=600`; assert 100 lines returned, no notice.
- `explicit_range_exceeding_cap_is_capped` — 600-line file,
  `start_line=1, end_line=600`; assert 500 lines returned + notice showing `"1–500"`
  (or `"1-500"`).
- `description_mentions_line_cap` — `tool.description()` contains `"500 lines"`.

## End-to-end verification

The phase ships a behavior change to a real tool, not just an agent-loop seam.
Drive it with the **production** `ReadFile` struct (not a mock): construct
`read_file(Scope::new(dir.path()).unwrap())`, write a 600-line file, call
`tool.execute(...)`, and assert the truncation. This is the existing test pattern
in the file — all tests already do this. The `truncates_whole_file_read_at_max_lines`
test IS the E2E verification. Paste its output in the completion Update Log.

## Authorizations

- [x] **May modify** `executor/src/tools/read_file.rs` only (source + tests).
- [ ] **No new dependencies.**
- [ ] May **NOT** modify `executor/src/governor/hard_fail.rs` (the threshold stays
      100 KB — the fix is in `read_file`, not the detector), any other tool, any
      agent-loop file, `Cargo.toml`, `docs/architecture.md`, `STANDARDS.md`, or
      `WORKFLOW.md`.

## Out of scope

- **Capping reads by byte count** rather than line count. Line count is
  sufficient for typical source files (~50 chars/line × 500 = ~25 KB).
  Pathologically long lines (>200 chars × 500 = >100 KB) remain possible —
  a future phase can add a secondary byte cap if this proves real in practice.
- **Changing the `RunawayOutput` threshold** in `hard_fail.rs`.
- **Any change to `patch` or other tools.**
- **Surfacing the cap in the executor contract template** — the tool's
  `description()` is embedded in the system prompt via the tool schema; no
  template change needed.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-04 (started)

**Executor:** rexyMCP executor

Implementing read_file output cap per spec: added `MAX_OUTPUT_LINES` constant, truncation logic with notice, updated tool description, and 6 new tests.

### Update — 2026-06-04 (complete)

**Executor:** rexyMCP executor

**Spec tasks completed:**
1. Added `MAX_OUTPUT_LINES = 500` constant with doc comment.
2. Implemented truncation logic: slices > 500 lines are trimmed, a notice appended, `lines_read` set to 500, and `"truncated": true` added to metadata.
3. Updated `description()` to mention the 500-line cap.
4. No error model changes — truncation is a successful `ToolResult`.

**Acceptance criteria verification:**
- [x] 600-line whole-file read returns 500 lines + notice; `lines_read == 500`, `truncated == true`
- [x] Truncation notice contains total line count (600) and range shown (1–500)
- [x] 499-line file returns all lines, no notice, no `truncated` key
- [x] Explicit `start_line=501, end_line=600` returns 100 lines, no cap
- [x] Explicit `start_line=1, end_line=600` returns 500 lines + notice
- [x] Notice for explicit range shows `1–500` (not `1–600`)
- [x] `tool.description()` contains `"500 lines"`
- [x] All 9 existing tests pass unchanged
- [x] No new dependency; no other file modified; no `tracing`; no `unsafe`
- [x] All four required commands pass with zero new warnings

**Required commands output:**
- `cargo fmt --all`: clean
- `cargo build`: clean, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: clean
- `cargo test`: 585 passed, 0 failed, 2 ignored

**End-to-end verification:** The `truncates_whole_file_read_at_max_lines` test drives the production `ReadFile` struct against a 600-line file in a `TempDir`, asserts truncation at 500 lines, presence of the notice with correct total/range, and correct metadata (`lines_read == 500`, `truncated == true`). Test output: `test tools::read_file::tests::truncates_whole_file_read_at_max_lines ... ok`.

**Files changed:**
- `executor/src/tools/read_file.rs` — source + tests

**Grep verification of truncation literal:**
```
$ grep -c 'read_file: truncated' executor/src/tools/read_file.rs
3
```
(1 in source format string, 2 in test assertions — literal landed correctly.)

**Notes for review:** None. Implementation matches spec exactly.

### Review verdict — 2026-06-04

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none. Single-file change (`read_file.rs`); all 3 spec
  tasks implemented exactly (constant, cap+notice, description). No other file
  touched. Executor committed cleanly as `feat:` (`f76f44d`) with a correct body.
  Independently re-ran all four gates: fmt/build/clippy clean, 585 tests pass (6
  new + all 9 original read_file tests intact). Spot-checked
  `truncates_whole_file_read_at_max_lines` and `small_file_not_truncated` — both
  discriminating (the positive asserts truncation at 500 and `truncated: true`; the
  negative asserts no `[read_file:` notice and no `truncated` key for a 499-line
  file). The `unwrap_or_else` in the production path is pre-existing
  (`canonicalize().unwrap_or_else(|_| path.clone())`) — not new.
- **Calibration:** none new. Phase-03 approved first try with a clean single-file
  spec — the split from the hook phases paid off. M9 is now fully done.
