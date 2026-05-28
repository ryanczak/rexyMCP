# rexyMCP — Architecture

> **Status:** Design. No code exists yet. This document is the source of truth
> for the *intended* design; `src/` (once it exists) is the source of truth for
> what actually runs. Milestones are listed in the **Status** section at the
> bottom — that list is the project plan.

## What rexyMCP is

rexyMCP lets you run a **principal-engineer / executor** development workflow
with **Claude Code as the harness**:

- **Claude (Opus/Sonnet) is the architect.** It decomposes a product spec into
  milestones → phase docs, dispatches each phase to an executor, reviews the
  result, and escalates by taking the work over itself when the executor is
  stuck.
- **A local LLM is the executor.** Qwen / Gemma (etc.) served over an
  OpenAI-compatible endpoint (vLLM, LM Studio, Ollama) implements one phase at a
  time, with the reliability plumbing — forgiving tool-call parsing, a 2-stage
  tool router, a governor/verifier — that small models need to produce usable
  work.

The two are bridged by an **MCP server**. This is the load-bearing design
constraint, so it is stated up front.

### Why an MCP server (and not a subagent)

Claude Code's native subagents (the Task tool, `.claude/agents/*.md`) run
**Claude models only**. The subagent `model:` field accepts `sonnet` / `opus` /
`haiku` / a Claude model ID / `inherit` — there is no field for a custom
endpoint. The `ANTHROPIC_BASE_URL` / Bedrock / Vertex provider overrides retarget
the **entire harness** model, not a single subagent, and the Claude Agent SDK is
likewise Anthropic-models-only.

The one supported way to make Claude delegate to a non-Anthropic model is an
**MCP server**: a custom server exposes a tool, and what happens *inside* that
tool — including an agentic loop driving a local model — is opaque to Claude.
The inner transcript never enters Claude's context; Claude sees only the tool's
return value. That property is exactly what we want for an executor sub-agent.

### Relationship to Rexy

rexyMCP is a **standalone product**. It borrows heavily from
[Rexy](../../rexy) — the terminal agent for small local models — which already
implements the AI client, forgiving parser, tools, governor/verifier, and the
architect/executor *workflow* (`docs/dev/WORKFLOW.md`, `STANDARDS.md`). rexyMCP
**lifts those modules and refactors freely**. Rexy is a parts donor, not a
dependency: nothing here links against Rexy, and parts of Rexy (the TUI, the
local planner, the cloud-escalation transport) are deliberately left behind.

## The three layers

```
┌─────────────────────────────────────────────────────────────┐
│ Claude Code (harness)                                         │
│   architect skill · review-phase skill · escalate skill       │
│   /architect · /dispatch · /review commands                   │
└───────────────┬───────────────────────────────────────────────┘
                │ MCP (stdio): execute_phase, executor_health
┌───────────────▼───────────────────────────────────────────────┐
│ mcp crate (binary) — rmcp stdio server                         │
│   tool schemas · progress notifications · output capping       │
└───────────────┬───────────────────────────────────────────────┘
                │ in-process call
┌───────────────▼───────────────────────────────────────────────┐
│ executor crate (library) — headless single-phase agent loop    │
│   ai client · forgiving parser · tools · router · governor ·    │
│   verifier · context budget · security/scope · PhaseResult      │
└───────────────┬───────────────────────────────────────────────┘
                │ OpenAI-compatible HTTP
┌───────────────▼───────────────────────────────────────────────┐
│ Local LLM — vLLM / LM Studio / Ollama (Qwen, Gemma, …)         │
└───────────────────────────────────────────────────────────────┘
```

### Layer 1 — `executor` crate (library)

The headless agentic loop. Given a **phase-doc path** and a **target-repo root**,
it loads the engineering standards + the phase spec, runs the local model through
a tool-using loop, verifies edits, runs the target project's build/lint/test
commands, and returns a structured result.

It is built by lifting and adapting Rexy modules. The lift/drop map:

