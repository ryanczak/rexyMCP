# Phase 05: dispatch + review + escalate skills

**Milestone:** M6 — Plugin + architect/review skills
**Status:** review
**Depends on:** M6 phase-01 (done) — `plugin/skills/{dispatch,review}/SKILL.md` stubs exist; the architect skill scaffold pattern is established. M6 phase-04 (done) — sets the SKILL.md frontmatter conventions, `/rexymcp:` namespacing, `allowed-tools` pattern syntax, and the load-bearing-prose-pre-injection technique.
**Estimated diff:** ~600 lines (three SKILL.md files; no Rust code)
**Tags:** language=markdown, kind=feature, size=l

## Goal

Fill in the remaining two skill stubs and create the third skill that
phase-01 didn't scaffold:

1. **`plugin/skills/dispatch/SKILL.md`** — replaces the phase-01 stub.
   Thin glue: invoke `execute_phase` via the MCP tool with the phase doc
   path + repo path; surface the returned `PhaseResult` (or the
   `hard_fail` briefing) to the user; suggest `/rexymcp:review` next.
2. **`plugin/skills/review/SKILL.md`** — replaces the phase-01 stub. The
   substantive one: re-run the project's command set, check against
   `STANDARDS.md`'s DoD, spot-check tests are real, **write the Review
   verdict + flip status to `done`** on pass / **file a bug + flip
   status back to `in-progress`** on fail. On `hard_fail` results,
   delegate to `/rexymcp:escalate` rather than review.
3. **`plugin/skills/escalate/SKILL.md`** — net-new skill directory.
   Given a `hard_fail` briefing, choose a lever per the architecture's
   escalation policy: **refined re-dispatch** (default for weak models),
   **session takeover** (last resort), or **resume** (future). The
   decision tree carries judgment-heavy content that gets pre-injected
   per the phase-04 pattern.

After this phase, the architect → dispatch → review → (escalate-on-fail)
cycle is fully wired in skills. Phase-06 is the dogfood that exercises
the whole chain against a real third-party repo.

## Architecture references

