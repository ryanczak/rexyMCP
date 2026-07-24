# Development Workflow

How a project is built under architect-driven development: who does what, what a
phase looks like, and how work moves from "planned" to "merged."

## Roles

**Principal engineer / architect.** Owns the architecture, breaks design into
phases, reviews completed phases, writes bug reports, decides scope changes. Does
not normally write implementation code — that's the executor's job.

**Executor.** Implements one phase at a time following the phase doc. Reads
`STANDARDS.md` at the start of every phase. Reports blockers when stuck. Never
invents scope. Never edits files outside the phase's authorization.

**Human (project owner).** Decides direction, vetoes architectural choices, runs
the show. Both the architect and the executor work for the human.

---

## Hierarchy

```
Milestone           — a coherent capability (M1 Foundations, M2 Tools, ...)
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
 ├── NEXT.md                           pointer to the active phase; executor reads first
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

 `NEXT.md` is maintained by the architect and tells the executor which phase to
 work on next. At a milestone boundary it says "none", signaling the human gate.
 The executor reads it before every session to locate the active phase doc.

 Phases are numbered in execution order. Phases that can run in parallel share a
 parent number with letter suffix (`phase-03a-x.md`, `phase-03b-y.md`).

---

## Milestones

Milestones come from the project plan. Each entry becomes a milestone with its
own `M<n>-<slug>/` directory. The architect expands a milestone into phases
**on demand, not all at once**, because earlier phases reveal information that
shapes later ones.

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
unless the spec is explicitly prescriptive.

The `Tags:` frontmatter line categorizes the phase (language, kind, size) so
metrics can be aggregated. The architect sets it when drafting; keep the
vocabulary consistent across phases.

### Phase doc template

```markdown
# Phase <n>: <Title>

**Milestone:** M<n> — <name>
**Status:** todo | in-progress | blocked | review | done
**Depends on:** phase-<m> (or "none")
**Estimated diff:** ~<n> lines
**Tags:** language=<rust|go|python|ts|...>, kind=<feature|refactor|bugfix|test>, size=<s|m|l>

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
ids. The section ends at the next `## ` heading (two hashes + space). A decimal
like `### 1.5x` is deliberately **not** seeded (it is not a task).

1. **<Task name>** — in `<path>`, <change>. <Why if non-obvious.>

## Acceptance criteria

Verifiable conditions — each one checkable by running a command or reading a file.

- [ ] `<command>` produces `<expected output>`.
- [ ] Test `<test_name>` passes.

## Test plan

Concrete tests to write — names + what they assert. Typically unit tests against
hermetic fakes (temp directory, mocked AI client, fixture replay).

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

- [ ] May add dependencies: `<dependency-name>`.
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
   when something went wrong. This is the supervision label for model evaluation
   *and* the substrate for human project review. One write, two consumers.

### Review verdict

Append to the approved phase's Update Log:

```markdown
### Review verdict — YYYY-MM-DD

- **Verdict:** approved_first_try | approved_after_N | rejected | escalated
- **Bounces:** <count> (bugs: <id(s)> — <max severity>, or "none")
- **Executor:** <model name>
- **Scope deviations:** <what the phase cut/deferred vs. its spec, or "none">
- **Calibration:** <fold filed / lesson, or "none">
```

Keep it terse — it's a label, not a narrative. The milestone retrospective rolls
these up at close.

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
and **stops**. The user advances explicitly. The architect does not draft or
dispatch the next phase on its own. This keeps the review a real gate and the
human in control of scope.

**Milestone boundaries are always a human gate.** When a milestone's in-scope
phases are all `done`, the review skill approves the final phase as normal and
stops — it does **not** write the retrospective or close the milestone. Milestone
close is a separate explicit step: the human invokes `/rexymcp:architect` to write
the milestone-specific retrospective, fold calibration lessons into `WORKFLOW.md`
(with sign-off), and update `NEXT.md` to "none". This is where direction changes
happen; it is never automated by the review step.

