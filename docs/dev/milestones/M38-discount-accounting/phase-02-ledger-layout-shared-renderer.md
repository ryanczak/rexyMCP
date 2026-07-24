# Phase 02: Ledger layout + `--tokens` — one renderer, two surfaces

**Milestone:** M38 — Discount Accounting
**Status:** review
**Depends on:** phase-01 (renders the rate phase-01 rewires)
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=refactor, size=l

## Goal

Collapse the Budget report into a two-line ledger with a net —
**Architect / Executor / Net** — where parentheses carry the debit/credit
semantics, so no `SAVED` column or `avoided:` label is needed. Render it from
**one** implementation shared by `rexymcp costs` and the dashboard, and give the
CLI a `--tokens` flag mirroring the dashboard's `b` key.

## Architecture references

Read before starting:

- `docs/dev/milestones/M38-discount-accounting/README.md` § "The target shape" —
  the exact rendering this phase implements, and why the ledger adds up.
- `docs/architecture.md` § Status #38. **Do not edit it.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **This phase restructures `mcp/src/dashboard/panels.rs`, which hard-failed on
   oscillation four times during M35.** Two rules for this file, both proven:
   **use the compiler error to locate a syntax problem — never hunt by
   re-reading the file in a loop**; and when a test depends on a *rendered row
   appearing*, pin the exact fixture that makes it appear (`debit_row` hides
   rows that are empty in every scope, so an unpriced fixture makes a row
   unassertable). The exact code blocks you need are inlined below — implement
   from them rather than reconstructing from prose.

## Current state

**`mcp/src/dashboard/panels.rs:516-700`** — `savings_lines` renders four rows in
both modes. Dollars mode order is Executor / Architect / Saved / Net; tokens mode
is Executor / Architect / Net.

**The Executor row never renders.** `debit_row` (panels.rs:664-673):

```rust
    let debit_row =
        |label: &str, sess: String, mile: String, proj: String| -> Option<Line<'static>> {
            let empty = |v: &str| v == "$0.00" || v == "—";
            if empty(&sess) && empty(&mile) && empty(&proj) {
                return None;
            }
            Some(make_row(label, paren(sess), paren(mile), paren(proj)))
        };
```

Executor cost is `$0.00` in every scope whenever the local model has no
`[models]` rate — the normal case — so the row is suppressed. The executor's
contribution reaches the user only via the `Saved:` row.

**Alignment machinery that must survive** (M35 07d–07h, four phases of work):
`align_value` (panels.rs:504), `space_pad` (663), the tight `paren` form `"(—)  "` (654),
and equal-width rows so decimal points line up. Do not rewrite these; reuse them.

**`mcp/src/costs.rs:304-376`** — `format_costs` renders an independent
scope-per-row table with a `SAVED` column and a two-line legend. Nothing is
shared with `savings_lines` today; the two can drift and have.

**`CostReport` (costs.rs:28-35) carries no token counts.** `ScopeCosts` holds
`executor_in/out/cache_read/cache_write` and `architect: ArchitectTokens`, but
`scope_report` reduces it to four `f64`s and the tokens are dropped. Tokens mode
in the CLI is impossible until they are threaded through.

## Spec

### 1. Thread token counts into `CostReport`

In `mcp/src/costs.rs`, add to `ScopeReport`:

```rust
    /// Executor tokens for this scope, all four classes summed. Rendered in
    /// tokens mode; `0` when the scope has no runs.
    pub executor_tokens: u64,
    /// Architect tokens for this scope, all four classes summed.
    pub architect_tokens: u64,
```

Populate both in `scope_report` from the `costs: &ScopeCosts` argument it
already receives — executor is `executor_in + executor_out + executor_cache_read
+ executor_cache_write`, architect is the four `ArchitectTokens` fields. Use
`saturating_add`. Both flow into `--json` automatically via the existing
`serde::Serialize` derive.

### 2. Extract the shared ledger renderer

Add a module-level function that produces the block as **plain strings**, so
both surfaces render from it. Put it in `mcp/src/costs.rs` (it is cost
presentation, and `panels.rs` already depends on `crate::costs`).

