# Phase 07b: Add the output-flood calibration signal (`calibrate-governor` reads `output_bytes`)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-07a
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

`rexymcp calibrate-governor` replays session logs to calibrate every governor stall
threshold — **except** the output-flood detector. The replay currently **drops**
`ToolResult` events (`_ => {}`), so the `output_bytes` field phase-02 added is never
read, and there is no signal to calibrate the windowed-output-flood limit against. This
phase adds that signal: the replay collects per-`ToolResult` `output_bytes`, and a new
`Signal` reports the **max windowed output-bytes sum per run** — mirroring the live
`check_windowed_output` detector — so an operator can set `[governor] output_window_bytes`
from real data.

It is **higher-is-worse** (a big flood is bad), so it flows through the direction-aware
percentile path phase-07a just added with **no report-layer change** — it reports
p50/p90/p99 automatically.

## Architecture references

Read before starting:

- `mcp/src/calibrate_governor.rs` — the whole change lives here. Post-07a state quoted
  in Current state: `RunReplay` (~18–28), `replay()`'s event `match` with the
  `_ => {}` that drops `ToolResult` (~431), the `Signal` enum + `label`/`direction`/
  `samples`, `SIGNALS`, and `format_report`'s hard-coded signal-label list.
- `executor/src/store/sessions/event.rs:50–59` — the `SessionEvent::ToolResult`
  variant (the `output_bytes` field to read). Quoted below.
- `executor/src/governor/hard_fail.rs:390–408` — `check_windowed_output`, the live
  detector whose semantics the new signal mirrors. Quoted below.

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

**`SessionEvent::ToolResult`** (`executor/src/store/sessions/event.rs:50–59`) — the
event the replay currently drops; `output_bytes` is the field to read:

```rust
ToolResult {
    name: String,
    succeeded: bool,
    output_preview: String,
    /// Full byte length (`content.len()`) of the tool output before truncation.
    /// `0` for records written before this field existed. The output-flood
    /// calibration signal reads this.
    #[serde(default)]
    output_bytes: u64,
},
```

**`RunReplay`** (`calibrate_governor.rs:~18–28`) — the per-run replay struct; it has
`tool_calls`, `verifier_error_counts`, `completion_empty`, but **no output-bytes**:

```rust
struct RunReplay {
    model: String,
    outcome: String,
    tool_calls: Vec<ToolCallSnapshot>,
    verifier_error_counts: Vec<usize>,
    completion_empty: Vec<bool>,
}
```

**`replay()`** (`calibrate_governor.rs:~406–441`) — builds `RunReplay` from the event
stream; the `_ => {}` arm is where `ToolResult` is dropped today:

```rust
for rec in records {
    match &rec.event {
        SessionEvent::SessionStart { model: m, .. } => model = m.clone(),
        SessionEvent::SessionEnd { status, .. } => outcome = status.clone(),
        SessionEvent::Parsed { tool_call } => tool_calls.push(/* ... */),
        SessionEvent::Verify { diagnostics } => verifier_error_counts.push(diagnostics.len()),
        SessionEvent::Completion { raw } => completion_empty.push(/* ... */),
        _ => {}   // <-- ToolResult falls here today
    }
}
```

**The live detector** (`executor/src/governor/hard_fail.rs:390–408`) — the semantics to
mirror: the **sum of the last `window` tool outputs** exceeds `limit`:

```rust
pub fn check_windowed_output(
    recent_output_bytes: &VecDeque<usize>,
    window: usize,
    limit: usize,
) -> Option<HardFailSignal> {
    if window == 0 || recent_output_bytes.len() < window {
        return None;
    }
    let bytes: usize = recent_output_bytes.iter().rev().take(window).sum();
    if bytes > limit { Some(HardFailSignal::CumulativeOutputFlood { window, bytes }) } else { None }
}
```

Its config defaults (`executor/src/config.rs:314–315`): `output_window = 6`,
`output_window_bytes = 256 * 1024`. The calibration signal uses a fixed window constant
of **6** to match `output_window`'s default (exactly as the oscillation signal uses a
fixed `OSCILLATION_WINDOW = 8`, independent of live config).

**The oscillation `samples()` arm** is the worked example for a sliding-window extractor
(min over windows); the new signal is the **max over windows of the window *sum***:

