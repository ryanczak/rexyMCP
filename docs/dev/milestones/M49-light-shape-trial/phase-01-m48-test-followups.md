# Phase 01: M48 test follow-ups

**Milestone:** M49 — Light-shape phase trial
**Status:** review
**Depends on:** none
**Estimated diff:** ~15 lines, test code only
**Tags:** language=rust, kind=test, size=s

## Goal

Two tests shipped by M48 are weaker than they should be. Fix both.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc.
3. Confirm a clean tree.

## Spec

1. **Make the boundary regression test exercise the retired routing** — in
   `executor/src/tools/bash.rs`, `piped_cargo_style_output_is_not_filtered`
   runs a `sh -c` pipeline. The cargo filter M48 deleted only ever acted on
   commands whose text began with `cargo`, so this test passes against the
   pre-M48 code too and guards nothing. Change the command so it begins with
   `cargo` and still produces the same three lines through the same `grep`
   (a leading `cargo --version >/dev/null && ` is one way; the test must
   stay hermetic, so nothing may depend on a Cargo project existing in the
   temp dir). Assertions unchanged.
2. **Delete dead setup** — in `executor/src/agent/tests.rs`,
   `repeated_identical_bash_trips_hard_fail` builds a tool registry that is
   never passed to the run. Remove it and any import that becomes unused.
   The test's assertions are unchanged and must still pass.
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`. The
   server-authored `(complete)` entry does not satisfy this.

## Acceptance criteria

- [ ] `piped_cargo_style_output_is_not_filtered` passes, and its command
      string begins with `cargo`.
- [ ] `repeated_identical_bash_trips_hard_fail` passes, and the function
      body constructs no `ToolRegistry`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m49-phase-01.txt; : > "$A"
cargo test -p rexymcp-executor piped_cargo_style_output_is_not_filtered 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor repeated_identical_bash_trips_hard_fail 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
grep -A6 'async fn piped_cargo_style_output_is_not_filtered' executor/src/tools/bash.rs | grep -o '"command": "[^ ]*' >> "$A"
```

Expected: both tests `ok` with `exit=0`, and the last line shows the
command string starting with `cargo`.

## Authorizations

None.

## Out of scope

- Production code. This phase touches test modules only.
- Any other test in either file.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-08 17:05 (progress)

Started phase-01 (executor: GLM-5.3-Flash-NVFP4-Spark under rexyMCP). Flipped
phase doc Status and the milestone README's phase-01 row to `in-progress`.
Verified the pre-M48 routing claim against commit `2860dd2^` before editing:
the old `filter_for_command` dispatched to `cargo_filter` when
`is_cargo_command` matched, i.e. the deleted routing acted only on command
text beginning with `cargo` (after trim), and the old `bash.rs` metadata
labeled that path `"cargo"` rather than `"generic"` — so the fix is to make
the test's command string begin with `cargo`. Implemented Spec §1 (the test's
`command` now leads with `cargo --version >/dev/null && `) and Spec §2 (the
dead registry construction and its `scope` binding deleted; `bash_with_filter`
and `registry_over` remain in use elsewhere in the file, so the import block
needed no change — the dead setup was a registry build that never reached
`run_with_verifier`, which constructs its own). Both edited tests pass;
regression-verified the §1 command does route the retired branch by
compiling the pre-M48 `bash.rs` against it and confirming
`cargo_filter` runs and drops the third line (see notes for review).

### Update — 2026-10-08 17:14 (end-to-end verification)

All acceptance criteria met. Both tests `ok` with `exit=0`; the final line
shows the command string now starting with `cargo`. Positive controls: the
first cargo-test run of the session (before the §1 edit) emitted the old
command line `"command": "sh` as the artifact's last line, proving the grep
observes the file's real content and the artifact records what actually ran;
the `exit=` lines come from `PIPESTATUS[0]` of the cargo invocations and
would read `exit=1` on a failing run (verified separately in the shell: a
`false` first pipeline stage yields `exit=1`). The §1 edit is exercised
against the retired routing, not just the current tree: restoring the
pre-M48 `bash.rs` from commit `2860dd2^` and running the same test showed
`cargo_filter` active and the `test result: ok` line dropped from the tool
output — i.e. the test fails against the pre-M48 code now (details in the
progress entry). `target/e2e/m49-phase-01.txt`, pasted byte-for-byte:

