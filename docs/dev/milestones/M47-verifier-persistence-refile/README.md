# M47 — Verifier Persistence Keyed on Re-editing

**Goal:** `VerifierFailurePersistent` stops terminating wiring sweeps. The
streak extends only when the executor re-edits a file already edited in the
streak while the author-error count does not fall; first-touching a new file
restarts it. The calibration replay computes the same streak from session
logs so the rule's floor stays observable.

**Status:** done *(opened 2026-09-20; closed 2026-09-21 at two phases,
both `approved_first_try`)*

**Depends on:** M34 (governor calibration framework —
`rexymcp calibrate-governor`), M37 (the last governor false-positive
milestone; same shape of fix).

**Origin:** [GitHub issue #10](https://github.com/ryanczak/rexyMCP/issues/10)
and its in-repo write-up
`docs/dev/issues/2026-09-20-verifier-persistence-kills-wiring-sweeps.md`
(+ `docs/dev/issues/scripts/verifier-persistence-*.py`).

## Why this milestone exists

The shipped rule (`executor/src/governor/hard_fail.rs`
`check_verifier_persistence`) fires after `verifier_persistence_threshold`
(default 6) consecutive post-edit verifies with `> 0` author diagnostics and
a non-decreasing count. Its notion of progress is "the count went down."

A **wiring sweep** — adding a field or variant to a type constructed or
matched in several files, then editing one site per turn — cannot make the
count go down until the last site lands: every consumer edited before the
definition adds one error, and they all vanish at once. The rule reads
finished work as no progress.

Replaying a downstream project's 435 session logs (the issue's evidence):

| Population | Result |
|---|---|
| 8 fires of the shipped rule | 7 wiring sweeps, 1 genuine stall |
| the 7 sweeps, re-dispatched plainly | all completed next dispatch (mean 185 turns) |
| the 1 stall, re-dispatched | hard-failed twice more |
| max streak among the 427 runs it did not kill | 5 |

Raising the threshold is not a tune: at 10 the rule fires on nothing in the
corpus. Of five candidate rules replayed against every log, only **refile**
— extend the streak only when the triggering write hit a file already
written in this streak — keeps the true positive (8/8 → 1/8 fires) with the
false-positive floor unchanged at 5. Its semantics are also the right ones:
*re-editing the same file while the count does not fall* is hitting the same
wall; *first-touching a new file* is a sweep by definition.

## Exit criteria

- **Rule.** `check_verifier_persistence` consumes per-verify samples of
  `(author_errors, edit path)` and extends the streak only on a
  non-decreasing positive count **and** a path already in the streak's path
  set; a zero clears streak and set; a decrease or a first-touched path
  restarts the streak at 1 with the set reduced to that path. Threshold stays
  6. No new config knob.
- **Signal.** `HardFailSignal::VerifierFailurePersistent` carries the
  re-edited file, and `describe()` names it, so the briefing says which wall
  was hit.
- **Replay parity.** `rexymcp calibrate-governor` reports a
  `verifier_refile_run` signal beside the existing `verifier_persistence_run`,
  derived from the same streak function the live rule uses, with the
  triggering path taken from the `parsed` write call preceding each `verify`
  event. Both columns stay so disagreement is visible.
