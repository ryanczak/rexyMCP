# rexyMCP — Architecture

> **Status:** Living design doc. M1–M29 are fully implemented and closed (M18's
> thread 4 / cold-start calibration battery is shelved by design, outside its
> committed scope; M27's stretch phase-07 advisory routing was not taken).
> **M30** (executor interruption) is the active milestone, in progress. The
> most recent arcs: **M26** (polish & hardening — loop-gate/hook/governor
> hardening), **M27** (the autonomous escalation loop — `/rexymcp:auto`,
> `continue_phase` resume, server-authored bookkeeping, per-role subagent
> delegation, and the architect loop journal / usage harvester), **M28**
> (edit-tool arg recovery), and **M29** (cleanup). This document is the source
> of truth for the *intended* design; the code under `executor/` and `mcp/` is
> the source of truth for what actually runs. Milestones are listed in the
> **Status** section at the bottom — that list is the project plan.

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
│   architect · dispatch · review · escalate · auto skills      │
│   /rexymcp:architect · :dispatch · :review · :escalate · :auto │
└───────────────┬───────────────────────────────────────────────┘
                │ MCP (stdio): execute_phase · continue_phase · executor_health · …
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
| Tools: `read_file` (500-line cap + truncation notice; `start_line`/`end_line` for ranges), `write_file`, `patch`, `bash`, `search`, `find_files`, `symbols` | `rexy/src/tools/` | **Lift**; every path scoped to the target-repo root. |
| Tool registry + 2-stage router | `rexy/src/tools/registry.rs`, `router.rs` | **Lift.** |
| Governor: scorer, verifier, hard-fail detector | `rexy/src/governor/` | **Lift.** |
| Context budget + compactor | `rexy/src/context/` | **Lift.** |
| Security: scope, capabilities, bash classify, redact, injection, audit | `rexy/src/security/` | **Lift.** Critical — a weak model running `bash` needs the allowlist. |
| Secret redaction / masking | `rexy/src/security/redact.rs`, `rexy/src/ai/filter.rs` (`mask_sensitive`, `init_masking`) | **Lift.** Applied to every turn before it is written to the session log. |
| Session JSONL log + reader | `rexy/src/store/sessions/jsonl.rs` (`SessionLogger`, `SessionRecord`, `SessionEvent`, `read_session_log`) | **Lift / adapt.** The executor's turn-by-turn record; query tools read it back. |
| Telemetry store (per-phase metrics) | `rexy/src/store/telemetry.rs` | **Lift / adapt.** Cross-project `PhaseRun` records — the substrate for the model scorecard. |
| Result truncation/formatting; error model | `rexy/src/result/`, `rexy/src/error/` | **Lift / adapt.** |
| TUI (fullscreen / classic) | `rexy/src/tui/` | **Drop.** The executor library is headless — progress flows to the session log / MCP, not a UI. (A read-only CLI dashboard over that log is a separate Layer-2 feature; see M8.) |
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
5a. **Post-write hook.** After every successful edit-class tool call (`write_file`,
   `patch`), run the project's configured `lint_fix` command (if set) then the
   `format` command (if set), both best-effort (failures discarded, never a
   hard-fail). This ensures the on-disk file is always in the formatter's expected
   state before the verifier reads it, regardless of turn ordering — a spec
   instruction to "run the formatter" cannot achieve this because a later write
   undoes it.
6. After edit-class tools, run the verifier (the project's typecheck/build). On
   failure, feed the diagnostics back for a retry.
7. The hard-fail detector watches for repetition loops, repeated verifier
   failures, and budget overflow. If it trips, assemble a **briefing** and stop.
8. When the model signals completion (`NoToolCall`), run the project's full
   command set (`{FORMAT_COMMAND}`/`{BUILD_COMMAND}`/`{LINT_COMMAND}`/
   `{TEST_COMMAND}`). If any gate exits non-zero, inject its output as a user
   message and re-enter the loop at step 3 — the model must fix the failure and
   re-signal completion. If the turn budget is exhausted while fixing gate
   failures, the result is `PhaseResult::BudgetExceeded`. Only when all
   configured gates pass does the loop return `PhaseResult::Complete`. (M19.)

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
- **Resume** *(committed at M27 kickoff, 2026-07-08)* — a
  `continue_phase(phase, guidance)` tool that resumes a failed phase
  **briefing-seeded**, not transcript-rehydrated: a *fresh* executor context
  built from the phase doc + the returned briefing + one targeted architect
  directive + the current working-tree diff, with `task_states` restored from
  the session log. This keeps "don't redo the 90% that's done" while discarding
  the accumulated context rot that full-transcript rehydration would preserve —
  the rot is precisely what the re-dispatch lever exists to escape, so replaying
  it would be self-defeating. The cheap middle lever for "model was almost
  there, hit one specific wall"; situational, never the default.

The three levers are exercised two ways. **Interactively**, the human invokes
the escalate skill per failure (the mode described above). **Autonomously**
(M27), an explicit opt-in `/rexymcp:auto` run drives the whole cycle — draft →
dispatch → review → escalate → re-dispatch/resume — across a milestone with no
per-phase human pause. The loop driver lives in the **plugin/skill layer**
(agent-neutral, composing the existing skills), never in the executor or
server: the executor stays a single-shot unit returning a structured briefing,
and rexyMCP still never calls a cloud provider — Claude, already the architect,
remains the escalation target. The loop is budgeted (`[escalation]
max_assists` per phase), journaled (every architect activity is a telemetry
record; token usage harvested from the client's own transcripts where
available, absent — never estimated — elsewhere), and hard-gated: milestone
boundaries, blockers, and budget exhaustion always stop for the human with a
structured loop report.

The briefing is a *fresh* brief, not a transcript replay — the shape Rexy already
defines: **goal** (verbatim), **acceptance criteria**, **current code state**
(diagnostics + working-set files), **what was tried** (compressed to 1–2 lines
each), **current blocker** (exact diagnostic), **budget remaining**.

#### The `PhaseResult` / briefing contract

`execute_phase` returns a single structured value:

```
PhaseResult {
  status:          "complete" | "hard_fail" | "budget_exceeded" | "cancelled",
  files_changed:   [ { path, change_summary } ],
  diff:            unified diff of the working tree (capped),
  command_outputs: { format, build, lint, test → tail of stdout/stderr },
  update_log:      the completion / blocker entry written into the phase doc,
  briefing:        present only when status is "hard_fail" / "budget_exceeded" — the escalation brief,
  cancellation:    present only when status is "cancelled" (M30) — { reason?, stage, turns_done }; leaves the tree dirty,
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
  model,                                // requested model id
  generation_params: { temperature, seed }, // sampling settings (None = endpoint default)
  phase_id, tags,                       // language, kind (feature|refactor|bugfix|test), size bucket
  status, escalated,                    // complete|hard_fail|budget_exceeded; did it hand off to Claude?
  // quality
  gates: { fmt, build, lint, test },    // booleans on first completion
  warnings, bugs_filed,                 // count + max severity (filled by the architect at review)
  bounces_to_approval,                  // 0 = first-try; the single most telling number
  // reliability (the small-model differentiator)
  parse_failure_rate, repairs_per_call, // from the forgiving parser
  verifier_retries, tool_success_rate,  // from the governor
  length_finish_rate,                   // fraction of completions that hit max_tokens (truncation signal)
  // efficiency
  turns, wall_clock_s, tokens,          // TokenBreakdown (in/out/cached)
  // endpoint-reported provenance (best-effort; None when the server omits them)
  served_model,                         // model id from the chat response — more accurate than requested
  context_window,                       // max_model_len from /v1/models; distinct from budget.context_length
  // tier calibration (M20; #[serde(default)] — absent in pre-M20 records)
  tier_telemetry: {
    tier,                               // LARGE | MEDIUM | SMALL — from [escalation] config
    doc_level,                          // 0–3 architect detail level
    escalation_count,                   // mid-phase Architect assists this run
    architect_input_tokens,             // Architect (Claude) input tokens
    architect_output_tokens,            // Architect (Claude) output tokens
  },
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

An MCP **stdio** server built on the `rmcp` crate. It exposes these tools:

- **`execute_phase`** — args: `phase_doc_path` (string), `repo_path` (string,
  the target-repo root), optional `model` / `profile` override. Calls the
  `executor` library in-process. As of M30 it is an **async job**: it spawns the
  run inside the serve process, registers it under a **`run_id`**, and returns
  `{ run_id }` immediately; the terminal `PhaseResult` is reaped with
  `get_run_status`. (The CLI `rexymcp run-phase` stays blocking and returns the
  `PhaseResult` directly.)
- **`get_run_status`** (M30) — args: `run_id` (string). Bounded long-poll (≈15s)
  on a spawned `execute_phase` run: returns `{ state: "running" }` while the run
  is in flight, the terminal `PhaseResult` once it completes / hard-fails / is
  cancelled, an infra `{ state: "failed", error }` if the run errored, or
  `{ state: "unknown" }` for an unrecognized `run_id`. This is how the architect
  (or the async skill loop, phase-05) reaps a spawned run.
- **`stop_phase`** (M30) — args: `run_id` (string). Fires the spawned run's
  cooperative `CancelSignal` so it aborts at the next turn boundary (or mid
  model-stream) and returns a `PhaseResult` with status `cancelled`,
  `cancellation.reason` `claude_stop`, and the partial diff (working tree left
  dirty). Returns `{ stopped: true }` if the `run_id` was known, `{ stopped:
  false }` otherwise. The cancel is cooperative and asynchronous — the caller
  polls `get_run_status` to observe the terminal `cancelled` result. This is the
  architect's mid-flight abort, at poll granularity; the human's client-agnostic
  path is the `.rexymcp/stop` sentinel (`rexymcp stop`, phase-04).
- **`continue_phase`** (M27) — args: `phase_doc_path`, `repo_path`, `guidance`
  (the architect's targeted directive), optional `model`. The **resume** lever:
  re-enters a `hard_fail`/`budget_exceeded` phase **briefing-seeded** — a fresh
  executor context from the phase doc + returned briefing + guidance + current
  working-tree diff, with `task_states` restored from the session log. Returns a
  `PhaseResult` like `execute_phase`.
- **`executor_health`** — args: optional endpoint override. Pings the configured
  OpenAI-compatible endpoint and lists available models. Lets the architect
  confirm the executor is reachable before dispatching.
- **`executor_log_search`**, **`executor_log_tail`**, **`get_turn`** — read back
  the JSONL session log (see "Session log & troubleshooting tools"). Each caps
  its own output so a debugging query can't re-flood Claude's context.
- **`model_scorecard`** — args: optional `tags` / `model` / `min_runs` filters.
  Aggregates the `PhaseRun` telemetry into the `model × tag` competency matrix
  with per-cell sample sizes. Lets the architect see which model + settings to
  dispatch a phase with. (MCP tool — Claude-facing.)
- **`model_profile`** (M18) — same filters; aggregates telemetry into a
  per-`(model, tag)` **capability profile**: strengths (gate-pass and
  approved-first-try rates, reliability means) plus ranked failure classes, with
  non-attributable classes (`spec_bug`, `infra_blip`) separated from the model's
  real weaknesses.

The `mcp` binary also exposes out-of-band **CLI commands** for human-facing use:

- **`rexymcp runs`** — lists individual `PhaseRun` records with their per-run
  statistics (model, settings, gates, reliability/efficiency, verdict), filterable
  by `--model`/`--tag`, newest-first. Human table + `--json`. The direct "see what
  this specific run did" view.
- **`rexymcp scorecard`** — aggregates runs into a **`model × settings`**
  competency matrix (same means as `model_scorecard` plus `length_finish_rate`),
  answering "which settings work best for this model?" Human table + `--json`.
- **`rexymcp status`** — one-shot liveness report for the most-recently-active
  session; see "Liveness" below.

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
  progress fires only if a future client opts in with a token. A richer live view
  over this same JSONL — a full-screen, continuously refreshed dashboard —
  shipped as **M8** (`rexymcp dashboard`): the opacity of a then-blocking
  `execute_phase` call was exactly what left the user without insight mid-phase,
  and a one-shot `status` only partly answers it. **As of M30, `execute_phase` is
  an async job** — it returns a `run_id` and the architect reaps the result by
  polling `get_run_status`, so the architect is no longer blocked inside one long
  call, and a running phase is **interruptible** out-of-band: `rexymcp stop`
  (human, second terminal, via the `.rexymcp/stop` sentinel) or `stop_phase`
  (architect, between polls) cancels it, returning a `cancelled` `PhaseResult`
  with the partial diff and a dirty working tree. `rexymcp status` / `dashboard`
  remain the liveness surface either way.
- **Context hygiene.** Returned output is capped (`MAX_MCP_OUTPUT_TOKENS`) so a
  phase's inner transcript can never flood Claude's context. Claude gets the
  `PhaseResult` summary + diff + (on failure) briefing — nothing more.
- **Roots.** The server **corroborates the target-repo root** — a second source
  for the scope boundary alongside `execute_phase`/`continue_phase`'s `repo_path`
  argument, so a mismatch is refused rather than silently trusted. The active
  source is the project-dir env var (`CLAUDE_PROJECT_DIR` /
  `ANTIGRAVITY_PROJECT_DIR`); the MCP `roots/list` half is **deferred** (M26,
  2026-07-07): rmcp 1.8.0 deprecated `Peer::list_roots` per MCP SEP-2577, so the
  server currently passes an empty roots list and the `roots.rs` corroboration
  logic waits for a roots replacement. (Sampling and elicitation are deliberately
  *not* used: Claude Code doesn't support server-initiated sampling, and we don't
  pull the human into the loop mid-phase.)

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
  - `dispatch` — thin glue around `execute_phase`: pre-flight `executor_health`,
    dispatch the phase, then **drive the async contract** (detect-and-adapt: poll
    `get_run_status` on a returned `run_id`, or use a direct `PhaseResult` from an
    old-serve / `run-phase` response), and surface the summary (→ review), the
    briefing (→ escalate), or a **`cancelled`** result (partial diff — the phase
    was stopped via `rexymcp stop` / `stop_phase`).
  - `review-phase` — check executor output against the Definition of Done in
    `STANDARDS.md`, rerun the project's commands, then approve or file a bug.
  - `escalate` — given a returned briefing, pick a lever: re-dispatch with a
    refined spec (default for weak models — see "Escalation"), session takeover,
    or **resume** (briefing-seeded `continue_phase`, shipped M27).
  - `auto` (M27) — the opt-in **autonomous milestone loop**. Composes the four
    skills above — draft → dispatch → review → escalate/re-dispatch — hands-off
    across a whole milestone with full review rigor and no per-phase pause,
    delegating dispatch/review to subagents on the `[architect] dispatch_model`
    / `review_model` role models, budgeted by `[escalation] max_assists`,
    journaling every activity, and stopping at a milestone boundary / blocker /
    budget exhaustion / runaway backstop with a structured loop report. It
    *composes, never forks* the other skills — an autonomous run of a step is the
    same procedure as an interactive one.
- **Commands:** `/rexymcp:architect`, `/rexymcp:dispatch <phase>`,
  `/rexymcp:review <phase>`, `/rexymcp:escalate <phase>`, `/rexymcp:auto [max-phases]`.
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
3. **Write `REXYMCP.md` and wire the import.** `REXYMCP.md` is rexyMCP's own,
   agent-neutral orientation file (the per-project command set, pointers to the
   process docs, and the note that the executor is a local LLM whose contract is
   embedded — *not* a file in this repo). It reaches Claude's context through a
   one-line `@REXYMCP.md` import appended to `CLAUDE.md` (the only file Claude
   Code auto-loads); rexyMCP owns `REXYMCP.md` and never co-opts or rewrites the
   user's `CLAUDE.md` content.
4. **Register the server.** Ensure the rexyMCP MCP server is in `.mcp.json` (if
   the plugin install didn't already do it).

It does **not** write an `AGENTS.md` or any executor-contract file — that content
reaches the local model through the embedded, in-process system-prompt assembly
(turn-cycle step 1), and reaches Claude through `REXYMCP.md` (imported by
`CLAUDE.md`) + the skills. Bootstrap is idempotent: an already-initialized repo
is left alone (or only its missing pieces repaired).

## End-to-end flow

1. The user gives Claude a product idea.
2. The `architect` skill **initializes the project if needed** (bootstrap:
   resolve the command set into `rexymcp.toml`, lay down the resolved
   `STANDARDS.md` / `WORKFLOW.md` + `REXYMCP.md` (imported by `CLAUDE.md`),
   register the MCP server — no `AGENTS.md`, no executor-contract file; see
   "Project initialization"), then
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
  (`format`/`build`/`lint`/`test`), plus an optional `lint_fix` autofixing
  command run by the post-write hook (step 5a above) — not advertised to the
  executor model, not a gate command,
- budget knobs (context %, max turns, gate retries, optional wall-clock ceiling),
- **`[executor] max_tokens`** (M23) — per-response output-token ceiling sent on
  every chat request (default 8192; per-model overridable in `[models."<id>"]`).
  Carved out of the remaining context window; the prior hardcoded 4096 truncated
  thinking models mid-reasoning before they reached a tool call,
- **`[executor] enable_thinking`** (M25) — opt-in reasoning toggle (default
  **false**; per-model overridable in `[models."<id>"]`). When false it is emitted
  on the wire as `chat_template_kwargs.enable_thinking = false`, so reasoning
  models default to thinking off unless a model's override turns it on,
- **`[executor] tier`** (M20) — executor capability tier
  (`LARGE`/`MEDIUM`/`SMALL`), set by `rexymcp calibrate`; derives default
  `max_turns` and `gate_retries` (wired M26),
- **`[escalation] max_assists`** (M20, consolidated M27) — the per-phase
  autonomous assist budget for the architect-side `/rexymcp:auto` loop:
  autonomous escalation round-trips on one phase before the loop stops for the
  human. Flat and tier-independent (default 3); consumed by the plugin skill
  layer, never the executor loop. (The overlapping `[budget] escalation_slots`
  was retired at M27; `calibrate` strips the stale key from old configs.)
- **`[architect]`** (M20) — Claude model id and cost rates
  (`input_per_mtok`, `output_per_mtok`) for the dashboard Architect cost column;
  a `known_model_rates` registry auto-fills rates for recognized Claude model IDs.

## Non-goals

- **No interactive TUI agent.** The executor loop is headless; rexyMCP is a
  bridge, not an interactive coding agent, and the executor surfaces its progress
  through the session log and MCP, not a terminal interface. This does **not**
  exclude a **read-only** live dashboard over that session data (the shipped
  `rexymcp dashboard`, M8) — a monitoring view of the same JSONL `rexymcp status`
  already reads is a viewer, not an agent surface.
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
   docs + `REXYMCP.md` (imported by `CLAUDE.md`), registering the MCP server —
   with no root `AGENTS.md`, since the executor contract is embedded rather than a
   file. The `architect` skill makes **pre-injection** an explicit
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
7. **M7 — Per-run statistics & model scorecard** *(done, 2026-06-02)*. Consume
   the `PhaseRun` telemetry accumulated from M4 onward to give the user detailed
   per-run statistics and a settings scorecard, so they can decide **which local
   model to use and which settings work best for it**. Ships:
   - `rexymcp runs` — per-run CLI view (model, settings, gates, reliability /
     efficiency, provenance, verdict), filterable and newest-first.
   - `PhaseRun` settings plumbing — `temperature`/`seed` configurable, sent on
     every request, and recorded with real values so the settings axis carries
     signal.
   - `PhaseRun` chat-stream provenance — served model id and
     `length_finish_rate` (fraction of completions truncated at `max_tokens`),
     captured additively via a new `AiEvent::Completion` event.
   - `PhaseRun` context window — `max_model_len` from `/v1/models`, best-effort,
     so a user can see whether their budget matches the model's actual capacity.
   - `rexymcp scorecard` — `model × settings` competency matrix CLI, same
     quality/reliability/efficiency/supervision means as the `model_scorecard`
     MCP tool plus `length_finish_rate`, answering "which settings perform best
     for this model?"
   All data is passive production telemetry — a byproduct of normal use, not a
   separate benchmark apparatus. The scorecard informs a **human decision**; there
   is no automated model-routing. Per-cell sample sizes are always shown, because
   individual phases are not controlled experiments and small models are
   high-variance. Depends on having data, so it lands after the loop (M4) and
   server (M5) have been producing records.
9. **M9 — Executor runtime hardening** *(done, 2026-06-04)*. Three fixes that
   close hard-fail classes a phase-spec instruction cannot close:
   - **Post-write hook** (phase-01/02): after every successful edit-class call,
     run `lint_fix` then `format` (both best-effort, failures discarded). Closes
     the formatting-overwrite race: an executor that runs the formatter mid-loop
     and then issues another `write_file` no longer leaves the file unformatted
     for the final `fmt --check`. `lint_fix: Option<String>` is a new optional
     field in `CommandConfig`; the hook is step 5a in the turn cycle above.
   - **`read_file` output cap** (phase-03): `read_file` returns at most 500 lines
     and appends a truncation notice with the file's total line count and shown
     range. Prevents `RunawayOutput` hard-fails when an executor reads a large
     source file whole.
8. **M8 — Live session dashboard** *(done, 2026-06-07; 16/16 phases complete,
   both exit criteria met)*. **Why it matters:** `execute_phase` is opaque *and*
   blocking — the MCP client sends no `progressToken`, so progress notifications
   never fire (see Layer 2 § "Liveness"), leaving the user blind for minutes at a
   time. `rexymcp status` gives a one-shot read; `rexymcp dashboard` is the live,
   continuously-refreshed view over the same JSONL feed. A monitoring view, not an
   interactive agent surface (see Non-goals). `rexymcp status` stays as a separate
   one-shot / scriptable command — both are kept. Ships:
   - `rexymcp dashboard --repo <path> [--session <id>] [--config <path>]` — a
     `ratatui` full-screen TUI, 500 ms poll, stays open until `q`/`Esc`/`Ctrl-C`,
     auto-follows a newly-started session when unpinned.
   - **Session panel** — phase, session id, model, state, turn count, stage, age.
   - **Budget panel** — tokens in/out, context % (color-coded gauge: green <50 /
     yellow 50–80 / red ≥80), tok/s (derived from `Metrics` record timestamps),
     and a tabular **Baseline / Executor / Architect / Net** cost breakdown across
     Session, Milestone, and Project scopes — plus an **Assists** counter (sum of
     mid-phase Architect escalations). Configurable cloud-baseline and Architect
     rates via `[dashboard]` / `[architect]` in `rexymcp.toml`; rows show `—`
     when the corresponding rate is unset.
   - **Reclaim panel** — aggregate live reclaim across all four M10 sources:
     compaction (event count, tokens freed, compression ratio), boundary-filter
     (`OutputFiltered`), superseded-read eviction (`ReadEvicted`), and redundant-read
     dedupe (`ReadDeduped`). Each lever line is omitted when its count is zero;
     placeholder `(no reclaim yet)` when none have fired.
   - **Activity transcript** — full scrollable replay of all session events
     (`Prompt`, `Completion`, `ToolCall`/`ToolResult`, `Verify`, `ParseFailed`,
     `HardFail`, `Compaction`, `Progress`, `Metrics`, `SessionStart`/`SessionEnd`),
     color-formatted per type, with multi-line tool output and tail-follow.
   - **Files panel** — per-file `+N -N` numstat, left-trimmed paths.
   - **`[dashboard]` config section** — `saved_input_per_mtok` /
     `saved_output_per_mtok` (f64, default 0.0 → `—` in Baseline/Net rows) and
     `saved_model` (optional string — auto-fills rates for recognized Claude model
     IDs, M15). A missing section falls back to defaults (purely additive). A
     sibling **`[architect]` config section** (M20) mirrors the same fields
     (`model`, `input_per_mtok`, `output_per_mtok`) for the Architect cost column.
10. **M10 — Context optimization** *(done, 2026-06-08)*. Two arcs shrink the
    executor's context footprint so the local model completes more phases without
    compaction or hard-fail. Ships:
    - **Arc A — boundary output filtering.** `executor/src/context/output_filter.rs`:
      generic ANSI-strip + consecutive-dup collapse + head/tail truncation tee'ing
      full output to a rotated recovery file (phase-01). A structured cargo filter
      (phase-02) routes `cargo` invocations through a diagnostics-preserving
      compressor. Per-lever `SessionEvent::OutputFiltered` records how many tokens
      each filter reclaimed (phase-03).
    - **Arc B — semantic context lifecycle.** `read_file` dedupes re-reads of
      unchanged files to a compact reference (phase-06, `ReadDeduped` event).
      Successful edits evict superseded prior reads for that path (phase-04,
      `ReadEvicted` event). The context compactor (phase-07) gains value-ranked
      in-place signaturization — shrinks lowest-value tool output first (command
      noise before file reads), protects the last 3 turns, preserves
      tool-call/result pairing.
    - **Measurement spine.** `Budget::estimate` correctness fix — now counts
      `tool_calls[n].arguments` + `tool_results[n].content` so `context_pct` grows
      turn-over-turn and the compactor fires on real pressure (phase-05). Phase-08
      surfaces the per-lever reclaim signals post-hoc and live:
      - **08a** — `ContextEfficiency` struct + `aggregate_context_efficiency` over
        the session log; `PhaseRun.context_efficiency` (`#[serde(default)]`).
      - **08b** — `PEAK_CXT` + `RECLAIMED` columns in `rexymcp runs`.
      - **08c/08d** — `peak_context_pct_mean` + `tokens_reclaimed_mean` on both
        scorecards (`model × tag` MCP tool; `model × settings` CLI).
      - **08e** — `StatusSummary` six additive fields + three `summarize` arms;
        the dashboard Compactions panel repurposed as the aggregate **Reclaim**
        panel; `rexymcp status` `reclaimed:` line.

11. **M11 — Polish** *(done, 2026-06-09; 7/7 phases approved_first_try)*. Maintainability,
    tuneability, and quality-of-life improvements. Three sub-goals:
    - **Configurable governor thresholds.** `IDENTICAL_CALL_THRESHOLD`,
      `VERIFIER_PERSISTENCE_THRESHOLD`, and `RUNAWAY_OUTPUT_BYTES` move from
      compile-time constants to a `[governor]` section in `rexymcp.toml`
      (phase-01, new `GovernorConfig` in `config.rs`).
    - **`rexymcp init`.** New CLI subcommand that scaffolds a fully-documented
      `rexymcp.toml` (including `[governor]`) and a `.mcp.json` Claude Code plugin
      entry so new projects can onboard without reading source (phase-02).
    - **File decomposition.** Four files that exceed the executor's 100 KB
      RunawayOutput limit have their test suites extracted to sibling `*_tests.rs`
      files: `agent/mod.rs` (4 420 → ≤ 900 lines, phase-03), `scorecard.rs`
      (1 153 → ≤ 400 lines, phase-04), `server.rs` (1 225 → ≤ 530 lines,
      phase-05a), `governor/verifier.rs` (1 163 → ≤ 340 lines, phase-05b). Pure
      move refactors — no logic changes.
    - **Executor temporal grounding.** The local model has no clock, so it
      stamps hallucinated dates in its Update Log. The system prompt now opens
      with a `Today's date is YYYY-MM-DD (UTC).` line formatted from the same
      injected `deps.clock` epoch-millis the rest of the loop uses (phase-06,
      `agent/prompt.rs`). Pure integer date arithmetic — no date dependency, no
      real wall-clock read, so it stays deterministic under test.

12. **M12 — Executor Tooling** *(done, 2026-06-10; 7 phase-docs / 9 dispatches, all approved, zero takeovers)*. Net-new
    executor capability aimed at
    making a weak local model more effective and more efficient. Each item below
    is an **intervention whose value is measurable against the scorecard** —
    rexyMCP using its own reason for existing. The discipline for the milestone:
    ship the certain-value half, gate the hypothesis half behind config, and let
    `bounces_to_approval` / `first_pass_rate` prune what doesn't earn its keep.
    **Locked scope (with the user, 2026-06-09): three arcs — Arc 0 (toolchain
    robustness) first, then Arc B (all three code-intelligence wins), then Arc A
    (task tracking, `task_tracking` default on).** See
    `docs/dev/milestones/M12-executor-tooling/README.md`. Three arcs:

    - **Arc 0 — toolchain robustness.** Validation features shell out to
      per-language toolchains; missing ones must **fail open at runtime** (the
      verifier returns a `Skipped` advisory naming the binary + remedy, distinct
      from a `Failed` infra error and from a `Checked` diagnostic, instead of the
      raw remedy-less spawn error it repeats today) and **fail-hard-advisory where
      a human can act** (a new `rexymcp doctor` command + architect bootstrap
      detection present a resolution plan). Detection lives in the architect +
      `doctor`, never in `rexymcp init` (which stays a static scaffolder, so an
      unsupported language like Zig runs on the Tier-0 command set alone). The
      discipline is folded into WORKFLOW.md/STANDARDS.md/the architect skill.

    - **Arc A — structured task tracking.** A per-session TODO list the executor
      **tracks and checks off as it works — it does not generate it** (decomposition
      stays the architect's job; the list is seeded from the phase doc's numbered
      Spec, and the executor only flips state pending → active → done and may append
      discovered sub-steps). Motivated by two logged failure modes a checklist
      directly attacks: dropped-subtask stalls (phase-08a did tasks 1–4 then stalled)
      and premature/false completion (phase-06). The "more context" cost is a
      non-issue: at the configured 131071-token window we have **never observed a
      compaction event**, so there is ample headroom. Pieces:
      - A new `SessionEvent::TaskUpdate { … }` (id, title, state) substrate; the
        loop seeds the list from the Spec and emits updates as the executor checks
        items off.
      - **Config-gated** (a `[tasks]` / `[executor] task_tracking` toggle, default
        TBD) so it is a clean **A/B** intervention — runs with it on vs. off are
        directly comparable on the scorecard. This is the load-bearing requirement:
        no measurement without an off-switch.
      - **Dashboard `Tasks` panel**, placed **above the Files panel** in the lower
        region — the Files panel's height is halved to make room, so the band shows
        TODO (active/pending/done) over Files. Same panel pattern as M8
        (`SessionEvent` → `summarize` fold → `panels.rs` render fn → `render.rs`
        layout slot).

    - **Arc B — incremental code intelligence (keep-it-simple; full LSP deferred).**
      We already have two of LSP's four headline features: `symbols` (tree-sitter
      definition search, Rust + Python) and the verifier's structured `Diagnostic`s
      (`cargo check`/`tsc`/`ruff` parsed from JSON). Rather than build a full LSP
      *client* — a subsystem (subprocess lifecycle, JSON-RPC, document-sync as the
      executor edits, per-language server discovery, multi-second rust-analyzer cold
      start that fights the turn loop) — harvest the specific high-value wins cheaply:
      - **Find-references in `symbols`** via tree-sitter (syntactic call-site search,
        deps already present). Attacks the multi-site-change failure class folded into
        WORKFLOW (§ "Prefer additive change shapes") — enumerate every call site
        *before* a breaking edit.
      - **Surface compiler suggested-fixes.** The verifier parses cargo JSON and has a
        `Help` severity but does not extract rustc's machine-applicable
        `suggested_replacement` spans; feeding those to the model ("rustc suggests X→Y
        at line N") is a cheap diagnostic upgrade.
      - **Structured test-failure parsing** — turn `cargo test` failures into
        expected-vs-actual records for the verifier-retry loop (extends the M10 cargo
        filter, which already preserves test summaries).
      - **Full LSP is a candidate, not committed.** Build it only if bounce-cause
        tagging shows symbol-resolution / wrong-API-usage is a *dominant* failure
        class. If the data says so, it is its own milestone (transport → lifecycle →
        doc-sync → tool surfaces → hermetic tests).

    **Non-goals (M12):** no executor-side planning/decomposition (the dropped Rexy
    planner stays dropped — Arc A tracks an architect-authored list, it does not
    create one); no full LSP client until measurement justifies the effort; no new
    cloud/network tooling (the executor stays offline — see "No internal cloud
    escalation").

13. **M13 — Dashboard Polish** *(done, 2026-06-10; 8/8 approved_first_try)*. The live
    dashboard (M8 wireframe + the M10 Reclaim and M12 Tasks panels) is functional
    but visually rough; this milestone makes it presentable. **Pure presentation
    layer — `mcp/src/dashboard/` plus read-only additions to `mcp/src/status.rs`'s
    `StatusSummary`/`summarize`.** No executor, loop, config, or `SessionEvent`
    schema change: every event the panels render already exists in the JSONL feed;
    M13 only changes how that feed is *displayed*. See
    `docs/dev/milestones/M13-dashboard-polish/README.md`. Threads:

    - **Legibility.** All low-contrast `Color::DarkGray` text is raised to
      `Rgb(200,200,200)` (the contrast the Completion-body/plain-text path already
      uses) so secondary text is readable. The Activity transcript gains word/line
      **wrapping** (long lines no longer run off the panel edge) with a working
      **tail-follow autoscroll** over the wrapped line count, and a position
      **scrollbar**.

    - **Activity completeness.** The transcript currently summarizes rather than
      shows two captured-but-dropped payloads: **injected context** (the `Prompt`
      record's full `rendered` text, today only a char count) and **tool-call
      arguments** (the `Parsed` record's `tool_call.arguments`, today only the
      name). Both are surfaced, reusing the existing body-truncation machinery, so
      the panel shows what the model was actually sent and asked to run. `<think>`
      reasoning blocks in Completion bodies are formatted distinctly from the
      answer. Each event line gains a relative timestamp.

    - **Panel polish.** The **Session** panel shows a session `duration:` (derived
      from a new `started_at` capture in `summarize`) and a **full-width spinner**
      carrying live `turn N · stage` status; the freshness `last update:` line moves
      to the **Budget** panel. The **Tasks** panel shows named tasks with
      checkbox/check glyphs (titles are already on the `TaskUpdate` event, currently
      discarded) and a done/total progress gauge matching the context-gauge style.

    **Non-goals (M13):** no new `SessionEvent` variants or schema changes (display
    only); no executor/loop/governor/config changes; no interactivity beyond the
    existing scroll/filter keys (the dashboard stays a monitoring view, not an
    agent surface — see M8 Non-goals); no new dependencies (`ratatui` already
    provides `Wrap`/`Scrollbar`).

14. **M14 — Cleanup** *(complete, 2026-06-10)*. Fixed the silent task-seeder
    failure (`seed_from_spec` stopping at `###` headings, producing zero tasks
    for 6 of 8 M13 phases), added a turn-0 empty-seed warning, folded the fix
    into `WORKFLOW.md`, and gathered the deferred M12/M13 cleanup items (two prod
    `eprintln!` in `server.rs`, stale `RUNAWAY_OUTPUT_BYTES` doc-comment in
    `read_file.rs`, `format_references` truncation-note copy bug in `symbols.rs`).
    2/2 phases, both approved_first_try. See
    `docs/dev/milestones/M14-cleanup/README.md`.

15. **M15 — Dashboard Polish (Round 2)** *(done, 2026-06-11; 3/3
    approved_first_try)*. Four targeted fixes to the live dashboard: (1) task
    titles use full panel width instead of a hardcoded 24-char cap; (2) `last
    update:` moves from the Budget panel to the Session panel; (3) `$ saved`
    auto-fills rates for recognised Claude model names via a new optional
    `saved_model` config field; (4) Activity transcript time-delta spans render
    in dull yellow instead of dim grey. Pure presentation layer for phases
    01–02; phase 03 adds one optional `String` field to `DashboardConfig`. No
    new `SessionEvent`, no new dependencies. See
    `docs/dev/milestones/M15-dashboard-polish-2/README.md`.

16. **M16 — Seeder Format Robustness** *(done, 2026-06-10; 1/1
    approved_first_try)*. Extends the task seeder
    (`executor/src/agent/tasks.rs`) to recognize the `### Task N — Title`
    heading style the architect naturally writes, in addition to the `N. Title`
    list items and `### N. Title` subheadings M14 added. Closes the recurring
    empty-seed mismatch (M15 phase-02 seeded zero tasks because its Spec used
    `### Task N —`; the `update_task` tool correctly rejected the executor's
    improvised id). Single-file, additive — the existing parse paths are
    untouched. See `docs/dev/milestones/M16-seeder-robustness/README.md`.

17. **M17 — Dashboard Polish (Round 3)** *(in progress, kicked off 2026-06-11)*.
    Six display-layer refinements to the live dashboard: (1) restore the
    dog-chasing-brain spinner animation, parametric on Session-panel width;
    (2) move `last update:` directly under `duration:` and capitalize every
    Session/Budget/Reclaim label; (3) add a `Milestone:` row whose name is
    derived from the milestone *directory* holding the running phase doc (no new
    config/event); (4) pan overflowing Tasks-panel titles back and forth;
    (5) upgrade Activity highlighting — Markdown for Completion bodies and
    extension-detected grammars for `read_file` results, both built on the
    crate's existing **syntect** dependency (tree-sitter was considered and
    dropped). No new `SessionEvent` and no new dependency in any phase. See
    `docs/dev/milestones/M17-dashboard-polish-3/README.md`.

18. **M18 — Capability-Aware Adaptation** *(done, 2026-06-15; 7/7 in-scope
    phases approved, thread 4 / phase-08 shelved by design)*.
    Make rexyMCP characterize each local model's strengths and failure modes and
    compensate for them — at draft time and at runtime — instead of relearning
    them by per-phase trial-and-error. **Foundational fix:** the supervision half
    of the eval loop was never wired — the executor writes every `PhaseRun`'s
    `architect_verdict`/`bounces_to_approval`/`bugs_filed`/`warnings` as `None`
    (`executor/src/agent/metrics.rs`), the telemetry store is append-only, and no
    write-back path exists, so the architect's verdict has only ever lived in
    phase-doc prose. Four threads: (1) **supervision write-back substrate** — an
    append-only `PhaseReview` annotation, folded onto its matching `PhaseRun` by
    phase identity at read time, written by a new `rexymcp review` CLI, carrying a
    structured **failure-class** taxonomy (`false_completion`, `prod_unwrap`,
    `multi_site_break`, …, plus `spec_bug`/`infra_blip` so non-model faults are
    not charged to the model); (2) a per-model **capability profile** (per-`(model,
    tag)` strengths + ranked failure classes) surfaced to the architect at draft
    time to make pre-injection data-driven; (3) **model-conditioned runtime knobs**
    — `task_tracking`/governor thresholds/router breadth/sampling resolve per-model
    defaults from config instead of global constants; (4) a **cold-start
    calibration battery** to seed a new model's profile — *shelved for later
    revisit, outside M18's committed scope*: it is active benchmark measurement
    and departs from this section's "passive production telemetry" principle, so
    when picked up it needs a talk-through and an explicit amendment of that
    principle before its phase is drafted. The scorecard still **informs** the
    human's model choice; M18 adds no automated model-routing. See
    `docs/dev/milestones/M18-capability-adaptation/README.md`.

19. **M19 — Structural Gate Enforcement** *(done, 2026-06-16; 1/1
    approved_after_1)*. Make `false_completion` structurally impossible:
    `execute_phase` now returns `PhaseResult::Complete` only when every
    configured DoD gate passes (exit 0). When a gate exits non-zero at
    completion, the runtime injects the failure output as a user message and
    continues the loop; turn-budget exhaustion while fixing gates yields
    `BudgetExceeded`. Backward-compatible: `EMPTY_COMMANDS` / `NoopRunner`
    (gates all `None`) paths unchanged. A ~130-line change in
    `executor/src/agent/mod.rs` closes the control-flow gap in the `NoToolCall`
    completion arm.

20. **M20 — Tier Calibration and Cost Visibility** *(done, 2026-06-16; 4/4
    phases approved)*. Three implementation phases plus a documentation sync:
    - **Phase-01** adds `[escalation]` + `[architect]` config sections and a
      `rexymcp calibrate LARGE|MEDIUM|SMALL` CLI subcommand — one knob that
      writes tier-derived budget defaults (`max_turns`, `escalation_slots`,
      `doc_level`) to `rexymcp.toml`. A shared `known_model_rates` registry in
      `executor/src/config.rs` auto-fills baseline and architect cost rates for
      recognized Claude model IDs.
    - **Phase-02** adds a `TierTelemetry` struct nested in `PhaseRun` via
      `#[serde(default)]` — five new fields: `tier`, `doc_level`,
      `escalation_count`, `architect_input_tokens`, `architect_output_tokens`.
      `EscalationEvent` record appended to `phase_runs.jsonl` each time a
      mid-phase Architect assist fires (M21 wires the firing; M20 defines the
      record).
    - **Phase-03** upgrades the dashboard Budget panel's Savings block from a
      gross three-scope `$ saved` row to a tabular **Baseline / Executor /
      Architect / Net** breakdown (Session × Milestone × Project columns) with
      a project Assists counter. Net savings now subtracts Architect spend from
      the cloud baseline. `ScopeCosts` struct introduced in
      `mcp/src/dashboard/panels.rs`; `BudgetRates` gains
      `architect_input_per_mtok` / `architect_output_per_mtok`.

21. **M21 — Task Coverage Gate** *(done)*. Close the `false_completion`
    blind spot on docs/no-code phases: add a task-coverage check to the
    `NoToolCall` completion arm, symmetric with M19's gate-retry loop. When
    tasks are seeded and any remain incomplete at `NoToolCall` time, inject a
    named-task list and loop. Single phase, ~120-line change in
    `executor/src/agent/command.rs` and `executor/src/agent/mod.rs`.
22. **M22 — Bookkeeping-Loop Resilience** *(done)*. Close four executor
    loop-failure modes diagnosed from a live netviz e2e run where a MEDIUM-tier
    model wrote correct code but could not reliably finish the bookkeeping tail:
    (A) an empty-output death spiral — a blank completion was treated as a
    completion attempt and re-ran the gates ~147× to the turn cap; (B) bogus task
    seeding — a prose numbered algorithm in `## Spec` seeded duplicate-titled
    tasks that drove an `update_task` fixation loop; and (C) a self-revert — `git
    checkout <edited-file>` discarded the model's own green work. Five phases,
    each structural and additive: empty-completion routing + a governor
    empty-completion stall; a stuck-gate-feedback stall; seeder precision
    (bold-name-required list tasks + de-dup); `update_task` results that echo the
    remaining incomplete ids; and a working-set-aware self-revert refusal. Same
    turn-cycle seam M19/M21 extended. (Server-authored bookkeeping — moving who
    authors the Status flip / Update Log — is deferred pending a contract
    discussion.)
23. **M23 — Truncation & Empty-Completion Recovery** *(done, 2026-06-18; 3/3
    phases approved)*. Recover
    from the truncation/empty-output tail before hard-failing, one layer below
    M22's `EmptyCompletionStall` terminator. Diagnosed from a netviz e2e run where
    the per-turn output cap (`max_tokens`, **hardcoded 4096** in the OpenAI backend)
    cut a MEDIUM-tier thinking model off mid-`<think>` before it reached a tool
    call — at only 45% context use — and the loop then mis-read the truncated stub
    as a completion attempt (`finish_reason` was captured for the scorecard but
    never acted on). Two phases: (01) make `max_tokens` a `[executor]` /
    `[models."<id>"]` config knob (default 8192) threaded through the backend; (02)
    in the `NoToolCall` arm, route a `finish_reason == "length"` turn to a
    truncation-specific recovery nudge instead of the completion path, and escalate
    the empty-recovery feedback to a no-reasoning directive after ≥ 2 consecutive
    empties. Additive; a dedicated truncation terminator is deferred (recover
    first — the loop stays bounded by the turn cap and M22's empty stall). A
    third cleanup phase collapsed the three sampling knobs into a `SamplingParams`
    struct (retiring phase-01's `too_many_arguments` allow) and fixed a latent
    `format_no_match` multibyte-slice panic.
24. **M24 — Edit-Loop Recovery** *(done; committed scope — single phase)*. Enrich the `patch` tool's
    no-op error so the executor recovers before the governor halts it. Diagnosed
    from a netviz e2e run where a MEDIUM-tier model introduced duplicate constants
    with one patch, then re-emitted a byte-identical `old_str`/`new_str` (a no-op)
    six times — the flat `no-op patch: old_str equals new_str` message gave it
    nothing to act on — until M22's `IdenticalToolCallRepetition` stall fired three
    turns later. Phase-01 moves the no-op check below the file read and replaces the
    dead-end string with a recovery message: the current `path:start-end` location,
    a line-numbered context window (mirroring `fuzzy_hint`), an occurrence-count
    note when the text appears more than once (the duplicate tell), and a
    `read_file`/move-on next step. Recover-first per M23: the governor stall stays
    the backstop, unchanged.
25. **M25 — Polish & Config Pass** *(done — 9/9 phases, 2026-06-30)*. A grab-bag of
    operator-facing polish, executor-configuration fixes, and dependency
    major-version bumps batched into one milestone, grouped by subsystem. Two
    threads. **Polish/config (01–04):** (01) `update_task` returns an actionable
    recovery hint (the canonical call shape + the still-incomplete task ids) when it
    receives null/empty/malformed arguments, instead of the generic
    `invalid arguments — expected {id, state}` advisory — the null→`{}` coercion at
    `parser/validate.rs` stays; only the tool's message gets richer. (02) A new
    `enable_thinking` knob (`[executor]` default **false**, per-model overridable
    via `[models."<id>"]` like `temperature`/`seed`/`max_tokens`) emitted on the
    wire as `chat_template_kwargs.enable_thinking` so reasoning models default to
    thinking off. (03) Budget panel — show the Executor/Architect savings rows only
    when their cost is > $0.00 and render them as parenthesized debits; Session
    panel — remove the `Last update` line. (04) Activity panel — wrap on word
    boundaries instead of mid-word; Tasks panel — double the title pan speed. All
    display-only for 03/04 (no `SessionEvent`/telemetry change); 01/02 are executor
    crate. **Dependency bumps (05–09, added 2026-06-29):** major-version bumps that
    `cargo update` alone cannot reach because they require a `Cargo.toml` constraint
    change — (05) `similar` 2→3, (06) `tree-sitter` 0.25→0.26 + `tree-sitter-python`
    0.23→0.25, (07) `toml_edit` 0.22→0.25, (08) `toml` 0.8→1, (09) `reqwest`
    0.12→0.13. Ordered smallest-to-largest blast radius; each ran the same recipe
    (bump the one constraint → `cargo update -p <crate>` → react only to
    compiler-flagged breaks → verify four gates) and landed with **zero source
    edits**. No crate was *added* by hand; phase-09's bump flips reqwest's default
    TLS backend from native-tls/openssl to rustls + aws-lc-rs (the rustls/aws-lc
    subtree appearing is an accepted automatic resolver consequence, decided with
    the user).
26. **M26 — Polish & Hardening — done** (9/9 phases, 2026-07-08). A
    consistency and hardening pass seeded by the post-M25 whole-codebase review
    (`docs/dev/codebase-review-2026-07-07.md`) rather than a dogfooding failure —
    the review surfaced seams that fail *silently*, so no e2e run trips them. Two
    threads. **Housekeeping (01–04):** refresh the stale `REXYMCP.md` contract
    lines (pre-M5 `mcp`-crate description; a hardcoded "M7 active" frontier —
    replaced by a pointer to `NEXT.md`/this section so it cannot rot again) and
    unify the divergent plugin manifests on one name (`rexymcp`); make
    `rexymcp run-phase` telemeter by default (`--no-telemetry` opt-out) so CLI
    runs stop vanishing from the scorecard; surface today-silent degradations
    (empty-or-missing `STANDARDS.md`, phase-doc heading drift) as
    architect-visible warnings via an additive `warnings` field on `PhaseResult`.
    (A fourth housekeeping item — wiring the client's real `roots/list` into
    `execute_phase` corroboration — was **deferred** 2026-07-07: rmcp 1.8.0
    deprecated `list_roots` per MCP SEP-2577, so wiring it would need a
    forbidden `#[allow(deprecated)]` and build on a protocol feature being
    removed. See the M26 README § "Roots corroboration deferred".) **Loop
    hardening:** extend the read-before-edit gate
    to `write_file` (today only `patch` is gated); make the post-write format
    hook actually rewrite touched files (today it runs the verify-only `--check`
    form — a no-op); wire or retire the dead budget/tier knobs
    (`gate_retries`, `escalation_slots`, `tier`, `max_assists` — defined,
    tested, and written by `calibrate`, consumed nowhere); add governor
    detectors for the blind spots adjacent to M22's (A,B,A,B oscillation,
    windowed cumulative output, an optional wall-clock ceiling); resolve the TS
    verifier's `tsc` via `node_modules/.bin` → `npx` → PATH so it stops
    silently `Skipped`-ing in real Node repos. No new features, no new
    dependencies; schema changes additive only.
27. **M27 — Autonomous Escalation Loop** *(done, 2026-07-09; committed scope
    01–06b, all approved; stretch phase-07 advisory routing not taken)*.
    Makes the architect↔executor cycle run hands-off across a whole milestone
    and closes the two design conversations queued since M22. Three threads.
    **Budget & journal substrate:** consolidate the escalation budget on
    `[escalation] max_assists` (the per-phase autonomous assist budget; retire
    the never-distinct `[budget] escalation_slots`), and give the architect's
    own activities (draft/dispatch/review/assist/takeover) a telemetry journal
    so `PhaseRun.escalation_count` becomes real. **Executor/server autonomy:**
    server-authored bookkeeping (D8/D9 — on a `complete` run the *server*
    writes the Status flip and a baseline Update Log entry from data it already
    holds, so correct code stops dying in the bookkeeping tail; amends the
    executor contract) and `continue_phase` briefing-seeded resume (fresh
    context from phase doc + briefing + guidance + diff, `task_states` restored
    from the session log — the committed third escalate lever). **Architect
    loop & accounting:** a `/rexymcp:auto` plugin skill composing the existing
    four skills into a full-milestone loop with full review rigor and no
    per-phase pause, budgeted by `max_assists`, stopping for the human at
    milestone boundaries / blockers / budget exhaustion with a structured loop
    report; architect token/cost harvested from Claude Code's local session
    transcripts and joined onto journal activities (absent — never estimated —
    on clients without transcripts). Design fixed at kickoff with the user; see
    the M27 README § Design.
28. **M28 — Edit-Tool Arg Recovery** *(done, 2026-07-09; single phase; phase-02 optional, not taken)*.
    Close [issue #1](https://github.com/ryanczak/rexyMCP/issues/1), surfaced by
    the M27 `/rexymcp:auto` live-validation run: near max context a small model
    truncates edit-tool calls and drops the required `path` field, and
    `write_file`/`patch` surface the **raw** `invalid arguments: missing field
    \`path\`` serde error — a dead end the model can't recover from (the run's
    log showed 8× before a stall fired). phase-01 replaces it with a
    model-visible recovery hint (names the missing field, echoes what was
    supplied, gives an example shape + next-step) via a shared
    `missing_args_hint` helper — the same enrich-a-dead-end-tool-error pattern as
    M24. Message-only and deterministic: it does **not** guess the `path` value
    (issue solution 1) or add context-pressure guards (solution 3); both are
    deferred. Extending the helper to the other 8 arg-parsing tools is a possible
    phase-02.
29. **M29 — Cleanup** *(done, 2026-07-09; single phase)*. Two unrelated
    infra fixes found during the M28 dispatch/review: (a) `finalize_complete`
    was guarded on `in-progress` and skipped a phase left at `todo` when the
    executor missed the `todo→in-progress` start-flip (the M28 AEON run) — broaden
    it to finalize a `todo` doc too, the same server-owns-bookkeeping robustness
    as M27 04b; (b) the M26 phase-08 `verify_typescript_spawns_resolved_local_binary`
    test writes-then-exec's a fake `tsc` and flakes on ETXTBSY under parallel
    `cargo test` — replace it with a deterministic `resolve_tsc_command` resolver
    test. One small cleanup phase.
30. **M30 — Executor Interruption** *(done 2026-07-10; opened 2026-07-09)*. Give the
    user **and** the architect a way to stop a running executor mid-phase — today
    there is none. The MCP `execute_phase` becomes an **async job** instead of a
    single blocking call: it spawns the run inside the serve process, registers it
    under a `run_id`, and returns immediately; a new `get_run_status(run_id)` tool
    bounded-long-polls the run and `stop_phase(run_id)` cancels it. Because Claude
    Code sends no MCP `notifications/cancelled` and the architect is itself blocked
    awaiting the call while a phase runs, the client-agnostic path is a filesystem
    sentinel: `rexymcp stop [--run <id>]` writes `.rexymcp/stop`, which a serve-side
    watcher (and the still-blocking CLI `run-phase`) honor — what a human uses from
    a second terminal. The executor loop gains a cooperative cancel check (a
    `tokio::sync::watch`-based `CancelSignal`, no new dependency) evaluated at the
    top of the turn loop and as a third `tokio::select!` branch that aborts a stuck
    in-flight model stream. Cancellation adds a fourth `PhaseResult` status,
    **`cancelled`**, which **leaves the working tree dirty** and reports the partial
    diff + stage + turns-done for the architect (or human) to triage — no
    auto-commit, no auto-revert. This supersedes the "opaque *and blocking*"
    characterization in § Layer 2 "Liveness". Phase-01 lands the executor-side
    primitive (`CancelSignal` + the `cancelled` outcome); later phases add the MCP
    job registry + tools, the CLI stop command + sentinel watcher, and the
    async-polling skill-loop rewrite.
