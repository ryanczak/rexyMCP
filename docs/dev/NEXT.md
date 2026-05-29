# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M2 phase-06 approved (`done`). **All six M2 phases are done.**

M2 is at its **milestone-close gate**: per WORKFLOW.md § "Phase progression &
triggers", this stops for human sign-off — the retrospective and any doc-folding
(calibration) are human judgment, not automated. Do not advance to M3 until the
project owner signs off on M2 and a phase is drafted.

**Milestone:** [M2 — Executor tools & security](milestones/M2-executor-tools/README.md) — review (all phases done; awaiting sign-off).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