| Capability | Rexy source | Disposition in rexyMCP |
|---|---|---|
| OpenAI-compatible AI client; circuit breaker + retry; `AiClient` trait | `rexy/src/ai/mod.rs`, `rexy/src/ai/backends/openai.rs` | **Lift.** Already provider-generic (`make_client` handles `openai`/`ollama`/`lmstudio`). Anthropic/Gemini backends optional. |
| Message / event / tool types | `rexy/src/ai/types.rs` | **Lift** the subset the loop needs (`Message`, `AiEvent`, `ToolSchema`, `ToolResult`). |
| `MockAiClient` for hermetic tests | `rexy/src/ai/testing.rs` | **Lift.** |
| Forgiving tool-call parser (6-stage pipeline) | `rexy/src/agent/parser/` | **Lift.** |
| Tools: `read_file`, `write_file`, `patch`, `bash`, `search`, `find_files`, `symbols` | `rexy/src/tools/` | **Lift**; every path scoped to the target-repo root. |
| Tool registry + 2-stage router | `rexy/src/tools/registry.rs`, `router.rs` | **Lift.** |
| Governor: scorer, verifier, hard-fail detector | `rexy/src/governor/` | **Lift.** |
| Context budget + compactor | `rexy/src/context/` | **Lift.** |
| Security: scope, capabilities, bash classify, redact, injection, audit | `rexy/src/security/` | **Lift.** Critical — a weak model running `bash` needs the allowlist. |
| Result truncation/formatting; error model | `rexy/src/result/`, `rexy/src/error/` | **Lift / adapt.** |
| TUI (fullscreen / classic) | `rexy/src/tui/` | **Drop.** No terminal UI; progress flows over MCP. |
| Local planner (TODO decomposition) | `rexy/src/planner/` | **Drop.** Decomposition is the architect's (Claude's) job. |
| Escalation **cloud transport** | `rexy/src/escalation/` | **Drop the transport; keep the briefing.** See below. |

#### The executor turn cycle

One `execute_phase` call runs this loop until the phase completes, hard-fails, or
hits a budget cap:

1. Load `STANDARDS.md` + the phase doc; build the executor system prompt.
2. Apply the context budget; compact if needed (never evict system messages).
3. Call the local model (`AiClient::chat`) with the routed tool schemas.
4. Run the model output through the forgiving parser → `ToolCall` or a
   `ParseFailure` that is fed back to the model as repair guidance.
5. Dispatch the tool through the governor → registry; all filesystem/bash access
   is scoped to the target-repo root.
