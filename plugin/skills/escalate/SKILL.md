---
name: escalate
description: >
  Decide what to do with a hard_fail briefing from execute_phase: refined
  re-dispatch (default), session takeover, or resume.
model: opus
argument-hint: "<phase>"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), WebFetch, WebSearch
---

# Escalate Skill

This skill handles **escalation decisions** when the executor returns a
`hard_fail` or `budget_exceeded` result. Given the briefing, you choose a
lever: refined re-dispatch (default for weak models), session takeover (last
resort), or resume. The decision is judgment-heavy — wrong defaults burn the
architect-executor split.

## Read these first

Before any action:

1. Read the **briefing** from the returned `PhaseResult.briefing`. This is
   your primary input — it contains `one_line`, `current_blocker`,
   `what_was_tried`, and `diagnostics`.
2. Read the **phase doc** (resolve `<phase>` from the argument) to
   understand the original spec, its acceptance criteria, and what the
   executor was attempting.
3. Read `<repo>/docs/dev/STANDARDS.md` for context on the engineering
   contract the executor was held to.
4. If the briefing references specific turns, query the session log via
   `executor_log_tail` + `get_turn` MCP tools using the phase's `log_path`.
5. Read any open bug reports in the milestone's `bugs/` directory that
   reference this phase.

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR`, `ANTIGRAVITY_PROJECT_DIR`, or the
nearest directory containing the milestone layout.

## 1. Refuse non-failure results

Check `PhaseResult.status`:

- If `"complete"`: this is a review, not an escalation. Point the user at
  `/rexymcp:review <phase>` and stop. Escalate is not for clean results.
- If `"hard_fail"` or `"budget_exceeded"`: proceed to §2.

## 2. Choosing a lever

A `hard_fail` briefing is a signal: the executor reached the budget, hit a
diagnostic it couldn't resolve, lost track of state, or otherwise stopped
without producing a clean `PhaseResult`. The escalation question is **what
changes** so the next attempt succeeds.

Three levers, in order of preference:

### Refined re-dispatch — the default for weak models

The local executor is a smaller LLM than you are; it lacks web access; it
cannot ask clarifying questions mid-phase. *Most* `hard_fail`s trace back to
a spec gap the executor couldn't bridge, not to an executor mistake the
executor should have avoided.

**Diagnostic:** read the briefing's `what_was_tried` list and ask "would a
tighter spec have prevented this?" If yes (and most of the time, yes),
refine and re-dispatch.

Common refinements that turn `hard_fail` into `approved_first_try`:

- **Add a worked example** the executor was missing — they were trying to
  invent something instead of pattern-matching.
- **Pin a negative case** (per `WORKFLOW.md` "Pin negative cases") — the
  executor satisfied the positive examples but tripped the boundary case.
- **Quote an API doc inline** instead of linking to it — the executor
  couldn't reach the link.
- **Authorize a narrow upstream edit** the executor needed but wasn't
  permitted (per `WORKFLOW.md` "Anticipate cross-boundary trait bounds").
- **Verify an external-API claim** (per `WORKFLOW.md` "Verify external APIs
  against live docs") — the architect's sketch was stale and the executor
  lost time trying to make it work.

This lever is cheap (one model call) and produces telemetry
(`PhaseRun.bounces_to_approval` increments by 1). The architect learns; the
executor learns by re-trying with better inputs; the `model_scorecard`
accumulates a real data point on bug-class-to-fix ratios.

### Session takeover — last resort

You (Claude) take over and implement the phase directly. Use this when:

- You've already done one refined re-dispatch and the same class of failure
  recurred (signaling the executor genuinely can't reach this work, not a
  spec gap).
- The briefing reveals the executor lost track of state in a way that a
  re-dispatch would just re-encounter (e.g. ran out of context budget on a
  phase that's too big for any spec refinement).
- The phase is on the critical path and the user is time-pressed.

**Cost: the telemetry gap.** When you implement the phase, the
`PhaseRun.architect_verdict` records `escalated` instead of an `approved_*`
from a model — you produce a successful artifact but *no* model-vs-spec data
point. The `model_scorecard` is blind to the run. Use sparingly.

When you do take over:

1. Flip the phase's `Status:` to `in-progress (architect takeover)` with a
   one-line note.
2. Implement the phase directly using your file-edit tools.
3. Run the command set yourself.
4. On completion, write the Review verdict with `Executor: Claude (direct)`
   and `Verdict: escalated`.
5. Flip to `done`.
6. Tell the user: "Phase completed via session takeover."

**Anti-pattern: skipping refined re-dispatch because "this case feels
special."** Every hard_fail feels special to the architect reading the
briefing. That's exactly why the discipline matters most when it feels least
convenient. If you find yourself jumping to takeover on the first failure,
**slow down**: read the briefing's `what_was_tried` carefully, ask "what
would a tighter spec change?", try the refinement once. The data is what
makes the model scorecard real over time.

### Resume — resume from a fresh briefing-seeded context

Call `continue_phase` to resume a phase when the failure reads as "we were
most of the way done and hit one specific wall" — a late `budget_exceeded`, or
a single diagnostic the executor couldn't clear — where the completed work is
worth preserving. The resumed run gets a fresh context seeded with the phase
doc, architect guidance, the current on-disk diff, and restored task states
from the prior session log.

**Choose resume over re-dispatch** when the *spec* was fine but the executor
just didn't finish (budget, a transient error, one stubborn lint). Re-dispatch
is better when the *spec* was the problem — the resumed context would carry the
same gap forward.

**Choose resume over takeover** when the executor can reach the work but just
needs more turns or a hint about what to fix.

**Execution steps:**

1. Call `continue_phase` with:
   - `phase_doc_path`: the phase doc path (same as the failed run).
   - `repo_path`: the target repository path.
   - `guidance`: a distilled string from the briefing — what to fix, what is
     already done, what to avoid re-doing.
   - `prior_log_path`: the failed `PhaseResult.log_path`, used to restore task
     states.
2. Treat the returned `PhaseResult` like any dispatch result: review on
   `complete`, escalate again on failure.

### Decision summary

| Failure shape | First-attempt lever |
|---|---|
| Spec gap (missing example, unclear acceptance, missed authorization) | Refined re-dispatch |
| External API drift (architect's sketch was stale) | Refined re-dispatch with verified docs |
| Boundary / negative case the spec didn't pin | Refined re-dispatch with pinned negative |
| Most of the phase done, hit one wall | Resume |
| Repeated same-class failure after one refinement | Session takeover |
| Context-budget exhaustion on a phase that's already minimal | Session takeover (or re-split into two phases) |
| Anything that feels special | Refined re-dispatch — feeling-special is not a lever |

## 3. Execute the chosen lever

### Refined re-dispatch

1. Amend the phase doc's Spec or Pre-flight based on the briefing's
   `what_was_tried` + `current_blocker` + `diagnostics`.
2. Add a `Notes for executor` block at the top of the Update Log explaining
   the refinement:

   ```markdown
   ### Notes for executor — YYYY-MM-DD

   <One paragraph: what was refined and why, referencing the briefing's
   specific fields.>
   ```

3. Tell the user: "Refinement applied. Re-dispatch via `/rexymcp:dispatch
   <phase>`."
4. Leave the phase's `Status:` as `in-progress` (the executor was mid-phase
   and is now refining; dispatch accepts both `todo` and `in-progress`).

### Session takeover

Follow the steps in §2 under "Session takeover — last resort."

### Resume

Follow the steps in §2 under "Resume — resume from a fresh briefing-seeded
context."

## 4. Write the escalation outcome

Always write an escalation entry to the phase doc's Update Log:

```markdown
### Update — YYYY-MM-DD HH:MM (escalation)

**Chosen lever:** refined re-dispatch | session takeover | resume (deferred)
**Rationale:** <one sentence: why this lever over the others>
```

Append this after the `<!-- entries appended below this line -->` comment.

## 5. What you do not do

- You do **not** escalate `complete` results — those go to `/rexymcp:review`.
- You do **not** auto-advance after a refined re-dispatch. The user
  dispatches explicitly via `/rexymcp:dispatch <phase>`.
- You do **not** modify `STANDARDS.md` or `WORKFLOW.md` without explicit
  user approval and a recurring-pattern fold.
