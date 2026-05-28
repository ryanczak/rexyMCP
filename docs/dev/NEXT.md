# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — **milestone boundary.**

M1 — Foundations is complete (phase-01, -02, -03 all `done`; live E2E verified).
Per WORKFLOW.md § "Phase progression & triggers", milestone boundaries stop for
project-owner sign-off, and M2's phases are **expanded on demand**, not in
advance. There is no active phase to execute until the architect drafts M2
phase-01.

**Milestone:** [M1 — Foundations](milestones/M1-foundations/README.md) — awaiting
sign-off.

**Next milestone:** M2 — Executor tools & security (see `docs/architecture.md`
Status §). Not yet expanded into phases.

---

Executor: do not pick up work while this says "milestone boundary." If you were
dispatched here, stop and wait for the architect to point this file at an M2
phase doc.
