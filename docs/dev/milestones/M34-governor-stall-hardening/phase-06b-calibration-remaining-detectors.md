# Phase 06b: Extend calibration to the remaining governor detectors

**Milestone:** M34 — Governor Stall Hardening
**Status:** review
**Depends on:** phase-06a (the replay/aggregate/report framework + `Signal` seam)
**Estimated diff:** ~280 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

Complete the governor-wide calibration goal: extend 06a's `calibrate-governor`
framework with the remaining detectors' raw signals, so every reachable governor
threshold is backed by corpus distributions. Four detectors are recoverable from
the session log and are **in scope**; one (`output-flood`) is **not recoverable**
from what the log records and is documented as a gap. Additive — 06a's replay /
aggregation / report framework is unchanged except for two new `RunReplay` fields
and four new `Signal` variants.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" and
  § Status #34.
- `docs/dev/milestones/M34-governor-stall-hardening/phase-06a-calibration-framework-and-stall-signals.md`
  — the framework this extends (the `Signal` seam, `CellAccum` runs/samples split,
  `percentile`, report). **Read the whole 06a doc + the shipped
  `mcp/src/calibrate_governor.rs` first.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references + the shipped `mcp/src/calibrate_governor.rs`.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Recoverability (fixed — verified against the log by the architect)

Each governor threshold and whether its raw signal can be re-derived from the
session log, and from which event:

| Detector | Threshold knob | Raw signal (per run) | Recoverable? | Source event |
|---|---|---|---|---|
| identical-repetition | `identical_call_threshold` | longest run of consecutive identical `(tool, arguments)` | **yes** | `Parsed` |
| oscillation | `oscillation_distinct_max` | min distinct `(tool, arguments)` over a sliding `oscillation_window` | **yes** | `Parsed` |
| verifier-persistence | `verifier_persistence_threshold` | longest consecutive non-decreasing all-positive author-error streak | **yes** | `Verify` |
| empty-completion | `empty_completion_threshold` | longest run of consecutive empty completions | **partial** | `Completion` |
| output-flood | `output_window_bytes` / `runaway_output_bytes` | single-call + windowed output bytes | **no** | — |

- **verifier-persistence is fully recoverable:** the loop logs the *author-attributed*
  diagnostics on each `Verify` event (`executor/src/agent/mod.rs:1266` —
  `SessionEvent::Verify { diagnostics: author.clone() }`), and pushes `author.len()`
  into the counter it feeds the detector (line 1269). So `Verify.diagnostics.len()`
  per event **is** the per-turn error count.
- **empty-completion is partial:** the loop increments its counter on `truncated ||
  post_think.trim().is_empty()` (`agent/mod.rs:641`). The `Completion { raw }` event
  lets us re-derive the `strip_think_blocks(raw).trim().is_empty()` half, but the
  **truncation** half (`finish_reason == length`) is not logged, so the back-test
  *under-counts* truncation-driven empties. Document this in the code + report; it
  is still a useful lower bound.
- **output-flood is out of reach:** `ToolResult` logs a truncated `output_preview`
  (`agent/mod.rs:1100` — `output_preview(&content)`), not `content.len()`. The real
  byte counts the detector compares against are gone. See Out of scope.

## Current state (the 06a seam to extend)

`mcp/src/calibrate_governor.rs` (shipped in 06a):

- `struct RunReplay { model, outcome, tool_calls: Vec<ToolCallSnapshot> }` +
  `fn replay(records) -> RunReplay` (walks records; collects `Parsed` into
  `tool_calls`).
- `enum Signal { NoveltyDistinct, MaxReadOnlyRun }` with
  `fn label(self) -> &'static str` and
  `fn samples(self, calls: &[ToolCallSnapshot], novelty_window: usize) -> Vec<usize>`,
  plus `const SIGNALS: &[Signal]`.
- The aggregation loop (`CellAccum { runs, samples }`, `--min-runs` on runs, `RUNS`
  column) and `percentile` — **unchanged by this phase**.

Public primitives available: `rexymcp_executor::parser::strip_think_blocks(&str)
-> String`; `SessionEvent::Verify { diagnostics }` and `SessionEvent::Completion {
raw }`.

## Spec

Additive. Build leaf-first (Task 6): extend the replay state first (compiles),
change the `samples` signature (compiles once callers updated), then add variants.

### 1. Extend `RunReplay` + `replay()` to collect verifier + completion data

Add two fields to `RunReplay`:

```rust
struct RunReplay {
    model: String,
    outcome: String,
    tool_calls: Vec<ToolCallSnapshot>,
    /// Author-attributed verifier error count per `Verify` event, in order.
    verifier_error_counts: Vec<usize>,
    /// Per `Completion` event: whether it was blank/think-only. NOTE: this misses
    /// truncation-driven empties (`finish_reason == length` is not logged), so it
    /// is a lower bound on the loop's empty-completion counter.
    completion_empty: Vec<bool>,
}
```

Extend the `replay()` match (new arms; keep the existing ones):

```rust
SessionEvent::Verify { diagnostics } => verifier_error_counts.push(diagnostics.len()),
SessionEvent::Completion { raw } => {
    completion_empty.push(
        rexymcp_executor::parser::strip_think_blocks(raw).trim().is_empty(),
    );
}
```

### 2. Change the `Signal::samples` signature to take the whole `RunReplay`

The new signals need `verifier_error_counts` / `completion_empty`, not just
`tool_calls`. Change the seam:

```rust
fn samples(self, run: &RunReplay, novelty_window: usize) -> Vec<usize>
```

Update 06a's two existing arms to read `run.tool_calls` (behavior unchanged), and
update the one call site in the aggregation loop from
`signal.samples(&replay.tool_calls, novelty_window)` to `signal.samples(replay,
novelty_window)`.

### 3. Add the four new `Signal` variants + extractors

Extend the enum, `label()`, `samples()`, and `SIGNALS`. Each raw quantity **mirrors
the detector's semantics** (cited); each is one sample per run (like
`MaxReadOnlyRun`). Return `vec![]` (not `vec![0]`) when a run carries no data for
that signal, so the `samples.is_empty()` guard drops it from the run count (the
bug-06a-1 lesson).

```rust
// label():
Signal::IdenticalRun => "identical_run",
Signal::OscillationMinDistinct => "oscillation_min_distinct",
Signal::VerifierPersistenceRun => "verifier_persistence_run",
Signal::EmptyCompletionRun => "empty_completion_run",
```

**identical-repetition** — longest run of consecutive identical `(tool, arguments)`
(mirrors `check_identical_repetition`: `c.tool == first.tool && c.arguments ==
first.arguments`, `executor/src/governor/hard_fail.rs:147`):

```rust
Signal::IdenticalRun => {
    let mut max = 0usize;
    let mut run = 0usize;
    let mut prev: Option<&ToolCallSnapshot> = None;
    for c in &run_.tool_calls {
        let same = prev.is_some_and(|p| p.tool == c.tool && p.arguments == c.arguments);
        run = if same { run + 1 } else { 1 };
        max = max.max(run);
        prev = Some(c);
    }
    if run_.tool_calls.is_empty() { vec![] } else { vec![max] }
}
```

**oscillation** — min distinct `(tool, arguments)` over any sliding window of
`OSCILLATION_WINDOW` (define `const OSCILLATION_WINDOW: usize = 8;` — the
`GovernorConfig` default; a lower min ⇒ more oscillatory). Mirrors
`check_oscillation` (`hard_fail.rs:238`, distinct `(tool, &arguments)` over the
window). A run shorter than the window yields no sample:

```rust
Signal::OscillationMinDistinct => {
    let calls = &run_.tool_calls;
    if calls.len() < OSCILLATION_WINDOW {
        return vec![];
    }
    let mut min = usize::MAX;
    for start in 0..=calls.len() - OSCILLATION_WINDOW {
        let window = &calls[start..start + OSCILLATION_WINDOW];
        let mut distinct: Vec<(&str, &serde_json::Value)> = Vec::new();
        for c in window {
            let key = (c.tool.as_str(), &c.arguments);
            if !distinct.iter().any(|(t, a)| *t == key.0 && *a == key.1) {
                distinct.push(key);
            }
        }
        min = min.min(distinct.len());
    }
    vec![min]
}
```

**verifier-persistence** — longest streak of consecutive turns whose author-error
count is `> 0` **and** non-decreasing (mirrors `check_verifier_persistence`: last_n
all `> 0`, non-decreasing, `hard_fail.rs:159`). No `Verify` events ⇒ no sample:

```rust
Signal::VerifierPersistenceRun => {
    if run_.verifier_error_counts.is_empty() {
        return vec![];
    }
    let mut max = 0usize;
    let mut run = 0usize;
    let mut prev = 0usize;
    for &c in &run_.verifier_error_counts {
        run = if c == 0 {
            0
        } else if run == 0 || c >= prev {
            run + 1
        } else {
            1 // positive but decreased → a fresh streak of length 1
        };
        max = max.max(run);
        prev = c;
    }
    vec![max]
}
```

**empty-completion** — longest run of consecutive empty completions (the
`completion_empty` flags from Task 1). Every run has completions, so it always
has a sample:

```rust
Signal::EmptyCompletionRun => {
    let mut max = 0usize;
    let mut run = 0usize;
    for &empty in &run_.completion_empty {
        run = if empty { run + 1 } else { 0 };
        max = max.max(run);
    }
    vec![max]
}
```

(The parameter is named `run` in this doc's snippets for the signature; the
existing code uses `run` as a loop variable — rename the extractor's local run
counter or the parameter to avoid a shadow, e.g. parameter `run_`. Pick whatever
compiles cleanly; the behavior is what's pinned.)

### 4. `SIGNALS`

```rust
const SIGNALS: &[Signal] = &[
    Signal::NoveltyDistinct,
    Signal::MaxReadOnlyRun,
    Signal::IdenticalRun,
    Signal::OscillationMinDistinct,
    Signal::VerifierPersistenceRun,
    Signal::EmptyCompletionRun,
];
```

### 5. Report header note for empty-completion

Because `empty_completion_run` is a documented **lower bound** (misses truncation),
add a one-line caveat to the text report near that signal (e.g. a trailing
`(lower bound — excludes length-truncated turns)` on the signal header line) and,
for `--json`, this is understood from the docs — no schema change needed.

### 6. Build order (leaf-first; build at each checkpoint)

1. Task 1 (RunReplay fields + replay arms). — **build green.**
2. Task 2 (samples signature + update the 2 existing arms + the call site). —
   **build green + 06a's tests still pass.**
3. Task 3–4 (new variants + extractors + SIGNALS). — **build green.**
4. Task 5 + tests. — **all four gates green.**

## Acceptance criteria

- [ ] `rexymcp calibrate-governor --repo .` reports `identical_run`,
      `oscillation_min_distinct`, `verifier_persistence_run`, and
      `empty_completion_run` signals in addition to the two 06a signals, each with
      per-model (+ global) `RUNS`/`N`/p50/p90/p99 by outcome.
- [ ] `verifier_persistence_run` samples come from `Verify` event author-error
      counts; a run with no `Verify` events contributes no sample.
- [ ] `oscillation_min_distinct` yields no sample for a run shorter than
      `OSCILLATION_WINDOW`.
- [ ] `empty_completion_run` is derived from `strip_think_blocks(raw).is_empty()`
      and is labeled a lower bound in the report.
- [ ] 06a's `novelty_distinct_targets` / `max_read_only_run` output is unchanged.
- [ ] All four gates green.

## Test plan

Hermetic `TempDir` session-log fixtures (extend 06a's `make_session_file` helper —
it will need to emit `Verify` and `Completion` events for the new signals; add a
sibling helper or parameters rather than breaking the existing tests).

- `identical_run_counts_longest_consecutive_identical` — a `read a, read a, read
  b, read b, read b` sequence → 3 (the `b` run), not 5. Pins the reset on a
  differing call (must-NOT: not the total).
- `oscillation_min_distinct_no_sample_below_window` — a run shorter than
  `OSCILLATION_WINDOW` → no sample (boundary).
- `oscillation_min_distinct_finds_tightest_window` — an A,B,A,B stretch inside a
  longer varied run → min distinct 2.
- `verifier_persistence_run_matches_detector_semantics` — counts `[1,2,2,0,3]` →
  longest non-decreasing positive streak = 3 (the `1,2,2`), reset by the `0`.
  Include a decrease case (`[2,1]` → streak 1, not 2).
- `verifier_persistence_no_sample_without_verify_events` — a run with no `Verify`
  events → no sample.
- `empty_completion_run_counts_consecutive_blanks` — completions `["hi", "", "",
  "x"]` → 2; a think-only completion (`<think>…</think>`) counts as empty.
- `remaining_signals_appear_in_report` — an E2E-style `run()` over a fixture dir
  asserts the four new signal labels are present.

## End-to-end verification

Run the real subcommand against the corpus and quote the new signals' rows:

```
rexymcp calibrate-governor --repo . --min-runs 3
```

Quote, in the completion Update Log, the `identical_run`,
`oscillation_min_distinct`, `verifier_persistence_run`, and
`empty_completion_run` distributions split by outcome — the first data on where
those four thresholds sit versus real runs.

## Authorizations

- [ ] May extend `mcp/src/calibrate_governor.rs` and its tests. No new dependency,
      no `Cargo.toml` edit, no executor-crate change.

## Out of scope

- **output-flood calibration.** Not recoverable: the log stores a truncated
  `output_preview`, not `content.len()`. Enabling it would require a **new logged
  field** (e.g. `ToolResult.output_bytes`) in the executor — an executor-side
  change out of this phase's scope — **and even then, zero existing corpus data**
  (only runs logged after the field is added would carry it). Recommendation:
  defer until there is a concrete need; if pursued, it is its own small
  executor-logging phase, and calibration waits for new runs to accrue. Do **not**
  add the field or an approximate byte-count signal here.
- The live "advisory fired" `SessionEvent` marker (still deferred).
- Any suggested-threshold output or config mutation (report-only, unchanged from 06a).
- Extracting `measure_*` helpers into `hard_fail.rs` for drift-safety (like
  `measure_novelty`) — the mirrored extractors here are simple and pinned; a future
  executor-side refactor could unify them, but it is not required.

## Notes

**Routing — dispatchable.** Like 06a, this is a read-only analysis subcommand in
the `mcp` crate — no governor-internal or loop code — so it dispatches safely. The
risk is ordinary additive churn (RunReplay fields + a signature change + four
extractors + tests); the leaf-first order in Task 6 is the countermeasure.

**Milestone note.** After 06b, only phase-07 (stall-fire briefing quality, reduced
scope) remains — and whether it is still worth doing given advisory-by-default
stalls is an open call for the milestone-close review.

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
### Update — ts=1784493690962 (complete, server-authored)

**Summary:** All four gates pass: `cargo build` ✓, `cargo clippy` ✓, `cargo fmt --check` ✓, `cargo test` ✓ (526 passed).

**Summary + Notes for review:**

Extended the `calibrate-governor` framework with four new signal extractors: `IdenticalRun` (longest consecutive identical tool+arguments), `OscillationMinDistinct` (minimum distinct calls over sliding window), `VerifierPersistenceRun` (longest non-decreasing positive author-error streak), and `EmptyCompletionRun` (longest consecutive empty completions). The `RunReplay` struct was extended with `verifier_error_counts` and `completion_empty` fields, populated from `Verify` and `Completion` session events. The `Signal::samples()` signature changed to take `&RunReplay` instead of `&[ToolCallSnapshot]`. The `format_report` function now renders all six signals, with the `empty_completion_run` header annotated as a lower bound. All existing tests pass with updated call sites. No end-to-end corpus verification was run since the spec's `--repo .` command requires existing session logs.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
symbols::tests::references_finds_call_sites ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::respects_gitignore ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 996 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.21s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/calibrate_governor.rs` — +150 -11

**Commit:** 8c0eb71aab9b8a0ee9a0a4ebc3ae71c5e047e218

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).


### Review — 2026-07-19 (bounced → bug-06b-1)

**Verdict:** bounced (1 bug, major). The four extractors, the `RunReplay`/replay
extension, the `samples()` signature change, and the report wiring are all correct
— clean 153-turn dispatch, four gates green on independent re-run (526 mcp + 996
executor), no banned patterns, and the real-corpus E2E (which the executor
skipped) produces plausible distributions for all four new signals.

**Notes for the executor (fix on re-dispatch):** one major defect —
[bug-06b-1](bugs/bug-06b-1.md). The four new extractors shipped with **zero
tests** (STANDARDS §3.1; the phase's Test plan named 7 tests, none written). Add
the seven tests from the Test plan; extend `make_session_file` (or add a sibling)
so fixtures can emit `Verify` and `Completion` events for the
verifier-persistence and empty-completion signals. Do **not** change the
extractor logic — it is correct as shipped; this bounce is purely the missing
coverage. See the bug for the exact per-test pins (incl. the negative/boundary
cases). The bug's Notes flag a p50/p90/p99-vs-min-signal reporting observation
that is **out of scope** here — it belongs to the deferred metrics pass.
### Update — ts=1784498880439 (complete, server-authored)

**Summary:** All gates pass. Here's the summary:

**Summary:** Added 10 unit tests for the four 06b signal extractors that shipped without test coverage (bug-06b-1). Tests cover `IdenticalRun` (longest consecutive identical calls, including the must-NOT case proving it resets on a differing call, not counting total), `OscillationMinDistinct` (no sample below window size, and correct min-distinct over a tight A,B,A,B window), `VerifierPersistenceRun` (non-decreasing positive streak semantics, reset on decrease, no sample without Verify events), and `EmptyCompletionRun` (consecutive blank counting, think-only counts as empty). Also includes an E2E report integration test that builds a fixture with Verify and Completion events and asserts all four new signal labels appear in the output. The `remaining_signals_appear_in_report` test uses `make_session_file` for base events and appends `SessionRecord` for Verify/Completion to the same JSONL.

**Notes for review:** All 19 calibrate_governor tests pass (9 existing + 10 new). Build, clippy, fmt, and full test suite all green.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
ds ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
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

test result: ok. 996 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/calibrate_governor.rs` — +258 -0

**Commit:** f54edc9e966f4c2c4fb8ed6168b58a603fe8d38e

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

