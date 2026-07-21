# Bug 1 on phase-06a: `costs` Project scope under-reports (milestone `None` filter is wrong) + `scope_costs` untested

**Severity:** major
**Status:** open
**Filed:** 2026-07-20

## What's wrong

Live `rexymcp costs --config rexymcp.toml --repo .` over the real store:

```
SCOPE         BASELINE  EXECUTOR ARCHITECT       NET
Session         $28.84     $0.00     $0.00    $28.84
Milestone      $413.19     $0.00     $0.00   $413.19
Project          $0.00     $0.00     $0.00     $0.00
```

**Project ($0.00) < Milestone ($413.19) is impossible.** The Milestone scope is a
*subset* of the Project scope (same project, one milestone), so Project cost must
be **≥** Milestone cost. Project reports ~$0 because the aggregation filter is
wrong.

`scope_costs` (mcp/src/costs.rs) filters:

```rust
.filter(|r| {
    r.project_id.as_deref() == Some(project_id) && r.milestone_id.as_deref() == milestone_id
})
```

When called for the **Project** scope with `milestone_id: None`, this requires
`r.milestone_id.as_deref() == None` — i.e. it matches **only runs that have no
milestone_id**. The store has 2 such runs (out of hundreds); every real M35/M-* run
carries a `milestone_id`, so the Project scope sums almost nothing. `None` was
meant to mean *"no milestone constraint — all project runs"* (phase doc Task 3:
"filtered by `project_id` **(and `milestone_id` when `Some`)**"), not *"milestone
must be None."*

The **dashboard's** `load_data` (mod.rs), which 06a mirrors, filters the project
scope by `r.project_id.as_deref() == Some(pid)` **only** — no milestone condition.
06a's unified `scope_costs` regressed that by making the milestone equality
unconditional.

The same `None`-means-`None` mistake is in `sum_architect_tokens` (copied into
costs.rs): for the Project scope it requires `a.milestone_id.as_deref() == None`,
so project-scope architect tokens under-count too.

**Why it shipped green:** `scope_costs` (and the `milestone_id` filtering it does)
has **no unit test**. `scope_report` is tested in isolation with hand-built
`ScopeCosts`, and `load_cost_report`'s only test hits the telemetry-disabled error
path. The buggy aggregation filter is entirely uncovered.

## What should happen

- Project scope aggregates **all** runs for the project regardless of milestone;
  Milestone scope aggregates only the runs of the active milestone. So
  `project.executor_in >= milestone.executor_in` always holds (superset).
- `None` for the `milestone_id` parameter means "no milestone constraint"; `Some(mid)`
  means "only that milestone."

## How to fix

**(1) `scope_costs` run filter** (mcp/src/costs.rs) — make the milestone condition
conditional on `Some`:

```rust
.filter(|r| {
    r.project_id.as_deref() == Some(project_id)
        && (milestone_id.is_none() || r.milestone_id.as_deref() == milestone_id)
})
```

**(2) `sum_architect_tokens` activity filter** (mcp/src/costs.rs) — same shape:

```rust
.filter(|a| {
    a.project_id.as_deref() == project_id
        && (milestone_id.is_none() || a.milestone_id.as_deref() == milestone_id)
})
```

(This intentionally makes 06a's core *correct*, diverging from the verbatim-copied
dashboard `sum_architect_tokens`; 06b reconciles the dashboard onto this core.)

**(3) Add a `scope_costs` test** that would fail under the bug. Build phase runs for
one `project_id` spanning **two** milestones (e.g. `mA`, `mB`) with distinct
non-zero `tokens.input_tokens`/`output_tokens`, plus one run for a *different*
project:

- `scope_costs(runs, &[], pid, None)` sums **both** milestones' tokens for `pid`
  and **excludes** the other project — assert `executor_in` equals the `mA + mB`
  sum (non-zero).
- `scope_costs(runs, &[], pid, Some("mA"))` sums **only** `mA` — assert
  `executor_in` equals just `mA`'s tokens, and `< the None total`.
- (Pin the superset: the `None` total ≥ the `Some(mA)` total.)

This test must **fail** under the current `r.milestone_id.as_deref() ==
milestone_id` filter (where `None` would yield 0 for milestone-tagged runs).

## Verification

- [ ] `rexymcp costs --config rexymcp.toml --repo .` shows `Project` ≥ `Milestone`
      for `BASELINE`/`NET` (Project no longer $0 while Milestone is $413).
- [ ] The new `scope_costs` test passes and fails under the unconditional-equality
      filter.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Notes for review

The `read_architect_activities(...).unwrap_or_default()` at costs.rs:166 is
**fine** — it mirrors the dashboard's best-effort activities read (missing/partial
activities → architect $0, a reasonable degradation); do not change it. The
architect column reading `$0.00` for Milestone/Project in the live run is expected
if the corpus's `ArchitectActivity` records don't carry matching
project/milestone ids — this bug is specifically about the **executor/baseline**
Project under-count; do not chase the architect zero beyond applying fix (2).
