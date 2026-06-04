# M9 — Executor runtime hardening

**Goal:** Fix recurring executor hard-fail classes at the runtime level when a
phase-spec instruction provably cannot, starting with the post-write formatting
race folded from M1/mp3-player.

**Status:** planning

**Depends on:** M4 (the agent loop: dispatch → verify → final command set)

**Exit criteria:**
- The executor runs the project's configured `format` command (and, when
  configured, a `lint_fix` autofix command) as a post-write, pre-verifier hook
  after every turn that wrote a file to disk, making formatting unconditional and
  turn-ordering-independent.
- The "executor formats, then a later `write_file` overwrites the formatted file,
  then the final `fmt --check` fails" hard-fail class no longer occurs.
- Mechanically-autofixable lint diagnostics are resolved each turn rather than
  accumulating into a `VerifierFailurePersistent` hard-fail.

## Architecture references

- `docs/architecture.md#the-executor-turn-cycle` — turn-cycle steps 5 (dispatch),
  6 (verify), and the final command set.

## Phases

| #  | Phase                                                                          | Status |
|----|--------------------------------------------------------------------------------|--------|
| 01 | post-write format hook ([phase-01-post-write-format-hook.md](phase-01-post-write-format-hook.md)) | done   |
| 02 | lint-fix in the post-write hook ([phase-02-lint-fix-hook.md](phase-02-lint-fix-hook.md))          | todo   |

## Notes

This milestone exists because the **calibration fold** "Post-write formatting is a
runtime concern, not a spec concern" (WORKFLOW.md) identifies a fix that must be
runtime-side: a phase spec cannot prevent a later `write_file` from undoing a
formatter the executor ran earlier in the same turn loop. M1/mp3-player hit this on
four phases (google/gemma-4-12b); the phase-03 spec pre-injected a "run the
formatter before staging" step and it still failed, confirming the fix belongs in
the runtime, not the spec.

Milestone assignment is provisional — the architect/human decides whether this is
M9 or folds into another milestone before `NEXT.md` points at phase-01.
