# rexyMCP

**Let Claude be the architect. Let a local model do the coding.**

rexyMCP is named after **Rexy**, my cattle dog. Rexy *herds*. rexyMCP keeps 
your local LLM moving in the right direction, nudges them forward, 
keeps them on task and returns completed work. Claude Code is the Architect. 
rexyMCP manages the Executor: a local model running on an OpenAI-compatible 
endpoint such as LM-Studio, ollama, or vLLM. The Executor does the coding. 
Claude decomposes your idea into an architecure, milestones wth spec'd phases.
The Executor codes each phase to spec and Claude reviews what comes back —
while rexyMCP keeps the local model on task, in bounds, and honest.

## Workflow

```
  you
   │
   ▼
/rexymcp:architect          ← Claude explores your repo and writes the design
   │
   ├─ bootstrap (first run) ─→ rexymcp.toml · STANDARDS.md · WORKFLOW.md
   │                            CLAUDE.md · .mcp.json
   │
   ├─ writes docs/architecture.md
   ├─ writes docs/dev/milestones/M1-<slug>/README.md
   └─ writes docs/dev/NEXT.md  ──────────────────────────────────────────┐
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:architect next     ← Claude drafts the next phase doc
   │
   └─ writes docs/dev/milestones/M<n>-<slug>/phase-<m>-<slug>.md
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:dispatch <phase>   ← Claude calls execute_phase over MCP
   │                            (local LLM does the work)
   │
   ├─ [complete]  ──────────────────────────────────────────────────────┐
   │                                                                     │
   └─ [hard_fail / budget_exceeded]                                     │
        │                                                               │
        ▼                                                               │
   /rexymcp:escalate <phase>                                            │
        │                                                               │
        ├─ refine spec → re-dispatch ─────────────────────────────────▶┤
        └─ session takeover (Claude finishes it) ────────────────────▶┤
                                                                        │
         ┌──────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:review <phase>     ← Claude reruns commands + checks the DoD
   │
   ├─ approved  → NEXT.md updated; repeat from "architect next"
   └─ bounced   → bug filed; executor re-dispatched with the fix spec
```

**Live progress during a run:**

```bash
rexymcp status --repo /path/to/your/repo   # pull-based; works while execute_phase is running
```

## Project docs — what each file is for

When rexyMCP bootstraps a target project it writes four files into `docs/dev/`
that the architect and executor use every session. Understanding them is the key
to using the workflow correctly.

| File | Who reads it | What it contains |
|------|-------------|-----------------|
| `docs/architecture.md` | Architect (Claude), you | The design: layers, data flows, non-goals, milestone roadmap. The single source of truth when architecture questions come up. The architect writes it; nothing else touches it without explicit authorization. |
| `docs/dev/STANDARDS.md` | Executor (local LLM), reviewer | The Definition of Done. The executor reads this at the start of every phase; the reviewer checks against it at the end. Contains the build/lint/test command set, hard rules (no `unwrap`, no `panic!`, no unauthorized dependencies), and the error model. |
| `docs/dev/WORKFLOW.md` | Architect, executor | The process rules: the phase lifecycle (todo → in-progress → review → done), the phase-doc template, the Update Log format, the bug-report template, the calibration-fold discipline. The workflow is embedded in the plugin and deployed to every target project. |
| `docs/dev/NEXT.md` | Executor (reads first, every session) | A single pointer to the active phase doc. The executor opens this before anything else to know what it's working on. At a milestone boundary it says "none" — the human gate before the next milestone starts. |

`docs/architecture.md` is the source-of-truth precedence winner: if it conflicts
with a phase doc or a process doc, the architecture wins. Spot a conflict → file
a blocker, don't pick a side.

## Milestone layout

Each milestone lives in its own directory. Phases are numbered in dispatch order;
bugs filed against a phase live in a `bugs/` subdirectory.

