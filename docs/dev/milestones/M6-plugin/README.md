# M6 — Plugin + architect/review skills

**Goal:** Package rexyMCP as a **Claude Code plugin** so a real architect/executor
session lands. The plugin bundles the M5 MCP server (`rexymcp serve`) with the
workflow that drives it — three skills (`architect`, `review-phase`, `escalate`),
three slash commands (`/rexymcp:architect`, `/rexymcp:dispatch`, `/rexymcp:review`), embedded templates
for `STANDARDS.md` / `WORKFLOW.md` / the executor contract (with `{…_COMMAND}`
placeholders that resolve per target project), and a bootstrap routine that
initializes an uninitialized target repo. Closes with an **end-to-end dogfood**
against a real third-party repo — first real architect→MCP→executor→review loop
without opencode in the picture.

**Status:** in progress — phase-01 drafted (`todo`); later phases drafted on demand.

**Depends on:** M5 (done) — `rexymcp serve` is the binary the plugin wraps;
six tools live; per-tool timeout is *client-side* (M5 phase-02 noted this is
M6's `.mcp.json` work). M4 (done) — `execute_phase_inner` accepts the
`executor_contract` + `standards` strings the plugin will supply.

**Exit criteria:**
- A **plugin package** in `plugin/` (Claude Code plugin layout — verify the
  exact filesystem convention at phase-01) registering the rexyMCP MCP server
  via `.mcp.json` (per-tool timeout raised toward the 10-minute ceiling) and
  exposing three skills + three slash commands.
- **Three skills:**
  - **`architect`** — explore target repo, write design doc + milestone README
    + phase docs into target repo's `docs/dev/`. Owns **project initialization
    (bootstrap)** and **pre-injection** (front-loads worked examples / idioms /
    few-shot tool-call exemplars / fetched reference docs into each phase doc,
    since the local LLM has no live callback to Claude).
  - **`review-phase`** — check executor output against the DoD in
    `STANDARDS.md`, rerun the project's commands, approve or file a bug.
  - **`escalate`** — given a `hard_fail` briefing, pick a lever: re-dispatch
    with refined spec (default for weak models), session takeover, or resume.
- **Three slash commands:** `/rexymcp:architect [next|next-phase]`,
  `/rexymcp:dispatch <phase>`, `/rexymcp:review <phase>` (plus
  `/rexymcp:escalate <phase>` auto-derived from the escalate skill —
  see phase-05).
- **Embedded templates:** generalized `STANDARDS.md` + `WORKFLOW.md` (in the
  plugin's resources, copied into target repo by bootstrap with placeholders
  resolved) and `executor_contract.md` (**embedded in the executor crate
  itself** via `include_str!`, substituted with `cfg.commands` at turn-cycle
  step 1 — *never* written to the target repo per architecture).
- **Bootstrap routine** in the `architect` skill: detect command set
  (`Cargo.toml` → `cargo …`, `package.json` → npm/yarn/pnpm, etc.) and write
  `rexymcp.toml`; lay down resolved process docs; write `CLAUDE.md`; register
  the MCP server in target repo's `.mcp.json`. **Does not** write `AGENTS.md`
  or an executor-contract file. Idempotent.
- **End-to-end dogfood:** at least one real architect → `/rexymcp:dispatch` → `execute_phase`
  → `review-phase` → approval cycle against a non-rexyMCP repo, with the
  whole stack live (Claude Code architect, rmcp stdio server, local LLM
  executor through an OpenAI-compatible endpoint). Surfaces the first
  real-world calibration data for M7.

## Architecture references

- `docs/architecture.md` — "Layer 3 — Plugin package" (skills, commands,
  embedded templates, the contract-is-embedded-only rule); "Project
  initialization (bootstrap)" (the four bootstrap steps); "End-to-end flow"
  (the architect → dispatch → review cycle); Status §M6 (gated-by-default
  phase progression + opt-in autonomous loop).
- M5 README — `execute_phase`'s `standards` / `executor_contract` inputs
  (currently empty/per-call; M6 fills them); the carry-forward list (per-tool
  MCP timeout, embedded templates, bootstrap).
- M4 README — the loop's turn-cycle step 1 (assembles the system prompt from
  contract + standards + phase doc); M4 phase-07a's prompt assembly.

