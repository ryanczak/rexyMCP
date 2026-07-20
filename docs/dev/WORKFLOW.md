# Development Workflow

How rexyMCP is built: who does what, what a phase looks like, and how work moves
from "planned" to "merged."

This is **also the workflow rexyMCP itself runs** — Claude as architect, a local
LLM as executor. The plugin (M6) ships generalized copies of this file and
`STANDARDS.md` as the templates the `architect` skill writes against. Keep the
two in sync: a lesson learned building rexyMCP folds back into these docs.

## Roles

**Principal engineer / architect.** Owns the architecture, breaks design into
phases, reviews completed phases, writes bug reports, decides scope changes. Does
not normally write implementation code — that's the executor's job. In the
rexyMCP runtime, this role is **Claude (Opus/Sonnet)**.

**Executor.** Implements one phase at a time following the phase doc. Reads
`STANDARDS.md` at the start of every phase. Reports blockers when stuck. Never
invents scope. Never edits files outside the phase's authorization. In the
rexyMCP runtime, this role is a **local LLM** reached through the
`execute_phase` MCP tool.

**Human (project owner).** Decides direction, vetoes architectural choices, runs
the show. Both the architect and the executor work for the human.

---

## Plugin and MCP server bootstrap

rexyMCP reaches Claude Code as a **plugin**, not as a project-level `.mcp.json`
entry. The plugin registration lives in `.claude-plugin/marketplace.json` at
the rexyMCP repo root. Installing the plugin once makes the `execute_phase`,
`executor_health`, and related MCP tools — as well as the skill slash commands
(`/rexymcp:dispatch`, `/rexymcp:review`, etc.) — available across all projects.

### Architect bootstrap checklist (new project engagement)

Before dispatching any phase in a project, confirm both of these:

1. **Plugin is installed.** The MCP tools and skills must be available in the
   current Claude Code session. Verify by calling `executor_health` or checking
   that `/rexymcp:dispatch` is a recognized slash command. If they are absent,
   ask the user to install the plugin via the Claude Code plugin marketplace.
   Do **not** attempt to create a `.mcp.json` in the project directory.

2. **`rexymcp.toml` exists.** Run `rexymcp health` or call `executor_health` to
   confirm the executor endpoint is reachable. If `rexymcp.toml` is missing,
   tell the user to run `rexymcp init` — this scaffolds a fully-documented
   template. If `rexymcp` is not yet installed, point the user to the plugin
   marketplace.

3. **Target toolchain is present.** Confirm the binaries rexyMCP's validation
   features shell out to are installed and on PATH (run `rexymcp doctor` once it
   exists, or `which`/`--version` checks): the Tier-0 `[commands]` toolchain
   (always required — the DoD gates run it), plus the Tier-1 enhancers for the
   project's language (`cargo`/`tsc`/`ruff`, future `rust-analyzer`). If a binary
   is missing, **present a resolution plan** and let the user choose — install it,
   or proceed with that enhancer degraded (the runtime falls back per the
   "Validation features depend on the target toolchain" calibration below). A
   language with no built-in Tier-1 verifier (e.g. Zig) is fine: it runs on
   Tier-0 alone — say so rather than treating it as missing tooling.

### `.mcp.json` is forbidden in project directories

Placing a `.mcp.json` in a project directory **conflicts with the plugin
installation**: Claude Code sees two server registrations for the same name,
which produces duplicate tool entries and prevents skill slash commands from
loading. **Never create, suggest creating, or write `rexymcp init` output to
`.mcp.json`.**

The `rexymcp init` command writes only `rexymcp.toml`. This is intentional:
the MCP server is already registered by the plugin; `rexymcp.toml` is the
project's runtime configuration, not a server registration.

---

## Hierarchy

```
Milestone           — a coherent capability (M1 Foundations, M2 Tools, …)
└── Phase           — one executor session's worth of work; one markdown file
    └── Task        — a single concrete change (one function, one file, one test)
```

A **milestone** is large (weeks of work). A **phase** is small (one focused
executor session, ideally < 500 lines of diff). If a phase is bigger than one
session, it's two phases — re-split it.

---

## Directory Layout

```
docs/dev/
├── STANDARDS.md                       engineering contract; read every phase
├── WORKFLOW.md                        this file
└── milestones/
    └── M<n>-<slug>/
        ├── README.md                  milestone overview
        ├── phase-01-<slug>.md         a phase doc
        ├── phase-02-<slug>.md
        └── bugs/
            └── bug-<phase>-<n>.md      review-finding bug reports
```

