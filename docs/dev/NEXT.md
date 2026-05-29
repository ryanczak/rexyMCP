# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-05 — governor: tool scorer + hard-fail
detector](milestones/M4-agent-loop/phase-05-governor.md) — drafted, `todo`.
Awaiting the user to dispatch.

**Routing:** **opencode (normal dispatch).** Unlike phase-04, this deliverable is
ordinary Rust (HashMap counters, structs, threshold checks) — the adaptation
*removes* Rexy's `<tool_result>`/`[verifier]` string-scraping, so the source
carries no close-tag-shaped literals or escaped-quote-in-JSON content. Not
opencode-hostile.

**Last completed:** [M4 / phase-04 — secret redaction
primitive](milestones/M4-agent-loop/phase-04-redaction.md) — approved_first_try
2026-05-29, direct execution (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–04 done; 05–08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
