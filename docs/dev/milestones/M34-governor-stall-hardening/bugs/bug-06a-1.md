# Bug 1 on phase-06a: `min_runs` and the `N` column count samples, not runs — overstating evidence for multi-sample signals

**Severity:** major
**Status:** verified
**Filed:** 2026-07-19

## What's wrong

The per-model filter and the reported `N` operate on **sample count**, but the
`--min-runs` flag name and the granularity intent (per-model with a floor that
"drops thin per-model cells") mean **run count**. For a signal that emits many
samples per run this badly overstates the evidence.

`mcp/src/calibrate_governor.rs` (per-model aggregation):

```rust
for ((model, outcome), samples) in by_model_outcome {
    let mut sorted = samples;
    sorted.sort();
    let n = sorted.len();          // <-- SAMPLE count
    if n < min_runs {              // <-- filters on samples, not runs
        continue;
    }
    rows.push(ReportRow { /* … */ n, /* … */ });   // <-- reports SAMPLE count as N
}
```

`Signal::NoveltyDistinct` emits **one sample per full novelty window** (many per
run); `Signal::MaxReadOnlyRun` emits **one sample per run**. So the two signals'
`N` are not comparable, and `--min-runs` filters them on different scales.

**Observed on the real corpus** (`rexymcp calibrate-governor --repo . --min-runs 3`):

```
signal: novelty_distinct_targets
(all)    budget_exceeded 253     1     2     3
Qwen/Qwen3.6-27B-PrismaAURA budget_exceeded 253     1     2     3
signal: max_read_only_run
(all)    budget_exceeded  1   276   276   276
```

Both `budget_exceeded` rows are the **same single run** — one run with a 276-call
read-only streak. `max_read_only_run` correctly shows `N=1` (one run);
`novelty_distinct_targets` shows `N=253` (253 windows from that one run), and
`--min-runs 3` **passed** it. A human reading `P50=1, N=253` would treat the
novelty floor as strongly supported when it rests on a **single run**. That is
precisely the ungrounded-number failure mode this milestone exists to prevent
(the whole advisory-until-calibrated rationale).

## What should happen

Per the phase-06a design decision ("**per-model, with a global fallback** — always
show per-cell sample size N; a `--min-runs` floor drops thin per-model cells"),
the floor is meant to drop cells backed by too few **runs**, and the reader must
be able to see how many independent runs a cell rests on.

- `--min-runs` must filter on the **number of runs** that contributed to the cell,
  not the number of samples.
- The report must make run-count visible so `N=253 from 1 run` cannot masquerade
  as well-populated. Keep the sample count too (percentiles are over samples) —
  show both, e.g. a `RUNS` column beside `N`.

## How to fix

In `mcp/src/calibrate_governor.rs`, track runs and samples separately per
`(signal, model, outcome)` cell:

1. When aggregating, record per cell both the accumulated samples **and** a run
   counter incremented once per run that contributed ≥1 sample to that signal
   (a run with no full novelty window contributes no novelty data, so it should
   not count toward the novelty run count). E.g. accumulate `Vec<Vec<usize>>`
   (per-run sample groups) per cell, or keep a parallel
   `HashMap<(String, String), usize>` of run counts.
2. Filter per-model rows on `runs < min_runs` (not `samples < min_runs`).
3. Add a `runs` field to `ReportRow` and render a `RUNS` column (both text and
   JSON); keep `N` as the sample count with percentiles computed over samples.
4. Apply the same run/sample distinction to the global `(all)` rows so the two
   are consistent.

Update the `min_runs_drops_thin_per_model_cells_into_global` test — and add a
case that a **single run producing many samples** (e.g. one 30-read novelty run,
window small enough to yield ≥3 windows) is dropped by `--min-runs 3` because it
is **1 run**, even though its sample count exceeds 3. That negative case is what
distinguishes the fix from the current behavior.

## Verification

- [ ] `rexymcp calibrate-governor --repo . --min-runs 3` no longer shows a
      per-model `novelty_distinct_targets` cell backed by a single run; run count
      is visible in the output.
- [ ] A hermetic test proves a one-run/many-sample cell is dropped by
      `--min-runs 3` (runs=1 < 3) while its samples still feed the `(all)` row.
- [ ] `cargo test -p rexymcp calibrate_governor` passes.
- [ ] All four gates green.
