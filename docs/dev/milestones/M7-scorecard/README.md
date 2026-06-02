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

**Status:** **done** (2026-06-02) — phase-01 + 04 + 05a/05b/05c + 06 all approved;
benchmarking deprecated (02/03a/03b rolled back). The per-run statistics substrate
and the `model × settings` scorecard are complete. See the retrospective in Notes.

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
| 04 | `rexymcp runs` — per-run statistics CLI view ([phase-04-runs-cli.md](phase-04-runs-cli.md)) | done (approved_first_try) |
| 05a | settings plumbing — temperature/seed configurable, sent, recorded ([phase-05a-settings-plumbing.md](phase-05a-settings-plumbing.md)) | done (approved_after_2) |
| 05b | chat-stream provenance — served model id + `finish_reason`/length-truncation rate ([phase-05b-stream-provenance.md](phase-05b-stream-provenance.md)) | done (approved_after_1) |
| 05c | context window — `max_model_len` from `/v1/models` ([phase-05c-context-window.md](phase-05c-context-window.md)) | done (approved_first_try) |
| 06 | `model × settings` scorecard slice — `rexymcp scorecard` CLI ([phase-06-settings-scorecard.md](phase-06-settings-scorecard.md)) | done (approved_first_try) |

**The per-run statistics direction (designed 2026-06-02 with the user)** decomposes
into three phases. Phase 05 was split into **05a (settings)** and **05b
(provenance)** at draft time — settings are the high-value, self-contained half and
fit one executor session; endpoint-reported provenance (response parsing + new
`PhaseRun` fields + the health path) is its own session:

- **04 — `rexymcp runs` (this).** A read-only CLI view that lists individual
  `PhaseRun` records (model, settings, gates, reliability/efficiency, verdict),
  filterable by model/tag, newest-first. The most direct "see detailed statistics
  for each run." Works on existing data.
- **05a — settings plumbing.** Make sampling settings (`temperature`/`seed`)
  configurable in `[executor]`, sent to the model on every chat request, and
  recorded with real values in `generation_params`. Today `generation_params` is
  always default `None` — this makes the "which settings" axis real. **This is the
  high-value, self-contained half** — settings are what *we* choose and vary, so
  they're the lever behind "which settings work best." Drafted 2026-06-02.
- **05b — chat-stream provenance** (currently parsed then discarded — see the AI
  client at `executor/src/ai/backends/openai.rs`): the **served model id** from the
  chat response `model` field (more accurate than the requested id; catches
  aliasing/fallback), and the **`finish_reason`** (especially the fraction of
  completions ending in `length` — a truncation / reliability signal alongside
  `parse_failure_rate`). Both ride the same `AiEvent::Done` extension, so they're
  one phase. Drafted 2026-06-02.
- **05c — context window** (`max_model_len` from `/v1/models`, captured via the
  health/models path — a separate source from the chat stream, hence its own
  phase). **Explicitly out of scope across 05b/05c: quantization / parameter count /
  weights revision** — the OpenAI-compatible API does not expose these portably
  (only the model-id string does, by naming convention), and provider-native probes
  (Ollama `/api/show`, etc.) would break the "any OpenAI endpoint" promise.
- **06 — settings slice on the scorecard (milestone closer).** A `rexymcp scorecard`
  CLI that aggregates runs into a `model × settings` matrix (same quality/reliability/
  efficiency means as the model × tag scorecard, plus `length_finish_rate`), so the
  user can see which settings work best for a model. Additive: a new
  `aggregate_by_settings` + CLI surface, leaving the existing model × tag
  `model_scorecard` MCP tool untouched. Depends on 05a/05b/05c's real data. Drafted
  2026-06-02.

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

---

## M7 retrospective (2026-06-02)

**Outcome.** The per-run statistics direction shipped end-to-end. After the
mid-milestone pivot away from benchmarking, M7 delivered six approved phases that
make every regular run measurable and comparable:

- **04** — `rexymcp runs` (per-run CLI view).
- **05a** — settings (temperature/seed) configurable → sent → recorded.
- **05b** — chat-stream provenance (served model id + `finish_reason`/length-truncation
  rate) via the additive `AiEvent::Completion` event.
- **05c** — endpoint context window (`max_model_len` from `/v1/models`), best-effort.
- **06** — `rexymcp scorecard` `model × settings` competency matrix.

A user can now see, per run and aggregated per (model, settings), the gates,
reliability (parse-failure + length-truncation), efficiency (turns/wall-clock), and
supervision (approved-first-try / bounces) signals — answering "which model and which
settings work best."

**What worked.**
- **Decomposition under pressure.** The original phase-05 was too large; splitting it
  into 05a/05b/05c at draft time (by data-source: settings / chat-stream / models
  endpoint) kept each phase to one executor session.
- **The architect/executor telemetry loop.** Verdicts and bounces were recorded as
  `PhaseRun` data, and the recurring failure class drove a real doc improvement (below).

**What broke, and the folds.**
- **Caller-cascade hard_fails (05a, 05b).** The executor (Qwen3.6-27B) twice hard-failed
  by making a breaking multi-site change (a function-signature change in 05a; an enum-
  variant *mutation* in 05b) and getting caught by the verifier's 3-strike limit before
  finishing the cascade. **Fold:** the WORKFLOW.md "Prefer additive change shapes"
  discipline (commit `e27e92c`) — prefer a new variant/field/function over mutating a
  symbol with wide blast radius, and pre-inject grep-verified site lists when a
  multi-site change is unavoidable. **Validated three times afterward:** 05b (additive
  `Completion` variant), 05c (three additive struct fields), and 06 (additive aggregation)
  all landed with zero cascade failures.
- **Bookkeeping drop-off (05a, bug-05a-1).** On one dispatch the executor produced
  correct, green code but skipped its end-of-phase commit + completion-log. Filed as a
  minor bounce; **did not recur** across 05b/05c/06 — stays at 1 occurrence (data, not a
  trend), no fold.
- **Backend stall (06).** The model server dropped the SSE connection mid-write
  (`hard_fail` `BackendError`). Recovered cleanly by re-dispatch (the executor resumed
  from the partial tree) — a live validation of the phase-01 `hard_fail`-degradation
  design. Infrastructure, not a quality issue.
- **Executor date hallucination (phase-04).** The executor stamped a wrong date in an
  Update Log; corrected at review. 1 occurrence, noted.

**Calibration ledger:** 1 fold landed (additive change shapes, user-approved). Two
watched items (bookkeeping drop-off; date hallucination) remain single occurrences —
no further folds warranted.

**Carried follow-up (for M8 kickoff / an architect pass).**
`docs/architecture.md`'s "Model effectiveness metrics & routing" section still carries
the **deprecated** "Benchmark vs. telemetry" + automated-"Routing" language and does not
yet describe the per-run-statistics direction, `rexymcp runs`, or the settings scorecard.
This needs an architect realignment pass (architecture.md is principal-engineer
territory, not executor-editable).
