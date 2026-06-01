# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M6 phase-06a closed (`done`, approved_first_try
on 2026-05-31). The dogfood-procedure.md (287L, 7 phases with
Do/Observe/Record/Stop-and-tell) and dogfood-log.md (117L with
dedicated Compaction observations section) are ready; pre-flight
verification clean (binary installs + runs, JSON files parse, 1937L
of content artifacts, zero leaks). **The ball is now in the user's
court** — phase-06a's prep is complete; the dogfood RUN happens
whenever the user has the time + a local LLM endpoint to point at.
**M6 phase-06b** drafts after the user has captured observations in
the log; 06b's job is the architect-synthesized M6 retrospective +
the compaction-monitoring decision + any calibration folds the
dogfood surfaces.

**Last completed:** [M6 / phase-06a — dogfood
preparation](milestones/M6-plugin/phase-06a-dogfood-prep.md) —
approved_first_try 2026-05-31 (zero deviations; thorough pre-flight
verification; Pre-flight 3 finding clean — no divergence from
architect's sketch of Claude Code plugin layout).

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
