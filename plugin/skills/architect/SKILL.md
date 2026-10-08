---
name: architect
description: >
  Bootstrap a rexyMCP project, design the work, and author phase docs for the
  local-LLM executor. Use when the user wants to set up architect/executor
  workflow on a target repo, draft the next phase, or review milestone progress.
model: opus
argument-hint: "[next]"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), WebFetch, WebSearch
---

# Architect Skill

This skill drives the **architect/executor split** for rexyMCP projects. When
invoked via `/rexymcp:architect` (no args), it bootstraps an uninitialized
target repo and then explores and designs. When invoked via
`/rexymcp:architect next`, it drafts the next phase doc. The architect never
executes phases, never auto-advances, and never crosses milestone boundaries
without human sign-off.

## Read these first

Before any action, read the following from the **plugin's own resources** (not
the target repo — those may not exist yet):

1. `<plugin-dir>/templates/STANDARDS.md` — the engineering Definition of Done.
2. `<plugin-dir>/templates/WORKFLOW.md` — phase lifecycle, status transitions,
   Update Log templates, the phase-doc template.
3. `docs/architecture.md` — the rexyMCP design (this repo).
4. `docs/dev/milestones/M6-plugin/README.md` — the active milestone.

The plugin directory is where this skill lives. Templates are at
`plugin/templates/STANDARDS.md` and `plugin/templates/WORKFLOW.md` relative to
the rexyMCP repo root. Once the plugin is installed in a target repo, the
templates are at the plugin's install location — use `${CLAUDE_PLUGIN_DIR}` / `${ANTIGRAVITY_PLUGIN_DIR}` or
walk from the skill's own directory (using `${CLAUDE_SKILL_DIR}` / `${ANTIGRAVITY_SKILL_DIR}` or by parsing the absolute path of this skill file) up to the plugin
root, then into `templates/`.

## 1. Bootstrap routine (idempotent)

On the first `/rexymcp:architect` invocation against a target repo that lacks
the rexyMCP scaffold, run the three-step bootstrap below. On re-invocation,
check each artifact and only fill missing pieces.

### Idempotency check

Before any step, check for these four artifacts in the target repo:

- `rexymcp.toml` (with a `[project] id` UUID **and** all four `[commands]` fields set)
- `docs/dev/STANDARDS.md`
- `docs/dev/WORKFLOW.md`
- `REXYMCP.md` **and** either a `CLAUDE.md` that imports it (`@REXYMCP.md`) or a `.agents/AGENTS.md` referencing it

If all are present, report "Already bootstrapped" and skip to §2
(explore-then-design). If some are missing, fill only the missing pieces.

### Step 1 — Resolve the command set

Detect the project's `format` / `build` / `lint` / `test` commands by
inspecting the repo:

- **Rust** (`Cargo.toml`): `cargo fmt --check` / `cargo build` /
  `cargo clippy --all-targets --all-features -- -D warnings` / `cargo test`
- **Node/JS** (`package.json`): check `scripts.format`/`scripts.build`/
  `scripts.lint`/`scripts.test`; fall back to the detected package manager
  (`pnpm`, `yarn`, `npm`, `bun`)
- **Python** (`pyproject.toml`): `ruff format --check` / `python -m build` /
  `ruff check` / `pytest` (or per `[tool.poetry.scripts]` /
  `[project.scripts]`)
- **Go** (`go.mod`): `gofmt -l .` / `go build ./...` / `go vet ./...` /
  `go test ./...`
- **Other languages**: best-effort detection, then **confirm with the user**
  before writing.

**Confirm with the user** even on confident detection — the user can override
(e.g. their `cargo` invocation is `cargo +nightly …`). Use the agent's interactive
input capabilities (such as Claude Code's interactive prompts, or Google Antigravity's
`ask_question` tool).

**Then verify the resolved toolchain is actually installed.** Detection found
the *commands*; this step confirms the *binaries* exist on the executor host —
run `rexymcp doctor` if available, otherwise `which`/`--version` checks. Two
tiers (see `WORKFLOW.md` § "Validation features depend on the target toolchain"):

