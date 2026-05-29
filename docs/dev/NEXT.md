# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M2 phase-04 approved (`done`, after one bounce); phase-05 not yet drafted.

Per WORKFLOW.md § "Phase progression & triggers" (gated by default), the
architect drafts the next phase on demand. M2 phase-05 (`bash` tool +
`bash_classify` block/confirm lists + the capabilities/audit layer) has not been
written yet — the project owner advances with `/architect next` (or asks the
architect to draft it).

**Milestone:** [M2 — Executor tools & security](milestones/M2-executor-tools/README.md) — in-progress (phase-01 through phase-04 done).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
