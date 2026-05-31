# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M5 phase-05b closed (`done`, approved_after_1 on
2026-05-31 after bug-05b-1 fixed). Both items resolved cleanly: `#[allow]`
gone, replaced with `RunPhaseConfig<'a>` struct (the same pattern phase-01
used for `Seams`/`AssemblyInput`); the two wrapper-level integration tests
landed via the Option-A seam refactor (`execute_phase_inner_with_client`).
**Phase-05 closes** — the full consumer split is live: human watches MCP
`notifications/progress` (05b), Claude queries logged `Progress` events
via `executor_log_search` (05a). Next step is the architect drafting **M5
phase-06 — roots corroboration** (the M5 closer: `roots/list` +
`CLAUDE_PROJECT_DIR` cross-checked against `execute_phase`'s `repo_path`).

**Last completed:** [M5 / phase-05b — progress MCP-notification consumer
(mcp side)](milestones/M5-mcp-server/phase-05b-progress-mcp-consumer.md) —
approved_after_1 2026-05-31 (one bounce:
[bug-05b-1](milestones/M5-mcp-server/bugs/bug-05b-1.md), verified — `#[allow]`
removed via `RunPhaseConfig` struct, wrapper-level tests landed via Option-A
seam refactor; see phase doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) — in
progress (M1–M4 done; M5 phases 01–05 done — six tools live + full
progress consumer split). Phase-06 (roots corroboration) remains as the
M5 closer.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.
