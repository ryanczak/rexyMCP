# Phase 03: Every edit tool in the reported diff

**Milestone:** M50 — Tree-based progress
**Status:** in-progress
**Depends on:** phase-02
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

The phase diff captures a file's "before" content only for `write_file` and
`patch`. Edits through `patch_lines`, `delete_file` and `move_file` never
reach it, so they are missing from `PhaseResult.diff` / `files_changed` and
from the progress numstat. Capture them too.

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` — the milestone's
  goal: every way a file changes shows up as progress and in the diff.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc.
3. Confirm a clean tree.

## Current state

In the agent loop (`executor/src/agent/mod.rs`), `edit_target`
(`agent/tools.rs`) names the file an edit call targets, and the loop uses
that one path for several things: the verifier baseline, the pre-edit
capture into `pre_edit_content`, superseded-read eviction, the post-write
hooks and the post-edit verify. `edit_target` covers `write_file` and
`patch` only. `build_diff` (`agent/outcome.rs`) and `numstat_from_pre_edit`
(`agent/progress.rs`) diff whatever is in `pre_edit_content` against disk,
treating an unreadable "after" as empty.

## Spec

1. **Diff-capture targets** — a new helper beside `edit_target` in
   `agent/tools.rs` that returns every path whose pre-edit content the diff
   needs for a call: the `path` of `write_file`, `patch`, `patch_lines` and
   `delete_file`; both `from` and `to` of `move_file`. Empty for every other
   tool and for a call missing the argument. Paths resolve against the
   project root the same way `edit_target` does, so the keys match.
2. **Capture before dispatch** — the loop captures pre-edit content for each
   of those paths (where not already captured) instead of only for
   `edit_target`'s path. Everything else `edit_target` drives — baseline,
   eviction, hooks, verify — is unchanged: do not widen `edit_target`. A
   deleted file then reports as all removals and a moved file as a removal
   at `from` plus an addition at `to`, with no change to `build_diff` or the
   numstat.
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] Preservation: every existing test passes unmodified.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- Helper, in `agent/tools.rs`:
  - `diff_targets_cover_every_edit_tool` — each of the five tools yields its
    path(s); `move_file` yields two.
  - `diff_targets_empty_for_non_edit_tools` — `read_file`, `bash`, `search`
    and a `write_file` call with no `path` yield nothing.
- Loop, in `agent/tests.rs`. The tests register the three tools
  (`patch_lines`, `delete_file`, `move_file` constructors in
  `executor/src/tools/`) alongside the existing registry; the `MockAiClientScript`
  script and `LoopDeps` shape follow `run_with_bash` in the same file. No git
  repo is needed: the "before" side is captured from disk.
  - `patch_lines_edit_appears_in_phase_diff` — a file edited with
    `patch_lines` appears in `files_changed` with the correct `+/-` summary.
  - `deleted_file_appears_as_removals_in_phase_diff` — a `delete_file`d file
    appears with zero additions and its line count as removals, and its
    lines appear as `-` lines in `diff`.
  - `moved_file_appears_as_removal_and_addition` — after `move_file`,
    `files_changed` has an all-removals entry for `from` and an all-additions
    entry for `to`.
  - `failed_delete_adds_nothing_to_diff` — `delete_file` on a path that
    does not exist leaves `files_changed` empty.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-03.txt; : > "$A"
for t in diff_targets_ patch_lines_edit_appears_in_phase_diff deleted_file_appears_as_removals_in_phase_diff moved_file_appears_as_removal_and_addition failed_delete_adds_nothing_to_diff; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count.

## Authorizations

- `executor/src/agent/mod.rs`, `executor/src/agent/tools.rs`,
  `executor/src/agent/tests.rs`.

No new dependencies.

## Out of scope

- `edit_target` and everything it drives. Whether `patch_lines` should also
  get the read-before-edit gate and the post-edit verify is a separate
  decision; note it in "Notes for review" if you have a view.
- `build_diff`, `numstat_from_pre_edit`, the tools themselves.
- Directory moves or deletes (`delete_file` refuses directories).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-27 00:10 (progress)

Started phase-03 (executor). Flipped status to `in-progress` and the milestone
README row to match. Reading the touched modules (`agent/tools.rs`,
`agent/mod.rs`, `agent/outcome.rs`, `agent/progress.rs`, `agent/tests.rs`) and
the three tool constructors (`tools/patch_lines.rs`, `tools/delete_file.rs`,
`tools/move_file.rs`) before writing the diff-targets helper.

### Update — 2026-10-27 00:23 (end-to-end verification)

Ran the phase doc's E2E block verbatim after the final code state (artifact
`target/e2e/m50-phase-03.txt`, pasted below). Positive control: each filter
shows its named test line plus a non-zero `passed` count, and each loop test
exercises the real tool against a real file in a `TempDir` — a `patch_lines`
rewrite, a `delete_file` of a 3-line file, a `move_file` across directories,
and a `delete_file` on a missing path that must add nothing — so the
assertions would fail if the capture had not reached `pre_edit_content` or
the diff/numstat had not consumed it.

```
test agent::tools::tests::diff_targets_resolve_absolute_paths_through_unchanged ... ok
test agent::tools::tests::diff_targets_empty_for_non_edit_tools ... ok
test agent::tools::tests::diff_targets_cover_every_edit_tool ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1084 filtered out; finished in 0.00s
diff_targets_ exit=0
test agent::tests::patch_lines_edit_appears_in_phase_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
patch_lines_edit_appears_in_phase_diff exit=0
test agent::tests::deleted_file_appears_as_removals_in_phase_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
deleted_file_appears_as_removals_in_phase_diff exit=0
test agent::tests::moved_file_appears_as_removal_and_addition ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
moved_file_appears_as_removal_and_addition exit=0
test agent::tests::failed_delete_adds_nothing_to_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
failed_delete_adds_nothing_to_diff exit=0
```
