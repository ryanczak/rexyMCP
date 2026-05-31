# M5 — MCP server

**Goal:** Wrap the M4 executor library in an **`rmcp` stdio MCP server** — the
load-bearing boundary that lets Claude Code (architect) dispatch a phase to the
local model and see only a structured `PhaseResult`, never the inner transcript.
The server exposes `execute_phase` + `executor_health`, the session-log query
tools (`executor_log_search` / `executor_log_tail` / `get_turn`), and
`model_scorecard`, with progress notifications, output capping, and target-repo
root corroboration.

**Status:** done — all seven phases (01, 02, 03, 04, 05a, 05b, 06) complete; signed off 2026-05-31 (retrospective below).

**Depends on:** M1 (config, `AiClient`, `OpenAiClient`, `health`), M2 (tools +
`Scope` + `ToolRegistry`), M3 (parser), M4 (`agent::execute_phase`, the JSONL
session log, the `PhaseRun` telemetry store). M5 is the layer that **composes**
them behind the MCP boundary; the executor seams it needs already have production
impls (`RealVerifier`, `RealCommandRunner`, `OpenAiClient`).

**Exit criteria:**
- An `rmcp` **stdio** server binary registering the tools below; it auto-connects
  when Claude Code enables the plugin (M6).
- **`execute_phase`** — args: `phase_doc_path`, `repo_path` (target-repo root),
  optional `model` override. Reads + parses the phase doc, assembles the
  production `PhaseInput` / `LoopDeps` (real OpenAI client, `RealVerifier`,
  `RealCommandRunner`, system clock injected at the composition root, scoped
  registry, cross-project telemetry dir), runs `agent::execute_phase`, and returns
  the **capped** `PhaseResult` (+ briefing on failure).
- **`executor_health`** — wraps `health::check`; lets the architect confirm the
  endpoint is reachable + list models before dispatching.
- **Session-log query tools** (`executor_log_search` / `executor_log_tail` /
  `get_turn`) over `store::sessions::read_session_log`, **each capping its own
  output** so a debugging query can't re-flood Claude's context.
- **`model_scorecard`** — aggregates `store::telemetry::read` into the model × tag
  competency matrix (optional `tags` filter).
- **Long runs:** the per-tool MCP timeout is set well above the 10 s default
  (toward the 10-minute ceiling). **Liveness:** progress notifications as the loop
  advances (turn, current tool/stage, files-changed numstat), also logged as
  `Progress` session events. **Context hygiene:** all returned output is capped
  (`MAX_MCP_OUTPUT_TOKENS`). **Roots:** `roots/list` + `CLAUDE_PROJECT_DIR`
  corroborate `repo_path` rather than silently trusting it.
- Hermetic + deterministic tests throughout (`MockAiClient`, injected clock, tool
  handlers exercised without a live transport / network).

## Architecture references

- `docs/architecture.md` — "Layer 2 — `mcp` crate (binary)" (the tool list +
  practical concerns: long runs, liveness, context hygiene, roots); "Session log &
  troubleshooting tools" (the query tools + per-tool output capping); "Model
  effectiveness metrics & routing" (`model_scorecard` over `PhaseRun`); Status §M5.
- M4: `agent::{execute_phase, PhaseInput, LoopDeps}`, `store::sessions`
  (`read_session_log`, `generate_session_id`), `store::telemetry` (`PhaseRun`,
  `read`), the `RealVerifier` / `RealCommandRunner` / `OpenAiClient` production
  seams, `context::Budget`, `security::Scope`, the built-in tool set.

## Phases

Expanded on demand (WORKFLOW.md § Milestones), not all at once.