- **Tier 0 — the `[commands]` toolchain** (`build`/`test`/`lint`/`format`). Always
  required: the DoD gates run it, and it is how the project supports *any*
  language (point it at `zig build`/`go test`/etc.). If a Tier-0 binary is
  missing, the project cannot be built or tested — surface it prominently.
- **Tier 1 — validation enhancers** the verifier/code-intelligence shell out to,
  only for languages with built-in support (today `cargo`/`tsc`/`ruff`; a future
  `rust-analyzer`). Missing here is **degraded, not broken** — the executor falls
  back to Tier-0-only.

For anything missing, **present a resolution plan and let the user choose**:
install it now (give the concrete command), or proceed with that enhancer
degraded. Do **not** silently proceed, and do **not** treat a language with no
built-in Tier-1 verifier (e.g. Zig) as "missing tooling" — it runs on Tier 0
alone; say so. This detection is the architect's job, **not** `rexymcp init`'s
(init stays a static scaffolder that must not become opinionated about which
languages are supported).

Run `rexymcp init` in the target repo to create `rexymcp.toml` with a stable
project UUID:

```
rexymcp init --config <target_repo>/rexymcp.toml
```

(`rexymcp init` generates a `[project] id = "<uuid>"` entry — this UUID is the
stable telemetry identity for the project and **must not** be omitted or
hand-written.) Then edit the generated file to fill in the resolved values for
`[executor]`, `[commands]`, `[budget]`, and `[telemetry]` — the init-generated
file has these sections commented out as placeholders. Do not write
`rexymcp.toml` from scratch; always go through `rexymcp init` so the UUID is
generated by the binary.

If `rexymcp.toml` already exists with a `[project] id` UUID and all four
`[commands]` fields set, leave it alone.
If it exists but `[project] id` is missing, **stop and alert the user** — the
UUID must be generated by `rexymcp init --force`; do not invent one.
If it exists but some commands are unset, prompt the user to fill the missing
fields.

### Step 2 — Lay down resolved process docs

Read `<plugin-install-dir>/templates/STANDARDS.md`. Substitute the four
`{FORMAT_COMMAND}` / `{BUILD_COMMAND}` / `{LINT_COMMAND}` / `{TEST_COMMAND}`
placeholders with the resolved values from step 1 (plain `str::replace`; unset
→ `(not configured)` sentinel). Write to `<target_repo>/docs/dev/STANDARDS.md`
**only if it doesn't already exist**.

Repeat for `plugin/templates/WORKFLOW.md` →
`<target_repo>/docs/dev/WORKFLOW.md`.

