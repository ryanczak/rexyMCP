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
│   /rexymcp:architect · /rexymcp:dispatch · /rexymcp:review commands                   │
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
| Secret redaction / masking | `rexy/src/security/redact.rs`, `rexy/src/ai/filter.rs` (`mask_sensitive`, `init_masking`) | **Lift.** Applied to every turn before it is written to the session log. |
| Session JSONL log + reader | `rexy/src/store/sessions/jsonl.rs` (`SessionLogger`, `SessionRecord`, `SessionEvent`, `read_session_log`) | **Lift / adapt.** The executor's turn-by-turn record; query tools read it back. |
| Telemetry store (per-phase metrics) | `rexy/src/store/telemetry.rs` | **Lift / adapt.** Cross-project `PhaseRun` records — the substrate for the model scorecard. |
| Result truncation/formatting; error model | `rexy/src/result/`, `rexy/src/error/` | **Lift / adapt.** |
| TUI (fullscreen / classic) | `rexy/src/tui/` | **Drop.** No terminal UI; progress flows over MCP. |
| Local planner (TODO decomposition) | `rexy/src/planner/` | **Drop.** Decomposition is the architect's (Claude's) job. |
| Escalation **cloud transport** | `rexy/src/escalation/` | **Drop the transport; keep the briefing.** See below. |

#### The executor turn cycle

One `execute_phase` call runs this loop until the phase completes, hard-fails, or
hits a budget cap:

1. Assemble the executor system prompt from three inputs: the **embedded
   executor contract** (the generalized, `{…_COMMAND}`-resolved executor-behavior
   rules — see Layer 3), the project's `STANDARDS.md`, and the phase doc (with the
   architect's pre-injection). The local model reads none of these as files — the
   crate assembles them in-process.
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

Every step that produces an event — the rendered prompt, the raw completion, the
parsed `ToolCall` or `ParseFailure`, the tool result, the verifier outcome, and
the governor/hard-fail signals — is appended (redacted) to the **session log**
described below. Logging is a side effect of the loop; it never changes what the
loop returns.

#### Escalation = Claude Code itself

Rexy escalates a stuck local model to a cloud provider *inside its own process*.
rexyMCP **keeps Rexy's hard-fail detection and briefing assembly but drops the
cloud transport**. Instead of calling Anthropic, the executor returns the
briefing up through the MCP boundary, and **Claude — already the architect — is
the escalation target.** This is itself the standard MCP "return a structured
result, let the host re-invoke" round-trip; rexyMCP's `PhaseResult` *is* that
round-trip, so no extra mechanism is needed for Claude to intervene.

Given the briefing, the architect picks one of three levers, situationally:

- **Re-dispatch with a refined spec** — Claude sharpens the phase doc (often via
  pre-injection: bake in the missing idiom/constraint, see Layer 3) and runs the
  phase clean from scratch. For weak models this is frequently the *most* robust
  option: a stuck local model's conversation is often part of the problem
  (context rot), and a clean restart with a better spec discards the confusion.
- **Session takeover** — Claude finishes the phase itself when it's beyond the
  local model's reach.
- **Resume** *(candidate — not yet committed; decide later)* — a
  `continue_phase(session_id, guidance)` tool would rehydrate the executor from
  the session log and inject one targeted directive so the local model continues
  from where it was, keeping the work it already did. The cheap middle lever for
  "model was almost there, hit one specific wall." **Caveat:** resume preserves
  the local model's accumulated context rot along with its progress, so it is a
  situational lever, never the default. It carries a real cost — the M4 loop must
  be able to serialize/rehydrate resumable state (message history, working set,
  remaining turn budget) from the session log — so it is recorded here as a
  design option, not a committed feature.

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
parser's repair history, and the raw tool transcript stay inside the executor —
but they are not thrown away. They go to the session log.

#### Session log & troubleshooting tools

The `PhaseResult` (and its briefing) is deliberately lean to protect Claude's
context. That is the right default, but when a phase hard-fails it leaves Claude
with only the *summary* of what went wrong. The full record needs to exist
somewhere queryable — so the executor writes a complete **JSONL session log** to
disk, and rexyMCP exposes tools that let Claude read it back **on demand**.

The principle is **pull, not push**: the log costs nothing in Claude's context
until Claude chooses to query it while debugging a failure. The log and the
briefing are complementary — the briefing handles the common case, the log is
there for the deep dive.

