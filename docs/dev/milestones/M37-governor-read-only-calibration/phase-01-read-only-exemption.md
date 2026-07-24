# Phase 01: Exempt read-only windows from the oscillation detectors

**Milestone:** M37 — Governor Read-Only Calibration
**Status:** review
**Depends on:** none
**Estimated diff:** ~180 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

`check_oscillation` and `check_identical_repetition` key on `(tool, arguments)`
and are blind to whether a call **mutates** anything. So a model re-running
`sed -n` / `cat` / `python3 -c` to diagnose a confusing failure is hard-killed on
the same threshold as a genuine write-thrash loop — **four times during the M35
arc**, every one recovering on a resume or refined re-dispatch carrying one
specific hint.

M34 already shipped `check_read_only_stall` for exactly this case. Make it the
**only** terminator for read-only loops: exempt a window containing no
file-mutating call from both oscillation detectors.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #37 — this milestone's design summary, and the
  two rejected alternatives (advisory mode; separate looser thresholds).
- `docs/architecture.md` § Status #34 — `NoProgressStall`, which becomes the sole
  read-only terminator, and the advisory-until-calibrated pivot this milestone
  deliberately does **not** repeat.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`executor/src/governor/hard_fail.rs:230-255`** — `check_oscillation` examines
the last `window` calls and fires when they collapse to `2..=distinct_max`
distinct `(tool, arguments)` pairs:

```rust
pub fn check_oscillation(
    recent: &VecDeque<ToolCallSnapshot>,
    window: usize,
    distinct_max: usize,
) -> Option<HardFailSignal> {
    if window == 0 || recent.len() < window {
        return None;
    }
    let mut distinct: Vec<(&str, &serde_json::Value)> = Vec::new();
    for call in recent.iter().rev().take(window) {
        let key = (call.tool.as_str(), &call.arguments);
        if !distinct.iter().any(|(t, a)| *t == key.0 && *a == key.1) {
            distinct.push(key);
        }
    }
    let n = distinct.len();
    if n >= 2 && n <= distinct_max {
        Some(HardFailSignal::Oscillation { distinct_calls: n, window })
    } else {
        None
    }
}
```

**`executor/src/governor/hard_fail.rs:138-158`** — `check_identical_repetition`
examines the last `threshold` calls and fires when they are all identical:

```rust
fn check_identical_repetition(
    recent: &VecDeque<ToolCallSnapshot>,
    threshold: usize,
) -> Option<HardFailSignal> {
    if recent.len() < threshold {
        return None;
    }
    let last_n: Vec<_> = recent.iter().rev().take(threshold).collect();
    let first = &last_n[0];
    let all_identical = last_n
        .iter()
        .all(|c| c.tool == first.tool && c.arguments == first.arguments);
    if !all_identical {
        return None;
    }
    Some(HardFailSignal::IdenticalToolCallRepetition {
        tool: first.tool.clone(),
        consecutive_count: threshold as u32,
    })
}
```

**The helper already exists and is already used here.**
`crate::tools::mutates_files` (`executor/src/tools/router.rs:29-31`) is
`categorize(tool_name) == Some(Category::Write)`. `check_read_only_stall`
(`hard_fail.rs:257-286`) already calls it:

```rust
    for call in recent.iter().rev() {
        if crate::tools::mutates_files(&call.tool) {
            break;
        }
        run += 1;
    }
```

**Defaults** (`executor/src/config.rs:269-278`): `identical_call_threshold: 6`,
`oscillation_window: 8`, `oscillation_distinct_max: 2`,
`read_only_stall_threshold: 60`. So a read-only loop is currently killed at 6 or
8 calls where `NoProgressStall` would allow 60 — an 8× difference, which is why
the tight detector always wins.

**Call sites** — `check_identical_repetition` runs inside `evaluate`
(`hard_fail.rs:115-125`); `check_oscillation` is called separately from
`executor/src/agent/mod.rs:1327`. **Neither call site changes in this phase** —
the exemption lives inside the two functions.

## Spec

### 1. Add a private window-scan helper

In `executor/src/governor/hard_fail.rs`, next to the two detectors:

```rust
/// True when the last `window` calls contain at least one file-mutating call.
///
/// The oscillation detectors fire on *thrash* — a model churning edits without
/// converging. A window with no mutating call is not thrash: it is diagnosis
/// (repeated `sed -n`/`cat`/`grep` while reading toward a fix), which
/// `check_read_only_stall` already terminates at its own, far looser threshold.
/// Firing the tight detectors on it kills runs mid-diagnosis.
///
/// `window` is clamped to the deque length, so a short history scans what exists.
fn window_has_mutation(recent: &VecDeque<ToolCallSnapshot>, window: usize) -> bool {
    recent
        .iter()
        .rev()
        .take(window)
        .any(|c| crate::tools::mutates_files(&c.tool))
}
```

### 2. Exempt read-only windows in `check_oscillation`

Add the guard **after** the existing `window == 0 || recent.len() < window`
early return, so a disabled or under-filled detector still short-circuits first:

```rust
    if window == 0 || recent.len() < window {
        return None;
    }
    // Read-only windows are diagnosis, not thrash — left to check_read_only_stall.
    if !window_has_mutation(recent, window) {
        return None;
    }
```

### 3. Exempt read-only windows in `check_identical_repetition`

Same shape, using `threshold` as the window — **this function's window is
`threshold`, not `oscillation_window`**; scan exactly the slice it inspects:

```rust
    if recent.len() < threshold {
        return None;
    }
    // Read-only repetition is diagnosis, not thrash — left to check_read_only_stall.
    if !window_has_mutation(recent, threshold) {
        return None;
    }
```

### 4. Update both doc comments