**Opt-in autonomous loop (off by default).** For hands-off runs, the user may
start an explicit `/rexymcp:auto` run that chains draft -> dispatch -> review ->
escalate/re-dispatch across phases with **full review rigor and no per-phase
pause** — the review procedure runs verbatim (independent gate re-runs, DoD walk,
telemetry verdict, commit); only the human pause between steps is removed. It is
explicitly enabled per run, never the default, and it **composes** the
interactive skills rather than forking them — a behavior difference between an
interactive and an autonomous run of the same step is a bug. Dispatch drives
`execute_phase`'s **async contract** — it polls `get_run_status` to reap each
spawned run — and a running phase is **interruptible** out-of-band (`rexymcp stop`
for the human, `stop_phase` for the architect between polls), which the loop
treats as a deliberate human signal. The loop stops for the human on: a milestone
boundary (always), any blocker or "What Executors Never Decide" item, exhaustion
of the per-phase assist budget (`[escalation] max_assists` autonomous escalation
round-trips on one phase), the loop-level runaway backstop, or a phase returning
**`cancelled`** (a deliberate `rexymcp stop` / `stop_phase` interrupt — the loop
surfaces the partial work and hands back). Every stop produces a **loop report** — phases run, verdicts,
assists spent, token/cost totals where harvested, and why it stopped — so the
human resumes from a briefing, not a scrollback dig. Every architect activity in
the loop is journaled to the telemetry store; token usage is harvested from the
client's own transcripts where available and recorded as absent elsewhere, never
estimated.

**The executor is a local LLM, not a coding agent.** The model driving phases
through this workflow is a single-purpose executor: it has the project's tool set,
the embedded contract + STANDARDS + the phase doc, and a bounded turn budget. It
does *not* have web access, cannot escalate mid-phase to a stronger model, and
does not negotiate scope. Treat its outputs as the work of a junior engineer who
cannot ask clarifying questions. Mismatched-expectations bugs are *spec bugs*, not
executor bugs.

**Front-load by task shape, not by default.** Whether to pre-inject — and how much
— depends on the kind of work:

- **Design-discovery phases** (the executor must find a load-bearing API or
  architecture constraint the spec does not fully determine): front-load the key
  constraint — the load-bearing seam, the critical API call, a worked example of
  the exact pattern to follow. One focused paragraph beats an exhaustive wall of
  context.
- **Mechanical phases** (move/rename/extract whose shape the spec fully
  determines): normal density; no front-loading needed.

**Lean bias: prefer under-specification over over-specification.** The architect
runs on a cloud model (Claude); the executor runs locally. Every extra token in the
spec costs cloud budget. A bounce from the local executor is cheaper than an
over-specified spec written by Claude. Front-load just enough to prevent the
predictable bounce — not everything you could say.

---

## Governing a running phase — the governor terminates, not the architect

Once a phase is dispatched, **the executor's governor is the authority that ends
the run.** Its terminators — the no-progress stall, the oscillation and
identical-repetition detectors, `max_turns`, and `wall_clock_secs` — are the
load-bearing boundary of the executor loop. A run that looks slow, stuck, or is
grinding through many turns is the governor's call, not the architect's. Letting it
run is *how* a real stall becomes a `hard_fail` + briefing (the input to
`/rexymcp:escalate`) and *how* the stall detectors accumulate the data that
calibrates them; pre-empting the governor destroys both.

**When the architect may cancel a run (`stop_phase`)** — only for one of these
three enumerated reasons, **never** because a run "looks slow" or "looks stuck":

1. **Explicit human instruction** to stop.
2. **A clearly mis-dispatched run** — wrong phase, wrong repo, or wrong config.
3. **A confirmed infrastructure fault the governor cannot see** (e.g. the endpoint
   died) — not a slow or long-running generation.

A long generation, a repeated tool-call fumble, a frozen diff — all are handled by
*waiting for the governor*. If the run exposes a spec-shape problem (a too-large
edit, a missing worked example), the fix is the **next** dispatch's spec — a refined
re-dispatch via `/rexymcp:escalate` — not killing the current run. The human's
`rexymcp stop` is always available as a deliberate signal; the architect's
`stop_phase` is bound by the list above.

