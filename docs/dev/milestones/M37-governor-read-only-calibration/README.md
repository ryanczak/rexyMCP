# M37 — Governor Read-Only Calibration

**Goal:** Stop the governor from hard-killing read-only diagnosis at
write-thrash thresholds, and clear the last of M35's accounting debt.

**Status:** in-progress *(opened 2026-07-24)*

**Depends on:** M34 (`NoProgressStall`, which this milestone makes the sole
read-only terminator), M35 (source of the calibration data and the debt)

## Why this milestone exists

**The STRONG fold from the M35 close — 4 occurrences, well past the
three-strike "fold immediately" line.** `check_oscillation`
(`executor/src/governor/hard_fail.rs:225-256`) and
`check_identical_repetition` (`:137-157`) key on `(tool, arguments)` and are
blind to whether a call **mutates** anything. So a model re-running
`sed -n`/`cat`/`python3 -c` to diagnose a confusing failure is terminated on the
same threshold as a genuine write-thrash loop.

M34 already shipped `check_no_progress_stall` (`:274`) for exactly this case —
N consecutive non-mutating calls, threshold 60 — and it already calls
`crate::tools::mutates_files`. The two are duplicate coverage at wildly
different thresholds, and the tighter one wins, which is why it pre-empts.

Across the M35 arc every one of the 4 oscillations recovered on a resume or a
refined re-dispatch carrying one specific hint — the production code had been
correct or nearly so each time. The runs were killed mid-diagnosis, not
mid-thrash.

**User decision (2026-07-23):** exempt windows containing no file-mutating call
from both detectors, leaving read-only loops to `NoProgressStall`.

Rejected alternatives, recorded so they are not re-litigated:

- **Advisory mode**, on M34 phase-05's `novelty_action` precedent — keeps a
  signal, but keeps the pre-emption risk and adds a config knob nobody has data
  to tune.
- **Separate looser read-only thresholds** — still hard-kills, just later, and
  doubles the threshold surface.

## Exit criteria

- A window of tool calls containing **no** file-mutating call fires neither
  `Oscillation` nor `IdenticalToolCallRepetition`. A window containing at least
  one mutating call behaves exactly as it does today (pinned by a negative
  test — this must not become a blanket disable).
- `NoProgressStall` still terminates a purely read-only run at its configured
  threshold; the exemption must not create an unterminated loop.
- `oscillation_stall` is in `FAILURE_CLASSES`
  (`executor/src/store/telemetry.rs:319`) and `is_known_failure_class` accepts
  it. Recorded 2× as an unknown open-vocab class during M35.
- **`missing_spec_test`** is in `FAILURE_CLASSES` too. Recorded open-vocab at
  the M38 phase-01 bounce (2026-07-24): the executor implemented the production
  change correctly but omitted one of the four tests the spec's § Test plan
  named. None of the nine existing classes fits — it is not
  `false_completion` (gates were green), not `scope_deviation` (nothing extra
  was touched), and not `spec_bug` (the spec named the test explicitly). A
  spec'd-but-unwritten test is a distinct and recurring enough failure mode to
  deserve its own label, or the scorecard buckets it as noise.
- One token formatter. `runs::fmt_tokens`, the inline formatter in `scorecard`,
  and `costs::format_tokens` collapse into the shared `metrics` helper, with
  every call site migrated.
- `calibrate-governor`'s output-flood byte columns render k/M-compacted, in line
  with the shared rendering 07c established.
- The **server-authored completion entry** satisfies STANDARDS §1: it ticks the
  phase doc's acceptance-criteria checkboxes and emits an
  `**End-to-end verification:**` block carrying the actual E2E output, so a
  `done` phase doc is not self-contradictory. Its `Executor:` line names the
  **dispatched** model (the same value as `PhaseRun.model`), never the model's
  self-report — pinned by a test that a self-reported name cannot reach the
  Update Log.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #37 — this milestone's design summary.
