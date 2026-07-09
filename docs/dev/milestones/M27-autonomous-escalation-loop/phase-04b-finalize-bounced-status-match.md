# Phase 04b: Finalize tolerates a bounced status line

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** review
**Depends on:** phase-03a (server-authored finalize), phase-04 (surfaced this defect)
**Estimated diff:** ~80 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

Fix the 03a server-authored finalize so it works on a **bounced** phase. Today
`finalize_complete` matches the phase-doc status line and README row **exactly**
against `in-progress`, but the review skill's bounce convention appends a note
(`**Status:** in-progress (bounced — see bugs/bug-04-1.md)`), so finalize
silently no-ops on any bounced-then-completed phase — the server writes no status
flip and no completion entry. This must land **before phase-06**: the autonomous
loop bounces and re-dispatches as normal operation, so without this fix the
marquee server-authored-finalize feature disengages exactly where it is needed.

## Architecture references

- M27 [README](README.md) § Exit criteria (server writes the flip + baseline
  entry on a `complete` run) and the phase-04 Review verdict (this defect's
  first occurrence).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Three match sites in `mcp/src/finalize.rs`, all exact and all defeated by a
bounce note:

- `status_is_in_progress` (`finalize.rs:52`): `line.trim() == "**Status:** in-progress"`.
- `flip_status_to_review` (`finalize.rs:60`): guards on the same `==` and then does
  `line.replace("**Status:** in-progress", "**Status:** review")` — note that even
  if the guard were relaxed, this substring replace on a bounced line would leave a
  **stale** `**Status:** review (bounced — …)`, which is wrong.
- `flip_readme_row` (`finalize.rs:164`): matches a row that
  `line.trim().ends_with("| in-progress |")`.

The phase-04 review reproduced the failure: `finalize_complete` returned
`Ok(false)` for a doc whose status line was
`**Status:** in-progress (bounced — see bugs/bug-04-1.md)`, so the architect had
to hand-author the flip + completion entry.

Existing `finalize.rs` tests pin the exact-match *positives* and the
`review`/`todo`/`done`/prose *negatives* — none asserts a bounced line is
*rejected*, so relaxing the match to a prefix does not break them. Re-verify by
running them.

## Spec

### 1. Prefix-tolerant in-progress predicate

In `mcp/src/finalize.rs`, add a private helper both status functions share (so
the match rule can't drift between them):

```rust
/// True iff `trimmed` is an in-progress status line, with or without a trailing
/// note (the review skill appends `(bounced — …)` on a bounce). The space before
/// the note is the delimiter, so `**Status:** in-progressish` does NOT match.
fn is_in_progress_status(trimmed: &str) -> bool {
    trimmed == "**Status:** in-progress" || trimmed.starts_with("**Status:** in-progress ")
}
```

Use it in `status_is_in_progress` (replace the `==` comparison). The space in the
`starts_with` arm is load-bearing — it is what keeps `in-progressish` /
`in-progress-foo` out.

### 2. `flip_status_to_review` drops the bounce note

Change the per-line guard to `is_in_progress_status(line.trim())`. On a match,
emit the line's **leading whitespace + `**Status:** review`** (the canonical
line), **not** a substring replace — so a bounced line becomes exactly
`**Status:** review` with the `(bounced — …)` note **removed** (it is stale once
the phase reaches review). A clean (unnoted) line still becomes `**Status:** review`,
byte-identical to today. Preserve the first-match-only behavior and the trailing
newline handling already in the function.

### 3. `flip_readme_row` tolerates a noted status cell

Change the row match so it fires when the row contains `phase_doc_filename` **and**
the row's **last table cell** (the text between the final two `|`, trimmed)
**starts with** `in-progress`. On a match, replace that last cell with ` review `
(dropping any note, mirroring Task 2), leaving the rest of the row and all other
rows byte-identical. A row whose last cell is `review` / `done` must **not** match.

## Acceptance criteria

- [ ] `cargo build` zero new warnings; `cargo clippy` and `cargo fmt --all --check`
      pass; `cargo test` passes (existing + new).
- [ ] `status_is_in_progress("**Status:** in-progress (bounced — see bugs/bug-04-1.md)")`
      is `true`; `"**Status:** review"`, `"**Status:** done"`, `"**Status:** todo"`,
      and `"**Status:** in-progressish"` are all `false`.
