# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-02 — embedded templates: `executor_contract` +
`STANDARDS` + `WORKFLOW`](milestones/M6-plugin/phase-02-embedded-templates.md)
— `todo`, **drafted, awaiting dispatch**. Three generalized Markdown
templates derived from this repo's `AGENTS.md` / `STANDARDS.md` /
`WORKFLOW.md`. `executor_contract.md` lives in `executor/templates/` for
phase-03's `include_str!`; `STANDARDS.md` + `WORKFLOW.md` live in
`plugin/templates/` for bootstrap to copy into target repos. All three
use `{FORMAT_COMMAND}` / `{BUILD_COMMAND}` / `{LINT_COMMAND}` /
`{TEST_COMMAND}` literal placeholders. Strict grep validation (precise
patterns this time per phase-01's calibration note — no false positives
on "RexyMCP" as product name). Sizable content phase (~900 lines) but
no Rust code changes.

**Last completed:** [M6 / phase-01 — plugin scaffold + `.mcp.json` +
slash-command stubs](milestones/M6-plugin/phase-01-plugin-scaffold.md) —
approved_first_try 2026-05-31 (three pre-flight 3 wins all declared
upfront; one architect-side spec-calibration note about over-broad grep
patterns; see phase doc Review verdict).

**Milestone:** [M6 — Plugin + architect/review skills](milestones/M6-plugin/README.md)
— in progress (M1–M5 done; M6 phase-01 done). M6 packages rexyMCP as a
Claude Code plugin; scaffold live, embedded templates + skills + bootstrap
+ dogfood remain.

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
