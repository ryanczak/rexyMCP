# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M1 / phase-01 — Workspace + config + error model](milestones/M1-foundations/phase-01-workspace-and-config.md)

**Status:** todo

**Milestone:** [M1 — Foundations](milestones/M1-foundations/README.md)

---

If the linked phase doc's status is `done` or `review`, this pointer is **stale**
— file a blocker and stop rather than guessing the next phase. The architect
updates this file when a phase is approved and the next one is ready to dispatch.
