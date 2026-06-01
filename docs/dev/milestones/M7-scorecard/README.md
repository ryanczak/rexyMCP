# M7 — Model scorecard & routing

**Goal:** Consume the `PhaseRun` telemetry accumulated since M4 to produce a
meaningful model-vs-model competency matrix and a routing policy that maps a
phase's tags to the best-scoring local model.

**Status:** in progress — phase-01 in progress; later phases drafted on demand.

**Depends on:** M6 (done) — the full stack is live, the dogfood validated the
loop, and the `PhaseRun` store has been accumulating records since M4.

**Exit criteria:**

- **Terminal backend `Err` → `hard_fail` degradation (phase-01, carry-over from
  M6).** A mid-phase terminal model error (after ≥1 completed turn) degrades to
  a `hard_fail` `PhaseResult` with briefing + partial artifacts, instead of
  aborting `execute_phase`. Pre-work connection errors (at turn 0) remain `Err`.
  Rationale: the architecture's escalation contract is "return a structured
  result, let the host re-invoke"; aborting discards recoverable work.
- **Curated benchmark suite.** A small set of reference phases (one per tag
  combination: language × kind × size) that can be dispatched against multiple
  models to produce controlled, apples-to-apples `PhaseRun` records. The
  benchmark runner picks the right phase per model-under-test and emits records
  into the shared telemetry store.
- **`model_scorecard` meaningful output.** With real `PhaseRun` data flowing,
  the Layer 2 `model_scorecard` tool (already implemented in M5) produces a
  non-trivial `model × tag → { n_runs, first_pass_rate, mean_turns,
  parse_failure_rate, mean_bugs, … }` matrix with sample sizes. The phase
  verifies it produces useful output on real data and fixes any gaps in the
  aggregation.
- **Routing policy.** A `recommend_model(tags)` function that reads the
  scorecard, selects the best-scoring model for a given tag set (argmax of
  `first_pass_rate` subject to a minimum sample size), and exposes an
  exploration policy (epsilon-greedy) so new models still get sampled. The
  architect can call this before dispatch, or `execute_phase` can accept a
  `model: "auto"` sentinel that invokes it.

## Architecture references

- `docs/architecture.md` — "Model effectiveness metrics & routing" (the
  `PhaseRun` schema, the scorecard aggregation, the routing policy design).
- `docs/architecture.md` § Layer 1 "Escalation = Claude Code itself" and
  "The `PhaseResult` / briefing contract" (grounds the phase-01 decision).
- `docs/architecture.md` § Layer 2 — `model_scorecard` tool (what the
  aggregation exposes to the architect).

## Phases

| #  | Phase                                                                   | Status |
|----|-------------------------------------------------------------------------|--------|
| 01 | terminal backend `Err` → `hard_fail` degradation ([phase-01-backend-error-degradation.md](phase-01-backend-error-degradation.md)) | todo |

Phases 02 (benchmark suite), 03 (scorecard data analysis + gaps), and 04
(routing policy) draft on demand.

## Notes

**Phase-01 is a carry-over from M6.** The M6 retrospective (phase-06b) decided
that mid-phase terminal backend errors should degrade to `hard_fail` rather than
aborting `execute_phase`. It is the only M7 phase with well-defined scope today
(the implementation sites are known: `executor/src/agent/mod.rs:238` and
`:271-273`); phases 02–04 depend on having real `PhaseRun` data and a running
benchmark setup, which is data that comes from *using* the system rather than
from the design.

**Routing depends on data.** A routing policy trained on N<5 runs per tag is
not a routing policy, it's noise. Phase-04 should be blocked until the
scorecard has a minimum sample size (suggest: 5+ runs per tag combination
exercised). The M7 retrospective will assess whether that threshold is met.
