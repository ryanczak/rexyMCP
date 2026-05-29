# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — awaiting the user to draft or dispatch the next phase.
The next phase is **07 — the turn-cycle loop** (`execute_phase`); it is not yet
drafted. ("06b" was dropped, not deferred — its only real deliverable, the
native-event→`ToolCall` adapter, folds into phase-07; see M4 README § Notes.)

**Last completed:** [M4 / phase-06 — `PhaseResult` + briefing
contract](milestones/M4-agent-loop/phase-06-phase-result.md) — approved_first_try
2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–06 done; 07–08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
