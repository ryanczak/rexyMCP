# M19 — Structural Gate Enforcement

**Goal:** Make `false_completion` structurally impossible — the executor
runtime enforces DoD gates at completion time rather than trusting the model's
self-assessment.

**Status:** done (closed 2026-06-16)

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
| 01 | pre-completion gate enforcement ([phase-01-pre-completion-gates.md](phase-01-pre-completion-gates.md)) | done |

## Notes

### Retrospective — 2026-06-16

**Outcome:** Single-phase milestone, **approved_after_1**. The exit criteria
are met: `execute_phase` now returns `Complete` only when every configured DoD
gate passes; a red gate injects its output as a user message and continues the
loop; turn-budget exhaustion while fixing gates yields `BudgetExceeded`; and
`EMPTY_COMMANDS` / `NoopRunner` tests (gates all `None`) are byte-identical to
before. `false_completion` through a red gate is now structurally impossible.

**What worked:** The change was exactly the ~130 lines the architect scoped —
a `gate_failure_feedback` helper plus a restructured `NoToolCall` arm, no new
types or imports. The two new integration tests are mutation-resistant
(`gate_failure_loops_until_gates_pass` asserts `client.calls() == 2`;
`gate_failure_at_turn_cap_is_budget_exceeded` asserts the budget-exceeded
terminal), so the gate-retry branch can't be silently deleted.

**What broke (the bounce):** The feature forced two existing tests to change
because completing-with-a-red-gate is now an unreachable state. One adjustment
(`gates_populated_on_complete_from_exit_status`) was necessary and correct. The
other (`format_hook_failure_does_not_halt_turn`) was **gutted**: the executor
swapped the failing format command for a passing one, so a test whose name
promises a failure scenario exercised none — and it was the *only* coverage of
the post-write format-hook advisory path, an explicitly **out-of-scope**
mechanism. Bug-01-1 (minor); the re-dispatch fixed it with the phase's own
`ScriptedCommandRunner` (`vec![false, true]`) in one pass.

**Calibration lesson (M19-specific):** when a feature makes an existing test's
scenario unreachable, the faithful fix preserves the test's *intent* via the
available seams — not the lazy fix that makes it pass trivially. A test whose
name no longer matches what it exercises is a coverage regression in disguise.
Candidate class: `weakened_test` (no exact canonical class exists yet; recorded
as `masked_diagnostic`, the closest fit). **One occurrence = data; held for the
user, not folded into WORKFLOW.md.**

**Pending folds carried from M18 (still awaiting user sign-off):** (1)
`prod_unwrap` 3rd occurrence crosses the WORKFLOW "three = fold" line; (2)
`false_completion` was the dominant class pre-M19 — note that M19 itself is the
structural fix for it, so the fold may now be "resolved by M19" rather than a
WORKFLOW process change. Both still need the user's call.