- **Corpus check at review (architect, not executor — the corpus is outside
  the executor's scope).** Replaying the downstream corpus through the
  shipped binary reproduces the issue's numbers: one run reaches a refile
  streak ≥ 6 and it is the stall; the max refile streak among `complete`
  runs is 5.
- All four gates green at every phase boundary.

## Architecture references

- `docs/architecture.md` § Status #47 — this milestone's summary; § M4
  step 7 — the hard-fail detector list this rule belongs to.
- `executor/src/governor/hard_fail.rs` — `HardFailSignal`, `evaluate`,
  `check_verifier_persistence`.
- `executor/src/agent/mod.rs` — the loop's verify step and hard-fail
  evaluation; `executor/src/agent/tools.rs` `edit_target` (which calls count
  as edits: `write_file`, `patch`).
- `mcp/src/calibrate_governor.rs` — `RunReplay`, `Signal`, `SIGNALS`,
  `format_report`.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Refile persistence rule ([phase-01-refile-persistence-rule.md](phase-01-refile-persistence-rule.md)): `VerifySample` + `verifier_persistence_streaks` + the rewritten check in `hard_fail.rs`; `file` on the signal; the agent loop records the edit path per verify; unit tests replaying the sweep and stall traces; agent-level sweep test | done |
| 02 | Replay refile signal ([phase-02-replay-refile-signal.md](phase-02-replay-refile-signal.md)): pair each `verify` with the preceding verified write in `calibrate_governor::replay`, add `VerifierRefileRun` to `SIGNALS` **and** `format_report`'s label list, synthetic-log tests | done |

Ordering: the executor crate first so phase-02 reuses the exported streak
function rather than re-implementing it.

## Notes

- **Known limitation, recorded not fixed:** two files edited alternately
  against the same wall (A, B, A, B, …) never fire, because a first-touched
  path resets the set to that path. The corpus has no such case; revisit only
  on telemetry.
- **Adjacent observation (out of scope):** `edit_target` recognises
  `write_file` and `patch` only. `patch_lines` is a registered tool
  (`mcp/src/runner.rs:205`) whose edits therefore run no post-edit verify and
  produce no `Verify` event — invisible to this rule and to the verifier
  retry loop alike. Candidate for its own fix.
- **Why not advisory-first:** the issue's replay already did the sampling
  over 435 logs, and the replay can reconstruct both rules from existing
  events, so no live advisory event is needed to compare them later.

## M47 retrospective — closed 2026-09-21

**Outcome: both phases `approved_first_try`, zero bugs, zero bounces, zero
assists**, on executor `local-inference-lab/GLM-5.3-Flash-NVFP4-Spark`
(125 and 173 turns). Every exit criterion is discharged, including the
architect-owned corpus check.

**The corpus check reproduces the issue exactly.** Replaying the downstream
project's 436 logs:

| Rule | Fires at threshold 6 | Max streak among `complete` |
|---|---|---|
| shipped (count-only) | 8 | 5 |
| refile (this milestone) | **1** | **5** |

The single surviving fire is `05a-6a740b0c` — the one run the issue
classified as a genuine stall, and the only one of the eight whose
re-dispatches hard-failed again. The seven wiring sweeps that each completed
on their next plain dispatch no longer fire. The false-positive floor is
unchanged at 5, so no healthy run moved closer to the threshold.

**What the two rules now show side by side**, from the shipped binary's own
report over the same corpus: on `hard_fail` runs the count-only signal sits
at P90 6 / P99 6 and refile at P90 3 / P99 5; on `complete` runs P99 falls
5 → 4. The sweep-only fires are downgraded and nothing else moved.

**What made this land clean.** Both phase docs carried their worked example
lint-checked and unit-tested in a scratch crate before it was written into
the spec, and both pre-dispatch checks were run against the real tree with
every non-preservation criterion confirmed failing. Phase-02's headline
gotcha — the signal being registered in `SIGNALS` but not in
`format_report`'s separate hardcoded label list, which computes a row that is
silently never printed — was called out in § Current state as "the single
most likely way to fail this phase", and the executor registered both lists
first try. That is the pre-injection thesis working: the failure was named
before it could happen.

**Review rigor.** Neither approval rested on the executor's report. All four
gates were re-run as separate invocations at each review, every acceptance
criterion was executed, and each phase's new tests were mutation-proved:
reverting the refile keying reddens exactly the three sweep/reset tests while
the single-file stall test stays green (phase-01); dropping the label from
`format_report`'s list reddens only the report test, and widening the
write-tool predicate to accept `patch_lines` reddens only the pairing test
(phase-02).

**Calibration recorded, no fold due.** Three items, each at first occurrence
and held as data per the one-is-data/two-is-trend/three-is-a-fix rule:

1. *Architect dispatched with a dirty tree* (phase-01). The executor's commit
   swept in uncommitted architect drafts; content verified unaltered. It
   chose the option that preserved the work and declared it, which is correct
   from that position. Remedy is procedural, and it was applied before
   phase-02: commit or stash architect drafts before dispatching.
2. *Executor fabricated its own model identity* in a phase-02 Update Log
   entry (`opus-4-6-headless`; the session log records GLM-5.3-Flash).
   Corrected in place with a visible annotation. No code affected, but it is
   the class WORKFLOW.md § "A pasted transcript is a claim, not evidence"
   names, and it corrupts the model attribution the scorecard depends on.
3. *Architect left `NEXT.md` pointing at the just-approved phase* instead of
   advancing it, caught at the next dispatch's pre-flight.

**Carried forward, unchanged:** the A/B/A/B alternation limitation and the
`patch_lines` verifier gap, both in § Notes above. The second is the stronger
candidate for its own milestone — `patch_lines` edits run no post-edit verify
at all, so they are invisible to this rule and to the verifier retry loop
alike.
