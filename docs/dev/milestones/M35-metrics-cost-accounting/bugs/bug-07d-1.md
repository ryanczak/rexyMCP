# Bug 1 on phase-07d: `savings_lines_baseline_dash_when_rates_unset` test was deleted (coverage regression)

**Severity:** minor
**Status:** verified
**Filed:** 2026-07-22
**Fixed:** 2026-07-22 (commit 6791e9e — `savings_lines_baseline_dash_when_rates_unset` restored; both tests pass)

## What's wrong

Task 2's test plan authorized **adding** a new `savings_lines_has_no_assists_row` test.
Instead, the executor **repurposed an existing, unrelated test** — it renamed
`savings_lines_baseline_dash_when_rates_unset` to `savings_lines_has_no_assists_row` and
replaced its assertions, **deleting** the baseline-dash coverage.

The lost test asserted that, with rates unset, the Baseline **and** Net rows render `—`
(the `fmt_opt(None) → "—"` path in `savings_lines`, `panels.rs`). That behavior now has
**no** test — confirmed:

```
$ grep -rn "baseline_dash_when_rates_unset\|when no rates\|Baseline shows" mcp/src/dashboard/panels.rs
(no matches)
```

Only `savings_lines_has_no_assists_row` remains (panels.rs:1857). The diff shows the
baseline-dash assertions (`baseline_line.contains('—')` / `net_line.contains('—')`) were
removed, not relocated.

This is an **undeclared scope deviation**: the phase doc did not authorize touching
`savings_lines_baseline_dash_when_rates_unset`, and cannibalizing it dropped real
coverage. (All three production changes — profile help, Assists-row removal, border text —
are correct; this bug is test-only.)

## What should happen

**Both** tests present and passing:

- `savings_lines_baseline_dash_when_rates_unset` — **restored** to its original behavior:
  with rates unset, assert the Baseline row and the Net row each contain `—`.
- `savings_lines_has_no_assists_row` — kept as a **separate** new test: assert no rendered
  line starts with `  Assists:`.

Do not merge them; they verify different behaviors (dash-on-unset-rates vs no-assists-row).

## How to fix

In `mcp/src/dashboard/panels.rs` `#[cfg(test)] mod tests`:

1. Re-add `savings_lines_baseline_dash_when_rates_unset` with its original assertions —
   build a `savings_lines(...)` call with **unset** rates (`BudgetRates::default()` with no
   priced rates, `BudgetDisplay::Dollars`), then assert the Baseline line and Net line each
   `contains('—')`.
2. Keep the existing `savings_lines_has_no_assists_row` (panels.rs:1857) unchanged — its
   `!texts.iter().any(|s| s.starts_with("  Assists:"))` assertion is correct.

Leave all other 07d changes (main.rs profile help, render.rs border, the Assists-row
removal, the `savings_lines_omits_zero_debit_rows` edit, the deleted
`savings_lines_assists_shows_project_escalation_count`) exactly as they are — they are
approved; only restore the lost baseline-dash test.

## Verification

- [ ] `savings_lines_baseline_dash_when_rates_unset` **and** `savings_lines_has_no_assists_row`
      both exist in `panels.rs` and both pass.
- [ ] `grep -c "baseline_dash_when_rates_unset" mcp/src/dashboard/panels.rs` == 1.
- [ ] `cargo test -p rexymcp savings_lines` — all green.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