## Phases

Expanded on demand (WORKFLOW.md § Milestones), not all at once.

| #  | Phase                                                              | Status |
|----|-------------------------------------------------------------------|--------|
| 01 | plugin scaffold + `.mcp.json` + slash-command stubs ([phase-01-plugin-scaffold.md](phase-01-plugin-scaffold.md)) | done |
| 02 | embedded templates: `executor_contract` + `STANDARDS` + `WORKFLOW` ([phase-02-embedded-templates.md](phase-02-embedded-templates.md)) | done |
| 03 | executor wires the embedded contract ([phase-03-executor-wires-contract.md](phase-03-executor-wires-contract.md)) | done |
| 04 | `architect` skill + bootstrap routine ([phase-04-architect-skill.md](phase-04-architect-skill.md)) | done |
| 05 | `dispatch` + `review` + `escalate` skills ([phase-05-dispatch-review-escalate.md](phase-05-dispatch-review-escalate.md)) | done |
| 06a | dogfood preparation (procedure + log template + pre-flight verification) ([phase-06a-dogfood-prep.md](phase-06a-dogfood-prep.md)) | done |
| 07a | SSE prefill-stall: first-token vs. inter-token timeout + retry ([phase-07a-sse-prefill-stall.md](phase-07a-sse-prefill-stall.md)) | done (approved_after_1: [bug-07a-1](bugs/bug-07a-1.md)) |
| 07b | executor liveness: `awaiting_model` heartbeat during the model wait ([phase-07b-executor-liveness-signal.md](phase-07b-executor-liveness-signal.md)) | todo |

Phases 07a / 07b are **dogfood-surfaced executor-resilience fixes** (smoketest
session `6a1dd72e`), not plugin work — placed here because the M6 dogfood is
what surfaced them. 07a is the root cause (a 90 s first-token timeout misfiring
on slow prefill); 07b closes the idle-vs-dead signal gap on the pull-based
`rexymcp status` path. Two deferred-to-06b design questions they deliberately do
**not** decide: (1) whether a terminal backend `Err` should degrade to a
structured `hard_fail` `PhaseResult` (preserving partial work) rather than abort
`execute_phase`; (2) whether a cross-dispatch resume / "continue if phase status
≠ done" mechanism is warranted. Both touch the contract / error model and are
architect calls for the retrospective.

Tentative remaining phases (draft when the prior one lands):

