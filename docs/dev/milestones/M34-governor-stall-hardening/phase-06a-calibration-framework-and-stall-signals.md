# Phase 06a: Governor calibration framework + stall-signal report

**Milestone:** M34 — Governor Stall Hardening
**Status:** review
**Depends on:** phase-04 (`measure_novelty`), phase-05 (advisory-demotion — so runs
reach natural length and the corpus reflects real behavior)
**Estimated diff:** ~400 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

Ship the first slice of the governor-wide metrics overhaul: a `rexymcp
calibrate-governor` subcommand that **replays the session-log corpus**
(`.rexymcp/sessions/*.jsonl`), re-derives each governor signal per run, labels
each run by **outcome**, and reports **per-model (with global fallback)**
distributions so a human can set thresholds from data. This phase builds the
replay/report **framework** and wires it for the **two stall signals** (novelty
distinct-targets, longest read-only run); phase-06b extends it to the remaining
detectors. **Report-only** — no auto-suggested numbers (scorecard ethos: telemetry
informs a human decision).

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" (the
  passive-telemetry, human-decision, always-show-sample-size ethos this mirrors)
  and § Status #34.
- `docs/dev/STANDARDS.md` § 3 (hermetic/deterministic tests) and § 2.2 (no
  premature abstraction — the extractor seam is justified by phase-06b, name it).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Design decisions (fixed with the user — do not re-litigate)

- **Scope:** governor-**wide** is the milestone goal, delivered across sub-phases.
  **This phase (06a): the framework + the two stall signals only.** 06b adds
  identical-repetition / oscillation / verifier-persistence / empty-completion /
  output-flood.
- **Ship shape:** a new `rexymcp calibrate-governor` subcommand (repeatable).
- **Granularity:** **per-model, with a global fallback** — always show per-cell
  sample size N; a `--min-runs` floor drops thin per-model cells (they roll into
  the global row).
- **Output:** **report only** (human-readable table + `--json`). Do **not** emit a
  suggested `[governor]` TOML block and do **not** mutate any config.
- **Corpus reach:** **re-derive** each signal from the raw `Parsed` event stream —
  do **not** depend on phase-04's `NoveltySample` events being present (older logs
  predate them; re-deriving also stays correct if `normalize_target` changes).
- **Live advisory marker:** out of scope (deferred; a phase-06b open question).

## Current state

The corpus is real: `<repo>/.rexymcp/sessions/session-<phase>-<id>.jsonl`, ~209
logs in this repo today. The reader and detector primitives already exist and are
public:

- `read_session_log(path: &Path) -> std::io::Result<Vec<SessionRecord>>` —
  `executor/src/store/sessions/jsonl.rs:54`.
- `SessionRecord { ts, turn, event }`; `SessionEvent` variants (serde tag
  `event_type`): `SessionStart { session_id, model, phase }`, `Parsed { tool_call
  }`, `ToolResult { name, succeeded, .. }`, `SessionEnd { status, turns }`
  (`executor/src/store/sessions/event.rs`).
- `parser::ToolCall { name: String, arguments: Value }`
  (`executor/src/parser/mod.rs:52`).
- `hard_fail::ToolCallSnapshot { tool: String, arguments: Value, succeeded: bool }`
  and `hard_fail::measure_novelty(&VecDeque<ToolCallSnapshot>, window) ->
  Option<NoveltyMeasurement { window, distinct_targets }>`
  (`executor/src/governor/hard_fail.rs`).
- `tools::mutates_files(tool: &str) -> bool` (M33 — the write-tool classifier).

The loop builds each snapshot exactly like this — mirror it in replay
(`executor/src/agent/mod.rs:1108`):

```rust
recent_tool_calls.push_back(ToolCallSnapshot {
    tool: tool_call.name.clone(),
    arguments: tool_call.arguments.clone(),
    succeeded,
});
```

Directory-enumeration idiom to mirror (`mcp/src/harvest.rs:96`):

```rust
let entries = match std::fs::read_dir(dir) {
    Ok(e) => e,
    Err(_) => return out,
};
let mut files: Vec<PathBuf> = entries
    .filter_map(|e| e.ok().map(|e| e.path()))
    .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
    .collect();
files.sort();
```

