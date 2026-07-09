---
name: auto
description: >
  Run the architect/executor loop hands-off across a milestone — draft,
  dispatch, review, escalate/re-dispatch — with full review rigor and no
  per-phase pause. Opt-in per run; stops at a milestone boundary, a blocker,
  assist-budget exhaustion, or the runaway backstop.
model: opus
argument-hint: "[max-phases]"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), Agent
---

# Auto Skill — the opt-in autonomous loop

This skill drives the architect/executor cycle **hands-off across a whole
milestone**: it drafts the next phase, dispatches it, reviews the result, and
escalates or re-dispatches on failure — cycling until the milestone closes or a
stop condition fires. It exists so a user who wants a hands-off run gets one,
without giving up any review rigor.

## The one invariant: compose, never fork

**This skill composes the existing four skills. It does not reimplement them.**

| Loop step | The skill it invokes |
|---|---|
| draft the next phase | `architect` (§3 phase-doc authoring) |
| dispatch a phase | `dispatch` |
| review a completed phase | `review` |
| decide an escalation lever | `escalate` |

The loop adds **only** sequencing, the per-phase assist budget, journaling, the
four stop conditions, and the loop report. It does **not** restate the Definition
of Done, the bug-report template, the escalation levers, or the status-flip
mechanics — each composed skill owns those, and running that skill's procedure
verbatim is what keeps an autonomous run identical to an interactive one.

**A behavior difference between an interactive run of a step and its autonomous
run is a bug**, not a feature. If you ever find yourself writing "in auto mode
we do X differently," stop — that is the anti-pattern this skill exists to avoid.

## Read these first

Before any action:

1. Read `<repo>/docs/dev/NEXT.md` (the active-phase pointer) and
   `<repo>/docs/dev/WORKFLOW.md` § "Opt-in autonomous loop (off by default)" and
   § "What Executors Never Decide" (the human-territory list this loop stops on).
2. Read all four composed skills — `architect`, `dispatch`, `review`, `escalate`
   — so you invoke their procedures unchanged.
