# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M4 phase-04 approved (`done`). Awaiting the user to
advance: `/architect next` to draft phase-05 (governor: scorer + hard-fail
detector), or `/dispatch 05` once written.

**Last completed:** [M4 / phase-04 — secret redaction
primitive](milestones/M4-agent-loop/phase-04-redaction.md) — approved_first_try
2026-05-29, direct execution (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–04 done; 05–08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