The CLI subcommand pattern to mirror (`mcp/src/main.rs`, the `Runs`/`Scorecard`
variants + dispatch arms): a clap struct variant with `--repo`/`--model`/`--json`
args, a dispatch arm that calls `module::run(...)` and prints the result. `--repo`
is the "target repo root (where `.rexymcp/sessions/` lives)" arg other commands
(`dashboard`, `stop`) already use.

## Spec

New module `mcp/src/calibrate_governor.rs`, a `Commands::CalibrateGovernor` clap
variant + dispatch arm in `mcp/src/main.rs`, and `mod calibrate_governor;` in the
mcp crate root. Build **leaf-first** (Task 6).

### 1. Replay a single session log into a `RunReplay`

```rust
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use rexymcp_executor::governor::hard_fail::{ToolCallSnapshot, measure_novelty};
use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};
use rexymcp_executor::store::sessions::jsonl::read_session_log;
use rexymcp_executor::tools;

/// One replayed run: model, terminal outcome, and the reconstructed tool-call
/// sequence the governor saw. `outcome` is the `SessionEnd.status` string
/// (`complete` / `hard_fail` / `budget_exceeded` / `cancelled`); a log with no
/// `SessionEnd` (crashed / in-flight) is labeled `"unknown"`.
struct RunReplay {
    model: String,
    outcome: String,
    tool_calls: Vec<ToolCallSnapshot>,
}

fn replay(records: &[SessionRecord]) -> RunReplay {
    let mut model = String::from("(unknown)");
    let mut outcome = String::from("unknown");
    let mut tool_calls = Vec::new();
    for rec in records {
        match &rec.event {
            SessionEvent::SessionStart { model: m, .. } => model = m.clone(),
            SessionEvent::SessionEnd { status, .. } => outcome = status.clone(),
            SessionEvent::Parsed { tool_call } => tool_calls.push(ToolCallSnapshot {
                tool: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
                succeeded: true, // the 06a stall signals key on tool+args, not success
            }),
            _ => {}
        }
    }
    RunReplay { model, outcome, tool_calls }
}
```

(The `succeeded: true` shortcut is sound because both 06a signals ignore it —
keep the comment. Do **not** pair `ToolResult` for `succeeded` in this phase.)

### 2. The signal-extractor seam (extensible for 06b)

An enum of calibratable signals; each pulls its sample(s) from a run's
`tool_calls`. **06b adds variants here** — that future extension is what justifies
the seam (STANDARDS § 2.2).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    /// Distinct normalized targets per full novelty window — many samples/run.
    NoveltyDistinct,
    /// Longest consecutive read-only run in the run — one sample/run.
    MaxReadOnlyRun,
}

impl Signal {
    fn label(self) -> &'static str {
        match self {
            Signal::NoveltyDistinct => "novelty_distinct_targets",
            Signal::MaxReadOnlyRun => "max_read_only_run",
        }
    }

    /// Extract this signal's raw samples from one run's tool-call sequence.
    fn samples(self, calls: &[ToolCallSnapshot], novelty_window: usize) -> Vec<usize> {
        match self {
            Signal::NoveltyDistinct => {
                // Replay turn-by-turn: measure_novelty over the growing history,
                // collecting distinct_targets at every full-window measurement.
                let mut deque: VecDeque<ToolCallSnapshot> = VecDeque::new();
                let mut out = Vec::new();
                for c in calls {
                    deque.push_back(c.clone());
                    if let Some(m) = measure_novelty(&deque, novelty_window) {
                        out.push(m.distinct_targets);
                    }
                }
                out
            }
            Signal::MaxReadOnlyRun => {
                let mut max = 0usize;
                let mut run = 0usize;
                for c in calls {
                    if tools::mutates_files(&c.tool) {
                        run = 0;
                    } else {
                        run += 1;
                        max = max.max(run);
                    }
                }
                vec![max]
            }
        }
    }
}

