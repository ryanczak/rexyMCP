# M50 — Tree-based progress

**Goal:** A `bash` call that changes the working tree counts as file progress,
for the governors and for the phase's reported diff, the same as an edit tool.

**Status:** in-progress

**Depends on:** M49.

**Why:** [Issue #14](https://github.com/ryanczak/rexyMCP/issues/14). The
governors decide "did the model make file progress" by tool name
(`tools::mutates_files`), and `bash` is never a mutating tool. Four
`NoProgressStall` hard-fails on a downstream project were runs editing
through bash (python heredocs, shell range cuts), one with all its edits on
disk by turn 20 and stalled at turn 120. Shell range cuts are now the
*required* shape for mechanical moves (WORKFLOW.md § "Executor self-sabotage
on delete-heavy rewrites"), so refusing bash-side writes is not an option.
Those stalls are not a model-quality signal.

**Exit criteria:**
- A `bash` call that changes a non-ignored file resets the read-only stall
  run; a `bash` call that changes nothing does not.
- "Mutation" means the same thing to the stall detector and the oscillation
  detector's read-only exemption.
- Files a `bash` call changed appear in the `PhaseResult` `diff` and
  `files_changed` and in the progress numstat.
- Files changed through `patch_lines`, `delete_file` or `move_file` appear
  there too (added 2026-10-26 at phase-02 review: the pre-edit capture only
  covered `write_file` and `patch`).
- A `patch_lines` of a file not read this session, or read before it
  changed, is refused like a `patch` (added 2026-10-27 at phase-03 review).
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status, M50 entry.
- `docs/architecture.md` § Status, M33 entry (the `mutates_files` single
  source of truth this milestone extends).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Bash tree changes count as progress ([phase-01-bash-tree-progress.md](phase-01-bash-tree-progress.md)) | done |
| 02 | Bash edits in the reported diff ([phase-02-bash-edits-in-diff.md](phase-02-bash-edits-in-diff.md)) | done |
| 03 | Every edit tool in the reported diff ([phase-03-all-edit-tools-in-diff.md](phase-03-all-edit-tools-in-diff.md)) | done |
| 04 | `patch_lines` is an edit-class call ([phase-04-patch-lines-edit-class.md](phase-04-patch-lines-edit-class.md)) | in-progress |

## Notes

Design decisions, 2026-10-26 (user):

1. **Signal: a fingerprint walk, not the progress numstat.** The issue
   proposed reusing the per-turn numstat, but that numstat only covers files
   an edit tool touched (`pre_edit_content`), so it is blind to bash too.
   Instead: walk the project root with the `ignore` crate (already a
   dependency) before and after each dispatched `bash` call, fingerprinting
   each non-ignored file by path, size and mtime. No git requirement, no new
   dependency. `touch` counts as a change; accepted.
2. **One predicate.** A snapshot "made progress" if its tool mutates files
   *or* its call changed the tree. Both the stall detector and the
   oscillation exemption (`window_has_mutation`) use it, so bash-edit
   A,B,A,B loops become visible to the oscillation detector. Accepted risk: a
   command that rewrites a non-ignored file every time (a test writing a
   tracked fixture) would reset the stall run on each call.
3. **Two phases.** Phase 01 is the governor fix. Phase 02 makes bash-changed
   files join the diff working set; their "before" content is not in the
   fingerprint, so phase 02 takes it from git HEAD (an untracked file is new).
   Drafted after phase 01 lands, since phase 01 decides what the loop learns
   about each bash call.
4. **Secondary fix in the issue (refuse bash-side writes) is rejected.** It
   conflicts with the shell-range-cut policy.
5. **`patch_lines` joins the edit class; `delete_file` and `move_file` do
   not** (user, 2026-10-27, from phase-03's review note). `patch_lines` over a
   drifted file has silently cut live code before, so it gets the
   read-before-edit gate and the post-edit treatment `patch` gets. A delete or
   move does not rewrite content the model never saw, so they stay ungated.
