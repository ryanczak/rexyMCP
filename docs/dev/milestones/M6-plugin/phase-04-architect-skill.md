# Phase 04: architect skill + bootstrap routine

**Milestone:** M6 — Plugin + architect/review skills
**Status:** todo
**Depends on:** M6 phase-01 (done) — `plugin/skills/architect/SKILL.md` stub exists. M6 phase-02 (done) — `plugin/templates/STANDARDS.md` and `plugin/templates/WORKFLOW.md` are what bootstrap copies. M5 (done) — the MCP server bootstrap registers.
**Estimated diff:** ~700 lines (SKILL.md content; no Rust code)
**Tags:** language=markdown, kind=feature, size=l

## Goal

Fill in the **architect skill** — `plugin/skills/architect/SKILL.md` — with
the complete prompt that drives Claude when the user invokes `/architect`.
This is the heaviest content phase in M6: the skill covers four
responsibilities, all interlocking.

1. **Bootstrap routine (idempotent)** — on first invocation against an
   uninitialized target repo, lay down the rexyMCP scaffold per
   architecture's four-step bootstrap (resolve commands → write
   `rexymcp.toml` → copy resolved process docs → write `CLAUDE.md` →
   register MCP server in `.mcp.json`). On re-invocation, only fill
   missing pieces.
2. **Explore-then-design** — survey the target repo, write the design
   doc into `docs/dev/architecture.md`, decompose into milestones.
3. **Phase-doc authoring** — write phase docs against the embedded
   `WORKFLOW.md` templates verbatim; update `NEXT.md` and the milestone
   README's phase table; flip statuses (`todo` → `in-progress` → etc.).
4. **Pre-injection** — front-load each phase doc with what the local LLM
   executor will need (worked examples, codebase idioms, gotchas,
   few-shot tool-call exemplars, fetched reference docs). This is the
   load-bearing concept that makes the architect-executor split work,
   since the executor has no live callback to Claude and no web access.

After this phase, `/architect` is a real entry point. The `/dispatch` and
`/review` skills (phase-05) build on top: dispatch invokes
`execute_phase`, review reads the returned `PhaseResult`. Phase-06 then
exercises the whole chain end-to-end against a real target repo.

## Architecture references

- `docs/architecture.md` — Layer 3 "Plugin package", specifically:
  - The `architect` skill bullet — its three responsibilities (explore,
    design, write phase docs) and the **pre-injection** explicit
    responsibility.
  - "Project initialization (bootstrap)" — the four steps, idempotent.
  - "End-to-end flow" — how `/architect` slots into the architect →
    `/dispatch` → executor → `/review` cycle.
- Status §M6 — gated-by-default phase progression + the opt-in autonomous
  loop. The architect skill must default to *stop after each phase*,
  never auto-advance.
- M6 phase-02: `plugin/templates/STANDARDS.md` and `WORKFLOW.md` — the
  process docs the bootstrap routine copies into target repos with
  `{...}_COMMAND` placeholders substituted.
- M6 phase-03: `assemble_executor_contract` substitution pattern — the
  bootstrap routine substitutes the same four placeholders the same way
  (str-replace, no Jinja, `(not configured)` sentinel for unset commands).
- M5 phase-06: roots corroboration — the architect knows the target repo
  root via `CLAUDE_PROJECT_DIR` and/or Claude Code's `roots/list`. Bootstrap
  writes into that root.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M6 README.