| #  | Phase                                                              | Status |
|----|-------------------------------------------------------------------|--------|
| 01 | phase-runner wiring (config + phase doc → `execute_phase`) ([phase-01-phase-runner.md](phase-01-phase-runner.md)) | done |
| 02 | rmcp server scaffold + `execute_phase` + `executor_health` ([phase-02-rmcp-scaffold.md](phase-02-rmcp-scaffold.md)) | done |
| 03 | session-log query tools (`executor_log_search` / `executor_log_tail` / `get_turn`) ([phase-03-log-query.md](phase-03-log-query.md)) | done |
| 04 | `model_scorecard` (model × tag competency matrix) ([phase-04-model-scorecard.md](phase-04-model-scorecard.md)) | done |
| 05a | progress callback seam + `Progress` log events (executor side) ([phase-05a-progress-callback.md](phase-05a-progress-callback.md)) | done |
| 05b | progress MCP-notification consumer (mcp side) ([phase-05b-progress-mcp-consumer.md](phase-05b-progress-mcp-consumer.md)) | done |
| 06 | roots corroboration (M5 closer) ([phase-06-roots-corroboration.md](phase-06-roots-corroboration.md)) | done |

Tentative remaining phases (draft when the prior one lands):

- **05** — progress notifications + `Progress` log events, **split into 05a +
  05b** because the work crosses a clean cohesive seam (executor producer vs
  mcp consumer), each session-sized and independently reviewable. Mirrors M4
  phase-07's splitting wisdom.
  - **05a** *(drafted)* — **executor side**: progress-callback seam on
    `LoopDeps`, four emission sites (`turn_start` / `tool:<name>` / `verify`
    / `command:<name>`), numstat helper reusing the working-set + `similar`
    machinery, `SessionEvent::Progress` log entries at each emission. Pins
    the callback contract that 05b consumes. The only M5 phase that touches
    `executor/` substantively — authorized inline.
  - **05b** *(drafted)* — **mcp consumer side**: the `execute_phase` handler
    extracts the progress token from `_meta.progressToken`, builds a
    `McpProgressNotifier` (which implements `ProgressCallback`), and threads
    it through `runner::run_phase` to `LoopDeps`. Fire-and-forget
    `tokio::spawn` on each emission. No `executor/` edits — the 05a contract
    is sufficient.
_All M5 phases drafted; 06 is the milestone closer (triggers retrospective +
calibration folds on approval)._

## Notes

**This milestone is net-new server code, not a lift.** Rexy is a TUI/CLI agent,
not an MCP server, so M5 has no Rexy donor — it composes M1–M4's public surface.
The executor seams it needs already have production impls (`RealVerifier`,
`RealCommandRunner`, `OpenAiClient`); M5 supplies the glue and the protocol layer.

**rexyMCP is not opencode (WORKFLOW.md § "Phase progression").** The product
executor reached through this server is a local LLM over an OpenAI-compatible
endpoint — never opencode. Do not design any server behavior around opencode's
tooling deficiencies.

**The system clock lives at the composition root.** The M4 loop is deterministic
because it only sees an injected `clock: &dyn Fn() -> u64`. M5 is the binary entry
that injects the *real* `SystemTime`-based clock (the one allowed real-time
source). The wiring stays unit-testable by keeping a clock-injecting seam (see
phase-01) so tests pass a fixed clock — the same "inject IO behind a seam" rule
M4 folded into STANDARDS §3.3.

**The executor contract + `STANDARDS.md` are inputs, not built here.** Embedding
the portable executor contract into the system prompt is **M6** (the plugin's
embedded templates). M5's `execute_phase` accepts the contract + standards text as
inputs and reads `STANDARDS.md` from the target repo's `docs/dev/` where present;
it does **not** invent the embedding. Keep that seam clean for M6 to fill.

**Telemetry dir is cross-project.** `PhaseRun` records accumulate across every repo
the executor touches (architecture § "Model effectiveness metrics & routing"), so
the telemetry dir is **not** under the target repo. The wiring plumbs it as an
`Option<&Path>`; where the cross-project store actually lives (config-driven /
user-data-dir) is resolved in phase-02 (server config), not phase-01.

