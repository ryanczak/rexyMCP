# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-05a closed (`done`, approved_first_try on
2026-05-30; third consecutive zero-deviation phase, *and* the first M5 phase
with substantive `executor/` edits — discipline held through the harder
case). The progress-callback seam is live on `LoopDeps`, four emission sites
fire in the turn cycle, and each emission writes a `SessionEvent::Progress`
record to the JSONL log. The **durable half** of the architecture's consumer
split is already working: Claude can query logged progress via
`executor_log_search { event_type: "progress" }` now. Next step is the
architect drafting **M5 phase-05b — mcp notification consumer side** (the
`execute_phase` handler builds a `ProgressCallback` that emits MCP
`notifications/progress`, plumbed through `runner::run_phase`).

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
