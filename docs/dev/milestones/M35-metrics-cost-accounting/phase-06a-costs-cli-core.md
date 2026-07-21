# Phase 06a: `rexymcp costs` — the cost-report core + CLI (Baseline/Executor/Architect/Net × Session/Milestone/Project)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-05b
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add `rexymcp costs`: a CLI table reporting **Baseline / Executor / Architect /
Net** across three scopes — **Session / Milestone / Project** — built on a new
shared cost-report core (`mcp/src/costs.rs`). The core computes **executor cost
from the real phase-03 pricing** (`config.model_rates`) instead of the dashboard's
hardcoded `$0.00` stub, so the executor finally carries a real (configurable)
price. Unpriced models (AEON-7) still cost `$0.00`, but now *derived*, not stubbed.

**Split (resolved with the user 2026-07-20):** phase-06 is two phases. **06a
(this)** = the shared cost-report core + `rexymcp costs` CLI. **06b** = rewiring
the dashboard Budget panel onto this core, the `b`-key tokens⇄currency toggle,
and cache-bucket inclusion. **Data source (resolved with the user):** `costs`
mirrors the dashboard's 3-scope model — Session from the live session log,
Milestone/Project from the telemetry store.

06a is **self-contained**: it does **not** touch the dashboard (`mcp/src/dashboard/`)
— it builds a parallel authoritative core that 06b will point the dashboard at.
A small store-aggregation is duplicated from the dashboard's inline copy here and
**de-duplicated in 06b** when the dashboard is rewired.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 6 (this phase begins its `costs` clause) and design finding "executor
  cost is hardcoded $0.00 in the dashboard."
- `mcp/src/dashboard/panels.rs` — `ScopeCosts`, `BudgetRates`, and `savings_lines`
  (the Baseline/Executor/Architect/Net math this core reproduces + fixes).
- `mcp/src/dashboard/mod.rs` — `load_data`'s store aggregation (the shape 06a's
  `scope_costs` reproduces) and `sum_architect_tokens`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** unit tests go in a `#[cfg(test)] mod tests` block at
   the bottom of `mcp/src/costs.rs`; CLI-parse tests inside `main.rs`'s existing
   `mod tests`.
6. **Module registration:** add `mod costs;` to `mcp/src/main.rs` alongside the
   other `mod` lines (e.g. near `mod scorecard;`).
7. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view a
   file/line range use `read_file` (`start_line`/`end_line`), not `sed -n`/`cat` —
   `bash` refuses `sed -i` and repeated identical reads trip the governor. On a
   `patch` "0 matches"/"changed on disk", `read_file` again then re-patch.

## Current state — the pieces to reuse (all already exist)

- **Scope token holder:** `ScopeCosts { executor_in: u64, executor_out: u64,
  architect: ArchitectTokens }` (`dashboard::panels`, re-exported as
  `crate::dashboard::ScopeCosts`). `ArchitectTokens { input, cache_creation,
  cache_read, output }` (all `u64`) with `.cost(&ArchitectRates) -> f64`
  (`rexymcp_executor::store::telemetry`).
- **Rate holder:** `BudgetRates { input_per_mtok: f64, output_per_mtok: f64,
  architect: ArchitectRates }` (`crate::dashboard::BudgetRates`). Built from config
  exactly as the dashboard does (main.rs:809–814):

  ```rust
  let (i, o) = cfg.dashboard.effective_rates();          // baseline $/Mtok (in, out)
  let rates = dashboard::BudgetRates {
      input_per_mtok: i,
      output_per_mtok: o,
      architect: cfg.architect.effective_architect_rates(),
  };
  ```

- **Executor pricing (the fix):** `cfg.model_rates(model) -> ModelRates` (phase-03;
  `ModelRates` is an alias of `ArchitectRates` — four `_per_mtok` fields). Unpriced
  model ⇒ all-zero ⇒ `$0.00`.
- **Session tokens (live log):** `crate::status::{load_records, summarize}` →
  `StatusSummary { last_input_tokens: Option<u32>, last_output_tokens: Option<u32>,
  phase: Option<String>, .. }`.
- **Store reads:** `rexymcp_executor::store::telemetry::{read (phase runs),
  read_architect_activities, fold_activities}`; the store file is
  `<telemetry.dir>/phase_runs.jsonl`.