**Monitoring an in-flight run — hand off, don't hover.** Claude Code sends no MCP
progressToken, so the human's `rexymcp status` / `rexymcp dashboard` is the live
view. In an **interactive** dispatch, the architect confirms the run started, then
**stops active polling** and hands off — the human watches and signals when to reap,
or the architect reaps the terminal result on its next turn. A continuous
`get_run_status` poll loop, with turn-by-turn narration or repeated session-log
`grep`/`tail`, is a large and avoidable Claude-token cost (each poll re-reads the
whole context) that buys nothing the dashboard doesn't already show. The
**autonomous** `/rexymcp:auto` loop has no human to hand off to, so it reaps by
polling — but minimally, without narration, and it never cancels for slow/stuck
either.

---

## What Executors Never Decide

- Whether something belongs in core vs. a plugin.
- Whether to add a dependency.
- Whether to change the architecture doc.
- Whether to skip a test, mark it as ignored, or suppress a warning.
- Whether to widen a phase's scope to fix a related issue noticed in passing.
- Whether to deviate from STANDARDS.md "because this case is special."

All of these are blockers. File them in the Update Log and stop.

---

## Calibration — fold lessons in

The workflow this document describes is the product's own workflow, and the plugin
embeds these files verbatim. So **everything learned building a project must be
folded into these docs** — there is no separate place for "lessons learned for
later."

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
string-matching, path resolution, or escape semantics, the boundary is where the
bugs live: give explicit *must-NOT-match* / *must-stay-hermetic* examples and
require tests for them, not only the positive cases. The executor implements the
spec literally, so an under-specified boundary leaks straight through. (An early
milestone's bounce traced to a positive-only spec — an escape test whose scope
root *was* the temp directory, so "outside the root" wrote outside the sandbox;
and a classifier that matched a shutdown keyword as a bare substring and so
blocked an unrelated command containing that substring. Both would have been
caught by a single pinned negative example.)

**Exact-format output needs exact-equality assertions, not substring checks.**
When code emits output whose exact shape is the contract (a markdown table
row, a wire payload, a rendered template line), a `contains(..)` or loose
disjunction assertion is blind to malformed supersets of the expected string —
the test passes on the broken output because the correct fragment is embedded
in it. Spec such tests as **exact equality on the full line/value plus a
pinned must-NOT** (the specific malformation, e.g. a doubled delimiter).
*(Folded after a formatter bug shipped five production misfires under a suite
whose substring assertions matched every malformed shape; the exact-equality
rewrite made a revert of the fix fail 4 of 6 tests.)*

### Pin the fixture that makes the row appear

When a test's assertion depends on **rendered output being present**, pin the
exact fixture that produces it. Renderers routinely hide rows that are empty in
every scope; under a fixture that leaves the row empty, the row never renders,
the test fails with "row missing" — and the executor reads that as a *production*
bug it must diagnose. It then re-reads the renderer in a loop looking for a
defect that isn't there, until a governor terminator ends the run.

