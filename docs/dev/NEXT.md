# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — **M7 is being redirected** (benchmarking deprecated
2026-06-02). The next phase for the new direction (detailed per-run statistics +
scorecard over regular runs) has not been drafted yet; it is the next
`/rexymcp:architect` task, to be designed with the user. Do not dispatch.

**Direction change (2026-06-02).** The benchmark-suite approach is dropped. The
scorecard concept is **kept**, but it will track **regular rexyMCP runs**, not
specialized benchmark runs. New goal: let users see detailed statistics for each
rexyMCP run so they can decide which local LLM to use and which settings work
best for it. Phases **02 / 03a / 03b** were rolled back — benchmark code reverted
(`971d0c4` phase-03a, `dc5b6be` phase-02), the unlanded 03b sweep discarded, and
the three phase docs banner-marked `rolled-back`. The `bench_suite` field on
`PhaseRun`, the scorecard `SourceFilter`, the `LoopDeps`/CLI threading, and the
sweep are all gone; `PhaseRun` + scorecard are back to their post-phase-01 state.

**Open follow-ups for the redesign:**
- `docs/architecture.md` § "Model effectiveness metrics & routing" still carries
  the "Benchmark vs. telemetry" + automated-routing language — needs an architect
  pass to realign with the per-run-statistics direction.
- Pre-existing red tests unrelated to the rollback: `config.rs` commit `6282060`
  bumped `stream_idle_timeout_secs` default 90→180 but left
  `config_defaults_first_token_and_idle_timeouts` (`config.rs:309`) and
  `config_omits_timeouts_keeps_defaults` (`config.rs:365`) asserting `90`. Two
  failing tests; fix the asserts to `180` (or whatever final value) before the
  next phase is reviewed.

**M6 closed** via [phase-06b — dogfood execution + retrospective +
close](milestones/M6-plugin/phase-06b-dogfood-close.md). The ms_pacman dogfood
(bootstrap + design, 5/5, no dispatch) was user-confirmed sufficient; the two
breakages it surfaced (tools-not-advertised `b78a081`; live-progress-can't-fire
`c4567fb`+`3374336`) are fixed. Full retrospective in the
[M6 README Notes](milestones/M6-plugin/README.md#notes).

**Decisions carried into M7** (the 07a/07b deferrals + compaction, decided in
06b):

1. **Terminal backend `Err` → `hard_fail` (yes, conditional).** A mid-phase
   terminal model error (after ≥1 turn of progress) should degrade to a
   `hard_fail` `PhaseResult` with briefing + partial work, instead of aborting
   `execute_phase` as it does today (`executor/src/agent/mod.rs:238` and
   `:271-273`, with the `:1545` test pinning the current abort). Pre-work
   connection errors stay `Err`. **This is the one decision with a code
   follow-up — an M7-adjacent implementation phase, not yet drafted.**
2. **Resume / `continue_phase` (no).** Stays an uncommitted architecture
   candidate; re-dispatch-with-refined-spec remains the default. Revisit only if
   `PhaseRun` telemetry shows a recurring high-progress / single-blocker pattern.
3. **Compaction monitoring (insufficient data).** No dispatch → no
   `CompactionReport`; keep the heuristic compactor; gather data on the first
   small-context (32k–128k) dispatch. No summarization milestone justified.

**Architecture amended in 06b:** Layer 2 § Liveness reworded push→pull —
`rexymcp status` is the human-liveness path; MCP progress is spec-correct but
unreachable with Claude Code's current client.

**Already-landed calibration fold (recorded in 06b):** an earlier run hit
`budget_exceeded` at the turn cap mid-verification; default `max_turns` raised
40 → 200 in `executor/src/config.rs` and the architect bootstrap template
(`plugin/skills/architect/SKILL.md`), since the executor runs against a local
LLM with no token cost. Per-project `[budget] max_turns` was already
configurable; only the defaults moved.

**Last completed:** [M7 / phase-01](milestones/M7-scorecard/phase-01-backend-error-degradation.md)
— approved_first_try 2026-06-01. (phase-02/03a/03b rolled back 2026-06-02 —
benchmarking deprecated.)

**Milestone:** [M7 — Per-run statistics & model scorecard](milestones/M7-scorecard/README.md)
— in progress, redirecting (M1–M6 done; M7 phase-01 done; benchmarking dropped;
per-run statistics direction pending design).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.

---

**M5 retrospective + folds at a glance** (for the M6 kickoff briefing):

- Seven phases: 01 / 02 / 03 / 04 / 05a / 05b / 06. Six approved_first_try;
  one bounced once ([bug-05b-1](milestones/M5-mcp-server/bugs/bug-05b-1.md),
  verified). 629 total tests (started M5 at 492 executor + 0 mcp; ended at
  512 executor + 117 mcp).
- Six tools live: `execute_phase`, `executor_health`, `executor_log_search`,
  `executor_log_tail`, `get_turn`, `model_scorecard`. Plus the full progress
  consumer split (live MCP `notifications/progress` for the human + logged
  `Progress` events for Claude's post-return queries) and target-repo-root
  corroboration.
- Two calibration folds added to WORKFLOW.md: *Wrap-vs-derive at protocol
  boundaries* (extending `### Derive intentionally`) and *Anticipate
  cross-boundary trait bounds* (new subsection). Five-recurrence threshold
  reached on the latter.
