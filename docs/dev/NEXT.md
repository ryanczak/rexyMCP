# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — M6 phase-05 closed (`done`,
approved_first_try on 2026-05-31). The three skills (dispatch, review,
escalate) are live; the escalation decision tree's pre-injected prose
preserved with voice + framing intact. Architect-side fix included to
align status-acceptance across dispatch/review/escalate at the
re-dispatch path. **M6 is now 5/6 done — only phase-06 (end-to-end
dogfood, M6 closer) remains.** The dogfood will produce the first real
`PhaseRun` telemetry and surface the compaction-monitoring data the M6
README phase-06 description pinned.

**Last completed:** [M6 / phase-05 — dispatch + review + escalate
skills](milestones/M6-plugin/phase-05-dispatch-review-escalate.md) —
approved_first_try 2026-05-31 (one declared deviation: review skill's
cargo-literal examples genericized to `<command from rexymcp.toml>`
placeholders; architect-side status-acceptance alignment fix
co-committed).

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