const SIGNALS: &[Signal] = &[Signal::NoveltyDistinct, Signal::MaxReadOnlyRun];
```

Pin the reuse: `NoveltyDistinct` **must** go through `measure_novelty` (not a
re-implementation) so the report can never drift from the live detector.

### 3. Aggregate per (signal, model, outcome) with a global fallback

For each signal, bucket samples by `(model, outcome)`; also keep an all-models
`(*, outcome)` global bucket. Compute count + p50/p90/p99 (nearest-rank on the
sorted samples). A per-model cell with `count < min_runs` is **omitted** from the
per-model rows (it still contributes to the global bucket).

```rust
/// Nearest-rank percentile of a sorted slice. `p` in 0.0..=1.0. Empty → 0.
fn percentile(sorted: &[usize], p: f64) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
```

Pin the boundary: `percentile(&[], _) == 0`; a single-sample slice returns that
sample for every `p`.

### 4. Report — text table + `--json`

Group the output **by signal**, then rows of `(model, outcome, N, p50, p90, p99)`:
global rows first (`model = "(all)"`), then per-model rows with `N >= min_runs`,
sorted for determinism. Mirror the column/`{:>}`-alignment style of
`runs::format_runs` / `scorecard`. `--json` emits the same aggregation as JSON.
**Always show N** (small models are high-variance).

### 5. `run()` entry point

```rust
pub struct CalibrateGovernorArgs<'a> {
    pub sessions_dir: &'a Path,
    pub model_filter: Option<&'a str>,
    pub novelty_window: usize,
    pub min_runs: usize,
    pub json: bool,
}

