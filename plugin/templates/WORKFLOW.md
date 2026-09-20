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

**Pre-dispatch check.** With the finished Spec in front of you, before
dispatch:

1. **Run the whole criteria block against the current tree** and require
   every line that is not a labelled *preservation criterion* to fail. A line
   that already passes certifies nothing.
2. **Re-derive every pinned count after accounting for the artifacts this
   Spec's own tasks create** — the test that must name the retired symbol, the
   `use` line that imports the new function, the fixture that adds a row. A
   count measured against the pre-phase tree is one the executor cannot reach.
3. **Any self-check verdict (`PASTE MATCH`, `GUARD OK`, …) is a numbered
   criterion here, not Task prose.** A verdict that lives only in a task can be
   omitted with every task complete, and the check silently moves to review.
4. **A criterion pinning an *absence* pins behaviour, not file text.**
   `grep -c 'X' <file>` → 0 also counts the negative test that must name `X`;
   pin the function's output or a mutation that must turn a named test red.

These are triggers for rules this document already carries (§ "Every
acceptance criterion must be satisfiable", § "Run every count criterion",
§ "Specs pin behavior, not rendering"); they sit here because the rules kept
being missed at drafting time, not at review.

## Test plan

Concrete tests to write — names + what they assert. Typically unit tests against
hermetic fakes (temp directory, mocked AI client, fixture replay).

- `test_<name>` in `<path>` — asserts <behavior>.

## End-to-end verification

Unit tests with hermetic fakes can pass while the real artifact the phase ships
is broken. For every acceptance criterion that references a real artifact (a
checked-in file, a CLI behavior, a binary entrypoint, a config the running binary
loads), verify against that real artifact before reporting complete.

**Capture mechanically, never by hand.** Redirect each command's output to a
file and paste that file's contents. Do not retype a transcript, reconstruct
one from memory, summarise results into prose, or copy lines from the phase doc
or a previous Update Log entry.

**The evidence goes in its own Update Log entry** titled
`### Update — <date> (end-to-end verification)`. The server-authored
`(complete)` entry never satisfies this — its command tails prove the gates
ran, not that the acceptance criteria were exercised against real artifacts.
**One entry per dispatch:** a bounced, re-dispatched phase needs a new entry
for the round that changed the code; an earlier round's entry describes a tree
that no longer exists.

<The architect writes the exact commands here as one runnable block, and seeds
the capture as the phase's final `## Spec` task — see WORKFLOW.md § "The E2E
block: runnable, complete, and seeded as a Spec task".>

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
   table) or **rejects** (runs the full bounce sequence below — writing the bug
   report is only its first step).
5. **Records a structured review verdict** (below) — at every approval, not just
   when something went wrong. This is the supervision label for model evaluation
   *and* the substrate for human project review. One write, two consumers.

### The bounce sequence — four steps, in order, none optional

A rejection is not "file a bug and flip the status." It is these four, and the
third is the one that determines whether the re-dispatch does anything:

1. **Write the bug report** in the milestone's `bugs/` directory.
2. **Flip the phase doc's `Status:`** back to `in-progress`, naming the bug.
3. **Refresh the phase doc's acceptance criteria** so the outstanding work is
   expressed *there*, and **run each new criterion to confirm it fails against
   the current tree**. Any count the fix will change is re-pinned to its new
   exact value ("more than N" is satisfied by what is already on disk;
   "exactly N+1" is not). **When the finding is an evidence artifact, pin the
   artifact where it lives** — the fenced transcript, scoped to its entry —
   not the prose describing it: a claim in prose *and* in a fence is two
   artifacts, and a grep for the prose polices one.
4. **Update the milestone README row** and record the bounce in telemetry.

**Step 3 is the load-bearing one, and it is the one that gets skipped.** The
executor evaluates the *phase doc* to decide whether there is work to do; it
does not evaluate the bug doc for that purpose. After a rejection, the phase
doc's criteria describe the tree the executor already built — so they all
pass, the doc certifies itself as finished, and the honest reading of it is
"complete, nothing to do." The bug report is where the diagnosis lives; the
acceptance criteria are where *doneness* lives. A bug doc is a supplement,
never a substitute.

