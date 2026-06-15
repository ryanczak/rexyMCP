# M19 — Structural Gate Enforcement

**Goal:** Make `false_completion` structurally impossible — the executor
runtime enforces DoD gates at completion time rather than trusting the model's
self-assessment.

**Status:** todo (staged; starts after M18 closes)

**Depends on:** M18 (uses the `Gates` / `CommandOutputs` types already in the
tree; no M18 feature depends on M19)

## Why now

`false_completion` has recurred across M12, M17, and M18: the executor reports
`complete` while the format gate is red, tests don't compile, or production
`unwrap`s remain. The current mitigation is pre-injection gotchas in phase
docs — architect labour that scales linearly with phases and still leaves the
door open.

The root cause is a control-flow gap: `run_command_set` already runs at
completion and the `Gates` result is already computed, but the loop does not
inspect it before returning `PhaseResult::Complete`. Closing that gap is a
~130-line change in the executor runtime. Once it lands, no amount of model
optimism can produce a `complete` result through a red gate: the gate output
is injected back into the conversation and the model must fix and re-complete.

## Exit criteria

- `execute_phase` returns `PhaseResult::Complete` only when all configured
  DoD gates pass (exit 0).
- When a gate exits non-zero at completion time, the runtime injects the
  failure output as a user message and continues the loop.
- If the model exhausts its turn budget while fixing gate failures, the result
  is `PhaseResult::BudgetExceeded` (not `Complete`).
- Existing tests that use `EMPTY_COMMANDS` / `NoopRunner` pass unchanged
  (gates all `None` → no enforcement → backward-compatible).

## Architecture references

- `docs/architecture.md` §"Layer 1 — executor library" — the turn cycle and
  the terminal outcomes (`complete` / `hard_fail` / `budget_exceeded`).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | pre-completion gate enforcement ([phase-01-pre-completion-gates.md](phase-01-pre-completion-gates.md)) | todo |

## Notes

### Retrospective — (filled at milestone close)
