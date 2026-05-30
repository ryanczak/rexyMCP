# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-01 closed (`done`, approved_after_1 on
2026-05-29 after bug-01-1 fixed). Next step is the architect drafting **M5
phase-02 — rmcp server scaffold + `execute_phase` + `executor_health`** (the
first phase to authorize the `rmcp` dependency).

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
