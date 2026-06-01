# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M6 phase-02 closed (`done`, approved_after_1 on
2026-05-31 after bug-02-1 fixed). All four content fixes applied (§3.1
universal-phrasing replacements, §2.6 donor cut, §4 cargo cache-race cut,
WORKFLOW Directory Layout `NEXT.md` added with explanation). Templates
are now genuinely portable. Next step is the architect drafting **M6
phase-03 — executor wires embedded contract** (small Rust edit:
`include_str!` the contract template, add
`assemble_executor_contract(cfg)` helper, modify the loop's turn-cycle
step 1 to use it).

**Last completed:** [M6 / phase-02 — embedded
templates](milestones/M6-plugin/phase-02-embedded-templates.md) —
approved_after_1 2026-05-31 (one bounce:
[bug-02-1](milestones/M6-plugin/bugs/bug-02-1.md), verified — four
conceptual genericization gaps the grep validations couldn't catch;
fixes applied cleanly).

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
