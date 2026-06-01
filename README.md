# rexyMCP

**Let Claude be the architect. Let a local model do the typing.**

rexyMCP is named after **Rexy**, my cattle dog. A cattle dog doesn't pull the
plow — it *herds*: it keeps the herd moving in the right direction, nips at
strays, and brings the work back to the rancher. rexyMCP herds **local LLMs**
the same way. Claude Code is the rancher; rexyMCP is the dog; a small local
model (Qwen, Gemma, or anything you can serve over an OpenAI-compatible
endpoint) is the herd that does the legwork. Claude decomposes your idea into
spec'd phases, hands each one to the local model, and reviews what comes back —
while rexyMCP keeps the local model on task, in bounds, and honest.

```
   you ──▶ Claude (architect) ──▶ MCP ──▶ local LLM (executor) ──▶ your repo
                  ▲                                  │
                  └────────── review / escalate ─────┘
```

## Why this exists

Claude Code's native subagents run **Claude models only** — there's no field for
a custom endpoint, and the provider overrides retarget the *entire* harness, not
one delegate. The single supported way to make Claude hand work to a
non-Anthropic model is an **MCP server**: Claude calls a tool, and whatever
happens inside that tool is opaque to it. rexyMCP puts a complete agentic loop
*inside* that tool.

The payoff: the local model's thousands of tokens of trial-and-error, tool
calls, and parser repairs never touch Claude's context. Claude sees one clean
structured result — and, when the executor gets stuck, a tight briefing it can
act on. Cheap, private, high-volume implementation work runs locally; Claude's
judgment is spent only where it counts.

## Features

**The executor — a headless single-phase agent loop**

- **OpenAI-compatible AI client** for any local endpoint: vLLM, LM Studio,
  Ollama, or anything that speaks the OpenAI chat API. Streaming, retries, and a
  pluggable `AiClient` trait with a `MockAiClient` for hermetic tests.
- **Forgiving tool-call parser** that repairs the malformed tool calls small
  models love to emit (fenced JSON, trailing commas, near-miss key names)
  instead of just rejecting the turn.
- **A full built-in toolset**, every operation scoped to the target-repo root:
  `read_file`, `write_file`, `patch` (anchored edits), `find_files`, `search`
  (content grep), `symbols` (tree-sitter symbol extraction for Rust & Python),
  and `bash`.
- **Security scope confinement** — file and shell operations cannot escape the
  target repository root. A read-before-edit rule keeps the model from blindly
  overwriting files it hasn't seen.
- **A governor** that watches for repetition loops, repeated verifier failures,
  and context/turn-budget overflow, and a **verifier** that runs the project's
  typecheck/build between edits and feeds diagnostics back for a retry.
- **Hard-fail briefings** — when the model is genuinely stuck, the loop stops
  and assembles a tight, structured briefing (one-line summary, working files,
  last attempt) and hands control back to Claude instead of burning the budget.
- **Final command gate** — on clean completion the loop runs the project's
  configured `format` / `build` / `lint` / `test` commands and only reports
  success if they pass.
- **Structured `PhaseResult`** — the single value that crosses the MCP boundary:
  a `status` (`complete` / `hard_fail` / `budget_exceeded`), the list of files
  changed, the diff, command outputs, an Update Log entry, an optional briefing,
  and a path to the session log.
- **Redacted JSONL session log** — every turn (prompts, parsed tool calls, tool
  results, progress, verifier runs) is written to disk with secrets redacted, so
  a lean result never means lost detail.
- **Cross-project telemetry** — each run can emit a `PhaseRun` record for the
  model scorecard.

**The MCP server (`rexymcp serve`)**

An `rmcp` stdio MCP server exposing six tools to Claude Code:

| Tool | What it does |
|------|--------------|
| `execute_phase` | Run the executor against a phase doc + target repo; returns a `PhaseResult`. |
| `executor_health` | Check connectivity to the configured LLM endpoint and list models. |
| `executor_log_search` | Search the session JSONL log by event type, tool name, or text. |
| `executor_log_tail` | Return the last N records of a session log (capped per field). |
| `get_turn` | Return all records for a single turn, uncapped — the raw-detail escape hatch. |
| `model_scorecard` | Aggregate cross-project telemetry into a model × tag competency matrix. |

The server corroborates the target-repo root from the MCP client's roots (or
`CLAUDE_PROJECT_DIR`) and emits live `notifications/progress` during long runs.

**The CLI (`rexymcp`)**

- `rexymcp health` — connectivity check against the configured endpoint.
- `rexymcp run-phase` — run a single phase from the command line (returns the
  `PhaseResult` as JSON), no MCP client required.
- `rexymcp serve` — start the MCP stdio server.
- `rexymcp status` — read a session log and report the latest phase progress
  (human summary or `--json`). This is the pull-based liveness path while a phase
  is mid-flight.

