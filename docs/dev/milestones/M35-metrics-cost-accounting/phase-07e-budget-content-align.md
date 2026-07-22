# Phase 07e: Budget panel content — align parenthesized debit values + combine the token in/out lines

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
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
