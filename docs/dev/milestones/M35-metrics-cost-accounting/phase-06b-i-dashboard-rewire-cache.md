# Phase 06b-i: Dashboard Budget panel → `costs` core + cache-bucket inclusion (de-dup the copied aggregation)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** in-progress (resume after a `claude_stop` stall — see the Notes-for-executor punch-list at the Update Log)
**Depends on:** phase-06a
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=refactor, size=m

## Goal

Point the dashboard Budget panel's savings block at the phase-06a cost core so
there is **one** cost computation, and stop ignoring cache token buckets:

1. **Rewire `savings_lines`** to compute Baseline/Executor/Architect/Net via
   `costs::scope_report` — deleting its inline cost math **and the hardcoded
   `$0.00` executor stub**. Executor cost is now *derived* (unpriced ⇒ `$0.00`).
2. **Cache buckets:** extend `ScopeCosts` with the executor cache-token totals and
   include them in `scope_report`'s executor cost (the milestone theme: cache
   `TokenBreakdown` buckets are recorded but ignored in cost).
3. **De-dup:** point the dashboard's `load_data` at `costs::scope_costs` /
   `costs::sum_architect_tokens` (made `pub(crate)` in 06a's module), deleting the
   dashboard's copied inline aggregation.

**Split (resolved with the user 2026-07-20):** phase-06b is two phases. **06b-i
(this)** is the pure/testable rewire + cache + de-dup. **06b-ii** is the interactive
`b`-key tokens⇄currency toggle (isolated so a toggle bug can't mask a rewire
regression).

**Behavior-preserving where it counts:** for an unpriced executor (AEON-7, our
config) the savings block renders **identically** — executor was `$0.00` (stub) and
is now `$0.00` (derived). The existing `savings_lines` tests are the regression net.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 6 ("the dashboard Budget panel renders from the same core, no longer
  ignores cache token buckets").
- `mcp/src/costs.rs` — the 06a core (`scope_report`, `scope_costs`,
  `sum_architect_tokens`, `ScopeReport`) this phase renders through and reuses.
- `mcp/src/dashboard/panels.rs` — `ScopeCosts`, `BudgetRates`, `savings_lines`
  (extended + rewired here).
- `mcp/src/dashboard/mod.rs` — `load_data` (de-duplicated onto `costs::scope_costs`).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Cascade discipline (green-at-every-step):** this phase adds fields to
   `ScopeCosts` and `BudgetRates`. Both already `#[derive(Default)]`. Add the new
   fields, then append `..Default::default()` to the struct literals that break
   (test literals, `main.rs`), so the crate stays compiling between edits. Do the
   `costs.rs` core changes first (they compile standalone), then the dashboard.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view a
   file/line range use `read_file` (`start_line`/`end_line`), not `sed -n`/`cat` —
   `bash` refuses `sed -i` and repeated identical reads trip the governor. On a
   `patch` "0 matches"/"changed on disk", `read_file` again then re-patch.

## Current state

- **`ScopeCosts`** (panels.rs:15) — `{ executor_in: u64, executor_out: u64,
  architect: ArchitectTokens }`, `#[derive(Debug, Clone, Copy, Default, PartialEq)]`.
  **Missing the executor cache buckets.**
- **`BudgetRates`** (panels.rs:23) — `{ input_per_mtok: f64, output_per_mtok: f64,
  architect: ArchitectRates }`, `#[derive(Debug, Clone, Copy, Default)]`. **Carries
  baseline + architect rates but not the executor model's rates** (which
  `scope_report` needs).
- **`costs::scope_report`** (costs.rs) — computes executor cost from
  `executor_in`/`executor_out` only (**no cache**); already handles baseline/
  architect/net. It is `pub`.
- **`costs::scope_costs` / `costs::sum_architect_tokens`** (costs.rs) — currently
  **private** (`fn`). Sum executor `input`/`output` tokens (no cache) + architect
  tokens. Must be `pub(crate)` for de-dup.
- **`savings_lines`** (panels.rs:480) — takes `(summary, rates: BudgetRates,
  milestone_costs: Option<ScopeCosts>, project_costs: ScopeCosts,
  project_escalation_count: u32)`. Builds the Session tokens from
  `summary.last_input_tokens`/`last_output_tokens` and computes Baseline/Executor/
  Architect/Net with **inline closures** — including `executor_val = |_,_| "$0.00"`
  (the stub) and `net_val = baseline − architect` (executor omitted). Renders a
  header + Baseline/Executor/Architect/Net rows (`debit_row` hides all-`$0.00`
  rows) + `Assists:` line, with a 2-scope vs 3-scope layout (`has_milestone`).
- **`load_data`** (mod.rs) — builds `project_costs` / `milestone_costs` `ScopeCosts`
  with **inline `.fold`s** over `phase_runs` (executor `input`/`output` only) plus a
  **private `sum_architect_tokens`** (mod.rs:44). This is the duplicate 06a copied.
- **Field names to use:** `TokenBreakdown` cache fields are `cache_read_tokens` /
  `cache_write_tokens` (both `u32`). Executor rates (`ModelRates` = `ArchitectRates`)
  cache fields are `cache_read_per_mtok` / `cache_creation_per_mtok`. Note the
  mapping: **cache-WRITE tokens are priced at the cache-CREATION rate** (see
  `ArchitectTokens::cost`, telemetry.rs:452 — `cache_creation` ↔ `cache_write`).

## Spec

### Task 1 — Extend `ScopeCosts` (panels.rs) with executor cache buckets

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ScopeCosts {
    pub executor_in: u64,
    pub executor_out: u64,
    pub executor_cache_read: u64,
    pub executor_cache_write: u64,
    pub architect: ArchitectTokens,
}
```

### Task 2 — Extend `BudgetRates` (panels.rs) with the executor model's rates

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub architect: ArchitectRates,
    /// The executor model's $/Mtok (from `cfg.model_rates`), for the Executor row.
    pub executor: rexymcp_executor::store::telemetry::ModelRates,
}
```

### Task 3 — Extend the core to include cache (`costs.rs`)

- **`scope_report`** — add the two executor cache terms to the executor cost
  (baseline stays input+output — there is no configured cache baseline rate):

  ```rust
  let executor = per_m(costs.executor_in, exec_rates.input_per_mtok)
      + per_m(costs.executor_out, exec_rates.output_per_mtok)
      + per_m(costs.executor_cache_read, exec_rates.cache_read_per_mtok)
      + per_m(costs.executor_cache_write, exec_rates.cache_creation_per_mtok);
  ```

- **`scope_costs`** — sum the cache buckets too, and make it **`pub(crate)`**:

  ```rust
  .fold(ScopeCosts::default(), |mut c, r| {
      c.executor_in         = c.executor_in.saturating_add(r.tokens.input_tokens as u64);
      c.executor_out        = c.executor_out.saturating_add(r.tokens.output_tokens as u64);
      c.executor_cache_read = c.executor_cache_read.saturating_add(r.tokens.cache_read_tokens as u64);
      c.executor_cache_write = c.executor_cache_write.saturating_add(r.tokens.cache_write_tokens as u64);
      c
  })
  ```

- Make **`sum_architect_tokens`** `pub(crate)` (no body change).

### Task 4 — Rewire `savings_lines` (panels.rs) onto `costs::scope_report`

Replace the inline value closures with per-scope `costs::scope_report` calls. Build
the Session `ScopeCosts` from the summary (cache = 0 — `StatusSummary` carries no
cache tokens; leave `executor_cache_read`/`write` at their `Default` 0):

```rust
let session_costs = ScopeCosts {
    executor_in: sess_in,
    executor_out: sess_out,
    ..Default::default()   // cache = 0, architect = default (Session has no architect)
};
let sess = crate::costs::scope_report(&session_costs, &rates.executor, &rates);
let mile = milestone_costs.map(|c| crate::costs::scope_report(&c, &rates.executor, &rates));
let proj = crate::costs::scope_report(&project_costs, &rates.executor, &rates);
```

Then render the same **Baseline / Executor / Architect / Net** rows (+ `Assists:`)
from the `ScopeReport` values instead of the deleted closures:
- Baseline / Net cells: `Some(v) → "$v.2f"`, `None → "—"` (a local `fmt_opt`).
- Executor / Architect cells: `"$v.2f"` (a local `fmt_dollars`).
- Keep the existing `has_milestone` 2-vs-3-scope layout, the header, `make_row`,
  the `debit_row` "hide all-`$0.00`" behavior (compare the rendered `$0.00`
  strings, unchanged), the `paren()` on debit rows, and the `Assists:` line.
- **Delete** the inline `cost` / `baseline_val` / `executor_val` (the `$0.00` stub)
  / `architect_val` / `net_val` closures — the values now come from `ScopeReport`.

`savings_lines`' signature is unchanged (still `summary, rates, milestone_costs,
project_costs, project_escalation_count`) — `rates` now carries `.executor`, so no
caller-signature change beyond `BudgetRates` gaining a field.

### Task 5 — De-dup `load_data` (mod.rs) onto `costs::scope_costs`

- Replace the inline `project_costs` `.fold` with
  `costs::scope_costs(&phase_runs, &folded_activities, pid, None)`.
- Replace the inline `milestone_costs` `.fold` with
  `costs::scope_costs(&phase_runs, &folded_activities, pid, Some(&milestone_dir))`
  (keep the dashboard's existing `resolve_milestone_dir(...).zip(project_id)`
  milestone resolution — only the *aggregation* is de-duplicated, not the
  milestone-dir lookup).
- **Delete** the dashboard's private `fn sum_architect_tokens` (mod.rs:44) — it is
  now `costs::sum_architect_tokens`, called inside `costs::scope_costs`.

### Task 6 — Wire the executor rate into the dashboard's `BudgetRates`

In `mcp/src/main.rs`, the `Commands::Dashboard` arm (main.rs:810) builds
`BudgetRates`. Add the executor rate:

```rust
let rates = dashboard::BudgetRates {
    input_per_mtok: i,
    output_per_mtok: o,
    architect: cfg.architect.effective_architect_rates(),
    executor: cfg.model_rates(&cfg.executor.model),
};
```

## Acceptance criteria

- [ ] `savings_lines` computes via `costs::scope_report`; the string literal
      `"$0.00"` no longer appears as an executor **stub** in `panels.rs`
      (`grep -n '|_in.*|_out.*"\$0\.00"' mcp/src/dashboard/panels.rs` returns
      nothing; a general `$0.00` may still appear in the `debit_row` hide-check).
- [ ] `ScopeCosts` carries `executor_cache_read`/`executor_cache_write`;
      `scope_costs` sums them from `tokens.cache_read_tokens`/`cache_write_tokens`;
      `scope_report`'s executor cost includes the two cache terms
      (cache-write priced at `cache_creation_per_mtok`).
- [ ] `load_data` calls `costs::scope_costs` for project + milestone scopes; the
      dashboard's private `sum_architect_tokens` is **deleted**
      (`grep -n "fn sum_architect_tokens" mcp/src/dashboard/mod.rs` returns nothing;
      it now lives only in `costs.rs`).
- [ ] For an **unpriced** executor, the savings block renders unchanged (the
      existing `savings_lines` tests pass, adjusted only for the new struct fields).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`
      all pass.

## Test plan

- **`costs.rs`:** extend the existing `scope_report_priced_executor_and_baseline`
  test (or add `scope_report_includes_executor_cache`) with **non-zero**
  `executor_cache_read`/`executor_cache_write` and a priced `exec_rates`; assert the
  executor cost includes `cache_read × cache_read_per_mtok + cache_write ×
  cache_creation_per_mtok` (concrete numbers). Extend/add a `scope_costs` test
  asserting the cache buckets are summed from `tokens.cache_read_tokens`/
  `cache_write_tokens`. (The `scope_costs_none_sums_all_milestones` test from 06a
  stays green — extend its `TokenBreakdown` literals with cache if you assert on
  cache, else leave.)
- **`panels.rs`:** the existing `savings_lines` tests must pass with the struct
  fields added (`..Default::default()` on `ScopeCosts` literals;
  `executor: ModelRates::default()` on `BudgetRates` literals). Their assertions
  are behavior-preserving for the unpriced executor (executor `$0.00` → still
  `$0.00`). Add one test with a **priced** executor `BudgetRates.executor` +
  a `ScopeCosts` carrying executor tokens, asserting the Executor row now shows a
  **non-`$0.00`** value (proving the stub is gone and the value is derived).
- **`mod.rs`:** if `load_data` has tests, they must still pass (the de-dup is
  behavior-preserving — `costs::scope_costs` reproduces the inline fold plus cache).

## End-to-end verification

Live dashboard cost consistency — `rexymcp costs` and the dashboard Budget panel now
share one core, so their numbers must agree. Run (read-only):

```
cargo run -p rexymcp -- costs --config rexymcp.toml --repo .
```

and confirm the `costs` Project/Milestone Baseline/Net still match the pre-phase
values (the core is unchanged for the CLI; this phase only makes the dashboard use
it). A full TUI render is not hermetically checkable here — the `savings_lines` unit
tests are the behavioral surface; state that in the completion Update Log. (Do not
claim a live TUI screenshot.)

## Authorizations

None. No new dependencies. No `docs/architecture.md` edit — the § "Model
effectiveness metrics" prose is unaffected. No `STANDARDS.md`/`WORKFLOW.md` edit.
Editing `mcp/src/costs.rs`, `mcp/src/dashboard/panels.rs`, `mcp/src/dashboard/mod.rs`,
and `mcp/src/main.rs` is in scope for this phase.

## Out of scope

- **The `b`-key tokens⇄currency toggle** — phase-06b-ii (the interactive
  `event_loop` piece).
- **A cache *baseline* rate** — baseline stays input+output (no cache baseline rate
  is configured); only the executor cost gains cache. Do not invent a baseline
  cache rate.
- **The phase-05b `LatestRun`-struct / `#[allow(clippy::type_complexity)]` cleanup**
  — `profile.rs` is untouched here.
- **`docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-20 (escalation)

**Chosen lever:** resume (`continue_phase`)
**Rationale:** the first dispatch did most of the work (panels.rs rewire ✓, cache
summing ✓, `pub(crate)` ✓, main.rs rate ✓) then **stalled** (~100 no-op
`awaiting_model` turns, frozen diff) and was `claude_stop`ped at turn 363. The
partial work is good but has a few concrete defects + litter; resume from the dirty
tree with the punch-list below. Two defects trace to spec gaps (activities threading
+ assists preservation in `load_data`), so a resume can bridge them.

### 🛑 Notes for executor — 2026-07-20 (RESUME PUNCH-LIST — finish these exact items, then gates)

The tree is dirty with your partial work. **Do NOT restart from scratch** — apply
these fixes on top of what's there, then run all four gates until green.

1. **DELETE the two scratch files** you created —
   `mcp/src/dashboard/mod_new.rs` and `mcp/src/dashboard/mod_new2.rs`. They are
   leftover rewrite scratch; the real code lives in `mod.rs`. `git status` / a
   directory read must show them gone. (`use delete_file`.)

2. **`mcp/src/dashboard/mod.rs` — you introduced a regression: fix it.** Your
   de-dup calls `crate::costs::scope_costs(&phase_runs, **&[]**, pid, …)` with an
   **empty** activities slice, so the dashboard's **Architect cost is now always
   $0**, and you hardcoded `project_escalation_count: 0` (dropping the Assists
   count). **Restore both:**
   - Re-add the folded activities read that the original had (do not pass `&[]`):
     ```rust
     let folded_activities = match (project_id, telemetry_dir) {
         (Some(_), Some(dir)) => rexymcp_executor::store::telemetry::fold_activities(
             rexymcp_executor::store::telemetry::read_architect_activities(
                 &dir.join("phase_runs.jsonl"),
             ).unwrap_or_default(),
         ),
         _ => Vec::new(),
     };
     ```
   - Pass `&folded_activities` (NOT `&[]`) to **both** `costs::scope_costs` calls
     (project scope with `None`, milestone scope with `Some(&milestone_dir)`).
   - Restore `project_escalation_count`: `folded_activities.iter().filter(|a|
     a.project_id.as_deref() == Some(pid) && a.activity == "assist").count() as u32`
     (its original computation) and pass it in **both** `DashboardData { … }` arms —
     not a hardcoded `0`.
   - Also: you left `}pub fn run_dashboard(` jammed on one line and deleted the
     `/// Run the dashboard event loop.` doc comment — restore the newline + that
     doc comment.

3. **`mcp/src/costs.rs` — Task 3 was NOT done.** `scope_report` still prices only
   input+output; add the two executor **cache** terms so summed cache is actually
   priced:
   ```rust
   let executor = per_m(costs.executor_in, exec_rates.input_per_mtok)
       + per_m(costs.executor_out, exec_rates.output_per_mtok)
       + per_m(costs.executor_cache_read, exec_rates.cache_read_per_mtok)
       + per_m(costs.executor_cache_write, exec_rates.cache_creation_per_mtok);
   ```
   (baseline stays input+output only — do not add a cache baseline term.)

4. **Test literals.** Every `ScopeCosts { … }` and `BudgetRates { … }` literal
   (in `panels.rs` tests ~line 1768+, `costs.rs` tests, anywhere) must set the new
   fields or use `..Default::default()` / `executor: ModelRates::default()`. The
   crate must compile.

5. **Add the cache tests** the Test plan names: a `scope_report` case with non-zero
   `executor_cache_read`/`executor_cache_write` + priced `exec_rates` asserting the
   cache is priced (cache-write at `cache_creation_per_mtok`), and a priced-executor
   `savings_lines` test asserting the Executor row is **non-`$0.00`**.

6. **Gates:** `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets
   --all-features -- -D warnings`, `cargo test` all green; `grep -rn "mod_new"
   mcp/src` returns nothing.