/// Enumerate `sessions_dir/*.jsonl`, replay each, aggregate, and return the
/// rendered report (text or JSON). A missing/empty dir yields an empty report,
/// not an error (the corpus may simply be empty).
pub fn run(args: &CalibrateGovernorArgs<'_>) -> String { /* … */ }
```

The dispatch arm resolves `sessions_dir` = `<repo>/.rexymcp/sessions` from the
`--repo` arg (default `.`), or a `--sessions-dir` override, then `println!`s the
returned report.

### 6. CLI wiring + build order (leaf-first)

1. Write `mcp/src/calibrate_governor.rs` (Tasks 1–5) and add `mod
   calibrate_governor;` to the mcp crate root. — **build green.**
2. Add the `Commands::CalibrateGovernor { repo, sessions_dir, model,
   novelty_window (default 24), min_runs (default 0), json }` clap variant. —
   **build green.**
3. Add the dispatch arm calling `calibrate_governor::run(...)`. — **build green.**
4. Tests (below). — **all four gates green.**

## Acceptance criteria

- [ ] `rexymcp calibrate-governor --repo <path>` prints a per-signal, per-model
      (+ global) distribution table with N + p50/p90/p99 for
      `novelty_distinct_targets` and `max_read_only_run`, labeled by run outcome.
- [ ] `NoveltyDistinct` samples are produced via `measure_novelty` (not a
      re-implementation).
- [ ] A per-model cell with `count < --min-runs` is dropped from per-model rows
      but still feeds the `(all)` global row.
- [ ] A missing/empty sessions dir yields an empty report (exit 0), not an error.
- [ ] `--json` emits the same aggregation as JSON.
- [ ] All four gates green.

## Test plan

Hermetic — build tiny session logs in a `TempDir` (write JSONL lines directly, or
serialize `SessionRecord`/`SessionEvent`) and assert on the aggregation, not exact
table whitespace (pin behavior, not rendering).

- `replay_extracts_model_outcome_and_tool_calls` — a fixture log → `RunReplay`
  with the right model, `SessionEnd.status`, and tool-call sequence.
- `run_with_no_sessionend_is_labeled_unknown` — a log missing `SessionEnd` →
  outcome `"unknown"` (boundary/negative case).
- `novelty_distinct_samples_match_measure_novelty` — a churn fixture → the
  `NoveltyDistinct` samples equal what `measure_novelty` yields over the same
  sequence (mutation-resistant: a diverging re-implementation fails this).
- `max_read_only_run_resets_on_mutating_call` — a sequence with an edit in the
  middle → the max is the longer read-only stretch, **not** the total (pins the
  reset — the must-NOT case).
- `min_runs_drops_thin_per_model_cells_into_global` — two models, one below
  `min_runs`; the thin model is absent from per-model rows but its samples are in
  the `(all)` global percentiles.
- `empty_sessions_dir_yields_empty_report` — no dir / no files → empty, no panic.
- `percentile_boundaries` — `percentile(&[], p) == 0`; a single-sample slice
  returns it for p50/p90/p99.

## End-to-end verification

Run the real subcommand against this repo's actual corpus and quote a slice:

```
rexymcp calibrate-governor --repo . --min-runs 3
```

Quote the `novelty_distinct_targets` rows split by outcome in the completion
Update Log — the first real data on how low *successful* runs' novelty goes vs.
the `novelty_distinct_floor = 6` default. (If the corpus is too small to be
interesting, say so and quote whatever it produces.)

## Authorizations

- [ ] May add `mod calibrate_governor;` to the mcp crate root and a new CLI
      subcommand. No new dependency (uses `std`, `serde_json`, and the existing
      executor surface). No `Cargo.toml` edit.

## Out of scope

- The remaining 5 detectors' signals (identical-repetition, oscillation,
  verifier-persistence, empty-completion, output-flood) — **phase-06b**. Leave the
  `Signal` enum easy to extend; do not add them here.
- Any suggested-threshold output or config mutation (report-only, fixed decision).
- A live "advisory fired" marker / dashboard surface (deferred).
- Reading `NoveltySample` events (re-derive from raw `Parsed` events instead).
- Pairing `ToolResult.succeeded` into the replay (the 06a signals ignore it).

## Notes

**Routing — dispatchable.** Unlike phases 01–05 this touches **no** governor
internals or the executor loop — it is a read-only analysis subcommand in the
`mcp` crate, so it can be dispatched to the local executor safely (no
governor-supervises-its-own-repair risk). The dispatch risk here is only the
ordinary multi-piece mechanical churn (module + CLI variant + dispatch arm +
tests) — the leaf-first build order in Task 6 is the countermeasure; each step
compiles.

## Update Log

(Filled in by the executor.)

### Update — 2026-07-19 18:23 (started)

**Executor:** Starting phase 06a — building `calibrate_governor` module with replay, signal-extractor seam, aggregation, report formatting, CLI wiring, and tests.

<!-- entries appended below this line -->
### Update — ts=1784487021469 (complete, server-authored)

**Summary:** All gates pass. Here's the summary:

**Summary:** Built the `rexymcp calibrate-governor` subcommand — a governor calibration framework that replays session logs, extracts stall-signal samples (novelty_distinct_targets and max_read_only_run), aggregates percentiles per (model, outcome) with a global fallback, and renders a report (text table or `--json`). The signal-extractor seam (`Signal` enum) delegates `NoveltyDistinct` through `measure_novelty` (not a re-implementation) and `MaxReadOnlyRun` through `tools::mutates_files`. Per-model cells below `--min-runs` are dropped from per-model rows but still feed the `(all)` global row.

**Notes for review:** The end-to-end run against this repo's corpus shows real data: `novelty_distinct_targets` for `budget_exceeded` runs has a P50 of 1 and P90 of 2 — far below the `novelty_distinct_floor = 6` default, confirming the floor is aggressive for budget-exceeded runs. Successful (`complete`) runs have P50=15, P90=20, which is above the default floor. The `max_read_only_run` P50 for complete runs is 12, which is below the `read_only_stall_threshold = 18` default. All 996 tests pass, clippy is clean, and the working tree is clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
ate_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
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

test result: ok. 996 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.08s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M34-governor-stall-hardening/README.md` — +1 -1
- `docs/dev/milestones/M34-governor-stall-hardening/phase-06a-calibration-framework-and-stall-signals.md` — +5 -1
- `mcp/src/calibrate_governor.rs` — +493 -0
- `mcp/src/main.rs` — +48 -0

**Commit:** 0218745ee71cf71729563ff8021c53d1a1144727

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

