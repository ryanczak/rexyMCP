# M21 — Task Coverage Gate

**Goal:** Close the `false_completion` blind spot on docs/no-code phases by
adding a task-coverage check to the `NoToolCall` completion arm, symmetric with
M19's gate-retry loop.

**Status:** done (1/1 phase approved, 2026-06-16)

**Depends on:** M19 (gate-retry loop in the `NoToolCall` arm — we extend the
same arm), M20 (closed; triggered this gap)

## Why now

M19 made `false_completion` through a *red gate* structurally impossible. M20
phase-04 surfaced the complementary blind spot: on a docs/no-code phase all four
gates pass by construction (fmt/build/lint/test have no code to fail), so the
gate-retry loop has nothing to trip on. The executor self-reported `complete`
with 2 of 10 tasks done — gates were green; the M19 guard never fired.

The fix is symmetric: maintain a task-states shadow in the loop, and after the
gate check passes, verify all seeded tasks are `Done` before returning
`PhaseResult::Complete`. If any remain incomplete, inject a named-task list and
loop — exactly as the gate check does.

## Exit criteria

- `execute_phase` with a seeded task list does **not** return `Complete` when
  the model signals `NoToolCall` before calling `update_task("done")` for every
  task.
- After the model calls `update_task(id, "done")` for all tasks, `execute_phase`
  returns `Complete`.
- When tasks are incomplete at the turn cap, `execute_phase` returns
  `BudgetExceeded`, not `Complete`.
- `EMPTY_COMMANDS` / no-task-tracking paths behave byte-identically to today
  (backward-compatible).

## Architecture references

- `executor/src/agent/mod.rs` — the `NoToolCall` arm (lines 564–639); the
  gate-retry block (578–627); the task-metadata block (809–830); `seeded`
  computation (132–136).
- `executor/src/agent/command.rs` — `gate_failure_feedback` (lines 106–140),
  the model to follow for `task_coverage_feedback`.
- `executor/src/agent/tasks.rs` — `Task` struct (lines 5–10).
- `executor/src/tools/update_task.rs` — how task state is persisted and
  returned as tool metadata.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Task-coverage gate in the NoToolCall completion arm ([phase-01-task-coverage-gate.md](phase-01-task-coverage-gate.md)) | done |

## Notes

### Retrospective — 2026-06-16

**Outcome:** 1/1 phase done. Verdict: phase-01 approved_after_1 (bug-01-1,
`parse_format`, minor — red fmt gate on first dispatch). Executor
Qwen/Qwen3.6-27B-FP8 throughout.

**What shipped.** The `false_completion` blind spot on docs/no-code phases is now
closed structurally. `execute_phase` maintains a `task_states` shadow map
(initialised from `seeded`, updated in the existing task-metadata block as
`update_task` calls land) and, after the M19 gate-retry check in the `NoToolCall`
completion arm, calls `command::task_coverage_feedback`. If any seeded task is not
`Done`, it injects a named-task list and loops — symmetric with the gate-retry
loop, and (like it) returns `BudgetExceeded` rather than `Complete` if the turn
cap is hit with tasks still incomplete. Zero `LoopDeps` struct change, so the
blast radius was three insertion points and the backward-compat pin
(`seeded.is_empty()` → `None` → all 807 pre-existing tests pass unmodified) held
exactly as designed.

**What worked:**
- **The symmetric-with-M19 design** made the phase small and the pre-injection
  cheap: quoting the gate-retry block verbatim as the shape to replicate gave the
  executor a near-exact template for both the helper and the loop hook. The two
  integration tests modelled on `gate_failure_loops_until_gates_pass` /
  `gate_failure_at_turn_cap_is_budget_exceeded` landed first-try.
- **`seeded.is_empty()` backward-compat pin** is the reason 807 existing tests
  needed zero edits — every pre-existing test seeds 0 tasks
  (`input().phase_doc = "PHASE"`), so the new check is inert for them.
- **Pre-injecting `TaskState: Copy`** pre-empted the clippy `clone_on_copy` the
  spec's literal `.state.clone()` would otherwise have tripped; the executor
  correctly dropped the `.clone()` and flagged it in Notes-for-review.

**Calibration data (1st-occurrence, no folds):**
1. **`parse_format` bounce (bug-01-1).** The first dispatch self-reported
   `complete` with a red `cargo fmt --all --check` — rustfmt wanted the
   `task_states` init chain collapsed to one line. The notable part: M19's
   gate-retry loop *should* have caught a red fmt gate, but it did not fire,
   because the executor's own gate run that turn apparently passed `format`. The
   diff surfaced only on the reviewer's independent `cargo fmt --all --check`.
   This is a **1st-occurrence executor-vs-reviewer fmt disagreement** — data, not
   a fold. If it recurs, the investigation is whether the executor's gate set runs
   `cargo fmt --all --check` (verify-only) or only the writing form, and whether
   its working-tree state at gate time differs from the reviewer's. Flagged for
   the user.

**No WORKFLOW folds proposed.** One occurrence of a new sub-pattern
(executor-vs-reviewer fmt divergence); the dominant cross-milestone
`false_completion` class is the one this milestone *closes* for no-gate-coverage
phases, complementing M19's red-gate closure. Together M19 + M21 cover both
variants structurally.
