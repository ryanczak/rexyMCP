# Phase 06b: `/rexymcp:auto` loop skill + loop report + WORKFLOW mirror

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** done
**Depends on:** phase-06a (done — the `[architect] dispatch_model`/`review_model` config keys this skill reads)
**Estimated diff:** ~360 lines (one new SKILL.md + a ~17-line WORKFLOW template paragraph swap)
**Tags:** language=prose, kind=feature, size=l

> **Direct-execution phase.** The deliverable is a prose plugin skill that
> orchestrates Claude Code subagents — not Rust for the local-LLM executor. It is
> **authored directly by the architect (Claude)**, not dispatched to the executor.
> It still follows the normal draft → implement → review gate; only the *executor*
> is "Claude (direct)". Record `Executor: Claude Code (direct)` in the review
> verdict.

## Goal

Ship `/rexymcp:auto` — the opt-in autonomous loop that chains draft → dispatch →
review → escalate/re-dispatch across a milestone's phases with **full review
rigor and no per-phase human pause**, stopping only at a milestone boundary, a
blocker, per-phase assist-budget exhaustion, or a loop-level runaway backstop.
It **composes** the existing four skills (does not reimplement their logic),
delegates dispatch and review to subagents on the configured role models
(06a's `dispatch_model`/`review_model`), journals every architect activity via
`rexymcp journal`, harvests token usage where available, and writes a **loop
report** on every stop. Also mirror the amended autonomous-loop paragraph into
the plugin's `WORKFLOW.md` template so target repos ship the accurate contract.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` — especially
  § "Design (fixed at kickoff)" (the four forks: full-milestone loop; full-rigor
  review with no pause; all three threads; consolidate on `max_assists`),
  § "Token/cost accounting — the honesty constraint", § "The loop report",
  § "Per-role model delegation", § "Client-integration notes", and the phase-06
  split note (the three shape decisions: split 06a/06b; loop report = session
  output + telemetry record, no committed files; delegation via subagents with a
  model override).
- `docs/dev/WORKFLOW.md` § "Opt-in autonomous loop (off by default)" — the
  already-amended repo paragraph this phase mirrors into the plugin template, and
  § "What Executors Never Decide" — the human-territory list the loop stops on.
- The four existing skills this one composes, verbatim, as the procedures the
  loop invokes: `plugin/skills/dispatch/SKILL.md`, `plugin/skills/review/SKILL.md`,
  `plugin/skills/escalate/SKILL.md`, `plugin/skills/architect/SKILL.md` (its
  §3 "Phase-doc authoring" is the draft step; §6 prohibitions #2/#3 are the
  auto-advance / milestone-boundary gates the loop must honor).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above — **read all four existing SKILL.md
   files end-to-end**; this skill's correctness is "invoke those procedures
   unchanged," so you must know exactly what they do.
3. Read this entire phase doc before touching any file.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Verify the current Claude Code subagent mechanism against live docs**
   before writing the delegation section. The architect cannot reliably
   enumerate (a) the exact `allowed-tools` token for spawning a subagent (the
   sketch below assumes `Task`), (b) whether a subagent invocation can carry a
   **per-subagent model override**, and (c) how a subagent is told to execute an
   existing skill's procedure. Sources in priority order: the official Claude
   Code docs (subagents / Task tool / skill frontmatter `model` field); the
   `claude-code-guide` agent; working examples in this plugin. **Trust the docs
   over the sketch.** Pin the *behavior* (dispatch/review run under the role
   model; drafting/escalation stay in the main loop); adapt the *mechanism* to
   the real API. If per-subagent model override is **not** supported, degrade per
   § "Delegation" below (run the step on the session model and note role-model
   delegation is unavailable — never fabricate a model switch). Record any
   divergence between sketch and docs in "Notes for review".

## Current state

**No `auto` skill exists.** `plugin/skills/` contains exactly four skills:
`architect/`, `dispatch/`, `escalate/`, `review/` — each a single `SKILL.md`.
This phase adds a fifth: `plugin/skills/auto/SKILL.md`.

**Config substrate is in place (06a).** `ArchitectConfig`
(`executor/src/config.rs`) carries `dispatch_model: Option<String>` and
`review_model: Option<String>`; `None` means **inherit the session model** (it
does *not* fall back to `[architect] model`). `[escalation] max_assists`
(`executor/src/config.rs:61`, default **3**) is the per-phase autonomous assist
budget (M27 phase-01).

**The journal CLI exists (02) and the harvester exists (05b).** Confirmed
surfaces (do not re-derive — pinned here):

- `rexymcp journal` flags (`mcp/src/main.rs:276`): `--config <path>` (required),
  `--phase-id <str>` (required), `--activity <str>` (required), and optional
  `--phase-doc <path>`, `--project-id <str>` (defaults to `[project].id`),
  `--milestone-id <slug>`, `--outcome <str>`, `--model <str>`,
  `--telemetry-path <path>`. It appends one `ArchitectActivity` record and prints
  `recorded <activity> activity for <phase-id> -> <path>`.
- The **canonical activity vocabulary** (`ARCHITECT_ACTIVITIES`,
  `executor/src/store/telemetry.rs:468`) is exactly these six — use these
  strings, an unknown kind warns:
  `draft`, `dispatch`, `review`, `assist`, `takeover`, `boundary`.
- `rexymcp harvest` flags (`mcp/src/main.rs:315`): `--config <path>` (required),
  `--transcript-dir <path>` (required), optional `--project-id`,
  `--telemetry-path`. Prints `harvested N messages, enriched N activities …`.
  It fills the `tokens` field on already-journaled activities by joining
  transcript usage on the journal time-window (05b).

**The plugin WORKFLOW template is stale.** `plugin/templates/WORKFLOW.md:387-391`
still carries the *old* three-line autonomous-loop paragraph
(`draft -> dispatch -> review`, "stopping only on a blocker or a milestone
boundary"). The repo's own `docs/dev/WORKFLOW.md:445-461` was amended at the M27
kickoff with the full mechanism (skill name, four stop conditions, compose-not-
fork, loop report, journaling/harvest honesty). This phase brings the template
into sync.

## Spec

### 1. Write `plugin/skills/auto/SKILL.md`

Create the new skill. **Frontmatter** (adapt token names to the live docs per
Pre-flight step 5):

```yaml
---
name: auto
description: >
  Run the architect/executor loop hands-off across a milestone — draft,
  dispatch, review, escalate/re-dispatch — with full review rigor and no
  per-phase pause. Opt-in per run; stops at a milestone boundary, a blocker,
  assist-budget exhaustion, or the runaway backstop.
model: opus
argument-hint: "[max-phases]"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), Task
---
```

The body pins the loop. Author it in the same voice as the existing skills
(imperative "you"; numbered procedure sections; a "What you do not do" tail).
It **must** contain the following load-bearing content — the seven sub-blocks
below are the spec for the skill body, not optional guidance:

#### 1a. Composition contract (the load-bearing invariant)

State up front, prominently: **this skill composes the existing four skills; it
never forks them.** Draft runs the `architect` skill's §3 phase-authoring
procedure; dispatch runs the `dispatch` skill; review runs the `review` skill;
escalation runs the `escalate` skill. The loop adds only **sequencing, the
assist budget, journaling, stop conditions, and the loop report**. Any behavior
difference between an interactive run of a step and its autonomous run **is a
bug** (README § Notes; WORKFLOW § "Opt-in autonomous loop"). Do not restate the
DoD walk, the bug-report template, the escalation levers, etc. — invoke the skill
that owns each.

#### 1b. Delegation model (which step runs where, on which model)

Pin the role map exactly (this is why 06a shipped only two keys):

| Step | Runs in | Model |
|---|---|---|
| **draft** | main loop | session model (context-hungry; needs the milestone thread) |
| **dispatch** | **subagent** | `[architect] dispatch_model`, else inherit session model |
| **review** | **subagent** | `[architect] review_model`, else inherit session model |
| **escalate decision** | main loop | session model (needs the briefing + design intent) |
| **refined re-dispatch** (the escalate lever's action) | **subagent** | `dispatch_model`, else inherit |
| **session takeover** | main loop | session model |

`None`/unset role model → **omit the override so the subagent inherits the
session model** (06a's inherit-by-default; do not substitute `[architect] model`,
which is the cost-rate model). A subagent is instructed to *execute the named
skill's procedure* for the given phase — it is a delegated run of that skill, not
a reimplementation. **Degrade rule:** if Pre-flight step 5 finds per-subagent
model override unsupported, run dispatch/review on the session model and state in
the loop report that role-model delegation was unavailable — never claim a model
switch that did not happen (README § honesty; STANDARDS silent-degradation rule).

#### 1c. The loop algorithm

Pin the control flow as an explicit numbered procedure. Shape:

```
Pre-flight:
  - Resolve <repo> (CLAUDE_PROJECT_DIR / ANTIGRAVITY_PROJECT_DIR / nearest
    docs/dev/milestones/ ancestor).
  - Confirm rexymcp.toml has [executor] + [commands]; else point at
    /rexymcp:architect and stop.
  - Call executor_health; if unreachable, stop (do not enter the loop).
  - Read [escalation] max_assists and [architect] dispatch_model/review_model.
  - Parse the optional max-phases arg (the runaway backstop); default 8.
  - Initialize: phases_this_run = 0.

Loop (repeat until a stop condition fires):
  1. DRAFT (main loop): run the architect skill's phase-authoring procedure to
     draft the next phase doc. If it reports the milestone boundary (NEXT.md
     would go to "none" / all in-scope phases done) -> STOP(boundary). Otherwise
     journal `draft`.
  2. assists_this_phase = 0.
  3. DISPATCH (subagent, dispatch_model): run the dispatch skill for the phase.
     Journal `dispatch` with outcome = the returned status.
  4. Branch on the PhaseResult status:
     - complete  -> go to REVIEW (step 5).
     - hard_fail / budget_exceeded -> go to ESCALATE (step 6).
  5. REVIEW (subagent, review_model): run the review skill for the phase.
     Journal `review` with outcome = the verdict.
     - approved -> phases_this_run += 1. If that was the milestone's last
       in-scope phase -> STOP(boundary). Else check the backstop (step 7),
       then continue the loop.
     - bounced (bug filed, phase flipped to in-progress) -> this is a re-dispatch
       round-trip: go to ASSIST (step 6a) with the bounce as the reason.
  6. ESCALATE (main loop): run the escalate skill's lever-choice on the briefing.
     6a. ASSIST accounting: if assists_this_phase >= max_assists -> STOP(budget)
         for this phase. Otherwise assists_this_phase += 1, journal `assist`
         (outcome = the lever or bounce reason), and:
         - refined re-dispatch / resume -> re-dispatch (subagent, dispatch_model);
           go to step 4 with the new result.
         - session takeover -> journal `takeover`, implement in the main loop,
           self-complete to done via the escalate skill's takeover steps;
           phases_this_run += 1; check the backstop (step 7); continue the loop.
     Any blocker / "What Executors Never Decide" item / contract-doc change /
     dependency request / spec-vs-architecture conflict surfaced by any step ->
     STOP(blocker).
  7. BACKSTOP: if phases_this_run >= max-phases -> STOP(runaway).
```

#### 1d. Stop conditions (four, exhaustive)

List them explicitly with what each means and that the loop **always** halts for
the human on each — never auto-continues past one:

- **boundary** — milestone's in-scope phases are all done (or NEXT.md would go to
  "none"). Absolute human gate (architect prohibition #3; WORKFLOW "Milestone
  boundaries are always a human gate"). The loop **never** crosses into the next
  milestone.
- **budget** — `assists_this_phase >= max_assists` on the current phase.
- **blocker** — any blocker, any "What Executors Never Decide" item, a contract-
  doc (STANDARDS/WORKFLOW/architecture) change need, a dependency request, or a
  spec-vs-architecture conflict.
- **runaway** — `phases_this_run >= max-phases` (the backstop; default 8,
  overridable via the invocation arg).

#### 1e. Journaling (exact command, every activity)

Every architect activity in the loop is journaled — this is what makes
`PhaseRun.escalation_count` real and feeds the accounting. After each step,
invoke (project_id defaults from config; pass `--model` = the model that actually
performed the step so per-activity cost uses the role model's rates):

```bash
rexymcp journal --config <repo>/rexymcp.toml \
  --phase-id <phase short id> \
  --phase-doc <abs phase-doc path> \
  --milestone-id <milestone dir slug, e.g. M27-autonomous-escalation-loop> \
  --activity <draft|dispatch|review|assist|takeover|boundary> \
  --outcome <status/verdict/reason> \
  --model <model that performed the step>
```

Use only the six canonical activity strings. The `boundary` activity (§1g) is the
persisted half of the loop report.

#### 1f. Token harvest at stop (Claude Code only, degrade gracefully)

On **every** stop, after journaling the `boundary` activity, attempt a harvest so
the loop-report cost totals are real:

```bash
rexymcp harvest --config <repo>/rexymcp.toml --transcript-dir <Claude Code session transcript dir>
```

The transcript dir is Claude Code's `~/.claude/projects/<slug>/` (05b). If the
client is not Claude Code, or the transcript dir cannot be located, **skip the
harvest** and report token/cost as **absent** in the loop report — never estimate
(README § honesty; the review § 3.4 no-fabrication rule applies architect-side).

#### 1g. The loop report (session output + telemetry record)

On every stop, do **both** (README phase-06 shape decision #2 — no committed
report file):

1. **Journal a `boundary` activity** whose `--outcome` is the stop reason
   (`boundary` / `budget` / `blocker` / `runaway`). This is the persisted,
   queryable half.
2. **Print a structured report** to the session for the human:

   ```markdown
   ## /rexymcp:auto loop report

   - **Milestone:** M<n> — <name>
   - **Stopped:** <boundary | assist budget exhausted on <phase> | blocker: <what> | runaway backstop>
   - **Phases this run:** <N>
     - <phase-id> — <verdict> (assists: <n>)
     - …
   - **Total assists spent:** <N>
   - **Token / cost:** <harvested totals, or "absent — <client> provides no transcript usage">
   - **What needs the human:** <the specific next action — sign off on the boundary, resolve the blocker, raise max_assists, or restart /rexymcp:auto with a larger backstop>
   - **Live view:** `rexymcp status` / `rexymcp dashboard`
   ```

   Point the human at `rexymcp status` / `rexymcp dashboard` for the live view —
   Claude Code sends no MCP progressToken, so a long run is invisible mid-phase
   from the MCP side (README § client-integration; memory
   [[claude-code-no-progress-token]]).

Optionally, a Claude Code-native stop notification is a cheap skill-layer
enhancement — mention it as optional and degrading, not a commitment (README §
client-integration).

#### 1h. "What you do not do" tail

Close with the prohibitions, mirroring the other skills' tails:

- You do **not** cross a milestone boundary — that is always a human gate.
- You do **not** modify STANDARDS.md / WORKFLOW.md / architecture.md, add a
  dependency, or resolve a spec-vs-architecture conflict — those are STOP(blocker).
- You do **not** fork the composed skills — you invoke their procedures unchanged.
- You do **not** fabricate token counts — harvested or absent.
- You do **not** skip the review gate or weaken its rigor because the loop is
  autonomous — the review skill runs verbatim.

### 2. Mirror the autonomous-loop paragraph into the plugin WORKFLOW template

In `plugin/templates/WORKFLOW.md`, **replace** the stale three-line paragraph at
lines 387-391 (the one beginning `**Opt-in autonomous loop (off by default).**
For hands-off runs, the user may turn on an autonomous mode that chains draft ->
dispatch -> review across phases,`) with the **content** of the amended paragraph
from `docs/dev/WORKFLOW.md` § "Opt-in autonomous loop (off by default)" (the one
naming `/rexymcp:auto`, the four stop conditions, compose-not-fork, the loop
report, and the journaling/harvest honesty constraint).

**Render it in the template's house style:** the template uses **ASCII arrows**
(`->`), not the repo's Unicode `→`; convert the arrows when mirroring. This is a
content mirror, not a byte copy — the paragraph must say the same things; adapt
punctuation glyphs to the template's existing convention. Do not touch any other
part of the template.

## Acceptance criteria

- [ ] `plugin/skills/auto/SKILL.md` exists with valid YAML frontmatter
      (`name: auto`, a `description`, `model`, `argument-hint`, `allowed-tools`)
      and a body covering all seven sub-blocks (1a–1h): composition contract,
      delegation model, loop algorithm, four stop conditions, journaling command,
      harvest-at-stop, and the loop report.
- [ ] The skill body pins the delegation role map: draft/escalate/takeover in the
      main loop; dispatch/review (and refined re-dispatch) in subagents on
      `dispatch_model`/`review_model`; unset → inherit the session model, **not**
      `[architect] model`.
- [ ] The skill uses only the six canonical activity strings (`draft`,
      `dispatch`, `review`, `assist`, `takeover`, `boundary`) and the real
      `rexymcp journal` / `rexymcp harvest` flag names from § Current state.
- [ ] The four stop conditions (boundary, budget, blocker, runaway) are each
      documented with their trigger, and the skill states the milestone boundary
      is an absolute human gate.
- [ ] The loop report is specified as both a printed session report **and** a
      `boundary` journal record — no committed report file.
- [ ] `plugin/templates/WORKFLOW.md`'s autonomous-loop paragraph now names
      `/rexymcp:auto`, lists the four stop conditions, states compose-not-fork,
      and describes the loop report + journaling/harvest honesty — matching the
      content of `docs/dev/WORKFLOW.md` § "Opt-in autonomous loop", rendered with
      the template's ASCII arrows.
- [ ] `rexymcp journal` accepts each of the six activity kinds against a temp
      telemetry path and appends a record (E2E below).
- [ ] No Rust source changed; no dependency added; `docs/architecture.md`
      untouched (the design is already recorded in the milestone README).

## Test plan

This phase ships **prose artifacts** (a skill + a template paragraph), not Rust —
there are no unit tests to add (STANDARDS §3.2: prose/docs are not unit-tested).
Verification is by the E2E block and by inspection against the acceptance
criteria. Do **not** invent Rust tests for the skill.

## End-to-end verification

The skill's composed CLIs are real artifacts — verify them against the running
binary, and quote the output in the completion Update Log:

1. **Journal round-trip for every activity kind.** Against a throwaway telemetry
   path, run `rexymcp journal … --activity <kind>` for each of the six kinds and
   confirm each prints `recorded <kind> activity for <phase-id> -> <path>` with no
   `unknown activity` warning. Example:

   ```bash
   for k in draft dispatch review assist takeover boundary; do
     rexymcp journal --config <repo>/rexymcp.toml --phase-id phase-06b \
       --milestone-id M27-autonomous-escalation-loop --activity "$k" \
       --outcome test --telemetry-path /tmp/rexymcp-auto-e2e/phase_runs.jsonl
   done
   ```

   Confirm the file now holds six `architect_activity` records (one per kind) with
   no warning line. **Use a throwaway `--telemetry-path`, never the project's real
   telemetry dir** (hermetic — do not pollute real telemetry with test records).

2. **Frontmatter validity.** Confirm `plugin/skills/auto/SKILL.md` parses as valid
   YAML frontmatter (same shape as the other four skills — compare against
   `plugin/skills/dispatch/SKILL.md`). Quote the frontmatter block.

3. **Template mirror.** `grep -n "/rexymcp:auto" plugin/templates/WORKFLOW.md`
   returns the mirrored paragraph; confirm by inspection it lists the four stop
   conditions and the compose-not-fork invariant.

A full live `/rexymcp:auto` run over a milestone is **not** part of this phase's
E2E (it is not hermetic and would consume a real milestone) — it is exercised on
the next real milestone the user runs autonomously. Note this explicitly in the
completion entry; the composed pieces are each individually verified above.

## Authorizations

- [x] May edit `plugin/templates/WORKFLOW.md` — the plugin-template mirror is the
      explicit deliverable of this phase (README kickoff note: "The plugin-template
      mirror of WORKFLOW.md is updated in phase-06 alongside the skill it
      describes"). This is the **template**, not the repo's `docs/dev/WORKFLOW.md`
      (which stays untouched — it was already amended at kickoff).
- [x] May create `plugin/skills/auto/SKILL.md` (a new file — the phase requires it).
- No new dependency. No `docs/architecture.md` edit. No `docs/dev/WORKFLOW.md`,
  `docs/dev/STANDARDS.md`, `clippy.toml`, `rustfmt.toml`, or CI edit.

## Out of scope

- Any Rust change. The config keys (06a), the journal CLI (02), and the harvester
  (05b) are all already shipped; this phase only *consumes* them from prose. If
  the skill seems to need a new CLI flag or a Rust behavior change, that is a
  **blocker**, not an in-phase edit.
- Advisory executor-model routing in dispatch — that is the phase-07 stretch row.
- A dedicated Rust loop-report record type — the shape decision (README #2) is
  that the loop report persists as the existing `boundary` `ArchitectActivity`
  plus printed session output; do not add a new telemetry record.
- Editing the other four skills. If composition reveals one of them needs a change
  to be loop-safe, file a blocker (it would be a shared-contract change) — do not
  edit it inline.
- `docs/dev/WORKFLOW.md` (the repo's own copy) — already amended at kickoff; only
  the *plugin template* is mirrored here.

## Update Log

(Filled in by the executor — here, the architect authoring directly.)

<!-- entries appended below this line -->

### Update — 2026-07-09 (complete, Claude Code direct)

**Executor:** Claude Code (direct) — direct-execution phase per the doc header.

**Summary:** Created `plugin/skills/auto/SKILL.md` (the `/rexymcp:auto` loop
skill) and mirrored the amended autonomous-loop paragraph into
`plugin/templates/WORKFLOW.md`. The skill body covers all seven sub-blocks:
the compose-never-fork invariant, the delegation role map (draft/escalate/
takeover in the main loop; dispatch/review/refined-re-dispatch in `Agent`
subagents on `dispatch_model`/`review_model`, inherit-by-default), the loop
algorithm, the four stop conditions (boundary/budget/blocker/runaway), the
exact `rexymcp journal` command + six canonical activity kinds, the
harvest-at-stop with graceful degrade, and the loop report (printed session
output + a `boundary` journal record — no committed report file). No Rust
changed; no dependency added; `docs/architecture.md` and `docs/dev/WORKFLOW.md`
untouched.

**Pre-flight step 5 finding (external-API verify) — divergence recorded:** the
draft sketch assumed the subagent-spawning tool was `Task` with a statically-
pinned model. The live Claude Code subagent docs
(`code.claude.com/docs/en/subagents.md`) confirm: (1) the tool is **`Agent`**
(`Task` is a deprecated backwards-compat alias) — the skill's `allowed-tools`
uses `Agent`; (2) **per-invocation dynamic model override IS supported** — the
orchestrator passes a `model` parameter on each `Agent` call, read from
`rexymcp.toml` at runtime (resolution order: `CLAUDE_CODE_SUBAGENT_MODEL` env >
per-call `model` param > subagent frontmatter > session model), so the
role-model delegation works as designed with no static-pin fallback needed;
(3) a subagent is told to run a composed skill by giving it the `Skill` tool
and instructing it in the prompt. The degrade rule (run on the session model +
say so in the loop report) is retained for clients/versions without per-call
override. This adaptation was resolvable from the live docs, so it was adapted
cleanly (not blocked) and is recorded here per the declare-deviations
discipline.

**Acceptance criteria:** all met (verified in the E2E block below).

**Commands (independent re-run — no Rust changed, gates confirm no regression):**

```
cargo fmt --all --check     → clean (no diff)
cargo build                 → Finished, zero warnings
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test                  → 928 executor + 483 mcp passed, 2 ignored
```

**End-to-end verification:**

1. **Journal round-trip, all six activity kinds** (throwaway `--telemetry-path
   /tmp/rexymcp-auto-e2e/phase_runs.jsonl`, hermetic — real telemetry untouched):

   ```
   recorded draft activity for phase-06b -> /tmp/rexymcp-auto-e2e/phase_runs.jsonl
   recorded dispatch activity for phase-06b -> …
   recorded review activity for phase-06b -> …
   recorded assist activity for phase-06b -> …
   recorded takeover activity for phase-06b -> …
   recorded boundary activity for phase-06b -> …
   ```

   File holds 6 `architect_activity` records (one per kind); **no `unknown
   activity` warning** on any — confirms the skill's six pinned strings match the
   `ARCHITECT_ACTIVITIES` vocabulary against the real binary.

2. **Frontmatter validity:** `plugin/skills/auto/SKILL.md` parses as valid YAML
   with the same key set as the other four skills (`allowed-tools`,
   `argument-hint`, `description`, `model`, `name`); `name: auto`, `model: opus`,
   `allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), Agent`.

3. **Template mirror:** `grep -n "/rexymcp:auto" plugin/templates/WORKFLOW.md`
   returns the mirrored paragraph; it names the four stop conditions
   ("milestone boundary (always)", "What Executors Never Decide", "assist
   budget", "runaway backstop"), states **composes** (not forks), and describes
   the **loop report** — all in the template's ASCII-arrow (`->`) house style.

   A full live `/rexymcp:auto` run over a milestone is **not** hermetic and is
   deliberately out of this phase's E2E — it is exercised on the next real
   milestone the user runs autonomously; the composed pieces are each verified
   above.

**Files changed:**
- `plugin/skills/auto/SKILL.md` — new; the `/rexymcp:auto` loop skill.
- `plugin/templates/WORKFLOW.md` — autonomous-loop paragraph mirrored from the
  repo's amended `docs/dev/WORKFLOW.md`.

**New tests:** none — prose artifacts (a skill + a template paragraph); STANDARDS
§3.2 exempts docs from unit tests. Verified by the E2E block + inspection.

**Notes for review:** the `Task`→`Agent` divergence above is the one adaptation
from the draft sketch; everything else landed as specified. The skill reuses the
exact `rexymcp journal`/`harvest` flag names and the six activity strings pinned
in the phase doc's § Current state.

### Review verdict — 2026-07-09

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude Code (direct) — planned direct-execution (prose skill
  orchestrating Claude Code subagents; not dispatchable to the local-LLM executor)
- **Scope deviations:** none — both tasks (the `auto` skill; the WORKFLOW template
  mirror) landed in full; nothing cut or deferred.
- **Calibration:** none folded. One external-API adaptation (data, not a fold):
  the draft sketch's `Task` subagent-tool assumption was corrected to `Agent`
  with a verified per-call `model` override, resolved cleanly from the live
  Claude Code subagent docs via the phase's Pre-flight step 5 — the
  external-API-verify discipline working as intended (WORKFLOW § "Verify external
  APIs against live docs"). All four gates green on independent re-run (483 mcp +
  928 executor, 2 ignored); no Rust changed, so no regression surface.

### Update — 2026-07-09 (post-approval refinement — test-driven, user-approved)

A partial live dry-run of `/rexymcp:auto` (orchestrating a dispatch against the
brainyscript e2e subject) surfaced two skill gaps, folded into
`plugin/skills/auto/SKILL.md` with the user's approval. Neither re-opens the
phase — both are additive prose refinements to the shipped skill:

1. **Entry with an already-active phase.** The loop's step 1 assumed it always
   drafts first; entering when `NEXT.md` already points at a `todo`/`in-progress`
   phase (a prior interactive draft, or a bounce left `in-progress`) would have it
   draft the *wrong* next phase. Reworked step 1 to **DRAFT-or-adopt**: adopt an
   existing active phase and dispatch it (no `draft` journal) before ever drafting
   a new one.
2. **`execute_phase` root corroboration undocumented.** The live dispatch failed
   with MCP `-32602` — the server refuses a `repo_path` that does not corroborate
   against the session's MCP roots / `CLAUDE_PROJECT_DIR`, so the loop **must run
   from the target repo's own Claude Code session**. Added a Pre-flight bullet
   asserting this and instructing the skill to stop-and-redirect when `<repo>` is
   not the session root. (This is why the dry-run from the rexyMCP-rooted session
   could not dispatch to brainyscript — expected, correct server behavior, not a
   loop bug.)

Also validated during testing: the `Agent`-subagent per-call model override and
MCP-tool access from delegated subagents both work in practice (proven by a live
subagent on `claude-sonnet-5` calling `executor_health`); the six-kind `rexymcp
journal` round-trip; and the server's root-corroboration guard returning a clean
actionable error rather than crashing. A full live loop (real gemma dispatch →
review) remains to be run from a brainyscript-rooted session.
