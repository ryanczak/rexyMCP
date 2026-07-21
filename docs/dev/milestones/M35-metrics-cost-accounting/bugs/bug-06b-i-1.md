# Bug 1 on phase-06b-i: cache pricing + derived-executor rendering are untested (headline feature uncovered)

**Severity:** major
**Status:** resolved (2026-07-21, fix dispatch 77d0490; both tests mutation-verified at review)
**Filed:** 2026-07-21

## What's wrong

The phase's two headline behaviors have **no test coverage**, contra the Test plan
(which named both) and STANDARDS §1 "new code is covered by tests":

### (A) Task 3 — the executor **cache** pricing in `scope_report` is unverified.

`scope_report` (mcp/src/costs.rs:48–54) now adds two cache terms to the executor
cost:

```rust
+ per_m(costs.executor_cache_read, exec_rates.cache_read_per_mtok)
+ per_m(costs.executor_cache_write, exec_rates.cache_creation_per_mtok);
```

But the only priced `scope_report` test (`scope_report_priced_executor_and_baseline`,
costs.rs:300) sets `executor_cache_read: 0` / `executor_cache_write: 0`, so the two
cache terms are never exercised with non-zero values.

**Mutation-verified at review:** replacing both cache rates with `0.0` (zeroing the
cache pricing) leaves **all 3 `scope_report` tests and all 10 `savings_lines` tests
green**. The cache pricing — this phase's headline ("no longer ignores cache token
buckets") — could be deleted with zero test failures.

### (B) The rewire's derived executor cost (Executor row) is only ever tested at `$0.00`.

Every `savings_lines` test (panels.rs:1633+) passes `executor: ModelRates::default()`
(all-zero), so the Executor row is always `$0.00` and the debit-row hide logic drops
it. The rewire's whole point — executor cost is now **derived** (non-`$0.00` when the
model is priced), replacing the hardcoded stub — is never exercised. The Test plan
explicitly required "a test with a **priced** executor `BudgetRates.executor` + a
`ScopeCosts` carrying executor tokens, asserting the Executor row now shows a
**non-`$0.00`** value (proving the stub is gone and the value is derived)."

## What should happen

The cache-pricing arithmetic and the derived-executor rendering must each be guarded
by a test that fails if the behavior regresses.

## How to fix

**(A) Cover `scope_report` cache pricing.** Either extend
`scope_report_priced_executor_and_baseline` or add a new
`scope_report_prices_executor_cache` test (mcp/src/costs.rs `mod tests`): build a
`ScopeCosts` with **non-zero** `executor_cache_read` / `executor_cache_write` and a
priced `exec_rates` (non-zero `cache_read_per_mtok` / `cache_creation_per_mtok`), and
assert `r.executor` equals the full four-term sum — pinning concretely, e.g.
`executor_cache_read = 1_000_000` at `cache_read_per_mtok = 2.0` contributes `$2.00`
and `executor_cache_write = 1_000_000` at `cache_creation_per_mtok = 8.0` contributes
`$8.00`. The test must **fail** if either cache term is removed or its rate zeroed
(re-run the review mutation: `per_m(costs.executor_cache_read, 0.0)` → the test must
go red).

**(B) Cover the derived Executor row.** Add a `savings_lines` test (panels.rs
`mod tests`) with `BudgetRates.executor` set to a **priced** `ModelRates` (non-zero
`input_per_mtok`/`output_per_mtok`) and a `ScopeCosts` (project and/or milestone)
carrying non-zero `executor_in`/`executor_out`; assert the rendered output contains
an `Executor:` row with a **non-`$0.00`** dollar value (the debit-row hide logic
keeps it because it is no longer `$0.00`). This proves the `$0.00` stub is gone and
the value is derived via `scope_report`.

## Verification

- [ ] Zeroing a `scope_report` cache term (`per_m(costs.executor_cache_read, 0.0)`)
      makes the new cache test **fail**.
- [ ] The new priced-executor `savings_lines` test asserts a non-`$0.00` Executor
      row and fails if `savings_lines` reverts to a `$0.00` executor value.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Notes for review

**The production code is correct** — the cache pricing math and the `load_data`
regression fix (real `folded_activities` passed to both scope calls, assists
restored) are right; this bounce is purely the missing test coverage. Do **not**
rewrite the production logic.

**Minor cleanup while you are here (optional but preferred):** in
`mcp/src/dashboard/mod.rs` `load_data`, the `None`-`project_id` arm still contains an
**unreachable** `costs::scope_costs(&phase_runs, &[], pid, Some(&milestone_dir))`
inside `resolve_milestone_dir(...).zip(project_id).map(...)` — since `project_id` is
`None` in that arm, `.zip(None)` yields `None` and the `.map` never runs. It compiles
and is harmless (dead), but it reads as a leftover of the `&[]` regression. Replace
that arm's `milestone_costs` with a plain `None` (the `.zip(project_id)` on `None`
already guarantees `None`), or drop the dead closure. Not a blocker on its own.