Each function's doc comment must state the exemption and name
`check_read_only_stall` as what handles the read-only case, so the next reader
does not "restore" the old behavior as a bug fix. Keep the existing prose;
append a sentence.

### 5. Tests

Write the tests named in § Test plan.

## Acceptance criteria

- [ ] `cargo build` is green.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] `cargo test` passes.
- [ ] A window of purely read-only calls fires **neither** `Oscillation` nor
      `IdenticalToolCallRepetition`.
- [ ] A window containing **at least one** mutating call fires exactly as it does
      today — verified by the pre-existing tests still passing **unchanged**.
- [ ] `check_read_only_stall` still terminates a purely read-only run at its
      configured threshold; its existing tests pass unchanged.

## Test plan

In `executor/src/governor/hard_fail.rs`'s `#[cfg(test)] mod tests`. The module
already has `ToolCallSnapshot` fixtures for both detectors — reuse those
builders rather than inventing a new shape.

**The exemption (positive):**

- `oscillation_exempts_read_only_window` — an A,B,A,B window of `read_file` /
  `bash`-with-a-read-command returns `None` where it fires today.
- `identical_repetition_exempts_read_only_window` — `threshold` identical
  `read_file` calls return `None`.

**The exemption must not become a blanket disable (negative — the important
half):**

- `oscillation_still_fires_when_window_has_a_write` — the same A,B,A,B window
  with **one** call swapped for `patch` still returns
  `Some(HardFailSignal::Oscillation { .. })`.
- `identical_repetition_still_fires_for_write_tool` — `threshold` identical
  `write_file` calls still fire.
- `oscillation_fires_when_mutation_is_oldest_in_window` — the mutating call sits
  at the **far edge** of the window. Pins that the scan covers the whole window,
  not just its head.
- `identical_repetition_window_is_threshold_not_deque_length` — a mutating call
  sits in the deque but **outside** the last `threshold` calls; the detector must
  still be exempt. This is the test that catches scanning the whole deque instead
  of the inspected slice — the most likely wrong implementation.

**The backstop still works:**

- `read_only_stall_still_terminates_after_exemption` — a purely read-only run of
  `read_only_stall_threshold` calls still returns `NoProgressStall`. Without
  this, "exempt read-only" could silently mean "read-only loops never terminate".

Assert on the returned `HardFailSignal` variant, not on a boolean.

## End-to-end verification

This phase ships no CLI surface, so the E2E is the calibration corpus — and this
is the **first change to a governor terminator since `calibrate-governor`
existed**, so capture it.

Run **before** making changes and again after:

```bash
cargo run -p rexymcp -- calibrate-governor --repo . 2>&1 | head -40
```

Paste both in the completion Update Log. The replay is over the recorded
session-log corpus, so the **distributions must not move** — this phase changes
which signals fire on *future* runs, not how past ones are scored. A changed
distribution means the replay path was touched, which is out of scope.

Then confirm the exemption on a real run rather than only in unit tests: the
next dispatch after this phase lands should not hard-fail on a read-only
inspection loop. That cannot be forced synthetically here — state in the Update
Log that it is deferred to the next dispatch's telemetry, and do **not** claim it
was observed.

## Authorizations

None. No new dependencies. No edits to `docs/architecture.md`, `STANDARDS.md`, or
`WORKFLOW.md`.

## Out of scope

- **Changing any threshold or default.** `identical_call_threshold`,
  `oscillation_window`, `oscillation_distinct_max` and
  `read_only_stall_threshold` keep their values. This phase changes *what the
  detectors look at*, not how much they tolerate.
- **Adding a config knob.** No `oscillation_action`; the advisory-mode
  alternative was considered and rejected (architecture.md §37) because it keeps
  the pre-emption risk and adds a knob nobody has data to tune.
- `check_low_novelty_stall`, `check_windowed_output`,
  `check_verifier_persistence`, `check_runaway_output` — untouched.
- The `FAILURE_CLASSES` additions (M37 phase-02), the token-formatter
  consolidation (03), byte-column compaction (04), and the completion-entry
  writer (05).
- Anything in `mcp/`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-24 03:00 (started)

Executor implementing phase 01: adding `window_has_mutation` helper, exempting
read-only windows in `check_oscillation` and `check_identical_repetition`,
updating doc comments, and writing 7 new tests.
### Update — ts=1784862880782 (complete, server-authored)

**Summary:** **Summary + Notes for review:**

Built the read-only window exemption for M37 phase 01. Added `window_has_mutation` helper in `hard_fail.rs` that scans the last N calls for any file-mutating tool, then gated both `check_oscillation` and `check_identical_repetition` behind it — read-only windows now return `None`, leaving `check_read_only_stall` as the sole terminator at its 60-call threshold. Updated both doc comments to name the exemption and cross-reference `check_read_only_stall`. Wrote 7 new unit tests covering the positive exemption, negative (still fires with writes), edge cases (mutation at far edge, mutation outside threshold window), and the backstop (read-only stall still terminates). Two pre-existing agent integration tests (`identical_tool_call_repetition_trips_hard_fail` and `oscillation_across_alternating_reads_trips_hard_fail`) were updated to use write tools instead of read-only tools so they continue exercising the detectors. All 1039 tests pass, `calibrate-governor` distributions are unchanged. The deferred real-run verification (next dispatch's telemetry) is noted per spec.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Checking rexymcp-executor v0.9.1 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.47s


TEST
e ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1039 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M37-governor-read-only-calibration/README.md` — +1 -1
- `docs/dev/milestones/M37-governor-read-only-calibration/phase-01-read-only-exemption.md` — +7 -1
- `executor/src/agent/tests.rs` — +9 -4
- `executor/src/governor/hard_fail.rs` — +205 -2

**Commit:** 4b9c62a8c57e35034cdc92ee20eacbb617b6e47d

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

