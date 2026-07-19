# M34 — Governor Stall Hardening

**Goal:** Close the small-model self-sabotage gaps a downstream project
(DaemonEye M4) surfaced — verify-loops that evade the governor and whole-session
self-reverts — by adding novelty-aware stall detection and a git-stash guard;
then, having watched the new detector misfire on a productive run, pivot to
**advisory-until-calibrated** detection: make the novelty measurement
*observable*, demote the detector to *advisory-by-default* (thresholds must be
data-backed, not guessed), and set the thresholds from a back-test of real runs.

**Status:** done (closed 2026-07-19)

**Depends on:** M33 (the `tools::mutates_files` classifier the new detectors reuse)

## Why now

M33 unified the mutating-tool classifier so the existing no-progress counter
stopped false-firing. But the DaemonEye M4 epoch surfaced two *new* failure
classes the governor could not see at all:

- **Verify-loops that evade every existing detector.** DaemonEye phases 05b and
  06 verify-looped for **529** and **167** turns respectively — varied
  `grep`/test/`git status` calls that made no code progress. `IdenticalToolCall`
  (needs exact repeats) and `Oscillation` (needs a small distinct set) both miss
  a loop of *varied* read-only calls; the executor ran until a human `rexymcp
  stop`.
- **Whole-session self-revert via `git stash`.** DaemonEye phases 01 and 03 saw
  the executor wipe its own correct work with `git stash` / `git stash pop`. The
  M22 self-revert guard scanned per-path `git checkout`/`restore`, but a bare
  `git stash` is a whole-session revert the per-path scan never saw.

Three fixes landed **directly** (see Phases 01–03) ahead of this milestone doc,
driven by GitHub issues and field reports rather than the phase-doc queue. This
milestone **formalizes that completed work for the record** and plans the two
follow-ons that close it out: the churn detector shipped untuned and invisible
(no way to calibrate `novelty_window`/`novelty_distinct_floor` from real runs),
and a fired stall tells the architect *that* churn happened but not *which*
targets were churned.

## Exit criteria

- `NoProgressStall` and `LowNoveltyStall` detectors exist, are configurable, and
  reset on any `Category::Write` tool (Phases 01, 03 — **met**).
- A bare `git stash` push is refused when the executor has edited files this
  session; restore/inspect forms stay allowed (Phase 02 — **met**).
- The novelty detector's per-run measurements are emitted to the session log and
  readable via the existing log-query tools (Phase 04 — **met**).
- The novelty detector is **advisory-by-default** (`[governor] novelty_action`),
  so an un-calibrated threshold cannot terminate a run the turn budget would still
  fund; the raw `NoProgressStall` stays as the terminating backstop (Phase 05 —
  **met**).
- Thresholds can be set from a back-test of the real session-log corpus, not
  guessed — the `rexymcp calibrate-governor` deliverable, per-model + global, for
  every reachable detector (Phases 06a/06b — **met**).
- *(Reduced scope)* a fired-stall briefing names the churned targets — **dropped**
  at close: advisory-by-default made the surface too narrow to be worth it
  (Phase 07 — **descoped**).
- All four gates green — **met**.

**The advisory-until-calibrated pivot (2026-07-18).** Phases 01–04 shipped the
detectors + observability. Phase-04's own dispatch then hard-failed when the
`LowNoveltyStall` detector fired on its own author at **turn 104 of a 600-turn
budget (82% unused)** — a data-free early-kill pre-empting a productive run, which
is contrary to rexyMCP's ethos. Decided with the user: detection becomes
**advisory until thresholds are data-backed**. This reshaped the tail — Phase 05
(advisory-demotion) and Phase 06 (calibration back-test / metrics overhaul) were
added ahead of the (now reduced-scope) briefing-quality phase, renumbered 07.

## Architecture references

- `docs/architecture.md` § Layer 1 (the executor turn cycle — the hard-fail
  detector list) and § Status #34 (this milestone).
- `docs/dev/milestones/M33-governor-mutating-classifier/README.md` — the
  `tools::mutates_files` unification these detectors depend on.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | No-progress read-only stall detector (FR-2) ([phase-01-no-progress-stall.md](phase-01-no-progress-stall.md)) | done |