- `docs/architecture.md` § Status #34 — `NoProgressStall` and the
  advisory-until-calibrated pivot this milestone declines to repeat.
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md`
  § "M35 retrospective" — folds 5 and 6, and the accepted debt.

## Phases

**Phase 01 is drafted** (2026-07-24). Phases 02–05 stay sketched — per
WORKFLOW.md § Milestones the architect expands on demand, and 01 changes a
terminator whose outcome may inform how the rest are scoped.

| #  | Phase | Status |
|----|-------|--------|
| 01 | Read-only exemption in the oscillation + identical-repetition detectors ([phase-01-read-only-exemption.md](phase-01-read-only-exemption.md)) | in-progress |
| 02 | `oscillation_stall` + `missing_spec_test` in `FAILURE_CLASSES` | todo |
| 03 | Consolidate the three token formatters into `metrics` | todo |
| 04 | k/M compaction for `calibrate-governor`'s byte columns | todo |
| 05 | Server-authored completion entry: tick acceptance criteria, emit an E2E block, write `Executor:` from the dispatched model | todo |

Phase 01 is the milestone; 02–05 are carried debt and can run in any order after
it. Phase 01 changes governor termination behavior, so it needs negative tests
pinning that mutating windows are untouched — a blanket disable would pass a
positive-only suite.

## Notes

**Phase 05 — why it exists (added 2026-07-23 at the M36 phase-01 review).**
STANDARDS §1 requires that every acceptance criterion be ticked and that any
criterion referencing a real artifact be verified end-to-end with **the actual
output quoted** in the completion Update Log. Since M27 phase-03 moved the
bookkeeping tail server-side, the server-authored completion entry does
neither: it writes a summary, gate labels, command-output tails, files-changed
and the commit sha, but leaves the phase doc's `- [ ]` boxes untouched and
emits no `**End-to-end verification:**` section.

The result is a `done` phase doc whose own acceptance criteria read as unmet,
and an E2E claim asserted in prose rather than evidenced. Reproduced on M35
phase-06e, 07g, 07h and M36 phase-01, 02, 03 — **6 occurrences**, well past the
fold-immediately threshold. It is not an executor defect and **cannot be fixed
by re-dispatch**: the executor no longer owns that output. Each review has been
silently absorbing the gap by verifying and ticking manually.

**A third defect, same writer (added 2026-07-23 at the M36 phase-03 review).**
The Update Log's `**Executor:**` line is written from the model's *self-report*,
and models misidentify themselves. M36 phase-03's entry claims
`Executor: Claude Sonnet 4.5 (executor)` when `rexymcp.toml`, `executor_health`,
and the run's own `PhaseRun` telemetry record all say `Qwen/Qwen3.6-27B-FP8`.

Severity is **cosmetic, not corrupting**: the scorecard, `profile`, and
`calibrate-governor` all read the config-derived `PhaseRun.model` field, so no
aggregate is polluted — but the phase doc is the human-readable record, and a
milestone retrospective read months later would attribute the work to the wrong
model. The server dispatched the run and knows which model it used; it should
write that value rather than let the model name itself.

Scope: the server's completion-entry writer, three defects in one place.

1. **Tick the acceptance criteria.** Deciding what justifies a tick is the
   design question — the safe shape is to tick only criteria whose verifying
   command the server actually ran and observed pass, leaving the rest for the
   reviewer rather than ticking optimistically. A false tick is worse than a
   blank box.
2. **Emit an `**End-to-end verification:**` block** with the actual output. The
   writer already receives the command outputs it would need to quote.
3. **Write `Executor:` from the dispatched model**, not from model self-report.
   Source it from the same value that populates `PhaseRun.model`, so the prose
   and the telemetry can never disagree. Pin a test that a self-reported model
   name in the transcript does **not** reach the Update Log.

**Sequencing against M36.** Independent — M36 is `mcp/` accounting and display,
M37 is `executor/governor` plus small `mcp/` cleanups. Phase 03 (token
formatters) touches `costs::format_tokens`, which M36 phase-02 also edits; run
M36 phase-02 first or expect a trivial conflict.

**Calibration data available.** `rexymcp calibrate-governor` (M34 06a/06b)
replays the session-log corpus and reports per-model and global distributions
by run outcome for every governor signal. Phase 01 should check its
`oscillation_min_distinct` low-tail output (M35 07a) before and after, and the
result belongs in the phase's Update Log — this is the first change to a
terminator since that tooling existed.
