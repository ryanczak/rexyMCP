# Phase 01: Refile persistence rule

**Milestone:** M47 — Verifier Persistence Keyed on Re-editing
**Status:** review
**Depends on:** none
**Estimated diff:** ~250 lines (one new struct + one new pub fn + a rewritten check in `hard_fail.rs`; three-line change in the agent loop; five new unit tests, four adapted, one new agent-level test)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`VerifierFailurePersistent` fires only when the executor keeps re-editing a
file it already edited in the current streak while the author-error count
does not fall. A sweep that edits a new file each turn with a flat or rising
count no longer terminates the run. The signal names the re-edited file.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #47 — why the rule changes.
- `docs/architecture.md` § M4 step 7 — the hard-fail detector this rule is
  one of.
- `docs/dev/milestones/M47-verifier-persistence-refile/README.md` — the
  replay evidence and the exact semantics (§ Exit criteria, first bullet).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`executor/src/governor/hard_fail.rs`**

- Imports (`hard_fail.rs:1`): `use std::collections::VecDeque;` only.
- The signal variant (`hard_fail.rs:22-24`):

  ```rust
  VerifierFailurePersistent {
      consecutive_failures: u32,
  },
  ```

  and its `describe` arm (`hard_fail.rs:64-68`):

  ```rust
  Self::VerifierFailurePersistent {
      consecutive_failures,
  } => {
      format!("verifier flagged errors on {consecutive_failures} consecutive turns")
  }
  ```

- `evaluate` (`hard_fail.rs:114-133`) takes
  `recent_verifier_error_counts: &[usize]` as its second parameter and passes
  it to the check.
- The check (`hard_fail.rs:188-209`) — this is what goes:

  ```rust
  fn check_verifier_persistence(counts: &[usize], threshold: usize) -> Option<HardFailSignal> {
      if counts.len() < threshold {
          return None;
      }
      let last_n = &counts[counts.len() - threshold..];

      // Must all be > 0
      if last_n.contains(&0) {
          return None;
      }

      // Must be non-decreasing oldest -> newest
      for w in last_n.windows(2) {
          if w[0] > w[1] {
              return None;
          }
      }

      Some(HardFailSignal::VerifierFailurePersistent {
          consecutive_failures: threshold as u32,
      })
  }
  ```

- Tests that build a `[usize]` counts slice and pass it to `evaluate`:
  `detects_verifier_persistence` (`:523`),
  `no_verifier_persistence_when_errors_decrease` (`:601`),
  `no_verifier_persistence_when_a_count_is_zero` (`:608`),
  `check_order_repetition_precedes_verifier` (`:630`), and every other
  `evaluate(&recent, &[], …)` call in the module (`detects_runaway_output`,
  `no_runaway_at_exact_threshold`, …). `describe_verifier_persistence`
  (`:481`) constructs the variant with `consecutive_failures: 3` only.

**`executor/src/agent/mod.rs`**

- Declaration (`mod.rs:195`):
  `let mut recent_verifier_error_counts: Vec<usize> = Vec::new();`
- The verify step (`mod.rs:1251-1284`) runs inside
  `if succeeded && let Some(path) = &edit_path { … }` — `path: &PathBuf`,
  the resolved absolute path of the file the `write_file`/`patch` call
  edited (`edit_target`, `agent/tools.rs:36`). On `VerifierResult::Checked`
  it does (`mod.rs:1277`):

  ```rust
  recent_verifier_error_counts.push(author.len());
  ```

- The hard-fail evaluation (`mod.rs:1322-1327`):

  ```rust
  let hard_fail_signal = evaluate(
      &recent_tool_calls,
      &recent_verifier_error_counts,
      Some((&tool_call.name, content.len())),
      &deps.governor,
  )
  ```

**`executor/src/agent/tests.rs`**

- `persistent_verifier_failure_trips_hard_fail` (`tests.rs:1294`) drives six
  `write_call(&dir, "a.rs", …)` turns against a `MockFileVerifier` returning
  one author diagnostic each, and asserts `PhaseStatus::HardFail` with
  `Blocker::HardFail(HardFailSignal::VerifierFailurePersistent { .. })`.
  Same file, so it must keep passing unchanged. Helpers: `write_call`
  (`:1120`), `checked` (`:1079`), `diag` (`:1068`), `run_with_verifier`
  (`:1084`), `token` (`:173`).