| 02 | Refuse `git stash` self-revert (FR-1) ([phase-02-git-stash-self-revert-guard.md](phase-02-git-stash-self-revert-guard.md)) | done |
| 03 | Low-novelty churn stall detector (issue #3) ([phase-03-low-novelty-stall.md](phase-03-low-novelty-stall.md)) | done |
| 04 | Novelty-detector observability ([phase-04-novelty-detector-observability.md](phase-04-novelty-detector-observability.md)) | done |
| 05 | Advisory-demotion of the novelty stall detector ([phase-05-advisory-demotion.md](phase-05-advisory-demotion.md)) | done |
| 06a | Governor calibration framework + stall-signal report ([phase-06a-calibration-framework-and-stall-signals.md](phase-06a-calibration-framework-and-stall-signals.md)) | done |
| 06b | Extend calibration to the remaining detectors ([phase-06b-calibration-remaining-detectors.md](phase-06b-calibration-remaining-detectors.md)) | done |
| 07 | Stall-fire briefing quality ([phase-07-stall-fire-briefing-quality.md](phase-07-stall-fire-briefing-quality.md)) | dropped (descoped at close) |

## Notes

**Phases 01–03 were executed directly by the architect (Claude Code), not
dispatched** — the same routing call M29/M32/M33 made, for the same reason: the
change lives in the very governor that supervises the executor loop, so
dispatching it to a model with its own stall tendencies risks tripping the thing
under repair mid-fix. Their phase docs (01–03) are **retrospective** — the code
was committed before the doc — and record what shipped, the commit SHA, and the
review verdict; they are not forward-looking specs. Phases 04–05 are drafted on
demand via `/rexymcp:architect next` and follow the normal draft → dispatch (or
direct) → review gate; the same governor-internal routing consideration applies
and is noted in each.

**Design record.** The two stall detectors are deliberately layered:
`LowNoveltyStall` (novelty) is the *early* catch and `NoProgressStall` (raw
volume, threshold demoted 20 → 60) is a high *backstop* beneath `max_turns`.
Novelty keys off `normalize_target`, which reuses M33's `tools::mutates_files` /
router category so the two classifiers never drift. Both detectors examine only
the *trailing read-only run* and reset on any file-mutating call, so read-heavy
exploration *between* edits never trips either.

## Retrospective — 2026-07-19

**Shipped.** 7 committed phases (01–06b); phase-07 descoped. The milestone
started as bookkeeping — formalizing three direct-executed governor fixes (FR-1
git-stash guard, FR-2 `NoProgressStall`, issue-#3 `LowNoveltyStall`) — and turned
into a genuine design correction.

**The pivot.** phase-04 (novelty observability) was dispatched and the
`LowNoveltyStall` detector **fired on its own author** at turn 104 of a 600-turn
budget (82% unused). That one event reframed the whole milestone: an un-calibrated,
data-free threshold was pre-empting a productive run — contrary to rexyMCP's
"no failure modes on arbitrary numbers" ethos. Decided with the user:
**advisory-until-calibrated**. The tail was reshaped on the spot — phase-05
(demote novelty to advisory-by-default, keep the raw 60-call `NoProgressStall` as
the real terminating backstop), and phase-06a/06b (build `rexymcp
calibrate-governor` — replay the session-log corpus, re-derive every reachable
governor signal, report per-model + global distributions by outcome). The
detector's misfire became the milestone's best evidence.

**Payoff — data over guesses.** The corpus now shows successful runs cluster at
novelty distinct-target **P50≈15** while stuck (`budget_exceeded`) runs collapse to
**P50≈1**, with the guessed `novelty_distinct_floor = 6` sitting in the empty gap
between. Every reachable detector (novelty, no-progress, identical, oscillation,
verifier-persistence, empty-completion) now has real distributions behind it. One
detector — output-flood — is a documented data gap (the log stores a truncated
`output_preview`, not `content.len()`); closing it needs a new logged field and is
deferred.

**Verdicts.** 01/02/03 approved_first_try (retrospective, direct); 04 escalated
(dispatch hard_failed on its own `LowNoveltyStall`, architect takeover finished the
one integration test); 05 approved_first_try (direct); 06a approved_after_1
(bug-06a-1, `spec_bug`); 06b approved_after_1 (bug-06b-1, `false_completion`). Two
of three dispatched phases bounced once each — both caught at review, both clean on
re-dispatch.

**Calibration data (fold candidates — NOT folded; hold for user sign-off):**
1. **Runs-vs-samples in aggregation specs** (bug-06a-1, `spec_bug`, 1st occurrence).
   The 06a spec said "cell with `count < min_runs`" without defining count-as-runs
   vs count-as-samples; for a multi-sample signal that let one run masquerade as
   N=253. A spec pinning a per-unit aggregation floor should state the unit and pin
   the negative (a 1-run/many-sample cell must be dropped). Data, not yet a trend.
2. **Executor skips the Test plan / ships new code untested** (bug-06b-1,
   `false_completion`, 1st occurrence *this milestone*). 06b implemented four
   extractors and wrote **zero** of the seven spec'd tests, self-reporting complete.
   Possibly a cross-milestone recurrence of the untested-new-code `false_completion`
   class (cf. M4/M27) — flag to check at the next fold window before touching the
   contract.

**Deferred to the planned metrics & reporting deep-dive:**
- `oscillation_min_distinct` is a *lower-is-worse* signal, but the report shows
  p50/p90/p99 (the high tail) — oscillatory runs don't surface. Min-type signals
  want the low percentiles. Reporting-design fix.
- output-flood needs a `ToolResult.output_bytes` logged field (executor-side) to
  become calibratable — and no corpus data until new runs accrue.
- Whether `calibrate-governor` should fold into / align with the existing
  `scorecard`/`runs`/dashboard surfaces.

**No contract-doc folds landed** (STANDARDS/WORKFLOW untouched) — both calibration
items are 1st-occurrence data awaiting the user's fold decision.