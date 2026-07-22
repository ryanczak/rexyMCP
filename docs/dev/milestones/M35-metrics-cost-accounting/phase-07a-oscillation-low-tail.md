# Phase 07a: Calibration reports the low tail for the lower-is-worse oscillation signal

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-06e
**Estimated diff:** ~300 lines
**Tags:** language=rust, kind=fix, size=m

## Goal

`rexymcp calibrate-governor` reports p50/p90/p99 (the **high** tail) uniformly for
every signal. But `oscillation_min_distinct` is a **lower-is-worse** signal — a run
that oscillates shows a *small* min-distinct — so the concerning tail is the **low**
end, and oscillatory runs never surface in the report. This is the M35 exit-criterion
item: *"Oscillation calibration reports low percentiles for its lower-is-worse
signal."* Fix: give each `Signal` a tail **direction** and report the
direction-appropriate percentiles — the low tail (e.g. p50/p10/p1) for the
lower-is-worse oscillation signal, the high tail (p50/p90/p99, unchanged) for the
other five.

This is **calibration reporting only** — the live oscillation *detector*
(`executor/src/governor/hard_fail.rs`) is untouched (see Out of scope).

## Architecture references

Read before starting:

- `mcp/src/calibrate_governor.rs` — the whole change lives here. Key spots quoted in
  Current state: the `Signal` enum (30–57), `OscillationMinDistinct`'s min-type sample
  extractor (104–122), the `percentile` helper (167–174), `ReportRow` (177–187),
  `format_report` (190–225), and the two `ReportRow`-build sites in `run()` (302–339).
- `mcp/src/scorecard_cli.rs:230` — `format_scorecard_shows_dropped_columns` is the
  template for "assert the report now renders the intended columns" tests.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Editing discipline (load-bearing — earlier M35 runs hard-failed here):** edit with
   `patch`/`patch_lines`, **never a whole-file `write_file`**. View with `read_file`
   (`start_line`/`end_line`), **never `sed -n`/`cat`, and NEVER run the same command
   twice** — the governor hard-fails on identical repeated calls. Read once, act.
   `cargo check -p rexymcp` after each edit.

## Current state

**`Signal`** (`calibrate_governor.rs:30–57`) — six signals; only
`OscillationMinDistinct` is min-type (lower-is-worse). Its extractor takes the
**minimum** distinct-call count over any window (`min = min.min(distinct.len())`,
119–121). The other five are max-type (a longest-run / distinct count where **higher**
is worse).

**`percentile`** (`calibrate_governor.rs:167–174`) — nearest-rank, `p` in `0.0..=1.0`:

```rust
fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
```

So on `sorted = [1,2,3,4,5,6,7,8]`: `percentile(.5)=5` (rank 4), `percentile(.1)=2`
(rank 1), `percentile(.01)=1` (rank 0), `percentile(.9)=7`, `percentile(.99)=8`. The
low-tail percentiles surface the small (oscillatory) values.

**`ReportRow`** (`calibrate_governor.rs:177–187`) — three fixed percentile columns:

```rust
struct ReportRow {
    signal: String,
    model: String,
    outcome: String,
    runs: usize,
    n: usize,
    p50: usize,
    p90: usize,
    p99: usize,
}
```

**The two build sites** in `run()` (`calibrate_governor.rs:309–318` global, `329–338`
per-model) both hard-code the high tail:

```rust
rows.push(ReportRow {
    signal: signal.label().to_string(),
    model: "(all)".to_string(),
    outcome,
    runs: cell.runs,
    n,
    p50: percentile(&sorted, 0.5),
    p90: percentile(&sorted, 0.9),
    p99: percentile(&sorted, 0.99),
});
```

**`format_report`** (`calibrate_governor.rs:190–225`) — groups rows by signal (one
header block per signal) and prints a fixed column header:

```rust
lines.push("MODEL  OUTCOME  RUNS  N  P50  P90  P99".to_string());
for row in signal_rows {
    lines.push(format!(
        "{:<8} {:<10} {:>4}  {:>4}  {:>4}  {:>4}  {:>4}",
        row.model, row.outcome, row.runs, row.n, row.p50, row.p90, row.p99
    ));
}
```

(There is already per-signal-block special-casing here — the `empty_completion_run`
block gets a custom header line at 209–213 — so per-signal header variation is an
established shape.)

## Spec

Pin the **behavior**; field names / exact column labels are the executor's call as
long as the behavior below holds.

### Task 1 — give each `Signal` a tail direction (calibrate_governor.rs)

Add a `TailDirection` enum (`HigherIsWorse`, `LowerIsWorse`) and a
`Signal::direction(self) -> TailDirection`. **Only `OscillationMinDistinct` is
`LowerIsWorse`; all five others are `HigherIsWorse`.** (Mirror the existing
`Signal::label` match at 47–57.)

### Task 2 — report the direction-appropriate tail (calibrate_governor.rs)

Rework `ReportRow` + the two build sites so the percentiles reported depend on the
signal's direction:

- **`HigherIsWorse`** (unchanged behavior): median + high tail →
  `percentile(.5)`, `percentile(.9)`, `percentile(.99)`.
- **`LowerIsWorse`** (the fix): median + **low** tail →
  `percentile(.5)`, `percentile(.10)`, `percentile(.01)`.

