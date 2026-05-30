# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-03 — session-log query tools
(`executor_log_search` / `executor_log_tail` /
`get_turn`)](milestones/M5-mcp-server/phase-03-log-query.md) — `todo`,
**drafted, awaiting dispatch**. The pull-not-push half of the MCP boundary:
three tools that let Claude grep / tail / drill into a phase's JSONL session
log on demand. `executor_log_search` + `executor_log_tail` cap each record;
`get_turn` is architecture-mandated as the one uncapped escape hatch (single
turn only). No new deps, no `executor/` edits — wraps records in
`LogQueryOutput { records: serde_json::Value }` to sidestep `JsonSchema`
cascading across `SessionEvent` (same trade-off as phase-02's
`ExecutePhaseOutput`). Mandates handler success-path tests per the phase-02
calibration carry-forward.

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
