# M21 — Task Coverage Gate

**Goal:** Close the `false_completion` blind spot on docs/no-code phases by
adding a task-coverage check to the `NoToolCall` completion arm, symmetric with
M19's gate-retry loop.

**Status:** in-progress (phase-01 active)

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
| 01 | Task-coverage gate in the NoToolCall completion arm ([phase-01-task-coverage-gate.md](phase-01-task-coverage-gate.md)) | todo |

## Notes

### Retrospective — (filled at milestone close)
