# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M5 / phase-06 — roots corroboration (M5
closer)](milestones/M5-mcp-server/phase-06-roots-corroboration.md) — `todo`,
**drafted, awaiting dispatch**. Before `execute_phase` does any work,
cross-checks the caller-supplied `repo_path` against (a) the MCP client's
`roots/list` and (b) `CLAUDE_PROJECT_DIR`. Hard refusal on mismatch;
permissive when both sources are absent (the M2 `Scope` remains the actual
security boundary — corroboration is a safety check against
misconfiguration). New pure `mcp/src/roots.rs` module + a shim in the
manual `ServerHandler::call_tool` for `execute_phase`. No `executor/`
edits, no new deps. **On approval, M5 closes** — milestone retrospective
+ the calibration folds queued across phases 02–05b (derive-vs-wrap rule,
cross-boundary trait bounds).

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
