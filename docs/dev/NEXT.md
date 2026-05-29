# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M4 phase-05 approved (`done`). Awaiting the user to
advance: `/architect next` to draft phase-06 (`PhaseResult` + briefing contract),
or `/dispatch 06` once written.

**Last completed:** [M4 / phase-05 — governor: tool scorer + hard-fail
detector](milestones/M4-agent-loop/phase-05-governor.md) — approved_first_try
2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–05 done; 06–08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