2. Read this entire phase doc.
3. **Verify Claude Code's skill format** before writing:
   - The exact YAML/Markdown frontmatter `SKILL.md` requires (name,
     description, tools, model, etc. — fields and shape).
   - **How skill args reach the skill body** — `/architect next`
     vs `/architect` (no args). Whether the skill prompt receives a
     templated arg variable or whether Claude infers from user text.
   - **The plugin install directory layout** at runtime — where
     `plugin/templates/STANDARDS.md` lives once the plugin is installed
     (relative path? `${CLAUDE_PLUGIN_DIR}` env var? something else?).
     The bootstrap routine reads from this location.
   - **Whether skills can invoke MCP tools** (almost certainly yes — that's
     the point — but verify the syntax for telling Claude "use the
     `execute_phase` tool" in a skill prompt).
   Sources: Claude Code docs, the `claude-code-guide` Agent, existing
   plugin examples. Trust docs over the architect's sketch; flag
   divergence in "Notes for review".
4. Confirm the four phase-02 template files exist (`plugin/templates/
   STANDARDS.md`, `plugin/templates/WORKFLOW.md`, `executor/templates/
   executor_contract.md`). The bootstrap routine references the first two
   (not the third — the contract is embedded in the executor binary).
5. Confirm `rexymcp serve --config <path>` is the binary invocation
   `.mcp.json` registers (per M6 phase-01).

## Spec

### 1. SKILL.md frontmatter

Follow Claude Code's required frontmatter format (Pre-flight 3 — verify
the exact field set). At minimum the skill should declare:

- **name:** `architect`
- **description:** one sentence Claude shows in the slash-command list:
  *"Bootstrap a rexyMCP project, design the work, and author phase docs
  for the local-LLM executor."*
- **model:** Claude (the most capable available — the architect needs to
  reason about design, fetch reference docs, generate phase content).
