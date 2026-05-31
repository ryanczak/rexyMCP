# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — **M5 is closed** (all seven phases `done`,
retrospective signed off 2026-05-31, two calibration folds applied to
WORKFLOW.md). At a **milestone boundary**: the next step is the human
kicking off **M6 — Plugin + architect/review skills** (Claude Code plugin
manifest, the `architect` / `review-phase` / `escalate` skills, slash
commands, embedded `STANDARDS.md` + `WORKFLOW.md` + executor contract,
project initialization bootstrap, dogfood against a real repo). The
architect expands M6 into phases on demand; nothing is drafted yet.

**Last completed:** [M5 / phase-06 — roots
corroboration](milestones/M5-mcp-server/phase-06-roots-corroboration.md) —
approved_first_try 2026-05-31 (M5 closer; zero deviations, 17 new tests
covering all match/mismatch/no-source/URI-parsing/symlink edges; see phase
doc Review verdict).

**Milestone:** [M5 — MCP server](milestones/M5-mcp-server/README.md) —
**done**. M1–M5 complete; M6 (Plugin + architect/review skills) is next,
awaiting human kickoff.

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
