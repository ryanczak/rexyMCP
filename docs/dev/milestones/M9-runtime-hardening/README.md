# M9 — Executor runtime hardening

**Goal:** Fix recurring executor hard-fail classes at the runtime level when a
phase-spec instruction provably cannot, starting with the post-write formatting
race folded from M1/mp3-player.

**Status:** in-progress (phase-04 added 2026-06-04)

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
| 03 | read_file output cap ([phase-03-read-file-line-cap.md](phase-03-read-file-line-cap.md))           | done   |
| 04 | split agent/mod.rs into focused submodules ([phase-04-agent-mod-split.md](phase-04-agent-mod-split.md)) | todo   |

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

**phase-03** (read_file output cap, approved_first_try): `read_file` now caps output
at 500 lines and appends a truncation notice with the file's total line count and
shown range. Single-file change, 6 new tests, clean `feat:` commit. The executor
that struggled to read `mod.rs` in phases 01 and 02 will now receive the first 500
lines with a clear pointer to re-read with `start_line`/`end_line` — no more
`RunawayOutput` on large files.

**Calibration:**
1. **Large-file edits (closed by phase-03):** two occurrences across phases 01/02
   traced to the executor reading `mod.rs` (~150–165 KB) whole. The runtime fix
   (phase-03 line cap) is now in place. The architect spec discipline (pre-inject
   exact patch targets) remains sound but is now a belt-and-suspenders measure
   rather than the only defense. **No WORKFLOW fold needed** — the user decided the
   runtime fix supersedes the spec-writing fold.
2. **Commit hygiene (phase-01):** re-dispatching against a dirty tree let the
   executor sweep unrelated changes into one commit. One occurrence — data only.