6. After edit-class tools, run the verifier (the project's typecheck/build). On
   failure, feed the diagnostics back for a retry.
7. The hard-fail detector watches for repetition loops, repeated verifier
   failures, and budget overflow. If it trips, assemble a **briefing** and stop.
8. On clean completion, run the project's full command set
   (`{FORMAT_COMMAND}`/`{BUILD_COMMAND}`/`{LINT_COMMAND}`/`{TEST_COMMAND}`),
   capture output, and return.

#### Escalation = Claude Code itself

Rexy escalates a stuck local model to a cloud provider *inside its own process*.
rexyMCP **keeps Rexy's hard-fail detection and briefing assembly but drops the
cloud transport**. Instead of calling Anthropic, the executor returns the
briefing up through the MCP boundary, and **Claude — already the architect — is
the escalation target.** Claude either:

- takes a **single-turn handoff** (makes the one focused edit the executor
  couldn't, hands control back), or
- performs a **session takeover** (finishes the phase itself), or
- **refines the phase doc** and re-dispatches.

The briefing is a *fresh* brief, not a transcript replay — the shape Rexy already
defines: **goal** (verbatim), **acceptance criteria**, **current code state**
(diagnostics + working-set files), **what was tried** (compressed to 1–2 lines
each), **current blocker** (exact diagnostic), **budget remaining**.

#### The `PhaseResult` / briefing contract

`execute_phase` returns a single structured value:

```
PhaseResult {
  status:          "complete" | "hard_fail" | "budget_exceeded",
  files_changed:   [ { path, change_summary } ],
  diff:            unified diff of the working tree (capped),
  command_outputs: { format, build, lint, test → tail of stdout/stderr },
  update_log:      the completion / blocker entry written into the phase doc,
  briefing:        present only when status != "complete" — the escalation brief,
}
```

This is the entire interface Claude reasons over. The inner loop's tokens, the
parser's repair history, and the raw tool transcript stay inside the executor.

### Layer 2 — `mcp` crate (binary)

An MCP **stdio** server built on the `rmcp` crate. It exposes two tools:

- **`execute_phase`** — args: `phase_doc_path` (string), `repo_path` (string,
  the target-repo root), optional `model` / `profile` override. Calls the
  `executor` library in-process and returns `PhaseResult`.
- **`executor_health`** — args: optional endpoint override. Pings the configured
  OpenAI-compatible endpoint and lists available models. Lets the architect
  confirm the executor is reachable before dispatching.

Practical concerns this layer owns:

- **Long runs.** A phase can take minutes; the MCP per-tool `timeout` is set well
  above the 10-second default (toward the 10-minute ceiling).
- **Liveness.** The server emits **MCP progress notifications** as the executor
  advances (turn count, current tool, verifier pass/fail) so Claude and the user
  see motion.
- **Context hygiene.** Returned output is capped (`MAX_MCP_OUTPUT_TOKENS`) so a
  phase's inner transcript can never flood Claude's context. Claude gets the
  `PhaseResult` summary + diff + (on failure) briefing — nothing more.

### Layer 3 — Plugin package

A Claude Code **plugin** bundles the MCP server with the workflow that drives it:

- **`.mcp.json`** registering the `mcp` server binary (auto-connects on enable).
- **Skills:**
  - `architect` — explore the target repo, then write the design doc, milestone
    README, and phase docs into the target repo's `docs/dev/`, following the
    `WORKFLOW.md` templates verbatim.
  - `review-phase` — check executor output against the Definition of Done in
    `STANDARDS.md`, rerun the project's commands, then approve or file a bug.
  - `escalate` — take over a stuck phase (single-turn fix or session takeover).
- **Commands:** `/architect`, `/dispatch <phase>`, `/review <phase>`.
- **Embedded templates:** generalized copies of `STANDARDS.md` / `WORKFLOW.md`.
  These use `{BUILD_COMMAND}` / `{LINT_COMMAND}` / `{TEST_COMMAND}` /
  `{FORMAT_COMMAND}` placeholders that resolve **per target project** from
  rexyMCP config, which is what makes the product language-agnostic.

## End-to-end flow

1. The user gives Claude a product idea.
2. The `architect` skill explores the target repo and writes the design doc + M1
   README + phase docs into the target repo's `docs/dev/`.
3. `/dispatch phase-01` → Claude calls the `execute_phase` MCP tool.
4. The MCP server runs the `executor` loop: local model drives the phase spec,
   verifier checks edits, the project's commands run at the end.
5. `execute_phase` returns `PhaseResult`; the engine appends the Update-Log
   completion entry. On hard-fail it returns the briefing instead.
6. The `review-phase` skill checks the DoD and reruns the commands. Pass → next
   phase. Fail → `escalate` (Claude fixes it) or file a bug and re-dispatch.

## Configuration

rexyMCP config (designed in M1) carries, per invocation or per target project:

- the OpenAI-compatible **endpoint** + **model** (executor),
- the **target-repo root** (the scope boundary for all file/bash tools),
- the **command set** that resolves the `{…_COMMAND}` placeholders
  (`format`/`build`/`lint`/`test`),
- budget knobs (context %, max turns, escalation slots).

## Non-goals

- **No terminal UI.** rexyMCP is a bridge, not an interactive agent. Progress is
  surfaced through MCP, not a TUI.
- **No local planning/decomposition.** The architect (Claude) owns milestone and
  phase decomposition. The executor implements one already-written phase.
- **No internal cloud escalation.** Escalation returns to Claude; rexyMCP never
  calls a cloud LLM provider itself.
- **No dependency on Rexy.** Code is lifted and vendored, not linked.

---

## Status

The project plan. Each entry becomes a milestone with its own
`docs/dev/milestones/M<n>-<slug>/` directory. Milestones are expanded into phases
**on demand**, not all at once.

1. **M1 — Foundations.** Cargo workspace (`executor` lib + `mcp` bin), config
   model (endpoint, target-repo root, per-project commands), error model, CI
   skeleton, AI client lifted from Rexy, and a health-check entrypoint that
   connects to a live local endpoint and lists models.
2. **M2 — Executor tools & security.** Lift the tools (`read_file`,
   `write_file`, `patch`, `bash`, `search`, `find_files`, `symbols`), the
   registry + 2-stage router, and the security scope/capabilities/bash-classify
   layer, all scoped to a target-repo root.
3. **M3 — Forgiving parser.** Lift the 6-stage tool-call parser pipeline and its
   model-feedback formatter.
4. **M4 — Headless agent loop + governor/verifier.** The single-phase executor
   loop: context budget, verifier, hard-fail detection, and the
   `PhaseResult` + briefing contract.
5. **M5 — MCP server.** The `rmcp` stdio server exposing `execute_phase` and
   `executor_health`, with progress notifications and output capping.
6. **M6 — Plugin + architect/review skills.** The Claude Code plugin manifest,
   the `architect` / `review-phase` / `escalate` skills, the slash commands, the
   embedded generalized `STANDARDS.md` / `WORKFLOW.md`, and an end-to-end dogfood
   against a real repo.
