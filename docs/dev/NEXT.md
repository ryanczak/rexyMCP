# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-04 closed (`done`, approved_first_try on
2026-05-30; second consecutive zero-deviation phase). The `model_scorecard`
tool is live: the (model, tag) competency matrix is queryable on demand.
Next step is the architect drafting **M5 phase-05 — progress notifications
+ `Progress` log events** (the only remaining M5 phase that requires an
executor change — a progress-callback seam in the loop, since the M4 turn
cycle has no progress hook today; `SessionEvent::Progress` + `FileNumstat`
were reserved in M4 phase-03 for exactly this).

**Last completed:** [M5 / phase-04 — `model_scorecard` (model × tag
competency matrix)](milestones/M5-mcp-server/phase-04-model-scorecard.md) —
approved_first_try 2026-05-30 (zero deviations, 25 new tests, all
Adaptation 3 negative cases pinned, derive-vs-wrap rule fully exercised
across M5; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phases 01–04 done — five tools live). M5 wraps the
M4 executor in an `rmcp` stdio server; `execute_phase`, `executor_health`,
`executor_log_search`, `executor_log_tail`, `get_turn`, `model_scorecard` all
shipped. Progress + roots remain (phases 05–06).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