Phases are numbered in execution order. Phases that can run in parallel share a
parent number with letter suffixes (`phase-03a-x.md`, `phase-03b-y.md`).

---

## Milestones

Milestones come from the **Status section at the bottom of
`docs/architecture.md`** — that list is the project plan. Each entry becomes a
milestone with its own `M<n>-<slug>/` directory. The architect expands a
milestone into phases **on demand, not all at once**, because earlier phases
reveal information that shapes later ones.

### Milestone README template

```markdown
# M<n> — <Title>

**Goal:** <one sentence: what capability this milestone unlocks>

**Status:** planning | in-progress | review | done

**Depends on:** M<earlier> (or "none")

**Exit criteria:**
- <verifiable condition>
- <verifiable condition>

## Architecture references

- `docs/architecture.md#<section>`

## Phases

| #  | Phase                                  | Status      |
|----|----------------------------------------|-------------|
| 01 | <slug> ([phase-01-<slug>.md](...))     | todo        |
| 02 | <slug> ([phase-02-<slug>.md](...))     | todo        |

## Notes

<freeform: design decisions made during the milestone, dead ends, things
future milestones depend on>
```

---

## Phases

A phase is **one self-contained unit of implementation work** an executor can
complete in one session without ambiguity. Phase specs are written to leave no
scope or architecture decisions open; the executor picks implementation details
unless the spec is explicitly prescriptive (e.g. for load-bearing types every
later phase imports).

The `Tags:` frontmatter line categorizes the phase (language, kind, size) so the
per-phase `PhaseRun` metrics can be aggregated into the model competency matrix
(see `docs/architecture.md` § "Model effectiveness metrics & routing"). The
architect sets it when drafting; keep the vocabulary consistent across phases so
the scorecard buckets cleanly.

### Phase doc template

```markdown
# Phase <n>: <Title>

**Milestone:** M<n> — <name>
**Status:** todo | in-progress | blocked | review | done
**Depends on:** phase-<m> (or "none")
**Estimated diff:** ~<n> lines
**Tags:** language=<rust|go|python|ts|…>, kind=<feature|refactor|bugfix|test>, size=<s|m|l>

## Goal

<One or two sentences. What does this phase accomplish? Why now?>

## Architecture references

Read before starting:

- `docs/architecture.md#<section>` — <one line on why>

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

<What exists in the repo today that this phase will modify. Specific file paths
and line numbers. Quote the relevant code if short.>

## Spec

Numbered tasks in execution order. Each names the exact file to edit and the
change to make. Three formats are accepted by the task seeder and all populate
the executor's Tasks panel:

- **List item:** `N. **<Task name>** — in \`<path>\`, <change>.` — concise;
  good when each task fits on one line.
- **Numbered subheading:** `### N. <Task name>` followed by detail paragraphs —
  good when a task needs code examples or sub-steps.
- **`Task`-prefixed subheading:** `### Task N — <Task name>` followed by detail
  paragraphs — the same as the numbered subheading, written in the natural
  "Task N" prose style. The separator after the number may be an em-dash
  (`—`, U+2014), a colon (`:`), or a dot (`.`).

All three can coexist in the same `## Spec` section. The seeder keys each task by
its number `N`, so the executor's `update_task(id="N", …)` calls match the seeded
ids — do **not** key updates by the phase number. The section ends at the next
`## ` heading (two hashes + space). A decimal like `### 1.5x` is deliberately
**not** seeded (it is not a task).

1. **<Task name>** — in `<path>`, <change>. <Why if non-obvious.>

## Acceptance criteria

Verifiable conditions — each one checkable by running a command or reading a file.

- [ ] `<command>` produces `<expected output>`.
- [ ] Test `<test_name>` passes.

## Test plan

Concrete tests to write — names + what they assert. Typically unit tests against
hermetic fakes (`TempDir`, `MockAiClient`, fixture replay).

- `test_<name>` in `<path>` — asserts <behavior>.

## End-to-end verification

Unit tests with hermetic fakes can pass while the real artifact the phase ships
is broken. For every acceptance criterion that references a real artifact (a
checked-in file, a CLI behavior, a binary entrypoint, a config the running binary
loads), verify against that real artifact before reporting complete, and quote
the actual output in the completion Update Log.

If the phase ships **no** runtime-loadable real artifact (a pure internal
refactor, a new private type, a test-only helper), write:

> Not applicable — phase ships no runtime-loadable artifact. <one sentence why>

## Authorizations

