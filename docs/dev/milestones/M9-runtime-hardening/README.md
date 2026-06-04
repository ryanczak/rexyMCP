# M9 — Executor runtime hardening

**Goal:** Fix recurring executor hard-fail classes at the runtime level when a
phase-spec instruction provably cannot, starting with the post-write formatting
race folded from M1/mp3-player.

**Status:** done (both in-scope phases approved 2026-06-04; at close gate)

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
| 02 | lint-fix in the post-write hook ([phase-02-lint-fix-hook.md](phase-02-lint-fix-hook.md))          | done   |

## Notes

This milestone exists because the **calibration fold** "Post-write formatting is a
runtime concern, not a spec concern" (WORKFLOW.md) identifies a fix that must be
runtime-side: a phase spec cannot prevent a later `write_file` from undoing a
formatter the executor ran earlier in the same turn loop. M1/mp3-player hit this on
four phases (google/gemma-4-12b); the phase-03 spec pre-injected a "run the
formatter before staging" step and it still failed, confirming the fix belongs in
the runtime, not the spec.

### Retrospective (2026-06-04)

Both in-scope phases done. **phase-01** (post-write format hook, approved_after_2)
and **phase-02** (lint_fix in the hook, approved_after_1) — Qwen/Qwen3.6-27B-FP8.
The runtime now runs `lint_fix` then `format` after every successful edit-class
turn, before the verifier; both best-effort. The formatting-overwrite hard-fail
class (the reason M9 exists) is closed, and autofixable lint no longer accumulates
into a `VerifierFailurePersistent`.

**What worked:** Both phases produced correct production code on the first real
attempt. The additive `CommandConfig` field (phase-02) kept production compiling —
the only churn was ~10 `#[cfg(test)]` literals, exactly as the "prefer additive
shapes" fold predicts. Splitting format (phase-01, no config change) from lint_fix
(phase-02, config schema change) per the phase-10b single-concern calibration kept
each phase's blast radius small.

**What broke — three executor bounces, all the same root cause:** every non-clean
dispatch (phase-01 dispatch-1, phase-02 dispatch-1) traced to the executor doing a
whole-file `read_file` on `executor/src/agent/mod.rs` (~150–165 KB) — once tripping
`RunawayOutput`, once `IdenticalToolCallRepetition`. (A third phase-01 bounce was an
infra SSE stall, not the executor.) Pre-injecting **exact `old_str`/`new_str` patch
targets** (not just "here's what the code looks like") resolved phase-02 cleanly:
the executor patched without ever reading the big file.

**Calibration → folds (pending user sign-off):**
1. **Large-file edits:** two occurrences = trend. Candidate WORKFLOW fold — when a
   phase edits a file too large to read whole, pre-inject exact patch targets and
   forbid a whole-file `read_file`. (Architect spec-writing discipline.)
2. **Runtime fix queued separately:** `read_file` should truncate at a threshold and
   gain `offset`/`limit` line params so a large read can't trip the hard-fail
   detector — drafted as the next M9 phase (phase-03), not a doc fold.
3. **Commit hygiene (phase-01):** re-dispatching against a dirty tree let the executor
   sweep unrelated changes into its commit. One occurrence — data only.
