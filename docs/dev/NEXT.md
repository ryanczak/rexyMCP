# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-02 — context budget + compaction](milestones/M4-agent-loop/phase-02-context-budget.md)

**Status:** todo — to be dispatched to opencode. (Note: `compactor.rs` carries
`<tool_result>` literals; the spec's `[compacted: ` grep proof catches a corrupted
write, with the heredoc workaround / direct-execution fallback if it does.)

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01 done).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
