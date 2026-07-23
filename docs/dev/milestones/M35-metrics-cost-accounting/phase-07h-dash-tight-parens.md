# Phase 07h: Tight parens on the debit "—" — `(—)`, padding outside, still decimal-aligned

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
**Depends on:** phase-07g
**Estimated diff:** ~35 lines
**Tags:** language=rust, kind=fix, size=s

## Goal

07g aligned the Budget-savings no-value `—` with the decimal column, but for a **debit**
(parenthesized) row it renders `(—  )` — the two alignment spaces sit **inside** the
parens, so the `)` floats away from the `—`. The parens should **hug** the `—` (`(—)`,
no internal spaces) **and** the `—` should stay on the decimal column.

Both are achievable together: keep `(—)` tight and put the 2 alignment spaces **after**
the `)` (`"(—)  "`). Right-aligned, that shifts the whole tight `(—)` left until the `—`
lands on the decimal column — same `—` position as today, just with the parens closed up
and the padding moved outside.

## ⚠️ Read this first — editing discipline on `panels.rs`

`mcp/src/dashboard/panels.rs` is dashboard rendering code. If a build/syntax error appears
after an edit, run `cargo build -p rexymcp` **once** and read the compiler's error location,
then open **only that region** with `read_file` and fix it. **Never** hunt for a problem by
re-reading the whole file in a loop with `python3 -c`/`bash`/`sed`/`cat` — repeating the
same read-only inspection command trips the governor's oscillation terminator (it has
hard-failed dashboard runs this milestone). Read once, fix once, verify with `cargo build`.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs` — the `paren` closure in `savings_lines` (672) and the
  `savings_lines_dash_aligns_with_decimal` test (~2374) added in 07g. `align_value` (~504,
  used by `space_pad` for the non-debit dash) — **do not** change it.

## Current state

The debit-value wrapper and the non-debit padder (`panels.rs:672–673`):

```rust
    let paren = |v: String| format!("({})", align_value(&v));
    let space_pad = |v: String| format!(" {} ", align_value(&v));
```

For a no-value `"—"`, `align_value("—")` is `"—  "` (dash + 2 spaces), so:
- **non-debit** (Baseline/Net): `space_pad("—")` → `" —   "` → the `—` lands on the decimal
  column (offset 4). **This is correct — leave it.**
- **debit** (Architect): `paren("—")` → `format!("({})", "—  ")` → `"(—  )"` → the two spaces
  are **inside** the parens. The `—` is on the decimal column, but the `)` is pushed two
  columns right of the `—`. **This is what to fix.**

A debit **number** is `($5.00)` (right-aligned, `.` at offset 4, `)` at offset 1). The
debit dash should read `(—)` (tight) with the `—` at that same offset-4 decimal column.

## Spec

### Task 1 — tight parens on the debit dash (`paren` closure, savings_lines)

Change the `paren` closure (`panels.rs:672`) so a `"—"` renders as **tight parens with the
padding after the `)`** — everything else (numbers) is unchanged:

```rust
    let paren = |v: String| {
        if v == "—" {
            // Tight parens; the 2 trailing spaces (the two decimal places a number shows)
            // shift `(—)` left under the decimal column when right-aligned.
            "(—)  ".to_string()
        } else {
            format!("({v})")
        }
    };
```

- The number branch is `format!("({v})")` — note `align_value` is **not** needed for a
  number (it passes numbers through unchanged), so dropping it here is behavior-preserving.
- **Do not change** `space_pad` (line 673), `align_value`, the `debit_row` empty-check
  (which still compares the raw `"—"`), `make_row`, or the tokens-mode block.

**Result:** `paren("—")` → `"(—)  "`. Right-aligned in the value column, the `—` stays on
the decimal column (offset 4, unchanged from 07g) and the parens are tight `(—)`. Numeric
debit values (`($5.00)`) are byte-for-byte unchanged.

## Acceptance criteria

- [ ] A debit no-value renders `(—)` (tight — **no** spaces between the parens and the `—`),
      not `(—  )`.
- [ ] The `—` still sits on the **decimal column**: in the dollars-mode savings block, the
      byte index of the `.` in a numeric row (e.g. Baseline `$X.XX`) equals the byte index of
      the `—` in the Architect `(—)` row (same as 07g's assertion — unchanged).
- [ ] Non-debit dashes (Baseline/Net, no parens) and numeric values render exactly as after
      07g.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.

## Test plan

Extend the existing **`savings_lines_dash_aligns_with_decimal`** test (added in 07g;
reuses the `savings_lines_architect_cost_shown_from_project_costs` fixture that renders a
Baseline `$` row and an Architect `(—)` row). Keep its alignment assertion and **add** a
tight-parens assertion:

```rust
    // decimal alignment (unchanged from 07g)
    assert_eq!(baseline.find('.'), architect.find('—'), "…");
    // NEW: tight parens — no spaces between the parens and the —
    assert!(
        architect.contains("(—)"),
        "debit no-value must render tight parens (—): {architect}"
    );
```

`architect.contains("(—)")` is **mutation-sensitive**: the old `"(—  )"` does not contain
the substring `(—)`, so the assertion fails under the pre-07h rendering and passes after.
The `align_value_pads_dash_to_decimal_column` unit test is unchanged (align_value still
serves the non-debit dash).

## End-to-end verification

The dashboard is a live TUI (not hermetically capturable). The extended
`savings_lines_dash_aligns_with_decimal` test is the pinned evidence — quote its result in
the completion Update Log. Optionally run `cargo run -p rexymcp -- dashboard --repo .` to
eyeball the Architect row now reading `(—)` with the `—` under the decimals.

## Authorizations

- Editing `mcp/src/dashboard/panels.rs` (the `paren` closure + the one test) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **`align_value`, `space_pad`, the non-debit dash, `fmt_opt`, the empty-check, `make_row`,
  the header, tokens mode** — all unchanged; only the `paren` closure's dash branch and the
  one test change.
- **Tokens-mode dash alignment** — still out of scope (as noted in 07g).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-23 04:53 (started)

**Executor:** LLM executor — started phase 07h.
### Update — ts=1784782601240 (complete, server-authored)

**Summary:** Done. Here's what was built:

**What:** Changed the `paren` closure in `savings_lines` to render the debit no-value dash as `(—)  ` — tight parens with the 2 alignment spaces moved outside the closing paren, instead of the previous `(—  )` with spaces inside. This keeps the `—` on the same decimal column while making the parens hug the dash.

**Deviation:** None — implemented exactly as specified.

**E2E verification:** `cargo test` passes (614 unit + 1032 integration, 2 ignored). The `savings_lines_dash_aligns_with_decimal` test now includes the new tight-parens assertion (`architect.contains("(—)")`). The `grep` confirms the literal `(—)  ` is present at line 676. All build/lint/format gates pass clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
_matching_kind ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — +1 -1
- `docs/dev/milestones/M35-metrics-cost-accounting/phase-07h-dash-tight-parens.md` — +5 -1
- `mcp/src/dashboard/panels.rs` — +14 -1

**Commit:** 400c4041a0534119034ec36899d6ef2863fe2161

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

