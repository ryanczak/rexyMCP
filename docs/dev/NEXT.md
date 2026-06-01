# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M6 / phase-06a — dogfood
preparation](milestones/M6-plugin/phase-06a-dogfood-prep.md) — `todo`,
**drafted, awaiting dispatch**. Opencode-executable prep for the M6
closer: write a step-by-step `dogfood-procedure.md` for the user to
follow + a `dogfood-log.md` template + run pre-flight verification
that the binary builds/installs/runs (`--help`) + the plugin JSON
files parse + the phases 02/04/05 validation greps still pass. No
Rust code; no new deps. Splits M6 phase-06 along the same boundary
M4-07 and M5-05 used: 06a is opencode prep, 06b is user-driven
dogfood run + architect-synthesized retrospective + compaction-
monitoring decision.

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
