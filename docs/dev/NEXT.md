# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-04 — `architect` skill + bootstrap
routine](milestones/M6-plugin/phase-04-architect-skill.md) — `todo`,
**drafted, awaiting dispatch**. The heaviest content phase in M6: fully
fills the phase-01 stub at `plugin/skills/architect/SKILL.md` with the
complete architect prompt covering seven responsibilities — frontmatter
+ bootstrap routine (idempotent, four architecture-mandated steps),
explore-then-design, phase-doc authoring against the embedded WORKFLOW
templates, **pre-injection** (the load-bearing concept with five named
injection types), status management (NEXT.md / phase doc / milestone
README in sync + Review verdict on every approval), and the explicit
list of things the architect doesn't do. No Rust code changes; no new
deps. Pre-flight 3 verifies Claude Code's skill frontmatter format +
plugin install dir path + arg-passing convention (same trust-docs-over-
sketch discipline as M5 / M6 phase-01).

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
