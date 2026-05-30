# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-02 — rmcp server scaffold + `execute_phase` +
`executor_health`](milestones/M5-mcp-server/phase-02-rmcp-scaffold.md) — `todo`,
**drafted, awaiting dispatch**. Stands up the `rmcp` 1.7 stdio MCP server in
`mcp/`, registers the two core tools (the `execute_phase` handler wraps
phase-01's `run_phase`; `executor_health` wraps `health::check`), adds an
output-capping pass (`MAX_FIELD_BYTES`) so a phase's inner transcript never
floods Claude's context, and resolves the cross-project telemetry dir as a real
`[telemetry] dir` config field. First phase to authorize new deps (`rmcp`,
`schemars`).

**Last completed:** [M5 / phase-01 — phase-runner
wiring](milestones/M5-mcp-server/phase-01-phase-runner.md) — approved_after_1
2026-05-29 (one bounce: [bug-01-1](milestones/M5-mcp-server/bugs/bug-01-1.md),
verified; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phase-01 done). M5 wraps the M4 executor in an `rmcp`
stdio server; the composition-root leaf is in place, the server scaffold is
next.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