- **Store aggregation shape** (from `load_data`, mod.rs — reproduce in `costs.rs`):

  ```rust
  let exec: ScopeCosts = phase_runs.iter()
      .filter(|r| r.project_id.as_deref() == Some(pid) /* && milestone match */)
      .fold(ScopeCosts::default(), |mut c, r| {
          c.executor_in  = c.executor_in.saturating_add(r.tokens.input_tokens as u64);
          c.executor_out = c.executor_out.saturating_add(r.tokens.output_tokens as u64);
          c
      });
  ```

  and `sum_architect_tokens(&folded_activities, Some(pid), milestone_id)` filters
  folded activities by `(project_id, milestone_id)` and sums their
  `ArchitectTokens` fields (see mod.rs:44–63 for the exact fold).

- **The `$0.00` stub being replaced:** `savings_lines`' `executor_val = |_in,_out|
  "$0.00"` (panels.rs:510). 06a's core computes it for real (below). The current
  Net = `baseline − architect` (executor omitted); 06a's Net = `baseline −
  executor − architect` (executor now real; identical when executor is `$0`).

## Spec

### Task 1 — The report types (`mcp/src/costs.rs`, new)

```rust
use std::path::Path;

use serde::Serialize;

use rexymcp_executor::config::Config;
use rexymcp_executor::store::telemetry::{self, ArchitectTokens, PhaseRun};

use crate::dashboard::{BudgetRates, ScopeCosts};
use crate::status;

/// One scope's four cost lines, in dollars. `baseline`/`net` are `None` when no
/// baseline rate is configured (rendered `—`); `executor`/`architect` are always
/// present (`0.0` when unpriced).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ScopeReport {
    pub baseline: Option<f64>,
    pub executor: f64,
    pub architect: f64,
    pub net: Option<f64>,
}

/// Baseline/Executor/Architect/Net across the three scopes.
#[derive(Debug, Clone, Serialize)]
pub struct CostReport {
    pub session: ScopeReport,
    /// `None` when no active milestone could be resolved (no project runs).
    pub milestone: Option<ScopeReport>,
    pub project: ScopeReport,
    pub assists: u32,
}
```

### Task 2 — The pure per-scope computation

```rust
/// Compute one scope's dollar lines. `exec_rates` are the executor model's
/// `$/Mtok` (from `cfg.model_rates`); `baseline` carries the cloud-baseline +
/// architect rates. u64-safe (does NOT route token totals through the u32
/// `TokenBreakdown`).
pub fn scope_report(costs: &ScopeCosts, exec_rates: &telemetry::ModelRates, baseline: &BudgetRates) -> ScopeReport {
    let per_m = |t: u64, r: f64| (t as f64 / 1_000_000.0) * r;
    let no_baseline = baseline.input_per_mtok == 0.0 && baseline.output_per_mtok == 0.0;

    let executor = per_m(costs.executor_in, exec_rates.input_per_mtok)
        + per_m(costs.executor_out, exec_rates.output_per_mtok);
    let architect = costs.architect.cost(&baseline.architect);
    let baseline_cost = if no_baseline {
        None
    } else {
        Some(per_m(costs.executor_in, baseline.input_per_mtok)
            + per_m(costs.executor_out, baseline.output_per_mtok))
    };
    let net = baseline_cost.map(|b| b - executor - architect);

    ScopeReport { baseline: baseline_cost, executor, architect, net }
}
```

Pin (mirror `savings_lines`): the baseline is computed over the **executor** token
totals (what those tokens would have cost at cloud rates), `no_baseline` triggers
when **both** baseline rates are `0.0`, and `net` subtracts both debits.

### Task 3 — The store aggregation + loader

Add a store-aggregation helper (reproducing the `load_data` shape — this is the
duplication 06b removes):

```rust
/// Sum executor tokens over project runs, optionally scoped to one milestone_id.
fn scope_costs(runs: &[PhaseRun], activities: &[telemetry::ArchitectActivity],
               project_id: &str, milestone_id: Option<&str>) -> ScopeCosts { … }
