# Phase 07g: Align the Budget savings "—" (no value) with the decimal column

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
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
