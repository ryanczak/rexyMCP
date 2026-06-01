# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M6 phase-04 closed (`done`, approved_first_try
on 2026-05-31). The 430-line architect SKILL.md is live: bootstrap
routine + explore-then-design + phase-doc authoring + pre-injection
guidance + status management + prohibitions. **The pre-injection
experiment worked** — three architect-pre-injected sections (pitfalls,
pre-injection, prohibitions) landed in the architect's voice with
specific examples preserved; opencode wrote the surrounding structure
cleanly. Meta-consistency: pre-injection used to write the prompt that
teaches pre-injection. Next step is the architect drafting **M6
phase-05 — `dispatch` + `review` + `escalate` skills** (smaller
content phase building on phase-04's pattern; the `/rexymcp:dispatch`
skill invokes `execute_phase`, `/rexymcp:review` reads `PhaseResult`
and approves/files-bug, `/rexymcp:escalate` handles the hard-fail
briefing levers).

**Last completed:** [M6 / phase-04 — architect skill + bootstrap
routine](milestones/M6-plugin/phase-04-architect-skill.md) —
approved_first_try 2026-05-31 (three declared deviations all
defensible: `/rexymcp:` namespacing, Claude-native allowed-tools
syntax with MCP tools via .mcp.json, opencode-harness-safe substitution
of literal tag syntax in pre-injection §4 example).

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
