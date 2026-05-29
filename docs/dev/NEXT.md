# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — **M4 is closed** (all phases `done`, retrospective signed
off 2026-05-29). At a **milestone boundary**: the next step is the human kicking
off **M5 — MCP server** (the `rmcp` stdio server exposing `execute_phase` +
`executor_health` + the log-query tools, plus telemetry-path resolution). The
architect expands M5 into phases on demand; nothing is drafted yet.

**Last completed:** [M4 / phase-08 — `PhaseRun`
telemetry](milestones/M4-agent-loop/phase-08-phaserun-telemetry.md) —
approved_first_try 2026-05-29 (M4 closer; see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md)
— **done**. M1–M4 complete; M5 (MCP server) is next, awaiting human kickoff.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