```rust
Signal::OscillationMinDistinct => {
    let calls = &run_.tool_calls;
    if calls.len() < OSCILLATION_WINDOW { return vec![]; }
    let mut min = usize::MAX;
    for start in 0..=calls.len() - OSCILLATION_WINDOW {
        let window = &calls[start..start + OSCILLATION_WINDOW];
        /* ... count distinct ... */
        min = min.min(distinct.len());
    }
    vec![min]
}
```

**`Signal::direction`** (added in 07a) already returns `HigherIsWorse` for the `_` arm,
so a new higher-is-worse signal needs **no** `direction` change and reports p50/p90/p99
automatically.

## Spec

### Task 1 — collect `output_bytes` in the replay (calibrate_governor.rs)

Add `output_bytes: Vec<usize>` to `RunReplay`. In `replay()`, add a match arm for
`SessionEvent::ToolResult { output_bytes, .. }` that pushes `*output_bytes as usize`
onto the vec (in log order — the sequence matters for the windowed sum). Initialize the
accumulator alongside the others and move it into the returned `RunReplay`. The
`_ => {}` arm stays for the remaining unhandled events.

### Task 2 — add the `OutputFloodWindowedBytes` signal (calibrate_governor.rs)

Add a fixed `const OUTPUT_FLOOD_WINDOW: usize = 6;` (mirrors the default `output_window`;
put it next to `OSCILLATION_WINDOW`). Then wire the new signal through **every** place
the other signals appear — miss one and it silently won't render:

1. **`Signal` enum** — a new variant, e.g. `OutputFloodWindowedBytes`.
2. **`Signal::label`** — a stable snake_case label, e.g. `"output_flood_windowed_bytes"`.
3. **`Signal::samples`** — a new arm computing the **max windowed sum**:
   ```rust
   Signal::OutputFloodWindowedBytes => {
       let ob = &run_.output_bytes;
       if ob.len() < OUTPUT_FLOOD_WINDOW { return vec![]; }
       let mut max = 0usize;
       for start in 0..=ob.len() - OUTPUT_FLOOD_WINDOW {
           let sum: usize = ob[start..start + OUTPUT_FLOOD_WINDOW].iter().sum();
           max = max.max(sum);
       }
       vec![max]
   }
   ```
   (One sample per run — the run's worst 6-output window. Requires a full window, like
   the oscillation signal; a run with fewer than 6 `ToolResult`s yields no sample.)
4. **`SIGNALS`** — add the variant to the slice so `run()` aggregates it.
5. **`format_report`'s hard-coded signal-label list** — add
   `"output_flood_windowed_bytes"` so its block renders. (This is the easy one to
   forget — the list is separate from `SIGNALS`.)
6. **`Signal::direction`** — **no change needed**; the `_ => HigherIsWorse` arm covers
   it. Do not add a `LowerIsWorse` case.

No change to `ReportRow`, the build sites, or the percentile routing — a higher-is-worse
signal flows through the 07a path and reports p50/p90/p99.

## Acceptance criteria

- [ ] `replay()` collects `ToolResult.output_bytes` into `RunReplay.output_bytes` in log
      order (a `ToolResult` event no longer falls through `_ => {}`).
- [ ] `Signal::OutputFloodWindowedBytes.samples()` returns the **max** windowed sum over
      windows of size 6: for `output_bytes = [10,20,30,40,50,60,70]`, the sample is the
      max of the two 6-windows (`[10..60]=210`, `[20..70]=270`) → **`270`**. A run with
      `< 6` `ToolResult`s yields **no** sample (`vec![]`).
- [ ] The signal is **higher-is-worse**: `Signal::OutputFloodWindowedBytes.direction()`
      == `HigherIsWorse`, and its report row's `tail` is `"higher-is-worse"` with p90/p99
      as the concerning columns.
- [ ] The `output_flood_windowed_bytes` block appears in both the text report (a header
      block with `P50 P90 P99`) and the JSON output.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] No change to the live detector, the executor loop, the event schema, or the other
      six signals' reported values.

## Test plan

Extend the inline `#[cfg(test)] mod tests` in `mcp/src/calibrate_governor.rs`. The
existing `make_session_file` helper only emits `Parsed` events — the replay/collection
test must build `SessionRecord`s with `SessionEvent::ToolResult` **inline** (pin the
variant shape from event.rs above). Pin behavior + names:

