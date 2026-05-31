# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-05b — progress MCP-notification consumer
(mcp side)](milestones/M5-mcp-server/phase-05b-progress-mcp-consumer.md) —
`todo`, **drafted, awaiting dispatch**. Threads `Option<&dyn ProgressCallback>`
through `runner::run_phase`/`run_phase_with` to `LoopDeps.progress`; adds a
`McpProgressNotifier` that implements `ProgressCallback` by
`tokio::spawn`-ing an async send to the rmcp peer's
`notifications/progress`; modifies the `#[rmcp::tool] execute_phase` method
to extract the progress token from `_meta.progressToken` and build the
notifier (none = no token, `None` callback). No `executor/` edits — 05a's
contract is sufficient. No new deps. Pre-flight tells opencode to verify
rmcp 1.7's exact peer/notify API against `cargo doc` (same discipline as
phase-02).

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
