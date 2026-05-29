# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-03 — JSONL session log (writer/reader + event schema)](milestones/M4-agent-loop/phase-03-session-log.md)

**Status:** in-progress — opencode filed a blocker (`SessionEvent` needs
`Deserialize`, but the embedded M3 parser types only derive `Serialize`).
**Resolved by the architect:** authorized adding `Deserialize` to the six parser
types (see phase doc § "Deserialize round-trip" + Update Log). Re-dispatch to
opencode to resume.

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01, phase-02 done).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
