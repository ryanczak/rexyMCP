# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** none — both dogfood-surfaced resilience fixes are now `done`:
[07a — SSE prefill-stall](milestones/M6-plugin/phase-07a-sse-prefill-stall.md)
(approved_after_1; [bug-07a-1](milestones/M6-plugin/bugs/bug-07a-1.md) verified)
and [07b — executor liveness `awaiting_model`
heartbeat](milestones/M6-plugin/phase-07b-executor-liveness-signal.md)
(approved_after_1; [bug-07b-1](milestones/M6-plugin/bugs/bug-07b-1.md) verified).
**The ball is back in the user's court for the M6 dogfood RUN** — there is no
executable phase for opencode to pick up. 06b drafts after the dogfood log is
captured.

- **07a** (SSE prefill-stall): the uniform 90 s `STREAM_CHUNK_TIMEOUT` judged
  first-token prefill latency by the same budget as inter-token gaps. Split into
  configurable first-token (600 s) and idle (90 s) budgets with a bounded
  pre-token retry. Bounced once (retry/timeout logic tested via a `#[cfg(test)]`
  duplicate, not the shipping path); fixed by extracting the decision fns into
  production and testing them directly incl. the keep-alive negative.
- **07b** (liveness heartbeat): emit `awaiting_model` before and every 15 s
  during the model wait so `rexymcp status` distinguishes a busy prefill from a
  hang. Bounced once (heartbeat tests used real `sleep` — architect spec gap:
  mandated `pause()`/`advance()` without authorizing tokio `test-util`); fixed
  with a dev-only `test-util` + virtual-time tests.

**Two contract/error-model questions 07a/07b deliberately do NOT decide
(deferred to 06b retrospective):** (1) whether a terminal backend `Err`
should degrade to a structured `hard_fail` `PhaseResult` (preserving partial
work) rather than abort `execute_phase`; (2) whether a cross-dispatch resume /
"continue if phase status ≠ done" mechanism is warranted.

**Still pending the user (separate from 07a/07b):** the M6 dogfood RUN —
phase-06a's prep is complete (dogfood-procedure.md, dogfood-log.md, clean
pre-flight). **M6 phase-06b** drafts after the user has captured observations
in the log; 06b's job is the architect-synthesized M6 retrospective + the
compaction-monitoring decision + any calibration folds the dogfood surfaces
(including the 07a/07b deferrals above).

**Calibration fold already landed (for 06b to record):** the first
dogfood run hit `budget_exceeded` at turn 40 mid-verification (a
scaffold phase — the last turn ran `prettier --check` green, then the
turn cap tripped). Default `max_turns` raised 40 → 200 in
`executor/src/config.rs` and the architect bootstrap template
(`plugin/skills/architect/SKILL.md`), since the executor runs against
a local LLM with no token cost. The cap was already per-project
configurable via `[budget] max_turns`; only the defaults moved. 06b
should fold this into the retrospective rather than re-derive it.

**Dogfood finding for 06b (live progress is a push that can't fire):**
the first run surfaced **zero** live MCP `notifications/progress` —
confirmed empirically that Claude Code's MCP client sends no
`progressToken`, and the spec only permits progress for a token-carrying
request. So the architecture's Layer 2 § "Liveness" assumption (live
notifications give the human motion) does not hold against the real
client. Two fixes already landed on master: the logged
`SessionEvent::Progress` records were wrongly gated on the live callback
(a no-token run logged no progress at all) — now decoupled so progress is
always logged (commit c4567fb); and a new `rexymcp status --repo <path>`
CLI reads the session JSONL for out-of-band, pull-based live status
(commit 3374336). 06b should fold this into the retrospective and decide
whether `docs/architecture.md` Layer 2 should be amended to describe
`rexymcp status` (pull) as the human-liveness path rather than MCP
progress (push). See `milestones/M6-plugin/dogfood-log.md` § "Surprises
and breakages."

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
