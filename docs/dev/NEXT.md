# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-01 — phase-runner wiring (config + phase doc →
`execute_phase`)](milestones/M5-mcp-server/phase-01-phase-runner.md) — `todo`,
**drafted, awaiting dispatch**. Net-new composition root in `mcp/src/runner.rs`
(phase-doc parser + registry builder + the `run_phase` assembler) plus a
`run-phase` CLI subcommand; no new dependency (`rmcp` is phase-02). Unit-testable
with `MockAiClient`.

**Last completed:** [M4 / phase-08 — `PhaseRun`
telemetry](milestones/M4-agent-loop/phase-08-phaserun-telemetry.md) —
approved_first_try 2026-05-29 (M4 closer; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done). M5 wraps the M4 executor in an `rmcp` stdio server;
phase-01 is the composition-root leaf the server's tools sit on.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