- **tools:** at least the standard file-edit + web-fetch + Bash + the
  rexyMCP MCP tools (`execute_phase`, the log-query tools,
  `model_scorecard` — even if the skill itself doesn't dispatch
  `execute_phase` directly, it informs the architect's drafting).
- Other fields per Claude Code's convention.

If the convention differs from this sketch, follow the convention and
flag in Notes for review.

### 2. Bootstrap routine (idempotent)

The skill prompt must instruct Claude to **run bootstrap on the first
`/architect` invocation against a target repo that lacks the rexyMCP
scaffold, and skip-or-repair on re-invocation**. The four-step routine
mirrors architecture verbatim:

#### Step 1 — Resolve the command set

- Detect the project's `format` / `build` / `lint` / `test` commands by
  inspecting the repo:
  - `Cargo.toml` → `cargo fmt --check` / `cargo build` / `cargo clippy
    --all-targets --all-features -- -D warnings` / `cargo test`
  - `package.json` → check `scripts.format/build/lint/test`, fall back to
    the detected package manager (`pnpm`, `yarn`, `npm`, `bun`)
  - `pyproject.toml` → `ruff format --check` / `python -m build` /
    `ruff check` / `pytest` (or per the `[tool.poetry.scripts]` /
    `[project.scripts]` table)
  - `go.mod` → `gofmt -l .` / `go build ./...` / `go vet ./...` /
    `go test ./...`
  - Other languages: best-effort detection, then **confirm with the user**
    before writing.
- **Confirm with the user** even on confident detection — the user can
  override (e.g. their `cargo` invocation is `cargo +nightly …` for some
  reason). Use Claude Code's interactive prompts.
- Write the resolved set to `<target_repo>/rexymcp.toml`:
  ```toml
  [executor]
  provider = "openai"  # or "ollama" / "lmstudio" / etc., user-chosen
  model = "<user-chosen>"
  base_url = "<user-chosen>"

  [commands]
  format = "<resolved>"
  build = "<resolved>"
  lint = "<resolved>"
  test = "<resolved>"

  [budget]
  context_length = <integer per model>
  max_context_pct = 70
  max_turns = 40
  escalation_slots = 1

  [telemetry]
  dir = "<user-chosen cross-project store>"
  ```
- If `rexymcp.toml` already exists with all four commands set, leave it
  alone (idempotent). If it exists but some commands are unset, prompt
  the user to fill the missing fields.

#### Step 2 — Lay down resolved process docs

- Read `<plugin-install-dir>/templates/STANDARDS.md` (Pre-flight 3 —
  verify path).
- Substitute the four `{FORMAT_COMMAND}` / `{BUILD_COMMAND}` /
  `{LINT_COMMAND}` / `{TEST_COMMAND}` placeholders with the resolved
  values from step 1 (plain `str::replace`, same as phase-03;
  unset → `(not configured)` sentinel).
- Write to `<target_repo>/docs/dev/STANDARDS.md` **only if it doesn't
  already exist**.
- Repeat for `plugin/templates/WORKFLOW.md` →
  `<target_repo>/docs/dev/WORKFLOW.md`.
- If either file already exists, leave it alone and note this in Claude's
  user-facing summary ("STANDARDS.md already present; not overwritten.
  To refresh from template, delete the file and re-run `/architect`.").

#### Step 3 — Write `CLAUDE.md`

Write `<target_repo>/CLAUDE.md` orienting Claude (in future sessions) as
the architect for this specific project. Content (substitute as needed):

```markdown
# CLAUDE.md

This file orients Claude Code as the **architect** for the <project name>
project, working alongside the rexyMCP MCP server and a local-LLM executor.

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

To dispatch a phase: `/dispatch <phase>`. To review the result:
`/review <phase>`.
```

If `CLAUDE.md` already exists, leave it alone (idempotent). Same "delete
to refresh" guidance.

#### Step 4 — Register the MCP server

- Read `<target_repo>/.mcp.json` if it exists; if not, create it.
- Ensure the `mcpServers.rexymcp` entry per M6 phase-01's `.mcp.json`
  (`command: "rexymcp"`, `args: ["serve", "--config",
  "./rexymcp.toml"]`).
- If a `rexymcp` server is already registered, leave the entry alone
  (idempotent). If a *different* MCP server config exists, merge — don't
  clobber other entries.

#### Idempotency check (top of the routine)

Before any of the four steps, the routine checks "is this repo already
bootstrapped?" by testing for the four artifacts (`rexymcp.toml`,
`docs/dev/STANDARDS.md`, `docs/dev/WORKFLOW.md`, `CLAUDE.md`, and a
`.mcp.json` with rexymcp registered). If all five are present, the
routine reports "Already bootstrapped" and proceeds to design/draft work.
If some are missing, the routine fills only the missing pieces.

The skill MUST NOT:
- Write `AGENTS.md` to the target repo (architecture: executor contract is
  embedded-only).
- Write an `executor_contract.md` file to the target repo (same reason).
- Overwrite existing user-edited files (the user may have customized
  `STANDARDS.md` for their project — respect it).

### 3. Explore-then-design

After bootstrap (or on re-invocation against an already-bootstrapped
repo), the skill proceeds to **explore and design**:

- **Survey the repo:** inspect existing structure, identify the build
  system / language / framework already in use, locate any existing docs
  or specs.
- **Engage the user** for the product goal: what is the project for, what
  capability should the next milestone unlock, what's in/out of scope.
- **Write `docs/architecture.md`** if absent — the design doc covering:
  - the system's three-or-so layers,
  - the major data flows,
  - non-goals (what the project explicitly will NOT do),
  - milestone roadmap (M1, M2, … with one-paragraph each).
- **Write the active milestone README** (`docs/dev/milestones/M<n>-<slug>/
  README.md`) — follow the embedded `WORKFLOW.md` § "Milestone README
  template" verbatim.
- **Update `docs/dev/NEXT.md`** to point at the next phase (or "none" at
  a milestone boundary; see § 4).

### 4. Phase-doc authoring

This is the architect's main steady-state activity. The skill prompt
must direct Claude to:

- **Follow the embedded `WORKFLOW.md` § "Phase doc template" verbatim** —
  every section the template names (Goal / Architecture references /
  Pre-flight / Current state / Spec / Acceptance criteria / Test plan /
  End-to-end verification / Authorizations / Out of scope / Update Log).
- **Size phase docs for one executor session** (typically <500 lines of
  diff per WORKFLOW.md § Phases).
- **Pin behavior, not rendering** (per WORKFLOW.md § Specs pin behavior).
- **Pin negative cases, not just positive ones** (per the same fold).
- **Pre-inject** per § 5 below.
- **On `/architect next`** (the args-bearing invocation): draft the next
  phase doc in the active milestone, write it to disk, update NEXT.md to
  point at it, **stop** (the user explicitly dispatches via `/dispatch`).
- **On milestone boundary** (last drafted phase is `done`): stop, update
  NEXT.md to "none", write the milestone retrospective in the README's
  Notes section, ask the user to sign off before starting the next
  milestone. **Never auto-advance.** (Per architecture: milestone
  boundaries are always a human gate.)

### 5. Pre-injection (the load-bearing concept)

This is the most important section of the skill. The skill prompt must
explain to Claude **why** pre-injection matters and **what** to inject.

**Why:** the local LLM executor has no web access, cannot ask Claude
clarifying questions mid-phase, has a limited context window (often
32k–128k), and may not know the project's idioms. The architect's
capability (web access, deep context, reasoning) reaches the executor
**only through what the phase doc contains** — there is no live channel.

**What to inject in every phase doc that needs it:**

1. **Worked examples.** When the phase asks the executor to do something
   non-trivial, include a concrete *working* example from the codebase
   ("here's how the existing X does Y; do the same shape"). Quote the
   relevant code with `file:line` references.
2. **Codebase idioms.** Project-specific conventions the executor should
   mirror (error-handling patterns, naming conventions, test structure).
3. **Gotchas.** Known-bad patterns to avoid, with concrete examples
   ("don't do X — it caused bug-Y-N which we fixed by Z").
4. **Few-shot tool-call exemplars.** For the target executor model, show
   one or two example tool calls in the exact format that model produces.
   The forgiving parser handles multiple formats but a hint reduces parse
   failures (and the model is more confident when it sees a worked
   example).
5. **Fetched reference / API docs.** When the phase integrates with an
   external library or API, the architect uses Claude's web tools
   (WebFetch / WebSearch) to fetch the relevant docs and **paste the
   relevant excerpts directly into the phase doc** (under "Architecture
   references" or a dedicated "Reference excerpts" subsection).

The skill prompt must include this list and frame it as "if you find
yourself wishing the executor could ask a clarifying question, that's a
sign you need to pre-inject the answer into the spec."

**Anti-patterns** (the skill should explicitly tell Claude to avoid):
- Linking to external docs without quoting the relevant section (the
  executor can't fetch).
- Saying "follow the existing pattern" without showing the existing
  pattern.
- Spec'ing a behavior whose exact wire format only Claude knows.

### 6. Status management

The skill prompt must direct Claude to maintain three pieces of state
consistently:

1. **Phase doc's `Status:` line** — flip `todo` → `in-progress` →
   `review` → `done` as the lifecycle progresses. The architect flips
   `todo` → (whatever the executor reports) → `done` on approval.
2. **Milestone README's phase table row** — must mirror the phase doc's
   status. Out-of-sync states are a bug.
3. **`docs/dev/NEXT.md`** — points at the active phase, or "none" at a
   milestone boundary.

The architect also writes the **Review verdict** block (per WORKFLOW.md
§ Review and Bug-Report Cycle) at every approval — not just when
something goes wrong. Approved_first_try is the most common verdict and
still gets a one-line entry.

### 7. What the architect does NOT do

The skill prompt must explicitly enumerate:

- **Does not execute phases.** Phases run via `/dispatch <phase>` →
  `execute_phase` MCP tool → local LLM. The architect drafts, dispatches,
  reviews; never writes the phase's code itself except to fix architect
  errors (and even then via a bug report or escalation).
- **Does not auto-advance.** After approving a phase, stop. The user runs
  `/architect next` or `/dispatch <next-phase>` to advance.
- **Does not cross milestone boundaries without human sign-off.**
- **Does not write code in `executor/` or `mcp/` of a rexyMCP-using
  project** (those paths are rexyMCP-internal; a target project has its
  own layout).
- **Does not modify `STANDARDS.md` / `WORKFLOW.md` without explicit user
  approval** (per the fold-on-recurring-pattern discipline in WORKFLOW.md
  § Calibration).

### 8. Format conventions for the SKILL.md

- Top-level section headings (`##`) match the seven responsibilities above
  (1–7), so a future architect-skill reader can locate guidance by topic.
- Keep the prompt prose-style (Claude reads it as instructions, not a
  literal program). Use concrete examples (`like this:` + code block)
  rather than abstract principles.
- Cite the embedded WORKFLOW.md and STANDARDS.md by section, not by line
  number (line numbers rot).
- No rexyMCP-internal references (no Rexy, no opencode, no `cargo`
  specifically — the four `{...}_COMMAND` placeholders handle the
  command set).

## Adaptations / decisions

1. **Bootstrap is the skill's job, not a separate program.** Claude
   reading the skill executes the four steps using Claude Code's
   file-edit / shell tools. No new Rust binary.
2. **Idempotency by file-existence check**, not by version-stamping or
   manifest tracking. Simpler; the user manages the lifecycle.
3. **`/architect` (no args) bootstraps + explores + designs;** `/architect
   next` drafts the next phase. **`/architect refresh` is NOT in this
   phase's scope** — if a user wants to refresh templates from the plugin,
   they delete the file and re-run. (Add `/architect refresh` later if
   dogfood shows it's wanted.)
4. **Pre-injection is the load-bearing concept** — explicit, with five
   named injection types. This is what the architect skill is *for*.
5. **No bootstrap-status command** (e.g. `/architect status`). The user
   can run `ls docs/dev/` themselves. Premature.
6. **No new dependency.** The skill is content (Markdown); the bootstrap
   uses Claude Code's existing file-edit tools.

## Acceptance criteria

- [ ] `plugin/skills/architect/SKILL.md` exists and has been **fully
      rewritten** (the phase-01 stub is gone). Frontmatter follows
      Claude Code's convention (pre-flight 3 — flag any divergence from
      the sketch in § 1).
- [ ] **All seven spec sections covered**, with the section heading style
      § 8 names: bootstrap routine, explore-then-design, phase-doc
      authoring, pre-injection, status management, what-the-architect-
      doesn't-do, plus any frontmatter / preamble Claude Code requires.
- [ ] **Bootstrap routine** describes the four architecture steps
      (resolve commands / write rexymcp.toml / lay down process docs /
      write CLAUDE.md / register MCP server) with an **idempotency check
      at the top** that decides skip-or-fill per the rules in § 2's
      "Idempotency check" subsection.
- [ ] **Bootstrap routine includes the explicit MUST-NOT list**: don't
      write `AGENTS.md`; don't write an `executor_contract.md`; don't
      overwrite user-edited files.
- [ ] **Pre-injection section** explicitly names all five injection
      types (worked examples / codebase idioms / gotchas / few-shot
      tool-call exemplars / fetched reference docs) with the framing
      "if you find yourself wishing the executor could ask a clarifying
      question, that's a sign you need to pre-inject the answer."
- [ ] **Status-management section** names the three pieces of state
      (phase doc Status, milestone README table, NEXT.md) and mandates
      the Review verdict block on every approval (not just on
      problems).
- [ ] **What-the-architect-doesn't-do section** explicitly names the
      five prohibitions (no phase execution / no auto-advance / no
      milestone boundary crossing / no executor-or-mcp internals /
      no STANDARDS-or-WORKFLOW edits without approval).
- [ ] **Phase-doc authoring section** mandates following the embedded
      `WORKFLOW.md` § "Phase doc template" *verbatim* (every named
      section), and references the WORKFLOW folds (pin behavior, pin
      negative cases).
- [ ] **The `/architect next` arg behavior** is described: drafts next
      phase, writes to disk, updates NEXT.md, stops (no auto-dispatch).
      The milestone-boundary behavior (stop + retrospective + ask user)
      is also described.
- [ ] **Validation greps** (the M6-calibrated patterns):
  - `grep -rnE '[Rr]exy[Mm][Cc][Pp]' plugin/skills/architect/SKILL.md`
    finds only references that are *legitimately* about rexyMCP (the
    server name, the binary, the workflow). Roughly: the file should
    mention rexyMCP by name when describing the MCP server, the
    binary invocation, the bootstrap output. Pin the count in Notes
    for review.
  - `grep -rnE 'opencode|Rexy(?!MCP)|cargo |Cargo\.toml' plugin/skills/
    architect/SKILL.md` — finds only the bootstrap step-1 command-
    detection examples (the `Cargo.toml → cargo …` mappings, etc.).
    No other rexyMCP-internal leaks.
- [ ] **No Rust code changes.** `cargo fmt --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` all pass unchanged.
- [ ] **No new dependencies.**
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review", even defensible ones. M6
      phase-02 hardened the *conceptual* leak detection — apply the
      gut-check ("would a target project executor reading this paragraph
      know what to do?") to every section of the skill body before
      reporting complete.

## Test plan

This phase ships no Rust code; the four `cargo` gates are unchanged.
Content verification is via the grep checks above plus a **manual read
pass**:

1. Run the validation greps from acceptance criteria. Pin output in the
   Update Log.
2. **Section-presence audit:** `grep '^## ' plugin/skills/architect/
   SKILL.md` — confirms each of the seven spec sections has a top-level
   heading. If a section is split into sub-sections, confirm coverage
   matches.
3. **Bootstrap MUST-NOT enforcement:** grep for the literal phrases
   "AGENTS.md" and "executor_contract" in SKILL.md — they should appear
   *only* in the "don't write these" context, not as instructions to
   write them.
4. **Read-pass:** read the SKILL.md end-to-end pretending to be a fresh
   Claude session invoked via `/architect` against an uninitialized
   third-party Python repo. Confirm:
   - You know what to do on first invocation (run bootstrap).
   - You know what to do on `/architect next` (draft next phase).
   - You know what to do at a milestone boundary (stop + retrospective).
   - The bootstrap step's command-detection rules cover the case
     (Python → `pyproject.toml` → ruff/pytest, or per `[project.scripts]`).
   - The pre-injection guidance gives you concrete instructions, not
     just "remember to pre-inject."
   Flag any place the prompt left you unsure.

No `#[cfg(test)]` blocks (no Rust). End-to-end exercise is phase-06
(dogfood) — that's the first time a real Claude session executes the
skill against a real target repo.

## End-to-end verification

> Not applicable — this phase ships skill content. End-to-end exercise
> lands in M6 phase-06 (dogfood), where Claude invokes `/architect`
> against a real target repo and the bootstrap routine runs against a
> real filesystem.
>
> If a **manual smoke test** is performed (e.g. invoking the skill
> against a throwaway temp directory in this dev environment to
> sanity-check the bootstrap routine reads the templates and writes the
> right files), document the result in the Update Log. Not required.

## Authorizations

- [x] **May rewrite** `plugin/skills/architect/SKILL.md` (replacing the
      phase-01 stub with the full skill body).
- [ ] **No Rust code changes.** No `executor/` or `mcp/` edits.
- [ ] **No new dependencies.** No `Cargo.toml` edits.
- [ ] May **NOT** modify `plugin/skills/dispatch/SKILL.md`,
      `plugin/skills/review/SKILL.md` (phase-05), `plugin/.mcp.json`
      (phase-01), `plugin/.claude-plugin/plugin.json` (phase-01),
      `plugin/templates/*` (phase-02), `executor/templates/
      executor_contract.md` (phase-02), or any other phase doc.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`,
      `WORKFLOW.md`, `AGENTS.md`.
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review". Especially watch for: a section
      where Claude Code's actual skill format diverged from § 1's
      sketch; a bootstrap detection rule (e.g. for a language we
      didn't anticipate) you decided to include; a place the
      grep-precision lesson from M6 phase-01/02 applied.

## Out of scope

- **`/dispatch` skill** — phase-05.
- **`/review` skill** — phase-05.
- **`/escalate` skill** — phase-05.
- **`/architect refresh`** (force-refresh templates from plugin) —
  Adaptation 3.
- **`/architect status`** (introspection) — Adaptation 5.
- **Dogfood execution** — phase-06.
- **Localization** — English-only.
- **Per-target-project pre-injection examples** — the skill teaches
  Claude *how* to pre-inject; the actual pre-injection content varies per
  project and is the architect's per-invocation responsibility.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
