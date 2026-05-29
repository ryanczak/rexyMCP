# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-04 — secret redaction
primitive](milestones/M4-agent-loop/phase-04-redaction.md) — drafted, `todo`.
Awaiting the user to dispatch.

**Routing:** **direct execution (Claude), not opencode.** The deliverable is
saturated with regex literals — escaped quotes, backslashes, PEM/`<tool_call>`-
shaped text inside Rust raw strings — exactly the content class that corrupts
opencode's tool-call serializer (WORKFLOW § "Route opencode-hostile content to
direct execution"; same call as the M3 parser and `bash_classify`). The phase doc
still follows the normal review gate; only the executor changes.

**Last completed:** [M4 / phase-03 — JSONL session log](milestones/M4-agent-loop/phase-03-session-log.md)
— approved_first_try 2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01, phase-02, phase-03 done).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
