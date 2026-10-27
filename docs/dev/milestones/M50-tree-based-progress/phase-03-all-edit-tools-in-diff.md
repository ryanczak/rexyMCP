# Phase 03: Every edit tool in the reported diff

**Milestone:** M50 — Tree-based progress
**Status:** done
**Depends on:** phase-02
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

The phase diff captures a file's "before" content only for `write_file` and
`patch`. Edits through `patch_lines`, `delete_file` and `move_file` never
reach it, so they are missing from `PhaseResult.diff` / `files_changed` and
from the progress numstat. Capture them too.

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` — the milestone's
  goal: every way a file changes shows up as progress and in the diff.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc.
3. Confirm a clean tree.

## Current state

In the agent loop (`executor/src/agent/mod.rs`), `edit_target`
(`agent/tools.rs`) names the file an edit call targets, and the loop uses
that one path for several things: the verifier baseline, the pre-edit
capture into `pre_edit_content`, superseded-read eviction, the post-write
hooks and the post-edit verify. `edit_target` covers `write_file` and
`patch` only. `build_diff` (`agent/outcome.rs`) and `numstat_from_pre_edit`
(`agent/progress.rs`) diff whatever is in `pre_edit_content` against disk,
treating an unreadable "after" as empty.

## Spec

1. **Diff-capture targets** — a new helper beside `edit_target` in
   `agent/tools.rs` that returns every path whose pre-edit content the diff
   needs for a call: the `path` of `write_file`, `patch`, `patch_lines` and
   `delete_file`; both `from` and `to` of `move_file`. Empty for every other
   tool and for a call missing the argument. Paths resolve against the
   project root the same way `edit_target` does, so the keys match.
2. **Capture before dispatch** — the loop captures pre-edit content for each
   of those paths (where not already captured) instead of only for
   `edit_target`'s path. Everything else `edit_target` drives — baseline,
   eviction, hooks, verify — is unchanged: do not widen `edit_target`. A
   deleted file then reports as all removals and a moved file as a removal
   at `from` plus an addition at `to`, with no change to `build_diff` or the
   numstat.
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] Preservation: every existing test passes unmodified.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- Helper, in `agent/tools.rs`:
  - `diff_targets_cover_every_edit_tool` — each of the five tools yields its
    path(s); `move_file` yields two.
  - `diff_targets_empty_for_non_edit_tools` — `read_file`, `bash`, `search`
    and a `write_file` call with no `path` yield nothing.
- Loop, in `agent/tests.rs`. The tests register the three tools
  (`patch_lines`, `delete_file`, `move_file` constructors in
  `executor/src/tools/`) alongside the existing registry; the `MockAiClientScript`
  script and `LoopDeps` shape follow `run_with_bash` in the same file. No git
  repo is needed: the "before" side is captured from disk.
  - `patch_lines_edit_appears_in_phase_diff` — a file edited with
    `patch_lines` appears in `files_changed` with the correct `+/-` summary.
  - `deleted_file_appears_as_removals_in_phase_diff` — a `delete_file`d file
    appears with zero additions and its line count as removals, and its
    lines appear as `-` lines in `diff`.
  - `moved_file_appears_as_removal_and_addition` — after `move_file`,
    `files_changed` has an all-removals entry for `from` and an all-additions
    entry for `to`.
  - `failed_delete_adds_nothing_to_diff` — `delete_file` on a path that
    does not exist leaves `files_changed` empty.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-03.txt; : > "$A"
for t in diff_targets_ patch_lines_edit_appears_in_phase_diff deleted_file_appears_as_removals_in_phase_diff moved_file_appears_as_removal_and_addition failed_delete_adds_nothing_to_diff; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count.

## Authorizations

- `executor/src/agent/mod.rs`, `executor/src/agent/tools.rs`,
  `executor/src/agent/tests.rs`.

No new dependencies.

## Out of scope

- `edit_target` and everything it drives. Whether `patch_lines` should also
  get the read-before-edit gate and the post-edit verify is a separate
  decision; note it in "Notes for review" if you have a view.
- `build_diff`, `numstat_from_pre_edit`, the tools themselves.
- Directory moves or deletes (`delete_file` refuses directories).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-27 00:10 (progress)

Started phase-03 (executor). Flipped status to `in-progress` and the milestone
README row to match. Reading the touched modules (`agent/tools.rs`,
`agent/mod.rs`, `agent/outcome.rs`, `agent/progress.rs`, `agent/tests.rs`) and
the three tool constructors (`tools/patch_lines.rs`, `tools/delete_file.rs`,
`tools/move_file.rs`) before writing the diff-targets helper.

### Update — 2026-10-27 00:23 (end-to-end verification)

Ran the phase doc's E2E block verbatim after the final code state (artifact
`target/e2e/m50-phase-03.txt`, pasted below). Positive control: each filter
shows its named test line plus a non-zero `passed` count, and each loop test
exercises the real tool against a real file in a `TempDir` — a `patch_lines`
rewrite, a `delete_file` of a 3-line file, a `move_file` across directories,
and a `delete_file` on a missing path that must add nothing — so the
assertions would fail if the capture had not reached `pre_edit_content` or
the diff/numstat had not consumed it.

```
test agent::tools::tests::diff_targets_resolve_absolute_paths_through_unchanged ... ok
test agent::tools::tests::diff_targets_empty_for_non_edit_tools ... ok
test agent::tools::tests::diff_targets_cover_every_edit_tool ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1084 filtered out; finished in 0.00s
diff_targets_ exit=0
test agent::tests::patch_lines_edit_appears_in_phase_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
patch_lines_edit_appears_in_phase_diff exit=0
test agent::tests::deleted_file_appears_as_removals_in_phase_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
deleted_file_appears_as_removals_in_phase_diff exit=0
test agent::tests::moved_file_appears_as_removal_and_addition ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
moved_file_appears_as_removal_and_addition exit=0
test agent::tests::failed_delete_adds_nothing_to_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1086 filtered out; finished in 0.01s
failed_delete_adds_nothing_to_diff exit=0
```

### Update — 2026-10-27 00:24 (complete, server-authored)

**Summary:** </think>Everything committed, working tree clean. Now the final message — Summary + Notes for review.

Wait — one detail to check: the phase doc says the loop captures "instead of only for `edit_target`'s path". I replaced the edit_target-based capture with the diff_targets loop, which covers a superset (write_file and patch's path are included in diff_targets). So `edit_path` remains used for baseline/eviction/hooks/verify only, unchanged. Good — that matches the spec ("do not widen edit_target").

One more check: the spec says "Empty for every other tool and for a call missing the argument." My diff_targets for move_file with `from` present but `to` missing yields `[from]` — the test plan says "a `write_file` call with no `path` yield nothing" but doesn't specify a move_file missing `to`. My implementation returns from when to is missing. Is that a deviation from "for a call missing the argument"? The spec's phrasing "Empty for every other tool and for a call missing the argument" could be read as: a call missing *the* argument yields empty. For move_file, "the argument" is ambiguous — it has two. I chose to yield `from` if present. The alternative would be requiring both. Hmm — should I be more literal? The spec sentence: "Empty for every other tool and for a call missing the argument. Paths resolve against the project root the same way `edit_target` does, so the keys match."

Strictly, "a call missing the argument" — for move_file there are two arguments. My interpretation: each named path is independent; a call missing one still captures the other. This is a defensible reading, and capturing `from` when `to` is missing is harmless (the move would fail anyway — actually no: if `to` is missing the tool errors, and the diff would show... `from` unchanged vs. before → skipped since before == after. So no spurious diff entry. Good — harmless).

But it IS a deviation from a strict reading. I should note it in "Notes for review". Actually, my test `diff_targets_empty_for_non_edit_tools` now asserts the behavior explicitly with a message explaining why. I'll flag it.

Also note the spec test plan named exactly 2 helper tests; I added a third (`diff_targets_resolve_absolute_paths_through_unchanged`). That's an extra test — fine, note it.

Test counts: 1085 executor lib tests pass (pre-phase 1078 → +7 new: 3 helper + 4 loop). Whole workspace: 1795 passed, 0 failed.

Preservation: "every existing test passes unmodified" — I didn't modify any existing test; the only edits to existing code were the import line and new registry helper. Confirmed via the diff (only additions to tests.rs plus the use-line reformat by cargo fmt).

Wait — cargo fmt reformatted the import line `use crate::tools::{bash_with_filter, patch, read_file, write_file};` into multi-line — that's a formatting change to an existing line, not a test modification. Fine.

Also the `session-phase` note: the phase doc's Update Log has my started entry and the E2E entry. Status still `in-progress`. Good.

Now final message: Summary + Notes for review, 2–6 sentences but substantive.</think>
All tasks complete, all gates green, tree clean. Final report:

**Summary:** Implemented M50 phase-03. Added `diff_targets` beside `edit_target` in `executor/src/agent/tools.rs` — it returns the `path` of `write_file`/`patch`/`patch_lines`/`delete_file` and both `from` and `to` of `move_file` (resolved the same non-canonicalizing way `edit_target` resolves, so keys match), and is empty for every other tool or a call missing its path argument. The loop's pre-edit capture (`agent/mod.rs` step 07e) now iterates `diff_targets` and inserts each not-yet-captured path's on-disk content into `pre_edit_content` via `or_insert_with`; `edit_target` and everything it drives (verifier baseline, superseded-read eviction, post-write hooks, post-edit verify) is untouched, as are `build_diff` and `numstat_from_pre_edit`. Four new loop tests in `agent/tests.rs` (new `run_with_all_edit_tools` helper registering `patch_lines`/`delete_file`/`move_file`, `MockAiClientScript`/`LoopDeps` shaped after `run_with_bash`, no git repo needed) plus three helper unit tests, including the two the Test plan names for `diff_targets_`.

**Notes for review:** (1) E2E block ran verbatim after the final code state; artifact `target/e2e/m50-phase-03.txt` pasted byte-for-byte into the `### Update — 2026-10-27 00:23 (end-to-end verification)` entry — every named test `ok`, every `exit=0`, non-zero passed counts. (2) All four gates green as separate invocations: `cargo fmt --all --check` exit 0, `cargo build` zero warnings, `cargo clippy --all-targets --all-features -- -D warnings` exit 0 (one `map_entry` lint fixed by switching to `or_insert_with`), `cargo test` 1795 passed / 0 failed workspace-wide, executor lib 1085 (1078 pre-phase + 7 new). (3) Preservation held: no existing test modified. (4) One interpretation to confirm: the spec's "empty … for a call missing the argument" — for `move_file` with `to` absent I still capture `from` (the dispatch then errors, the file is unchanged, so the diff skips the entry; harmless), pinned by an explicit assertion in `diff_targets_empty_for_non_edit_tools`. (5) Added a third helper test beyond the plan, `diff_targets_resolve_absolute_paths_through_unchanged`, pinning that absolute paths pass through unchanged like `edit_target`'s. (6) Out-of-scope view for the record: `patch_lines` (and the new delete/move paths) still bypass the read-before-edit gate and post-edit verify that `write_file`/`patch` get — noted, not changed. (7) Status left at `in-progress` in the phase doc and README row; no `(complete)` entry written, per the contract. Committed as `4ed8f46`; `git status --short` is clean.

