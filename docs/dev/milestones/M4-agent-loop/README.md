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
| 03 | JSONL session log: writer/reader + event schema ([phase-03-session-log.md](phase-03-session-log.md)) | done |
| 04 | secret redaction primitive ([phase-04-redaction.md](phase-04-redaction.md)) | done |
| 05 | governor: tool scorer + hard-fail detector ([phase-05-governor.md](phase-05-governor.md)) | done |
| 06 | `PhaseResult` + briefing contract ([phase-06-phase-result.md](phase-06-phase-result.md)) | done |
| 07a | turn-loop core ([phase-07a-loop-core.md](phase-07a-loop-core.md)) | review |

Tentative remaining phases (draft when the prior one lands):

- **07 — the turn-cycle loop (`execute_phase`)**, split into four sub-phases
  because the full composition (eight subsystems + read-before-edit + the native
  seam + diff + logging) far exceeds one executor session:
  - **07a** *(drafted)* — **loop core**: prompt assembly, budget + compaction, the
    chat→parse→dispatch turn cycle, the **native-call seam** (`AiEvent::
    ToolCallGeneric` → `parser::ToolCall { origin: Origin::Native }`, dispatched
    through the same path as a text-extracted call — absorbs the old "06b", see
    Notes), the tool scorer, terminating `complete` / `budget_exceeded` (+ budget
    briefing). Control flow only.
  - **07b** — **session log**: weave the redacted JSONL log (`store::sessions`
    `SessionLogger` + `redact`) through the 07a steps as a side effect; the
    `SessionEvent` kinds map to each turn event.
  - **07c** — **governance**: post-edit `verifier::verify` retry (`Baseline`
    author/ambient partition, diagnostic feedback), `hard_fail::evaluate` →
    `hard_fail` briefing (`Blocker::HardFail`, populated diagnostics), and the
    **read-before-edit invariant** (working-set + mtime).
  - **07d** — **completion artifacts**: the final command set
    (`{FORMAT,BUILD,LINT,TEST}_COMMAND` from `CommandConfig`), unified-diff
    generation, and `files_changed` / `command_outputs` population.
- **08** — **`PhaseRun`** telemetry (`store/telemetry.rs`): cross-project metrics
  record (gates, turns, tokens, parse-failure rate, verifier retries).

## Notes

**This milestone is the first net-new composition, not a pure lift.** The
turn-cycle loop (phase-07) is rexyMCP's own orchestration; the pieces it composes
(verifier, budget, governor, session log, briefing) are lifts/adaptations.

**"06b" was dropped, not deferred (decided 2026-05-29).** Phase-06 originally
sketched a follow-on "06b" lifting Rexy's `parser/native.rs` + `stream.rs`. On
inspection neither fits rexyMCP's streaming, headless design: (1) Rexy's
`native::extract(&Value)` parses native tool calls out of a *full non-streaming
response* (OpenAI **and** Anthropic), but rexyMCP's `AiClient::chat` is
streaming-only — the OpenAI backend already extracts native `tool_calls` from the
SSE deltas and emits `AiEvent::ToolCallGeneric`, and rexyMCP never calls
Anthropic, so the lift would have zero callers; (2) Rexy's `stream.rs`
(`StreamRenderer`/`StreamDisplay`) is a **TUI** indicator, and rexyMCP is a
headless MCP server whose liveness is progress notifications (M5), so it has no
consumer. The only genuine gap — turning a native `ToolCallGeneric` event into a
`parser::ToolCall { origin: Origin::Native }` for uniform dispatch — is a small
adapter that belongs *in* the loop that consumes the event, so it folds into
phase-07 rather than standing alone. `Origin::Native` already exists in the M3
type set.

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
(resolve at phase-07): whether the verifier's language set stays a curated
per-compiler list (Rust/TS/Py, extensible) or becomes config-driven with a
pluggable diagnostic parser. Lift the per-compiler verifier now; revisit when the
loop wires it.

**Redaction is its own phase (04), not part of the session log.** Rexy's
`security/redact.rs` is a stub — redaction is net-new (like `scope`/`bash_classify`).
The session-log **writer is redaction-agnostic**: it writes whatever `SessionRecord`
it's given. Records are redacted **upstream** (by the loop, phase-07, composing
redact → log) so secrets never reach disk. Splitting it keeps phase-03 a tractable
log-mechanism + schema phase and phase-04 a focused net-new redactor.

phase-04 is scoped to the **content redactor** (secret-prefix + tagged-value
patterns, opt-in high-entropy, `[REDACTED:<type>]` marker). **Path-based
read-refusal** (never *reading* `.env` / `*.pem` / `~/.ssh/**` at all) was
originally sketched under phase-04 but is deferred: it is a read-tool / `scope`
concern (refusing a read), distinct from redacting captured content before a
write. Pick it up in a dedicated read-tool phase if wanted, not phase-04.

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
