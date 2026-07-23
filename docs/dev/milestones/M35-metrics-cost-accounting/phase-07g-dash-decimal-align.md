# Phase 07g: Align the Budget savings "—" (no value) with the decimal column

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-07f
**Estimated diff:** ~80 lines
**Tags:** language=rust, kind=fix, size=s

## Goal

In the dashboard Budget panel's savings block, a **no-value `—`** does not line up with the
**decimal points** of the `$X.XX` amounts above/below it. After 07e, dollar values are
right-aligned with a 1-char gutter so their decimal point sits a fixed distance from the
cell's right edge; a bare `—` sits at the *last-digit* column instead, one place right of
the decimals. Fix: pad a `—` so it lands on the **decimal column** — the `—` reads as
aligned with the `.`s in the same column.

The numbers are always `$X.XX` (exactly 2 decimals), so the decimal point sits `2 + 1`
(two decimals + the 1-char gutter) from the right; a bare `—` (at the gutter) needs the **2
decimal places** worth of trailing spaces to reach it.

## ⚠️ Read this first — editing discipline on `panels.rs`

`mcp/src/dashboard/panels.rs` is dashboard rendering code. If a build/syntax error appears
after an edit, run `cargo build -p rexymcp` **once** and read the compiler's error location,
then open **only that region** with `read_file` and fix it. **Never** hunt for a problem by
re-reading the whole file in a loop with `python3 -c`/`bash`/`sed`/`cat` — repeating the
same read-only inspection command trips the governor's oscillation terminator (it has
hard-failed dashboard runs this milestone). Read once, fix once, verify with `cargo build`.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs` — `fmt_tokens` (490, a nearby module-level helper to place
  the new one beside); `savings_lines` (506–705), specifically `fmt_opt` (527), the
  `paren` (659) and `space_pad` (660) closures, the `debit_row` empty-check (665), and the
  Baseline/Net/Architect value sites. `savings_lines_architect_cost_shown_from_project_costs`
  (~1913) — the **known-good fixture** the render test reuses.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above **and the editing-discipline note above**.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. `cargo build -p rexymcp` after each change.

## Current state

The dollars-mode value formatting in `savings_lines` (`panels.rs:659–703`). Non-debit
values (Baseline, Net) go through `space_pad`; debit values (Executor, Architect) through
`paren`. A no-value is `fmt_opt(None)` → `"—"`:

```rust
    let paren = |v: String| format!("({v})");
    let space_pad = |v: String| format!(" {v} ");
    let debit_row =
        |label: &str, sess: String, mile: String, proj: String| -> Option<Line<'static>> {
            let empty = |v: &str| v == "$0.00" || v == "—";   // checks the RAW value
            if empty(&sess) && empty(&mile) && empty(&proj) { return None; }
            Some(make_row(label, paren(sess), paren(mile), paren(proj)))
        };
    // Baseline / Net: make_row(label, space_pad(fmt_opt(..)), …)
    // Architect (debit): fmt_opt(..) → debit_row → paren(..)
```

`make_row` right-aligns each value in width `W` (10 or 9). So `$1.23` space_padded to
` $1.23 ` lands its `.` at offset **4** from the right (gutter 1 + `23` = 3, then `.`); a
`—` space_padded to ` — ` lands the `—` at offset **2** (gutter 1, then `—`) — two columns
right of the decimals. (Debit is the same story with `)` as the gutter: `($1.23)` has `.`
at offset 4, `(—)` has `—` at offset 2.)

## Spec

### Task 1 — add a pure `align_value` helper (panels.rs)

Add a module-level free fn near `fmt_tokens` (`panels.rs:490`) — the pure, testable core:

```rust
/// Right-pad a Budget-savings value so a no-value "—" lands on the decimal column of the
/// numeric neighbors. Amounts render `$X.XX` (2 decimals); once wrapped with a 1-char
/// gutter (a trailing space, or `)`) and right-aligned, the decimal point sits 4 columns
/// from the cell's right edge. A bare "—" sits 2 columns in, so it needs 2 trailing spaces
/// (the 2 decimal places) to reach the decimal column. Everything else passes through.
fn align_value(v: &str) -> String {
    if v == "—" {
        "—  ".to_string() // "—" + 2 spaces (the two decimal places a number shows)
    } else {
        v.to_string()
    }
}
```

### Task 2 — route `space_pad` and `paren` through `align_value` (savings_lines)

Change **only** the two closures (`panels.rs:659–660`) so a `—` value is decimal-aligned;
numbers are unchanged:

```rust
    let paren = |v: String| format!("({})", align_value(&v));
    let space_pad = |v: String| format!(" {} ", align_value(&v));
