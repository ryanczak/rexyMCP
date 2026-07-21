# Bug 1 on phase-05b: `profile --cost` rows are indistinguishable + token summation untested

**Severity:** major
**Status:** open
**Filed:** 2026-07-20

## What's wrong

Two defects, both surfaced at review of the otherwise-green phase.

### (A) The `PHASE` column shows the coarse recorded `phase_id`, so distinct phases render as duplicate-looking rows — the report is unreadable.

Live `rexymcp profile --config rexymcp.toml --cost` over the real M35 corpus:

```
PHASE      MILESTONE                    ATTEMPTS  VERDICT             TOKENS   COST
phase-04   M35-metrics-cost-accounting  1         escalated           35960k   —
phase-04   M35-metrics-cost-accounting  2         approved_after_1     2836k   —
phase-05   M35-metrics-cost-accounting  1         approved_first_try   2661k   —
phase-05   M35-metrics-cost-accounting  1         approved_first_try   3424k   —
phase-05   M35-metrics-cost-accounting  1         approved_first_try  12328k   —
```

The **grouping is correct** — these are genuinely distinct phases (04a/04b;
05a-i/05a-ii/05a-iii/05b), each a separate `phase_doc_path`, correctly split
(note 04b's `attempts=2` from its bounce, 04a's `escalated` takeover). But
`format_phase_costs` renders the `PHASE` cell from `row.phase_id`
(`profile_cli.rs`, the `row.phase_id` in the `format!`), and the executor records
`phase_id` **coarsely** (`"phase-04"`, `"phase-05"` — the milestone-phase group,
not the sub-phase). So every 05* sub-phase prints as `phase-05` and the reader
cannot tell them apart. Acceptance criterion "one row per **shipped phase**" is
met in grouping but defeated in presentation: the rows are not identifiable.

The distinguishing identity is already in the data — `phase_doc_path`
(`…/phase-05a-iii-scorecard-by-cli.md`) — but `PhaseCost` drops it, keeping only
the coarse `phase_id`.

### (B) The token-summation-across-attempts has no test coverage (mutation-proven).

`aggregate_phase_costs` sums each token class across a phase's attempts
(`profile.rs:316–331`, four `saturating_add`s). The test named for this,
`phase_costs_sum_tokens_across_attempts`, builds its two runs via
`make_run_with_path`, which sets `tokens: Default::default()` (all zeros), and
asserts `tokens.input_tokens == 0` / `output_tokens == 0`. Because the inputs are
zero, the assertion holds regardless of whether summation happens.

**Mutation verification (run at review):** replacing
`.saturating_add(run.tokens.input_tokens)` with `.saturating_add(0)` (summation
becomes a no-op) leaves **all 6 phase-cost tests and all 28 profile tests green**.
The headline behavior of the phase — "cost summed across every attempt" — is
therefore unverified, contra the phase's Test plan, which pinned "two runs …
**each with a non-zero `TokenBreakdown`** … tokens equal to the per-class sum."

## What should happen

- (A) The `PHASE` column must **identify the phase**. Display the
  `phase_doc_path` **file stem** (e.g. `phase-05a-iii-scorecard-by-cli`) when a
  run carries a `phase_doc_path`, falling back to `phase_id` only when it does
  not. Distinct phases must render as distinct, identifiable rows.
- (B) A test must fail if the per-attempt token summation regresses.

## How to fix

**(A) Carry and display the phase-doc identity.**

1. In `mcp/src/profile.rs`, add a field to `PhaseCost`:
   ```rust
   /// The phase's `phase_doc_path` (from the latest attempt), when recorded.
   /// Its file stem is the human phase label; falls back to `phase_id`.
   pub phase_doc_path: Option<String>,
   ```
   In `aggregate_phase_costs`, capture `phase_doc_path` from the same latest run
   you already read `phase_id`/`model`/`milestone_id` from (extend the
   `latest_run` tuple), and set it on the emitted `PhaseCost`.

2. In `mcp/src/profile_cli.rs` `format_phase_costs`, render the `PHASE` cell from
   the doc-path **stem** when present, else `phase_id`. A small helper:
   ```rust
   fn phase_label(row: &PhaseCost) -> &str {
       row.phase_doc_path
           .as_deref()
           .and_then(|p| std::path::Path::new(p).file_stem().and_then(|s| s.to_str()))
           .unwrap_or(&row.phase_id)
   }
   ```
   (Widen the `PHASE` column so a `phase-05a-iii-scorecard-by-cli`-length stem
   isn't truncated; exact width is not pinned.)

**(B) Make the summation test real.** In
`phase_costs_sum_tokens_across_attempts`, set **distinct non-zero** per-class
tokens on the two runs (mutate `.tokens` after `make_run_with_path`, e.g. run A =
`{input:100, output:10, cache_read:5, cache_write:1}`, run B = `{input:200,
output:20, cache_read:7, cache_write:3}`) and assert the emitted
`PhaseCost.tokens` equals the **element-wise sum** (`input:300, output:30,
cache_read:12, cache_write:4`). The test must fail under the `.saturating_add(0)`
mutation on any of the four classes.

Add a `phase_label`-behavior test in `profile_cli.rs`: a `PhaseCost` with
`phase_doc_path = Some(".../phase-05a-iii-scorecard-by-cli.md")` renders `PHASE`
containing `phase-05a-iii-scorecard-by-cli` (not the bare `phase_id`); one with
`phase_doc_path = None` renders the `phase_id`.

## Verification

- [ ] `rexymcp profile --config rexymcp.toml --cost` shows distinct, identifiable
      `PHASE` labels (the 05* sub-phases no longer all read `phase-05`).
- [ ] `phase_costs_sum_tokens_across_attempts` asserts the non-zero element-wise
      sum and **fails** if any `saturating_add(run.tokens.<class>)` is replaced
      with `saturating_add(0)`.
- [ ] The new `format_phase_costs` label test passes (stem shown when
      `phase_doc_path` is `Some`, `phase_id` when `None`).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Notes for review

The `executor/src/ai/types.rs` `JsonSchema`-on-`TokenBreakdown` change from the
first dispatch is **accepted, keep it** — it was forced by the (architect-
over-specified) `#[derive(JsonSchema)]` on `PhaseCost`; it is minimal, adds no
dependency, and is harmless. Do **not** revert it in the fix. (A future cleanup
could drop `JsonSchema` from `PhaseCost` since it has no MCP consumer, but that is
out of scope for this bounce.)