```
docs/
└── dev/
    ├── NEXT.md                           ← active phase pointer (read first)
    ├── STANDARDS.md                      ← engineering Definition of Done
    ├── WORKFLOW.md                       ← phase lifecycle + templates
    └── milestones/
        └── M<n>-<slug>/
            ├── README.md                 ← milestone goal, exit criteria, phase table, retrospective
            ├── phase-01-<slug>.md        ← phase spec (pre-flight, goal, spec, tests, DoD, update log)
            ├── phase-02-<slug>.md
            ├── phase-02a-<slug>.md       ← parallel phases share a parent number + letter suffix
            ├── phase-02b-<slug>.md
            └── bugs/
                └── bug-<phase>-<n>.md    ← review-finding bug reports (severity, fix, verification)
```

A **phase doc** contains everything the executor needs and nothing else: goal,
architecture references, a step-by-step spec (exact files + changes), acceptance
criteria, a test plan, end-to-end verification, explicit authorizations (what the
executor may touch), explicit out-of-scope (what it must not touch), and an Update
Log the executor fills in as it works. The architect pre-injects worked examples,
codebase idioms, and fetched reference docs into each phase doc because the local
executor has no web access and cannot ask clarifying questions mid-run.

A **milestone README** tracks the phase table (phase number, slug, link, status),
the exit criteria for the milestone, and a retrospective written by the architect
at milestone close.

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
- **Cross-project telemetry** — each run emits a `PhaseRun` record for the model
  scorecard.

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
endpoint.

**3. Confirm rexyMCP can reach the model:**

```bash
rexymcp health --config rexymcp.toml
```

**4. Install the Claude Code plugin**

**Option A — test mode** (no permanent install; good for trying it out):

```bash
claude --plugin-dir ./plugin
```

**Option B — persistent install** from the local checkout:

```bash
claude plugin install ./plugin
```

**Option C — install from GitHub** using `marketplace.json`:

The plugin ships a `.claude-plugin/marketplace.json` that lets Claude Code
discover and install it directly from the repository URL:

```bash
claude plugin install github:<owner>/rexyMCP
```

The `marketplace.json` at `plugin/.claude-plugin/marketplace.json` describes the
plugin name, version, author, category, and keywords. If you are hosting a fork
or a private mirror, point at your own URL:

```bash
claude plugin install git+https://your-host/rexyMCP.git
```

After any install method, the skills appear as `/rexymcp:architect`,
`/rexymcp:dispatch`, `/rexymcp:review`, and `/rexymcp:escalate`, and the six MCP
tools become available to Claude. The plugin's `.mcp.json` launches
`rexymcp serve --config ./rexymcp.toml`, so the binary must be on `$PATH`.

**5. Bootstrap your target project**

Open a Claude Code session in your target repo (not the rexyMCP repo) and run:

```
/rexymcp:architect
```

The skill detects the project's build/test commands, writes the config files,
and walks you through the first design session. Everything is idempotent — safe
to re-run if a file is missing.

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
mcp/        binary  — the `rexymcp` CLI and rmcp stdio MCP server
plugin/     Claude Code plugin package — skills, slash commands, templates
  ├── .claude-plugin/
  │     ├── plugin.json           plugin manifest (name, version, description)
  │     └── marketplace.json      marketplace metadata (install via GitHub URL)
  ├── .mcp.json                   MCP server registration (auto-connects on enable)
  ├── skills/
  │     ├── architect/SKILL.md    /rexymcp:architect skill instructions
  │     ├── dispatch/SKILL.md     /rexymcp:dispatch skill instructions
  │     ├── review/SKILL.md       /rexymcp:review skill instructions
  │     └── escalate/SKILL.md     /rexymcp:escalate skill instructions
  └── templates/
        ├── STANDARDS.md          generalized DoD template (placeholders resolved per project)
        └── WORKFLOW.md           generalized workflow template
docs/       architecture + the architect/executor dev process for rexyMCP itself
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
