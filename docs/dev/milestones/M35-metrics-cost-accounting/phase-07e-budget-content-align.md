# Phase 07e: Budget panel content — align parenthesized debit values + combine the token in/out lines

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-07d
**Estimated diff:** ~90 lines
**Tags:** language=rust, kind=fix, size=m

## Goal

Two Budget-panel content fixes (user-directed, 2026-07-22):

1. **[#5] Align the parenthesized (debit) values** in the Budget savings block. Debit
   rows render `($1.23)`; non-debit rows render `$1.23`. Both are right-aligned in the
   same column width, so the `)` on a debit pushes its **digits one column left** of the
   non-debit digits above/below — the numbers don't line up. Fix so the **digits align
   column-wise** regardless of the parens (reserve a gutter column for the `(`/`)`).
2. **[#6] Combine the two token lines** — `budget_lines` emits `Tokens in:  N` and
   `Tokens out: N` on separate lines; merge into one: `Tokens in: N out: N`.

Both are in `mcp/src/dashboard/panels.rs`. #6 also **shortens the Budget panel by one
row** (relevant to the later trailing-blank phase, 07f).

## ⚠️ Read this first — editing discipline on `panels.rs`

`panels.rs` is dashboard rendering code. If a build/syntax error appears after an edit,
run `cargo build -p rexymcp` **once** and read the compiler's error location, then open
**only that region** with `read_file` and fix it. **Never** hunt for a syntax problem by
re-reading the whole file in a loop with `python3 -c`/`bash`/`sed`/`cat` — repeating the
same read-only inspection command trips the governor's oscillation terminator (it has
hard-failed an earlier run). Read once, fix once, verify with `cargo build`.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs` — `budget_lines` (452–487, the token lines are 459–462)
  for #6; `savings_lines` (506–707) for #5 — specifically the `paren` closure (662), the
  `make_row` closure (559–568), and the Baseline/Net `make_row` calls (675–680, 700–705).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above **and the editing-discipline note above**.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. `cargo build -p rexymcp` after each change.

## Current state

**[#6] `budget_lines`** (`panels.rs:459–462`) — the two token lines to merge:

```rust
    let mut lines = vec![
        Line::from(format!("Tokens in:  {in_toks}")),
        Line::from(format!("Tokens out: {out_toks}")),
    ];
```

**[#5] the savings-block value formatting** — `make_row` right-aligns each pre-formatted
value string in a fixed width (`panels.rs:559–568`):

```rust
    let make_row = |label: &str, v_sess: String, v_mile: String, v_proj: String| -> Line<'static> {
        if has_milestone {
            Line::from(format!("  {:<10}{:>10}{:>10}{:>10}", label, v_sess, v_mile, v_proj))
        } else {
            Line::from(format!("  {:<10}{:>9}{:>9}", label, v_sess, v_proj))
        }
    };
```

Debit rows (Executor, Architect) are parenthesized by the `paren` closure
(`panels.rs:662`) before `make_row`; Baseline and Net are **not**:

```rust
    let paren = |v: String| format!("({v})");
    // ... debit_row(...) → make_row(label, paren(sess), paren(mile), paren(proj))
    // ... Baseline → make_row("Baseline:", fmt_opt(sess.baseline), …)   // no parens
    // ... Net      → make_row("Net:",      fmt_opt(sess.net),      …)   // no parens
```

**The bug:** for column width `W` (10 or 9), `make_row` does `{:>W}`. A non-debit `$1.23`
right-aligns with its last digit at column `W`. A debit `($1.23)` right-aligns with the
`)` at column `W`, so its last digit is at column `W-1` — **one left** of the non-debit
digits above/below. This is what makes the parenthesized numbers not line up.

(Tokens mode — `display == BudgetDisplay::Tokens`, the block at 572–659 — has **no**
parenthesized rows, so its numbers already align. Leave it unchanged.)

## Spec

### Task 1 — combine the token in/out lines (`budget_lines`, panels.rs)

Replace the two-line vec (`panels.rs:459–462`) with a single line:

```rust
    let mut lines = vec![Line::from(format!("Tokens in: {in_toks} out: {out_toks}"))];
```

Update the `budget_lines` test that asserts the two separate token lines (search the
`#[cfg(test)]` block for the test rendering `budget_lines` and checking `Tokens in:` /
`Tokens out:` — likely `budget_lines_shows_tokens`) to assert the **single** combined
line `Tokens in: {in} out: {out}` instead. Do not change the `Tok/s` line or the
`(No metrics yet)` early return.