3. Read `<repo>/rexymcp.toml`: confirm `[executor]` + `[commands]` are present
   (bootstrap done), and read `[escalation] max_assists` (the per-phase assist
   budget, default 3) and `[architect] dispatch_model` / `review_model` (the
   role models for delegation; unset = inherit the session model).

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR`,
`ANTIGRAVITY_PROJECT_DIR`, or the nearest ancestor containing
`docs/dev/milestones/`.

## 1. Delegation model — which step runs where, on which model

This is why the config carries exactly two role keys. Delegate the two
**mechanical, procedure-driven** steps to subagents on their role model; keep the
two **context-hungry, judgment** steps in the main loop where the milestone
thread lives.

| Step | Runs in | Model |
|---|---|---|
| **draft** | main loop | session model — needs the talk-through history |
| **dispatch** | **subagent** | `[architect] dispatch_model`, else inherit |
| **review** | **subagent** | `[architect] review_model`, else inherit |
| **escalate decision** | main loop | session model — needs the briefing + design intent |
| **refined re-dispatch** (the lever's action) | **subagent** | `dispatch_model`, else inherit |
| **session takeover** | main loop | session model |

**Mechanism (verified against the Claude Code subagent docs):** spawn a subagent
with the **`Agent`** tool and pass the role model as the invocation's `model`
parameter, read from `rexymcp.toml` at runtime. Give the subagent the `Skill`
tool and instruct it in its prompt to run the composed skill for the phase — e.g.
"Run the `/rexymcp:dispatch` procedure for `<phase>` and report the returned
`PhaseResult` verbatim." The subagent returns the structured result (status,
verdict, briefing) to the main loop; the loop makes every branching decision.

**Inherit-by-default (06a semantics).** If a role model is **unset** (`None`),
**omit the `model` parameter** so the subagent inherits the session model. Do
**not** substitute `[architect] model` — that field is the cost-rate model, a
separate concern.

**Degrade rule (honesty).** The verified mechanism supports per-call model
override, so role-model delegation is the normal path. But if, in a given client
or version, a per-invocation model override is unavailable, run dispatch/review
on the session model and **say so in the loop report** ("role-model delegation
unavailable; ran on the session model"). Never claim a model switch that did not
happen.

## 2. The loop

**Pre-flight (once):**

- Resolve `<repo>`. Confirm `rexymcp.toml` has `[executor]` + `[commands]`; if
  not, point the user at `/rexymcp:architect` to bootstrap and **stop**.
- **Run from the target repo's own session.** `execute_phase` enforces a
  root-corroboration check: it refuses any `repo_path` that does not match the
  MCP client's advertised roots or `CLAUDE_PROJECT_DIR` / `ANTIGRAVITY_PROJECT_DIR`.
  So `<repo>` must be **this session's** project root — you cannot drive a loop
  for repo B from a session rooted in repo A (the dispatch fails with MCP error
  `-32602` before reaching the executor). If `<repo>` is not the session root,
  **stop** and tell the user to launch Claude Code in the target repo and re-run.
- Call the `executor_health` MCP tool. If the endpoint is unreachable, surface
  the error and **stop** — do not enter the loop against a dead executor.
- Read `max_assists` and the two role models.
- Parse the optional `max-phases` argument (the runaway backstop). **Default 8.**
- Initialize `phases_this_run = 0`.

**Loop — repeat until a stop condition fires:**

1. **DRAFT-or-adopt** (main loop). First check `NEXT.md`: if it **already** points
   at an active `todo` or `in-progress` phase (e.g. one drafted in a prior
   interactive session, or left `in-progress` by an earlier bounce), **adopt that
   phase** — skip drafting and go straight to dispatch (do not journal `draft`; no
   drafting happened). Only when there is **no** active phase do you run the
   `architect` skill's §3 phase-authoring procedure to draft the next one and
   journal `draft` (§4). Either way, if drafting reports the **milestone boundary**
   (the milestone's in-scope phases are all `done`, or `NEXT.md` would go to
   "none") → **STOP(boundary)** (§3).
2. Set `assists_this_phase = 0`.
3. **DISPATCH** (subagent, `dispatch_model`). Run the `dispatch` skill for the
   phase. Journal `dispatch` with `--outcome` = the returned status.
4. **Branch on `PhaseResult.status`:**
   - `complete` → go to **REVIEW** (step 5).
   - `hard_fail` / `budget_exceeded` → go to **ESCALATE** (step 6).
5. **REVIEW** (subagent, `review_model`). Run the `review` skill for the phase.
   Journal `review` with `--outcome` = the verdict.
   - **approved** → `phases_this_run += 1`. If that was the milestone's last
     in-scope phase → **STOP(boundary)**. Else go to **BACKSTOP** (step 7), then
     continue the loop.
   - **bounced** (the review skill filed a bug and flipped the phase to
     `in-progress`) → this is a re-dispatch round-trip: go to **ASSIST** (step 6a)
     with the bounce as the reason (the executor fixes the bug on re-dispatch).
6. **ESCALATE** (main loop). Run the `escalate` skill's lever-choice on the
   briefing.
   - **6a. ASSIST accounting.** If `assists_this_phase >= max_assists` →
     **STOP(budget)** (do not spend another round-trip on this phase). Otherwise
     `assists_this_phase += 1`, journal `assist` (`--outcome` = the lever or
     bounce reason), and act on the chosen lever:
     - **refined re-dispatch / resume** → re-dispatch (subagent,
       `dispatch_model`); go back to step 4 with the new result.
     - **session takeover** → journal `takeover`, implement the phase in the main
       loop and self-complete to `done` via the `escalate` skill's takeover
       steps; `phases_this_run += 1`; go to **BACKSTOP** (step 7); continue.
   - If **any** step surfaces a blocker, a "What Executors Never Decide" item, a
     contract-doc (STANDARDS / WORKFLOW / architecture) change need, a dependency
     request, or a spec-vs-architecture conflict → **STOP(blocker)**.
7. **BACKSTOP.** If `phases_this_run >= max-phases` → **STOP(runaway)**.

## 3. Stop conditions (four — the loop always halts for the human)

The loop **never** auto-continues past any of these. On each, go to §5 (harvest +
loop report) and stop.

- **boundary** — the milestone's in-scope phases are all `done` (or `NEXT.md`
  would go to "none"). This is an **absolute human gate** — the loop **never**
  crosses into the next milestone (the retrospective, calibration folds, and the
  go/no-go for the next milestone are human judgment; see the `architect` skill
  §6 prohibition #3 and WORKFLOW "Milestone boundaries are always a human gate").
- **budget** — `assists_this_phase >= max_assists` on the current phase. The
  human decides whether to raise `max_assists`, take over, or re-scope the phase.
- **blocker** — any blocker, any "What Executors Never Decide" item, a
  contract-doc change need, a dependency request, or a spec-vs-architecture
  conflict. These are human territory by definition.
- **runaway** — `phases_this_run >= max-phases` (the backstop; default 8,
  overridable via the `max-phases` argument). A safety net against an unbounded
  run, not a normal exit.

## 4. Journaling — every activity, exact command

Every architect activity in the loop is journaled to the telemetry store — this
is what makes `PhaseRun.escalation_count` real and feeds per-activity token/cost
accounting. After each step, run (from `<repo>`; `--project-id` defaults from
`[project].id`; pass `--model` = the model that **actually performed** the step
so cost uses that role model's rates):

```bash
rexymcp journal --config <repo>/rexymcp.toml \
  --phase-id <phase short id> \
  --phase-doc <abs phase-doc path> \
  --milestone-id <milestone dir slug, e.g. M27-autonomous-escalation-loop> \
  --activity <draft|dispatch|review|assist|takeover|boundary> \
  --outcome <status / verdict / lever / stop reason> \
  --model <model that performed the step>
