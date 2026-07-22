# Phase 07c: calibrate-governor rendering alignment (shared `percentile`) + command discoverability

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
**Depends on:** phase-07b
**Estimated diff:** ~180 lines
**Tags:** language=rust, kind=refactor, size=m

## Goal

Two closing items for M35's reporting debt:

1. **Rendering alignment (exit criterion):** `calibrate-governor` carries its **own**
   `percentile` helper — an un-shared duplicate of a derivation that belongs in the
   shared metrics module every other reader uses. Move it to
   `executor/src/store/metrics.rs` and route `calibrate_governor` through it, so
   `calibrate-governor` aligns with the shared rendering like `runs`/`scorecard`/`costs`.
2. **Discoverability:** the metrics/reporting commands don't reference each other, so
   `calibrate-governor` in particular is an **orphan** — nothing points a user to it.
   Add a **"See also"** cross-reference to each reporting command's `--help` so the
   family (`runs` / `scorecard` / `profile` / `costs` / `calibrate-governor`) is mutually
   discoverable.

This is the **last M35 phase** — after it lands and is approved, the milestone hits its
boundary (retrospective + calibration folds, a separate human-gated step).

## ⚠️ Read this first — the oscillation gotcha (phase-07b hard-failed here)

`mcp/src/calibrate_governor.rs` is the exact file **phase-07b oscillation-hard-failed
on**: after some `patch` edits, the executor lost track of the file's brace balance and
then **re-ran the same `python3 -c` file-inspection command ~12 times** hunting for the
problem — the governor's oscillation detector terminated the run.

**Do not repeat that.** If a build/syntax error appears after an edit:

1. Run `cargo build -p rexymcp` **once** and read the compiler's error location (file:line).
2. Open **only that region** with `read_file` (`start_line`/`end_line`) **once**.
3. Fix with `patch`, then `cargo build` again.

**Never** locate a syntax/brace problem by re-reading the whole file in a loop with
`python3 -c`, `bash`, `sed`, or `cat` — repeating the same read-only inspection command is
what trips the oscillation terminator. **Let the compiler tell you where the problem is.**
Read once, fix once, verify.

## Architecture references

Read before starting:

- `executor/src/store/metrics.rs` — the shared derivations module ("the single home for
  every *derived* metric number"). Public helpers at lines 11/20/30/42/54; `#[cfg(test)]
  mod tests` at ~64. `percentile` will be added here. It has **no** percentile today.
- `mcp/src/calibrate_governor.rs` — the local `percentile` (nearest-rank) and its call
  sites, quoted in Current state.
- `mcp/src/main.rs` — the `Commands` enum (~90); the reporting-command variants whose
  doc-comments (clap `about`/long-help) get the "See also" line: `Runs` (~157),
  `Scorecard` (~188), `Profile` (~216), `Costs` (~271), `CalibrateGovernor` (~407).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc — **including the oscillation gotcha above** — before
   touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Editing discipline:** edit with `patch`/`patch_lines`, **never a whole-file
   `write_file`**. `cargo build -p rexymcp` after each file. Follow the oscillation gotcha.

## Current state

**The local `percentile`** in `mcp/src/calibrate_governor.rs` (nearest-rank; this is the
duplicate to move):

```rust
fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
```

**Its call sites** — the two `ReportRow` build sites in `run()` each call it three times
(via the 07a direction split), e.g.:

```rust
let (tail_str, p_near, p_far) = if matches!(direction, TailDirection::LowerIsWorse) {
    ("lower-is-worse", percentile(&sorted, 0.1), percentile(&sorted, 0.01))
} else {
    ("higher-is-worse", percentile(&sorted, 0.9), percentile(&sorted, 0.99))
};
let p_mid = percentile(&sorted, 0.5);
```

plus the `percentile_boundaries` unit test that calls it directly.

**`metrics.rs` public-helper shape** (the module `percentile` joins) — e.g.:

```rust
/// Generation throughput in output tokens per second. ...
pub fn tokens_per_sec(output_tokens: u32, gen_time_s: f64) -> Option<f64> { ... }
```

**The reporting commands' doc-comments** in `main.rs` are clap `about` strings, e.g.:

```rust
/// List individual PhaseRun records, or show one in detail
Runs { ... }

/// Calibrate governor thresholds by replaying the session-log corpus
CalibrateGovernor { ... }
```

clap renders the **first** doc-comment line as the short `about` and subsequent lines as
the long `--help` body, so an appended "See also" line shows in `<cmd> --help`.

## Spec

### Task 1 — move `percentile` to the shared metrics module (metrics.rs)

Add to `executor/src/store/metrics.rs` (public, byte-for-byte the same nearest-rank
logic — do **not** change the algorithm):

```rust
/// Nearest-rank percentile of a **sorted** slice. `p` in `0.0..=1.0`. Empty → 0.
/// The one definition of percentile, shared by calibrate-governor's stall-signal report.
pub fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
```

Move the percentile unit test into `metrics.rs`'s `mod tests` (the boundary/low-tail
cases: empty→0, single element, and `percentile(&[1,2,3,4,5,6,7,8], 0.1) == 2` /
`0.01 == 1` / `0.5 == 5` / `0.9 == 7` / `0.99 == 8`).

### Task 2 — route `calibrate_governor` through the shared `percentile` (calibrate_governor.rs)

Delete the local `fn percentile`. Add an import (mirror the existing
`use rexymcp_executor::store::...` imports at the top of the file), then replace every
`percentile(&sorted, X)` call with `metrics::percentile(&sorted, X)` (or a direct
`use rexymcp_executor::store::metrics::percentile;` and leave the call sites unqualified —
executor's call). Remove the now-migrated percentile unit test from this file (it lives
in `metrics.rs` now). **No behavior change** — same values, same report.

### Task 3 — "See also" cross-references for discoverability (main.rs)

Append a **"See also"** line to the doc-comment of each reporting-family command so the
family is mutually discoverable and `calibrate-governor` is no longer an orphan. For each
of `Runs`, `Scorecard`, `Profile`, `Costs`, `CalibrateGovernor`, add a second doc-comment
line naming the *other* four, e.g. for `CalibrateGovernor`:

```rust
/// Calibrate governor thresholds by replaying the session-log corpus
///
/// See also: runs, scorecard, profile, costs.
CalibrateGovernor { ... }
```

Keep the first line (the short `about`) unchanged; the "See also" goes on a later line so
it lands in `<cmd> --help`. Name the sibling **command names** as a user types them
(`runs`, `scorecard`, `profile`, `costs`, `calibrate-governor`) — each command lists the
*other* four. Exact wording is the executor's call; it must name the four siblings.

## Acceptance criteria

- [ ] `rexymcp_executor::store::metrics::percentile` exists (public) and
      `mcp/src/calibrate_governor.rs` no longer defines its own `percentile` — all call
      sites use the shared one.
- [ ] `metrics::percentile` behavior is unchanged: `[1..=8]` gives `0.5→5`, `0.1→2`,
      `0.01→1`, `0.9→7`, `0.99→8`; empty→0; single element→that element.
- [ ] `calibrate-governor`'s report is byte-identical to before the move (same
      percentiles for every signal — this is a pure extract-to-shared-module refactor).
- [ ] `rexymcp calibrate-governor --help` (and `runs`/`scorecard`/`profile`/`costs`
      `--help`) each show a "See also" line naming the other four reporting commands.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] No behavior change beyond the percentile relocation and the help text.

## Test plan

- **`metrics.rs` `mod tests`** — `percentile_nearest_rank` (or keep the moved test's
  name): empty→0; single→element; `[1..=8]` at 0.01/0.1/0.5/0.9/0.99 → 1/2/5/7/8.
- **`calibrate_governor.rs`** — the existing signal/report tests already assert the
  reported percentiles (e.g. `oscillation_report_surfaces_low_tail`,
  `output_flood_windowed_max_over_run`); they must still pass unchanged, proving the
  shared `percentile` produces identical values. No new calibrate_governor test is
  required for the move; do not delete the existing ones.
- The "See also" help text is verified end-to-end (below), not by a unit test (pinning
  exact clap help output is rendering, not behavior).

## End-to-end verification

1. **Alignment (real artifact):** run `cargo run -p rexymcp -- calibrate-governor --repo .
   --min-runs 0` and confirm the report still renders (same shape as phase-07b's E2E:
   the six signal blocks, oscillation showing P50/P10/P1). Quote the top of the output.
2. **Discoverability (real artifact):** run `cargo run -p rexymcp -- calibrate-governor
   --help` and `cargo run -p rexymcp -- costs --help`; **quote the "See also" line** from
   each, confirming each names its four siblings.

## Authorizations

- Editing `executor/src/store/metrics.rs` (add `percentile` + its test),
  `mcp/src/calibrate_governor.rs` (use the shared fn, delete the local + its test), and
  `mcp/src/main.rs` (doc-comment "See also" lines) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **k/M number compaction of calibrate-governor's percentile columns** (e.g. rendering the
  output-flood bytes as `22.2k`). It adds column-width fiddliness to the exact file that
  just oscillated, for a polish gain — **deferred as an M35-close follow-up**, together
  with the related DRY cleanup below. Leave `format_report`'s column formatting as-is.
- **Consolidating the three token/number formatters** (`runs::fmt_tokens` `{}k`,
  `scorecard_cli`'s inline `{:.0}k`, `costs::format_tokens` `{:.1}k`/`{:.1}M`) into one
  shared `metrics` helper. They render *differently*, so merging them changes output and
  their tests — a real DRY cleanup, but out of scope here; note for M35 close.
- **Any change to signal math, the report's tail/direction logic (07a), or the
  output-flood signal (07b).** 07c is a pure percentile-relocation + help-text change.
- **`Calibrate` (tier calibration → config)** — it's a config-writer, not part of the
  reporting family; do not add it to the "See also" set.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784756186961 (complete, server-authored)

**Summary:** All three tasks are complete. Here's the summary:

**What was built:**
1. Moved `percentile` to `executor/src/store/metrics.rs` as a public shared function with unit tests covering empty, single-element, and `[1..=8]` boundary cases (0.01→1, 0.1→2, 0.5→5, 0.9→7, 0.99→8).
2. Routed `calibrate_governor.rs` through the shared `percentile` via `use rexymcp_executor::store::metrics::percentile;`, removing the local duplicate and its migrated test.
3. Added "See also" cross-reference lines to all five reporting commands (`runs`, `scorecard`, `profile`, `costs`, `calibrate-governor`) so each names the other four.

**Verification:**
- `cargo fmt --all --check` — clean
- `cargo clippy` — clean
- `cargo test` — 1032 passed, 0 failed
- `calibrate-governor --help` shows "See also: runs, scorecard, profile, costs." ✓
- `costs --help` shows "See also: runs, scorecard, profile, calibrate-governor." ✓
- `runs --help` shows "See also: scorecard, profile, costs, calibrate-governor." ✓

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
mplete_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.10s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — +1 -1
- `docs/dev/milestones/M35-metrics-cost-accounting/phase-07c-calibrate-alignment-discoverability.md` — +1 -1
- `executor/src/store/metrics.rs` — +26 -0
- `mcp/src/calibrate_governor.rs` — +1 -23
- `mcp/src/main.rs` — +11 -0

**Commit:** d15d0d39293bd3eb45f6c9007aaf87c324f24573

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