### Task 2 — align debit values in the savings block (`savings_lines`, panels.rs)

The debit values are 2 chars wider than non-debit (the `(` and `)`). To make the digits
align, give the **non-debit** dollars-mode values (Baseline, Net) the same 2-char gutter
— a leading and trailing space — so `make_row`'s `{:>W}` right-aligns everything with the
digits ending one column in from the field edge, matching the debit rows' digits.

1. Next to the `paren` closure (`panels.rs:662`), add a sibling:
   ```rust
   let space_pad = |v: String| format!(" {v} ");
   ```
   (A non-debit value gets a leading + trailing space, the same width the `(`/`)` add to
   a debit value.)
2. Wrap the **Baseline** `make_row` values (`panels.rs:675–680`) in `space_pad(...)`:
   ```rust
   out.push(make_row(
       "Baseline:",
       space_pad(fmt_opt(sess.baseline)),
       space_pad(fmt_opt(mile.baseline)),
       space_pad(fmt_opt(proj.baseline)),
   ));
   ```
3. Wrap the **Net** `make_row` values (`panels.rs:700–705`) in `space_pad(...)` the same
   way.
4. **Do not** touch `make_row`, the `paren`/`debit_row` path (Executor/Architect keep
   their parens), the header, or the tokens-mode block.

**Why this aligns the digits** — worked example (no-milestone, `W=9`, dollars mode):

```
before (broken):                 after (space_pad on Baseline/Net):
  Baseline:      $12.34            Baseline:     $12.34
  Executor:     ($5.00)            Executor:    ($5.00)
                     ^ digit col                     ^ digits now share a column
```

With `space_pad`, Baseline `" $12.34 "` right-aligned in 9 puts its last digit at column
`W-1` (a space in the gutter at `W`), matching Executor `"($5.00)"` whose last digit is
also at `W-1` (the `)` at `W`). The decimal points and last digits line up.

**Note (out of scope):** the column *header* labels (`Session`/`Milestone`/`Project`)
stay right-aligned to the full width `W`, so in dollars mode a header label may sit one
column right of the value digits below it. That is a cosmetic header-vs-value offset, not
the value-to-value misalignment this phase fixes — leave the header as is.

## Acceptance criteria

- [ ] The Budget panel shows **one** token line, `Tokens in: {in} out: {out}` (not two).
- [ ] In the dollars-mode savings block, the digits of a **non-debit** row (Baseline or
      Net) and a **debit** row (Executor or Architect) **align column-wise** — the decimal
      point (and last digit) of a value sits in the **same column** across rows,
      parenthesized or not.
- [ ] Debit rows are still parenthesized (`($1.23)`); non-debit rows are not.
- [ ] Tokens-mode savings block is unchanged.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.

## Test plan

- **`budget_lines_shows_tokens`** (edited) — assert the single combined line
  `Tokens in: {in} out: {out}` renders (and no separate `Tokens out:` line).
- **Add `savings_lines_debit_digits_align_with_non_debit`** — build a **dollars-mode**
  savings block with priced values so **both** a non-debit row (Baseline) and a debit row
  (Executor) render with `$` amounts (reuse the priced-scenario setup from an existing
  test such as `savings_lines_architect_cost_shown_from_project_costs`). Render to
  strings, find the Baseline line and the Executor line, and assert their **first `.`
  (decimal point) is at the same byte index** — the mutation-sensitive alignment pin:
  ```rust
  assert_eq!(
      baseline_line.find('.'),
      executor_line.find('.'),
      "debit and non-debit decimal points must align:\n{baseline_line}\n{executor_line}"
  );
  ```
  This **fails under the old code** (the `)` shifts the debit decimal point one column
  left) and passes after the `space_pad` fix. Also assert the Executor line contains `(`
  and `)` (parens preserved).

## End-to-end verification

The dashboard is a live TUI (not hermetically capturable). Rely on the unit tests as the
pinned evidence and quote `savings_lines_debit_digits_align_with_non_debit` +
`budget_lines_shows_tokens` in the completion Update Log. Optionally run
`cargo run -p rexymcp -- dashboard --repo .` to eyeball the aligned Budget savings block
and the single token line, but the tests are the artifact.

## Authorizations

- Editing `mcp/src/dashboard/panels.rs` (`budget_lines`, `savings_lines`, and their
  tests) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **The header-vs-value column offset** (see the Task 2 note) — leave the header labels
  right-aligned to the full width; do not restructure the header.