- [ ] `flip_status_to_review` turns a bounced status line into exactly
      `**Status:** review` (no residual `(bounced …)`), and a clean line's result is
      byte-identical to today.
- [ ] `flip_readme_row` flips a row whose last cell is `in-progress (bounced, bug-04-1)`
      to `review`, and returns `None` for a `review`/`done` row.
- [ ] `finalize_complete` on a `Complete` result whose doc status line carries a
      bounce note flips the doc to `**Status:** review` and appends the
      `(complete, server-authored)` entry (the end-to-end proof).

## Test plan

Add to the `finalize.rs` test module (mirror the existing test shapes):

- `status_is_in_progress_matches_bounced_line` — the noted line is `true`.
- `status_is_in_progress_rejects_in_progressish` — the space-delimiter negative.
- `flip_status_to_review_drops_bounce_note` — bounced line → exactly `**Status:** review`.
- `flip_readme_row_flips_bounced_row` — noted row cell → `review`; a sibling
  `review` row is untouched.
- `finalize_flips_bounced_status_and_appends_entry` — the integration proof:
  a `Complete` result + a `TempDir` phase doc whose status line is
  `**Status:** in-progress (bounced — see bugs/bug-04-1.md)` finalizes to
  `review` with the server-authored entry present (mirror
  `finalize_flips_status_and_appends_entry`).

## End-to-end verification

Quote the `finalize_flips_bounced_status_and_appends_entry` run showing the
`TempDir` doc's status line before (`in-progress (bounced …)`) and after
(`review`) plus the appended `(complete, server-authored)` entry. That exercises
the real `finalize_complete` path end-to-end (the same fake `RecordingRunner`
the sibling finalize integration tests use).

## Authorizations

None. Pure `mcp/src/finalize.rs` change (production + tests). No new dependency,
no `Cargo.toml` / `architecture.md` / `STANDARDS.md` / `WORKFLOW.md` / contract /
skill edit.

## Out of scope

- **No change to the review skill's bounce convention.** This phase fixes the
  server side (option A from the phase-04 verdict); the human-readable
  `(bounced — …)` note on the status line stays.
- **No change to the completion-entry format, the git commit logic, or the
  `Complete`-status gate** in `finalize_complete` — only the three match/flip
  helpers.
- Do not touch any phase doc other than this one (the phase-04 doc's status line
  is already resolved).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-09 03:23 (started)

**Executor:** Claude (Sonnet 4.5)
**Action:** Started phase 04b implementation.
### Update — ts=1783568064382 (complete, server-authored)

**Summary:** All three tasks are complete. Here's the summary:

**Summary + Notes for review:**

Fixed `mcp/src/finalize.rs` so the server-authored finalize path tolerates a bounced status line (`**Status:** in-progress (bounced — …)`). Three changes:

1. **Extracted `is_in_progress_status()`** — a shared prefix-tolerant predicate that matches `**Status:** in-progress` exactly or with a trailing space-delimited note. The load-bearing space prevents false positives like `in-progressish`.

2. **`flip_status_to_review`** now emits a clean `**Status:** review` line (with preserved leading whitespace), dropping any stale bounce note. Previously used `str::replace` which would have left `**Status:** review (bounced — …)`.

3. **`flip_readme_row`** now isolates the last table cell and matches it by `starts_with("in-progress")`, replacing the full cell content with ` review `. A `review`/`done` cell does not match.

All 917 tests pass (21 finalize-specific), `cargo fmt`, `cargo clippy`, and `cargo build` are clean. The `finalize_flips_bounced_status_and_appends_entry` integration test proves the end-to-end path: a `TempDir` phase doc with `**Status:** in-progress (bounced — see bugs/bug-04-1.md)` is flipped to `**Status:** review` with the `(complete, server-authored)` entry appended.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.37s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
.. ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 917 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` — +1 -1
- `docs/dev/milestones/M27-autonomous-escalation-loop/phase-04b-finalize-bounced-status-match.md` — +6 -1
- `mcp/src/finalize.rs` — +134 -14

**Commit:** a12119e2f4b6491e5be02803755206d856af2f28

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

