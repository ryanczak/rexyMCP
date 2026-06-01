---
name: escalate
description: >
  Decide what to do with a hard_fail briefing from execute_phase: refined
  re-dispatch (default), session takeover, or resume (future).
model: opus
argument-hint: "<phase>"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), WebFetch, WebSearch
---

# Escalate Skill

This skill handles **escalation decisions** when the executor returns a
`hard_fail` or `budget_exceeded` result. Given the briefing, you choose a
lever: refined re-dispatch (default for weak models), session takeover (last
resort), or resume (not yet implemented). The decision is judgment-heavy —
wrong defaults burn the architect-executor split.

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

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR` or the
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

### Resume — not yet implemented

A `continue_phase` MCP tool that resumes a failed phase from a checkpoint is
a possible future addition (the M4 session log makes the prerequisites
available), but does not exist today. If a hard_fail reads like "we were 90%
done and just ran out of turns," refined re-dispatch with a tightened spec
(smaller scope, more pre-injection) is still the right call. Note the resume
question in calibration if it recurs; fold a new milestone if the pattern
hardens.

### Decision summary

| Failure shape | First-attempt lever |
|---|---|
| Spec gap (missing example, unclear acceptance, missed authorization) | Refined re-dispatch |
| External API drift (architect's sketch was stale) | Refined re-dispatch with verified docs |
| Boundary / negative case the spec didn't pin | Refined re-dispatch with pinned negative |
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
4. Flip the phase's `Status:` back to `todo` (or leave it `in-progress` if
   the executor was mid-phase — the dispatch skill will check).

### Session takeover

Follow the steps in §2 under "Session takeover — last resort."

### Resume

Tell the user: "The resume lever (continue_phase) is not yet implemented.
Pick refined re-dispatch or session takeover instead."

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