If this phase needs anything from STANDARDS.md §5, declare it here:

- [ ] May add dependencies: `<crate-name>`.
- [ ] May touch `docs/architecture.md` (specifically: <which section>).

(If nothing is authorized, write "None.")

## Out of scope

What the executor must **not** do, even if tempted. Things that look related but
belong to a later phase.

- <scope boundary>

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
```

---

## Update Log entries

Three entry types — use whichever fits.

### Progress note (in-progress)

```markdown
### Update — YYYY-MM-DD HH:MM (progress)

<One paragraph: what you've done since the last update, what you're working on
now, anything surprising. No need to log every micro-step.>
```

### Blocker (stop and wait)

```markdown
### Update — YYYY-MM-DD HH:MM (blocker)

**Blocked on:** <one-line summary>
**What I tried:** <concrete attempts, in order>
**What I need:** <decision | clarification | authorization>
```

### Completion (phase done)

```markdown
### Update — YYYY-MM-DD HH:MM (complete)

**Summary:** <one paragraph: what was built, any deviations from the spec and why>

**Acceptance criteria:** all ticked above.

**Commands:**

```
{FORMAT_COMMAND}
<paste output>

{BUILD_COMMAND} 2>&1 | tail -20
<paste tail output>

{LINT_COMMAND} 2>&1 | tail -20
<paste tail output>

{TEST_COMMAND} 2>&1 | tail -30
<paste tail output>
```

**End-to-end verification:**

For each command in the phase doc's E2E section, paste the actual output. (If the
phase doc declared E2E N/A, restate the reason in one line.)

**Files changed:**
- `<path>` — <one-line summary>

**New tests:**
- `<test_name>` in `<path>`

**Commits:**
- `<sha>` — <subject line>

**Notes for review:** <anything the reviewer should know>
```

---

## Review and Bug-Report Cycle

When the executor marks a phase **review**, the architect:

1. Reads the phase doc + diff + Update Log completion entry.
2. Runs the commands themselves to confirm they actually pass.
3. Spot-checks the tests are real (not passing via assertion omission).
4. Either **approves** (flips to `done`, updates the milestone README's phase
   table) or **rejects** (writes bug reports in the milestone's `bugs/`
   directory and flips the phase back to `in-progress`).
5. **Records a structured review verdict** (below) — at every approval, not just
   when something went wrong. This is the supervision label for the model eval
   *and* the substrate for the human project review (architecture.md § "One
   review record, two consumers"). One write, two consumers — don't keep the
   verdict only in the commit message, where it isn't queryable.

### Review verdict

Append to the approved phase's Update Log (and, once `PhaseRun` exists in M4,
populate the matching `PhaseRun` supervision fields from the same values):

```markdown
### Review verdict — YYYY-MM-DD

- **Verdict:** approved_first_try | approved_after_N | rejected | escalated
- **Bounces:** <count> (bugs: <id(s)> — <max severity>, or "none")
- **Executor:** <local model / opencode | Claude Code (direct)>
- **Scope deviations:** <what the phase cut/deferred vs. its sketch, or "none">
- **Calibration:** <fold filed / lesson, or "none">
```

Keep it terse — it's a label, not a narrative. The milestone retrospective rolls
these up at close; a future `project_review` tool (M7) reads them back.

### Bug report template

File at `docs/dev/milestones/M<n>-<slug>/bugs/bug-<phase>-<n>.md`.

```markdown
# Bug <n> on phase-<phase>: <One-line title>

**Severity:** blocker | major | minor | nit
**Status:** open | acknowledged | fixed | verified
**Filed:** YYYY-MM-DD

## What's wrong
<Concrete. Quote the offending code with file:line. State observed behavior.>

## What should happen
<Concrete. Reference the architecture doc section or phase spec requirement.>

## How to fix
<Specific instruction: file path, what to change, expected result.>

## Verification
- [ ] <command produces expected output>
- [ ] <test_name passes>
```

### Severity meanings

- **blocker** — phase cannot be merged in this state.
- **major** — must fix before done; correctness or contract violation.
- **minor** — should fix; style, naming, a missing-but-not-critical test.
- **nit** — optional preference; executor may decline with reasoning.

---

## Status Flow

```
todo ──► in-progress ──► review ──┬─► done
                  ▲                │
                  └────────────────┘ (bug report filed)
              ▲
              └─ blocked   (executor waiting on architect)