Nothing in the `mcp` crate calls `evaluate` or constructs the signal
(phase-02 will consume the new streak function).

## Spec

### 1. `VerifySample` and the streak function (`hard_fail.rs`)

Extend the imports to `use std::collections::{HashSet, VecDeque};` and add
`use std::path::{Path, PathBuf};`. Then add, above `evaluate`, exactly this
(it has been lint-checked against `-D warnings` on this toolchain):

```rust
/// One post-edit verifier sample: the author-attributed diagnostic count and
/// the path of the edit that triggered the verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifySample {
    pub author_errors: usize,
    pub path: PathBuf,
}

/// Streak length after each sample. A streak is consecutive verifies with
/// `author_errors > 0`, a non-decreasing count, **and** a triggering path
/// already edited earlier in the same streak. A zero clears it; a decrease or
/// a first-touched path restarts it at 1 with the path set reduced to that path.
pub fn verifier_persistence_streaks(samples: &[VerifySample]) -> Vec<usize> {
    let mut out = Vec::with_capacity(samples.len());
    let mut streak = 0usize;
    let mut prev: Option<usize> = None;
    let mut paths: HashSet<&Path> = HashSet::new();
    for s in samples {
        if s.author_errors == 0 {
            streak = 0;
            paths.clear();
            prev = None;
        } else {
            let non_decreasing = prev.is_none_or(|p| s.author_errors >= p);
            if non_decreasing && paths.contains(s.path.as_path()) {
                streak += 1;
            } else {
                streak = 1;
                paths.clear();
            }
            paths.insert(s.path.as_path());
            prev = Some(s.author_errors);
        }
        out.push(streak);
    }
    out
}
```

`pub` on both: phase-02's replay uses them from the `mcp` crate.

### 2. Rewrite the check

Replace `check_verifier_persistence` in full:

```rust
fn check_verifier_persistence(
    samples: &[VerifySample],
    threshold: usize,
) -> Option<HardFailSignal> {
    let streak = *verifier_persistence_streaks(samples).last()?;
    if streak < threshold {
        return None;
    }
    let file = samples.last()?.path.display().to_string();
    Some(HardFailSignal::VerifierFailurePersistent {
        consecutive_failures: streak as u32,
        file,
    })
}
```

