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
| 01 | post-edit verifier + `Diagnostic` ([phase-01-verifier.md](phase-01-verifier.md)) | todo |

Tentative remaining phases (draft when the prior one lands):

- **02** — context budget + compaction (`context/`: token accounting, budget,
  compactor; never evict system messages).
- **03** — the JSONL **session log** writer + reader (`store/sessions/jsonl.rs`),
  redacted, reusing M3's `Serialize` types as the event schema.
- **04** — governor: per-(task,tool) **scorer** + **hard-fail detector**
  (repetition loops, repeated verifier failures, budget overflow).
- **05** — `PhaseResult` + the **briefing** contract (adapt `escalation/packet.rs`:
  drop cloud transport + the planner TODO section; the brief is *returned* to
  Claude). `Origin::Native` + `parser/native.rs` + `stream.rs` (deferred from M3)
  also land around here.
- **06** — the **turn-cycle loop** (`execute_phase`): net-new orchestration
  composing AI client + parser + tools + governor + budget + session log. Owns the
  **read-before-edit invariant** (working-set + mtime).
- **07** — **`PhaseRun`** telemetry (`store/telemetry.rs`): cross-project metrics
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

**Redaction + the session log are M4's, but the redaction primitive itself**
(`security/redact.rs`) is still a Rexy stub — it must be implemented (net-new,
like `scope`) as part of the session-log phase, not assumed lifted.