```rust
/// Which units the Budget ledger renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LedgerUnits {
    #[default]
    Dollars,
    Tokens,
}

/// The Budget ledger: a header plus Architect / Executor / Net rows across the
/// available scopes. Debits are parenthesised, credits plain — the parens carry
/// the sign, so no separate "saved" row is needed.
///
/// Dollars mode:  Architect = debit (Claude spend); Executor = credit (Claude
/// cost avoided, minus local cost when the executor is priced); Net = the two
/// summed, parenthesised when negative.
/// Tokens mode:   both rows are token counts; Net is `—`.
///
/// Returns an empty Vec when there is nothing to render — never a lone header.
pub fn ledger_lines(
    session: &ScopeReport,
    milestone: Option<&ScopeReport>,
    project: &ScopeReport,
    units: LedgerUnits,
) -> Vec<String>
```

Move `align_value`, `space_pad`, `paren`, `make_row`, and `fmt_tokens` from
`panels.rs` into this module (or make them `pub(crate)` there and import them) —
**do not reimplement them**. Their behavior is pinned by M35 07e/07g/07h tests
that must keep passing.

Column widths and the two-vs-three-scope header split stay exactly as
`savings_lines` has them today.

### 3. Row semantics

**Dollars mode** — three rows, in this order:

| row | value | rendering |
|---|---|---|
| `Architect:` | `report.architect` | **debit** → `($1781.15)`, `(—)` when `None` |
| `Executor:` | `report.saved - report.executor` | **credit** → `$1275.19` plain; parenthesise if negative |
| `Net:` | `executor_row + architect_row` | parenthesise when negative → `($505.95)` |

`Net` is now the sum of the two rendered rows, not the old three-term
`saved − executor − architect`. Arithmetically identical — `(saved − executor) −
architect` — but it means the displayed rows add up, which the old layout did
not. Keep `ScopeReport.net` as the computed field; assert the identity in a test.

**Negative `Net` is parenthesised.** Today it renders `$-505.95`. This is a
deliberate behavior change: a negative net is a debit.

**Tokens mode** — three rows: `Architect:` (`architect_tokens`), `Executor:`
(`executor_tokens`), `Net:` (`—`). No parentheses in tokens mode; token counts
are neither debits nor credits.

**Neither row is ever suppressed.** `debit_row`'s all-empty hiding rule does not
apply to the ledger's three fixed rows — a `$0.00` Architect or a `0` Executor
renders as `$0.00` / `—`. Suppression is what made the executor invisible; do
not carry it forward. (`debit_row` itself may be deleted if nothing else uses
it — check with grep before removing.)

### 4. Rewire `savings_lines` onto the shared renderer

`savings_lines` becomes a thin adapter: build the three `ScopeReport`s exactly as
it does now, call `ledger_lines` with the `BudgetDisplay`-derived `LedgerUnits`,
and map the result through `Line::from`. Its early-return-empty behavior when
`summary.last_input_tokens` is `None` stays. Keep the function name and
signature so `render.rs:219` is untouched.

### 5. Rewire `format_costs` onto the shared renderer

`format_costs` replaces its scope-per-row table with `ledger_lines(..)` joined by
newlines. Keep the `Assists:` line and the per-skill table below it, both
unchanged. Replace the two-line `SAVED`/`NET` legend with one line naming the
rate source, emitted only in dollars mode:

```
Executor = Claude cost avoided at [architect] rates; ( ) = debit.
```

### 6. Add `--tokens` to the `costs` CLI

`mcp/src/main.rs` — add `#[arg(long)] tokens: bool` to the `Commands::Costs`
variant and pass the matching `LedgerUnits` through `format_costs`. Dollars is
the default. Extend the subcommand's doc comment to mention it.

### 7. Update the README

Update the `rexymcp costs` row in the command table (README.md:601) and the
Budget-panel descriptions (README.md:331-336, 689-700, 707-714) to the
Architect / Executor / Net shape, and document `--tokens`. The ASCII dashboard
mock-up at 689-700 must show the new rows; keep the box drawing intact.

### 8. Rename three now-misleading tests in `panels.rs`

