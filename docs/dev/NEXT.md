# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-05a — progress callback seam + `Progress` log
events (executor side)](milestones/M5-mcp-server/phase-05a-progress-callback.md)
— `todo`, **drafted, awaiting dispatch**. The only M5 phase that touches
`executor/` substantively. Adds `progress: Option<&'a dyn ProgressCallback>`
to `LoopDeps`, four emission sites in the turn cycle (`turn_start` /
`tool:<name>` / `verify` / `command:<name>`), a numstat helper reusing the
existing working-set + `similar` machinery, and logs each emission as a
`SessionEvent::Progress` record (the durable half of the consumer split). M5
phase-05 split into 05a+05b along the cohesive seam (executor producer vs
mcp consumer) — 05b drafts when 05a lands.

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
