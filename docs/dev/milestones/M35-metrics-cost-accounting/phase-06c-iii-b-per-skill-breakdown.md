# Phase 06c-iii-b: Per-skill architect cost breakdown

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** in-progress
**Depends on:** phase-06c-iii-a
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Surface **where the architect (Claude Code) money goes, by skill**. The ledger
(06c-i) already buckets by `(project × session × model × skill)`, and 06c-iii-a
prices it per-model; this phase groups that by **skill** and shows it:

1. `rexymcp costs` **always appends** a per-skill architect table —
   **SKILL / TOKENS / COST / %** (of total architect cost), project-scoped, per-model
   priced, sorted by cost descending. (A transcript deep-dive found `rexymcp:dispatch`
   ≈ 49% of architect cost — this makes that visible.)
2. The dashboard Budget panel gains a **one-line top-skill hint** (the single most
   expensive skill). **Not** a full mini-panel — one line only, to keep the TUI change
   small.

**Harvest-freshness is NOT in this phase** — it was folded into 06e (the periodic
sweep keeps the ledger fresh, making a per-`costs` staleness footer redundant; sweep
*liveness* belongs with the sweep in 06e). Do not add any freshness/last-harvested
display here.

## Architecture references

Read before starting:

- `mcp/src/costs.rs` — `scope_costs` (the ledger-grouping + per-model-cost pattern to
  mirror for `skill_costs`), `CostReport` (add a field), `load_cost_report` (populate
  it), `format_costs` (append the table). `ArchitectLedger` + `ArchitectLedger::cost`
  + `ArchitectConfig::rates_for` are the pricing primitives (06c-ii).
- `mcp/src/dashboard/mod.rs` — `load_data` (already reads the ledger + takes
  `&ArchitectConfig` after 06c-iii-a) and `DashboardData` (add a field).
- `mcp/src/dashboard/render.rs` — where the Budget panel lines are assembled (add one
  line).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement:** tests in the existing `#[cfg(test)] mod tests` block of the file
   under test (`costs.rs`).
6. **Editing discipline (load-bearing — every prior 06c phase hard-failed here):** edit
   with `patch`/`patch_lines`, **never a whole-file `write_file`**. View with
   `read_file` (`start_line`/`end_line`), **never `sed -n`/`cat`, and NEVER run the same
   read (or any) command twice** — the governor hard-fails on identical repeated calls;
   this sank three prior runs. Read a region once, then act. Run `cargo check -p rexymcp`
   after each file.

## Current state

**`scope_costs`** (costs.rs, from 06c-iii-a) — the ledger-grouping + per-model pricing
pattern to mirror. Its architect branch:

```rust
        let mut cost = 0.0_f64;
        for l in ledgers.iter().filter(|l| l.project_id.as_deref() == Some(project_id)) {
            // ... sum tokens ...
            if let Some((inp, out)) = architect.rates_for(&l.model) {
                cost += l.cost(inp, out);
            }
        }
```

**`CostReport`** (costs.rs) — `{ session, milestone: Option, project, assists }`.
**`format_costs`** (costs.rs) — builds the scope table + an `Assists:` line, returns a
`String`. The `fmt_dollars` helper is defined there.

**`ArchitectLedger`** fields: `model: String`, `skill: String`, `tokens: ArchitectTokens`
(`input`/`cache_creation`/`cache_read`/`output`, all `u64`).

**`DashboardData`** (dashboard/mod.rs) + `load_data` — after 06c-iii-a, `load_data` reads
the ledger and takes `&ArchitectConfig`. `render.rs` assembles the Budget panel from
`savings_lines(...)`.

## Spec

### Task 1 — `SkillCost` + `skill_costs` (costs.rs)

```rust
/// One skill's architect spend: total tokens (all four classes) and per-model USD cost.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SkillCost {
    pub skill: String,
    pub tokens: u64,
    pub cost: f64,
}

/// Per-skill architect cost for a project, from the ledger, priced per-model.
/// Sorted by `cost` descending (ties broken by `skill` for determinism).
pub(crate) fn skill_costs(
    ledgers: &[telemetry::ArchitectLedger],
    architect: &rexymcp_executor::config::ArchitectConfig,
    project_id: &str,
) -> Vec<SkillCost> {
    use std::collections::HashMap;
    let mut acc: HashMap<String, (u64, f64)> = HashMap::new();
    for l in ledgers.iter().filter(|l| l.project_id.as_deref() == Some(project_id)) {
        let toks = l.tokens.input
            .saturating_add(l.tokens.cache_creation)
            .saturating_add(l.tokens.cache_read)
            .saturating_add(l.tokens.output);
        let cost = architect.rates_for(&l.model).map_or(0.0, |(i, o)| l.cost(i, o));
        let e = acc.entry(l.skill.clone()).or_insert((0, 0.0));
        e.0 = e.0.saturating_add(toks);
        e.1 += cost;
    }
    let mut out: Vec<SkillCost> = acc
        .into_iter()
        .map(|(skill, (tokens, cost))| SkillCost { skill, tokens, cost })
        .collect();
    out.sort_by(|a, b| b.cost.total_cmp(&a.cost).then_with(|| a.skill.cmp(&b.skill)));
    out
}
```

