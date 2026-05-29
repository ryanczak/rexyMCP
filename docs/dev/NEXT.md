# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-03 — JSONL session log (writer/reader + event schema)](milestones/M4-agent-loop/phase-03-session-log.md)

**Status:** todo — to be dispatched to opencode. (Writer is a clean lift; the
event schema is net-new, reusing M3 `ToolCall`/`ParseFailure` + `Diagnostic` and
reserving the `Progress` variant. Redaction is split out to phase-04.)

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01, phase-02 done).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
