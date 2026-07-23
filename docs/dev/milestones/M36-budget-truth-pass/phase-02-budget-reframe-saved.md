# Phase 02: Budget reframe — Baseline becomes Executor's `saved`

**Milestone:** M36 — Budget Truth Pass
**Status:** todo
**Depends on:** none
**Estimated diff:** ~220 lines (mostly a mechanical field rename across tests)
**Tags:** language=rust, kind=refactor, size=m

## Goal

`Baseline` is the first number the user sees in `rexymcp costs` and in the
dashboard Budget panel, and it reads as a cost — but it is a **counterfactual**:
executor tokens repriced at cloud rates, i.e. money *not* spent. Reframe it so
the panel answers "what am I spending?" first: **Architect is the debit,
Executor token usage is a saving.** The number itself survives — renamed
`saved`, moved below the two real buckets, and stated as a property of the
Executor row. `Net` survives unchanged as the bottom line.

No arithmetic changes in this phase. Only the field name, the row order, and the
labels.

## Architecture references

Read before starting:

- `docs/dev/milestones/M36-budget-truth-pass/README.md` — the framing this
  phase implements.
- `docs/architecture.md` § Status #35 — describes the surface being revised.
  **Do not edit it**; the architect amends it separately.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**`mcp/src/costs.rs:15-24`** — the field carrying the counterfactual:

```rust
/// One scope's four cost lines, in dollars. `baseline`/`net` are `None` when no
/// baseline rate is configured (rendered `—`); `executor`/`architect` are always
/// present (`0.0` when unpriced).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
pub struct ScopeReport {
    pub baseline: Option<f64>,
    pub executor: f64,
    pub architect: Option<f64>,
    pub net: Option<f64>,
}
```

**`mcp/src/costs.rs:57-68`** — the arithmetic, which this phase does **not**
change:

```rust
    let baseline_cost = if no_baseline { None } else {
        Some(per_m(costs.executor_in, baseline.input_per_mtok)
           + per_m(costs.executor_out, baseline.output_per_mtok))
    };
    let net = match (baseline_cost, architect) {
        (Some(b), Some(a)) => Some(b - executor - a),
        _ => None,
    };
```

**`mcp/src/costs.rs:295-316`** — the CLI table header and row order
(`SCOPE BASELINE EXECUTOR ARCHITECT NET`).

**`mcp/src/dashboard/panels.rs:516-726`** — `savings_lines`, which renders the
block header (`"Savings"` / `"Savings (tok)"`) and four rows in the order
Baseline / Executor / Architect / Net, in both a Dollars and a Tokens mode.

**Supporting evidence that `saved` is the right name:** the config fields this
number is computed from are *already* called `saved_input_per_mtok` /
`saved_output_per_mtok` (`executor/src/config.rs:180-186`, doc comment: "The
`$ saved` baseline"). Only the display ever said "Baseline". No config key
changes in this phase.

**Rendering machinery to preserve.** Phases M35 07d–07h tuned decimal-column
alignment in this block: `align_value` (panels.rs:504), `space_pad`, the tight
`paren` form `"(—)  "`, and `debit_row`'s all-empty hiding rule. Keep all of it.
`Executor` and `Architect` stay **debit** rows (parenthesised, hidden when empty
in every scope); `Saved` and `Net` stay non-debit rows rendered via `space_pad`.

## Spec

### 1. Rename `ScopeReport.baseline` → `saved`

In `mcp/src/costs.rs`, rename the struct field and its doc comment. This is a
**wide mechanical rename with no additive alternative** — the field is public
and every site stops compiling at once. Work file-by-file in this order and run
`cargo build` after each file so the cascade is bounded:

**File 1 — `mcp/src/costs.rs`.** Grep-verified prod sites: lines 15, 16, 20, 38,
44, 47, 57, 61, 62, 65, 71, 207, 227, 244, 256, 276, 304. Test-module sites:
26 further occurrences below line 360. Rename:

- the field `baseline: Option<f64>` → `saved: Option<f64>`;
- the local `baseline_cost` → `saved_cost` and `no_baseline` → `no_saved_rates`;
- the `scope_report` parameter `baseline: &BudgetRates` → `saved_rates:
  &BudgetRates` (it is a *rates* struct, not a cost — the old name conflated
  them);
- local bindings named `baseline` at the call sites (207, 227, 244, 256, 276)
  → `saved_rates`;
- test helper `priced_baseline()` → `priced_saved_rates()`, and test names
  containing `baseline` to the matching `saved` form (e.g.
  `scope_report_no_baseline_is_none` → `scope_report_no_saved_rate_is_none`).

Then `cargo build` — it must be green before moving on.

**File 2 — `mcp/src/dashboard/panels.rs`.** Prod sites: lines 34 (doc comment on
`BudgetRates`), 513 (doc comment on `savings_lines`), 586, 695-698. Test-module
sites: 30 further occurrences below line 733. Update field accesses
(`sess.baseline` → `sess.saved`, etc.) and the test names/assertions.

Then `cargo build` again.

Keep the *function* name `savings_lines` — it renders the savings block and the
name is still accurate.

### 2. Reorder and relabel the CLI table in `mcp/src/costs.rs::format_costs`

New column order and header, with `SAVED` moved to the end so the two real
buckets lead:

```
SCOPE         EXECUTOR ARCHITECT       NET     SAVED
```

Row values in the matching order: `fmt_dollars(r.executor)`, `fmt_opt(r.architect)`,
`fmt_opt(r.net)`, `fmt_opt(r.saved)`. Column widths stay `{:<12}` for the label
and `{:>10}` for each value.

After the `Assists:` line and before the per-skill table, emit a blank line and a
two-line legend so the semantics are on-screen:

```
SAVED = executor tokens priced at Claude rates — work not billed to Claude.
NET   = SAVED − EXECUTOR − ARCHITECT.
```

Emit the legend only when at least one scope has a `Some(saved)`; when no saved
rate is configured every `SAVED`/`NET` cell is `—` and the legend is noise.

### 3. Relabel and reorder the dashboard block in `savings_lines`

**Dollars mode** — header label changes from `"Savings"` to `"Spend"`; row order
becomes Executor, Architect, Saved, Net:

```
Spend            Session  Milestone    Project
  Executor:       ($0.00)    ($0.00)    ($0.00)
  Architect:          (—)        (—)  ($1720.37)
  Saved:            $2.17   $1228.72   $1228.72
  Net:                  —          —   $-491.65
```

Mechanically: move the `Baseline:` `make_row` call (currently panels.rs:694-699)
to sit **after** the Architect `debit_row` and **before** the `Net:` row, and
relabel it `"Saved:"`. Keep it a `space_pad`/non-debit row. The Executor and
Architect `debit_row` calls are unchanged apart from the field rename.

**Tokens mode** — header label changes from `"Savings (tok)"` to
`"Spend (tok)"`, and the `Baseline:` row is **removed**, not renamed. In tokens
mode it rendered `executor_in + executor_out`, a strict subset of the Executor
row's all-four-classes total on the very next line — a redundant row that under-
reports the same quantity. Tokens mode becomes three rows: Executor, Architect,
Net (`—`).

Leave the `b`-key toggle, the panel border hint, and `budget_lines` untouched.

### 4. Update the README

In `README.md`, update the four places that name the old shape — lines 27, 335,
601, 693, 712 — to the new labels and order. Line 693 is an ASCII dashboard
mock-up: update the `Baseline:` row to `Saved:` and keep the surrounding box
drawing intact. Do not restructure the surrounding prose beyond what the rename
requires.

### 5. Tests

Update the renamed tests per Task 1, and add the new assertions in § Test plan.

## Acceptance criteria

- [ ] `cargo build` is green.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] `cargo test -p rexymcp` passes.
- [ ] `grep -rn "Baseline\|BASELINE" mcp/src README.md` returns **no** matches in
      user-visible strings. (Matches inside a doc comment explaining the
      historical name are acceptable and should be called out in Notes for
      review; a match in a rendered label is a failure.)