```

The status lives in the phase doc's frontmatter and is mirrored in the milestone
README's phase table. The two **must** match.

---

## Phase progression & triggers

"Mark a phase done" and "write the next phase" are **separate acts**. Marking
done — flipping the phase to `done`, updating the README phase table, committing
— is a checkpoint. Drafting the next phase is a fresh decision that benefits from
the just-finished work being on disk. Keeping them separate lets the human
inspect before more work is generated.

**Default: gated.** After a review passes, the architect marks the phase `done`
and **stops**. The user advances explicitly — e.g. `/architect next` to draft the
next phase doc, or `/dispatch <phase>` to run the next already-written phase. The
architect does not draft or dispatch the next phase on its own. This keeps the
review a real gate and the human in control of scope.

**Milestone boundaries are always a human gate.** When a milestone's in-scope
phases are all `done`, the architect stops for human sign-off regardless of mode.
This is where the retrospective and doc-folding happen (see "Calibration"), which
is human judgment — never automated.

**Opt-in autonomous loop (off by default).** For hands-off runs, the user may
start an explicit `/rexymcp:auto` run (M27) that chains draft → dispatch →
review → escalate/re-dispatch across phases with **full review rigor and no
per-phase pause** — the review procedure runs verbatim (independent gate
re-runs, DoD walk, telemetry verdict, commit); only the human pause between
steps is removed. It is explicitly enabled per run, never the default, and it
**composes** the interactive skills rather than forking them — a behavior
difference between an interactive and an autonomous run of the same step is a
bug. Dispatch drives `execute_phase`'s **async contract** — it polls
`get_run_status` to reap each spawned run — and a running phase is
**interruptible** out-of-band (`rexymcp stop` for the human, `stop_phase` for the
architect between polls), which the loop treats as a deliberate human signal. The
loop stops for the human on: a milestone boundary (always), any blocker or "What
Executors Never Decide" item, exhaustion of the per-phase assist budget
(`[escalation] max_assists` autonomous escalation round-trips on one phase), the
loop-level runaway backstop, or a phase returning **`cancelled`** (a deliberate
`rexymcp stop` / `stop_phase` interrupt — the loop surfaces the partial work and
hands back, never silently re-dispatching a stopped phase). Every stop produces a
**loop report** — phases run, verdicts, assists spent, token/cost totals where
harvested, and why it stopped — so the human resumes from a briefing, not a
scrollback dig. Every architect activity in the loop is journaled to the
telemetry store; token usage is harvested from the client's own transcripts
where available and recorded as absent elsewhere, never estimated.

**Route opencode-hostile content to direct execution.** Some content classes
reliably corrupt the opencode executor's tool-call serializer — notably
close-tag-shaped text (`<tool_call>`/`</function>`), markdown fences, and escaped
quotes inside JSON string literals inside Rust source (see the archived
`docs/dev/archive/AGENTS.md` § "Writing files when the opencode tool harness
fails"). When a phase's deliverable is
saturated with that content — the M3 forgiving parser was the canonical case
(5/5 phases) — dispatching to opencode wastes a round of corruption-then-recovery.
The architect should **pre-route such a phase to direct execution** (the principal
engineer / Claude implements it) rather than dispatch. The phase doc still records
who executed it and follows the normal review gate; only the *executor* changes.

> **This is an opencode limitation, not rexyMCP's — and the workaround is
> transitional.** rexyMCP is *not* opencode and must never inherit its tooling
> deficiencies. rexyMCP's executor ingests a weak model's messy output through
> structured tool dispatch (`write_file` / `patch` take content as JSON args) and
> the **forgiving parser** (M3) — which exists precisely to *repair* close-tags,
> fences, and escaped-quote content rather than corrupt on it. So the very content
> that breaks opencode is what rexyMCP is built to handle gracefully. This routing
> rule applies only while opencode is the executor *building* rexyMCP; once
> rexyMCP dogfoods itself (M6), it goes away. Never design a rexyMCP feature around
> an opencode limitation.

---

## What Executors Never Decide

- Whether something belongs in core vs. a plugin.
- Whether to add a dependency.
- Whether to change the architecture doc.
- Whether to skip a test, mark it `#[ignore]`, or `#[allow]` a warning.
- Whether to widen a phase's scope to fix a related issue noticed in passing.
- Whether to deviate from STANDARDS.md "because this case is special."

All of these are blockers. File them in the Update Log and stop.

---

## Calibration — fold lessons in

The workflow this document describes is the same workflow rexyMCP runs as a
product, and the plugin embeds these files verbatim. So **everything learned
building rexyMCP must be folded into these docs** before the architect skill
ships — there is no separate place for "lessons learned for later."

