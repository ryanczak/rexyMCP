# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-01 — post-edit verifier + `Diagnostic`](milestones/M4-agent-loop/phase-01-verifier.md)

**Status:** todo

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (M1, M2, M3 done; signed off 2026-05-28).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