```

- **No call-site changes** — Baseline/Net/Architect keep calling `space_pad(...)` /
  `paren(...)` as they do.
- **The `debit_row` empty-check is unaffected** — it compares the **raw** `sess`/`mile`/
  `proj` (still `"—"` from `fmt_opt`) *before* `paren` applies `align_value`. Do **not**
  change the empty-check.
- Result: `space_pad("—")` → `" —   "` and `paren("—")` → `"(—  )"`, each placing the `—`
  at offset 4 (the decimal column) after right-alignment; `$X.XX` values are byte-for-byte
  unchanged.

## Acceptance criteria

- [ ] `align_value("—")` returns `"—  "` (dash + 2 spaces); `align_value("$1.23")` returns
      `"$1.23"` unchanged.
- [ ] In the dollars-mode savings block, a `—` value's `—` sits at the **same column** as
      the decimal point (`.`) of a numeric value in the same column (e.g. a `Baseline`
      `$X.XX` above an `Architect` `(—)`): the byte index of the `.` in the Baseline line
      equals the byte index of the `—` in the Architect line.
- [ ] Numeric values (`$X.XX`, `($X.XX)`) render exactly as before (07e alignment intact);
      the debit-row hide-when-empty behavior is unchanged.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.

## Test plan

- **`align_value_pads_dash_to_decimal_column`** (pure, mutation-sensitive) in `panels.rs`
  `mod tests`:
  ```rust
  assert_eq!(align_value("—"), "—  ");
  assert_eq!(align_value("$1.23"), "$1.23");
  assert_eq!(align_value("$0.00"), "$0.00");
  ```
  (Fails if `align_value` stops padding the dash.)
- **`savings_lines_dash_aligns_with_decimal`** (render-level) in `panels.rs` `mod tests` —
  **reuse the exact setup of `savings_lines_architect_cost_shown_from_project_costs`**
  (~panels.rs:1913): `rates { input_per_mtok: 5.0, output_per_mtok: 25.0, executor: default }`,
  `project_costs { executor_in: 1_000_000, …, architect: { input: 1_000_000, … },
  architect_cost: Some(5.0) }`, `milestone: None`, `BudgetDisplay::Dollars`. That fixture
  renders a **Baseline** row with `$` amounts (a `.`) and an **Architect** row whose
  **session** value is `(—)` (session architect is unpriced → `—`, project is `$5.00` → the
  row shows). Assert:
  ```rust
  let texts: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
  let baseline = texts.iter().find(|s| s.contains("Baseline:")).expect("Baseline row");
  let architect = texts.iter().find(|s| s.contains("Architect:")).expect("Architect row");
  assert_eq!(
      baseline.find('.'),
      architect.find('—'),
      "the — must sit at the decimal column:\n{baseline}\n{architect}"
  );
  ```
  Both are the Session (first) value column; the labels are `{:<10}`-padded so the value
  columns start at the same byte offset, and everything before the `.`/`—` is ASCII, so the
  byte index equals the column. **Mutation-sensitive:** under the old code `(—)`'s `—` is one
  column left of the `.`, so the indices differ and the assert fails.

## End-to-end verification

The dashboard is a live TUI (not hermetically capturable). The two unit tests above are the
pinned evidence — quote both results in the completion Update Log. Optionally run
`cargo run -p rexymcp -- dashboard --repo .` to eyeball the savings block with a `—` sitting
under the decimals, but the tests are the artifact.

## Authorizations

- Editing `mcp/src/dashboard/panels.rs` (`align_value` + `savings_lines`'s `space_pad`/`paren`
  closures + the two tests) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Tokens-mode (`BudgetDisplay::Tokens`) dash alignment.** Tokens mode renders `X.Yk`/`—`
  without the `space_pad`/`paren` wrappers, so this fix (which routes through those closures)
  does not touch it. A separate follow-up if wanted; note it, do not wire it here.
- **`fmt_opt` / the `debit_row` empty-check / `make_row` / the header** — unchanged; the fix
  is only the `align_value` helper + the two closure bodies.
- **Numeric-value formatting** (07e's `space_pad` gutter for `$X.XX`) — preserved exactly.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784781520224 (complete, server-authored)

**Summary:** Summary + Notes for review:

**What was built:** Added the `align_value` helper in `panels.rs` that right-pads a no-value `"—"` with 2 trailing spaces so it lands on the decimal column of `$X.XX` amounts. Routed both `space_pad` and `paren` closures through `align_value` so the fix applies to both non-debit (Baseline/Net) and debit (Executor/Architect) rows. Added two unit tests: `align_value_pads_dash_to_decimal_column` (pure helper) and `savings_lines_dash_aligns_with_decimal` (render-level alignment verification).

**Verification:** All 1032 tests pass, `cargo fmt --all --check`, `cargo build`, and `cargo clippy` all pass clean. The `align_value` literal `"—  "` is confirmed present in the source via grep. Working tree is clean with one committed change.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
 tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.10s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/dashboard/panels.rs` — +71 -2

**Commit:** a7573423fde316abe796fdae1dacb3956482b580

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-22

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** AEON-7/Qwen3.6-27B-AEON (35 turns, clean — no hard_fail, **no oscillation**)
- **Scope deviations:** none — `panels.rs` only (+71/−2), exactly Tasks 1–2: the pure
  `align_value` helper + routing `space_pad`/`paren` through it. No call-site changes; the
  `debit_row` empty-check, `make_row`, `fmt_opt`, the header, and tokens mode are untouched.
- **Verification:** reviewer re-ran all four gates green (fmt/build/clippy; `614` mcp-bin +
  `1032` executor-lib). Both new tests are **real and mutation-sensitive** — neutering
  `align_value` (drop the pad) makes `align_value_pads_dash_to_decimal_column` fail AND the
  render-level `savings_lines_dash_aligns_with_decimal` fail with "the — must sit at the
  decimal column" (confirmed by a temporary mutation, then restored), proving the render test
  genuinely finds the Architect `(—)` row and discriminates the alignment. The render test
  reuses the known-good `savings_lines_architect_cost_shown_from_project_costs` fixture, so no
  fixture fragility (the 07e lesson applied).
- **Calibration:** none new. A clean, contained fix on `panels.rs` — the 07e/07g pre-injection
  discipline (pin the exact known-good fixture when a test depends on a rendered row) and the
  anti-oscillation gotcha continue to work; the executor did not oscillate on the dashboard
  file this time.