**The Claude Code plugin**

A packaged plugin that bundles the MCP server with the architect/executor
workflow as skills and slash commands:

- `/rexymcp:architect` — bootstrap a project, explore it, and author phase docs.
- `/rexymcp:dispatch` — send the executor at a drafted phase via `execute_phase`.
- `/rexymcp:review` — review a completed phase against the Definition of Done and
  either approve it or file a bug.
- `/rexymcp:escalate` — decide what to do with a `hard_fail` briefing
  (refined re-dispatch, session takeover, or resume).

## How it works

A single `execute_phase` call runs a headless agent loop against a phase spec and
a target repo:

1. Load the engineering standards + the phase doc; build the executor prompt
   (with the embedded executor contract).
2. Drive the local model through a tool-using loop — every file and shell
   operation **scoped to the target-repo root**.
3. Run model output through the **forgiving parser** that repairs malformed tool
   calls instead of just failing.
4. After edits, run the project's verifier (typecheck/build) and feed
   diagnostics back for a retry.
5. The **governor** watches for repetition loops, repeated verifier failures, and
   budget overflow — if the model is stuck, it assembles a **briefing** and hands
   control back to Claude.
6. On clean completion, run the project's full `format` / `build` / `lint` /
   `test` command set and return a structured `PhaseResult`.

Escalation isn't a cloud call — it's **Claude itself**. A stuck executor returns
its briefing up through MCP, and Claude (already the architect) picks a lever:
re-dispatch with a sharper spec, take the session over, or resume. Every turn is
also written to a redacted **JSONL session log** Claude can query on demand (via
`executor_log_search` / `executor_log_tail` / `get_turn`), so a lean result never
means lost detail. rexyMCP never links a cloud LLM provider.

## Installation

Requires a recent Rust toolchain (`edition = "2024"`) and a running
OpenAI-compatible LLM endpoint (vLLM, LM Studio, or Ollama).

**1. Build and install the binary**

```bash
cargo install --path mcp     # puts `rexymcp` on your $PATH
# or, for development:
cargo build
```

**2. Write a config file** (`rexymcp.toml`) pointing at your local model:

```toml
[executor]
provider = "lmstudio"                       # "openai" | "ollama" | "lmstudio"
model    = "qwen2.5-coder"                  # model id as your endpoint knows it
base_url = "http://localhost:1234/v1"       # OpenAI-compatible endpoint
# api_key = "..."                           # optional; local endpoints usually ignore it

# Commands run as the final gate in the target project:
[commands]
format = "cargo fmt --all --check"
build  = "cargo build"
lint   = "cargo clippy --all-targets --all-features -- -D warnings"
test   = "cargo test"

[budget]
context_length  = 32768   # the model's context window in tokens
max_context_pct = 70      # % of the window the loop may fill before compacting
max_turns       = 200     # hard turn cap before budget_exceeded
escalation_slots = 1      # briefings returned to the architect per phase

# [telemetry]
# dir = "~/.rexymcp/telemetry"   # enables cross-project scorecard records
```

All values have defaults; a missing config falls back to a local LM Studio
endpoint. Config values can be overridden by environment variables.

**3. Confirm rexyMCP can reach the model:**

```bash
rexymcp health --config rexymcp.toml
rexymcp health --base-url http://localhost:11434/v1   # ad-hoc override
```

**4. Install the Claude Code plugin** so Claude can drive the executor:

```bash
claude --plugin-dir ./plugin
```

The skills then appear as `/rexymcp:architect`, `/rexymcp:dispatch`,
`/rexymcp:review`, and `/rexymcp:escalate`, and the six MCP tools become
available to Claude. (The plugin's `.mcp.json` launches `rexymcp serve` with
`./rexymcp.toml`, so the binary must be on `$PATH`.)

**Run a phase from the CLI** (no MCP client needed):

```bash
rexymcp run-phase --config rexymcp.toml \
  --phase-doc docs/dev/phase-01.md \
  --repo /path/to/target/repo

rexymcp status --repo /path/to/target/repo    # live progress mid-run
```

## Layout

```
executor/   library — the headless single-phase agent loop
mcp/         binary  — the `rexymcp` CLI and rmcp stdio MCP server
plugin/      Claude Code plugin package — skills, slash commands, templates
docs/        architecture + the architect/executor dev process
```

## Development

rexyMCP is built phase-by-phase by its own workflow — Claude architecting, a
local model executing — dogfooding the product on itself. Contributors and AI
agents work the phase-driven process documented in
[`AGENTS.md`](AGENTS.md), [`docs/dev/STANDARDS.md`](docs/dev/STANDARDS.md), and
[`docs/dev/WORKFLOW.md`](docs/dev/WORKFLOW.md). The full design lives in
[`docs/architecture.md`](docs/architecture.md). The gates:

```bash
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