```

- executor: fold `runs` filtered by `project_id` (and `milestone_id` when `Some`),
  summing `tokens.input_tokens`/`output_tokens` as `u64` (per the quoted shape).
- architect: `sum_architect_tokens`-equivalent over `activities` filtered by
  `(project_id, milestone_id)` — reproduce the mod.rs:44–63 fold (sum `input`,
  `cache_creation`, `cache_read`, `output`).

Then the public loader:

```rust
pub fn load_cost_report(config_path: &Path, repo: &Path, session: Option<&str>,
                        telemetry_path: Option<&Path>) -> Result<CostReport, String>
```

1. `Config::load_with_env(config_path)` (map err to `String`); resolve the
   telemetry file (`telemetry_path` override, else `cfg.telemetry.dir.join("phase_runs.jsonl")`,
   else `Err("telemetry disabled: …")` — same message shape as `profile_cli`).
2. Baseline `BudgetRates` from config (the main.rs:809–814 shape). Executor rates =
   `cfg.model_rates(&cfg.executor.model)`.
3. `project_id = cfg.project.id` (the loaded config's `[project] id`). If `None`,
   project/milestone scopes are `ScopeCosts::default()` (all zero) — Session still
   computes.
4. **Session** `ScopeCosts`: `status::summarize(status::load_records(repo, session)?)`
   → `{ executor_in: summary.last_input_tokens.unwrap_or(0) as u64, executor_out:
   summary.last_output_tokens.unwrap_or(0) as u64, architect: default }`
   (Session has **no** architect cost — mirrors `savings_lines`, which passes
   `ArchitectTokens::default()` for the session column). If `load_records` errors
   (no session), Session is `ScopeCosts::default()`.
5. Read `runs` (`telemetry::read`) + folded `activities`
   (`fold_activities(read_architect_activities(file))`). **Project** = `scope_costs(&runs,
   &activities, pid, None)`. **Active milestone_id** = the `milestone_id` of the
   **latest** (`max ts`) run for `pid` that has one; **Milestone** =
   `Some(scope_costs(&runs, &activities, pid, Some(mid)))` when such a milestone
   exists, else `None`.
   (This is a store-native milestone resolution — the dashboard resolves the
   milestone from the repo filesystem; 06b reconciles. Pin the store-native
   choice here.)
6. **Assists** = count of folded `activities` with `project_id == Some(pid)` and
   `activity == "assist"` (mirrors mod.rs).
7. Build `CostReport` via `scope_report` for each scope (executor rates + baseline).

### Task 4 — The table formatter

```rust
pub fn format_costs(report: &CostReport) -> String
```

- Header: `SCOPE`, `BASELINE`, `EXECUTOR`, `ARCHITECT`, `NET` (exact spacing not
  pinned).
- One row per scope: `Session`, `Milestone` (**omitted** when
  `report.milestone.is_none()`), `Project`.
- Dollar cells `${:.2}`; `baseline`/`net` render `—` when `None`. Add a trailing
  `Assists: {n}` line (mirrors the dashboard panel).
- Use a local `fmt_dollars`/`fmt_opt` helper — do **not** import dashboard
  internals (its `fmt_dollars` is a private closure).

### Task 5 — The CLI command

In `mcp/src/main.rs`:

- Add `mod costs;`.
- Add a `Commands::Costs` variant (model it on `Commands::Dashboard`, main.rs:802,
  which has `repo`/`session`/`config`):

  ```rust
  /// Report token cost (Baseline/Executor/Architect/Net) across
  /// Session / Milestone / Project.
  Costs {
      /// Path to the config file
      #[arg(long, default_value = "rexymcp.toml")]
      config: PathBuf,
      /// Repo whose session log + project telemetry to report on
      #[arg(long, default_value = ".")]
      repo: PathBuf,
      /// Session id (default: latest)
      #[arg(long)]
      session: Option<String>,
      /// Override the telemetry phase_runs.jsonl path
      #[arg(long)]
      telemetry_path: Option<PathBuf>,
      /// Emit JSON instead of a human table
      #[arg(long)]
      json: bool,
  },
  ```

- Dispatch arm: `costs::load_cost_report(&config, &repo, session.as_deref(),
  telemetry_path.as_deref())`; on `Ok`, print `serde_json::to_string_pretty` (if
  `json`) or `costs::format_costs(&report)`; on `Err`, `eprintln!` + `exit(1)` (the
  `profile`/`scorecard` arm shape).

## Acceptance criteria

- [ ] `rexymcp costs` prints a `SCOPE`/`BASELINE`/`EXECUTOR`/`ARCHITECT`/`NET`
      table with `Session` + `Project` rows (and `Milestone` when a milestone is
      active); `--json` emits the `CostReport`.
- [ ] **Executor cost is computed via `cfg.model_rates`, not hardcoded `$0.00`** —
      `grep -n '\$0\.00' mcp/src/costs.rs` returns nothing; a priced executor model
      yields a non-zero `executor`, an unpriced one yields `0.0`.
- [ ] `scope_report` computes `baseline`/`net` as `None` when both baseline rates
      are `0.0`, and `net = baseline − executor − architect` otherwise.
- [ ] Session scope carries **no** architect cost (architect column `0.0`/`$0.00`
      for Session).
- [ ] The dashboard (`mcp/src/dashboard/`) is **unchanged** by this phase.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`
      all pass.

