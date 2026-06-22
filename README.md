# rexyMCP

**Let Claude|Antigravity be the *Architect*. Let a local model do the *execution*.**

rexyMCP is named after Rexy, my cattle dog. Rexy lives to herd — to keep the
flock moving in one direction, to circle back the stragglers, to never let the
work wander off. **rexyMCP herds your local LLM the same way.** Claude Code is
the *Architect*: it decomposes your idea into a product plan with milestones and
spec'd phases, then reviews every result. A local model is the *Executor*: it
writes the code, one bounded phase at a time, on an OpenAI-compatible endpoint
(vLLM, LM Studio, Ollama). And in between — keeping the Executor on task,
in-bounds, and honest, turn after turn — runs rexyMCP: parsing its messy tool
calls, catching its loops before they spiral out of control, feeding compiler
errors straight back into the next turn, and refusing to let the LLM touch anything
outside the repo. The Architect designs; the Executor codes; **rexyMCP keeps the
herd together and moving toward the goal.**

Your *Architect* runs in either **Claude Code** or **Google Antigravity** —
rexyMCP ships the same skills and MCP tools to both, so the workflow is identical
whichever you drive.

Two things make rexyMCP stand out:

1. **It is purpose-built for small local models.** A 7B–27B model *will* make
   mistakes a frontier model never would. So every layer of the loop has a
   specific answer: a forgiving tool-call parser that *repairs* malformed output
   instead of aborting the turn, a post-edit verifier that feeds compiler
   diagnostics back for an immediate retry, a governor that catches repetition
   and runaway output, security-scoped tools that can't escape the target repo,
   and a read-before-edit rule that stops blind overwrites. Models that would be
   too unreliable to use unguarded become productive executors for bounded,
   spec-driven work — because the plumbing corrects their failure modes
   automatically.

2. **It runs a virtuous cycle of continuous improvement.** Every dispatch is a
   labeled data point — gates, parse-failure rate, length-truncation rate,
   turns, peak context, the Architect's verdict — written to a shared telemetry
   store. `rexymcp runs`, `rexymcp scorecard`, and `rexymcp profile` turn that
   passive exhaust into evidence: *which model, at which settings, on which kind
   of work, actually earns its keep on your codebase.* And recurring failure
   patterns fold back into the workflow contract that governs all future phases.
   The tool gets measurably better the more you use it.

---

## Table of contents