Change `evaluate`'s second parameter to `recent_verifier_samples:
&[VerifySample]` and pass it through. Keep the check's position in
`evaluate` (after identical-repetition, before runaway-output) — the
`check_order_repetition_precedes_verifier` test pins that order.

### 3. The signal names the file

Add `file: String` to the `VerifierFailurePersistent` variant and rewrite
its `describe` arm to:

```rust
Self::VerifierFailurePersistent {
    consecutive_failures,
    file,
} => {
    format!(
        "verifier flagged errors on {consecutive_failures} consecutive edits of {file}"
    )
}
```

The variant derives `Serialize`/`Deserialize`/`JsonSchema` with the rest of
the enum; the new field is additive and needs no attribute.

### 4. The agent loop records the edit path

In `executor/src/agent/mod.rs`:

- `mod.rs:195` becomes
  `let mut recent_verifier_samples: Vec<VerifySample> = Vec::new();`
  (import `VerifySample` from `crate::governor::hard_fail` alongside the
  existing `evaluate` import).
- `mod.rs:1277` becomes:

  ```rust
  recent_verifier_samples.push(VerifySample {
      author_errors: author.len(),
      path: path.clone(),
  });
  ```

  `path` is the `&PathBuf` bound by the enclosing
  `if succeeded && let Some(path) = &edit_path`.
- `mod.rs:1324` passes `&recent_verifier_samples`.

The identifier `recent_verifier_error_counts` no longer appears in `mod.rs`.

### 5. Adapt the existing `hard_fail.rs` tests

Add one helper at the top of `mod tests`:

```rust
fn sample(author_errors: usize, path: &str) -> VerifySample {
    VerifySample {
        author_errors,
        path: PathBuf::from(path),
    }
}
```

Then convert every `evaluate(&recent, <counts>, …)` call: an empty `&[]`
stays `&[]`; a `[2usize, 2, 2, 2, 2, 2]` becomes
`vec![sample(2, "a.rs"); 6]` (same file — the semantics these tests pin are
unchanged for a single file); `[5usize, 3, 1]` → `[sample(5, "a.rs"),
sample(3, "a.rs"), sample(1, "a.rs")]`; `[2usize, 0, 2]` likewise.
`describe_verifier_persistence` gains `file: "src/x.rs".to_string()` and
additionally asserts the description contains `"src/x.rs"`.

### 6. New `hard_fail.rs` tests

Each asserts `verifier_persistence_streaks` exactly and, where stated,
`evaluate` with `GovernorConfig::default()` (threshold 6) and an empty
`VecDeque`:

- `verifier_persistence_ignores_sweep_across_files` — samples
  `1@x.rs, 1@mod.rs, 1@args.rs, 2@args.rs, 2@events.rs, 2@stream.rs`
  (the issue's fatal window: six patches, five files, count 1→2). Streaks
  `[1, 1, 1, 2, 1, 1]`; `evaluate` returns `None`. **The shipped rule fires
  on this input** — that is the bug.
- `verifier_persistence_fires_on_single_file_stall` — six `1@a.rs`. Streaks
  `[1, 2, 3, 4, 5, 6]`; `evaluate` returns
  `VerifierFailurePersistent { consecutive_failures: 6, file }` with `file`
  ending in `a.rs`.
- `verifier_persistence_new_file_resets_path_set` —
  `1@a, 1@a, 1@a, 1@b, 1@a, 1@a`. Streaks `[1, 2, 3, 1, 1, 2]`: after `b`
  the set is `{b}`, so returning to `a` restarts at 1. `evaluate` returns
  `None`.
- `verifier_persistence_zero_clears_streak` —
  `2@a, 2@a, 2@a, 0@a, 2@a, 2@a, 2@a`. Streaks `[1, 2, 3, 0, 1, 2, 3]`.
- `verifier_persistence_decrease_restarts_streak` —
  `5@a, 3@a, 3@a, 3@a, 3@a, 3@a`. Streaks `[1, 1, 2, 3, 4, 5]` and
  `evaluate` returns `None`; appending one more `3@a` makes `evaluate`
  return `Some`.

### 7. New agent-level test (`agent/tests.rs`)

`sweep_across_files_does_not_trip_verifier_persistence`, modelled on
`persistent_verifier_failure_trips_hard_fail` (`tests.rs:1294`): six
`write_call`s to **six distinct files** `a.rs` … `f.rs`, the same
six-diagnostic `MockFileVerifier`, `max_turns` 10, then `token("done")`.
Assert that **if** `result.briefing` is `Some`, its `current_blocker` is not
`Blocker::HardFail(HardFailSignal::VerifierFailurePersistent { .. })`. (Do
not assert the overall status: other detectors and the final gate run are
not this test's subject.)

### 8. Capture the end-to-end evidence

Run the two blocks in § End-to-end verification **verbatim and unmodified**,
paste the artifact file into a new Update Log entry headed
`### Update — <date> (end-to-end verification)`, and append the
`PASTE MATCH` / `PASTE MISMATCH` line the self-check prints. On
`PASTE MISMATCH`, fix the pasted fence from the file and re-run the
self-check until it prints `PASTE MATCH`. The server-authored `(complete)`
entry does not satisfy this task.

## Acceptance criteria

- [ ] Test `verifier_persistence_ignores_sweep_across_files` passes.
- [ ] Test `verifier_persistence_fires_on_single_file_stall` passes.
- [ ] Test `verifier_persistence_new_file_resets_path_set` passes.
- [ ] Test `verifier_persistence_zero_clears_streak` passes.
- [ ] Test `verifier_persistence_decrease_restarts_streak` passes.
- [ ] Test `sweep_across_files_does_not_trip_verifier_persistence` passes.
- [ ] **Preservation criteria:** `persistent_verifier_failure_trips_hard_fail`
      and `check_order_repetition_precedes_verifier` still pass.
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/governor/hard_fail.rs | grep -c 'pub fn verifier_persistence_streaks('`
      prints `1` (now 0).
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/governor/hard_fail.rs | grep -c 'file: String'`
      prints `1` (now 0).
- [ ] `grep -c 'Vec<VerifySample>' executor/src/agent/mod.rs` prints `1`
      (now 0), and `grep -c 'recent_verifier_error_counts' executor/src/agent/mod.rs`
      prints `0` (now 3; `mod.rs` holds no test module — its tests live in
      `tests.rs`).
- [ ] The E2E Update Log entry ends with the line `PASTE MATCH`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