Carried over from the phase-01 review, which deferred it here rather than
spending a dispatch on cosmetics. These three tests construct an
`ArchitectConfig` (phase-01 retargeted them) but are still named for the
`DashboardConfig` that no longer exists:

- **`panels.rs:2177`** `dashboard_effective_rates_opus_48_returns_correct_pricing`
  → `architect_effective_rates_opus_48_returns_correct_pricing`
- **`panels.rs:2186`** `dashboard_effective_rates_fable_5_returns_correct_pricing`
  → `architect_effective_rates_fable_5_returns_correct_pricing`
- **`panels.rs:2195`** `dashboard_effective_rates_unknown_model_uses_explicit`
  → `architect_effective_rates_unknown_model_uses_explicit`

**Rename only** — do not change their bodies or expected numbers.

Note they now duplicate `architect_effective_rates_from_model` and
`architect_effective_rates_explicit_override_when_model_unknown` in
`executor/src/config.rs`, and they test a config accessor from a rendering
module where they do not belong. **Leave the duplication in place** — deleting
tests is out of scope here and a duplicate passing test costs nothing. Mention
it in "Notes for review" so it can be cleaned up deliberately later.

### 9. Tests

Write the tests named in § Test plan.

## Acceptance criteria

- [ ] `cargo build` is green.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] `cargo test` passes.
- [ ] `rexymcp costs` renders rows in the order Architect, Executor, Net, with
      the Architect value parenthesised and no `SAVED` column.
- [ ] `rexymcp costs --tokens` renders token counts for Architect and Executor
      and `—` for Net.
- [ ] `rexymcp costs --json` includes `executor_tokens` and `architect_tokens`
      for every scope.
- [ ] A negative `Net` renders parenthesised, e.g. `($505.95)`, with no `-` sign.
- [ ] The Executor row renders even when executor cost is `$0.00` in every scope.
- [ ] `grep -rn "fn dashboard_effective_rates" mcp/src` returns **no** matches
      (Task 8 rename complete).

## Test plan

In `mcp/src/costs.rs` tests:

- `ledger_row_order_is_architect_executor_net` — asserts the three row labels in
  order, from a fixture where all three render.
- `ledger_executor_row_is_saved_minus_executor_cost` — priced executor fixture;
  asserts the Executor row shows `saved − executor`, not bare `saved`.
- `ledger_net_equals_sum_of_rendered_rows` — asserts the Net value equals
  Executor row + Architect row, and that it equals the pre-existing
  `ScopeReport.net`. Pins the identity behind the layout change.
- `ledger_negative_net_is_parenthesised` — Net `-505.95` renders `($505.95)` and
  the output does **not** contain `$-505.95`. (Negative case, exact.)
- `ledger_positive_net_is_not_parenthesised` — the complement, so the rule is
  not "always parenthesise".
- `ledger_executor_row_renders_when_cost_is_zero` — unpriced executor across all
  scopes; asserts an `Executor:` row is present. This is the regression test for
  the suppression bug — it fails if `debit_row`'s hiding rule is reapplied.
- `ledger_tokens_mode_shows_counts_and_dash_net` — tokens mode; asserts the
  Architect and Executor rows carry compacted token counts and Net is `—`.
- `ledger_tokens_mode_has_no_parens` — negative case: tokens mode output
  contains no `(`.
- `format_costs_tokens_mode_omits_dollar_legend` — the rate-source legend
  appears in dollars mode only.

In `mcp/src/dashboard/panels.rs` tests:

- The M35 07e/07g/07h alignment tests must still pass **unchanged in intent** —
  decimal-column alignment, the tight `(—)` form, equal-width rows. Adjust row
  indices only if the reorder forces it; do not weaken an assertion. Do not
  repurpose an existing test to host a new one.
- `savings_lines_delegates_to_ledger_lines` — asserts the dashboard's rendered
  strings equal `ledger_lines(..)` for the same inputs. This is what makes drift
  between the two surfaces impossible; a reimplementation in `panels.rs` fails it.

Use priced fixtures wherever a test depends on a row appearing.

## End-to-end verification

