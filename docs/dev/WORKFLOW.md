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
change to make.

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
turn on an autonomous mode that chains draft → dispatch → review across phases,
stopping only on a blocker or a milestone boundary. It is explicitly enabled per
run, never the default. When on, the architect still files blockers rather than
improvising, and still halts at milestone boundaries.

**Route opencode-hostile content to direct execution.** Some content classes
reliably corrupt the opencode executor's tool-call serializer — notably
close-tag-shaped text (`<tool_call>`/`</function>`), markdown fences, and escaped
quotes inside JSON string literals inside Rust source (see AGENTS.md § "Writing
files when the opencode tool harness fails"). When a phase's deliverable is
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