- `output_flood_windowed_max_over_run` — call `Signal::OutputFloodWindowedBytes.samples()`
  on a `RunReplay` with `output_bytes = [10,20,30,40,50,60,70]`; assert `vec![270]`.
- `output_flood_requires_full_window` — `output_bytes` of length 5 → `vec![]`.
- `output_flood_direction_is_higher_is_worse` — assert the new signal's `direction()` is
  `HigherIsWorse` (guards against someone later mis-tagging it lower-is-worse).
- `replay_collects_tool_result_output_bytes` — build a `SessionRecord` stream containing
  two `SessionEvent::ToolResult { output_bytes: .. }` events (plus a `SessionStart`);
  assert `replay(&records).output_bytes` equals the two byte counts in order. **Pin the
  negative too:** a `ToolResult` with the pre-field default (`output_bytes: 0`) is
  collected as `0`, not skipped.
- `output_flood_signal_appears_in_report` — build runs with enough `ToolResult`s to
  produce samples; assert the JSON report contains a row with
  `signal == "output_flood_windowed_bytes"` and `tail == "higher-is-worse"`.

## End-to-end verification

`calibrate-governor` reads the real session-log corpus, which now carries `output_bytes`
(phase-02 onward). Run it and quote the new block:

```
cargo run -p rexymcp -- calibrate-governor --repo . --min-runs 0
```

Quote the `output_flood_windowed_bytes` block (header `P50 P90 P99`, higher-is-worse)
and one `--json` row showing `"signal": "output_flood_windowed_bytes"` /
`"tail": "higher-is-worse"`. If the local corpus has too few runs with ≥6 `ToolResult`s
to populate a row, say so and rely on the unit tests as the pinned evidence (quote
`output_flood_windowed_max_over_run` + `replay_collects_tool_result_output_bytes`).

(Note: the `calibrate-governor` CLI flag is `--repo <dir>` reading `<dir>/.rexymcp/sessions`,
**not** `--config`.)

## Authorizations

- Editing `mcp/src/calibrate_governor.rs` is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No edits to `executor/**` — the event schema and detector are **read-only references**
  here; do not touch them.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **The live output-flood detector** (`check_windowed_output`) and its config
  (`output_window`, `output_window_bytes`, `runaway_output_bytes`). This phase adds the
  *calibration signal* only; re-tuning the thresholds from the new report is a later
  human decision.
- **A single-call output signal** (mirroring `runaway_output_bytes` / `check_runaway_output`).
  One windowed signal is enough for this phase; a per-`ToolResult`-max signal, if wanted,
  is separate.
- **calibrate-governor rendering alignment** (moving `percentile` to the shared
  `metrics.rs`, scorecard/runs table conventions) and the **discoverability** pass — that
  is phase-07c, which closes M35.
- **Changing the 07a percentile/direction machinery.** The new signal reuses it
  unchanged.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — 2026-07-22 (escalation)

**Chosen lever:** resume (`continue_phase`)
**Rationale:** The spec was not the problem — the implementation is essentially complete
(all six wiring points present: `OutputFloodWindowedBytes`, its label, `OUTPUT_FLOOD_WINDOW`,
`SIGNALS`, `format_report`'s label list, and `samples`; `RunReplay.output_bytes` collected;
tests written). The `hard_fail` was the governor's oscillation terminator
(`distinct_calls: 2, window: 8`) firing after the executor broke brace-balance mid-edit —
the file has **exactly one missing `}`** (288 open vs 287 close, cargo reports an unclosed
delimiter near the `mod tests` block / the `min_runs_drops_thin_per_model_cells_into_global`
test) — and then **looped ~12× on an identical `python3 -c` file-inspection command** trying
to locate it. Work worth preserving + one mechanical wall = resume, not re-dispatch or
takeover. Guidance steers to `cargo build` once for the exact location and away from the
repeated-inspection loop that tripped the terminator.

