# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-02 — embedded
templates](milestones/M6-plugin/phase-02-embedded-templates.md) —
**in-progress** (re-opened 2026-05-31 on deeper architect review; see
[bug-02-1](milestones/M6-plugin/bugs/bug-02-1.md)). The first-pass
approval stands on lexical/structural work; a follow-up read-through
caught four conceptual genericization gaps no grep could catch — three
rexyMCP-specific concepts (tools / agent-loop / parser-stage) in
STANDARDS §3.1, a donor-project mention in §2.6, a Rust-specific
cache-race warning in §4, and a missing `NEXT.md` in WORKFLOW's
Directory Layout (the executor contract reads it first but the workflow
doesn't say it exists). All four are content-only fixes (~30 lines).
**Re-dispatch to opencode** to address bug-02-1.

**Last completed:** [M6 / phase-02 — embedded
templates](milestones/M6-plugin/phase-02-embedded-templates.md) —
approved_first_try 2026-05-31 (928 lines; zero deviations; second
M6-phase grep-precision call-out from opencode — flagging not folding
yet).

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
