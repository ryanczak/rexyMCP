# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-03 closed (`done`, approved_first_try on
2026-05-30, the cleanest M5 phase yet: zero `executor/` edits, zero new deps,
zero scope deviations). The three log-query tools are live: Claude can now
grep / tail / drill into a phase's JSONL session log on demand without
re-flooding context. Next step is the architect drafting **M5 phase-04 —
`model_scorecard`** (aggregate `store::telemetry::read` into the model × tag
competency matrix; read-only over the cross-project store).

**Last completed:** [M5 / phase-03 — session-log query
tools](milestones/M5-mcp-server/phase-03-log-query.md) — approved_first_try
2026-05-30 (zero deviations, 41 new tests, architecture-mandated `get_turn`
uncapped-escape-hatch property tested directly; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phases 01–03 done). M5 wraps the M4 executor in an
`rmcp` stdio server; server scaffold + execute_phase + executor_health +
log-query tools are in place. `model_scorecard` + progress + roots remain
(phases 04–06).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