### Task 2 — `CostReport.by_skill` (costs.rs)

Add `pub by_skill: Vec<SkillCost>` to `CostReport`. In `load_cost_report`, after the
ledger is read (06c-iii-a added `let ledgers = ...`), populate it for the project scope:
`let by_skill = if let Some(pid) = project_id { skill_costs(&ledgers, &cfg.architect, pid) } else { Vec::new() };` and set it on both the `Some(pid)` and `None` `CostReport` returns (empty in the `None` arm).

### Task 3 — append the by-skill table in `format_costs` (costs.rs)

After the `Assists:` line, if `!report.by_skill.is_empty()`, append a blank line then a
table. **Total** for the `%` column = the sum of `by_skill[*].cost` (equals the project
architect cost). Pin the columns and behaviour, not exact spacing:

- A header row naming SKILL, TOKENS, COST, and % (e.g. `"By skill (architect)"` title).
- One row per `SkillCost`: skill name; token count (a compact form is fine, e.g.
  `1.5M`/`200k`/raw — reuse or mirror any existing token formatter); `fmt_dollars(cost)`;
  and the percent `cost / total * 100` to one decimal (`0.0%` when total is 0 — avoid
  divide-by-zero).
- Rows already sorted by cost desc (from `skill_costs`).

### Task 4 — dashboard one-line top-skill hint (mod.rs + render.rs)

- **`DashboardData`**: add `pub top_skill: Option<SkillCost>`.
- **`load_data`**: compute `top_skill` = `skill_costs(&ledgers, architect, pid).into_iter().next()` (the highest-cost skill) for the `Some(pid)` arm; `None` in the no-project arm and the error arms.
- **`render.rs`**: after the Budget panel's `savings_lines`, push **one** line when
  `data.top_skill` is `Some` and its `cost > 0.0`, e.g.
  `format!("  Top skill: {} ${:.2}", ts.skill, ts.cost)` (exact text not pinned; it
  must name the skill and its cost). No new panel, no layout/constraint changes.

## Acceptance criteria

- [ ] `rexymcp costs` output includes a per-skill architect section: one row per skill
      with SKILL, TOKENS, COST, and %-of-architect, **sorted by cost descending**.
- [ ] `skill_costs` prices each ledger record at **its own model's** rate (per-model),
      groups by skill, and sums tokens across all four classes.
- [ ] The percent column is `cost / total_architect_cost * 100`, and is `0.0%` (no
      panic/NaN) when the total is `0`.
- [ ] The dashboard Budget panel shows a **single** top-skill line (highest-cost skill)
      when architect data exists; no new panel or layout change.
- [ ] No harvest-freshness / last-harvested display is added (that is 06e).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] `profile`, `scope_report`/`scope_costs` behaviour, and `executor/src/**` are
      untouched.

## Test plan

In `mcp/src/costs.rs` `mod tests` (reuse the `ledger(model)` helper 06c-iii-a added, or
build records inline; give records distinct `skill` values):

- **`skill_costs_groups_and_prices_per_model`** — ledgers: two `dispatch` records
  (opus + sonnet-5) and one `review` record (opus); assert the returned Vec has the
  `dispatch` entry first (higher combined cost) with `cost ==` opus+sonnet dispatch sum
  and the `review` entry second. **Mutation-sensitive:** fails if grouping ignores skill
  or prices all at one model.
- **`skill_costs_sorted_by_cost_desc`** — records where a later-alphabetical skill has
  the higher cost; assert it sorts first (by cost, not name).
- **`format_costs_appends_by_skill_percent`** — a `CostReport` with a `by_skill` of two
  skills whose costs are e.g. `$30` and `$10`; assert `format_costs` output contains both
  skills, `$30.00`/`$10.00`, and the percents `75.0%` / `25.0%`.
- **`skill_costs_empty_is_empty`** — no ledgers → empty Vec (and `format_costs` omits the
  section; the `%` path does not divide by zero).

All tests hermetic (pure functions; no TempDir needed).

## End-to-end verification

Run `rexymcp costs` against the **real** telemetry store and quote the output in the
completion Update Log: confirm the per-skill section lists the project's architect skills
(`dispatch`/`review`/`architect`/`escalate`/…/`other`) with tokens, dollar cost, and
percents that **sum to ~100%**, sorted by cost (dispatch expected near the top). Do not
hand-edit the store.

## Authorizations

- Editing `mcp/src/costs.rs`, `mcp/src/dashboard/mod.rs`, and `mcp/src/dashboard/render.rs`
  is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Harvest freshness / last-harvested / sweep liveness** — 06e owns it. Add nothing here.
- **A full dashboard per-skill panel** — one line only (Task 4). No new `Constraint`/
  layout, no panel.
- **`profile`** — untouched (executor-only; ledger has no phase key).
- **Per-skill at session/milestone scope** — project scope only (architect is only
  attributable at project scope, per 06c-iii-a).
- **Changing `scope_report`/`scope_costs`** — this phase only *adds* `skill_costs`; the
  scope-table path is unchanged.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-22 04:15 (started)

**Started by:** executor (06c-iii-b)

**Action:** Implementing per-skill architect cost breakdown (SkillCost struct, skill_costs function, CostReport.by_skill, format_costs table, dashboard top-skill hint).
