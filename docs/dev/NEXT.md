# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M2 / phase-06 — 2-stage tool router primitive](milestones/M2-executor-tools/phase-06-router.md)

**Status:** todo

**Milestone:** [M2 — Executor tools & security](milestones/M2-executor-tools/README.md) — in-progress (phase-01 through phase-05 done; phase-06 is the **last** — its approval closes M2, a human gate).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
