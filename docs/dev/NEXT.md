# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-01 — plugin scaffold + `.mcp.json` +
slash-command stubs](milestones/M6-plugin/phase-01-plugin-scaffold.md) —
`todo`, **drafted, awaiting dispatch**. Top-level `plugin/` directory,
`.mcp.json` registering `rexymcp serve --config <path>` with a raised
per-tool timeout on `execute_phase` (toward the 10-minute ceiling), three
slash-command stubs (filled in by phase-04/05), and a `plugin/README.md`
orientation doc. No Rust code changes, no new deps. Pre-flight tells the
executor to verify Claude Code's actual plugin filesystem layout +
`.mcp.json` schema against the docs (same trust-docs-over-sketch
discipline as M5 phase-02 / 05b / 06).

**Last completed:** [M5 / phase-06 — roots
corroboration](milestones/M5-mcp-server/phase-06-roots-corroboration.md) —
approved_first_try 2026-05-31 (M5 closer; zero deviations).

**Milestone:** [M6 — Plugin + architect/review skills](milestones/M6-plugin/README.md)
— in progress (M1–M5 done). M6 packages rexyMCP as a Claude Code plugin
(skills + slash commands + embedded templates + bootstrap) and closes with
an end-to-end dogfood against a real third-party repo.

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.

---

**M5 retrospective + folds at a glance** (for the M6 kickoff briefing):

- Seven phases: 01 / 02 / 03 / 04 / 05a / 05b / 06. Six approved_first_try;
  one bounced once ([bug-05b-1](milestones/M5-mcp-server/bugs/bug-05b-1.md),
  verified). 629 total tests (started M5 at 492 executor + 0 mcp; ended at
  512 executor + 117 mcp).
- Six tools live: `execute_phase`, `executor_health`, `executor_log_search`,
  `executor_log_tail`, `get_turn`, `model_scorecard`. Plus the full progress
  consumer split (live MCP `notifications/progress` for the human + logged
  `Progress` events for Claude's post-return queries) and target-repo-root
  corroboration.
- Two calibration folds added to WORKFLOW.md: *Wrap-vs-derive at protocol
  boundaries* (extending `### Derive intentionally`) and *Anticipate
  cross-boundary trait bounds* (new subsection). Five-recurrence threshold
  reached on the latter.
