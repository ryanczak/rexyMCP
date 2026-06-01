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
change to make.

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
phases are all `done`, the architect stops for human sign-off regardless of mode.
This is where the retrospective and doc-folding happen, which is human judgment —
never automated.

**Opt-in autonomous loop (off by default).** For hands-off runs, the user may
turn on an autonomous mode that chains draft -> dispatch -> review across phases,
stopping only on a blocker or a milestone boundary. It is explicitly enabled per
run, never the default.

**The executor is a local LLM, not a coding agent.** The model driving phases
through this workflow is a single-purpose executor: it has the project's tool set,
the embedded contract + STANDARDS + the phase doc, and a bounded turn budget. It
does *not* have web access, cannot escalate mid-phase to a stronger model, and
does not negotiate scope. Treat its outputs as the work of a junior engineer who
cannot ask clarifying questions: the spec must front-load everything (worked
examples, idioms, few-shot exemplars, fetched reference docs — the *pre-injection*
the architect owns). Mismatched-expectations bugs are *spec bugs*, not executor
bugs.

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
