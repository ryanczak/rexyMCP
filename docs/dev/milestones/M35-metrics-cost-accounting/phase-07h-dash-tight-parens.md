# Phase 07h: Tight parens on the debit "—" — `(—)`, padding outside, still decimal-aligned

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
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