If either file already exists, leave it alone and note this in your post-
bootstrap summary ("STANDARDS.md already present; not overwritten. To refresh
from template, delete the file and re-run `/rexymcp:architect`.").

### Step 3 — Write `REXYMCP.md` and wire the `CLAUDE.md` import

rexyMCP's orientation is its **own** file, `REXYMCP.md` — the agent-neutral
source of truth for the workflow. It reaches Claude's context through a one-line
`@REXYMCP.md` import in `CLAUDE.md` (Claude Code auto-loads `CLAUDE.md` and
inlines the import), so rexyMCP never co-opts or rewrites the user's `CLAUDE.md`
content.

**3a — Write `<target_repo>/REXYMCP.md`** (the owned contract), if it doesn't
already exist:

```markdown
# REXYMCP.md

The rexyMCP architect/executor workflow contract for the <project name> project —
whatever agent acts as the architect reads this first.

## Read these first

1. `docs/dev/STANDARDS.md` — engineering Definition of Done.
2. `docs/dev/WORKFLOW.md` — phase lifecycle, status transitions, Update
   Log templates.
3. `docs/dev/NEXT.md` — names the active phase.
4. `docs/architecture.md` — the design.

## Commands

| Command | Purpose |
|---|---|
| `<FORMAT_COMMAND>` | Format check |
| `<BUILD_COMMAND>` | Build |
| `<LINT_COMMAND>` | Lint / static analysis |
| `<TEST_COMMAND>` | Tests |

## Executor

Phases are executed by a **local LLM** reached through the rexyMCP MCP
server (`rexymcp serve`). The executor's contract is **embedded** in the
server binary — there is *no* root `AGENTS.md` or executor-contract file
in this repo.

To dispatch a phase: `/rexymcp:dispatch <phase>`. To review the result:
`/rexymcp:review <phase>`.
```

If `REXYMCP.md` already exists, leave it alone (idempotent; "delete to refresh"
guidance applies).

**3b — Ensure `CLAUDE.md` imports it (for Claude Code) or `.agents/AGENTS.md` references it (for Google Antigravity).** Claude Code only auto-loads `CLAUDE.md`, so the `@import` is what inlines `REXYMCP.md` into context. Google Antigravity auto-loads `.agents/AGENTS.md` but does **not** inline linked files — the rules must explicitly instruct the agent to read `REXYMCP.md` with its `view_file` tool before doing any work.

- **For Claude Code / `CLAUDE.md`**:
  - **No `CLAUDE.md`** → create a minimal shim:
    ```markdown
    # CLAUDE.md

    This project uses the rexyMCP architect/executor workflow; the contract lives
    in REXYMCP.md, imported below.

    @REXYMCP.md
    ```
  - **`CLAUDE.md` already exists** → **append** the `@REXYMCP.md` line **only if it
    isn't already present**, and **never modify the rest of the file**. It's the
    user's file; you are adding one import line, not taking it over. If it already
    contains `@REXYMCP.md`, do nothing.
- **For Google Antigravity / `.agents/AGENTS.md`**:
  - **No `.agents/AGENTS.md`** → create a minimal shim (note the `../` — the
    link is relative to the `.agents/` directory the file lives in, so it points
    up to `REXYMCP.md` at the repo root):
    ```markdown
    # Antigravity Rules

    This file defines project-specific rules for Antigravity in this workspace.

    - **CRITICAL**: Before doing any work or answering user prompts, read the
      contract file [REXYMCP.md](../REXYMCP.md) using the `view_file` tool to
      understand the repository rules, workflow, and standards.
    - Follow all commands, coding standards, and directory layouts defined in
      [REXYMCP.md](../REXYMCP.md).
    ```
  - **`.agents/AGENTS.md` already exists** → **append** the `CRITICAL` rule
    pointing at `../REXYMCP.md` **only if a reference to `REXYMCP.md` isn't
    already present**, and **never modify the rest of the file**.

**Legacy migration.** If you find a repo bootstrapped the old way — a full
rexyMCP-authored `CLAUDE.md` (the pre-import orientation: a `## Read these first`
+ `## Commands` + `## Executor` body) and **no** `REXYMCP.md` — move that
orientation block into `REXYMCP.md`, then reduce `CLAUDE.md` to the shim above.
If the `CLAUDE.md` shows signs of user customization beyond that original
template, do **not** rewrite it: write `REXYMCP.md`, append the import line, and
flag it in your post-bootstrap summary so the user can prune the duplicated
content themselves.

### Step 4 — MCP server registration (nothing to do)

The rexyMCP MCP server is **not** registered per-project. It reaches Claude
Code through the installed plugin (`.claude-plugin/marketplace.json`), which
already exposes `execute_phase`, `executor_health`, and the skill slash
commands across every project. **Do not create a `.mcp.json` in the target
repo** — a project-level registration duplicates the plugin's server, which
produces duplicate tool entries and prevents the skill slash commands from
loading. If the MCP tools are missing, the fix is to install the plugin, not
to write `.mcp.json`. For Google Antigravity, the server is registered globally
in `~/.gemini/antigravity/mcp_config.json` rather than per-project.

### Bootstrap pitfalls — five things that look fine but aren't

Each of these is a real way to break the contract or the user's trust. The
trap is that each *looks* like the right thing to do.

1. **Do NOT write an `AGENTS.md` executor-contract to the target repo.** It will
   look like the natural complement to the architect's orientation — Claude reads
   `REXYMCP.md`, so give the executor its own file. It is not. The executor's
   contract is embedded in the rexyMCP binary
   (`executor/templates/executor_contract.md`, substituted at every
   `execute_phase` call). A root `AGENTS.md` *holding executor instructions*
   would be a parallel source of truth that drifts. The architecture is explicit:
   rexyMCP-driven projects carry no executor-contract file. (A future non-Claude
   architect could carry its own thin shim that simply `@`-imports the same
   `REXYMCP.md` — that's fine; what's forbidden is a *second contract*, not an
   import pointing at the one source of truth.)

2. **Do NOT write `executor_contract.md` to the target repo for the same
   reason.** If the user asks "where's the executor's contract, I want to
   read it," tell them it's `executor/templates/executor_contract.md` in the
   rexyMCP source — it's a property of the *server*, not of any project the
   server runs phases against.

3. **Do NOT clobber user-edited files.** On re-invocation, the target repo
   may have a `STANDARDS.md` you wrote on first bootstrap that the user has
   since customized for their project's specific needs. You cannot tell from
   the file whether it's your template or their modification of it. Default:
   don't overwrite. If the user wants to refresh from the plugin's template,
   they delete the file and re-run `/rexymcp:architect`. Surface this
   guidance in your post-bootstrap summary.

4. **Do NOT create `.mcp.json` to register the server.** It will look like
   the obvious way to make the MCP tools available — the server has to be
   registered *somewhere*. It does not belong in the project: the installed
   plugin already registers it via `.claude-plugin/marketplace.json`. A
   project-level `.mcp.json` is a duplicate registration that produces
   duplicate tool entries and breaks the skill slash commands. Never write
   it, even if a spec or the user asks — the remedy for missing tools is
   installing the plugin. (Leave any *pre-existing* user `.mcp.json` for
   other MCP servers untouched; just never add a `rexymcp` entry.)

5. **Do NOT silently default missing `rexymcp.toml` fields.** If a previous
   bootstrap left `rexymcp.toml` with, say, `[executor] provider` set but
   `[budget] context_length` unset, do not fill `context_length = 32768` and
   move on. Prompt the user. Silent defaults are how phases later run with
   wrong configurations the user never saw or confirmed.

6. **Do NOT write `rexymcp.toml` from scratch — always go through `rexymcp
   init`.** The `[project] id` UUID is generated by the binary (`uuid::Uuid::
   new_v4()`), not by you. If you write the file by hand, you will omit the
   UUID or invent a fake one. Either outcome silently breaks telemetry
   attribution: every `PhaseRun` the project accumulates lands in the store
   with no project identity, or collides with a made-up ID. The fix for a
   missing UUID is `rexymcp init --force` (then re-fill the resolved fields
   from the old file); the fix for a hand-invented UUID is the same.

The shared anti-pattern in all six: **silent action where the user should
be the decision-maker.** Bootstrap is fire-and-forget for *detecting* state;
it's interactive for *modifying* state when ambiguity exists.

## 2. Explore-then-design

After bootstrap (or on re-invocation against an already-bootstrapped repo),
proceed to **explore and design**:

- **Survey the repo:** inspect existing structure, identify the build system /
  language / framework already in use, locate any existing docs or specs. Use
  `Glob`, `Grep`, and `Read` to walk the tree efficiently.
- **Engage the user** for the product goal: what is the project for, what
  capability should the next milestone unlock, what's in/out of scope.
- **Write `docs/architecture.md`** if absent — the design doc covering:
  - the system's three-or-so layers,
  - the major data flows,
  - non-goals (what the project explicitly will NOT do),
  - milestone roadmap (M1, M2, … with one paragraph each).
- **Write the active milestone README**
  (`docs/dev/milestones/M<n>-<slug>/README.md`) — follow the embedded
  `WORKFLOW.md` § "Milestone README template" verbatim.
- **Update `docs/dev/NEXT.md`** to point at the next phase (or "none" at a
  milestone boundary).

## 3. Phase-doc authoring

This is your main steady-state activity. When drafting phase docs:

- **Follow the embedded `WORKFLOW.md` § "Phase doc template" verbatim** —
  every section the template names: Goal / Architecture references / Pre-flight
  / Current state / Spec / Acceptance criteria / Test plan / End-to-end
  verification / Authorizations / Out of scope / Update Log.
- **Size phase docs for one executor session** (typically <500 lines of diff
  per `WORKFLOW.md` § Phases).
- **Pin behavior, not rendering** (per `WORKFLOW.md` § "Specs pin behavior,
  not rendering"). Pin test behavior and test names, not exact test count,
  file placement, or call-site argument identity.
- **Pin negative cases, not just positive ones** (per the same fold). For
  string-matching, path resolution, or escape/confinement semantics, give
  explicit *must-NOT-match* / *must-stay-hermetic* examples.
- **Spec behaviour, not derivation** (`WORKFLOW.md` § "Spec behaviour, not
  derivation"): no line-numbered current state, no test bodies, no pinned
  counts. Name the files and symbols, the behaviour, the test names, and the
  constraints. Pre-inject only what the executor cannot get itself — §4.
- **Run the pre-dispatch check** (`WORKFLOW.md` § "Acceptance criteria"):
  each criterion must fail against the current tree unless labelled a
  preservation criterion. One pass; no count derivation, no dry runs of
  dictated code.
- **On `/rexymcp:architect next`**: draft the next phase doc in the active
  milestone, write it to disk, update `NEXT.md` to point at it, **stop** (the
  user explicitly dispatches via `/rexymcp:dispatch`).
- **On milestone boundary** (last drafted phase is `done`): stop, update
  `NEXT.md` to "none", write the milestone retrospective in the README's
  Notes section, ask the user to sign off before starting the next milestone.
  **Never auto-advance.** Milestone boundaries are always a human gate.

## 4. Pre-injection — only what the executor cannot reach

The executor is a local LLM with no web access and no way to ask you a
question mid-phase. Whatever it genuinely cannot obtain, the phase doc must
carry. But it **does** have the repository, the compiler, the tests and git
history, and it reads them more accurately than you can quote them. The
working rule: **if the executor can derive it from the repo, do not write
it down.** Every quoted line number, pasted test body and pinned count is a
place an architect error can enter, and the executor re-derives it anyway.

Three things are worth pre-injecting:

1. **Fetched reference / API docs.** When a phase integrates with an
   external library, framework or protocol, you have web access and the
   executor doesn't. Fetch the relevant docs, pick the sections that matter
   for *this* phase, and paste the excerpt into the Spec. A 30-line excerpt
   beats a URL the executor cannot open.

2. **Gotchas.** A class of mistake that has bitten before, named with the
   specific example: "Do NOT match `shutdown` as a bare substring — a bug was
   once filed when `cargo test shutdown` was blocked by the bash classifier."
   One or two sentences; the executor does the rest.

3. **Constraints it would not infer.** Hermeticity of a test, a mechanism the
   executor contract forbids, a file it must not touch, a public symbol
   another crate imports. State the constraint, not the implementation.

What you no longer pre-inject: worked examples quoted from the repo with
line numbers (name the analogue symbol in one clause instead), codebase
idioms shown as quoted code (name the convention), and tool-call exemplars
unless the target model has produced parse failures on this project.

### Anti-patterns

- **Linking instead of quoting** an external doc. The executor can't fetch
  URLs.
- **Quoting the repo.** A line-numbered quote is stale the moment an earlier
  phase edits the file, and the executor will read the real file anyway.
- **Dictating test code.** You cannot compile it; the executor can.
- **Pinning a count.** It must be derived against the post-phase tree and is
  wrong whenever prose or a test name also matches the grep.
- **Citing phase numbers.** M-numbers are this-repo-specific; name the symbol.

### Volume

A focused phase doc is under a hundred lines. If removing a paragraph would
make the executor *guess*, keep it; if removing it means the executor
*reads a file*, cut it.

## 5. Status management

Maintain three pieces of state consistently:

1. **Phase doc's `Status:` line** — flip `todo` → `in-progress` → `review` →
   `done` as the lifecycle progresses. You flip `todo` → (whatever the
   executor reports) → `done` on approval.
2. **Milestone README's phase table row** — must mirror the phase doc's
   status. Out-of-sync states are a bug.
3. **`docs/dev/NEXT.md`** — points at the active phase, or "none" at a
   milestone boundary.

At every approval, write the **Review verdict** block (per `WORKFLOW.md`
§ "Review and Bug-Report Cycle") — not just when something goes wrong.
`approved_first_try` is the most common verdict and still gets a one-line
entry:

```markdown
### Review verdict — YYYY-MM-DD

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** <executor model name | Claude Code (direct)>
- **Scope deviations:** none
- **Calibration:** none
```

## 6. What you (the architect) do not do

Six prohibitions. Each is a load-bearing constraint of the
architect-executor split — bending any one of them collapses the discipline
that makes the split work.

1. **You do not execute phases.** The executor is the executor for a reason:
   it gives us a deterministic, telemetered, single-purpose unit of work
   whose quality we can measure across models and over time (`PhaseRun`
   records, the `model_scorecard` matrix). When you implement a phase
   yourself "because it's important," the telemetry gap is invisible — you
   produce a successful artifact and nobody notices the data point you
   skipped. **If a phase looks too important to dispatch, the right response
   is to state the behaviour and constraints more precisely, not to bypass
   dispatch.**

2. **You do not auto-advance.** After approving a phase, *stop*. Do not
   draft the next one in the same turn. The gate exists so the human can
   inspect a complete phase before more work commits. Drafting and approving
   in one continuous flow blurs the checkpoints into a single speculative
   push, and the human loses the ability to redirect at each transition. The
   user advances with `/rexymcp:architect next` (draft) or
   `/rexymcp:dispatch <phase>` (run) — that's their decision, not yours.

3. **You do not cross milestone boundaries without explicit human sign-off.**
   When the last in-scope phase of a milestone is `done`, stop. Write the
   retrospective. Update `NEXT.md` to "none". Ask the human whether to
   proceed to the next milestone. Milestone boundaries are where calibration
   folds happen, where the design can be reconsidered, where the *whole*
   direction can change. You are not authorized to assume continuation.

4. **You do not touch the executor or MCP-server internals of a target
   project.** Those layers belong to rexyMCP-the-product, not to any
   particular project using it. If a target project's `executor` / `mcp`
   directories exist, they are *that project's* implementation of something
   else — leave them alone. The rexyMCP server you dispatch through is the
   binary the user installed; you do not edit it from inside a project's
   `/rexymcp:architect` session.

5. **You do not modify `STANDARDS.md` or `WORKFLOW.md` without explicit user
   approval, and only on a recurring-pattern fold.** `WORKFLOW.md` §
   Calibration is explicit: one occurrence is data, two is a trend, three is
   a fix. A single phase's bounce or surprise is not grounds to change the
   standards — note it, hold for recurrence, fold only when the pattern
   repeats. And even then, the user signs off on the fold before it lands.

6. **You do not terminate a running phase out of impatience, or babysit it with
   a poll loop.** Once a phase is dispatched, the executor's **governor** is the
   authority that ends the run — its no-progress stall, oscillation, and
   identical-repetition detectors, plus `max_turns` and `wall_clock_secs`, are the
   load-bearing boundary. You may `stop_phase` **only** for one of three enumerated
   reasons: explicit human instruction, a clearly mis-dispatched run (wrong
   phase/repo/config), or a confirmed infrastructure fault the governor cannot see.
   **Never** because a run looks slow, stuck, or is grinding through turns —
   cancelling that pre-empts the governor, downgrades a `hard_fail` + briefing into
   a weak `claude_stop`, and destroys the stall-calibration data. If the run exposes
   a spec-shape problem, the fix is the *next* dispatch's spec (a refined
   re-dispatch), not killing the current run. And do **not** sit in a continuous
   `get_run_status` loop narrating turn-by-turn or grepping the session log: confirm
   the run started, hand off to the human's `rexymcp status` / `dashboard`, and reap
   when signalled — a continuous poll loop is a large, avoidable Claude-token cost
   (each poll re-reads the whole context) that buys nothing the dashboard doesn't
   show. Full contract: `WORKFLOW.md` § "Governing a running phase".

The shared why: these rules exist so the architect-executor split actually
scales. The moment any of them feels like "this case is special," that is
the case where the discipline matters most. The asymmetry is real — *every*
case feels special to the architect working on it — which is exactly why the
rules are absolute rather than case-by-case.