Make the row **self-describing** so the JSON consumer knows which tail it got: carry a
direction/tail indicator on `ReportRow` (e.g. a `tail: "higher-is-worse" |
"lower-is-worse"` string, or the three columns renamed to role-based names like
`p_mid`/`p_near`/`p_far` plus the tail field). Do **not** leave a field literally named
`p90` holding a p10 value — that is the trap; rename or tag so nothing is mislabeled.
Keep the median (`p50`) meaningful for both directions.

### Task 3 — label the tail correctly in the text report (calibrate_governor.rs)

In `format_report`, the per-signal header block must render column labels matching the
signal's direction: `P50 P90 P99` for higher-is-worse signals, `P50 P10 P1` (or the
equivalent low-tail labels) for `oscillation_min_distinct`. Add a short note on the
oscillation block that **low = worse** (mirror the existing `empty_completion_run`
custom-header pattern at 209–213), so a reader isn't misled into thinking a small
number is benign. The five higher-is-worse blocks render exactly as today.

## Acceptance criteria

- [ ] `Signal::direction` returns `LowerIsWorse` for `OscillationMinDistinct` and
      `HigherIsWorse` for the other five signals.
- [ ] For a lower-is-worse sample set, the reported percentiles are the **low** tail:
      given oscillation samples whose sorted form is `[1,2,3,4,5,6,7,8]`, the row
      reports median `5`, and the two tail columns are the low values (`2` at p10 and
      `1` at p1) — **not** `7`/`8`. **Mutation-sensitive:** this must fail under the old
      uniform-p90/p99 code (which would report `7`/`8`).
- [ ] For a higher-is-worse signal (e.g. `identical_run`), the reported percentiles are
      **unchanged** — median + high tail (`percentile(.9)`, `percentile(.99)`).
- [ ] The JSON output is self-describing: a consumer can tell from the row which tail
      the columns represent (a `tail`/direction field or role-named columns), with no
      field mislabeled (no `p90`-named field holding a p10 value).
- [ ] The text report's `oscillation_min_distinct` block header names the low-tail
      percentiles and flags low = worse; the other five blocks are unchanged.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] The live oscillation detector, the signal extractors (samples), and the other
      five signals' reported values are unchanged.

## Test plan

Extend the inline `#[cfg(test)] mod tests` in `mcp/src/calibrate_governor.rs` (reuse
its fixture builders — `make_session_file` etc.). Pin behavior + names:

- `signal_direction_maps_oscillation_low_others_high` — asserts
  `OscillationMinDistinct` → `LowerIsWorse` and at least two others (e.g.
  `IdenticalRun`, `NoveltyDistinct`) → `HigherIsWorse`.
- `oscillation_report_surfaces_low_tail` — build a report whose oscillation samples
  sort to `[1,2,3,4,5,6,7,8]`; assert the row's median is `5` and the tail columns are
  the low values (`2`, `1`), **not** `7`/`8`. Mutation-sensitive against the old code.
- `higher_is_worse_signal_reports_high_tail_unchanged` — for a max-type signal with the
  same sample set, assert median `5` and tail `7`/`8` (the existing behavior is
  preserved).
- `format_report_labels_oscillation_tail_low` — assert the rendered
  `oscillation_min_distinct` block header names the low-tail percentiles (behavioral
  `contains` check on the low-tail labels, not exact whitespace), and that a
  higher-is-worse block still shows the high-tail labels.
- Keep/extend `percentile_boundaries` (644–651) if the percentile helper stays local;
  add a case pinning a low-percentile value (`percentile([1..=8], 0.1) == 2`).

## End-to-end verification

`calibrate-governor` reads the real session-log corpus. For the artifact check, run it
against this repo's telemetry and quote the `oscillation_min_distinct` block:

```
cargo run -p rexymcp -- calibrate-governor --config rexymcp.toml
```

Quote the rendered `oscillation_min_distinct` block and confirm its header names the
**low**-tail percentiles (P50/P10/P1 or equivalent) — contrast with a higher-is-worse
block still showing P50/P90/P99. If the local corpus has too few oscillation samples to
populate the block (min-runs filtered), say so and rely on the unit tests as the pinned
evidence (quote `oscillation_report_surfaces_low_tail`). Also quote
`--json` for one row showing the self-describing tail/direction field.

## Authorizations

- Editing `mcp/src/calibrate_governor.rs` is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths
  (the existing `serde_json::to_string_pretty(...).unwrap_or_else(...)` at 343 is a
  fallback, not a new unwrap — leave it).

## Out of scope

- **The live oscillation detector** (`executor/src/governor/hard_fail.rs`
  `check_oscillation`) and its `distinct_max`/window threshold. This phase changes only
  how the *calibration replay* reports percentiles, not how the runtime governor fires.
  Re-tuning the detector from the new report is a later calibration decision, not code
  here.
- **Moving `percentile` into the shared `executor/src/store/metrics.rs`** and adopting
  the scorecard/runs table-formatting conventions (uppercase headers, `—`, k/M
  compaction). That is the **calibrate-governor rendering-alignment** work — deferred to
  phase-07c. 07a keeps `percentile` where it is and only fixes the tail direction.
- **The `output_bytes` output-flood signal** (the replay currently drops `ToolResult`
  events). That is a *new signal* — phase-07b.
- **Changing any signal's sample extractor** (the `samples()` math). Only the
  *percentile selection + labelling* changes.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