Fold on a **recurring pattern**, not a one-off:

- One occurrence = calibration data; note it, don't change docs yet.
- Two occurrences = trend worth folding; update the relevant doc.
- Three occurrences = the doc was wrong; fold immediately.

Where each lesson lands:

| Lesson | Lands in |
|---|---|
| Executor needs to remember X every phase | `STANDARDS.md` |
| Every implementation should uphold X | `STANDARDS.md` |
| Architect spec-writing / review discipline | this file |
| Phase-doc or bug-report template addition | this file |

The architect revisits both docs **after each milestone closes**, before drafting
the next milestone's phase 01. If no folds are warranted, the milestone README's
Notes section says so explicitly: "M<n> retrospective: no new patterns, no
folds." Silence is not the default.

### Specs pin behavior, not rendering

When writing a phase spec, pin the **test behavior** (what it asserts) and the
**test name** (so coverage is auditable) — but do **not** pin exact test count,
test-file placement, or call-site argument identity. Those are the executor's
structural calls. When pinning a grep literal in the E2E block, pin user-visible
**content**, not source-text rendering (path qualifiers, whitespace nuance,
markdown formatting marks). If you can't decouple content from rendering, use a
prose behavioral assertion and verify by inspection instead of grep.

**Pin negative cases, not just positive ones.** For specs that hinge on
string-matching, path resolution, or escape/confinement semantics, the boundary
is where the bugs live: give explicit *must-NOT-match* / *must-stay-hermetic*
examples and require tests for them, not only the positive cases. The executor
implements the spec literally, so an under-specified boundary leaks straight
through. (M2 calibration: two bounces traced to positive-only specs — bug-04-1, an
escape test whose scope root *was* the `TempDir`, so "outside the root" wrote
outside the sandbox; and bug-05-1, a classifier that matched `shutdown` as a bare
substring and so blocked `cargo test shutdown`. Both would have been caught by a
single pinned negative example.)

**Exact-format output needs exact-equality assertions, not substring checks.**
When code emits output whose exact shape is the contract (a markdown table
row, a wire payload, a rendered template line), a `contains(..)` or loose
disjunction assertion is blind to malformed supersets of the expected string —
the test passes on the broken output because the correct fragment is embedded
in it. Spec such tests as **exact equality on the full line/value plus a
pinned must-NOT** (the specific malformation, e.g. `!contains("||")`).
(Folded 2026-07-10: `flip_readme_row`'s five production misfires — bug-03a-1's
duplicated cell and the doubled trailing pipe `| review ||` — all survived a
suite asserting `contains("| review |")`, a substring of every malformed shape
too; M32's exact-equality rewrite made a revert of the fix fail 4 of 6 tests.
Same family: M30 bug-01-1's loose `Cancelled || HardFail` disjunction.)

### Derive intentionally

Before pinning serde derives on a struct, ask whether it actually gets serialized
at runtime. If yes, pin them — they're load-bearing. If no, omit them; an unused
derive can force upstream derive additions on shared types and push the executor
into unauthorized edits of settled phases.

The same applies to **wired-in state, not just derives**: don't have a phase
record into / populate something whose consumer doesn't exist yet. (M4: phase-07a's
spec told the loop to `scorer.record(...)`, but nothing read the score until
phase-08's `tool_success_rate` — a seven-phase stretch of dead, unobservable
computation that the executor rightly flagged each review. Either pin the consumer
in the same phase, or defer the write until the phase that consumes it.)

**Wrap-vs-derive at protocol boundaries.** When exposing a type at a protocol
boundary (MCP tool output, JSONL log line, telemetry record), the boundary trait
(`JsonSchema`, `Serialize`, …) has to apply to *every* type in the schema tree.
Two ways to satisfy that:

- **Derive directly** when the schema tree is small and locally-owned. The
  output type is one struct (or a couple) of primitives the architect controls;
  adding the derive is a one-line edit, no upstream cascade. *(M5: `Health`
  was one struct, `ScorecardRow` was one struct — both derived `JsonSchema`
  directly.)*
- **Wrap in a single-field `serde_json::Value` carrier** when the schema tree
  is large or foreign. The wrapper struct (`ExecutePhaseOutput { result:
  Value }`, `LogQueryOutput { records: Value, truncated: bool }`) derives the
  boundary trait; the inner `Value` carries the pre-serialized payload, so no
  derive has to be added to the foreign types. *(M5: phase-02 wrapped
  `PhaseResult`; phase-03 wrapped `Vec<SessionRecord>` — both have many
  internal types from another crate.)*

