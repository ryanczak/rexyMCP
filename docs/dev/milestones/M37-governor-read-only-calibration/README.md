# M37 — Governor Read-Only Calibration

**Goal:** Stop the governor from hard-killing read-only diagnosis at
write-thrash thresholds, and clear the last of M35's accounting debt.

**Status:** planning

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
- One token formatter. `runs::fmt_tokens`, the inline formatter in `scorecard`,
  and `costs::format_tokens` collapse into the shared `metrics` helper, with
  every call site migrated.
- `calibrate-governor`'s output-flood byte columns render k/M-compacted, in line
  with the shared rendering 07c established.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #37 — this milestone's design summary.
- `docs/architecture.md` § Status #34 — `NoProgressStall` and the
  advisory-until-calibrated pivot this milestone declines to repeat.
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md`
  § "M35 retrospective" — folds 5 and 6, and the accepted debt.

## Phases

Not yet expanded. Per WORKFLOW.md § Milestones, the architect expands a
milestone into phases on demand. Expected shape:

| #  | Phase | Status |
|----|-------|--------|
| 01 | Read-only exemption in the oscillation + identical-repetition detectors | todo |
| 02 | `oscillation_stall` in `FAILURE_CLASSES` | todo |
| 03 | Consolidate the three token formatters into `metrics` | todo |
| 04 | k/M compaction for `calibrate-governor`'s byte columns | todo |

Phase 01 is the milestone; 02–04 are carried debt and can run in any order after
it. Phase 01 changes governor termination behavior, so it needs negative tests
pinning that mutating windows are untouched — a blanket disable would pass a
positive-only suite.

## Notes

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