- [ ] The dollars-mode block renders rows in the order Executor, Architect,
      Saved, Net.
- [ ] The tokens-mode block renders exactly three rows.

## Test plan

In `mcp/src/costs.rs` tests:

- `format_costs_header_has_no_baseline_column` — asserts the header line is
  **exactly** `format!("{:<12}{:>10}{:>10}{:>10}{:>10}", "SCOPE", "EXECUTOR",
  "ARCHITECT", "NET", "SAVED")`. Exact equality on the full line, not
  `contains` — a substring check passes on a malformed superset.
- `format_costs_legend_present_when_saved_priced` — with a priced saved rate,
  asserts both legend lines appear.
- `format_costs_legend_absent_when_unpriced` — with zero saved rates, asserts
  the string `"SAVED ="` does **not** appear. (Negative case.)

In `mcp/src/dashboard/panels.rs` tests:

- `savings_lines_row_order_is_executor_architect_saved_net` — with a fixture
  where all four rows render, asserts the row labels in order. Assert on the
  sequence of labels, not on one row in isolation.
- `savings_lines_header_says_spend` — asserts the dollars-mode header begins
  `"Spend"` and does **not** contain `"Savings"`.
- `savings_lines_tokens_mode_has_three_rows` — asserts tokens mode emits header
  + 3 rows, and that no row is labelled `"Saved:"`. (Negative case: guards
  against renaming the row instead of removing it.)
- The existing alignment tests from M35 07e/07g/07h must still pass **unchanged
  in intent** — the decimal column and the tight `(—)` form are preserved. If a
  row-order change forces an index shift in an existing test, adjust the index
  only; do not weaken the assertion. Do not repurpose an existing test to host a
  new assertion — add new ones.

## End-to-end verification

```bash
cargo run -p rexymcp -- costs --config rexymcp.toml --repo .
```

Paste the actual output in the completion Update Log. Expected: the header reads
`SCOPE  EXECUTOR  ARCHITECT  NET  SAVED`, no line contains "Baseline", and the
per-skill table below is unchanged.

Then launch the dashboard and press `b` once to toggle to tokens mode:

```bash
cargo run -p rexymcp -- dashboard --repo .
```

Describe the rendered Budget block in both modes in the Update Log (a pasted
screenshot-style transcription of the block is fine). Confirm the dollars block
has four rows in the specified order and the tokens block has three.

## Authorizations

- [x] May edit `README.md` (specifically: the `costs`/Budget-panel descriptions
      at lines 27, 335, 601, 693, 712).

No new dependencies. No edits to `docs/architecture.md`.

## Out of scope

- Changing any arithmetic. `saved` and `net` compute exactly as `baseline` and
  `net` do today.
- Renaming the config keys `saved_input_per_mtok` / `saved_output_per_mtok` —
  they are already correctly named.
- Renaming the `other` skill bucket (phase 03).
- Scoping the architect ledger to milestone or session, or removing the `—`
  cells that result from it. Known limitation, separate work.
- Any change to `budget_lines`, the `b`-key handler, or the panel border hint.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
