# M50 — Tree-based progress

**Goal:** A `bash` call that changes the working tree counts as file progress,
for the governors and for the phase's reported diff, the same as an edit tool.

**Status:** done *(opened 2026-10-26, closed 2026-10-27; four phases)*

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
| 04 | `patch_lines` is an edit-class call ([phase-04-patch-lines-edit-class.md](phase-04-patch-lines-edit-class.md)) | done |

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

## M50 retrospective — closed 2026-10-27

**Outcome:** issue #14 is fixed, and so are two diff gaps found along the way.
A `bash` call that changes the tree now counts as file progress for the stall
detector, the oscillation exemption and novelty. Files changed by `bash`,
`patch_lines`, `delete_file` and `move_file` now appear in the phase diff and
the progress numstat. `patch_lines` is now edit-class: it is gated by
read-before-edit and verified after each edit.

| Phase | Verdict | Executor turns | Notes |
|---|---|---|---|
| 01 bash tree progress | `escalated` | 120 + 120 + 120, then takeover | first stall was the bug itself; two genuine stalls after |
| 02 bash edits in diff | `approved_first_try` | 212 | |
| 03 every edit tool in diff | `approved_first_try` | 116 | added mid-milestone at phase-02 review |
| 04 `patch_lines` edit-class | `approved_first_try` | 112 | added mid-milestone (user decision 5) |

**What went wrong in phase-01, all of it spec-side:**
- The Test plan said "a small `read_only_stall_threshold`" without naming how
  a loop test sets one. Two runs spent 120 calls each hunting for it, and the
  refined re-dispatch that named it still stalled. The executor re-audited
  finished work despite an explicit "do not re-read" note.
- The spec missed that the loop writes its own session log under
  `<root>/.rexymcp/`. In a repo that does not gitignore it, every `bash` call
  would have read as a tree change and the stall detector would never fire on
  bash. The architect's negative loop test caught this at takeover, and the
  fingerprint now prunes the root's `.rexymcp/` unconditionally.
- The first run's stall was the bug being fixed (scripted bash edits under
  the old governor). It is not a model-quality signal.

**What worked:** phases 02–04 used short docs (~100–130 lines) that named the
analogue helper from the previous phase (`run_bash_stall_script`,
`run_with_bash`, `run_with_all_edit_tools`) instead of describing the harness.
All three landed first try, in falling turn counts.

**Calibration, for the human:**
1. **Prose-only mutation evidence: four of four phases.** Every executor
   Update Log claimed its tests "would fail" without pasting a failing run. The
   reviewer reproduced each one (all were real). WORKFLOW.md's E2E guidance
   asks for captured output, but nothing asks for a captured *mutation* run, so
   the review skill's "accept the executor's evidence" path never applied.
   Three occurrences is a fix by the calibration rule; the candidate fold is to
   seed a "mutation-check, captured" task the way the E2E capture is seeded.
   Held for your decision (contract doc).
2. **Leaked reasoning in `completion_summary`** (phase-03, `</think>…` ahead of
   the Summary). One occurrence; held.
3. **Executor self-misreporting:** a wrong pre-phase test count (02) and an
   Update Log naming the executor "Claude" (04). Minor; held.

**Follow-ups outside this milestone:**
- `rexymcp calibrate-governor` replays snapshots from session logs, which do
  not record `changed_tree`, so its replays are still tool-name-only.
- The running `rexymcp serve` must be restarted onto the rebuilt binary to get
  any of this.
