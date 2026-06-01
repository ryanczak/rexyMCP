# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M6 phase-03 closed (`done`, approved_first_try
on 2026-05-31; textbook clean execution, all 6 prescribed tests in the
exact shape spec'd, cross-cutting `executor_contract` drop surgical).
The contract is now truly embedded-only: the executor crate
`include_str!`s phase-02's template, substitutes the four `{...}_COMMAND`
placeholders at every turn-cycle step 1, and the MCP server no longer
passes an empty-string placeholder. Next step is the architect drafting
**M6 phase-04 — `architect` skill + bootstrap routine** (the heaviest
content phase: full skill prompt covering explore-then-design, design-doc
/ milestone-README / phase-doc authoring, pre-injection, plus the
four-step bootstrap routine).

**Last completed:** [M6 / phase-03 — executor wires the embedded
contract](milestones/M6-plugin/phase-03-executor-wires-contract.md) —
approved_first_try 2026-05-31 (zero deviations; 6 new tests including
the future-proofing `placeholder_set_is_exactly_the_four_authorized`
that scans the embedded template for unauthorized `{...}` substrings).

**Milestone:** [M6 — Plugin + architect/review skills](milestones/M6-plugin/README.md)
— in progress (M1–M5 done; M6 phases 01–02 done). M6 packages rexyMCP
as a Claude Code plugin; scaffold + templates live, executor wiring +
skills + bootstrap + dogfood remain.

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