```

Use **only** these six canonical activity strings — an unknown kind is recorded
but warns:

- `draft` — authored or refined a phase doc
- `dispatch` — dispatched a phase to the executor
- `review` — reviewed a completed phase against the DoD
- `assist` — refined + re-dispatched (or resumed) after a bounce / hard_fail / budget_exceeded
- `takeover` — took the phase over directly (session takeover)
- `boundary` — reached a milestone boundary or any loop stop condition (§5)

## 5. On any stop — harvest, then the loop report

Do all of the following, in order, on **every** stop:

**a. Journal the `boundary` activity** with `--outcome` = the stop reason
(`boundary` / `budget` / `blocker` / `runaway`). This is the persisted, queryable
half of the loop report.

**b. Harvest token usage (Claude Code only; degrade gracefully).** So the report's
cost totals are real, not estimated:

```bash
rexymcp harvest --config <repo>/rexymcp.toml --transcript-dir <Claude Code session transcript dir>
```

The transcript dir is Claude Code's `~/.claude/projects/<slug>/`, where `<slug>`
is the project path with `/` replaced by `-` (e.g. `-home-matt-src-rexyMCP`). If
the client is **not** Claude Code, or the transcript dir cannot be located, **skip
the harvest** and report token/cost as **absent** — never estimate Claude's own
usage (the same no-fabrication rule the review applies to executor tokens).

**c. Print the loop report** to the session (no committed report file — the
persisted half is the `boundary` record from step a):

```markdown
## /rexymcp:auto loop report

- **Milestone:** M<n> — <name>
- **Stopped:** <boundary | assist budget exhausted on <phase> | blocker: <what> | runaway backstop>
- **Phases this run:** <N>
  - <phase-id> — <verdict> (assists: <n>)
  - …
- **Total assists spent:** <N>
- **Token / cost:** <harvested totals, or "absent — <client> provides no transcript usage">
- **What needs the human:** <the specific next action — sign off on the milestone
  boundary, resolve the blocker, raise max_assists and restart, or restart
  /rexymcp:auto with a larger backstop>
- **Live view:** `rexymcp status` / `rexymcp dashboard`
```

Point the human at `rexymcp status` / `rexymcp dashboard` for the live view:
Claude Code sends no MCP progressToken, so a long run is invisible mid-phase from
the MCP side. A Claude Code-native stop notification is a cheap optional
skill-layer enhancement — use it if available, but it is not required and must
degrade silently elsewhere.

## 6. What you do not do

- You do **not** cross a milestone boundary — that is always a human gate.
- You do **not** modify `STANDARDS.md` / `WORKFLOW.md` / `docs/architecture.md`,
  add a dependency, or resolve a spec-vs-architecture conflict — those are all
  **STOP(blocker)**.
- You do **not** fork the composed skills — you invoke their procedures unchanged.
  Any interactive-vs-autonomous behavior difference is a bug.
- You do **not** fabricate token counts — harvested or reported absent, never
  estimated.
- You do **not** skip the review gate or soften its rigor because the run is
  autonomous — the `review` skill runs verbatim, independent gate re-runs and all.
- You do **not** spend past `max_assists` on a phase, or past the `max-phases`
  backstop on a run — both are hard stops for the human.
