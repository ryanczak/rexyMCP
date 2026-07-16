# M33 — Governor Mutating-Tool Classifier Unification

**Goal:** The no-progress stall governor and the escalation briefing recognize
*every* file-mutating tool (`write_file`/`patch`/`patch_lines`/`delete_file`/
`move_file`), sourced from the router's `Category::Write` set, so an executor
that edits via `patch_lines` is never falsely hard-failed with `NoProgressStall`
and its edited files are never under-reported in a briefing.

**Status:** done

**Depends on:** none

## Why now

[Issue #2](https://github.com/ryanczak/rexyMCP/issues/2): the router
(`executor/src/tools/router.rs:17`) is the canonical tool-category classifier and
correctly lists five write tools as `Category::Write`. But two consumers kept
their own stale hardcoded pair `["patch", "write_file"]` that drifted from it:

- `executor/src/governor/hard_fail.rs` — the no-progress read-only stall counter
  only reset on `patch`/`write_file`. An executor editing via `patch_lines` (a
  real, registered write tool) kept incrementing the "consecutive read-only"
  counter *while actively editing* and got a false `NoProgressStall` hard_fail.
  Observed live: three consecutive false hard_fails on a downstream project
  (DaemonEye phases 07/08/09), two of which left the tree corrupted mid-edit and
  needed an architect takeover. The `read_only_stall_threshold` knob can't
  compensate — raising it only delays the false positive.
- `executor/src/phase/briefing.rs` — `collect_working_files` gated on the same
  two names, so `patch_lines`/`delete_file`/`move_file` edits were omitted from
  the escalation briefing's working-file set, degrading triage.

Two hardcoded copies of a fact the router already owns — classic drift. The fix
makes the router the single source of truth.

## Exit criteria

- The governor resets the no-progress counter on any `Category::Write` tool
  (proven for `patch_lines`/`delete_file`/`move_file`, not just the old pair).
- The briefing collects working files for every write tool, resolving
  `move_file`'s destination via its `to` argument (not `path`).
- Neither consumer carries a private mutating-tools list; both call
  `tools::mutates_files`.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Layer 1 (the router / tool categories) and § Status
  #33 (this milestone).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Route governor + briefing through `tools::mutates_files` ([phase-01-unify-mutating-tool-classifier.md](phase-01-unify-mutating-tool-classifier.md)) | done |

## Notes

Single-phase cleanup milestone (the M29/M32 shape), **executed directly by the
architect (Claude Code)** rather than dispatched: the bug lives in the very
governor that supervises the executor loop, so dispatching it to a
`patch_lines`-favoring model risks tripping the false hard_fail mid-fix. The
change is small and surgical; the review gate still applies.

<!-- retrospective appended at milestone close -->
