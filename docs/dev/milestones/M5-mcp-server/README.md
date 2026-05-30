# M5 — MCP server

**Goal:** Wrap the M4 executor library in an **`rmcp` stdio MCP server** — the
load-bearing boundary that lets Claude Code (architect) dispatch a phase to the
local model and see only a structured `PhaseResult`, never the inner transcript.
The server exposes `execute_phase` + `executor_health`, the session-log query
tools (`executor_log_search` / `executor_log_tail` / `get_turn`), and
`model_scorecard`, with progress notifications, output capping, and target-repo
root corroboration.

**Status:** in progress — phase-01 drafted (`todo`); later phases drafted on demand.

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
| 04 | `model_scorecard` (model × tag competency matrix) ([phase-04-model-scorecard.md](phase-04-model-scorecard.md)) | todo |

Tentative remaining phases (draft when the prior one lands):

- **05 — progress notifications + `Progress` log events.** Emit MCP
  `notifications/progress` heartbeats (turn, stage/tool, files-changed `+/-`
  numstat encoded in the `message` string) and log each as a `Progress` session
  event. **Requires a progress-callback seam in the loop** (an executor change —
  authorize in that phase): the M4 loop has no progress hook today, and
  `SessionEvent::Progress` + `FileNumstat` were reserved in M4 phase-03 for exactly
  this. Reuses the loop's working-set + diff machinery; the heartbeat is a liveness
  summary, never a second source of truth (see M4 README § "Progress heartbeats").
- **06 — roots corroboration.** Query `roots/list` + read `CLAUDE_PROJECT_DIR` to
  cross-check `execute_phase`'s `repo_path`; flag a mismatch rather than trusting
  it. (Sampling + elicitation are deliberately **not** used — see architecture.)

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