- `docs/architecture.md` — Layer 3 "Plugin package":
  - The `review-phase` skill bullet ("check executor output against the
    Definition of Done in `STANDARDS.md`, rerun the project's commands,
    then approve or file a bug").
  - The `escalate` skill bullet ("given a returned briefing, pick a
    lever: re-dispatch with a refined spec (default for weak models —
    see 'Escalation'), session takeover, or resume").
  - "Escalation = Claude Code itself" section — the strategic argument
    for refined re-dispatch as the default.
  - "End-to-end flow" — the architect → `/rexymcp:dispatch` → executor
    → `/rexymcp:review` cycle.
- M6 phase-04: SKILL.md frontmatter conventions, `/rexymcp:` namespacing,
  `allowed-tools` syntax, the load-bearing-prose-pre-injection pattern.
- M5 phase-02: `execute_phase` MCP tool signature + returned
  `PhaseResult` shape (capped) + the `log_path` for drill-down.
- M5 phase-03: `executor_log_search` / `executor_log_tail` / `get_turn`
  log-query tools — the review skill uses these when the capped
  `PhaseResult` doesn't carry enough detail.
- M4 phase-06: `PhaseResult { status: complete | hard_fail |
  budget_exceeded, … }` + `briefing: Option<Briefing>` (present on
  failure).
- WORKFLOW.md § Review and Bug-Report Cycle — the review verdict + bug
  report templates. Both skills reference these.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M6 README.
2. Read this entire phase doc.
3. **Verify Claude Code's skill-to-skill invocation convention.** The
   review skill needs to *delegate* to `/rexymcp:escalate` when a
   `hard_fail` is observed. Does Claude Code let one skill invoke
   another (e.g. by suggesting the user run a different slash command),
   or does it support a structured "transfer to skill X" handoff?
   Verify via Claude Code docs / `claude-code-guide` Agent / working
   examples. **Trust docs over the sketch** (per WORKFLOW.md "Verify
   external APIs against live docs"); flag in Notes for review.
4. **Verify how MCP tool results reach the skill body.** When `/rexymcp:
   dispatch` invokes `execute_phase`, the result returns through the MCP
   tool-call mechanism. Confirm Claude can read it directly from the
   tool response (likely yes), and whether the progress notifications
   M5 phase-05b emits surface to the user automatically vs requiring
   explicit handling in the skill prompt.
5. Confirm the existing phase-01 stubs at `plugin/skills/dispatch/
   SKILL.md` and `plugin/skills/review/SKILL.md`. Confirm
   `plugin/skills/escalate/SKILL.md` does **not** yet exist (phase-01
   only scaffolded the three slash-command-named skills, not escalate
   — see § 1 below).
6. Confirm the architect skill (phase-04) uses `/rexymcp:` namespacing
   throughout — the dispatch/review/escalate skills must match.

## Spec

### 1. The escalate skill needs a new directory

Phase-01 created `plugin/skills/{architect,dispatch,review}/` —
matching the three named slash commands in architecture. The `escalate`
skill was named in architecture as the third *skill* but not as a fourth
*slash command*. Phase-05 **creates `plugin/skills/escalate/`** + its
`SKILL.md`. Claude Code's plugin convention (per phase-01 + phase-04
findings) auto-derives the slash-command form from the skill name, so
the skill is implicitly invokable as `/rexymcp:escalate` even though
the architecture's "commands" list emphasizes only the three common
ones.

### 2. dispatch skill — `plugin/skills/dispatch/SKILL.md`

**Replace the phase-01 stub.** Thin glue around `execute_phase`.

#### Frontmatter

```yaml
---
name: dispatch
description: >
  Dispatch a phase to the local-LLM executor via execute_phase. Use after
  /rexymcp:architect has drafted a phase doc and the user is ready to run it.
model: sonnet
argument-hint: "<phase>"
allowed-tools: Read, Bash(*)
---
```

`model: sonnet` — dispatch is mechanical (parse arg → invoke tool →
report result); opus is overkill. `allowed-tools` is minimal —
dispatch reads the phase doc to confirm context and may shell out to
inspect the working tree, but does not edit files. **MCP tools
(`execute_phase`, `executor_health`) come through `.mcp.json`
registration, not `allowed-tools`** (per phase-04's finding).

#### Body sections

1. **Read these first** — read `<repo>/docs/dev/NEXT.md` to confirm the
   active phase, read the phase doc itself to confirm it's `todo`, read
   `<repo>/rexymcp.toml` to confirm bootstrap is complete.
2. **Pre-flight: executor reachability** — invoke `executor_health`
   (optional `base_url` override). If unreachable, surface the error
   and stop; do not invoke `execute_phase` against a dead endpoint.
3. **Invoke `execute_phase`** — args: `phase_doc_path` (resolve from
   `<phase>` arg + the milestone path convention), `repo_path` (the
   target repo root via `CLAUDE_PROJECT_DIR` / Claude Code's roots).
   Pass `model` override if the user supplied one.
4. **While running** — Claude Code surfaces MCP progress notifications
   to the user automatically (per phase-04 Pre-flight 4 finding). The
   skill prompt doesn't need to manage them; just note in the prompt
   that the user will see them.
5. **On return: `complete`** — surface the `PhaseResult` summary
   (status, files_changed, command_outputs tails, log_path) to the
   user. Suggest `/rexymcp:review <phase>` as the next step. Do **not**
   review here — that's a separate skill with a separate verdict.
6. **On return: `hard_fail`** — surface the briefing's `one_line`,
   `current_blocker`, `what_was_tried`, `diagnostics`. Suggest
   `/rexymcp:escalate <phase>` as the next step. Do **not** decide
   the escalation lever here.
7. **On return: `budget_exceeded`** — same shape as `hard_fail` (a
   briefing is returned). Suggest `/rexymcp:escalate <phase>`.

The skill ends after surfacing the result. It does not loop, does not
re-dispatch, does not advance to review. Each step is a user gate.

### 3. review skill — `plugin/skills/review/SKILL.md`

**Replace the phase-01 stub.** The substantive skill.

#### Frontmatter

```yaml
---
name: review
description: >
  Review a completed phase against STANDARDS.md DoD, rerun the project's
  commands, and either approve (flip to done) or file a bug.
model: opus
argument-hint: "<phase>"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*)
---
```

`model: opus` — review requires judgment (is the code actually right? are
the tests real? does the design hold?). `allowed-tools` includes Write
+ Edit because review writes the verdict (or a bug doc) and flips
status lines.

#### Body sections

1. **Read these first** — the phase doc (status should be `review`),
   `STANDARDS.md` (the DoD checklist), the milestone README (phase
   table), and the returned `PhaseResult` (the dispatch skill or the
   user passed it; if not available in context, query via
   `executor_log_tail` + `get_turn` MCP tools using the phase's
   `log_path`).
2. **Refuse non-`review`-status phases** — if the phase doc's status is
   not `review`, stop and tell the user. Don't review a `todo` or
   `in-progress` phase.
3. **Refuse `hard_fail` / `budget_exceeded` results** — if the
   `PhaseResult.status` is not `complete`, this is an escalation, not
   a review. Point the user at `/rexymcp:escalate <phase>` and stop.
4. **Re-run the command set** — read `<repo>/rexymcp.toml`'s
   `[commands]` section, run `format` / `build` / `lint` / `test` in
   sequence (separate invocations — the WORKFLOW fold pinned this), and
   confirm each passes. **The executor's `PhaseResult.command_outputs`
   is the executor's run; this is the architect's independent re-run.**
   If any differs (executor passed, you fail), surface it as a possible
   environment-vs-spec mismatch.
5. **Walk the DoD checklist** (`STANDARDS.md` §1) — for each box,
   verify it's met by inspecting the diff, the phase doc, the test
   output. Pay extra attention to:
   - No `unwrap()` / `expect()` / `panic!()` in production paths (or
     the language equivalent) — grep for them.
   - No `#[allow]` / language-equivalent to mask diagnostics.
   - No `TODO` / `FIXME` / debug calls in the new code.
   - New code is covered by tests (per `STANDARDS.md` §3).
6. **Spot-check tests are real** — pick one or two new tests; confirm
   they would actually fail if the code under test were broken
   (sometimes by deleting an assertion mentally and verifying the test
   would still pass — that's a fake test).
7. **Walk the phase doc's Acceptance criteria** — every checkbox in the
   phase doc's Acceptance criteria section should be verifiable; verify
   each one.
8. **On pass:**
   a. Write the **Review verdict** block (per `WORKFLOW.md` § Review
      and Bug-Report Cycle) at the bottom of the phase doc's Update
      Log. Even for `approved_first_try`, write the one-line entry —
      this is the supervision label that turns telemetry into eval.
   b. Flip the phase doc's `Status:` line from `review` to `done`.
   c. Update the milestone README's phase-table row to `done`.
   d. Update `<repo>/docs/dev/NEXT.md` (active phase → "none" if
      this was the milestone's last in-scope phase; otherwise leave
      for `/rexymcp:architect next`).
   e. Commit (conventional commit: `docs: approve <milestone> <phase>
      (done, <verdict>)`).
   f. **Stop.** Do not draft the next phase. The user explicitly
      advances via `/rexymcp:architect next`. (Per phase-04 §6
      prohibitions: no auto-advance.)
9. **On fail:**
   a. Write a bug report (per `WORKFLOW.md` § Bug report template) at
      `<repo>/docs/dev/milestones/M<n>-<slug>/bugs/bug-<phase>-<n>.md`.
   b. Flip the phase doc's `Status:` line from `review` back to
      `in-progress` (with the bug reference).
   c. Update the milestone README's phase-table row to `in-progress`.
   d. Commit (conventional commit: `docs: bounce <milestone> <phase>
      — bug-<n>-<n> (<short summary>)`).
   e. Tell the user: "Bounced. Re-dispatch via `/rexymcp:dispatch
      <phase>` once the bug is fixed."
   f. **Stop.** Do not fix the bug yourself; that's the executor's job.
10. **On milestone close** (this phase was the milestone's last
    in-scope phase, all sibling phases `done`): also write the
    milestone retrospective in the README's Notes section, fold any
    calibration lessons into WORKFLOW.md (with user sign-off), and
    update NEXT.md to "none". The user kicks off the next milestone.

### 4. escalate skill — `plugin/skills/escalate/SKILL.md`

**Create the directory + file** (phase-01 didn't scaffold this one).

#### Frontmatter

```yaml
---
name: escalate
description: >
  Decide what to do with a hard_fail briefing from execute_phase: refined
  re-dispatch (default), session takeover, or resume (future).
model: opus
argument-hint: "<phase>"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(*), WebFetch, WebSearch
---
```

`model: opus` — escalation is the *most* judgment-heavy decision in the
workflow. Wrong default (session takeover when refined re-dispatch
would have worked) burns the architect-executor split. `allowed-tools`
is broad because refined re-dispatch may require fetching new reference
docs, editing the phase doc, etc.

#### Body sections

1. **Read these first** — the briefing (from the returned `PhaseResult.
   briefing`), the phase doc, `STANDARDS.md`, the session log via
   `executor_log_tail` + `get_turn` (drill into specific turns the
   briefing references), the bug docs in the milestone's `bugs/`
   directory if any are open.
2. **Refuse non-failure results** — if `PhaseResult.status == complete`,
   this is a review, not an escalation. Point the user at `/rexymcp:
   review` and stop.
3. **Decision tree — choose a lever** (pre-injected per § 5 below).
4. **Execute the chosen lever:**
   - **Refined re-dispatch:** amend the phase doc's Spec or Pre-flight
     based on the briefing's "what was tried" + "current blocker" +
     "diagnostics". Add a `Notes for executor` block at the top of the
     Update Log explaining the refinement. Tell the user: "Refinement
     applied. Re-dispatch via `/rexymcp:dispatch <phase>`."
   - **Session takeover:** flip the phase doc's `Status:` to
     `in-progress (architect takeover)` with a one-line note. Implement
     the phase directly using your file-edit tools. Run the command set
     yourself. On completion, write the Review verdict with
     `Executor: Claude (direct)` and `Verdict: escalated`. Flip to
     `done`. Tell the user: "Phase completed via session takeover."
   - **Resume:** not yet supported (no `continue_phase` MCP tool).
     Tell the user this lever is reserved for future work and pick one
     of the other two.
5. **Always: write the escalation outcome** to the phase doc's Update
   Log as a `### Update — YYYY-MM-DD HH:MM (escalation)` entry naming
   the chosen lever and why.

### 5. Pre-injected decision tree — `plugin/skills/escalate/SKILL.md`

> The escalation decision tree is judgment-heavy and the rationale is
> non-obvious. The architect-supplied draft prose below should be
> integrated as the **core of the decision-tree section** — adapt
> connective tissue, but preserve the voice, the specific examples,
> and the framing of "refined re-dispatch is the default for a reason."

```markdown
### Choosing a lever

A `hard_fail` briefing is a signal: the executor reached the budget,
hit a diagnostic it couldn't resolve, lost track of state, or
otherwise stopped without producing a clean `PhaseResult`. The
escalation question is **what changes** so the next attempt succeeds.

Three levers, in order of preference:

1. **Refined re-dispatch — the default for weak models.** The local
   executor is a smaller LLM than you are; it lacks web access; it
   cannot ask clarifying questions mid-phase. *Most* `hard_fail`s
   trace back to a spec gap the executor couldn't bridge, not to an
   executor mistake the executor should have avoided.

   Diagnostic: read the briefing's `what_was_tried` list and ask
   "would a tighter spec have prevented this?" If yes (and most of
   the time, yes), refine and re-dispatch.

   Common refinements that turn `hard_fail` into `approved_first_try`:
   - **Add a worked example** the executor was missing — they were
     trying to invent something instead of pattern-matching.
   - **Pin a negative case** (per WORKFLOW.md "Pin negative cases") —
     the executor satisfied the positive examples but tripped the
     boundary case.
   - **Quote an API doc inline** instead of linking to it — the
     executor couldn't reach the link.
   - **Authorize a narrow upstream edit** the executor needed but
     wasn't permitted (per WORKFLOW.md "Anticipate cross-boundary
     trait bounds").
   - **Verify an external-API claim** (per WORKFLOW.md "Verify
     external APIs against live docs") — the architect's sketch was
     stale and the executor lost time trying to make it work.

   This lever is cheap (one model call) and produces telemetry
   (`PhaseRun.bounces_to_approval` increments by 1). The architect
   learns; the executor learns by re-trying with better inputs; the
   `model_scorecard` accumulates a real data point on bug-class-to-fix
   ratios.

2. **Session takeover — last resort.** You (Claude) take over and
   implement the phase directly. Use this when:
   - You've already done one refined re-dispatch and the same class
     of failure recurred (signaling the executor genuinely can't reach
     this work, not a spec gap).
   - The briefing reveals the executor lost track of state in a way
     that a re-dispatch would just re-encounter (e.g. ran out of
     context budget on a phase that's too big for any spec
     refinement).
   - The phase is on the critical path and the user is time-pressed.

   Cost: **the telemetry gap.** When you implement the phase, the
   `PhaseRun.architect_verdict` records `escalated` instead of an
   `approved_*` from a model — you produce a successful artifact but
   *no* model-vs-spec data point. The `model_scorecard` is blind to
   the run. Use sparingly.

   When you do take over, flip the phase's `Status:` to
   `in-progress (architect takeover)` with a one-line note, do the
   work, write the Review verdict yourself (`Executor: Claude
   (direct)`, `Verdict: escalated`), flip to `done`. The artifact is
   real; the data is missing.

   **Anti-pattern: skipping refined re-dispatch because "this case
   feels special."** Every hard_fail feels special to the architect
   reading the briefing. That's exactly why the discipline matters
   most when it feels least convenient. If you find yourself jumping
   to takeover on the first failure, **slow down**: read the
   briefing's `what_was_tried` carefully, ask "what would a tighter
   spec change?", try the refinement once. The data is what makes
   the model scorecard real over time.

3. **Resume — not yet implemented.** A `continue_phase` MCP tool that
   resumes a failed phase from a checkpoint is a possible future
   addition (the M4 session log makes the prerequisites available),
   but does not exist today. If a hard_fail reads like "we were 90%
   done and just ran out of turns," refined re-dispatch with a
   tightened spec (smaller scope, more pre-injection) is still the
   right call. Note the resume question in calibration if it recurs;
   fold a new milestone if the pattern hardens.

### Decision summary

| Failure shape | First-attempt lever |
|---|---|
| Spec gap (missing example, unclear acceptance, missed authorization) | Refined re-dispatch |
| External API drift (architect's sketch was stale) | Refined re-dispatch with verified docs |
| Boundary / negative case the spec didn't pin | Refined re-dispatch with pinned negative |
| Repeated same-class failure after one refinement | Session takeover |
| Context-budget exhaustion on a phase that's already minimal | Session takeover (or re-split into two phases) |
| Anything that feels special | Refined re-dispatch — feeling-special is not a lever |
```

### 6. Format conventions

Same as phase-04:
- Top-level section headings (`##`) for each numbered responsibility so
  a future skill reader can locate guidance by topic.
- Concrete examples (`like this:` + code block) over abstract
  principles.
- No rexyMCP-internal references except the legitimately-rexyMCP ones
  (skill name, server reference, the embedded-contract architecture
  note).
- `/rexymcp:` namespacing throughout.

## Adaptations / decisions

1. **Three skills, three SKILL.md files.** `dispatch` + `review` replace
   phase-01 stubs; `escalate` is net-new. Phase-01 didn't scaffold
   escalate because the architecture lists only three slash commands,
   but architecture lists three *skills* — the fourth implicit
   `/rexymcp:escalate` follows from Claude Code's skill-name →
   slash-command auto-derivation (phase-01 + phase-04 findings).
2. **Models per skill:** dispatch = sonnet (mechanical); review = opus
   (judgment); escalate = opus (most judgment-heavy of the three).
3. **`/rexymcp:` namespacing** matches phase-04 (and the architect-side
   follow-up that updated M6 README + architecture.md).
4. **Pre-injection for the escalate decision tree** follows the
   phase-04 pattern. The other two skills are mechanical enough that
   the spec's prose suffices.
5. **Review skill writes the verdict + flips status** (per phase-04 §5
   status-management). The dispatch skill never touches phase status —
   it just surfaces what came back.
6. **Review skill bounces on `hard_fail` rather than reviewing.** Mixed
   results don't get a verdict; they get an escalation. The
   `PhaseResult.status` is the discriminator.
7. **Session takeover keeps the artifact, loses the data.** The
   pre-injected decision tree makes this trade-off explicit so the
   architect doesn't bypass refined re-dispatch reflexively.
8. **No new dependencies. No Rust code.**

## Acceptance criteria

- [ ] `plugin/skills/dispatch/SKILL.md` exists with the phase-01 stub
      fully replaced. Frontmatter follows § 2 (model: sonnet,
      argument-hint: `<phase>`, allowed-tools: minimal). Seven body
      sections covering read-these-first / pre-flight executor_health
      / invoke execute_phase / progress notifications / on-complete /
      on-hard_fail / on-budget_exceeded.
- [ ] `plugin/skills/review/SKILL.md` exists with the phase-01 stub
      fully replaced. Frontmatter follows § 3 (model: opus,
      allowed-tools: Read/Write/Edit/Glob/Grep/Bash(\*)). Ten body
      sections covering read / refuse-non-review-status /
      refuse-hard_fail / re-run command set / DoD checklist /
      spot-check tests / acceptance criteria / on-pass / on-fail /
      on-milestone-close.
- [ ] `plugin/skills/escalate/SKILL.md` exists in the **net-new
      directory** `plugin/skills/escalate/`. Frontmatter follows § 4
      (model: opus, allowed-tools broad). Five body sections plus the
      pre-injected decision tree.
- [ ] **Escalate's pre-injected decision tree is integrated with
      voice + framing + the three-levers ordering preserved.** The
      "refined re-dispatch is the default for weak models" rationale,
      the "session takeover loses telemetry" trade-off, the
      "anything that feels special → refined re-dispatch" pin all
      appear substantially in the architect's words. Adapt connective
      tissue; preserve load-bearing content. Flag any rewrite in Notes
      for review.
- [ ] All three skills use `/rexymcp:` namespacing for cross-references
      (no bare `/dispatch`, `/review`, `/escalate`, `/architect`).
- [ ] Review skill correctly handles the three `PhaseResult.status`
      outcomes (`complete` → review path; `hard_fail` → delegate to
      escalate; `budget_exceeded` → delegate to escalate).
- [ ] Dispatch skill correctly handles the same three outcomes
      (surfaces each; suggests next-step skill; never decides for the
      user).
- [ ] Escalate skill enforces the **refused-result guard**: if
      `PhaseResult.status == complete`, point the user at
      `/rexymcp:review` and stop (escalate isn't for clean results).
- [ ] **Status-management discipline preserved** (per phase-04 §5 +
      §6): the review skill flips status + writes verdict + commits;
      the dispatch skill never touches status; the escalate skill
      flips status only when taking over.
- [ ] **No auto-advance** (per phase-04 §6 prohibition #2): every
      skill ends with "stop" / "the user advances explicitly." No
      skill silently chains into another.
- [ ] **Validation greps** (per the M6-calibrated patterns):
  - `grep -rnE '[Rr]exy[Mm][Cc][Pp]' plugin/skills/{dispatch,review,
    escalate}/SKILL.md` — only legitimate refs (server name, MCP tool
    names like `rexymcp serve`, slash-command namespacing). Count
    pinned in Notes for review.
  - `grep -rnE 'opencode|Rexy(?!MCP)|cargo |Cargo\.toml' plugin/skills/
    {dispatch,review,escalate}/SKILL.md` — zero hits (the three new
    skills don't have a bootstrap-step exception like the architect
    skill does).
- [ ] **Read-pass gut-check** (the M6 bug-02-1 lesson — grep alone
      isn't enough for content): pretend to be a fresh Claude invoked
      against each of the three new skills in turn, walking through the
      common-case scenarios:
  - `/rexymcp:dispatch phase-01` against a `todo` phase with a
    reachable endpoint → confirm you know to run health-check,
    invoke execute_phase, and what to do with each return shape.
  - `/rexymcp:review phase-01` against a `review`-status phase with a
    `complete` result → confirm you know to run commands, walk DoD,
    write verdict, flip status.
  - `/rexymcp:escalate phase-01` against a `hard_fail` result →
    confirm you know to read the briefing, walk the decision tree,
    choose a lever, execute, and not skip refined re-dispatch
    reflexively.
- [ ] No Rust code changes. No new dependencies. The four `cargo` gates
      pass unchanged.
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in Notes for review. The phase-04 self-review accuracy
      bar holds.

## Test plan

Same shape as phase-04: no Rust code → no automated tests beyond the
four `cargo` gates (which remain green by construction since no Rust
changes).

Content validation:
1. Run the validation greps from acceptance criteria. Pin output in the
   Update Log's Commands block.
2. **Section-presence audit:** `grep '^## ' plugin/skills/{dispatch,
   review,escalate}/SKILL.md` confirms each spec'd section landed.
3. **Read-pass gut-check** per the acceptance criterion above.

No `#[cfg(test)]` blocks (no Rust). End-to-end exercise is phase-06
(dogfood).

## End-to-end verification

> Not applicable — phase ships skill content. End-to-end exercise lands
> in M6 phase-06 where Claude invokes the full `/rexymcp:architect →
> dispatch → review → (escalate)` cycle against a real third-party
> repo.
>
> If a **manual smoke test** is performed (e.g. invoking each skill in
> a throwaway directory to sanity-check the prompt steers right),
> document the result in the Update Log. Not required.

## Authorizations

- [x] **May rewrite** `plugin/skills/dispatch/SKILL.md` (replacing the
      phase-01 stub).
- [x] **May rewrite** `plugin/skills/review/SKILL.md` (replacing the
      phase-01 stub).
- [x] **May create** `plugin/skills/escalate/` directory and
      `plugin/skills/escalate/SKILL.md` (net-new, no stub exists).
- [ ] **No Rust code changes.** No `executor/` or `mcp/` edits.
- [ ] **No new dependencies.** No `Cargo.toml` edits.
- [ ] May **NOT** modify `plugin/skills/architect/SKILL.md` (phase-04's
      deliverable), `plugin/.mcp.json` (phase-01), `plugin/
      .claude-plugin/plugin.json` (phase-01), `plugin/templates/*`
      (phase-02), `executor/templates/executor_contract.md` (phase-02),
      or any other phase doc.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`,
      `WORKFLOW.md`, `AGENTS.md`.
- [ ] **Calibration carry-forward (mandatory):** declare every scope
      deviation in "Notes for review". Pre-flight 3 carry-over: when
      Claude Code's actual skill-to-skill delegation convention differs
      from the spec's sketch, follow the convention and flag.

## Out of scope

- **Dogfood execution** — phase-06.
- **`continue_phase` MCP tool / resume lever support** — flagged in the
  escalate decision tree as future work; not built here.
- **Skill-to-skill structured handoff** — phase-05 punts to "the
  dispatch skill tells the user to run /rexymcp:review next"; a real
  handoff (e.g. dispatch automatically chaining into review on
  `complete`) breaks the per-skill gate and contradicts phase-04 §6
  prohibition #2.
- **Localization** — English-only.
- **Custom verdict templates per project** — the WORKFLOW.md template
  is the format; per-project variation isn't in scope.
- **Approval workflow signing / cryptographic provenance** — out of
  scope; the architect's identity is implied by the commit author.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-31 (progress)

Status flipped to in-progress. Reading pre-flight items: verified Claude Code's skill-to-skill delegation is via user-suggested slash commands (no structured handoff), confirmed MCP tool results surface through the tool-call mechanism in context. Confirmed existing stubs at dispatch/ and review/, confirmed escalate/ does not exist. Starting with dispatch SKILL.md.

### Update — 2026-05-31 (complete)

**Summary:** Replaced the phase-01 stubs for dispatch and review SKILL.md files and created the net-new escalate SKILL.md. Dispatch (7 sections) is thin glue: read NEXT.md + phase doc + rexymcp.toml, check executor_health, invoke execute_phase, surface complete/hard_fail/budget_exceeded results with suggested next-step slash commands. Review (10 sections) is the substantive gate: refuse non-review-status and hard_fail results, re-run the command set, walk the DoD checklist, spot-check tests, walk acceptance criteria, write verdict + flip status on pass, file bug + flip back on fail, handle milestone close. Escalate (5 sections + pre-injected decision tree) carries the load-bearing prose: refined re-dispatch as default, session takeover as last resort with telemetry-cost framing, resume deferred. All three use `/rexymcp:` namespacing throughout. One declared deviation: the review skill's command-set examples originally used `cargo` literals, which the forbidden grep caught; replaced with generic `<command from rexymcp.toml>` placeholders.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.87s

cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s

cargo test
test result: ok. 518 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**
- Not applicable — phase ships skill content (Markdown only). End-to-end exercise lands in M6 phase-06 (dogfood).
- Validation greps:
  - `grep -rnE '[Rr]exy[Mm][Cc][Pp]' plugin/skills/{dispatch,review,escalate}/SKILL.md` — 22 hits, all legitimate refs (rexymcp.toml, /rexymcp: slash commands, rexymcp serve).
  - `grep -rnE 'opencode|Rexy(?!MCP)|cargo |Cargo\.toml' plugin/skills/{dispatch,review,escalate}/SKILL.md` — zero hits.
  - `grep '^## ' plugin/skills/{dispatch,review,escalate}/SKILL.md` — dispatch: 7 sections (Read these first, 1-5, 6); review: 10 sections (Read these first, 1-10); escalate: 5 sections (Read these first, 1-5) + pre-injected decision tree integrated under §2.
- Read-pass gut-check:
  - `/rexymcp:dispatch phase-01` against a todo phase with reachable endpoint → reads NEXT.md, confirms todo, checks rexymcp.toml, runs executor_health, invokes execute_phase, on complete suggests /rexymcp:review, on hard_fail/budget_exceeded suggests /rexymcp:escalate. Correct.
  - `/rexymcp:review phase-01` against a review-status phase with complete result → refuses non-review/hard_fail, runs commands from rexymcp.toml, walks DoD, spot-checks tests, walks acceptance criteria, on pass writes verdict + flips to done + commits, on fail writes bug + flips to in-progress + commits. Correct.
  - `/rexymcp:escalate phase-01` against a hard_fail result → refuses complete results, reads briefing + phase doc + STANDARDS.md, walks decision tree (refined re-dispatch default → session takeover last resort → resume deferred), executes chosen lever, writes escalation outcome. Anti-pattern warning against skipping refined re-dispatch preserved. Correct.

**Files changed:**
- `plugin/skills/dispatch/SKILL.md` — replaced phase-01 stub with full 7-section dispatch skill.
- `plugin/skills/review/SKILL.md` — replaced phase-01 stub with full 10-section review skill.
- `plugin/skills/escalate/SKILL.md` — net-new file, 5 sections + pre-injected decision tree.

**New tests:** None — no Rust code changes.

**Commits:** (pending)

**Notes for review:** One scope deviation: review skill's command-set examples originally used `cargo` literals (caught by forbidden grep); replaced with generic `<command from rexymcp.toml>` placeholders to satisfy the zero-hits acceptance criterion. This is defensible — the review skill runs whatever commands are in rexymcp.toml, not cargo-specific ones. Pre-flight 3 finding: Claude Code's skill-to-skill delegation is via user-suggested slash commands (no structured handoff); the review skill's "delegate to /rexymcp:escalate" means suggesting the user run that command, consistent with phase-04's no-auto-advance prohibition.

verification: fmt OK · clippy OK · tests 518 passed · build OK