```
test tools::bash::tests::piped_cargo_style_output_is_not_filtered ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1054 filtered out; finished in 0.01s
exit=0
test agent::tests::repeated_identical_bash_trips_hard_fail ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1054 filtered out; finished in 0.01s
exit=0
"command": "cargo
```

Gates: `cargo fmt --all --check` clean; `cargo build` finished 0 warnings
(`Finished dev profile ... in 1.07s` on a forced rebuild); `cargo clippy
--all-targets --all-features -- -D warnings` finished clean on a forced
re-check; `cargo test` → 708 + 2 + 1053 passed, 0 failed, 2 ignored across
lib, integration, and doc-test targets.

### Update — 2026-10-08 17:15 (complete, server-authored)

**Summary:** All three spec tasks are complete and every gate is green.

**Summary:** Made the M48 boundary regression actually guard the retired cargo routing: `piped_cargo_style_output_is_not_filtered` now runs `cargo --version >/dev/null && sh -c '…'` — the command string begins with `cargo` (which is what `is_cargo_command` keyed on pre-M48), the pipeline still emits the same three grep-selected lines, and the test stays hermetic because the leading cargo invocation only needs the binary to exit 0 and writes nothing. Deleted the dead setup in `repeated_identical_bash_trips_hard_fail` (the `ToolRegistry`/scope/binding lines; no import became unused — `bash_with_filter` is still used by two other tests in that file). Ran the phase doc's E2E block verbatim and pasted the artifact byte-for-byte into the Update Log's end-to-end entry.

**Positive controls (evidence is live):** before the edit, the same block emitted `"command": "sh` as the artifact's last line, so the grep demonstrably reads the real file and would have caught the unchanged test; I separately confirmed `PIPESTATUS[0]` reports `exit=1` on a failing stage, so the `exit=0` lines can distinguish pass from fail. I additionally verified the criterion "would have failed against the retired filter" directly: restoring pre-M48 `bash.rs` from `2860dd2^` and running the new test showed `cargo_filter` running and dropping the `test result: ok` line (the test asserted that line and would have failed). The restored pre-M48 file didn't compile against the M48 tree (it references the deleted `filter_for_command`), so the regression run itself was done under a compiled pre-M48 `output_filter.rs` + pre-M48 `bash.rs` pair, with the working tree restored to the M48 state immediately after; the final committed tree differs from HEAD only by the two spec'd test edits plus the phase-doc/README status flips.

**Deviations:** none material. Notes for review: (1) the E2E artifact's third-from-last line set is from the final code state and ends `"command": "cargo` as the spec expects; (2) the `.rexymcp/output/cmd-output-*.log` recovery files created by test runs in the repo root during the session are pre-existing tool behavior, not phase output; (3) phase doc and README rows were left at `in-progress` and no `(complete)` entry was written, per the contract.

**Verification:** `cargo fmt --all --check` clean · `cargo build` zero warnings (forced rebuild) · `cargo clippy --all-targets --all-features -- -D warnings` clean (forced re-check) · `cargo test` 708 + 2 + 1053 passed, 0 failed, 2 ignored. E2E artifact `target/e2e/m49-phase-01.txt` shows both tests `ok` with `exit=0` and the command string beginning with `cargo`. Committed as `b16c981` (code + status flips) and `8d6e968` (E2E/progress Update Log entries); `git status` clean.

**Executor:** local-inference-lab/GLM-5.3-Flash-NVFP4-Spark

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
   Compiling rexymcp-executor v0.9.1 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.12s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.12s


TEST
state_returns_advisory_error ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::update_task::tests::flips_active_task_to_done ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test store::telemetry::tests::append_is_atomic_under_concurrent_appenders ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1053 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `docs/dev/milestones/M49-light-shape-trial/README.md` — +1 -1
- `docs/dev/milestones/M49-light-shape-trial/phase-01-m48-test-followups.md` — +52 -1
- `executor/src/agent/tests.rs` — +0 -3
- `executor/src/tools/bash.rs` — +1 -1

**Commit:** 8d6e9683e2da90e9258b32650373488e13d9b390

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).
