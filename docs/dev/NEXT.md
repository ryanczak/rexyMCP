# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-04 — `model_scorecard` (model × tag competency
matrix)](milestones/M5-mcp-server/phase-04-model-scorecard.md) — `todo`,
**drafted, awaiting dispatch**. Aggregates the cross-project `phase_runs.jsonl`
into the (model, tag) competency matrix — gates pass rate, reliability means
(parse-failure / repairs / tool-success / verifier-retries), efficiency
(turns / wall-clock), escalation rate, and (where the architect has filled
them) supervision metrics (`approved_first_try_rate`,
`bounces_to_approval_mean`). Read-only; routing is M7. **`ScorecardRow`
derives `JsonSchema` directly** — small mcp-owned schema tree, per the
matured derive-vs-wrap rule (phases 02–03). No new deps, no `executor/` edits.

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
