# M7 — Per-run statistics & model scorecard

> **⚠ DIRECTION CHANGE — 2026-06-02.** The benchmark-suite approach is
> **deprecated**. M7 originally aimed at a curated benchmark suite (the same
> phases run by each model) plus an automated routing policy. That is dropped.
> The scorecard concept is **kept**, but it now aggregates **regular rexyMCP
> runs**, not specialized benchmark runs. Phases **02 / 03a / 03b** were rolled
> back (code reverted in `971d0c4` + `dc5b6be`; benchmark partial work
> discarded). Phase-01 stands. The detailed phase plan for the new direction is
> the next architect task — the exit criteria below are the high-level target,
> not yet decomposed into phase docs.

**Goal:** Give rexyMCP users **detailed, per-run statistics** for every regular
run, so they can make informed decisions about **which local LLM to use** and
**which settings work best** for that LLM. The `model_scorecard` aggregation is
retained, fed by ordinary production telemetry rather than benchmark records.

**Status:** in progress — phase-01 done; benchmarking deprecated; the per-run
statistics direction is pending design.

**Depends on:** M6 (done) — the full stack is live, and the `PhaseRun` store has
been accumulating regular-run records since M4.

**Exit criteria** (high-level target for the new direction — to be decomposed
into phase docs):

- **Terminal backend `Err` → `hard_fail` degradation (phase-01 — DONE).** A
  mid-phase terminal model error (after ≥1 completed turn) degrades to a
  `hard_fail` `PhaseResult` with briefing + partial artifacts, instead of
  aborting `execute_phase`. Pre-work connection errors (at turn 0) remain `Err`.
- **Per-run statistics surface.** Each regular run's detailed stats — model,
  generation settings (temperature/seed/…), gates, parse-failure rate,
  repairs-per-call, verifier retries, tool-success rate, turns, wall-clock,
  tokens, bounces, verdict — are viewable per run, so a user can compare runs
  and see which model + settings performed best.
- **Scorecard over regular runs.** The `model_scorecard` tool (M5) aggregates
  ordinary `PhaseRun` records (no benchmark provenance) into a model × tag (and,
  candidate, model × settings) competency matrix with sample sizes.

## Architecture references

- `docs/architecture.md` — "Model effectiveness metrics & routing" (the
  `PhaseRun` schema, the scorecard aggregation). **Note:** the "Benchmark vs.
  telemetry" and automated-"Routing" portions of that section are now superseded
  by this direction change and need an architect pass to realign.
- `docs/architecture.md` § Layer 1 "Escalation = Claude Code itself" and
  "The `PhaseResult` / briefing contract" (grounds the phase-01 decision).
- `docs/architecture.md` § Layer 2 — `model_scorecard` tool.

## Phases

| #  | Phase                                                                   | Status |
|----|-------------------------------------------------------------------------|--------|
| 01 | terminal backend `Err` → `hard_fail` degradation ([phase-01-backend-error-degradation.md](phase-01-backend-error-degradation.md)) | done (approved_first_try) |
| 02 | benchmark provenance on `PhaseRun` + scorecard source filter ([phase-02-benchmark-provenance.md](phase-02-benchmark-provenance.md)) | rolled-back (benchmarking deprecated) |
| 03a | thread `bench_suite` through the loop + stamp a single benchmarked run ([phase-03a-bench-suite-threading.md](phase-03a-bench-suite-threading.md)) | rolled-back (benchmarking deprecated) |
| 03b | `rexymcp bench` multi-model sweep + one minimal fixture ([phase-03b-bench-sweep.md](phase-03b-bench-sweep.md)) | rolled-back (never landed) |
| 04 | `rexymcp runs` — per-run statistics CLI view ([phase-04-runs-cli.md](phase-04-runs-cli.md)) | todo |

**The per-run statistics direction (designed 2026-06-02 with the user)** decomposes
into three phases:

- **04 — `rexymcp runs` (this).** A read-only CLI view that lists individual
  `PhaseRun` records (model, settings, gates, reliability/efficiency, verdict),
  filterable by model/tag, newest-first. The most direct "see detailed statistics
  for each run." Works on existing data.
- **05 — settings plumbing.** Make sampling settings (temperature/seed/…)
  configurable, sent to the model, and recorded with real values in `PhaseRun`.
  Today `generation_params` is always default `None` — this makes the "which
  settings" axis real.
- **06 — settings slice on the scorecard.** Aggregate/compare `model × settings`
  (depends on 05's real data).

## Notes

**Phase-01 is a carry-over from M6.** The M6 retrospective (phase-06b) decided
that mid-phase terminal backend errors should degrade to `hard_fail` rather than
aborting `execute_phase`. Implemented and approved.

**Benchmarking deprecation (2026-06-02).** The benchmark-suite path (phases
02 / 03a / 03b) was rolled back. Rationale: rather than build a separate
controlled-benchmark apparatus (curated fixtures, a multi-model sweep, a
`bench_suite` provenance distinction), the scorecard will track **regular**
rexyMCP runs and surface detailed per-run statistics, letting the user make the
model/settings call directly. The reverted commits removed the `bench_suite`
field on `PhaseRun`, the scorecard `SourceFilter`, the `LoopDeps`/CLI threading,
and the (unlanded) sweep. The phase docs are retained with rolled-back banners
for historical context. `docs/architecture.md`'s benchmark/routing language
still needs an architect pass to match this direction.