Cost trade-off: wrapping adds one nesting layer in the JSON output (`{
"result": {...} }` vs `{...}`); deriving forces the boundary trait on every
type in the tree (and risks cascading derive additions onto settled types in
other crates, which is exactly the trap M4 phase-03 hit with `Deserialize` on
the parser types). Choose at draft time per type, not at code time.

### Anticipate cross-boundary trait bounds

When a phase introduces a new protocol or async boundary (MCP tool, async
runtime, JSONL persistence), **enumerate in the spec the trait bounds the
boundary will require** — `Serialize`, `Deserialize`, `Send`, `Sync`,
`JsonSchema`, etc. — and check at draft time whether the types crossing the
boundary already satisfy them. If they don't, the spec either authorizes the
narrow upstream edit to add the bound, or pins the wrapper pattern (see
"Wrap-vs-derive" above) to sidestep it.

The cost of missing this at draft time is repeating one of two failure
modes: (1) the executor discovers the missing bound mid-phase, files a
blocker, and waits for architect authorization to edit an upstream crate
(M4 phase-03: `Deserialize` on the M3 parser types); or (2) the executor
adds the bound without authorization and the architect catches it at review
as a scope deviation (M5 phase-02: `Send + Sync` on `LoopDeps.clock`,
`JsonSchema` on `Health`). Both end in the right place, but both cost a
round trip.

Recurrences before fold: M4 phase-03 (`Deserialize` on parser types,
architect-resolved blocker); M5 phase-02 (`Send + Sync` on the clock +
`JsonSchema` on `Health`, declared deviations); M5 phase-03 (`JsonSchema`
cascade across SessionEvent — sidestepped via `Value` wrap, planned in
spec); M5 phase-04 (`JsonSchema` on `ScorecardRow`, planned in spec); M5
phase-05a (`Send + Sync` on `ProgressCallback`, planned in spec). Five
occurrences — the rule is well-established now; subsequent phases should
catch it at draft time, not review time.

### Verify external APIs against live docs

When a phase references an external API the architect cannot live-verify
(an SDK's macro names, a protocol's wire format, a CLI's config schema, a
plugin manifest shape, a third-party crate's surface), the spec MUST
include a **Pre-flight step** instructing the executor to verify the
specifics against the live documentation and **trust the docs over the
architect's sketch**.

The architect's reference sketch in such specs is the *intent* and
*behavior* the phase pins; the *exact* field names, macro forms, file
paths, and frontmatter shapes are the executor's to discover and adapt.
Any divergence between sketch and live docs the executor cannot resolve
from the phase doc is surfaced as a **blocker** (returned to the architect
as a briefing — the executor is headless and cannot ask inline), not a
silent fix during execution. The architect responds with a refined spec or
amendment and re-dispatches. A divergence the executor *can* resolve from
the supplied reference is adapted cleanly and recorded in "Notes for
review" rather than blocked on. **A blocker is cheap; a wrong silent fix is
expensive.**

Pair this with the **declare-deviations** discipline: even when the
executor adapts cleanly to the live docs (the right call), the
adaptation is named in "Notes for review" so the architect can update
their mental model of the API for future specs.

Recurrences before fold: M5 phase-02 (rmcp 1.7 macro names + transport
path divergence from sketch); M5 phase-05b (`#[rmcp::tool]` macro cannot
receive a context arg — forced a manual `ServerHandler` impl); M5
phase-06 (rmcp 1.7 `list_roots` API verification); M6 phase-01 (three
findings: Claude Code's `.mcp.json` has no timeout field at all; modern
plugins use `skills/<name>/SKILL.md` not legacy `commands/`;
`.claude-plugin/plugin.json` manifest is required); M6 phase-04 (three
findings: slash commands are namespaced `/<plugin>:<skill>`; the `model`
field accepts a fixed enum; `allowed-tools` is Claude-native pattern
syntax, not MCP tool names). Eleven findings across six phases — the
pattern repeats across every external-API integration. Folding the
discipline so it does not depend on individual executor judgment or
individual architect spec-writing memory.

The **Pre-flight step's shape**:

> N. **Verify the current `<external API>` <thing>** before coding. The
>    architect cannot reliably enumerate the exact `<field/macro/path/
>    shape>` and the sketch in § X below may be wrong. Sources to consult,
>    in priority order: the official docs site; the upstream source / tool
>    introspection (`cargo doc`, `--help`, etc.); working examples from
>    other consumers. **Trust the docs over the sketch.** Pin the
>    *behavior* this phase requires; let the executor adapt the
>    *structure* to the real convention. Flag any divergence in "Notes
>    for review".

### Prefer additive change shapes; avoid wide-blast-radius breaking changes

When a phase requires modifying a type used at many call sites (an enum variant,
a function signature, a trait method), the architect must choose whether the spec
asks the executor to **mutate** the existing symbol or **add** a new one.

**Mutation is high-risk** when the type has many call sites: every site stops
compiling the moment the definition changes, the executor must update all of them
before the build is green again, and the verifier's consecutive-failure limit (3
strikes) can fire before the cascade completes — leaving the codebase in a
broken-in-progress state. The more call sites, the narrower the window.