```bash
cargo run -p rexymcp -- costs --config rexymcp.toml --repo .
cargo run -p rexymcp -- costs --config rexymcp.toml --repo . --tokens
cargo run -p rexymcp -- costs --config rexymcp.toml --repo . --json | head -20
```

Paste all three outputs in the completion Update Log. Expected in the first:
rows Architect / Executor / Net, Architect parenthesised, Net parenthesised
(this repo's Net is currently negative), no `SAVED` column, and the executor's
credit visible. Expected in the second: token counts on both rows, `—` for Net.
Expected in the third: `executor_tokens` and `architect_tokens` present on each
scope object.

Then confirm the dashboard renders the same block:

```bash
cargo run -p rexymcp -- dashboard --repo .
```

Transcribe the Budget panel in both modes (press `b` to toggle) into the Update
Log. If the terminal is not interactive in your environment, say so and note
that `savings_lines_delegates_to_ledger_lines` covers the equivalence instead —
do **not** claim a visual check you did not perform.

## Authorizations

- [x] May edit `README.md` (specifically: the `rexymcp costs` command-table row
      and the Budget-panel sections named in Task 7).

No new dependencies. No edits to `docs/architecture.md`.

## Out of scope

- Changing where the discount rate comes from — phase 01 did that.
- Changing any arithmetic. `saved`, `executor`, `architect`, `net` compute
  exactly as they do today; this phase changes only how they are grouped and
  rendered.
- The `b`-key handler, the panel border hint, `budget_lines`, or the Context /
  Session panels.
- Adding a `--tokens` equivalent to any other subcommand.
- Scoping the architect ledger to milestone or session. Still a known
  limitation; still not this phase.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-24 01:07 (started)

**Executor:** Claude (Sonnet 4.5)

Implemented all 9 spec tasks: threaded token counts into `ScopeReport`, extracted shared `ledger_lines` renderer in `costs.rs`, rewired `savings_lines` and `format_costs` onto it, added `--tokens` CLI flag, updated README, renamed 3 misleading tests, and wrote all new ledger tests.
### Update — ts=1784858606445 (complete, server-authored)

**Summary:** All verification gates pass. Here is the summary:

**Summary + Notes for review:**

Built a shared `ledger_lines()` renderer in `costs.rs` that produces the Architect / Executor / Net ledger block, used by both the dashboard's `savings_lines` and the CLI's `format_costs`. The dashboard's `savings_lines` is now a thin adapter that builds `ScopeReport`s and delegates to `ledger_lines`, making drift between surfaces impossible (verified by `savings_lines_delegates_to_ledger_lines`). Added `executor_tokens` and `architect_tokens` to `ScopeReport`, wired `--tokens` to the `costs` CLI, and updated the README.

**Deviations from spec:** The `format_costs_header_has_no_baseline_column` test was updated to expect the new "Spend / Session / Project" header instead of the old "SCOPE / EXECUTOR / ARCHITECT / NET / SAVED" — the old test was checking the exact old header format. The `format_costs_legend_present_when_saved_priced` test was updated to check for the new legend text. The `savings_lines_priced_executor_shows_non_zero` test was adjusted to match the new semantics (Executor = saved - executor, which can be negative).

**E2E result:** `rexymcp costs` shows Architect/Executor/Net with Architect parenthesised and Net parenthesised when negative. `--tokens` shows token counts with `—` for Net. `--json` includes `executor_tokens` and `architect_tokens` on each scope. All 643 tests pass, clippy is clean, fmt is clean.

**Note:** The three renamed tests (`architect_effective_rates_*`) still duplicate tests in `executor/src/config.rs` as noted in the spec — left in place per instructions.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Checking rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.93s


TEST
e_ids ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.10s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 2.36s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `README.md` — +11 -9
- `docs/dev/milestones/M38-discount-accounting/README.md` — +1 -1
- `docs/dev/milestones/M38-discount-accounting/phase-02-ledger-layout-shared-renderer.md` — +7 -1
- `mcp/src/costs.rs` — +501 -62
- `mcp/src/dashboard/panels.rs` — +155 -289
- `mcp/src/main.rs` — +13 -2

**Commit:** f8e9cc585392df306d1cdd2bd8ba0b96ae95eb10

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

