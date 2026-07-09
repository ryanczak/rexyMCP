# M27 — Autonomous Escalation Loop

**Goal:** Make the architect↔executor cycle run hands-off across a whole
milestone: a `/rexymcp:auto` skill drives draft → dispatch → review →
escalate/re-dispatch with full review rigor and no per-phase human pause,
stopping only at milestone boundaries, blockers, or budget exhaustion — plus the
executor/server changes that make autonomy *cheap* (server-authored bookkeeping,
briefing-seeded resume) and *honest* (a loop journal with per-activity token/cost
accounting for the architect's own work).

**Status:** in-progress (kicked off 2026-07-08)

**Depends on:** M26 (gate_retries wired; governor blind spots closed; the loop
inherits a hardened executor)

## Why now

M26's phase-06 wire-or-retire talk-through resolved that the escalation knobs
(`[escalation] max_assists`, `[budget] escalation_slots`) were never
executor-internal — their natural consumer is an **architect-side** autonomous
loop that reads a returned briefing, applies an escalate lever, and re-dispatches
without a human in the inner loop (see the
[M26 README](../M26-polish-and-hardening/README.md#escalation-budgeting-moved-to-m27)
§ "Escalation budgeting moved to M27"). M27 is that loop, plus the two queued
design conversations it absorbs: **D8/D9 server-authored bookkeeping**
(review § 3.2 — deletes the M22-class "correct code dies in the bookkeeping
tail" failures the loop would otherwise burn assists on) and the **resume lever**
(review § 3.1 — the cheap middle lever for "90% done, hit one wall").

The user's framing at kickoff: make the rexyMCP workflow more autonomous and
better integrated with Claude Code *and* other/future MCP clients — so the loop
driver is an agent-neutral plugin skill, with Claude Code-specific enhancements
(transcript-based token accounting, notifications) layered as optional and
degrading gracefully elsewhere.

## Design (fixed at kickoff, 2026-07-08, with the user)

Four forks were resolved in the kickoff talk-through:

1. **Autonomy level: full milestone loop.** `/rexymcp:auto` drafts the next
   phase, dispatches, reviews, approves/escalates, commits, and advances —
   cycling until the milestone closes or a stop condition fires. Milestone
   boundaries remain an absolute human gate (architect prohibition #3 is
   unchanged). Stop conditions: (a) milestone boundary; (b) per-phase assist
   budget exhausted; (c) any blocker/design-fork the skills already define as
   human territory (contract-doc changes, dependency requests,
   spec-vs-architecture conflicts); (d) a loop-level runaway backstop (max
   phases per run — pinned at phase-draft time).
2. **Review gate: full rigor, no pause.** The review skill runs verbatim inside
   the loop — independent gate re-runs, DoD walk, telemetry verdict, commit.
   The human inspects at the boundary via the **loop report** (below). In
   exchange, **every architect activity is metered**: detailed token/cost
   accounting tracked over time so the human can tune how much Claude-work the
   rexyMCP SDLC consumes.
3. **Scope: all three threads.** Loop skill + D8/D9 server-authored bookkeeping
   + `continue_phase` resume + telemetry/accounting substrate.
4. **Budget knobs: consolidate on `max_assists`.** `[escalation] max_assists`
   becomes *the* per-phase autonomous-escalation budget (round-trips the loop
   may spend on one phase before stopping for the human).
   `[budget] escalation_slots` is retired — removed from config, `calibrate`,
   and the `init` template; it never gained distinct semantics.

### Token/cost accounting — the honesty constraint

Claude cannot meter its own tokens from inside a skill. The design is therefore
two-part:

- **Loop journal (portable).** The loop skill records each architect activity —
  `draft`, `dispatch`, `review`, `assist` (escalate-refine), `takeover`,
  `boundary` — as a structured telemetry record (the M20 `EscalationEvent`
  substrate generalizes or gains a sibling) with phase id, timestamps, and
  outcome. Works identically on any MCP client. `PhaseRun.escalation_count`
  becomes real.
- **Usage harvester (Claude Code-specific, optional).** Claude Code writes
  per-message token usage into its local session transcripts
  (`~/.claude/projects/<slug>/*.jsonl`). A `rexymcp` CLI subcommand parses them
  and joins usage onto journal activities by session/time window, filling the
  M20 `architect_input_tokens`/`architect_output_tokens` fields and the
  dashboard's architect-cost rows (rates from `[architect]`, already live since
  M20 phase-03). On other clients tokens stay absent — **degrade to
  counts-and-durations, never fabricate.**

### The loop report

Whenever the loop stops — boundary, budget, blocker — it writes a structured
stop-report (milestone, phases run, verdicts, assists spent per phase,
token/cost totals if harvested, why it stopped, what needs the human) so the
human resumes from a briefing, not a scrollback dig. It is the milestone-level
analogue of the executor's `briefing`.

### Per-role model delegation (decided 2026-07-08, with the user)

The user selects the **architect model** for the session via `/model`; rexyMCP
never overrides it (skills are prompt expansions — there is no programmatic
hook to switch the running session's model, and a config key pretending
otherwise would be a silent no-op). Inside an `/rexymcp:auto` run, the loop
**delegates** per-task work to subagents running the model configured for that
role in `rexymcp.toml`:

```toml
[architect]
model = "claude-opus-4-8"            # cost rates (already exists, M20)
dispatch_model = "claude-sonnet-5"   # /auto delegates dispatch to this
review_model   = "claude-sonnet-5"   # /auto delegates review to this
```

- **No config for a role → inherit the architect model** (the native subagent
  default: omitted override = session model), so the default case needs no code.
- **No `draft_model` key, by design.** Drafting is the most context-hungry
  activity — it leans on the talk-through history and milestone thread in the
  main conversation, which a fresh subagent doesn't have. Drafting always runs
  in the main loop on the architect model; shipping a knob for it would be the
  silent-no-op trap.
- **Interactive mode is unchanged.** Outside `/auto`, skills run in the main
  loop on the session model as today.
- **Accounting implication (phase-05 input):** a delegated activity's cost uses
  the *role model's* rates, not the architect's — the harvester schema must not
  assume a single architect model per session. Subagent usage also lands in the
  Claude Code transcripts differently than main-loop turns; the join logic must
  handle both.
- Config keys + delegation mechanics land in **phase-06** (the only consumer);
  the accounting note lands in **phase-05**. This is the architect-side twin of
  the phase-07 stretch row (executor-model advisory routing).

### Client-integration notes

- Claude Code sends **no MCP progressToken** (long-standing), so a long
  autonomous run is invisible mid-phase from the MCP side; the loop skill must
  point the human at `rexymcp status` / `rexymcp dashboard` as the live view.
- Claude Code-native notification at stop-gates is a cheap optional enhancement
  (skill-layer, no server change) — considered at the loop-skill phase, not a
  commitment.

## Exit criteria

- `[budget] escalation_slots` is gone from `BudgetConfig`, `rexymcp calibrate`,
  and the `rexymcp init` template; `[escalation] max_assists` is documented (and
  consumed by the loop skill) as the per-phase autonomous assist budget.
- An assisted loop run appends journal records to the telemetry store and
  `PhaseRun.escalation_count` reflects the real assist count (non-zero on an
  assisted run, 0 otherwise).
- On a `complete` `execute_phase`, the **server** writes the phase doc's Status
  flip (`in-progress` → `review`) and a baseline Update Log completion entry
  from data it already holds; the executor contract no longer instructs the
  executor to author them (contract-template amendment authorized in that
  phase). A MEDIUM-tier model that writes correct code no longer dies in the
  bookkeeping tail.
- `continue_phase(phase, guidance)` resumes a `hard_fail`/`budget_exceeded`
  phase **briefing-seeded** — fresh context from phase doc + briefing +
  architect guidance + current diff, `task_states` restored from the session
  log — and the escalate skill's resume lever is un-stubbed with criteria for
  choosing it over re-dispatch.
- Architect token/cost per activity is visible in the dashboard/scorecard on
  Claude Code (harvested, never estimated); other clients see activity counts
  and durations.
- `/rexymcp:auto` exists as a plugin skill, composes the existing four skills
  without duplicating their logic, honors all stop conditions, and writes the
  loop report on every stop.
- All four gates green after every phase; no new dependency without explicit
  phase-doc authorization; telemetry/schema changes additive only
  (`#[serde(default)]`).

## Architecture references

- [`docs/dev/codebase-review-2026-07-07.md`](../../codebase-review-2026-07-07.md)
  §§ 3.1 (resume), 3.2 (server-authored bookkeeping), 3.3 (advisory routing).
- [M26 README](../M26-polish-and-hardening/README.md) § "Escalation budgeting
  moved to M27" — the kickoff mandate.
- `docs/architecture.md` § "Escalation = Claude Code itself" (amended at this
  kickoff: resume committed, autonomous loop added) and § Status #27.
- [M22 retrospective](../M22-bookkeeping-resilience/README.md) — the D8/D9
  deferral this milestone closes.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Consolidate escalation budget knobs ([phase-01-consolidate-escalation-knobs.md](phase-01-consolidate-escalation-knobs.md)) | done |
| 02  | Loop-journal substrate: `ArchitectActivity` record + `rexymcp journal` CLI (retire `EscalationEvent`) ([phase-02-loop-journal-substrate.md](phase-02-loop-journal-substrate.md)) | done |
| 02b | `escalation_count` wiring: retire orphaned `tier_telemetry.escalation_count`, derive the dashboard Assists counter from `assist` journal records ([phase-02b-escalation-count-wiring.md](phase-02b-escalation-count-wiring.md)) | done |
| 03a | Server-authored finalize: on `complete`, the server writes the Status flip + baseline Update Log + README row + separate `docs:` commit (additive, dormant-safe) ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | done |
| 03b | Retire the executor bookkeeping gate + amend the executor contract (flips authorship, activating 03a's finalize) ([phase-03b-retire-executor-bookkeeping-gate.md](phase-03b-retire-executor-bookkeeping-gate.md)) | done |
| 04 | `continue_phase` briefing-seeded resume ([phase-04-continue-phase-resume.md](phase-04-continue-phase-resume.md)) | done |
| 04b | Finalize tolerates a bounced status line — fixes the 03a server-authored-finalize no-op surfaced in phase-04 review ([phase-04b-finalize-bounced-status-match.md](phase-04b-finalize-bounced-status-match.md)) | done |
| 05a | Architect token substrate: `ArchitectTokens`/`ArchitectRates` cache-aware types, retire `TierTelemetry.architect_*`, dormant cache-aware dashboard cost path ([phase-05a-architect-token-substrate.md](phase-05a-architect-token-substrate.md)) | done | review |
| 05b | Architect usage harvester: Claude Code transcript reader + `message.id` dedup + ISO→epoch parse + per-phase time-window join + `rexymcp harvest` CLI (fills 05a's tokens) | planned |
| 06 | `/rexymcp:auto` loop skill + loop report + WORKFLOW template mirror | planned |
| 07 | *(stretch)* Advisory model routing in dispatch (review § 3.3) | planned |

Phase 02 was **split at draft time** (2026-07-08) into 02 (the write-side
substrate: the record type + producer CLI) and 02b (the read-side wiring: the
dashboard counter + orphaned-field retirement), following the M26 07a/07b
precedent — each half is one bounded executor session. Two forks the kickoff
left open were resolved with the user at draft time: (1) **generalize, not
sibling** — the dead M20 `EscalationEvent` (zero producers, zero readers) is
retired and its `assist` concern folds into `ArchitectActivity` as one of six
kinds; (2) **rewire the Assists counter in phase-02(b)** — the dashboard counter
derives from `assist` journal records and the never-written
`tier_telemetry.escalation_count` field is retired (the same consolidation
phase-01 applied to `escalation_slots`).

Phase 03 was **split at draft time** (2026-07-08) into 03a (the additive,
dormant-safe server-side author: on a `complete` run the server writes the
Status flip + baseline Update Log entry + README row and makes a separate
`docs:` commit) and 03b (the authorship flip: retire the executor's
pre-completion `bookkeeping_feedback` gate and amend the executor contract so
the executor stops authoring the completion tail — which activates 03a's
finalize). The design forks were resolved with the user at draft time:
(1) **commit ownership** — the executor still commits its *code*; the server
commits the *bookkeeping* as a separate `docs:` commit (two commits per
phase); (2) **qualitative splice** — the server authors the mechanical entry
skeleton (gates, files changed, commit sha) and splices in a Summary +
Notes-for-review the executor returns via the `PhaseResult.completion_summary`
channel. 03a lands first because it is inert (no-ops on an already-`review`
doc) until 03b removes the executor gate.

Phase 05 was **split at draft time** (2026-07-09) into 05a (substrate) and 05b
(harvester), following the 03a/03b precedent. Three design forks were resolved
with the user before drafting: (1) **per-phase attribution via journal
time-windows** — harvested transcript usage is attributed to the
`ArchitectActivity` whose journal-timestamp window contains each message, rolling
up phase → milestone → project; (2) **no date crate** — the one time operation
(fixed-format ISO-8601-Zulu → epoch-ms, to window against the journal's epoch-ms
`ts`) is an exact hand-rolled `days_from_civil` conversion, bit-identical to a
crate for this UTC format and consistent with the established "raw epoch-ms, no
date crate" convention; (3) **separate cache rates** — architect cost bills
uncached-input / cache-creation / cache-read / output at their real per-class
rates (cache-read 0.1×, cache-creation 1.25× the input rate), not a flat
input/output pair, because cache tokens dominate real usage. Consequent to fork
(3), the user chose a **targeted rewrite of the architect-token model** (05a):
one coherent `ArchitectTokens` type (+ `ArchitectRates` + a `cost` method)
replaces the scattered flat fields, the dead `TierTelemetry.architect_*_tokens`
are retired, and the executor's `TokenBreakdown` is left untouched (real history,
`$0` cost, no accounting benefit). The write path is **append + fold at read**
(last-write-wins by `(phase_id, activity, ts)`, mirroring `fold_reviews` + the
resume last-write-wins idiom): 05b's harvester appends enriched activity copies
and a new `fold_activities` overlays the latest at read time. 05a is additive and
**dormant** — every architect token count is 0 until 05b's harvester runs, so the
dashboard is unchanged after 05a.

Phases are drafted **on demand** via `/rexymcp:architect next`; rows above are
the plan, not final specs. Ordering: substrate first (01–02), then the two
executor/server autonomy pieces biggest-win-first (03–04), then accounting (05),
then the loop skill that consumes all of it (06). 03 and 04 each touch a design
seam (executor contract; resume entrypoint) and may split at draft time (the
M26 07a/07b precedent). 07 is a stretch row — skill-layer only, drafted only if
appetite remains after 06.

## Notes

### Kickoff decisions (2026-07-08, with the user)

- The four design forks and their resolutions are recorded in § Design above.
- **Human-gate inventory after M27:** milestone kick-off and close (retrospective
  + folds), any blocker the executor or loop files, per-phase assist-budget
  exhaustion, and anything WORKFLOW § "What Executors Never Decide" lists. The
  per-phase draft/dispatch/review pauses are removed *only* inside an explicit
  `/rexymcp:auto` run; the default interactive mode is unchanged.
- **Loop skill composes, never forks.** `/rexymcp:auto` invokes the same
  architect/dispatch/review/escalate skill procedures; it adds sequencing,
  budgets, journaling, and stop conditions. Any behavior divergence between
  interactive and autonomous runs of the same step is a bug.
- **`architect_*_tokens` are harvested or absent.** No chars/4-style estimates
  of Claude's usage (the review § 3.4 lesson applies architect-side too).
- **WORKFLOW.md amendment at kickoff** expands the existing "Opt-in autonomous
  loop (off by default)" paragraph with the concrete mechanism (skill name,
  stop conditions, budget knob, loop report). The plugin-template mirror of
  WORKFLOW.md is updated in phase-06 alongside the skill it describes.

<!-- retrospective appended at milestone close -->