# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-05b — progress MCP-notification consumer
(mcp side)](milestones/M5-mcp-server/phase-05b-progress-mcp-consumer.md) —
**in-progress** (bounced 2026-05-31; see
[bug-05b-1](milestones/M5-mcp-server/bugs/bug-05b-1.md)). Two items: a
`#[allow(clippy::too_many_arguments)]` on `run_phase` (hard-rule violation
— clean fix is the same struct-grouping pattern phase-01 used for
`run_phase_with`) and the two missing wrapper-level integration tests
(explicit acceptance criteria; opencode declared the skip with a rationale
that didn't hold). The functional code is sound and gates are clean (tests
610); manual `ServerHandler` impl handling rmcp 1.7's macro limitation is
the right architectural call (accepted, not bounced). **Re-dispatch to
opencode** to address bug-05b-1.

**Last completed:** [M5 / phase-05a — progress callback seam + Progress log
events (executor side)](milestones/M5-mcp-server/phase-05a-progress-callback.md)
— approved_first_try 2026-05-30 (zero deviations, 17 new tests, all
callback-contract pins held including the panicking-callback non-feature;
see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phases 01–04 + 05a done — six tools live + the
liveness producer side). Phase-05b (mcp consumer side) and phase-06 (roots
corroboration) remain.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
