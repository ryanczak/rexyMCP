# rexyMCP

**Let Claude be the *Architect*. Let a local model do the execution.**

**rexyMCP** is named after Rexy, my cattle dog. Rexy loves to *herd*. **rexyMCP** 
keeps your local LLM moving in the right direction, nudges it forward, keeps
it on task and returns completed work. Claude Code is the *Architect* and **rexyMCP** 
shepherds the *Executor*: a local model running on an OpenAI-compatible endpoint 
such as LM Studio, Ollama, or vLLM. The *Executor* does the coding. The *Architect* 
decomposes your idea into a product plan with milestones and spec'd phases; 
the *Executor* codes each phase to spec and the *Architect* reviews the output, all while 
**rexyMCP** keeps the local model on task, in-bounds, and honest using realtime data.

Two things make rexyMCP stand out. First, it is **purpose-built for small local
models**: a forgiving tool-call parser that repairs malformed output instead of
aborting, a post-edit verifier that feeds compiler diagnostics back into the
turn, a loop detector that catches repetition before it burns the budget, and
security-scoped tools that prevent the model from doing damage outside the
target repo. Weaker models that would be too unreliable to use unguarded become
productive executors for bounded, spec-driven work because the plumbing catches
and corrects their failure modes automatically. Second, it implements a
**virtuous cycle of continuous improvement**: every dispatch is a labeled data
point — gates, parse-failure rate, truncation rate, turns, the architect's
verdict — and recurring failure patterns fold back into the workflow contract
(`WORKFLOW.md`) that governs all future work. The tool gets measurably better
with use, and `rexymcp runs` / `rexymcp scorecard` close the loop so you can
see exactly which models and settings are earning their keep on your codebase.

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
   └─ writes docs/dev/NEXT.md  ───────────────────────────────────────────┐
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:architect next     ← Claude drafts the next phase doc
   │
   └─ writes docs/dev/milestones/M<n>-<slug>/phase-<m>-<slug>.md ─────────┐ 
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:dispatch <phase>   ← Claude calls execute_phase over MCP
   │                            (local LLM does the work)
   │
   ├─ [complete]  ──────────────────────────────────────────────────────┐
   │                                                                    │
   └─ [hard_fail / budget_exceeded]                                     │
        │                                                               │
        ▼                                                               │
   /rexymcp:escalate <phase>                                            │
        │                                                               │
        ├─ refine spec → re-dispatch ─────────────────────────────────▶┤
        └─ session takeover (Claude finishes it) ─────────────────────▶┤
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
rexymcp dashboard --repo /path/to/your/repo   # live TUI — stays open, auto-follows sessions
rexymcp status   --repo /path/to/your/repo    # one-shot summary; scriptable with --json
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
- **Per-run statistics & settings scorecard** — each run emits a `PhaseRun`
  record (model, sampling settings, served-model id, length-truncation rate,
  context window, gates, reliability/efficiency metrics, and — at review — the
  architect's verdict) into a shared telemetry store. Two CLI views let you act
  on that data: `rexymcp runs` lists individual runs with all their stats so you
  can see exactly what happened on each dispatch; `rexymcp scorecard` aggregates
  runs into a **`model × settings` competency matrix** so you can compare e.g.
  `temperature=0.2` vs `temperature=0.7` for your model on your kind of work. The
  `model_scorecard` MCP tool (Claude-facing) provides the `model × tag` slice for
  the architect to consult at dispatch time.

**The MCP server (`rexymcp serve`)**

An `rmcp` stdio MCP server exposing six tools to Claude Code:

| Tool | What it does |
|------|--------------|
| `execute_phase` | Run the executor against a phase doc + target repo; returns a `PhaseResult`. |
| `executor_health` | Check connectivity to the configured LLM endpoint and list models. |
| `executor_log_search` | Search the session JSONL log by event type, tool name, or text. |
| `executor_log_tail` | Return the last N records of a session log (capped per field). |
| `get_turn` | Return all records for a single turn, uncapped — the raw-detail escape hatch. |
| `model_scorecard` | Aggregate cross-project `PhaseRun` telemetry into a model × tag competency matrix, with per-cell sample sizes, to compare models. |

**The CLI (`rexymcp`)**

- `rexymcp health` — connectivity check against the configured endpoint.
- `rexymcp run-phase` — run a single phase from the command line (returns the
  `PhaseResult` as JSON), no MCP client required.
- `rexymcp serve` — start the MCP stdio server.
- `rexymcp dashboard` — live full-screen TUI over the session JSONL: Session,
  Budget (tokens · context % · tok/s · $ saved), Compactions, Activity transcript
  (scrollable full replay), and Files panels. Stays open and auto-follows new
  sessions. `--config` loads `[dashboard]` rates for the `$ saved` line.
- `rexymcp status` — one-shot session summary (human or `--json`). Scriptable;
  the lightweight alternative to `dashboard` for CI / piping.
- `rexymcp runs` — list individual `PhaseRun` records with per-run stats (model,
  settings, gates, reliability, provenance, verdict). Filterable by `--model` /
  `--tag`; `--json` for scripting.
- `rexymcp scorecard` — aggregate runs into a **`model × settings` competency
  matrix** so you can compare sampling settings for a given model. Filterable by
  `--model` / `--tag` / `--min-runs`; `--json` for scripting.

**The Claude Code plugin**

A packaged plugin that bundles the MCP server with the architect/executor
workflow as skills and slash commands:

- `/rexymcp:architect` — bootstrap a project, explore it, and author phase docs.
- `/rexymcp:dispatch` — send the executor at a drafted phase via `execute_phase`.
- `/rexymcp:review` — review a completed phase against the Definition of Done and
  either approve it or file a bug.
- `/rexymcp:escalate` — decide what to do with a `hard_fail` briefing
  (refined re-dispatch, session takeover, or resume).

## Built for small / local models

rexyMCP is designed around the reality that a 7B–27B local model **will** make
mistakes that a frontier model would not. Every layer of the executor has a
specific answer to that constraint:

| Challenge | rexyMCP's answer |
|---|---|
| Small models emit malformed tool calls — trailing commas, fenced JSON, near-miss key names | **Forgiving 6-stage parser** repairs on the fly and feeds model-visible feedback when it can't, rather than silently failing or aborting the turn |
| The model gets into a loop, retrying the same broken edit | **Loop detector** (part of the governor) recognizes repetition patterns and converts them to a `hard_fail` briefing before they burn the turn budget |
| The model edits a file and breaks the build | **Post-edit verifier** runs the project's typecheck/build after every edit-class tool call and injects the compiler diagnostics back into the turn for a retry — the model sees what's broken and fixes it without a wasted round-trip |
| Small models lose track of the task or hallucinate scope | **Read-before-edit invariant**: the loop tracks which files the model has read this session and refuses to patch files it hasn't seen, preventing blind overwrites |
| The context window fills up mid-phase | **Context budgeting + compaction**: configurable `max_context_pct` triggers a summary compaction before the window is exhausted, so the model doesn't hit a hard cutoff in the middle of a thought |
| The model uses `bash` irresponsibly | **Security scope confinement** restricts every file and shell operation to the target-repo root, and a bash classifier blocks the command categories most likely to cause damage |
| You don't know if a model is actually good for your codebase | **Per-run telemetry** records the objective outcome of every dispatch — gates, parse-failure rate, truncation rate, turns, the architect's verdict — so over time you accumulate real evidence about which models and settings earn their keep on your kind of work |

The result is that models that would be too unreliable to use unguarded — Qwen
7B, Gemma 9B, a freshly-quantized 27B — become useful executors for bounded,
spec-driven implementation work, precisely because the loop catches and
corrects their failure modes automatically.

## The improvement loop

rexyMCP doesn't just use a workflow — it *learns from it*. Every milestone
closes with a retrospective, and every recurring failure pattern feeds back into
the process docs that govern all future work.

**How it works:**

1. **The architect labels every run.** Each `execute_phase` call produces a
   `PhaseRun` record; when the architect approves or bounces a phase, it fills
   in the supervision fields — `architect_verdict`, `bounces_to_approval`,
   `bugs_filed`. Hard-fail briefings describe the blocker precisely. Review
   bounces come with a structured bug report (severity, what's wrong, how to
   fix, verification steps). All of this is structured data, not narrative, so
   it accumulates over time without becoming noise.
2. **Patterns surface through the scorecard.** `rexymcp runs` and `rexymcp
   scorecard` make it visible which models + settings produce first-try
   approvals, how often a given model truncates, where bounces cluster. The
   data is passive — a byproduct of normal use, not a separate eval apparatus.
3. **Recurring failures fold into the workflow docs.** When a failure class
   hits two occurrences it's a trend; at three it's a documented fix. The
   calibration discipline is explicit: one occurrence is data, two is a
   trend, three is a rule change. The architect folds the lesson into
   `WORKFLOW.md` (the spec-writing contract) or `STANDARDS.md` (the DoD),
   with sign-off, so every future phase starts with that knowledge pre-applied.
   The WORKFLOW.md embedded in the plugin is the canonical copy — changes here
   propagate to every new project that bootstraps from it.

**A concrete example from building rexyMCP itself:** during M7, the executor
(Qwen3.6-27B) twice failed by mutating a shared type used at many call sites,
getting trapped by the verifier's 3-strike limit before it could finish
updating all the callers. Two occurrences → a trend → a fold: the
["Prefer additive change shapes"](docs/dev/WORKFLOW.md) rule was added to the
spec-writing contract. The very next phase, the architect used an additive new
variant instead of mutating the existing one. Zero cascade. The fold earned its
keep on the first application.

The improvement loop turns rexyMCP's own development history into a better
tool for every project that uses it.

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

# [dashboard]
# saved_input_per_mtok  = 3.0    # USD/Mtok for cloud input  — shown as "$ saved" in the TUI
# saved_output_per_mtok = 15.0   # USD/Mtok for cloud output   (0.0 = omit, show "—")
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
[`CLAUDE.md`](CLAUDE.md), [`docs/dev/STANDARDS.md`](docs/dev/STANDARDS.md), and
[`docs/dev/WORKFLOW.md`](docs/dev/WORKFLOW.md). The full design lives in
[`docs/architecture.md`](docs/architecture.md). The gates:

```bash
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
