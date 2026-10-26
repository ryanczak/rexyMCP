# Phase 01: Bash tree changes count as progress

**Milestone:** M50 — Tree-based progress
**Status:** todo
**Depends on:** none
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The governors decide "the model made file progress" from the tool name alone,
so a `bash` call that rewrites files never resets the no-progress stall run.
Runs editing through bash are hard-failed as `NoProgressStall` while their
edits are on disk. Make a `bash` call that changed the working tree count as
progress.

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` § Notes — the
  user's design decisions; this phase implements 1 and 2.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`check_read_only_stall` and `window_has_mutation` in
`executor/src/governor/hard_fail.rs` test each `ToolCallSnapshot` with
`tools::mutates_files` (`executor/src/tools/router.rs`), which is false for
`bash`. The agent loop (`executor/src/agent/mod.rs`) pushes one snapshot per
tool call and knows nothing about what a `bash` call did to disk.

## Spec

1. **Tree fingerprint** — new module `executor/src/agent/tree_fingerprint.rs`.
   A fingerprint of the project root records every non-ignored file's
   relative path, size and mtime. Comparing two fingerprints yields the
   relative paths added, removed or changed, sorted (phase 02 consumes this
   list, so return the paths, not just a bool). Best-effort: an entry that
   cannot be read is skipped; the walk never returns an error.
   Walk rules, each one a gotcha:
   - `.gitignore` must be honoured even when the root is **not** a git repo
     (tests run in a bare `TempDir`). `ignore`'s `WalkBuilder` honours
     gitignore files only inside a git repo unless told otherwise.
   - Hidden files count (an edit to `.github/workflows/ci.yml` is progress),
     but `WalkBuilder` skips hidden entries by default.
   - The `.git/` directory never counts. Pruning it during the walk, rather
     than filtering its files afterwards, keeps the walk cheap.

   `ignore` 0.4.26 (`Cargo.lock`), the builder methods involved:
   `WalkBuilder::new(root)`, `.hidden(bool)` ("enables ignoring hidden
   files", default `true`), `.require_git(bool)` ("whether a git repository
   is required to apply git-related ignore rules", default `true`),
   `.filter_entry(|e: &DirEntry| -> bool)` (an entry for which the predicate
   returns `false` is skipped, and a directory is not descended into).
2. **Snapshot carries the tree signal** — `ToolCallSnapshot` gains a
   `changed_tree: bool`, `#[serde(default)]` so a snapshot serialized before
   this phase still deserializes. Add a method on `ToolCallSnapshot` that is
   true when `mutates_files(tool)` or `changed_tree`. That method is now the
   one definition of "made file progress"; `check_read_only_stall` and
   `window_has_mutation` both use it. Update their doc comments, and the doc
   comment on `mutates_files`, which currently calls itself the single
   source of truth for file progress.
3. **The loop sets it** — in `executor/src/agent/mod.rs`, take a fingerprint
   immediately before a `bash` call is dispatched and another after it
   returns; `changed_tree` is true when they differ. Fingerprint only around
   `bash` calls that actually dispatch: not around other tools, and not
   around a `bash` call that was refused or deduplicated before dispatch
   (it ran nothing). Every other snapshot gets `false`.
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`. The
   server-authored `(complete)` entry does not satisfy this.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] `check_read_only_stall` and `window_has_mutation` contain no direct
      call to `mutates_files`; both go through the snapshot method.
- [ ] Preservation: every existing test in `executor/src/governor/` and
      `executor/src/agent/tests.rs` passes unmodified apart from the new
      field in snapshot literals.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

All hermetic: `TempDir` roots, no git binary, no network.

- `tree_fingerprint.rs`:
  - `detects_written_file` — a new file, and an existing file rewritten to a
    different size, each appear in the changed paths.
  - `detects_deleted_file` — a removed file appears in the changed paths.
  - `unchanged_tree_compares_equal` — two walks with no write between them
    yield no changed paths.
  - `gitignored_write_is_not_a_change` — with `target/` in a `.gitignore`
    and no `.git` directory, writing under `target/` yields no changed paths.
  - `dot_git_write_is_not_a_change` — writing under `.git/` yields no
    changed paths.
  - `hidden_file_write_is_a_change` — writing `.github/ci.yml` does.
- `hard_fail.rs`:
  - `read_only_stall_resets_on_tree_changing_bash` — a run of read-only
    calls interrupted by a `bash` snapshot with `changed_tree: true` counts
    only the calls after it.
  - `read_only_stall_counts_bash_without_tree_change` — the same run with
    `changed_tree: false` fires at threshold, as today.
  - `oscillation_applies_to_tree_changing_bash_window` — an A,B,A,B window
    of `bash` calls where at least one changed the tree is not exempt and
    fires; the same window with no tree change stays exempt.
  - `tool_call_snapshot_deserializes_without_changed_tree` — JSON with no
    `changed_tree` key deserializes with `false`.
- `agent/tests.rs` (driven by `MockAiClient`, real `bash` tool in a temp
  root):
  - `bash_edit_resets_read_only_stall` — a scripted run whose read-only call
    count would cross a small `read_only_stall_threshold`, except that a
    `bash` call writing a file sits in the middle, does not end in
    `NoProgressStall`.
  - `read_only_bash_still_trips_stall` — the same script with the middle
    `bash` call writing nothing ends in `NoProgressStall`.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-01.txt; : > "$A"
for t in tree_fingerprint read_only_stall_resets_on_tree_changing_bash read_only_stall_counts_bash_without_tree_change oscillation_applies_to_tree_changing_bash_window tool_call_snapshot_deserializes_without_changed_tree bash_edit_resets_read_only_stall read_only_bash_still_trips_stall; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count (a filter that matched no
test prints none).

## Authorizations

Files that hold a `ToolCallSnapshot { … }` literal or are otherwise touched:

- `executor/src/agent/tree_fingerprint.rs` (new), `executor/src/agent/mod.rs`,
  `executor/src/agent/tests.rs`
- `executor/src/governor/hard_fail.rs`
- `executor/src/phase/briefing.rs` (test literals only)
- `mcp/src/calibrate_governor.rs` (snapshot literals only: the replay sets
  `changed_tree: false`, see § Out of scope)
- `executor/src/tools/router.rs` (the `mutates_files` doc comment only)

No new dependencies (`ignore` is already one).

## Out of scope

- The `PhaseResult` diff, `files_changed` and the progress numstat — phase 02.
- The bash classifier. Bash-side writes stay allowed; do not extend
  `RefuseInPlaceEdit`.
- Fingerprinting around any tool other than `bash`.
- Governor thresholds and config.
- Recording `changed_tree` in the session log. `rexymcp calibrate-governor`
  replays snapshots from logs, so its replays stay tool-name-only for now;
  the replay passes `false`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