## Test plan

In `mcp/src/costs.rs` `mod tests`:

- `scope_report_priced_executor_and_baseline` — a `ScopeCosts` with non-zero
  `executor_in/out` + architect tokens, priced `exec_rates` **and** baseline rates:
  assert `executor` = the per-Mtok sum (non-zero), `architect` = `tokens.cost(rates)`,
  `baseline = Some(_)`, `net = Some(baseline − executor − architect)`. Pin the
  arithmetic with concrete numbers (e.g. 1_000_000 input tokens at `$5/Mtok` = `$5.00`).
- `scope_report_unpriced_executor_is_zero_not_stub` — priced baseline, **unpriced**
  `exec_rates` (all `0.0`): `executor == 0.0` (proves it is *computed*, not a
  literal — a mutation hardcoding `1.0` would still be `0.0` here, so also assert a
  priced case in the test above). 
- `scope_report_no_baseline_is_none` — baseline rates `0.0`: `baseline == None` and
  `net == None` (negative pin), while `executor`/`architect` still compute.
- `format_costs_omits_milestone_when_none` — a `CostReport` with `milestone: None`
  renders no `Milestone` row but does render `Session`/`Project`; `—` shows for a
  `None` baseline. `format_costs_shows_milestone_when_some` — the mirror.
- `load_cost_report_telemetry_disabled_errors` — a config with `[telemetry] enabled
  = false` (the `profile_cli` fixture shape) → `Err` containing `telemetry disabled`.

A CLI-parse test in `main.rs` (`cli_parse_costs_*`): `costs --config rexymcp.toml`
parses with `repo == "."` default and `json == false`.

## End-to-end verification

Live against the real store + this repo's session log (read-only):

```
cargo run -p rexymcp -- costs --config rexymcp.toml --repo .
cargo run -p rexymcp -- costs --config rexymcp.toml --repo . --json
```

Confirm the table shows Session/Milestone/Project rows with `EXECUTOR` `$0.00`
(AEON-7 unpriced — but derived), `ARCHITECT` non-zero for Milestone/Project (real
architect tokens exist in our corpus), `BASELINE`/`NET` populated (this repo's
`[dashboard] saved_*` rates are set) with Session architect `$0.00`. Quote the real
output in the completion Update Log.

## Authorizations

None. No new dependencies (`serde` already used). No `docs/architecture.md` edit —
the § "Model effectiveness metrics" prose is unaffected by adding a CLI cost
surface. No `STANDARDS.md`/`WORKFLOW.md` edit. No dashboard edit (06b owns that).

## Out of scope

- **The dashboard Budget-panel rewire, the `b`-key toggle, cache-bucket
  inclusion** — all phase-06b. 06a deliberately leaves the dashboard's `$0.00`
  stub and inline aggregation in place; 06b rewires it onto this core and removes
  the duplication.
- **Cache token buckets in the `costs` executor cost** — 06a sums input+output
  only (matching today's `ScopeCosts`); 06b extends `ScopeCosts` with cache and
  fixes both surfaces.
- **The phase-05b follow-up** (convert `PhaseCostAccumulator`'s 5-tuple to a named
  struct + drop `#[allow(clippy::type_complexity)]`) — `profile.rs` is untouched
  here; that sweep lands in a later profile-touching change or phase-07.
- **`docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
