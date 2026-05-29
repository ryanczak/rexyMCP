# M4 — Headless agent loop + governor/verifier

**Goal:** Build the single-phase executor loop — the headless turn cycle that
drives a local model through one phase (parse → tool dispatch → verify → retry),
under a context budget and a hard-fail detector, and returns the structured
`PhaseResult` (+ a `briefing` on failure). Plus the redacted JSONL **session log**
and the per-phase **`PhaseRun`** metrics record.

**Status:** in-progress

**Depends on:** M1 (AI client), M2 (tools + registry + scope + bash classifier),
M3 (forgiving parser). M4 is the layer that **composes** all three into a loop.

**Exit criteria:**
- `execute_phase(phase_doc, repo_root, …) -> PhaseResult` runs the turn cycle
  (architecture.md § "The executor turn cycle", steps 1–8): assemble prompt →
  budget/compact → `AiClient::chat` → `parser::parse` → governor/registry dispatch
  (scope-confined) → post-edit **verify** → hard-fail detection → final command
  set → return.
- `PhaseResult { status, files_changed, diff, command_outputs, update_log,
  briefing? }` — `briefing` present only on `hard_fail` / `budget_exceeded`.
- **Escalation = Claude:** the briefing is *returned*, not sent to a cloud model
  (no cloud transport — architecture § "Escalation = Claude Code itself").
- Every turn event is written (redacted) to a **JSONL session log** under
  `<repo>/.rexymcp/sessions/`, reusing M3's `Serialize` parser types as the event
  schema; a per-phase **`PhaseRun`** record is emitted to the cross-project
  telemetry store.
- The **read-before-edit invariant** is enforced by the loop (architecture §
  Status, M4): `patch` refuses a file the executor hasn't read this session or
  that changed on disk underneath it.
- Hermetic + deterministic throughout (`MockAiClient`, injected clock, no network).

## Architecture references

- `docs/architecture.md` — "The executor turn cycle" (the 8-step loop); the
  `PhaseResult` / briefing contract; "Escalation = Claude Code itself"; "Session
  log & troubleshooting tools"; "Model effectiveness metrics & routing"
  (`PhaseRun`); Status §M4 (read-before-edit).
- Rexy source: `governor/` (verifier, scorer, hard_fail), `context/` (budget,
  tokens, compactor), `store/sessions/jsonl.rs`, `store/telemetry.rs`,
  `escalation/packet.rs` (the briefing — keep the brief, drop the cloud transport
  and the planner TODO section).

## Phases

Expanded on demand (WORKFLOW.md § Milestones), not all at once.

| #  | Phase                                                              | Status |
|----|-------------------------------------------------------------------|--------|
| 01 | post-edit verifier + `Diagnostic` ([phase-01-verifier.md](phase-01-verifier.md)) | done |
| 02 | context budget + compaction ([phase-02-context-budget.md](phase-02-context-budget.md)) | done |
| 03 | JSONL session log: writer/reader + event schema ([phase-03-session-log.md](phase-03-session-log.md)) | todo |

Tentative remaining phases (draft when the prior one lands):

- **04** — **redaction** primitive (net-new, like `scope`/`bash_classify`):
  secret-prefix + tagged-value patterns + path-based blanket redaction, with a
  `[REDACTED:<type>]` marker. Applied to records **upstream** of the log (by the
  loop) — the phase-03 log writer is redaction-agnostic.
- **05** — governor: per-(task,tool) **scorer** + **hard-fail detector**
  (repetition loops, repeated verifier failures, budget overflow).
- **06** — `PhaseResult` + the **briefing** contract (adapt `escalation/packet.rs`:
  drop cloud transport + the planner TODO section; the brief is *returned* to
  Claude). `Origin::Native` + `parser/native.rs` + `stream.rs` (deferred from M3)
  also land around here.
- **07** — the **turn-cycle loop** (`execute_phase`): net-new orchestration
  composing AI client + parser + tools + governor + budget + session log
  (redact → log). Owns the **read-before-edit invariant** (working-set + mtime).
- **08** — **`PhaseRun`** telemetry (`store/telemetry.rs`): cross-project metrics
  record (gates, turns, tokens, parse-failure rate, verifier retries).

## Notes

**This milestone is the first net-new composition, not a pure lift.** The
turn-cycle loop (phase-06) is rexyMCP's own orchestration; the pieces it composes
(verifier, budget, governor, session log, briefing) are lifts/adaptations.

**rexyMCP is not opencode (WORKFLOW.md § "Phase progression").** The loop drives a
local model whose messy output is handled by the M3 forgiving parser + structured
tools — it does **not** inherit opencode's serializer fragility. Do not design any
loop behavior around an opencode limitation.

**Verifier vs. final command set — distinct.** The post-edit **verifier**
(phase-01, step 6) needs *structured* `Diagnostic`s for retry feedback, so it
parses per-compiler output (cargo `--message-format=json`, `tsc`, `ruff --json`).
The **final command set** (step 8) just runs the config-resolved
`{FORMAT,BUILD,LINT,TEST}_COMMAND`s and captures stdout/stderr tails into
`PhaseResult.command_outputs` — no structured parsing. **Open design question**
(resolve at phase-06): whether the verifier's language set stays a curated
per-compiler list (Rust/TS/Py, extensible) or becomes config-driven with a
pluggable diagnostic parser. Lift the per-compiler verifier now; revisit when the
loop wires it.

**Redaction is its own phase (04), not part of the session log.** Rexy's
`security/redact.rs` is a stub — redaction is net-new (like `scope`/`bash_classify`).
The session-log **writer is redaction-agnostic**: it writes whatever `SessionRecord`
it's given. Records are redacted **upstream** (by the loop, phase-07, composing
redact → log) so secrets never reach disk. Splitting it keeps phase-03 a tractable
log-mechanism + schema phase and phase-04 a focused net-new redactor.

**Timestamps without `chrono`.** Rexy's log uses `chrono::Utc::now()`. rexyMCP has
no `chrono` dep, and STANDARDS forbids `Utc::now()` in production (determinism).
The `SessionRecord.ts` is a `u64` (unix epoch millis) **set by the caller** (the
loop injects it from a clock); the log writer never reads the clock. No `chrono`.

**Progress heartbeats (design decision — implemented in M5, schema reserved in
M4 phase-03).** MCP `notifications/progress` heartbeats during a long
`execute_phase` carry a **files-changed + per-file `+/-` numstat summary** (turn,
current stage/tool, top-N changed files with line counts + totals), encoded in the
notification's `message` string (the protocol has no structured payload field).
Confirmed: **progress notifications do not enter Claude's context** — they are
human-facing liveness UI. Their durable value is captured by **logging each
progress event (redacted) into the session JSONL log** as a `progress` event kind,
queryable on demand via the M5 log-query tools (`executor_log_search` with a
`kind`/progress filter — *no dedicated tool by default*; the model's tool surface
stays lean). Consumer split: the **human** watches the live notifications (this is
where mid-call **abort** decisions happen); **Claude** queries the logged progress
**post-return** (debugging a `hard_fail`, seeing how the diff evolved) — it cannot
query mid-call because `execute_phase` is synchronous and suspends Claude until
`PhaseResult`. The live numstat reuses the loop's working-set + diff machinery
(already needed for read-before-edit and `PhaseResult.diff`); the heartbeat is a
*liveness summary*, never a second source of truth.