In `executor/src/governor/hard_fail.rs` `mod tests` (the five tests of Spec
§6 plus the adaptations of §5) and `executor/src/agent/tests.rs` (§7). Test
names are as listed in § Acceptance criteria; each unit test asserts the
exact streak vector as well as the `evaluate` outcome, so a streak
off-by-one cannot hide behind a threshold.

## End-to-end verification

The real artifact is the executor library's governor behaviour, exercised by
the named tests. Run exactly this block from the repo root; it is also
Spec's final task:

```bash
mkdir -p target/e2e
A=target/e2e/m47-phase-01.txt; : > "$A"
cargo test -p rexymcp-executor verifier_persistence 2>&1 | tail -30 >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor sweep_across_files_does_not_trip 2>&1 | tail -15 >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
echo "streaks_fn=$(sed -n '1,/^#\[cfg(test)\]/p' executor/src/governor/hard_fail.rs | grep -c 'pub fn verifier_persistence_streaks(')" >> "$A"
echo "counts_ident=$(grep -c 'recent_verifier_error_counts' executor/src/agent/mod.rs)" >> "$A"
```

Expected: both test runs report `0 failed` with `exit=0`; `streaks_fn=1`;
`counts_ident=0`.

Paste `target/e2e/m47-phase-01.txt` byte-for-byte into a new Update Log entry
headed `### Update — <date> (end-to-end verification)`, then run the paste
self-check and append its verdict line to the same entry:

```bash
D=docs/dev/milestones/M47-verifier-persistence-refile/phase-01-refile-persistence-rule.md
L=$(grep -n '^### Update .*(end-to-end verification)' "$D" | tail -1 | cut -d: -f1)
tail -n +"$L" "$D" | awk '/^```/{c++; next} c==1{print} c==2{exit}' > target/e2e/pasted-01.txt
diff target/e2e/pasted-01.txt target/e2e/m47-phase-01.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

The server-authored `(complete)` entry does not satisfy this section.

## Authorizations

None. No dependencies, no `Cargo.toml`, no config-schema changes.

## Out of scope

- **`mcp/src/calibrate_governor.rs`** — the replay signal is phase-02. Do
  not touch.
- **`verifier_persistence_threshold`** stays 6; no new `[governor]` knob.
- **`docs/architecture.md`, `README.md`, plugin skills** — architect-owned.
- **`edit_target` not recognising `patch_lines`** (`agent/tools.rs:36`):
  an adjacent gap recorded in the milestone README. Do not widen
  `edit_target`.
- **The A/B/A/B alternation gap** described in the README Notes: a known
  limitation, not to be fixed here.
- Do not "improve" the path stored in `VerifySample` (relative-to-root
  display, canonicalisation); `path.clone()` of the resolved edit path is
  the spec.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-09-21 00:30 (progress)

Started phase-01. Flipped phase doc Status to `in-progress` and the milestone
README's phase-01 row to match. Implementing Spec tasks 1–8: `VerifySample` +
`verifier_persistence_streaks` in `hard_fail.rs`, the rewritten
`check_verifier_persistence`, `file` on the signal, the agent loop recording
the edit path per verify, the five new unit tests, the four adapted ones, and
the agent-level sweep test, then the E2E capture.

### Update — 2026-09-21 00:36 (end-to-end verification)

All spec §8 criteria met. `cargo test -p rexymcp-executor verifier_persistence`
ran the ten persistence tests (nine unit, one agent-level) plus the six
single-file-stall names the criteria call out — `0 failed`, `exit=0` on both
captures. `streaks_fn=1` proves the streak function landed above
`#[cfg(test)]` in `hard_fail.rs`; `counts_ident=0` proves the agent loop no
longer names `recent_verifier_error_counts`. Positive control is inherent in
the capture: the sweep test asserts the rule does **not** fire on the issue's
sweep window while the adapted `detects_verifier_persistence` asserts it still
fires on the single-file stall, so a no-op re-keying could not have produced
this output. `grep -c 'file: String'` over the non-test head of
`hard_fail.rs` = 1; `grep -c 'Vec<VerifySample>' agent/mod.rs` = 1.
`target/e2e/m47-phase-01.txt`, pasted byte-for-byte:

```
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.04s
     Running unittests src/lib.rs (target/debug/deps/executor-300152e5d9c0cc60)

running 10 tests
test governor::hard_fail::tests::describe_verifier_persistence ... ok
test governor::hard_fail::tests::no_verifier_persistence_when_errors_decrease ... ok
test governor::hard_fail::tests::detects_verifier_persistence ... ok
test governor::hard_fail::tests::verifier_persistence_fires_on_single_file_stall ... ok
test governor::hard_fail::tests::verifier_persistence_decrease_restarts_streak ... ok
test governor::hard_fail::tests::no_verifier_persistence_when_a_count_is_zero ... ok
test governor::hard_fail::tests::verifier_persistence_new_file_resets_path_set ... ok
test governor::hard_fail::tests::verifier_persistence_zero_clears_streak ... ok
test governor::hard_fail::tests::verifier_persistence_ignores_sweep_across_files ... ok
test agent::tests::sweep_across_files_does_not_trip_verifier_persistence ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 1050 filtered out; finished in 0.02s

exit=0
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.04s
     Running unittests src/lib.rs (target/debug/deps/executor-300152e5d9c0cc60)

running 1 test
test agent::tests::sweep_across_files_does_not_trip_verifier_persistence ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1059 filtered out; finished in 0.02s

exit=0
streaks_fn=1
counts_ident=0
```

PASTE MATCH

### Update — ts=1789951106933 (complete, server-authored)

**Summary:** **Summary + Notes for review**

Implemented M47 phase-01: the verifier-persistence hard-fail rule is re-keyed on re-editing. `hard_fail.rs` gains the `pub struct VerifySample { author_errors, path }` and `pub fn verifier_persistence_streaks(&[VerifySample]) -> Vec<usize>` exactly as specified (streak extends only on a non-decreasing count *and* a path already in the streak's path set; a zero clears, a decrease or first-touched path restarts at 1 with the set reduced to that path); `check_verifier_persistence` is rewritten on top of it and `evaluate` now takes `&[VerifySample]`; `VerifierFailurePersistent` gains `file: String` and its describe arm names the re-edited file; the agent loop (`mod.rs`) records `VerifySample { author_errors: author.len(), path: path.clone() }` per verify and passes `&recent_verifier_samples` to `evaluate`, with `recent_verifier_error_counts` fully gone. Tests: five new unit tests (`verifier_persistence_ignores_sweep_across_files`, `..._fires_on_single_file_stall`, `..._new_file_resets_path_set`, `..._zero_clears_streak`, `..._decrease_restarts_streak`) each asserting the exact streak vector plus the `evaluate` outcome; four existing tests + `describe_verifier_persistence` adapted via the `sample` helper; new agent-level `sweep_across_files_does_not_trip_verifier_persistence`. E2E capture ran verbatim (positive control built in: the sweep test must not fire while `detects_verifier_persistence` still fires on the single-file stall) and is pasted byte-for-byte in the end-to-end Update Log entry, which ends `PASTE MATCH`; all four gates are green (fmt, build, clippy `-D warnings`, 1058 tests / 0 failed) and the work is committed as `8fc88e5`.

**Notes for review:** (1) The tree was not clean at start — it carried the architect's own uncommitted draft of phase-02 plus NEXT.md/README wording edits; since the contract requires a clean tree to commit, I included those pre-existing doc changes in the same commit rather than stashing or reverting architect-authored content — flag if you want them split out. (2) Phase status left at `in-progress` per the lifecycle; no `(complete)` entry written. (3) `verifier_persistence_decrease_restarts_streak` builds its "appending one more makes it fire" case by chaining an extra sample rather than `samples.push` (array → `Vec`); semantics identical to the spec's wording. (4) `edit_target`'s `patch_lines` gap and the A/B/A/B alternation limitation remain untouched as directed.

**Executor:** local-inference-lab/GLM-5.3-Flash-NVFP4-Spark

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
_task_to_active ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test store::telemetry::tests::append_is_atomic_under_concurrent_appenders ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1058 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.10s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-9705ec1e04e62e42)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-80b252dbdf01b3a7)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**

- `docs/dev/milestones/M47-verifier-persistence-refile/README.md` — +1 -1
- `docs/dev/milestones/M47-verifier-persistence-refile/phase-01-refile-persistence-rule.md` — +60 -1
- `executor/src/agent/mod.rs` — +7 -4
- `executor/src/agent/tests.rs` — +34 -0
- `executor/src/governor/hard_fail.rs` — +184 -30

**Commit:** 8fc88e53d40698b0bc09de20cfbaece31d97e530

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).