*(Folded 2026-08-09 from DaemonEye, a downstream rexyMCP project. Two
empty-diff re-dispatches traced to stale criteria, including one where a loud
bug-doc header, a do-not-touch list, and enumerated edits were all present and
still insufficient: that round returned `complete` with an empty diff in 31
turns; the next round, with four criteria confirmed failing, did the work in
82. The executor's report was honest each time; the spec lied.)*

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

## Root cause
<Why it happens — the mechanism, not the patch. Cite file:line.>

## Definition of done
- [ ] <command produces expected output — run it and confirm it FAILS against
      the current tree before dispatching>
- [ ] <test_name passes>
```

A `## How to fix` section is **optional, and admissible only when the architect
has actually run the fix** — see § "State the symptom, the root cause and the
DoD — not the fix".

### Severity meanings

- **blocker** — phase cannot be merged in this state.
- **major** — must fix before done; correctness or contract violation.
- **minor** — should fix; style, naming, a missing-but-not-critical test.
- **nit** — optional preference; executor may decline with reasoning.

### State the symptom, the root cause and the DoD — not the fix

**The three required bug-report sections are What's wrong, Root cause, and
Definition of done. A `How to fix` section is optional, and admissible only
when the architect has actually run the fix.** Otherwise, describe the
constraint the solution has to satisfy and let the executor choose the edit.

This inverts the older instinct — prescribe the patch and lean on the executor
to type it — because prescription is where architects are least reliable: a
prescribed fix is a *system fact*, a claim that this edit applied to this tree
produces that result, and it is authored from reasoning about code rather than
from running it. The failure mode is specific: **a prescribed fix is trusted
precisely because it is specific.** A vague instruction gets sanity-checked
against the code; a confident code block gets typed in. The more precise the
prescription, the more damage a wrong one does — and precision is not evidence
of correctness when the author never executed it.

What the executor is reliably good at, given a correct root cause, is finding
the edit. It has the compiler, the linter, the test suite and the actual tree;
the architect has none of those at spec-writing time. Give it the diagnosis and
the finish line.

**When you do include a worked example, it must be quoted from code that
exists** (per § "Derive every spec fact from its source"). Quoting an existing
pattern is evidence; authoring a new one is a guess wearing the costume of
evidence — and the executor cannot tell them apart.

*(Folded 2026-08-09 from DaemonEye, after four wrong prescribed fixes in one
milestone: two the executor implemented faithfully and the phase bounced; one
it correctly refused and the finding was withdrawn; one was an impossible
instruction that burned an entire dispatch before the governor stopped it. In
every case the executor's behavior was correct given what it was told.)*

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

**Prescribed code must pass the project's lint gate, not just the compiler.**
A worked example is a spec fact the executor copies verbatim, so it inherits
every gate the project runs. Before speccing a replacement block, put it in a
scratch tree and run the lint command on it; "it compiles" is not the gate.

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

### A struct field's blast radius is every construction site — Authorizations must name them all

When a spec adds a field to a struct, every `StructName { … }` literal in the
crate must change or the build breaks — including the ones in other files'
test modules. Enumerate them at draft time (`grep -rn 'StructName {' src/`)
and list **every file that holds one** in § Authorizations. A spec that says
"add the field at every construction site" while its Authorizations forbid
touching a file that holds three of them hands the executor a choice between
a compile error and an authorization breach. The same enumeration applies to
a *semantic* change that forces test literals elsewhere (a default that now
reports a different state, say).

*(Folded 2026-09-20 from a downstream project, three occurrences.)*

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

**Split delete-heavy rewrites away from additive work.** A phase that both adds
a new module *and* rips out an old path is where executors thrash — reverting
their own uncommitted work or looping on read-only checks. Split it: an
additive phase (new code, nothing deleted) followed by a rewire phase (delete +
wire). The additive half does not trigger the pathology, and if the rewire half
fails, the additive code is already safely on disk. *(Proven twice downstream:
both times the split contained the blast radius — the additive files survived
and only the delete-heavy file needed reconstruction.)*

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

### Give the executor a condition it can check, not an instruction it can agree with

When a requirement keeps going unmet, the reflex is to state it more clearly.
That reflex is wrong often enough to be worth naming: if the executor can
satisfy a phase's tracked work and still miss the requirement, no amount of
rewording closes the gap, because nothing in its own loop ever evaluates the
requirement. **Change what it can check, not how you said it.**

Two shapes that work, both proven:

- **Make it a tracked task.** Only `## Spec` is seeded into the executor's
  task list (`executor/src/agent/tasks.rs` matches a heading of exactly
  `## Spec` and parses that section alone). A requirement stated in any other
  section is invisible to the "have I finished?" check the executor actually
  runs, so it finishes every task it is tracking and reports complete in good
  faith. Move the requirement into `## Spec` as a numbered task.
- **Give it a self-check with a falsifiable output.** A command the executor
  runs against its own work that prints `PASS`/`FAIL` — extract the pasted
  transcript and diff it against the artifact; grep the mutated line to prove
  the mutation applied; state the exact test count the run must report. Then
  make that output an acceptance criterion.

And **run the check against a known-bad input before you spec it.** A detector
that cannot fail is worth exactly as much as a mutation that cannot fail.

*(Folded 2026-08-09 from DaemonEye — two occurrences, and what makes them
worth generalising is the controlled contrast with what failed first. A
missing end-to-end entry drew three rounds of increasingly specific prose
about how to write the block, and stayed missing; making the capture a
`## Spec` task fixed it immediately, both times it was used. A retyped
transcript drew a remedy aimed at artifact size — 2,555 lines shrunk to 56 so
it could be pasted whole — and it was retyped from memory anyway; a
`PASTE MATCH` self-check fixed it on the first try, byte-identical. Both
wording fixes failed; both structural fixes worked first try. The wording was
already good, and the effort spent on it bought nothing.)*

### The E2E block: runnable, complete, and seeded as a Spec task

Three architect rules make the phase-doc template's § "End-to-end
verification" actually produce evidence:

**1. Give the commands as one runnable block, never as prose.** Write the
exact shell the executor should run — output redirected to an artifact file,
exit markers included. Where a result's success case produces *no* output
(a grep that finds nothing, a diff over identical inputs), the exit marker is
the whole proof. **When a command is piped, the marker must record
`${PIPESTATUS[0]}`, never `$?`** — `$?` after `cmd | tail -20` is `tail`'s
exit and is 0 whatever `cmd` did, so the block green-washes every failure the
pipe truncates:

```sh
cmd 2>&1 | tail -20; echo "exit=${PIPESTATUS[0]}"
```

**The artifact must contain only bytes the executor can round-trip.** Raw
ANSI escapes or other control bytes make a byte-exact paste impossible, and
the resulting mismatch is architect-caused. Strip them inside the generator
itself (`cmd 2>&1 | sed 's/\x1b\[[0-9;]*m//g' >> "$A"`), never as a
post-edit. And **everything the entry must contain has to be produced
*by the block***: evidence named in prose outside the fence, or a manual step
inside it (`# make the edit by hand, then:`), is a gap the executor can only
fill with narrative, because narrative is the only thing left to fill it with.

**2. Mutation pairs are `## Spec` tasks, not shell edits inside the block.**
The executor contract **forbids in-place shell edits** — `sed -i`, `perl -i`,
and `>`/`tee` redirects into a source file are banned outright, and `bash`
refuses them (`executor/templates/executor_contract.md`, the do-not list). A
mutation pair written as a shell one-liner is therefore **unrunnable as
specified**: the executor either silently substitutes its `patch` tool — and
the marker `echo`s sitting between the banned commands go missing from the
transcript — or it narrates across the seam. Write each pair as three numbered
tasks in `## Spec`:

1. **Apply** — a `patch` on the pinned line, given as a worked example with
   the exact `old_str` and `new_str`, then a marker `echo` appended to the
   artifact file and the mutated test run appended after it.
2. **Restore** — the inverse `patch`, then its marker and the restored run.
3. **Prove it applied** — a `grep -c` of the mutated text after *each*
   direction, appended to the artifact. **Not optional**: a `patch` whose
   `old_str` no longer matches fails loudly, but one that matches the *wrong*
   line does not, and a mutation that silently did not apply certifies a
   vacuous guard.

Marker `echo`s and test runs are ordinary shell and stay legal; only the
*edit* moves to the `patch` tool. **Never spec `git checkout <file>` as the
restore** when the file holds the round's own uncommitted work — it discards
it. And per § "Derive every spec fact from its source", run the mutation
commands yourself before writing them into the spec.

**3. Seed the capture as the phase's last numbered task in `## Spec`.** The
E2E section is not seeded into the executor's task list — only `## Spec` is —
so a perfect block placed only there does not get run, and the executor
reports complete in good faith. The task's shape:

> ### Task N — Capture the end-to-end evidence
>
> Run the block in § End-to-end verification **verbatim and unmodified**, then
> paste the resulting artifact file into a new Update Log entry headed
> `### Update — <date> (end-to-end verification)`. The server-authored
> `(complete)` entry does not satisfy this.

Keep the block itself in § End-to-end verification — the task points at it.
The obligation is what gets tracked, not a duplicate of the commands.

*(Folded 2026-08-09 from DaemonEye, where this was the single largest failure
class: ten of one milestone's fourteen bounces and two of its four architect
takeovers were the missing-evidence requirement alone — and it was never a
capability problem; in each case the executor had run the commands and its
claims held up when checked. What separated producing runs from
non-producing ones, consistently: a literal runnable block vs. prose, an
explicit "the server (complete) entry does not count," and — decisively — the
capture existing as a seeded task. Three phases carried increasingly precise
block-writing guidance and produced no entry; the only rounds that produced
one were the rounds where the capture was an enumerated `## Spec` task. The
shell-edit prohibition went unnoticed for three phases because the executor's
silent `patch` substitution grades green.)*

### A pasted transcript is a claim, not evidence

**At review, re-run every command in the phase doc's End-to-end section and
diff the result against what was pasted.** Reading a transcript for
plausibility is not verification: a fabricated transcript is *built* to read
as plausible, and the gate set cannot see it — nothing an executor writes into
a markdown file affects a build.

The three shapes this takes, all observed downstream:

- **Paraphrase in place of a quote.** The entry describes what the command
  showed instead of showing it. Cheapest to spot: grep the entry for the
  command string and an exit-code marker; if neither is there, no transcript
  was pasted.
- **A splice inside an otherwise-real transcript.** The dangerous one: 24 real
  lines and one copied from a neighbouring file with a field swapped. No
  reading of the block reveals the bad line — only re-running and diffing.
- **Results asserted in the completion summary** while the Update Log holds
  only a progress stub.

**A true claim in a hand-made transcript is still a failure.** In every
observed case the underlying behavior was correct — what was missing was the
evidence chain, which is the only part that survives to the next reader.
Approving on "the claims check out" trains the next transcript to be written
rather than captured.

**A completion summary is a claim too, including its deviations line.** That
line has been wrong in both directions: "Deviations from spec: None" while an
unrelated tool's user-visible text had been rewritten, and a reported removal
of a binding that never existed in the file. Neither caused a regression;
neither was catchable by reading the summary carefully. Read the diff, not the
narrative, and treat an undeclared change and a fabricated one as the same
class of finding. (Recorded against a majority of *accurate* self-reports in
the same milestone — the point is not that executors lie, it is that accuracy
is unknowable from the text and cheap to establish from the diff.)

Two rules follow:

- **Executor:** capture mechanically (§ "End-to-end verification" in the phase
  template). Never hand-assemble.
- **Reviewer:** re-run and diff. "The transcript looks right" is inadmissible;
  "I re-ran it and it matched" is the finding — the check that matters is the
  one the author cannot fake by writing more convincingly.

**Give the executor a paste-fidelity check it can run itself.** Telling it to
paste verbatim is not enough, and neither is making the artifact small (a
56-line artifact has been retyped from memory). Add a final `## Spec` task
that extracts the pasted block back out of the phase doc and diffs it against
the artifact, printing `PASTE MATCH` / `PASTE MISMATCH`, and make `PASTE
MATCH` an acceptance criterion. On a mismatch the `diff` names the retyped
lines, so the executor fixes them from the file instead of guessing.

```bash
D=docs/dev/milestones/M<n>-<slug>/phase-NN-<slug>.md
START=$(grep -n '^### Update .*(end-to-end verification)' $D | tail -1 | cut -d: -f1)
tail -n +$START $D | awk '/^```/{n++; next} n==1' > /tmp/pasted-NN.txt
diff /tmp/pasted-NN.txt /tmp/e2e-NN.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

**Anchor the search to the heading, not a bare substring** — the phrase
"end-to-end verification" also appears in the spec's own prose and in the
server-authored `(complete)` entry, which is appended *after* the executor's
check runs; a bare-substring grep re-run at review extracts the wrong block
and reads a false `PASTE MISMATCH`. And run the check against a known-bad
entry before speccing it.

*(Folded 2026-08-09 from DaemonEye: three transcript-fabrication occurrences
in one milestone — paraphrase, splice, prose-only — each costing a full
dispatch-and-review round trip, the splice caught only because that reviewer
happened to re-run; then two consecutive retyped-transcript bounces in a later
milestone, resolved on the first outing of the self-check above,
byte-identical. Same mechanism as § "Give the executor a condition it can
check": what moves the executor is a condition it can evaluate.)*

### Coverage claims are inadmissible without mutation proof

**Never write "test X guards line Y" in a spec, a review, or an Update Log
unless the claim has been demonstrated by mutation** — break the line, watch
that test fail, restore it, watch it pass, and quote the pair. The usual
mechanism behind a false coverage claim is a **fixture default**: a shared
`make_*` helper that initialises a field to the value the assertion checks for
makes every assertion on that field tautological, and no gate can see it.

Rules that follow:

- **A spec must not name the discriminating test.** Naming it plants the
  conclusion; the executor then reports what the spec suggested rather than
  what it observed. Require the demonstration instead.
- **"The tests pass" is admissible. "The tests would catch a regression here"
  is not** — unless the mutation pair is quoted alongside it.
- **When reviewing a phase whose deliverable *is* coverage, re-run the
  mutations independently.** A claimed mutation check is not one.

**Confirm the property is observable before pinning it** — otherwise the spec
asks for a test that cannot exist, and what comes back passes on unrelated
grounds. When a spec names a branch, describe a sequence that *reaches* it,
not merely the value it returns (two branches returning the same value make
the assertion prove nothing about which ran). When a spec pins an observable
property, verify it is observable at all (ordering a serializer discards
cannot be asserted through the serializer). The tell in every case is that
the mutation does not fail the test — which is why the mutation must be run,
and run by the reviewer.

**A guard's premise must be demonstrated, not described.** A test for a guard
or exclusion passes trivially whenever the input would have produced the same
result by another path. The shape is always a negative assertion ("returns
`None`", "the row is absent") paired with a fixture that is *inert* rather
than *near-miss*: seeding a non-empty fixture is not sufficient — **the
fixture must be one the input would otherwise match**, and the spec should say
why the seed is reachable. A comment stating the intent is not the
demonstration; require the both-directions mutation pair, not the rationale.

**When a test depends on fixture ordering, assert the order in the test.** A
fixture built so "A is tried first, fails, and B is used" only tests the
fallback if A really comes first — and when that order is decided by something
the author reasoned about rather than ran (a ranking function, a sort, a hash
iteration), it is a system fact like any other and wrong often enough to
matter. One line converts a silent false pass into a loud self-describing
failure: assert the premise (`assert_eq!(hits.first()…, Some(expected),
"fixture precondition: …")`) before asserting the behavior.

*(Folded 2026-08-09 from DaemonEye, where this family recurred across four
milestones: three false coverage claims before the rule existed; three
unobservable-property specs, all architect-authored; three vacuous guards
caught only by a reviewer running the mutation; and one false ordering premise
that cost a full dispatch — the executor could not make the required mutation
fail and stalled after ~45 consecutive runs of that one test, which is the
pathology working correctly, refusing to certify an unfalsifiable guard.)*

### Every acceptance criterion must be satisfiable, and its mechanics pinned

Before dispatch, **re-read every acceptance criterion against the body of your
own spec — and against what the executor is permitted to do** — and confirm
the spec does not instruct the executor to violate it. The recurring failures:

- **Contradiction.** A criterion requires a count to stay fixed while the
  spec's own tasks change it, or requires a file to show no changes while the
  Test plan puts that phase's new tests in it. Green is impossible; the
  executor does the work correctly and then burns its remaining turns fighting
  the criterion until the governor fires.
- **Under-specification.** A criterion asks the executor to prove a property
  of its own diff without naming a baseline — but the executor commits as it
  works, so a bare `git diff` says nothing about committed work. **Pin the
  baseline commit and the exact command.**
- **A mechanism the harness forbids.** If the criterion depends on a
  particular mechanism (restore this file, edit in place with `sed -i`),
  **name the mechanism and confirm the executor contract permits it** — a
  refinement built around a command the shell guard blocks is not a
  refinement. Include these workflow docs themselves in that check: a
  criterion form prescribed here can still be one the contract bans.

*(Folded 2026-08-09 from DaemonEye: two `NoProgressStall` hard-fails caused by
unsatisfiable criteria — sixty read-only turns each, the implementation
correct both times — then two further breaches after the rule existed, one of
them a rule in the workflow docs themselves prescribing a shell-edit form the
executor contract bans. The criterion was checked against the tree and never
against the rest of its own spec.)*

**Validate every mechanical criterion against the tree the phase will
*produce* — by executing it, not reasoning about it.** Running a count today
proves the "now" value; it does not prove the target is reachable once the
phase's own tests, `use` lines and doc comments land. Prototype the intended
delta in a scratch copy, run the criterion there, and paste what it printed.
Reasoning the gap out has failed every time it was tried.

**A criterion about a gate is validated by running that gate**, not by a
proxy that resembles it. Deleting an `#[allow(dead_code)]` and grepping for
the attribute measures the attribute's absence; only the lint command shows
whether the tree is green without it.

**Count the Spec, don't estimate it.** A pinned number that describes the
phase's own tasks is arithmetic over prose you have already written: if the
Test plan names 12 tests, a criterion demanding 11 is wrong before dispatch.
Write the arithmetic *in the criterion* — `(now 6) + 1 named test = 7` — so
the executor can re-derive it from the doc, and anchor `fn`/`pub` greps with
the `(` or trailing token that matches a declaration and nothing else. A
number the executor cannot re-derive is one it is right to distrust.

*(Folded 2026-09-20 from a downstream project, eleven occurrences across five
milestones; the executor was right every time and twice filed a blocker
rather than pad the count.)*

### A cleanup obligation's criterion must assert the cleanup *ran*

When a phase hands out a resource — an alternate screen, a terminal mode, a
lock, a temp file — the criterion guarding its release must assert **that the
release happened, and how many times**. A criterion that asserts a mechanism
*exists* can be satisfied while the phase is more broken than before.

Write:

- [ ] Test `x_guard_runs_teardown_on_normal_exit` passes: a guarded scope
      returning normally ran the teardown **exactly once** (`== 1`, not `>= 1`).

Not:

- [ ] `grep -c "impl Drop" src/foo.rs` prints 1.

Assert the count, not merely non-zero: a teardown that runs twice is also
wrong, and `>= 1` hides it. Each occurrence of this class measured a proxy for
the obligation instead of the obligation.

*(Folded 2026-09-20 from a downstream project, three occurrences — one a
round-2 fix that satisfied every round-1 criterion and broke the common path.)*

### Run every count criterion; never derive it

A phase doc that pins a count (`grep -c … returns 4`) is making a claim about
the tree. **Run the command and paste its answer; never compute the number by
reasoning about the code.** Two corollaries:

- **Text-based greps count prose.** A doc comment, an assertion message, or a
  phrase in the spec you are writing will match. When a count comes back
  higher than expected, look for prose before assuming code.
- **The instrument can be blind.** A single-line grep cannot see a multi-line
  split or the same call on a differently-named variable — both report clean
  while real sites remain. Before pinning a criterion, ask what the instrument
  *cannot* see; where the type system can enforce the property instead,
  prefer that and retire the grep.

**Re-run the block before dispatch, not only at drafting.** A number correct
when the phase was written goes stale as soon as an earlier phase edits the
file. Numbers age; re-run, don't trust — and a phase doc that lists line
numbers should say they are current-as-of-drafting and point at the command
that re-derives them.

*(Folded 2026-08-09 from DaemonEye: four derived miscounts — one reporting
success against an unmet goal — two blind instruments, and a fifth miscount
from staleness in a phase drafted under the rule; meanwhile running the counts
caught an error in roughly a third of the phases that did it.)*

**Scope the search so the criterion's own corpus cannot satisfy it.** A grep
over a file that also contains the criterion's text, the spec's dictated
prose, or a pinned test vector can never reach its target. Three recipes:

| The grep is about… | Scope it with |
|---|---|
| production code only | `sed -n '1,/^#\[cfg(test)\]/p' <file> \| grep -c …` |
| the Update Log only | `sed -n '/^## Update Log/,$p' <doc> \| grep -c …` |
| a section of a doc | `sed -n '/^## Section/,/^## /p' <doc> \| grep -c …` |
| an identifier the phase itself introduces | prod-scope it (row 1) and expect the phase's own test names, `use` lines and doc comments to match it — they arrive with the code |

Make the tell a literal step: **after writing a grep criterion, run it against
the phase doc you are writing.** A non-zero hit on your own spec means the
corpus is contaminated — scope it or pick a different token.

**A criterion that already passes is either a preservation criterion or a
vacuous one — label which.** If the "now" value you *ran* does not match the
"now" value you *wrote*, stop. When a criterion is meant to pass now and keep
passing, say **preservation criterion** in the text so the executor does not
read it as a task. And **`git diff <branch>` on the working branch is empty by
construction** — pin a body anchor or a baseline commit hash instead.

*(Folded 2026-09-20 from a downstream project, eight occurrences, all caught
at drafting once the criteria block was run before dispatch.)*

### A check the executor cannot satisfy is a bounce you wrote yourself

Running a count against today's tree proves the "now" value. It does not
prove the target is reachable, that the anchor survives the executor's own
edits, or that the test a mutation is meant to redden exists. Four shapes,
each seen to cost a full turn budget or a round trip:

- **A count asserting a structure that does not exist** — an exit path the
  compiler reports unreachable, counted as a site to instrument.
- **An anchor the executor's own edit invalidates** — sites identified by line
  number when the phase's first task inserts lines above them.
- **A whole-file grep whose target the spec itself puts in the file** — a
  `→ 0` where the Test plan requires the symbol in a test, or a `→ 2` where the
  natural `use` import makes correct code read 3.
- **A mutation naming an arm no test renders.**

The rule: **before dispatch, show every criterion, anchor and mutation is
satisfiable — not merely that its "now" value is what you wrote.**

1. **A count's target is derived from something you ran.** When the compiler
   can settle a structural claim (reachability, an arm's existence, a trait
   bound), ask it — a probe edit plus a build is cheaper than a lost budget.
2. **Anchors are quoted text, never line numbers.** Quote the unique line(s)
   `patch` should match; those survive every edit but the one meant to change
   them.
3. **Scope a grep to what it is actually about** — production-only via the
   recipes above, or include the call's argument (`'fn_name(&'`) to count
   call sites rather than mentions.
4. **A mutation prescription names the test AND the assertion, and you have
   confirmed that test exists and reaches that arm.** If you cannot point at
   the assertion message the failure will print, you have described a wish.

The tell, every time: the check was written from the spec's own description
of the code rather than from the code. Hitting the turn cap on such a phase
is a spec smell, not a capacity problem; the remedy is a satisfiable spec, not
a larger cap.

*(Folded 2026-09-20 from a downstream project, four occurrences, two of them
full turn-budget losses.)*

### A sweep's scope is its convertible sites, not its matches

When a phase applies one mechanical change to every instance of a pattern — a
call wrapped in an adapter, an API migrated, a symbol renamed — the match
count is where scoping **starts**, not where it ends. A hit can be real,
correctly matched, and still not convertible: its enclosing context may not
admit the change (in Rust, e.g., a sync enclosing fn for an `async`
conversion, or a `Drop` impl), it may already be in the target form, or its
expression may extend past the line a slice boundary was drawn on.

The remedy is cheap: **write the classifier, not just the counter.** For every
hit, report its enclosing function and whether that context admits the
conversion; then put the unconvertible ones in the phase doc **by name**, as a
do-not-convert list with the reason for each. That table buys two things — the
executor does not spend turns discovering the compiler error and then "fixing"
it by widening scope, and the finish condition becomes an exact residue ("the
scan reports 4, and all four are on the list") instead of an unreachable zero.

**Sites needing a restructure go in a restructure phase**, not bundled into a
mechanical sweep: the phase looks uniform, gets sized as uniform, and then one
site consumes the run. And when a sweep exhausts a symbol's last use, **say
what happens to its import** — count the remaining uses *after* the planned
conversions, and either authorise the deletion explicitly or name the
surviving uses, so the executor neither trips a strict-lint gate on a dead
import nor deletes a live one on pattern-momentum.

*(Folded 2026-08-09 from DaemonEye: five mis-scoped sites across one sweep
milestone, plus two hard-fails on the exhausted-import case, where build and
strict clippy disagree about whether an unused test-module import matters.)*