The spec's job is to remove the ambiguity up front: name the fixture values that
make the row appear, and say why (e.g. "use a **priced** rates fixture — the
`$0.00` debit row is hidden by the all-empty rule, so an unpriced fixture makes
the assertion unsatisfiable").

*(M35 07e: the executor's own new test used an unpriced fixture, so the Executor
debit row was hidden; the run hard-failed on a read/test oscillation while
diagnosing it. A resume carrying the one-line priced-fixture hint landed clean in
19 turns — the production code had been correct the whole time.)*

### Pre-inject compiler-error-driven recovery on oscillation-prone files

When a phase touches a file with a **history of oscillation hard-fails**, state
the recovery discipline explicitly in the spec: *use the compiler error to locate
a syntax problem; never hunt for it by re-reading the file in a loop.* Pair it
with an exact code block for any structural edit, rather than a prose description
the executor must reconstruct by reading.

*(M35: proven on 07f — a `render.rs` restructure landed with no oscillation on a
file that had oscillated 3× earlier in the same milestone. The runtime-level fix
for the underlying terminator behavior is M37's read-only exemption; this
discipline is what the architect controls in the meantime.)*

### Derive every spec fact from its source

A phase doc is full of assertions of fact: a `file:line`, a CLI flag, a list of
call sites, a corpus measurement, a condition the executor must satisfy. Every
one of them is a claim the executor **cannot check and will implement
literally**.

**Before dispatch, derive each such fact by running the tool that defines it** —
`grep` the sites, `--help` the flag, re-read the line numbers, recompute the
figure through the same code path the product uses. Never restate one from
memory, and never carry one forward from an earlier draft: both drift, and a
draft that was correct when written is not correct after the phase before it
lands.

The failure is silent by construction — nothing in the toolchain checks a phase
doc's prose against the code. Severity scales badly:

- A wrong line number costs the executor a search.
- A wrong flag costs a round trip (or a declared deviation, if the executor is
  disciplined enough to catch it).
- **An acceptance criterion that contradicts its own Spec, or a verification
  that is arithmetically unsatisfiable, cannot be met at all.** The executor
  either bounces on the architect's error or adapts and gets recorded as
  deviating.

The same discipline applies to bug docs and re-dispatch notes, which are specs
too: a worked "here is the exact replacement code" block that calls a function
the last phase inlined is worse than no worked example, because it is trusted.

*(Folded 2026-07-24 after **ten** occurrences across M36–M38, only two caught
before dispatch: a corpus figure quoted pre-dedup (59.6M asserted vs 36.1M
actual); a file list written from memory that missed `main.rs`; a design
requirement dropped between conversation and spec; an acceptance criterion
demanding zero matches while its own task required a fixture keeping them; an
E2E block using `init --config` when the flag is `--dir`; drifted line numbers;
a rename list naming three sites when the phase invalidated six; a bug doc's
worked fix citing `align_value` after a restructure had inlined it; a
verification demanding a cross-field column equality that the layout makes
unsatisfiable; and a status edit replacing strings the executor's bookkeeping
had already rewritten. The executor caught three of the ten and adapted
correctly, declaring each — which is the declare-deviations discipline working,
not a substitute for the architect deriving the fact.)*

### Derive intentionally

Before adding protocol-derived traits to a struct, ask whether it actually gets
serialized at runtime. If yes, add them — they're load-bearing. If no, omit them;
an unused derive can force upstream additions on shared types and push the executor
into unauthorized edits of settled phases.

The same applies to **wired-in state, not just derives**: don't have a phase
record into something whose consumer doesn't exist yet. Either pin the consumer
in the same phase, or defer the write until the phase that consumes it.

**Wrap-vs-derive at protocol boundaries.** When exposing a type at a protocol
boundary (tool output, log line, telemetry record), the boundary trait has to
apply to *every* type in the schema tree. Two ways to satisfy that:

- **Derive directly** when the schema tree is small and locally-owned. The
  output type is one struct of primitives the architect controls; adding the
  derive is a one-line edit, no upstream cascade.
- **Wrap in a single-field generic carrier** when the schema tree is large or
  foreign. The wrapper struct derives the boundary trait; the inner carrier
  carries the pre-serialized payload, so no derive has to be added to the foreign
  types.

Cost trade-off: wrapping adds one nesting layer in the output; deriving forces the
boundary trait on every type in the tree. Choose at draft time per type, not at
code time.

### Anticipate cross-boundary trait bounds

When a phase introduces a new protocol or async boundary (tool, async runtime,
persistence), **enumerate in the spec the trait bounds the boundary will require**
and check at draft time whether the types crossing the boundary already satisfy
them. If they don't, the spec either authorizes the narrow upstream edit to add
the bound, or pins the wrapper pattern to sidestep it.

The cost of missing this at draft time is repeating one of two failure modes:
(1) the executor discovers the missing bound mid-phase, files a blocker, and waits
for architect authorization; or (2) the executor adds the bound without
authorization and the architect catches it at review as a scope deviation. Both
end in the right place, but both cost a round trip.

### Verify external APIs against live docs

When a phase references an external API the architect cannot live-verify
(an SDK's macro names, a protocol's wire format, a CLI's config schema, a
plugin manifest shape, a third-party library's surface), the spec MUST
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

The **Pre-flight step's shape**:

> N. **Verify the current `<external API>` <thing>** before coding. The
>    architect cannot reliably enumerate the exact `<field/macro/path/
>    shape>` and the sketch in § X below may be wrong. Sources to consult,
>    in priority order: the official docs site; the upstream source / tool
>    introspection; working examples from other consumers. **Trust the
>    docs over the sketch.** Pin the *behavior* this phase requires; let
>    the executor adapt the *structure* to the real convention. Flag any
>    divergence in "Notes for review".

Use this step whenever the phase touches an external library, a third-
party manifest format, a tool's CLI flag set, or any other surface the
architect can't introspect from inside their own session. Skipping it
when it applies is how silent improvisations enter the codebase.

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

*(Folded from M7/phase-05b: two hard_fails of the same class — breaking a
multi-site type change — on two separate phases, Qwen3.6-27B-FP8. Additive
restructure resolved both.)*

**When the cascade is truly unavoidable, pre-inject a topological (leaf-first)
edit order.** Some changes have no additive shape — a required (non-defaultable)
field on a widely-constructed type, or a trait derive whose `#[derive]` on a
container fails until every nested field type also carries it. For these, the
spec must give the **exact edit order in which every intermediate step
compiles**: dependencies (leaf types, callee signatures) first, dependents
(containers, callers) last, with an explicit "run the build now, it must be
green" checkpoint at each file boundary. An unordered cascade leaves the
project non-compiling for many consecutive turns and the verifier's strike
limit fires mid-cascade regardless of how correct the individual edits are.
*(Folded after two occurrences with the countermeasure proven both times: an
unordered required-field cascade struck out and needed a takeover; an
unordered derive-graph cascade struck out, then a refined re-dispatch pinning
the leaf-first order landed clean first-try.)*

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

Validation features (a verifier that runs the project's checker, code-intelligence
features like find-references or compiler-suggested-fixes) shell out to
**per-language toolchains** the executor host must actually have. They split into
two tiers, and the tiers answer "fail open or fail hard?" differently:

- **Tier 0 — the `{BUILD_COMMAND}` / `{TEST_COMMAND}` / `{LINT_COMMAND}` /
  `{FORMAT_COMMAND}` toolchain.** Language-agnostic, user-configured, and
  **already a hard requirement**: a phase cannot reach `done` without build/test
  passing (STANDARDS §1). **This is how the project supports *any* language** —
  point the command set at the language's tools and the loop + DoD gates work,
  even for a language with no dedicated verifier.
- **Tier 1 — validation *enhancers*** that **augment** Tier 0 with incremental,
  structured feedback. The loop **degrades gracefully** to Tier-0-only without
  them. Enhancers backed by a *compiled-in library* (e.g. a bundled parser grammar)
  need **no** machine install; only enhancers that **shell out to a binary** are a
  runtime-availability concern.

**Fail-open at runtime; fail-hard-*advisory* where a human is present.** The
deciding axis is *who can act on a missing tool, and when*:

- **At the human-present boundary** (project bootstrap / first design session):
  detect missing toolchain binaries and **present a resolution plan** — install
  instructions, or scope the feature to the languages whose toolchain is confirmed
  present and defer the rest. The user chooses; advisory, not a refusal.
- **At runtime inside the headless executor**: a missing binary must **degrade to
  a model-visible advisory that names the binary and the remedy** and let the
  executor keep working — never a panic, never an opaque "spawn failed", and never
  an outcome the verifier governor counts as a *failure strike* (a missing tool is
  a skipped/advisory outcome, distinct from "the tool ran and found errors").

**When drafting a phase that adds or extends a validation feature, the architect
must:** (1) enumerate the runtime binaries it invokes (name + minimum version +
the exact flags / machine-readable format it parses), distinguishing compiled-in
libraries from machine binaries; (2) confirm they are present and emit that format
— or instruct a Pre-flight check; (3) if a binary is missing for a target
language, inform the user with a resolution plan before shipping a feature that
would only degrade; (4) pin the missing-binary runtime behavior in the phase doc
as a named advisory, per the rule above. Record the feature's toolchain
dependencies in the phase doc (Pre-flight or a "Toolchain dependencies" line).