- **[#3] The trailing blank row** on Session/Budget/Context — the **next** phase (07f),
  drafted after this one so the Budget panel's final height (Assists removed in 07d, token
  lines combined here) is settled. Do **not** touch `render.rs`'s
  `Layout::vertical([Length(11), …])` header band here.
- **`make_row`'s signature, the `paren`/`debit_row` path, and the tokens-mode block** —
  unchanged; the fix is only the two `space_pad` wrappings + the token-line merge.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — 2026-07-22 (escalation)

**Chosen lever:** resume (`continue_phase`)
**Rationale:** The production fix is **correct** — the `space_pad` change and the combined
token line are on the working tree and 13/14 `savings_lines` tests pass. The `hard_fail`
was the governor's oscillation terminator firing on a read/test loop: the executor's *new*
test `savings_lines_debit_digits_align_with_non_debit` panics "Executor row missing"
because it set an **unpriced** executor (`rates.executor: ModelRates::default()`,
`executor_out: 0`) → Executor cost `$0.00` → the debit row is **hidden** by the empty-row
check, so the test never reaches its alignment assertion; the executor looped trying to
diagnose it. Work worth preserving + one test-setup wall = resume with a targeted hint
(use the priced-executor setup from `savings_lines_priced_executor_shows_non_zero`).

### Update — 2026-07-22 22:56 (started)

**Executor:** phase-07e executor
**Action:** Started phase 07e — combine token in/out lines and align debit values in Budget panel.
### Update — ts=1784765095018 (complete, server-authored)

**Summary:** The prior executor already implemented both production fixes (the `space_pad` closure + Baseline/Net wrappings for debit alignment, and the combined `Tokens in: N out: N` line). My only change was fixing the broken test `savings_lines_debit_digits_align_with_non_debit` — it panicked because the Executor debit row was hidden (unpriced executor → `$0.00` → row hidden). I gave the test a priced executor setup so the row renders and the alignment assertion passes. Removed the duplicated assertion and the stray duplicate comment. All 611 tests pass, all four gates (fmt, build, clippy, test) are clean, and the working tree is committed and clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.09s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.09s


TEST
ate_task::tests::flips_pending_task_to_active ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
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

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/dashboard/panels.rs` — +11 -17

**Commit:** 10e9c9adcdcfc79343bb6bb257c428d8f6702e0c

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-22

- **Verdict:** approved_after_1
- **Bounces:** 1 — one escalation round-trip (hard_fail → resume), no review-bug filed.
  Failure class: `oscillation_stall` (governor oscillation terminator fired on a
  read/test loop while the executor tried to diagnose a *failing test*).
- **Executor:** AEON-7/Qwen3.6-27B-AEON (first run: `space_pad` + token-combine production
  fix correct but hard_fail'd on the test; resume: test-only fix, complete @ 19 turns).
- **Scope deviations:** none — final diff is `mcp/src/dashboard/panels.rs` only. `#6`:
  `budget_lines` emits one `Tokens in: N out: N` line. `#5`: `space_pad` closure + the six
  Baseline/Net wrappings; `make_row`, the `paren`/`debit_row` path, the header, and the
  tokens-mode block untouched, exactly as specced.
- **Verification:** reviewer re-ran all four gates green (fmt/build/clippy; `611` mcp-bin +
  `1032` executor-lib). Both new/edited tests are **real and mutation-sensitive**:
  `budget_lines_shows_tokens` pins the single combined token line; and neutering `space_pad`
  to a no-op makes `savings_lines_debit_digits_align_with_non_debit` **fail** with "debit and
  non-debit decimal points must align" (confirmed by a temporary mutation, then restored) —
  proving it genuinely guards the alignment fix.
- **Calibration:** the hard_fail was a **read/test oscillation loop** — the executor's own
  new test failed ("Executor row missing": an unpriced fixture → the `$0.00` debit row is
  *hidden*), and it looped re-reading + re-testing without diagnosing the hidden row. Two
  lessons for the M35-close fold: (1) **third `panels.rs`/dashboard oscillation of the arc**
  — reinforces the "read-only-inspection repetition → advisory" governor fold and the
  `oscillation_stall` failure-class gap; (2) **pre-injection**: when a spec asks for a test
  that depends on a *rendered row appearing*, pin the **exact fixture that makes it appear**
  (here: a priced executor) — a rendering filter (`$0.00` debit rows hidden) silently
  suppressed the row and the executor couldn't self-diagnose it. The resume, with the exact
  priced-fixture hint, landed clean in 19 turns.
