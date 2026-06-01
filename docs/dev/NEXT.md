# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-03 — executor wires the embedded
contract](milestones/M6-plugin/phase-03-executor-wires-contract.md) —
`todo`, **drafted, awaiting dispatch**. The only Rust edit in M6. New
`executor/src/agent/contract.rs` with `include_str!` of phase-02's
template + `assemble_executor_contract(commands)` substitutor; signature
change to `assemble_system_prompt(commands, standards, phase_doc)` (drops
the `executor_contract` parameter); cross-cutting drop of the
`executor_contract` field/plumbing from `PhaseInput`, `RunPhaseConfig`,
`execute_phase_inner`, the CLI handler, and all 13 current refs. After
this, the contract is *truly* embedded-only — the MCP server no longer
passes an empty-string placeholder. No new deps; pure stdlib
(`include_str!` + `str::replace`).

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
