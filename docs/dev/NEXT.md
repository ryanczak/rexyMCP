# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M3 / phase-01 — parser core types + `strip_think_blocks` + `detect`](milestones/M3-forgiving-parser/phase-01-types-and-detect.md)

**Status:** review — implemented by Claude Code (opencode's serializer corrupted the parser source); awaiting architect sign-off.

**Milestone:** [M3 — Forgiving parser](milestones/M3-forgiving-parser/README.md) — in-progress (M2 signed off 2026-05-28).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
