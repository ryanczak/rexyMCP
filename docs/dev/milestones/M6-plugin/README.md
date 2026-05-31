# M6 — Plugin + architect/review skills

**Goal:** Package rexyMCP as a **Claude Code plugin** so a real architect/executor
session lands. The plugin bundles the M5 MCP server (`rexymcp serve`) with the
workflow that drives it — three skills (`architect`, `review-phase`, `escalate`),
three slash commands (`/architect`, `/dispatch`, `/review`), embedded templates
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
- **Three slash commands:** `/architect [next|next-phase]`, `/dispatch
  <phase>`, `/review <phase>`.
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
- **End-to-end dogfood:** at least one real architect → `/dispatch` → `execute_phase`
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
| 01 | plugin scaffold + `.mcp.json` + slash-command stubs ([phase-01-plugin-scaffold.md](phase-01-plugin-scaffold.md)) | todo |

Tentative remaining phases (draft when the prior one lands):

- **02 — embedded templates: `executor_contract` + `STANDARDS` + `WORKFLOW`.**
  Net-new generalized Markdown derived from this repo's `AGENTS.md` /
  `STANDARDS.md` / `WORKFLOW.md`, with `{FORMAT_COMMAND}` / `{BUILD_COMMAND}` /
  `{LINT_COMMAND}` / `{TEST_COMMAND}` placeholders. Drops the rexyMCP-internal
  references (Rexy lift protocol, opencode-specific notes, specific
  workspace-layout details). The `STANDARDS.md` + `WORKFLOW.md` templates go
  into the plugin's resources (bootstrap copies + resolves them). The
  `executor_contract` template goes into **`executor/templates/`** so the
  executor crate can `include_str!` it for runtime prepending (phase-03 wires
  it in).
- **03 — executor wires embedded contract.** Small executor edit: `include_str!`
  the contract template, add a `assemble_executor_contract(cfg: &CommandConfig)
  -> String` helper that substitutes placeholders, modify the loop's turn-cycle
  step 1 (prompt assembly) to use it. The `executor_contract` input on
  `execute_phase_inner` (currently passed by `mcp/src/server.rs` as `""`)
  becomes either a no-op or removed; callers no longer pass it.
- **04 — `architect` skill (with bootstrap routine).** The heaviest skill. The
  skill prompt covers: explore-then-design, design-doc / milestone-README /
  phase-doc authoring against the embedded `WORKFLOW.md` templates,
  **pre-injection** (worked examples + idioms + few-shot exemplars + fetched
  reference docs baked into each phase doc), and the **bootstrap routine** (four
  steps from architecture: detect command set → write `rexymcp.toml` → lay down
  resolved process docs → write `CLAUDE.md` → register `.mcp.json`).
  Idempotent. The slash command `/architect` invokes the skill.
- **05 — `review-phase` + `escalate` skills + slash commands.** Smaller content
  phase. `review-phase` reads the returned `PhaseResult` (or the briefing on
  hard-fail) against the resolved `STANDARDS.md` DoD, reruns the project's
  command set, approves (status flip + Update Log) or files a bug. `escalate`
  takes a briefing, picks a lever (refined re-dispatch is the weak-model
  default; session takeover or resume are alternatives). `/review <phase>` and
  `/dispatch <phase>` map to these.
- **06 — end-to-end dogfood (M6 closer).** Pick a real third-party repo (small,
  open-source, language ≠ Rust to exercise the placeholder substitution);
  bootstrap it; have Claude design a single small phase; dispatch through the
  live MCP server to a local model; review; iterate if bounced. **Captures the
  first real `PhaseRun` telemetry** (the M5 store finally has a non-self data
  point). The retrospective documents what broke and what worked — first
  real-world calibration data for M7.

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

**Bootstrap is idempotent.** Re-running `/architect` against a repo that
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