**Output capping is the boundary's whole point.** Every tool that can return
inner-transcript-sized data (`execute_phase`, the log-query tools) caps its output
so the MCP boundary keeps doing its job: Claude sees the `PhaseResult` summary +
diff + (on failure) briefing, and queries the log on demand — pull, not push.

## M5 retrospective (milestone close, 2026-05-31)

Seven phases (01, 02, 03, 04, 05a, 05b, 06) — six approved_first_try, one
bounced once (05b → bug-05b-1 → verified). The `rmcp` 1.7 stdio MCP server is
live as `rexymcp serve --config <path>`, registering seven tools (`execute_phase`,
`executor_health`, `executor_log_search`, `executor_log_tail`, `get_turn`,
`model_scorecard`, plus `execute_phase`'s implicit corroboration check), with
the full progress consumer split (live MCP `notifications/progress` for the
human + logged `Progress` events for Claude's post-return queries) and
target-repo-root corroboration against the MCP client's `roots/list` +
`CLAUDE_PROJECT_DIR`. Total tests: **629** (started M5 at 492 executor + 0 mcp;
ended at 512 executor + 117 mcp — 117 new mcp tests over the milestone).

**M5 is the first net-new composition that crosses an external protocol
boundary.** Every prior milestone composed library code internally; M5 puts
the executor behind an MCP transport that a real client (Claude Code) will
drive. That shift surfaced one architectural-divergence pattern, one
cross-cutting trait-bound pattern, and one wrap-vs-derive pattern — see folds
below.

**Phase splits worked.** 05 split into 05a (executor producer) and 05b (mcp
consumer) along the cohesive seam M4 phase-07 first taught us. The split kept
each sub-phase session-sized and independently reviewable; 05a was
approved_first_try, 05b bounced once on bug-05b-1, and the bug fix didn't
ripple back into 05a. Splitting where the consumer/producer seam already
lives is a reliable move.

**Pre-flight 3 worked.** Phases 02, 05b, and 06 all had pre-flight steps that
told opencode to verify the live `rmcp` 1.7 API against `cargo doc` rather
than trust the architect's sketch. The discipline paid off three times:
opencode confirmed `rmcp::transport::stdio` (not `::io::stdio`) in phase-02,
discovered that `#[rmcp::tool]` can't receive a context arg in phase-05b
(triggering the manual `ServerHandler` refactor — *the architectural
divergence below*), and verified the `list_roots()` peer method in phase-06.
The lesson: **when wrapping an external API the architect can't live-call,
require the executor to verify against the real docs and trust their findings
over the architect's plausible-but-stale sketch.** Already folded into the
phase docs as a recurring Pre-flight 3 pattern; no fresh WORKFLOW update
needed.

**Architectural divergence pattern (declared in 05b, accepted, no fold).**
phase-05b discovered that `#[rmcp::tool]`'s macro signature can't receive a
context arg in rmcp 1.7. The right response was to replace the macro-derived
`ServerHandler` for `execute_phase` with a manual `impl ServerHandler` that
explicitly delegates the other four tools to `Self::tool_router()`. This is
a one-off external-API constraint, not a recurring rule worth folding into
WORKFLOW. The Pre-flight 3 pattern above is the generalizable lesson.

### Calibration folds — applied this close

Two patterns recurred across M5 phases (≥2 occurrences) that warrant
WORKFLOW updates.

**Fold 1: Wrap-vs-derive at protocol boundaries** (extended `### Derive
intentionally` in WORKFLOW.md). Exercised four times across M5:
- Wrap with `serde_json::Value`: phase-02 `ExecutePhaseOutput` (foreign
  `PhaseResult` tree); phase-03 `LogQueryOutput` (foreign `SessionRecord` +
  `SessionEvent` + `ToolCall` + …).
- Derive directly: phase-02 `Health` (small, isolated); phase-04
  `ScorecardRow` (small, mcp-owned).
The rule: **when exposing a type at a protocol boundary, derive the boundary
trait (e.g. `JsonSchema`) when the schema tree is small and locally-owned;
wrap in a single-field `serde_json::Value` carrier when the tree is large or
foreign (and dragging the derive across it would cascade).** Wrapping costs
one nesting layer in the JSON output; deriving costs `JsonSchema` on every
type in the tree.

**Fold 2: Anticipate cross-boundary trait bounds at boundary introduction**
(new subsection in WORKFLOW.md `## Calibration` — `### Anticipate
cross-boundary trait bounds`). Recurrences across M4 + M5:
- M4 phase-03 (architect-resolved blocker): `Deserialize` on M3 parser types
  for the JSONL log round-trip.
- M5 phase-02 (declared deviation): `Send + Sync` on `LoopDeps.clock`
  (rmcp's macro requires `Send` futures); `JsonSchema` on `Health`.
- M5 phase-03 (sidestepped via wrap): `JsonSchema` across the
  SessionRecord/SessionEvent tree avoided by wrap.
- M5 phase-04 (planned in spec): `JsonSchema` on `ScorecardRow` from the start.
- M5 phase-05a (planned in spec): `Send + Sync` on `ProgressCallback`.
Five recurrences. The rule: **when introducing a new protocol or async
boundary in a phase spec, enumerate the trait bounds the boundary will
require (`Serialize`/`Deserialize`/`Send`/`Sync`/`JsonSchema`/…) and check
whether they already exist on the types crossing it. If they don't, the
phase either adds them (with an authorized executor edit and a wrap-vs-derive
decision) or sidesteps via a wrapper. Plan this at draft time, not at
review time.**

### Per-phase calibration tally

- **bug-01-1** (phase-01, one bounce → verified): missing CLI-parse test. M2-era
  "pin negative cases" fold already covers the general lesson; the
  specific pattern here is *Acceptance criteria checkbox is the contract, not
  the suggestion*. Phase-01 → phase-04 internalized this; phase-05b
  partially regressed (see bug-05b-1) before snapping back.
- **bug-05b-1** (phase-05b, one bounce → verified): two items —
  `#[allow(clippy::too_many_arguments)]` (hard-rule violation; clean fix
  was the struct-grouping pattern already in use elsewhere) + missing
  wrapper-level integration tests (acceptance criteria miss with a
  declared-but-wrong rationale). The lesson layered on M2's: *a declared
  deviation against an explicit acceptance criterion checkbox is still a
  bounce — declaration ≠ approval*. The bug-doc's "spec must pin the
  testability mechanism, not just name the test" is also worth noting; my
  spec didn't pin the `execute_phase_inner_with_client` seam refactor as
  the testability mechanism. **Architect calibration:** when a wrapper-level
  test is mandated, the spec should also pin (or authorize) the seam that
  makes it hermetic-testable. Not folding into WORKFLOW yet (one
  occurrence); flag if it recurs in M6.

**Self-review accuracy held throughout** (the phase-01 origin lesson):
opencode declared deviations in Notes for review at every phase past
phase-01, including the substantial 05b architectural divergence (which
turned out to be a one-off pattern, not a fold trigger). The "declare
even-defensible deviations" muscle is now reflexive.

### What carries forward to M6

- **Plugin packaging:** `.mcp.json` template, slash commands, skills.
- **Embedded executor contract + STANDARDS templates** — phase docs
  acknowledged this hand-off explicitly. M5 left `executor_contract` as an
  empty-string input on every `execute_phase` call; M6 fills it from an
  embedded resource.
- **Per-tool MCP timeout** — Layer 2 architecture mandates it; phase-02
  noted this is client-side (`.mcp.json`), which is M6 territory.
- **Project initialization (bootstrap)** — the `architect` skill's
  responsibility per architecture; M5 left `STANDARDS.md` as a per-call
  read from the target repo.
- **URL-encoded path components in roots URIs** — Adaptation 3 in phase-06,
  pinned as a follow-up; revisit if M6 dogfood surfaces it.
