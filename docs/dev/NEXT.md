# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M7 / phase-03b — `rexymcp bench` multi-model sweep + one
minimal fixture](milestones/M7-scorecard/phase-03b-bench-sweep.md)
(`todo` — ready to dispatch).

phase-01 (terminal `Err` → `hard_fail`) is `done` (approved_first_try
2026-06-01). phase-02 (benchmark provenance + scorecard `SourceFilter`) is
`done` (approved_after_1 2026-06-01). phase-03a (thread `bench_suite` through
`LoopDeps`/`emit_phase_run` + a `run-phase --bench-suite` flag, the stamped
single-run primitive) is `done` (approved_after_1 2026-06-02; one minor bounce
on a prohibited `#[allow]`, fixed via the `RunFullArgs` grouping idiom).

**phase-03b in one line:** add a `rexymcp bench` command that runs a benchmark
suite (reference phase docs against a frozen fixture) across one or more
`--model`s, copying the pristine fixture into a fresh `TempDir` per run and
emitting a `bench_suite`-stamped `PhaseRun` for each. Ships the sweep engine +
**one** minimal `smoke` fixture (a standalone cargo crate + one trivial phase);
the curated breadth is phase-03c. Authorizes two scoped `Cargo.toml` edits
(`exclude = ["benchmarks"]` on the workspace; `tempfile` dev→prod in mcp).

**phase-03 split (decided 2026-06-01/02 with the user):** 03a = the
stamp/threading primitive (done). 03b = the sweep engine + one fixture (`--model`
repeatable CLI flag; copy-into-`TempDir`-per-run). 03c = the curated breadth
(language × kind × size matrix + per-suite command config), drafted after 03b.

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

**Last completed:** [M7 / phase-03a](milestones/M7-scorecard/phase-03a-bench-suite-threading.md)
— approved_after_1 2026-06-02 (one minor `#[allow]` bounce, fixed via grouping).

**Milestone:** [M7 — Model scorecard & routing](milestones/M7-scorecard/README.md)
— in progress (M1–M6 done; M7 phase-01/02/03a done; phase-03b in `todo`).

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