- **Format.** One JSON object per line, one record per turn event. The event
  schema reuses the executor's already-`Serialize`-able types (`ToolCall`,
  `ParseFailure` with its `RepairOp` history, the verifier outcome, the
  governor/hard-fail signals). Lifted from `rexy/src/store/sessions/jsonl.rs`
  (`SessionLogger` / `SessionRecord` / `SessionEvent`).
- **Redaction.** The executor reads files from the target repo, so a turn can
  capture secrets. **Every record is passed through the lifted redaction layer
  (`security/redact.rs` + `ai/filter.rs::mask_sensitive`) before it is written.**
- **Location.** Under the target repo, namespaced and git-ignored:
  `<repo_root>/.rexymcp/sessions/<phase>-<session_id>.jsonl`. The
  `execute_phase` result reports the log path so Claude can reference it.
- **Query tools** (exposed by Layer 2). Each tool **caps and summarizes its own
  output** — a debugging query must never re-flood the context the lean
  `PhaseResult` was protecting:
  - `executor_log_search(session, query)` — grep/filter the log (by event kind,
    tool name, error text) and return matching turns, truncated.
  - `executor_log_tail(session, n)` — the last `n` turn events.
  - `get_turn(session, n)` — the full record for one turn (the one place the raw
    detail is allowed through, scoped to a single turn).

The richest signal for "why did this executor get stuck" is the
`ParseFailure`/repair history plus the verifier and hard-fail records — these are
exactly what the briefing compresses, so the log is where Claude goes when the
compression lost the detail it needs.

#### Model effectiveness metrics & the scorecard

rexyMCP is well-positioned to measure *which local model does well on which kind
of work, run with which settings* — because every phase is a spec'd unit with
objective gates and an architect grade, the labeled dataset is a byproduct of the
normal workflow, not something to manufacture. The same session log that powers
troubleshooting also feeds a per-phase metrics record. The payoff is a **human
decision aid**: when you have a fleet of local models and a pile of sampling
settings, the scorecard tells you which combination has actually been earning its
keep on work like the work in front of you.

**The `PhaseRun` record.** At the end of each `execute_phase`, the executor emits
one summary record (alongside the JSONL turn log) into a **cross-project**
telemetry store — accumulate across every repo the executor has touched, not
per-repo:

```
PhaseRun {
  model, generation_params,             // who + how (temperature, seed)
  phase_id, tags,                       // language, kind (feature|refactor|bugfix|test), size bucket
  status, escalated,                    // complete|hard_fail|budget_exceeded; did it hand off to Claude?
  // quality
  gates: { fmt, build, lint, test },    // booleans on first completion
  warnings, bugs_filed,                 // count + max severity (filled by the architect at review)
  bounces_to_approval,                  // 0 = first-try; the single most telling number
  // reliability (the small-model differentiator)
  parse_failure_rate, repairs_per_call, // from the forgiving parser
  verifier_retries, tool_success_rate,  // from the governor
  // efficiency
  turns, wall_clock_s, tokens,          // TokenBreakdown (in/out/cached)
  // supervision label (filled at review)
  architect_verdict,                    // approved_first_try | approved_after_N | rejected | escalated
}
```

The executor fills the objective fields at phase end; the architect's review
fills `bugs_filed`, `bounces_to_approval`, and `architect_verdict` — the
supervision label that turns telemetry into an eval.

**One review record, two consumers.** The architect's per-phase review verdict
(the supervision fields above, plus *which executor ran it* — local model vs.
direct — and any scope deviation / calibration note) is captured **once, as a
structured record at approval**, and serves two readers without a second pipeline:
the **machine eval** (`model_scorecard`, below — `model × tag` competency) and a
**human project review** (milestone × phase: status, verdict, bounces, deferrals,
folds — for understanding project state in depth). Same single-source discipline
as the session log: write once, derive views; never maintain parallel narratives
that drift. The architect writes this verdict at every approval (WORKFLOW.md §
"Review and Bug-Report Cycle"); until `PhaseRun` exists (M4), it lives in the phase
doc's Update Log + the milestone retrospective, which are its substrate.

**Phase tagging.** Phase-doc frontmatter carries a `Tags:` line (language, kind,
size) so runs are categorizable. The architect sets it when drafting the phase.

**Scorecard.** A Layer 2 tool, `model_scorecard`, aggregates `PhaseRun` records
into a competency matrix that answers the two questions a user actually has when
picking how to run the executor:

- *Which model for this kind of work?* — the `model × tag` slice:
  `model × tag → { n_runs, first_pass_rate, mean_turns, parse_failure_rate,
  mean_bugs, … }` with sample sizes, e.g. "Qwen leads on `rust`/`feature`, Gemma
  on `go`/*."
- *Which settings for this model?* — the same metrics sliced by
  `generation_params` (temperature, seed, and any other sampling knobs recorded
  on the run), e.g. "Qwen at `temperature=0.2` first-passes `rust` work more
  often than at `0.7`." Settings ride along on every `PhaseRun`, so this slice is
  a byproduct of normal use, exactly like the model slice — no special apparatus.

Every cell carries its **sample size**, and that is load-bearing, not decoration:
this is passive production telemetry, so each phase ran once at its own
difficulty, and any single cell is confounded. Small models are high-variance;
trends become legible across N runs, never one. The reader weighs a cell as
evidence, not proof — which is exactly why the tool reports `n_runs` next to
every number. The governor's per-(task-type, tool) scorer (lift/drop map above) is
the *within-session* seed of this same matrix; persisting it cross-session is what
makes it durable.

**Per-run detail.** Aggregates tell you *which* model/settings tend to win; the
individual `PhaseRun` records tell you *why* a specific run went the way it did.
The scorecard drills from a cell down to the runs in it — each with its model,
settings, gates, reliability metrics, turns, tokens, and verdict — and from a run
down to its JSONL session log for the full transcript. Same **pull, not push**
discipline as the log: the detail exists on demand and never floods Claude's
context until asked for.

**Project review (human view).** A sibling Layer 2 view, `project_review`,
aggregates the *same* `PhaseRun` records along the **milestone × phase** axis
(rather than `model × tag`): per-phase status, `architect_verdict`,
`bounces_to_approval`, bugs, executor, scope deviations, and the milestone
retrospectives / calibration folds. It is the human's in-depth project-state lens
— pull-on-demand, never bubbled into Claude's context per call — and reuses the
scorecard's storage, not a parallel one.

**Model selection is the human's call.** rexyMCP surfaces the data; the architect
(or user) reads the scorecard and chooses the `model` and settings for the next
`execute_phase`. There is no automated tag→model router. A weak-model fleet is
high-variance and the best pick depends on context the human holds — a deadline,
which model is already warm, how costly a bounce is here, a model just added that
has no track record yet. The scorecard exists to **inform** that decision, not to
make it; its job is to turn a pile of accumulated runs into a legible "for work
like this, these model+settings combos have been earning their keep, at this
sample size."

### Layer 2 — `mcp` crate (binary)

An MCP **stdio** server built on the `rmcp` crate. It exposes two tools:

- **`execute_phase`** — args: `phase_doc_path` (string), `repo_path` (string,
  the target-repo root), optional `model` / `profile` override. Calls the
  `executor` library in-process and returns `PhaseResult`.
- **`executor_health`** — args: optional endpoint override. Pings the configured
  OpenAI-compatible endpoint and lists available models. Lets the architect
  confirm the executor is reachable before dispatching.
- **`executor_log_search`**, **`executor_log_tail`**, **`get_turn`** — read back
  the JSONL session log (see "Session log & troubleshooting tools"). Each caps
  its own output so a debugging query can't re-flood Claude's context.
- **`model_scorecard`** — args: optional `tags` / `model` / `min_runs` filters.
  Aggregates the `PhaseRun` telemetry into the `model × tag` (and `model ×
  settings`) competency matrix with per-cell sample sizes (see "Model
  effectiveness metrics & the scorecard"). Lets the user see which model +
  settings to dispatch a phase with.

Practical concerns this layer owns:

- **Long runs.** A phase can take minutes; the MCP per-tool `timeout` is set well
  above the 10-second default (toward the 10-minute ceiling).
- **Liveness (pull, not push).** The human-liveness path is **`rexymcp status
  --repo <path>`**, an out-of-band CLI that reads the per-record-flushed session
  JSONL and reports the latest stage (turn count, current tool, verifier
  pass/fail, `awaiting_model` heartbeat). The server *also* emits **MCP progress
  notifications** as the executor advances, but these are **spec-correct and
  unreachable with Claude Code's current client**: a server may only emit
  `notifications/progress` for a request that supplied a `progressToken`, and
  Claude Code's MCP client sends none (confirmed empirically in the M6 dogfood —
  the `execute_phase: client progress_token absent` diagnostic). So the logged
  `SessionEvent::Progress` records are written unconditionally (independent of any
  live watcher), and `rexymcp status` is what surfaces motion to the human; MCP
  progress fires only if a future client opts in with a token.
- **Context hygiene.** Returned output is capped (`MAX_MCP_OUTPUT_TOKENS`) so a
  phase's inner transcript can never flood Claude's context. Claude gets the
  `PhaseResult` summary + diff + (on failure) briefing — nothing more.
- **Roots.** The server queries Claude Code's `roots/list` (and reads
  `CLAUDE_PROJECT_DIR`) to **corroborate the target-repo root** — a second source
  for the scope boundary alongside `execute_phase`'s `repo_path` argument, so a
  mismatch can be caught rather than silently trusted. (Sampling and elicitation
  are deliberately *not* used: Claude Code doesn't support server-initiated
  sampling, and we don't pull the human into the loop mid-phase.)

### Layer 3 — Plugin package

A Claude Code **plugin** bundles the MCP server with the workflow that drives it:

- **`.mcp.json`** registering the `mcp` server binary (auto-connects on enable).
- **Skills:**
  - `architect` — explore the target repo, then write the design doc, milestone
    README, and phase docs into the target repo's `docs/dev/`, following the
    `WORKFLOW.md` templates verbatim. **Pre-injection is an explicit
    responsibility:** because the local model can't call back to Claude live
    (Claude Code does not support MCP sampling) and can't reach Claude Code's web
    tools, the architect front-loads what the weak model will need *into the
    phase doc* — worked examples, codebase idioms, gotchas, few-shot exemplars of
    correct tool calls for the target model, and any fetched reference/API docs
    (Claude has web fetch/search at the architect level; the executor does not).
    This is the primary, offline, per-phase-free way Claude's capability reaches
    the local model.
  - `review-phase` — check executor output against the Definition of Done in
    `STANDARDS.md`, rerun the project's commands, then approve or file a bug.
  - `escalate` — given a returned briefing, pick a lever: re-dispatch with a
    refined spec (default for weak models — see "Escalation"), session takeover,
    or resume (candidate, if `continue_phase` is built).
- **Commands:** `/rexymcp:architect`, `/rexymcp:dispatch <phase>`, `/rexymcp:review <phase>`.
- **Embedded templates:** generalized copies of `STANDARDS.md` / `WORKFLOW.md`
  **and the executor contract** (`executor_contract.md` — the portable subset of
  this repo's `AGENTS.md`: hard rules, phase lifecycle, blocker/completion
  protocol, grep-for-literals. The opencode-specific operational notes in today's
  `AGENTS.md` are dropped, because the product executor is a local LLM over an
  OpenAI endpoint, not opencode). All three use `{BUILD_COMMAND}` /
  `{LINT_COMMAND}` / `{TEST_COMMAND}` / `{FORMAT_COMMAND}` placeholders that
  resolve **per target project** from rexyMCP config, which is what makes the
  product language-agnostic. The executor contract and `STANDARDS.md` are what the
  `executor` crate prepends to every phase's system prompt (Layer 1, turn-cycle
  step 1); the contract is **embedded-only** — a rexyMCP-driven project never
  carries a root `AGENTS.md` or an executor-contract file.

### Project initialization (bootstrap)

The `architect` skill owns getting a new target repo ready. On `/rexymcp:architect`
against a repo with no `docs/dev/` scaffold, it bootstraps **before** designing,
then proceeds to explore and write the design:

1. **Resolve the command set.** Detect the project's `format`/`build`/`lint`/`test`
   commands (e.g. `Cargo.toml` → `cargo …`, `package.json` → the package manager)
   and/or confirm with the user; write them to rexyMCP config (`rexymcp.toml`).
   These resolve the `{…_COMMAND}` placeholders everywhere downstream.
2. **Lay down the process docs.** Write the resolved `docs/dev/STANDARDS.md` and
   `WORKFLOW.md` from the embedded templates (placeholders filled in).
3. **Write `CLAUDE.md`.** Orient Claude as the architect for this project: the
   per-project command set, pointers to the process docs, and the note that the
   executor is a local LLM reached through rexyMCP whose contract is embedded —
   *not* a file in this repo.
4. **Register the server.** Ensure the rexyMCP MCP server is in `.mcp.json` (if
   the plugin install didn't already do it).

It does **not** write an `AGENTS.md` or any executor-contract file — that content
reaches the local model through the embedded, in-process system-prompt assembly
(turn-cycle step 1), and reaches Claude through `CLAUDE.md` + the skills.
Bootstrap is idempotent: an already-initialized repo is left alone (or only its
missing pieces repaired).

## End-to-end flow

1. The user gives Claude a product idea.
2. The `architect` skill **initializes the project if needed** (bootstrap:
   resolve the command set into `rexymcp.toml`, lay down the resolved
   `STANDARDS.md` / `WORKFLOW.md` + `CLAUDE.md`, register the MCP server — no
   `AGENTS.md`, no executor-contract file; see "Project initialization"), then
   explores the target repo and writes the design doc + M1 README + phase docs
   into `docs/dev/`.
3. `/rexymcp:dispatch phase-01` → Claude calls the `execute_phase` MCP tool.
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
   `PhaseResult` + briefing contract. Includes the **JSONL session log** — every
   turn event written (redacted via the lifted `security/redact.rs` +
   `ai/filter.rs`) to `<repo>/.rexymcp/sessions/`, using the lifted
   `store/sessions/jsonl.rs` writer. Also emits the per-phase **`PhaseRun`
   metrics record** (objective fields: gates, turns, tokens, parse-failure rate,
   verifier retries) into the cross-project telemetry store — see "Model
   effectiveness metrics & the scorecard." Also owns the **read-before-edit
   invariant**: the loop tracks a per-session working set of files the executor
   has read (with mtime), and `patch` refuses to edit a file the executor hasn't
   read this session or that changed on disk underneath it. The M2 `patch` tool
   (phase-04) deliberately ships *without* this enforcement — it lives in the
   loop's session state, not the stateless tool — so M4 wires the check around the
   tool rather than inside it.
5. **M5 — MCP server.** The `rmcp` stdio server exposing `execute_phase` and
   `executor_health`, with progress notifications and output capping. Also
   exposes the **session-log query tools** (`executor_log_search`,
   `executor_log_tail`, `get_turn`) that read the M4 log back on demand, each
   capping its own output, plus **`model_scorecard`** which aggregates the
   `PhaseRun` telemetry into the model × tag competency matrix. Queries
   `roots/list` / `CLAUDE_PROJECT_DIR` to corroborate the target-repo root
   against `execute_phase`'s `repo_path`.
6. **M6 — Plugin + architect/rexymcp:review skills.** The Claude Code plugin manifest,
   the `architect` / `review-phase` / `escalate` skills, the slash commands, the
   embedded generalized `STANDARDS.md` / `WORKFLOW.md` **and executor contract**
   (the portable subset of `AGENTS.md`; opencode-specific notes dropped), and an
   end-to-end dogfood against a real repo. The `architect` skill also owns
   **project initialization** (see Layer 3): bootstrapping an uninitialized target
   repo — resolving the per-project command set, laying down the resolved process
   docs + `CLAUDE.md`, registering the MCP server — with no root `AGENTS.md`, since
   the executor contract is embedded rather than a file. The `architect` skill makes **pre-injection** an explicit
   responsibility (worked examples, idioms, few-shot tool-call exemplars, fetched
   reference docs baked into the phase doc) — the primary way Claude's capability
   reaches the local model, since there's no live callback. Phase progression is
   **gated by default** (see
   `docs/dev/WORKFLOW.md` § "Phase progression & triggers"): after a review
   passes the architect marks the phase done and stops; the user advances with
   `/rexymcp:architect next` (draft the next phase) or `/rexymcp:dispatch <phase>` (run an
   existing one). Milestone boundaries always stop for human sign-off. An opt-in
   autonomous loop (off by default) can chain draft → dispatch → review until a
   blocker or milestone boundary.
7. **M7 — Per-run statistics & model scorecard.** Consume the `PhaseRun`
   telemetry accumulated from M4 onward to give the user detailed per-run
   statistics and a `model_scorecard` aggregation, so they can decide **which
   local model to use and which settings work best for it**. Surfaces the
   `model × tag` and `model × settings` slices with per-run drill-down, and the
   per-cell sample sizes that keep high-variance small-model data honest. (An
   earlier benchmark-suite + automated-routing plan was dropped 2026-06-02 — the
   scorecard tracks regular production runs, not a separate controlled-benchmark
   apparatus, and informs a human decision rather than auto-routing. See the M7
   README direction-change note.) Depends on having data, so it lands after the
   loop (M4) and server (M5) have been producing records.