**Additive shapes sidestep this entirely.** A new enum variant, a new struct field
with `#[serde(default)]`, a new function that takes the role of the old one — these
keep the codebase compiling at every step. Only the *new* code needs updating; the
old code keeps working until it is deliberately migrated.

**At draft time, before speccing a multi-site mutation, ask:**
- Is there an additive shape that achieves the same behavioral goal?
  - Add a *sibling* variant instead of changing the existing one?
  - Add a *new* field with `#[serde(default)]` instead of widening an existing
    field's type?
  - Add a *new* function and migrate callers one-by-one instead of changing the
    signature of the current one?
- If mutation is unavoidable, can the blast radius be bounded to ≤ 3 sites (within
  the verifier's retry budget)?

If yes to either, use the additive shape and pre-inject it. If the blast radius
exceeds ~3 sites and no additive alternative exists, flag it explicitly in the phase
doc and instruct the executor to `cargo build` after **each individual site** before
moving to the next.

**What to pre-inject when a multi-site change is unavoidable:**
Give the executor a `grep`-verified complete list of every site, in the order to
update them, with a "build after this site" instruction after any site that would
break a separate file. An incomplete list is how this class of failure happens — the
executor changes the definition and runs out of runway.

Recurrences before fold: M7 phase-05a (changing `build_chat_body`/`OpenAiClient::new`
signatures without updating all callers — `E0061` on 2 sites); M7 phase-05b (mutating
`AiEvent::Done` from a tuple variant to a struct variant — `E0164`/`E0533` on 2 sites
before the 3-strike verifier limit). Both resolved by additive restructure (phase-05a:
a worked-example cascade in Task 5; phase-05b: new sibling `AiEvent::Completion`
variant leaving `Done` untouched).

**When the cascade is truly unavoidable, pre-inject a topological (leaf-first)
edit order.** Some changes have no additive shape — a required (non-defaultable)
field on a widely-constructed type, or a trait derive whose `#[derive]` on a
container fails until every nested field type also carries it. For these, the
spec must give the **exact edit order in which every intermediate step
compiles**: dependencies (leaf types, callee signatures) first, dependents
(containers, callers) last, with an explicit "run the build now, it must be
green" checkpoint at each file boundary. An unordered cascade leaves the crate
non-compiling for many consecutive turns and the verifier's strike limit fires
mid-cascade regardless of how correct the individual edits are.
(Folded 2026-07-10 after two occurrences with the countermeasure proven both
times: M30 phase-03 — a required `cancel` field across ~14 sites, 2 hard_fails
→ session takeover; M31 phase-02 — a `JsonSchema` derive graph applied
top-down, hard_fail at 6 strikes, then a refined re-dispatch pinning the
leaf-first order landed clean first-try.)

### Post-write formatting is a runtime concern, not a spec concern

When a formatter (`ruff format`, `gofmt`, `rustfmt`, etc.) is part of the
project's command set, a recurring class of verifier hard-fail arises: the
executor runs the formatter during its turn loop, then issues a subsequent
`write_file` that overwrites the formatted file with unformatted content.
The verifier fires on the unformatted file, produces 3 consecutive failures,
and halts with a hard_fail.

**Root cause:** The executor's tool-call loop is not atomic with respect to
formatting. Any `write_file` issued *after* the format step undoes it.
The executor is not buggy — it formatted correctly; it simply continued
working and overwrote the result.

**What does not work:** Spec-level "Completion checklist" instructions to
run the formatter before `git add`. M1/phase-03 of mp3-player pre-injected
this instruction explicitly; the executor ran it, then issued another
`write_file` afterward. A spec instruction cannot prevent a later write.

**The fix is runtime-level — and it has since landed:** the rexyMCP runtime
runs a **post-write, pre-verifier hook** (`run_post_write_hooks`) after every
turn that wrote files, before the verifier. The hook invokes the project's
`[commands] format_fix` and `lint_fix` — the **writing** forms of the
formatter/linter (e.g. `cargo fmt --all`, `ruff format`, `gofmt -w`),
**distinct from** the verify-only `format`/`lint` **gate** commands. This makes
formatting unconditional and turn-ordering-independent. The hook is **inert
unless `format_fix`/`lint_fix` are configured**: with them unset, the
`format`/`lint` gates stay verify-only and no auto-formatting happens (that is
the historical "hook is a no-op" state — a config gap, not a code gap).

**For the architect:** Do not add "run the formatter" steps to completion
checklists in phase specs — proven ineffective for this failure class (a later
`write_file` still undoes it). Instead, ensure `[commands] format_fix` /
`lint_fix` are set (the writing forms) in the target project's config so the
hook auto-formats each turn; the `rexymcp init` template scaffolds both as
commented lines. If they are unset, apply the formatting fix manually on
close-out.

*(Folded from M1/mp3-player: four phases (01×2, 02, 03) on
google/gemma-4-12b hit the same ruff formatting verifier halt. Spec
instruction pre-injected in phase-03 — still failed, confirming the fix must be
runtime-side. The runtime hook has since landed: `run_post_write_hooks` runs
`[commands] format_fix`/`lint_fix` post-write, pre-verifier.)*

### Validation features depend on the target toolchain — verify availability at design time

rexyMCP's validation features shell out to **per-language toolchains** the
executor host must actually have. They split into two tiers, and the tiers
answer "fail open or fail hard?" differently:

- **Tier 0 — the `[commands]` toolchain** (`format`/`build`/`lint`/`test`, e.g.
  `cargo …`, `zig build`, `go test`). Language-agnostic, user-configured in
  `rexymcp.toml`, and **already a hard requirement**: a phase cannot reach `done`
  without `build`/`test` passing (STANDARDS §1). **This is how rexyMCP supports
  *any* language**, including ones with no built-in verifier — you point
  `[commands]` at the language's tools and the loop + DoD gates work.
- **Tier 1 — validation *enhancers*** (the verifier's `cargo check`/`tsc`/`ruff`,
  and code-intelligence features like find-references or compiler-suggested-fixes).
  These are language-specific and **augment** Tier 0 with incremental, structured
  feedback. The loop **degrades gracefully** to Tier-0-only without them. Note:
  enhancers backed by *compiled-in crates* (tree-sitter grammars) need **no**
  machine install — only enhancers that **shell out to a binary** (`cargo`, `tsc`,
  `ruff`, a future `rust-analyzer`) are a runtime-availability concern.

**Fail-open at runtime; fail-hard-*advisory* where a human is present.** The
deciding axis is *who can act on a missing tool, and when*:

- **At the human-present boundary** (first `/architect` / bootstrap): detect
  missing toolchain binaries and **present a resolution plan** — install
  instructions, or scope the feature to the languages whose toolchain is
  confirmed present and defer the rest. The user chooses; this is advisory, not a
  refusal to continue.
- **At runtime inside the headless loop**: a missing binary must **degrade to a
  model-visible advisory that names the binary and the remedy** ("rust-analyzer
  not found on PATH; find-references unavailable") and let the executor keep
  working with its other tools — never a panic, never an opaque "spawn failed",
  and never an outcome the governor counts as a verifier *failure strike* (a
  missing tool is a `Skipped`/advisory outcome, distinct from "the tool ran and
  found errors").

**When drafting a phase that adds or extends a validation feature, the architect
must:** (1) enumerate the runtime binaries it invokes (name + minimum version +
the exact flags / machine-readable format it parses), distinguishing compiled-in
crates from machine binaries; (2) confirm they are present and emit that format —
or instruct a Pre-flight check; (3) if a binary is missing for a target language,
inform the user with a resolution plan before shipping a feature that would only
degrade; (4) pin the missing-binary runtime behavior in the phase doc as a named
advisory, per the rule above. Record the feature's toolchain dependencies in the
phase doc (Pre-flight or a "Toolchain dependencies" line).

*(Folded 2026-06-09, user-initiated at the M12 — Executor Tooling kickoff, when
validation features — find-references, compiler suggested-fixes, structured
test-failure parsing — entered the roadmap. Not a bounce-driven fold; a
forward-looking discipline for a new feature class.)*