- [How it works (the herd loop)](#how-it-works-the-herd-loop)
- [Watch it work — the live dashboard](#watch-it-work--the-live-dashboard)
- [Install & quick start](#install--quick-start)
- [The plugin (Claude Code & Antigravity)](#the-plugin-claude-code--antigravity)
  - [Skills](#skills-rexymcpname)
  - [MCP tools](#mcp-tools)
- [The workflow in depth](#the-workflow-in-depth)
  - [Project docs](#project-docs--what-each-file-is-for)
  - [Milestone & phase layout](#milestone--phase-layout)
- [The CLI](#the-cli)
  - [Command reference](#command-reference)
  - [Example output](#example-output)
- [Built for small / local models](#built-for-small--local-models)
- [The improvement loop](#the-improvement-loop)
- [Configuration — `rexymcp.toml` in full](#configuration--rexymcptoml-in-full)
- [Under the hood — the executor](#under-the-hood--the-executor)
- [Repository layout](#repository-layout)
- [Development](#development)

---

## How it works (the herd loop)

```
  you
   │
   ▼
/rexymcp:architect          ← Claude explores your repo and writes the design
   │
   ├─ bootstrap (first run) ─→ rexymcp.toml · STANDARDS.md · WORKFLOW.md
   │                            REXYMCP.md (+ CLAUDE.md / .agents/AGENTS.md)  (idempotent — safe to re-run)
   │
   ├─ writes docs/architecture.md
   ├─ writes docs/dev/milestones/M1-<slug>/README.md
   └─ writes docs/dev/NEXT.md  ───────────────────────────────────────────┐
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:architect next     ← Claude drafts the next phase doc (spec + tests + DoD)
   │
   └─ writes docs/dev/milestones/M<n>-<slug>/phase-<m>-<slug>.md ─────────┐
                                                                          │
         ┌────────────────────────────────────────────────────────────────┘
         ▼
/rexymcp:dispatch <phase>   ← Claude calls execute_phase over MCP
   │                            (the local LLM does the work; rexyMCP herds it)
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
/rexymcp:review <phase>     ← Claude reruns format/build/lint/test + checks the DoD
   │
   ├─ approved  → status → done; NEXT.md advances; repeat from "architect next"
   └─ bounced   → bug filed; executor re-dispatched with the fix spec
```

The MCP boundary is load-bearing. The Executor's inner transcript stays opaque;
Claude sees only a structured **`PhaseResult`** — status, files changed, diff,
command outputs, an Update Log entry, and (on failure) a tight `briefing`.
rexyMCP **never links a cloud provider**: escalation returns the briefing to
Claude rather than calling out to any cloud LLM itself.

**Watch a run live:**

```bash
rexymcp dashboard --repo /path/to/your/repo   # full-screen TUI — stays open, auto-follows sessions
rexymcp status    --repo /path/to/your/repo   # one-shot summary; scriptable with --json
```

---

## Watch it work — the live dashboard

`execute_phase` is opaque *and* blocking: once Claude dispatches a phase, the
local model can churn for minutes while the MCP call sits there returning
nothing until it's done. That's the right boundary for protecting Claude's
context — but it leaves *you* with no idea what's happening. **`rexymcp
dashboard` is the window into that black box.**

```bash
rexymcp dashboard --repo /path/to/your/repo
```

![The rexymcp dashboard](docs/rexymcp_dashboard.png)

Point it at your target repo and it opens a full-screen TUI that polls the
executor's session JSONL every 500 ms, **auto-following** whichever session is
currently live (start a new dispatch and the dashboard jumps to it — no
restart). It stays open until you press `q`/`Esc`/`Ctrl-C`. At a glance you see
exactly what rexyMCP is working on:

- **Session** — which milestone, phase, model, and session is running; current
  state, turn count, and stage, with a dog-chases-its-brain liveness spinner so
  you can tell motion from a stall.
- **Budget** — tokens in/out and tok/s, plus a **Savings** block pricing the
  local run against a hypothetical cloud baseline (Baseline / Executor / Net
  across Session · Milestone · Project).
- **Context** — context-window utilization (green/yellow/red) and how many
  tokens each reclaim lever has recovered.
- **Activity** — the full, scrollable transcript of the run: every prompt, tool
  call, tool result, verifier outcome, and parse repair, color-coded by type and
  filterable with `f`. This is where you *watch the model think and act*.
- **Tasks** — the phase's Spec-seeded checklist, checked off live as the
  executor completes each step.
- **Files** — every file the executor has touched, with its `+added / -removed`
  numstat.

It is a **read-only monitoring view** — it never drives the executor, only
reflects it (see [Non-goals in the
architecture](docs/architecture.md#non-goals)).

### Pairs perfectly with tmux + Claude Code

The dashboard really shines in a **tmux split alongside Claude Code**:

- One pane runs **Claude Code** as the Architect — you `/rexymcp:dispatch` a
  phase and Claude blocks on the `execute_phase` MCP call.
- The other pane runs **`rexymcp dashboard --repo .`** — and immediately lights
  up with the live run Claude just kicked off, auto-following the new session.

So while Claude waits on the opaque MCP boundary, you get a real-time feed of
exactly what the local model is doing turn-by-turn: which tool it just called,
whether the verifier is happy, how much context it's burned, which tasks are
left. When the phase finishes, Claude surfaces the `PhaseResult` in its pane and
you've already watched the whole thing happen in the other. It turns a
minutes-long black box into a glass cockpit.

---

## Install & quick start

rexyMCP drives projects in **any programming language** — the executor calls
language-neutral file/search/shell tools and runs *your* project's own
format/build/lint/test commands. (rexyMCP itself is written in Rust, so building
the binary needs a Rust toolchain; the projects it works on do not.)

Requires a recent Rust toolchain (to build rexyMCP) and a running
OpenAI-compatible LLM endpoint (vLLM, LM Studio, or Ollama).

**1 — Build & install the binary**

```bash
cargo install --path mcp     # puts `rexymcp` on your $PATH
# or, for development:
cargo build
```

**2 — Scaffold a config** in your target repo:

```bash
cd /path/to/your/repo
rexymcp init                 # writes rexymcp.toml with a generated [project] id
```

Then point `[executor]` at your local model (see
[Configuration](#configuration--rexymcptoml-in-full) for every option):

```toml
[executor]
provider = "lmstudio"                   # "openai" | "ollama" | "lmstudio"
model    = "qwen2.5-coder"              # model id as your endpoint knows it
base_url = "http://localhost:1234/v1"   # OpenAI-compatible endpoint

[commands]                              # YOUR project's commands — any language
format = "prettier --check ."           #   Rust: cargo fmt --all --check
build  = "npm run build"                #   Go:   go build ./...
lint   = "eslint ."                     #   Python: ruff check
test   = "npm test"                     #   …whatever your stack uses
```

These four commands are the only language-specific part of the setup — point
them at whatever your project uses (`cargo`, `npm`, `go`, `pytest`, `make`, …).

**3 — Confirm rexyMCP can reach the model, and the toolchain is present:**

```bash
rexymcp health --config rexymcp.toml    # endpoint reachable? list models
rexymcp doctor --config rexymcp.toml    # are the [commands] binaries on PATH?
```

**4 — Install the plugin** (see [the plugin section](#the-plugin-claude-code--antigravity) for all options, including Google Antigravity):

```bash
claude plugin install ./plugin          # from the local checkout
```

**5 — Bootstrap your project** — open Claude Code (or Google Antigravity) *in your target repo* and run:

```
/rexymcp:architect
```

The skill detects your build/test commands, writes the workflow files, and walks
you through the first design session. Everything is idempotent — safe to re-run.

> Tip: you don't even need an MCP client to drive a phase. `rexymcp run-phase`
> runs one end-to-end from the shell and prints the `PhaseResult` as JSON.

---

## The plugin (Claude Code & Antigravity)

The plugin (`plugin/`, manifest version **0.1.1**) bundles the MCP server with
the architect/executor workflow as **four skills** and **seven MCP tools**. Its
`.mcp.json` launches `rexymcp serve --config ./rexymcp.toml`, so the binary must
be on `$PATH`. The same package drives both **Claude Code** and **Google
Antigravity** — they consume the identical skills and MCP tools, so the workflow
is the same in either harness.

### Claude Code

```bash
# A — test mode (no permanent install; good for trying it out)
claude --plugin-dir ./plugin

# B — persistent install from the local checkout
claude plugin install ./plugin

# C — install from GitHub via the repo-root marketplace.json
claude plugin install github:<owner>/rexyMCP
```

The marketplace manifest lives at the **repo root**
(`.claude-plugin/marketplace.json`) and points at `source: "./plugin"`. To host
a fork or private mirror, point at your own URL:

```bash
claude plugin install git+https://your-host/rexyMCP.git
```

### Google Antigravity

Add the plugin to your global customization root by copying or symlinking the
`plugin/` directory there (e.g. `~/.gemini/config/plugins/rexymcp-plugin`), then
register the MCP server in your global `~/.gemini/config/mcp_config.json`:

```json
{
  "mcpServers": {
    "rexymcp": {
      "command": "rexymcp",
      "args": ["serve", "--config", "./rexymcp.toml"]
    }
  }
}
```

Antigravity auto-loads project rules from `.agents/AGENTS.md` (the bootstrap
writes one pointing at `REXYMCP.md`), and the architect skill uses Antigravity's
`ask_question` tool wherever Claude Code would use an interactive prompt.

### Skills (`/rexymcp:<name>`)

| Skill | Model | Args | What it does |
|---|---|---|---|
| **`/rexymcp:architect`** | opus | `[next]` | Bootstrap a project (idempotent), explore & design it (`architecture.md`, milestone README, `NEXT.md`), and author phase docs. `next` drafts the next phase. Pre-injects worked examples, codebase idioms, and fetched API docs into each phase doc — the executor has no web access and can't ask questions mid-run. Never executes a phase or crosses a milestone boundary without human sign-off. |
| **`/rexymcp:dispatch`** | sonnet | `<phase>` | Thin glue around `execute_phase`: reads `NEXT.md` + the phase doc + `rexymcp.toml`, runs an `executor_health` pre-flight, then dispatches. On `complete` it surfaces the summary and suggests `/rexymcp:review`; on `hard_fail`/`budget_exceeded` it surfaces the briefing and suggests `/rexymcp:escalate`. |
| **`/rexymcp:review`** | opus | `<phase>` | Reviews a phase whose status is `review`: reruns the `[commands]` set (format/build/lint/test as separate invocations), walks the STANDARDS Definition of Done and the phase's acceptance criteria, spot-checks that tests are real. Pass → records the verdict via `rexymcp review`, flips status to `done`, advances `NEXT.md`, commits (and writes the milestone retrospective if it's the last phase). Fail → files a bug, bounces status to `in-progress`. |
| **`/rexymcp:escalate`** | opus | `<phase>` | Decides what to do with a `hard_fail` briefing: **refined re-dispatch** (default — tighten the spec, add a Notes-for-executor block, keep status `in-progress`), **session takeover** (last resort — the Architect implements it directly and records an `escalated` verdict), or **resume** (reserved). |

### MCP tools

The `rmcp` stdio server (`rexymcp serve`) exposes **seven** tools to Claude Code:

| Tool | What it does |
|---|---|
| `execute_phase` | Run the executor against a phase doc + target repo; returns a `PhaseResult`. The `repo_path` is corroborated against the MCP client's `roots/list` and the project-dir env var (`CLAUDE_PROJECT_DIR` / `ANTIGRAVITY_PROJECT_DIR`) — a mismatch refuses the call. Params: `phase_doc_path`, `repo_path`, optional `model`. |
| `executor_health` | Check connectivity to the configured LLM endpoint and list models. Optional `base_url` override. |
| `executor_log_search` | Search a session JSONL log by `event_type` (exact), `tool_name` (substring, on tool events), and/or `query_text` (substring) — all AND-ed. Capped per-record, limit default 20 / max 50. |
| `executor_log_tail` | Return the last *N* records of a session log (default 10, max 50), each capped per field. |
| `get_turn` | Return **all** records for a single turn, uncapped — the raw-detail escape hatch. |
| `model_scorecard` | Aggregate cross-project `PhaseRun` telemetry into a **model × tag competency matrix**: gates pass rate, reliability means (parse-failure / repairs / tool-success / verifier-retries), efficiency (turns / wall-clock), escalation rate, and supervision metrics. Filter by `tags` (AND), `model`, `min_runs`. |
| `model_profile` | Aggregate telemetry into a **per-(model, tag) capability profile**: strengths (gate-pass and approved-first-try rates, reliability means) plus ranked failure classes with counts. Non-attributable classes (`spec_bug`, `infra_blip`) are separated from the model's real weaknesses. |

---

## The workflow in depth

### Project docs — what each file is for

When rexyMCP bootstraps a target project it writes four files into `docs/dev/`
(plus a `REXYMCP.md` orientation file at the root — pulled into the Architect's
context by a one-line `@REXYMCP.md` import in `CLAUDE.md` for Claude Code, or by
a rule in `.agents/AGENTS.md` that directs Antigravity to read it) that the
Architect and Executor use every session. Understanding them is the key to using
the workflow correctly.

| File | Who reads it | What it contains |
|---|---|---|
| `docs/architecture.md` | Architect, you | The design: layers, data flows, non-goals, milestone roadmap. The **single source of truth** when architecture questions come up. The Architect writes it; nothing else touches it without explicit authorization. |
| `docs/dev/STANDARDS.md` | Executor, reviewer | The **Definition of Done**. The executor reads it at the start of every phase; the reviewer checks against it at the end. Holds the build/lint/test command set, the project's hard rules (language-appropriate — e.g. no unchecked error handling, no unauthorized dependencies), and the error model. Generated from a template with the project's commands substituted in. |
| `docs/dev/WORKFLOW.md` | Architect, executor | The **process rules**: phase lifecycle (`todo → in-progress → review → done`), the phase-doc and milestone-README templates, the Update Log / bug-report / review-verdict formats, and the calibration-fold discipline. The copy embedded in the plugin is canonical — changes propagate to every new project that bootstraps from it. |
| `docs/dev/NEXT.md` | Executor (read first, every session) | A single pointer to the active phase doc. At a milestone boundary it says "none" — the **human gate** before the next milestone starts. |

`docs/architecture.md` wins all ties: **architecture > active phase doc >
STANDARDS**. Spot a conflict → file a blocker, don't pick a side.

### Milestone & phase layout

Each milestone lives in its own directory. Phases are numbered in dispatch
order; bugs filed against a phase live in a `bugs/` subdirectory.

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
criteria, a test plan, end-to-end verification, explicit authorizations (what
the executor may touch), explicit out-of-scope, and an Update Log the executor
fills in as it works. Because the local executor has no web access and cannot
ask clarifying questions mid-run, the Architect *pre-injects* worked examples,
codebase idioms, and fetched reference docs directly into each phase doc.

A **milestone README** tracks the phase table (number, slug, link, status), the
milestone's exit criteria, and a retrospective written by the Architect at
close.

---

## The CLI

`rexymcp` is one binary with twelve subcommands. Flags are all long-form; there
are no short aliases.

### Command reference

| Command | Purpose | Key flags |
|---|---|---|
| `rexymcp init` | Scaffold `rexymcp.toml` in a directory (generates the `[project] id`). | `--dir <path>` (default `.`), `--force` |
| `rexymcp health` | Connectivity check against the configured endpoint; lists models. | `--config <path>`, `--base-url <url>` |
| `rexymcp doctor` | Verify the toolchain is installed: Tier-0 `[commands]` binaries (required — a missing one exits non-zero) and Tier-1 verifier enhancers (`cargo` / `tsc` / `ruff`, advisory / fail-open). | `--config <path>`, `--json` |
| `rexymcp calibrate <TIER>` | Write tier-derived budget defaults (`tier`, `max_turns`, `gate_retries`, `[escalation]`) to the config, preserving comments and explicit overrides. | `<TIER>` = `LARGE`\|`MEDIUM`\|`SMALL`; `--config <path>` |
| `rexymcp run-phase` | Run a single phase from the shell; prints the `PhaseResult` as JSON. No MCP client required. | `--config`, `--phase-doc`, `--repo` (all required), `--model` |
| `rexymcp serve` | Start the MCP stdio server. | `--config <path>` (required) |
| `rexymcp status` | One-shot session summary (human or `--json`). The lightweight, scriptable alternative to `dashboard` for CI / piping. | `--repo <path>` (required), `--session <substr>`, `--json` |
| `rexymcp dashboard` | Live full-screen TUI over the session JSONL (see panels below). Stays open and auto-follows new sessions. | `--repo <path>` (required), `--session <id>`, `--config <path>` (drives cost rates) |
| `rexymcp runs` | List individual `PhaseRun` records with per-run stats. | `--config` (required), `--model`, `--tag` (repeatable, AND), `--limit <n>` (default 20; `0` = all), `--telemetry-path`, `--json` |
| `rexymcp scorecard` | Aggregate runs into a **model × settings** competency matrix — compare e.g. `temp=0.2` vs `temp=0.7` for your model on your work. | `--config` (required), `--model`, `--tag` (repeatable), `--min-runs <n>`, `--telemetry-path`, `--json` |
| `rexymcp profile` | Aggregate runs into a **per-(model, tag) capability profile** — strengths and ranked failure classes. | same flags as `scorecard` |
| `rexymcp review` | Record an Architect review verdict as a `PhaseReview` annotation (folds into the run's telemetry). Usually invoked by `/rexymcp:review`. | `--config`, `--phase-id`, `--verdict` (required); `--phase-doc`, `--project-id`, `--failure-class` (repeatable), `--bounces`, `--bugs-filed`, `--warnings`, `--telemetry-path` |

**Calibration tiers** set how much hand-holding the Architect provides and how
many retries the Executor gets before escalation fires:

| `<TIER>` | `executor.tier` | `budget.max_turns` | `budget.gate_retries` | `[escalation]` |
|---|---|---|---|---|
| `LARGE` | `LARGE` | `400` | unlimited (removed if present) | removed |
| `MEDIUM` | `MEDIUM` | `250` | `2` | removed |
| `SMALL` | `SMALL` | `100` | `1` | `max_assists = 3` |

`max_turns` is written only when absent, so an explicit value survives a
re-calibrate.

### Example output

**`rexymcp status`** — one-shot, human form (only emits a line when it has data):

```text
phase: phase-01-task-coverage  session: 1781658-qwen
model: Qwen/Qwen3.6-27B-FP8
state: running
turn 42, stage verify
reclaimed: 18432 tokens (filter 12000, evict 4096, dedupe 2336, compaction 0)
tasks: 3/7 done (1 active)
last update: 5s ago
```

Add `--json` for the full `StatusSummary` (token counts, context %, throughput
stats, per-file numstat, the task list) — ideal for CI.

**`rexymcp runs --limit 3`** — one row per dispatch:

```text
AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT             SERVED_MODEL   TRUNC  CXT_WIN  PEAK_CXT  RECLAIMED
2h      qwen3  dashboard      seed=42      ✓✓✓✓   66     complete  approved_first_try  qwen3.6-27b    0%     128k     74%       18k
5h      qwen3  dashboard      seed=42      ✓✓✓✗   250    budget_…  —                   qwen3.6-27b    4%     128k     91%       40k
1d      gemma  api,parser     default      ✓✓✓✓   88     complete  approved_after_1    gemma2-9b      2%     32k      63%       7k
```

`GATES` is four chars in `format/build/lint/test` order (`✓`/`✗`). `TRUNC` is the
length-finish rate (responses cut off at the token limit); `PEAK_CXT` is peak
context-window utilization; `RECLAIMED` sums every token the context optimizer
recovered.

**`rexymcp scorecard --min-runs 5`** — the `model × settings` matrix:

```text
MODEL  SETTINGS          N  GATES  PARSE_FAIL  LENGTH_FIN  AFT_RATE  TURNS_MEAN  PEAK_CXT  RECLAIMED
qwen3  temp=0.2         12   0.92       0.03          0%      0.67       78.50       81%        22k
qwen3  temp=0.7          9   0.78       0.11          0%      0.44      112.30       88%        40k
gemma  default           7   0.71       0.18          2%      0.29      131.00       77%        15k
```

`GATES` = mean gate-pass rate, `AFT_RATE` = approved-first-try rate — read down
the column to see which settings actually win on your codebase. `rexymcp
profile` complements this by ranking each model's *failure classes* so you know
*how* it tends to fail, not just how often.

**`rexymcp doctor`**:

```text
# (a TypeScript/Node project — Tier 0 reflects ITS commands)
=== Tier 0 (required) ===
  [     ok] npm — format, build, test
  [     ok] eslint — lint

=== Tier 1 (advisory) ===
  [MISSING] cargo — Rust (install the Rust toolchain via https://rustup.rs)
  [     ok] tsc — TypeScript (npm install -g typescript)
  [MISSING] ruff — Python (pip install ruff)

All required tools are present.
```

Tier-0 lists *your* project's command binaries (here `npm`/`eslint`); Tier-1 is
the fixed set of verifier enhancers. Tier-1 always fails open — the missing
`cargo`/`ruff` above don't affect the exit code; only a missing Tier-0 binary
does.

**`rexymcp dashboard`** — a `ratatui` TUI refreshed every 500 ms, auto-following
new sessions. Six panels:

```text
┌─ Session ───────────────┬─ Budget ───────────────────────┬─ Context ──────────┐
│ Milestone: M21 …        │ Tokens in:  742833             │ Usage: 74% (97k/13… │
│ Phase: phase-01         │ Tokens out: 7122               │ Events: 2          │
│ Session: 1781658-qwen   │ Tok/s: 38.4  (avg 35, max 52)  │ Freed: 41k tokens  │
│ Model: Qwen/Qwen3.6-27B │ Savings   Session  Milestone … │ Filter: 18 calls … │
│ State: running          │   Baseline:  $0.50   $3.20  …  │ Evict: 6 reads …   │
│ Duration: 4m12s         │   Executor:  $0.00   $0.00  …  │ Dedupe: 3 reads …  │
│ Turn 42, stage verify   │   Net:       $0.50   $3.20  …  │                    │
│      🐕      🧠          │   Assists: 0                   │                    │
├─ Activity [f=filter] ───┴───────────────────┬─ Tasks ────┴────────────────────┤
│ [t42] bash  <your test command>             │ Tasks ▕███████▏░░░░░░░ 3/7    43%│
│   running 187 tests …                       │ ☑ Read config                   │
│ [t42] verify  typecheck → 0 new errors      │ ▶ Wire the gate                 │
│ …  (scrollable full replay) …               ├─ Files ─────────────────────────┤
│                                             │   src/…/dashboard.ts   +148 -64 │
│                                             │   src/…/render.ts       +12  -6 │
└─────────────────────────────────────────────┴─────────────────────────────────┘
  q/Esc quit · f filter · ↑↓ scroll · PgUp/PgDn page · Home/End jump
```

- **Session** — milestone / phase / session / model / state / duration / turn /
  stage, plus a dog-chases-its-brain liveness spinner while running.
- **Budget** — token counts, tok/s (with avg/max/min), and a **Savings** block
  pricing the local run against a cloud baseline:
  Baseline / Executor (always `$0.00`) / Architect / Net across
  Session · Milestone · Project columns, plus an Assists counter. `--config`
  loads `[dashboard]` and `[architect]` rates for this breakdown.
- **Context** — context-window usage (green/yellow/red), compaction events, and
  per-lever reclaim (filter / evict / dedupe).
- **Activity** — the full session transcript, scrollable, with a togglable
  activity-type filter (`f`).
- **Tasks** — the phase's Spec-seeded checklist, checked off live.
- **Files** — every changed file with its `+added / -removed` numstat.

---

## Built for small / local models

rexyMCP is designed around the reality that a 7B–27B local model **will** make
mistakes a frontier model would not. Every layer of the executor has a specific
answer to that constraint:

| Challenge | rexyMCP's answer |
|---|---|
| Small models emit malformed tool calls — trailing commas, fenced JSON, near-miss key names | **Forgiving parser** recognizes six output formats (Hermes, fenced/loose JSON, YAML, XML-variant, plain text) and applies repair transforms (fuzzy name match, param aliasing, type coercion, default-fill, JSON repair, string-escape) before giving up — and when it must give up, it feeds *model-visible* feedback instead of silently aborting the turn |
| The model loops, retrying the same broken edit | **Governor loop detector** trips on N identical consecutive tool calls (default 6) and converts it to a `hard_fail` briefing before it burns the turn budget |
| The model edits a file and breaks the build | **Post-edit verifier** runs the project's typecheck/build after every edit and injects the diagnostics back into the turn (with language-appropriate detail — e.g. compiler suggested-fixes and structured test digests for Rust, TypeScript, and Python today) so the model fixes it without a wasted round-trip |
| Small models lose track of the task or hallucinate scope | **Read-before-edit invariant**: a `patch` is refused on any file the model hasn't `read_file`'d this session (or whose mtime changed since), preventing blind overwrites |
| The context window fills up mid-phase | **Context budgeting + the M10 reclaim levers**: `max_context_pct` triggers value-ranked compaction (noisiest tool output first; the last 3 turns protected). An output filter trims and de-dups command output at the boundary (ANSI strip, a structured compressor for noisy build output, overflow spilled to a recovery file); a read-lifecycle lever evicts superseded file reads after an edit and dedupes re-reads. Every lever is metered and visible live |
| The model uses `bash` irresponsibly | **Scope confinement** restricts every file and shell op to the target-repo root, and a bash classifier blocks the destructive command categories (`rm -rf`, `git push --force`, `git reset --hard`, `mkfs`, `curl … \| sh`, publish/upload, fork bombs, …) |
| You don't know if a model is actually good for your codebase | **Per-run telemetry** records the objective outcome of every dispatch so you accumulate real evidence about which models and settings earn their keep |

The result: models too unreliable to use unguarded — Qwen 7B, Gemma 9B, a
freshly-quantized 27B — become useful executors for bounded, spec-driven work,
precisely because the loop catches and corrects their failure modes
automatically.

---

## The improvement loop

rexyMCP doesn't just *use* a workflow — it *learns from it*. Every milestone
closes with a retrospective, and every recurring failure pattern feeds back into
the process docs that govern all future work.

1. **The Architect labels every run.** Each `execute_phase` call produces a
   `PhaseRun` record; on review the Architect fills in the supervision fields
   (`architect_verdict`, `bounces_to_approval`, `bugs_filed`) via `rexymcp
   review`. Hard-fail briefings describe the blocker precisely; review bounces
   come with a structured bug report. It's structured data, not narrative, so it
   accumulates without becoming noise.
2. **Patterns surface through the scorecard.** `rexymcp runs`, `rexymcp
   scorecard`, and `rexymcp profile` make it visible which models + settings
   produce first-try approvals, how often a model truncates, and where bounces
   cluster. The data is passive — a byproduct of normal use, not a separate eval
   apparatus.
3. **Recurring failures fold into the workflow docs.** *One occurrence is data,
   two is a trend, three is a rule change.* When a failure class hits that bar
   the Architect folds the lesson into `WORKFLOW.md` (the spec-writing contract)
   or `STANDARDS.md` (the DoD), with sign-off, so every future phase starts with
   that knowledge pre-applied. The `WORKFLOW.md` embedded in the plugin is the
   canonical copy — folds propagate to every project that bootstraps from it.

**A concrete example from building rexyMCP itself:** during M10, the executor
(Qwen3.6-27B) stalled five times on phases that required editing the same
pattern across many sites — a match-arm wall here, struct-literal churn there.
The hypothesis: split work so each dispatch touches only one non-default struct
literal. To validate it, two sibling phases ran back-to-back as a controlled
A/B: 08c (1 literal, single file) landed clean in 66 turns; 08d (3 literals
across two files) stalled exactly as predicted — same pre-injection quality on
both arms. Five occurrences plus a clean A/B → a confident fold: *prefer
splitting a feature by output-struct so each dispatch touches ≤1 non-default
struct literal.* Phase 08e, drafted as the low-churn counter-shape, landed
first-try and confirmed the lever. That fold now governs every future phase.

The improvement loop turns rexyMCP's own development history into a better tool
for every project that uses it.

---

## Configuration — `rexymcp.toml` in full

Every section and every field is optional, and a **missing config file falls
back to a working local-LM-Studio default**. In practice you need `[executor]`
(a real `model` and the right `base_url`) and `[commands]` (your project's
commands, in any language); everything else has sensible defaults. Four environment variables
override `[executor]` at load time (env beats TOML): `REXYMCP_PROVIDER`,
`REXYMCP_MODEL`, `REXYMCP_BASE_URL`, `REXYMCP_API_KEY`.

A fully-commented example showing every section:

```toml
# ── Project identity ──────────────────────────────────────────────
[project]
id = "caede0f9-ad0b-447f-aa43-ec09b953ed75"  # UUID from `rexymcp init`; scopes telemetry to this project

# ── The local model the executor drives ───────────────────────────
[executor]
provider = "openai"                       # "openai" | "ollama" | "lmstudio" (all OpenAI-compatible)
model    = "Qwen/Qwen3.6-27B-FP8"         # model id as your endpoint knows it   (default: "")
base_url = "http://localhost:1234/v1"     # OpenAI-compatible endpoint           (default shown)
# api_key = "..."                         # optional; local endpoints usually ignore it
first_token_timeout_secs = 600            # wait for the first token (prefill)    (default 600)
stream_idle_timeout_secs = 240            # max gap between tokens once streaming  (default 240)
# temperature = 0.2                       # sent on every request; omitted → endpoint default
# seed        = 42                        # deterministic sampling seed; omitted → none
# max_tokens  = 8192                      # per-response output ceiling (default 8192); raise for thinking models
task_tracking = true                      # seed a per-session checklist from the phase Spec (default true)
# tier = "MEDIUM"                         # "LARGE" | "MEDIUM" | "SMALL" — usually set via `rexymcp calibrate`

# ── The final command gate — YOUR project's commands, any language ──
#    (run in order: format → build → lint → test)
[commands]
format   = "prettier --check ."           # Rust: cargo fmt --all --check · Go: gofmt -l .
build    = "npm run build"                # Rust: cargo build · Go: go build ./...
lint     = "eslint ."                     # Rust: cargo clippy … · Python: ruff check
test     = "npm test"                     # Rust: cargo test · Python: pytest · Go: go test ./...
# lint_fix = "eslint --fix ."             # optional auto-fix command

# ── Turn / context budget ─────────────────────────────────────────
[budget]
context_length   = 32768                  # the model's context window in tokens  (default 32768)
max_context_pct  = 70                     # % of the window to fill before compacting (default 70)
max_turns        = 200                    # hard turn cap before budget_exceeded   (default 200)
escalation_slots = 1                      # briefings returned to the architect per phase (default 1)
# gate_retries   = 2                      # gate-retry loops before escalation; omitted → derive from tier

# ── Cross-project telemetry store (enables the scorecard) ─────────
[telemetry]
dir = "~/.rexymcp/telemetry"              # ~ is expanded; omit the section to disable telemetry

# ── Dashboard "$ saved" baseline (hypothetical cloud price) ───────
[dashboard]
saved_input_per_mtok  = 3.0               # USD / Mtok cloud input   (0.0 → "—" in the Budget panel)
saved_output_per_mtok = 15.0              # USD / Mtok cloud output
# saved_model = "claude-opus-4-8"         # known Claude id → auto-fills the two rates above

# ── Architect-escalation cost (a real cost center) ────────────────
[architect]
# model = "claude-opus-4-8"               # known Claude id → auto-fills the rates below
input_per_mtok  = 5.0                      # USD / Mtok architect input
output_per_mtok = 25.0                     # USD / Mtok architect output

# ── M10 context output-filter kill-switch ─────────────────────────
[context]
output_filter = true                      # false → raw head+tail truncation, no recovery file (default true)

# ── Governor hard-fail thresholds ─────────────────────────────────
[governor]
identical_call_threshold        = 6       # consecutive identical tool calls → hard-fail (default 6)
verifier_persistence_threshold  = 6       # consecutive verifier-failing turns → hard-fail (default 6)
runaway_output_bytes            = 102400  # single tool-output byte cap → hard-fail (default 100 KiB)

# ── SMALL-tier mid-phase architect assists ────────────────────────
[escalation]
max_assists = 3                           # autonomous architect assists before hard_fail (default 3)

# ── Per-model overrides (exact model-id match) ────────────────────
[models."Qwen/Qwen3.6-27B-FP8"]
temperature                    = 0.2      # any of these override the global value for this model only
# seed                         = 7
# max_tokens                   = 16384     # per-model output ceiling (thinking models often need more)
# task_tracking                = false
# identical_call_threshold     = 8
# verifier_persistence_threshold = 8
# runaway_output_bytes         = 204800
```

**Section reference:**

| Section | Purpose |
|---|---|
| `[project]` | `id` — per-project UUID (from `rexymcp init`) that scopes telemetry to this project regardless of path. |
| `[executor]` | The local model + connection: `provider`, `model`, `base_url`, `api_key`, the two streaming timeouts, sampling (`temperature`, `seed`, `max_tokens`), `task_tracking`, and `tier`. |
| `[commands]` | The `format` / `build` / `lint` / `test` (+ optional `lint_fix`) commands run as the final gate. |
| `[budget]` | `context_length`, `max_context_pct`, `max_turns`, `escalation_slots`, `gate_retries`. |
| `[telemetry]` | `dir` — the cross-project store. Omit to disable; `~` is expanded. |
| `[dashboard]` | Cloud-baseline `$/Mtok` rates for the Budget panel's Savings block (or a `saved_model` to auto-fill them). |
| `[architect]` | `$/Mtok` rates for the **real** cost of architect-escalation assists (or a Claude `model` to auto-fill). |
| `[context]` | `output_filter` kill-switch for the M10 boundary filter. |
| `[governor]` | The three hard-fail thresholds (identical-call, verifier-persistence, runaway-output). |
| `[escalation]` | `max_assists` for SMALL-tier autonomous architect assists. |
| `[models."<id>"]` | Per-model overrides (exact-id match) for sampling, task-tracking, and the governor thresholds. |

**Known-model rate table** (recognized by `[architect] model` and `[dashboard]
saved_model`, in USD/Mtok input/output): `claude-opus-4-8`/`-4-7`/`-4-6` →
5/25 · `claude-sonnet-4-6` → 3/15 · `claude-haiku-4-5` → 1/5 ·
`claude-fable-5`/`claude-mythos-5` → 10/50. Anything else falls back to the
explicit rate fields.

---

## Under the hood — the executor

The `executor` crate is the headless single-phase agent loop. The turn cycle is
**parse → tool dispatch (via the governor) → verify → final command gate**.

- **OpenAI-compatible AI client** (`ai/`) — streaming SSE with usage accounting,
  a three-layer retry stack (circuit breaker, HTTP 429/5xx retry, stream
  retry), and a pluggable `AiClient` trait with a `MockAiClient` for hermetic
  tests. Captures the served-model id and a full token breakdown
  (input/output/cache-read/cache-write).
- **Forgiving tool-call parser** (`parser/`) — detects six output formats and
  applies repair transforms (fuzzy name-match, param aliasing, type coercion,
  default-fill, JSON repair, string-escape) before falling back to model-visible
  feedback. Built for exactly the malformations small models emit.
- **Eleven built-in tools** (`tools/`), every operation scoped to the repo root:
  `read_file`, `write_file`, `patch` (exact-substring edit; read-gated),
  `patch_lines` (line-range edit), `delete_file`, `move_file`, `search` (regex,
  gitignore-aware, ripgrep engine), `find_files` (glob), `symbols` (tree-sitter
  definitions & references — Rust & Python), `bash`, and `update_task`.
- **Security scope confinement** (`security/`) — file and shell ops cannot escape
  the repo root (no `..`, absolute-path, or symlink escape); a bash classifier
  blocks the destructive command categories; a read-before-edit rule (in the
  agent loop) refuses a `patch` on an unseen or changed file.
- **Governor** (`governor/`) — trips a `hard_fail` on identical-call loops,
  persistent verifier failure, or runaway single-tool output, each on a
  configurable threshold; scores per-tool reliability with Laplace smoothing.
- **Verifier** (`agent/verify.rs`) — runs after every successful edit against a
  session-start baseline (so ambient errors aren't blamed on the model), feeds
  back rustc machine-applicable fixes plus `cargo check` / `tsc --noEmit` /
  `ruff` diagnostics, and **fails open** when a toolchain binary is missing.
- **Context optimization** (`context/`) — value-ranked compaction at the budget
  ceiling, an output filter that trims/de-dups command output (overflow spilled
  to a recovery file), and a read-lifecycle lever that evicts superseded reads
  and dedupes re-reads. Every reclaimed token is metered.
- **Structured task tracking** (`agent/tasks.rs`) — the loop seeds a checklist
  from the phase's `## Spec` and the executor flips items `pending → active →
  done`. It *tracks* the list; decomposition stays the Architect's job. Catches
  dropped-subtask stalls and premature completion. Toggle via `[executor]
  task_tracking`.
- **Hard-fail briefings** (`phase/briefing.rs`) — when the model is genuinely
  stuck, the loop assembles a tight brief (goal, acceptance criteria,
  diagnostics, ≤5 working files, what was tried, current blocker, budget
  remaining) and hands control back to Claude instead of burning the budget.
- **Final command gate** (`agent/command.rs`) — on clean completion, runs the
  configured commands in order **format → build → lint → test** and only reports
  success if they pass.
- **`PhaseResult`** (`phase/result.rs`) — the single value that crosses the MCP
  boundary: `status` (`complete` / `hard_fail` / `budget_exceeded`), files
  changed, the diff, per-command outputs, an Update Log entry, an optional
  `briefing`, and the session-log path.
- **Per-run telemetry** (`store/telemetry.rs`) — every dispatch appends a
  `PhaseRun` record: model, sampling params, served-model id, status, gates,
  turns, wall-clock, token breakdown, context window, `length_finish_rate`,
  decomposed reliability components (parse-failure rate, repairs-per-call,
  tool-success rate, verifier-retries), context-efficiency (peak context % and
  tokens reclaimed by source), `project_id`/`milestone_id`, tier telemetry, and
  — at review — the Architect's verdict.
- **JSONL session log** (`store/sessions/`) — every turn (prompts, parsed tool
  calls, tool results, verifier runs, progress, compaction/reclaim events, task
  updates) is written to `session-<id>.jsonl`, so a lean `PhaseResult` never
  means lost detail. The `executor_log_search` / `executor_log_tail` / `get_turn`
  MCP tools read it back.

---

## Repository layout

```
executor/   library — the headless single-phase agent loop (crate `rexymcp-executor`)
mcp/        binary  — the `rexymcp` CLI and rmcp stdio MCP server (bin `rexymcp`)
plugin/     plugin package (Claude Code & Google Antigravity)
  ├── .claude-plugin/
  │     └── plugin.json           Claude Code plugin manifest (name, version, description)
  ├── plugin.json                 Antigravity plugin manifest
  ├── .mcp.json                   MCP server registration → `rexymcp serve --config ./rexymcp.toml`
  ├── skills/
  │     ├── architect/SKILL.md    /rexymcp:architect
  │     ├── dispatch/SKILL.md     /rexymcp:dispatch
  │     ├── review/SKILL.md       /rexymcp:review
  │     └── escalate/SKILL.md     /rexymcp:escalate
  └── templates/
        ├── STANDARDS.md          generalized DoD template (placeholders resolved per project)
        └── WORKFLOW.md           generalized workflow template
.claude-plugin/
  └── marketplace.json            marketplace metadata (install via GitHub URL); source → ./plugin
.agents/
  └── AGENTS.md                   Antigravity project rules → read REXYMCP.md before working
docs/       architecture + the architect/executor dev process for rexyMCP itself
```

---

## Development

rexyMCP is built phase-by-phase by its own workflow — Claude architecting, a
local model executing — dogfooding the product on itself. Contributors and AI
agents work the phase-driven process documented in
[`REXYMCP.md`](REXYMCP.md), [`docs/dev/STANDARDS.md`](docs/dev/STANDARDS.md), and
[`docs/dev/WORKFLOW.md`](docs/dev/WORKFLOW.md); the full design lives in
[`docs/architecture.md`](docs/architecture.md). The gates (run as separate
invocations, not chained):

```bash
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
