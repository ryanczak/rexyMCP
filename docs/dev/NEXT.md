# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M3 / phase-05 — feedback formatter + `parse()` orchestration](milestones/M3-forgiving-parser/phase-05-feedback-parse.md)

**Status:** review — implemented by Claude Code; awaiting architect sign-off. (phase-01 through phase-04 approved/`done`.) **Last M3 phase — approval is a milestone-boundary human gate.**

**Milestone:** [M3 — Forgiving parser](milestones/M3-forgiving-parser/README.md) — in-progress (M2 signed off 2026-05-28).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
