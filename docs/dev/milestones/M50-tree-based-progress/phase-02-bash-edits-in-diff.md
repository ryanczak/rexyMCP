# Phase 02: Bash edits in the reported diff

**Milestone:** M50 — Tree-based progress
**Status:** in-progress
**Depends on:** phase-01
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The `PhaseResult` `diff` / `files_changed` and the progress numstat cover only
files an edit tool touched, so a phase whose edits went through `bash` comes
back with an empty or partial diff. Make files a `bash` call changed show up
there too.

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` § Notes, decision 3.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

The agent loop (`executor/src/agent/mod.rs`) keeps `pre_edit_content`: the
"before" text of each file, captured just before an edit tool first touches
it. `build_diff` (`agent/outcome.rs`) and `numstat_from_pre_edit`
(`agent/progress.rs`) diff those entries against disk. Phase 01 fingerprints
the tree around each dispatched `bash` call (`agent/tree_fingerprint.rs`), but
the loop only uses the result as a bool.

## Spec

1. **HEAD content lookup** — a helper that, given the project root and a path
   relative to it, returns that file's text at git `HEAD`, or `None` when
   there is none: root not inside a git repo, repo with no commits, file
   untracked at HEAD, or content not UTF-8. Never errors; runs `git` as a
   subprocess (the crate already does this elsewhere). Gotcha: the project
   root may be a *subdirectory* of the repo, and `git show HEAD:<path>`
   resolves `<path>` from the repo top level. A path starting with `./` is
   resolved from the current working directory instead, so run it as
   `git show HEAD:./<rel>` with the working directory set to the project root.
2. **Bash-changed files join the diff working set** — in the loop, keep the
   changed paths from the `bash` call's before/after comparison, not just the
   bool. Add each one that is not yet in `pre_edit_content` to it, with the
   HEAD content from task 1 as its "before" side (`None` = a new file). Use
   the same key form the edit tools use: a file edited by both an edit tool
   and `bash` must be one entry, whichever came first. A path already in
   `pre_edit_content` keeps its existing "before".
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

Write the tests early: start on the lookup tests as soon as the helper
compiles.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] Preservation: every existing test passes unmodified.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

Tests that need a commit build a git repo in a `TempDir`. Constraint: they
must not read the host's git config. Set `GIT_CONFIG_GLOBAL=/dev/null` and
`GIT_CONFIG_NOSYSTEM=1` on every `git` call, and pass the identity
(`-c user.name=… -c user.email=…`) and `-c commit.gpgsign=false` to the
commit. A host with commit signing or no identity configured must still pass.

- HEAD lookup:
  - `head_content_returns_committed_text` — a committed file returns its
    committed text even after the working copy changed.
  - `head_content_is_none_for_untracked_file`
  - `head_content_is_none_outside_git_repo`
  - `head_content_resolves_from_subdirectory_root` — project root is a
    subdirectory of the repo; the lookup still finds the file.
- Loop, in `agent/tests.rs` (`MockAiClientScript`, `bash` registered via
  `bash_with_filter` the way `run_bash_stall_script` does):
  - `bash_edit_appears_in_phase_diff` — a committed file rewritten by `bash`
    appears in `files_changed` with the correct `+/-` summary, and its change
    is in `diff`.
  - `bash_created_file_appears_as_new_in_phase_diff` — an untracked file
    created by `bash` shows as all additions.
  - `file_edited_by_tool_and_bash_appears_once` — `write_file` then `bash`
    on the same file gives one `files_changed` entry, diffed against the
    original content.
  - `read_only_bash_adds_nothing_to_diff` — `bash` calls that change nothing
    leave `files_changed` empty.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-02.txt; : > "$A"
for t in head_content_ bash_edit_appears_in_phase_diff bash_created_file_appears_as_new_in_phase_diff file_edited_by_tool_and_bash_appears_once read_only_bash_adds_nothing_to_diff; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count.

## Authorizations

- `executor/src/agent/mod.rs`, `executor/src/agent/tests.rs`,
  `executor/src/agent/tree_fingerprint.rs`, and one new or existing module
  under `executor/src/agent/` for the lookup.

No new dependencies.

## Out of scope

- `build_diff` and `numstat_from_pre_edit`: they already diff whatever is in
  `pre_edit_content`, so they need no change.
- Recording `changed_tree` or changed paths in the session log.
- Dirty-at-start trees: HEAD is the "before" by design (pre-flight requires a
  clean tree).
- `destructive_restore_refusal` now also protects bash-edited files, because
  it reads `pre_edit_content`. That is intended; do not change it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-26 16:06 (progress)

Started phase-02 (executor). Flipped status to `in-progress` and the milestone
README row to match. Reading the touched modules (`agent/mod.rs`,
`agent/tree_fingerprint.rs`, `agent/outcome.rs`, `agent/tests.rs`) before
writing the HEAD-content lookup helper.
