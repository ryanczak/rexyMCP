# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-07b — session
log](milestones/M4-agent-loop/phase-07b-session-log.md) — drafted, `todo`.
Awaiting the user to dispatch. (Second of the 07a–07d split; weaves the redacted
JSONL session log through the 07a turn loop.)

**Routing:** **direct execution (Claude), not opencode.** Same opencode-hostile
content class as 07a / the M3 parser phases — the tests reuse `MockAiClientScript`
with `<tool_call>`/JSON-in-Rust fixtures and plant secret literals (WORKFLOW §
"Route opencode-hostile content"). Pre-route to direct execution; normal review
gate.

**Last completed:** [M4 / phase-07a — turn-loop
core](milestones/M4-agent-loop/phase-07a-loop-core.md) — approved_first_try
2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–06 + 07a done; 07b–07d + 08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
