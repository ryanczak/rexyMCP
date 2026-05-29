# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-07d —
read-before-edit](milestones/M4-agent-loop/phase-07d-read-before-edit.md) —
drafted, `todo`. Awaiting the user to dispatch. (Safety invariant: refuse a
`patch` on an unread / changed-underneath file; pure gate + working-set. 07e
=completion artifacts is the last 07 sub-phase.)

**Routing:** **direct execution (Claude), not opencode.** Same opencode-hostile
content class as 07a–07c — `MockAiClientScript` `<tool_call>`/JSON-in-Rust
fixtures (WORKFLOW § "Route opencode-hostile content"). Pre-route to direct
execution; normal review gate.

**Last completed:** [M4 / phase-07c — verifier retry +
hard-fail](milestones/M4-agent-loop/phase-07c-verifier-hardfail.md) —
approved_first_try 2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–06 + 07a–07c done; 07d–07e + 08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