**Executor:** nvidia/GLM-5.3-Flash-FP4-Local

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
:invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::references_exclude_strings_and_comments ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::references_python_identifier ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
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

test result: ok. 1085 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.11s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `docs/dev/milestones/M50-tree-based-progress/README.md` — +1 -1
- `docs/dev/milestones/M50-tree-based-progress/phase-03-all-edit-tools-in-diff.md` — +40 -1
- `executor/src/agent/mod.rs` — +13 -8
- `executor/src/agent/tests.rs` — +216 -1
- `executor/src/agent/tools.rs` — +97 -1

**Commit:** 4ed8f4663a597bcb26cd8defa6f6bbd2c0c99c2c

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-10-27

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** nvidia/GLM-5.3-Flash-FP4-Local (116 turns)
- **Scope deviations:** none. One extra helper test
  (`diff_targets_resolve_absolute_paths_through_unchanged`). For a `move_file`
  call missing `to`, `from` is still captured. That's harmless: the dispatch
  fails, the file is unchanged, and the diff skips it. Accepted.
- **Calibration:** the executor's mutation evidence was prose again (third
  time this milestone), so the reviewer reproduced it: narrowing
  `diff_targets` to `write_file` | `patch` fails the `patch_lines`,
  `delete_file` and `move_file` loop tests, while the negative test still
  passes. The executor's view (Notes for review 6): `patch_lines`, `delete_file`
  and `move_file` still skip the read-before-edit gate and the post-edit
  verify. Held as a follow-up. New observation: this run's
  `completion_summary` began with leaked reasoning text (`</think>…`) ahead of
  the real Summary, which suggests the completion-summary extraction does not
  strip a reasoning block. Held for recurrence.
