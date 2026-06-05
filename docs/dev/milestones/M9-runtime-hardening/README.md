# M9 — Executor runtime hardening

**Goal:** Fix recurring executor hard-fail classes at the runtime level when a
phase-spec instruction provably cannot, starting with the post-write formatting
race folded from M1/mp3-player.

**Status:** in-progress (phase-07 added 2026-06-05)

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
| 04 | split agent/mod.rs into focused submodules ([phase-04-agent-mod-split.md](phase-04-agent-mod-split.md)) | done   |
| 05a | split dashboard: extract filter, highlight, transcript ([phase-05a-dashboard-split-filter-highlight-transcript.md](phase-05a-dashboard-split-filter-highlight-transcript.md)) | done   |
| 05b | split dashboard: extract panels, render, event_loop ([phase-05b-dashboard-split-panels-render-loop.md](phase-05b-dashboard-split-panels-render-loop.md)) | done   |
| 06  | update dashboard spinner animation ([phase-06-spinner-update.md](phase-06-spinner-update.md)) | done   |
| 07  | align header panel borders with body panel borders ([phase-07-dashboard-border-alignment.md](phase-07-dashboard-border-alignment.md)) | done   |

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

### phase-04 addendum (2026-06-04) — structural refactor, escalated

**phase-04** (split `agent/mod.rs` into focused submodules, **escalated**): a
pure move-only refactor extracting ~550 lines of private helpers from the
4 507-line `mod.rs` into 4 new private modules (`log`, `tools`, `outcome`,
`metrics`) and extending 2 public ones (`progress`, `command`). 585 tests pass
unchanged. Shipped via **architect session takeover** after two executor
hard-fails.

**What broke — two bounces, two distinct causes:**
1. **dispatch-1 (architect spec gap):** the original ordering constraint claimed
   "Phase A keeps the build green because new files aren't linked until Task 7."
   False — `progress.rs`/`command.rs` are already-compiled `pub mod`s, so
   extending them with `use super::log::…` *before* declaring `mod log;` made the
   build red and the verifier hard-failed at 3 consecutive red turns. Also: the
   `command.rs` import block omitted `CommandOutputs`/`Gates` (the moved
   `run_command_set`'s return types). Both fixed in the spec → refined re-dispatch.
2. **dispatch-2 (executor stall):** the executor applied Task 7a cleanly
   (module decls + `use` re-exports) but then looped on identical `read_file`
   calls instead of patching out the original function bodies (Task 7b),
   tripping `IdenticalToolCallRepetition`. Session takeover finished the
   mechanical deletion (560 lines) + fixed the test-module imports that were no
   longer in `super::*` scope after extraction.

**Calibration:**
3. **Mechanical-deletion churn → `IdenticalToolCallRepetition` (second
   occurrence).** phase-10b stalled the same way on multi-edit *test-update*
   churn; phase-04 stalled on multi-deletion churn in a large file. Two
   occurrences of the same class = a trend, not yet a fix. If it recurs, fold a
   WORKFLOW rule: split a large mechanical-deletion/edit task from its companion
   step, or pre-inject exact `old_str` anchors per group. Confirmed with the user
   that **raising `IDENTICAL_CALL_THRESHOLD` is not the lever** — the stall was
   identical *reads* (the executor stuck, not making progress), so a higher
   threshold buys more stuck turns, not success, and weakens the shared
   verifier-persistence governor.
4. **Spec ordering for cross-module refactors (new, one occurrence).** When a
   refactor adds cross-module references (`use super::X`) into already-compiled
   modules, the `mod X;` declaration must land *before or with* the reference,
   not in a later task. Verify intermediate build states are actually reachable
   when sequencing a multi-step move. Data point.