**Calibration (second occurrence — a trend now):** this is the **second** M35-arc
`hard_fail` where the executor escaped to a repeated read-only *shell inspection* loop
(`python3 -c` here; `sed -i`/`sed -n` in the 06c arc) after patch edits drifted its model of
the file. One occurrence was data; two is a trend worth folding at M35 close (reinforces the
held "make read-only-inspection repetition advisory / raise its threshold" fold).
### Update — ts=1784752576058 (complete, server-authored)

**Summary:** Summary: The prior run left the file with three issues — a missing closing brace (from a broken brace-balance mid-edit), a duplicated `fn min_runs_drops_thin_per_model_cells_into_global()` declaration, and a test helper `make_replay_with_verify_and_completion` that discarded both `completion_empty` and `output_bytes` (causing 2 pre-existing tests to fail). I fixed all three: added the missing `}`, removed the duplicate declaration, and restored the helper to properly forward `completion_empty`. The output-flood signal implementation was already complete — all six wiring points (Signal variant, label, const, SIGNALS entry, format_report label list, and samples arm) plus the replay collection and tests were already on disk.

All gates pass: `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` (611 passed). The `calibrate-governor` CLI now reports `output_flood_windowed_bytes` with P50/P90/P99 percentiles and `higher-is-worse` tail direction, populated from real session data.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
e_ids ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1031 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.38s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/calibrate_governor.rs` — +4 -5

**Commit:** 6d9e7d32f4784702d631292ac15a67aefe862c25

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-22

- **Verdict:** approved_after_1
- **Bounces:** 1 — one escalation round-trip (hard_fail → resume), no review-bug filed.
  Failure class: `oscillation_stall` (governor oscillation terminator fired on a repeated
  `python3 -c` inspection loop after a mid-edit brace break). Note: `oscillation_stall`
  is **not** yet in the canonical `FAILURE_CLASSES` vocabulary (telemetry.rs:319) — it is
  recorded as a new open-vocabulary class, which is the M35-close fold this run motivates.
- **Executor:** AEON-7/Qwen3.6-27B-AEON (first run hard_fail @ ~275 attempts; resume
  complete @ 25 turns)
- **Scope deviations:** none — the final diff is `mcp/src/calibrate_governor.rs` only
  (+186 net), exactly Tasks 1–2. `executor/**` (event schema, live detector) read-only as
  required; the 07a percentile/direction machinery reused unchanged. All six wiring points
  present (enum/label/`OUTPUT_FLOOD_WINDOW`/`samples`/`SIGNALS`/`format_report` list) and
  `RunReplay.output_bytes` collected in `replay()`.
- **Verification:** reviewer re-ran all four gates green (fmt/build/clippy; `611` mcp-bin +
  `1031` executor-lib). New tests (5) pass and are real: `output_flood_windowed_max_over_run`
  asserts `vec![270]` for `[10..70]` (mutation-sensitive — min or first-window gives 210);
  `output_flood_requires_full_window` → empty for 5 inputs; `replay_collects_tool_result_output_bytes`;
  direction + report-appearance. **Real-artifact E2E:** `calibrate-governor --repo .` renders
  the `output_flood_windowed_bytes` block with `P50 P90 P99`; `--json` rows carry
  `"tail":"higher-is-worse"` with real percentiles (e.g. p_near 22166 / p_far 44821) populated
  from corpus `output_bytes`.
- **Calibration:**
  1. **Shell-inspection oscillation loop — SECOND occurrence (a trend).** The hard_fail was
     the executor escaping to a repeated read-only shell inspection (`python3 -c` file reads)
     after patch-drift confused its model of the file, exactly the class of the 06c-arc `sed`
     loop. Two occurrences → fold at M35 close: the held "make read-only-inspection repetition
     advisory / raise its threshold" governor fold, and pre-inject "use the compiler error to
     locate a brace/syntax problem; never hunt by re-reading the file" into future specs.
     Also motivates adding `oscillation_stall` (or a `governor_stall`) entry to `FAILURE_CLASSES`.
  2. **Minor test-coverage nit:** `replay_collects_tool_result_output_bytes` pins `100`/`250`
     but not the spec's requested `output_bytes: 0` negative (that a pre-field-default `0` is
     collected as `0`, not skipped). Harmless — the collection pushes unconditionally — but the
     negative case is unpinned.
  3. **Status-flip header clobber did NOT recur** — 07b's header is intact through both the
     hard_fail and the resume.

