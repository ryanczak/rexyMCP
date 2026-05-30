# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-02 closed (`done`, approved_first_try on
2026-05-30). The `rmcp` 1.7 stdio MCP server is live (`rexymcp serve`), the
two core tools are wired (`execute_phase` + `executor_health`), output
capping is in place, and `[telemetry] dir` config plumbs through. Next step
is the architect drafting **M5 phase-03 — session-log query tools**
(`executor_log_search` / `executor_log_tail` / `get_turn` over
`store::sessions::read_session_log`, each capping its own output).

**Last completed:** [M5 / phase-02 — rmcp server scaffold + execute_phase +
executor_health](milestones/M5-mcp-server/phase-02-rmcp-scaffold.md) —
approved_first_try 2026-05-30 (six declared scope deviations, all
defensible; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phases 01–02 done). M5 wraps the M4 executor in an
`rmcp` stdio server; the server scaffold + two core tools are in place,
log-query tools + scorecard + progress + roots remain (phases 03–06).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