- **06 — end-to-end dogfood (M6 closer), split into 06a + 06b** because
  06a's prep is opencode-executable but the actual run requires the user
  driving Claude Code + the rexyMCP plugin + a local LLM endpoint against
  a real third-party repo (which opencode in dev mode cannot do). Same
  splitting wisdom as M4 phase-07a–e and M5 phase-05a/b.
  - **06a** *(drafted)* — **dogfood preparation**: a step-by-step
    `dogfood-procedure.md` the user follows, a `dogfood-log.md` template
    they fill in, target-repo selection criteria, and pre-flight
    verification that the binary builds + installs + runs (`--help`) +
    the plugin JSON files parse + the validation greps from phases
    02 / 04 / 05 still pass.
  - **06b** *(drafts after 06a lands)* — **dogfood execution + M6
    close**: the user runs through the procedure on their schedule and
    fills in the log; the architect (me) then synthesizes the log into
    the M6 retrospective, decides the compaction-monitoring question
    (heuristic eviction vs justifying a summarization-based-compaction
    follow-up milestone), and folds any patterns the dogfood surfaced.

  Pick a real third-party repo (small, open-source, language ≠ Rust to
  exercise the placeholder substitution); bootstrap it; have Claude
  design a single small phase; dispatch through the live MCP server to a
  local model; review; iterate if bounced. **Captures the first real
  `PhaseRun` telemetry** (the M5 store finally has a non-self data
  point). The retrospective documents what broke and what worked —
  first real-world calibration data for M7.

  **Compaction-monitoring concern (must surface in phase-06's spec).** M4
  shipped *heuristic* compaction (`executor::context::compactor` — signaturize
  old tool-result bodies, then evict oldest non-system messages until under
  75 % of the budget ceiling). What opencode and Claude Code use instead is
  **LLM-based summarization** — a second model call digests early turns into
  a summary that replaces them, preserving semantic content. The user has
  observed that opencode on Qwen-3.6-27B at 262k context still hits
  compaction repeatedly; rexyMCP's deployment target is *smaller* local
  models with *smaller* context windows (32k–128k typical), where heuristic
  eviction will bite harder.

  Phase-06's dogfood is the right place to gather data, not the right place
  to fix it. Capture, per phase run:
  - `CompactionReport { messages_signaturized, messages_evicted,
    tokens_before, tokens_after }` totals — already populated by the loop,
    just needs to surface (probably aggregate into a per-`PhaseRun` field,
    or query out of the session log).
  - Whether the phase still completed cleanly after compaction fired vs
    whether the model "forgot" something evicted (visible as the model
    re-asking for context it already had, repeating tool calls, or
    hard-failing on losing-track-of-state).
  - How early in a phase compaction fires for our target local-model
    context sizes.

  If the data shows heuristic eviction is materially hurting completion
  quality, that justifies a follow-up milestone (likely between M6 and M7):
  **summarization-based compaction**, which needs a summarizer prompt, an
  injection-point in the message list, "what's safe to summarize" rules
  (system prompt: no; tool calls: maybe — they're already
  signaturized once), and telemetry on summary quality. If the data shows
  heuristic eviction is fine for our workload, we save the model-call cost.
  **Do not build summarization speculatively** — wait for the dogfood data.

## Notes

**M6 is largely content, not Rust code.** Phases 01, 02, 04, 05 are
Markdown / JSON / plugin-manifest authoring; only phase-03 is a small Rust
edit. The "calibration" patterns will differ — less about trait bounds, more
about **template content quality** and **pre-injection effectiveness**. The
WORKFLOW folds from M2–M5 still apply (pin negative cases, declare scope
deviations, etc.) but the failure modes will be content-level (a confusing
skill instruction, a missing placeholder, a forgotten step in the bootstrap
sequence) rather than type-system-level.

**Executor contract is embedded-only.** Per architecture, a rexyMCP-driven
project carries no root `AGENTS.md` and no executor-contract file. The
contract lives in `executor/templates/executor_contract.md` (phase-02 creates
it), is `include_str!`'d at compile time (phase-03), and is substituted with
the per-target-project command set at runtime. This is the **load-bearing
design choice** that makes rexyMCP language-agnostic without per-project
contract maintenance.

**Bootstrap is idempotent.** Re-running `/rexymcp:architect` against a repo that
already has the scaffold should *not* clobber existing docs. The architect
skill detects the present state (config exists? STANDARDS.md exists?
CLAUDE.md exists? `.mcp.json` registers rexyMCP?) and only writes the
missing pieces. Phase-04 pins this discipline.

**Per-tool MCP timeout is `.mcp.json`.** Architecture: "per-tool `timeout` is
set well above the 10-second default (toward the 10-minute ceiling)." The
`.mcp.json` schema lets the client know how long it should wait for each
tool. Phase-01 sets this for `execute_phase` (long); the other five tools
use the default. Verify Claude Code's exact `.mcp.json` schema for per-tool
timeout fields in phase-01's pre-flight.

**Pre-flight 3 carries forward.** Phases 02 / 05b / 06 of M5 each had a
pre-flight step that told opencode to verify the live external API
(`rmcp` 1.7) against `cargo doc` rather than trust the architect's sketch.
The same discipline applies to **Claude Code's plugin format**: the
architect can't reliably enumerate the exact filesystem layout, the
`.mcp.json` schema, or the skill/slash-command file format. Phase-01's
pre-flight tells the executor to verify against Claude Code's own
documentation (the docs site, `claude-code-guide` Agent if available, or
working examples in other plugins).

**M6 closer triggers M7 kickoff.** Per architecture, M7 depends on having
data; the dogfood phase produces the first real `PhaseRun` records. The
M6 retrospective will reference how much telemetry accumulated and whether
the `model_scorecard` matrix produces meaningful output yet.
