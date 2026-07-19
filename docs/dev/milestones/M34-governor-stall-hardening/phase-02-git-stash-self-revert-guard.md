# Phase 02: Refuse `git stash` self-revert (FR-1)

**Milestone:** M34 — Governor Stall Hardening
**Status:** done
**Depends on:** none
**Estimated diff:** ~114 lines
**Tags:** language=rust, kind=feature, size=m

> **Retrospective phase doc.** This work was executed **directly by the architect
> (Claude Code)** and committed as `a9399a0` (feat) + the docs portion of
> `dc1155d` *before* this doc was written. The doc records what shipped.

## Goal

Extend the working-set-aware self-revert refusal to cover `git stash` push forms.
The M22 guard scanned per-path `git checkout` / `git restore` (including the
`HEAD -- <path>` form); a bare `git stash` is a *whole-session* revert that the
per-path token scan never saw.

## Why

DaemonEye M4 phases 01 and 03 saw the executor wipe its own correct work via
`git stash` / `git stash pop`, then loop in confusion — the exact self-sabotage
the M22 refusal exists to prevent, through a form it missed.

## What shipped

- `stashes_working_tree(command)` classifier — `executor/src/agent/tools.rs` —
  matches the **push** forms (bare `git stash`, `git stash push`, `git stash
  save`) and correctly *allows* the restore/inspect forms (`pop`/`apply`/`list`/
  `show`/`drop`/`branch`/`clear`). Handles compound commands (e.g. `... && git
  stash`).
- Wired into the existing pre-run bash refusal: when the executor has edited any
  file this session and issues a stash-push, the command is refused before it
  runs — mirroring the per-path `git checkout`/`restore` refusal.

## Acceptance criteria

- [x] Bare `git stash` / `git stash push` / `git stash save` are refused when the
      session has edits.
- [x] Restore/inspect forms (`pop`, `apply`, `list`, `show`, …) stay allowed.
- [x] A stash-push inside a compound command is caught.
- [x] With no session edits, stash is allowed (nothing to protect).
- [x] All four gates green.

## Test plan (shipped)

- `refuses_bare_stash_when_edited`
- `refuses_stash_push_and_save_when_edited`
- `refuses_stash_in_compound_command`
- `allows_stash_restore_and_inspect_forms` — the load-bearing negative.
- `allows_stash_when_no_edits`

## End-to-end verification

Not applicable — pure loop-internal refusal path, exercised by the hermetic
tests above; the motivating artifact is the DaemonEye field report.

## Authorizations

- [x] Touched `docs/architecture.md` § Layer 1 (self-revert note) + `README.md`
      guardrail list via `dc1155d`.

## Out of scope

- The stall detectors (phases 01, 03).

## Update Log

<!-- entries appended below this line -->

### Update — 2026-07-14 (complete)

**Summary:** Shipped `stashes_working_tree` + the stash-push refusal, covering
the whole-session revert the per-path guard missed. Committed `a9399a0`; docs
folded via `dc1155d`.

### Review verdict — 2026-07-18 (retrospective)

- **Verdict:** approved_first_try (retrospective — reviewed at M34 formalization)
- **Bounces:** none
- **Executor:** Claude Code (direct)
- **Scope deviations:** none
- **Calibration:** none — completes the M22 self-revert family (checkout/restore
  → + stash). Pinned negative (`allows_stash_restore_and_inspect_forms`) follows
  the STANDARDS "pin negative cases" discipline for classifier work.
