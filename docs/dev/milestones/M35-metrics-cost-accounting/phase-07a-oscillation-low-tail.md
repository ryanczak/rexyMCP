# Phase 07a: Calibration reports the low tail for the lower-is-worse oscillation signal

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
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
### Update — ts=1784739048935 (complete, server-authored)

**Summary:** All verification passes. Here's the summary:

**What was built:** The calibration report now correctly reports low-tail percentiles (P50/P10/P1) for the `oscillation_min_distinct` signal (lower-is-worse) while keeping high-tail (P50/P90/P99) for the other five signals.

**Key changes to `mcp/src/calibrate_governor.rs`:**
- Added `TailDirection` enum (`HigherIsWorse` / `LowerIsWorse`) and `Signal::direction()` — only `OscillationMinDistinct` returns `LowerIsWorse`.
- `ReportRow` renamed fields from `p50/p90/p99` to `p_mid/p_near/p_far` with a `tail` string field, making JSON output self-describing.
- Build sites compute direction-appropriate percentiles: low-tail (p10/p01) for oscillation, high-tail (p90/p99) for others.
- `format_report` renders `P10/P1` headers for oscillation blocks with a "lower-is-worse" note, `P90/P99` for others.
- Added 4 tests covering direction mapping, low-tail reporting, high-tail preservation, and header labeling.

**End-to-end verification:** Live corpus run confirms oscillation block shows `P50 P10 P1` headers with a "lower-is-worse" annotation, while other signals (e.g., `identical_run`) still show `P50 P90 P99`. JSON output includes `"tail": "lower-is-worse"` on oscillation rows. All 1031 tests pass, build/lint/format are clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
t tools::symbols::tests::references_finds_call_sites ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1031 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/calibrate_governor.rs` — +222 -11

**Commit:** 7cd66b495083c296dbbf68a0fa18bdf0f0042a25

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

