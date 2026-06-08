# Phase 08c: Aggregate context-efficiency into the model × tag scorecard

**Milestone:** M10 — Context optimization
**Status:** todo
**Depends on:** phase-08a (`PhaseRun.context_efficiency` capture — done),
phase-08b (per-run `rexymcp runs` columns — done)
**Estimated diff:** ~70 lines (incl. tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Make M10's context-efficiency signal **comparable across runs** in the
**model × tag** scorecard (the `ScorecardRow` returned by the `model_scorecard`
MCP tool). Add two aggregate fields — **`peak_context_pct_mean`** (mean peak
context-window utilization) and **`tokens_reclaimed_mean`** (mean total tokens
reclaimed by all four M10 levers) — computed over the runs in each (model, tag)
bucket that actually carry context telemetry.

Phase-08b surfaced a *single run's* efficiency in `rexymcp runs`. This phase
rolls it up so a user can compare, e.g., "Qwen on `rust` averages 71% peak
context and reclaims ~9k tokens/run" against another model. It is deliberately
scoped to the **model × tag** scorecard only (`ScorecardRow` + its `aggregate`
function + its `Accumulator`, all in `mcp/src/scorecard.rs`). The **model ×
settings** scorecard (`SettingsScorecardRow` + the `scorecard` CLI renderer) is
the sibling phase **08d** — split out because it carries three struct-literal
sites across two files, whereas this phase touches exactly **one** struct
literal in **one** file. Keeping them apart keeps each dispatch single-concern.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `ScorecardRow` (model × tag) matrix is the cross-run comparison substrate;
  the `model_scorecard` MCP tool serves it.
- `docs/dev/milestones/M10-context-optimization/README.md` § "Phases" (row 08c).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The field this phase reads (already exists — do not change it)

`executor/src/store/telemetry.rs` defines `ContextEfficiency`, nested on every
`PhaseRun` as `pub context_efficiency: ContextEfficiency` (added in 08a). Its
fields, verbatim:

```rust
pub struct ContextEfficiency {
    /// Highest `context_pct` observed across the run's per-turn `Metrics`
    /// events; `0.0` if none were emitted. This is a FRACTION in [0.0, 1.0]
    /// (e.g. 0.68 == 68% of the context window), not a percentage.
    pub peak_context_pct: f64,
    pub compaction_count: usize,
    pub compaction_tokens_reclaimed: usize,
    pub output_filtered_tokens: usize,
    pub read_evicted_tokens: usize,
    pub read_deduped_tokens: usize,
}
```

`peak_context_pct` is a **fraction** in `[0.0, 1.0]`. "Total reclaimed" for a run
is the **sum of all four** `*_tokens` fields (the three per-lever sources **plus**
compaction) — the same sum phase-08b's `RECLAIMED` column uses.

### The struct, accumulator, and aggregator this phase modifies

**All changes are in `mcp/src/scorecard.rs`.** This file already holds the exact
idiom you will mirror: an **optional mean over the subset of runs where a value
is present** (`length_finish_rate_mean`). Quoting the three pieces of that idiom
so you can pattern-match them for the two new fields:

1. **The output field** (`ScorecardRow`, around `scorecard.rs:173`) — note the
   sibling `Option<f64>` mean already there:

   ```rust
   pub struct ScorecardRow {
       pub model: String,
       pub tag: String,
       pub n_runs: usize,
       // ... existing always-present means ...
       /// Mean of `bounces_to_approval` over runs where it is `Some`.
       /// `None` when no such runs.
       pub bounces_to_approval_mean: Option<f64>,
   }
   ```

2. **The accumulator** (`Accumulator`, around `scorecard.rs:213`) — derives
   `Default`, so **adding fields here is purely additive** (no literal to update;
   it is only ever built via `or_default()`):

   ```rust
   #[derive(Debug, Default)]
   struct Accumulator {
       n: usize,
       // ... existing sums ...
       bounces_sum: f64,
       bounces_n: usize,
   }
   ```

3. **The conditional accumulation** inside the `for tag in &run.tags` loop of
   `aggregate` (around `scorecard.rs:276`) — the "only count present values"
   pattern, with a paired `_n` counter:

   ```rust
   if let Some(b) = run.bounces_to_approval {
       acc.bounces_sum += b as f64;
       acc.bounces_n += 1;
   }
   ```

4. **The constructor** — the **single `ScorecardRow { ... }` literal** in the
   whole codebase, in the `filter_map` of `aggregate` (around `scorecard.rs:292`)
   — emits the `Option` mean, guarding the divide on the `_n` counter:

   ```rust
   bounces_to_approval_mean: if acc.bounces_n > 0 {
       Some(acc.bounces_sum / acc.bounces_n as f64)
   } else {
       None
   },
   ```

There is **exactly one** `ScorecardRow` struct literal (the constructor above).
`grep -n 'ScorecardRow {' mcp/src/scorecard.rs` returns only the struct
definition and that one constructor. The `model_scorecard` MCP tool
(`mcp/src/server.rs`) builds a `ModelScorecardOutput { rows, ... }` wrapper, **not**
a `ScorecardRow` literal, so it needs no change — the new fields serialize
through automatically (`ScorecardRow` derives `Serialize`).

**Do not touch** `aggregate_by_settings` / `SettingsScorecardRow` /
`SettingsAccumulator` (they are phase-08d) or `mcp/src/scorecard_cli.rs` or
`mcp/src/server.rs`.

## Spec

Numbered tasks in execution order. **All in `mcp/src/scorecard.rs`.**

### The "context-measured" predicate (pins the behavior of both fields)

A run **carries context telemetry** iff `run.context_efficiency.peak_context_pct
> 0.0`. (Every run since the per-turn `Metrics` emit — phase-06a — has a nonzero
peak; legacy/pre-08a runs deserialize `context_efficiency` to all-zeros via
`#[serde(default)]` and so are excluded.) Both new means are computed over **only
the context-measured runs** in each bucket, sharing **one** `_n` counter. This
mirrors the `length_finish_rate_mean` idiom (mean over the present subset), and
matches phase-08b's per-run sentinel (`peak_context_pct == 0.0` → "—").

**Boundary case to preserve (pin a test on it):** a run that *is* context-measured
(`peak_context_pct > 0.0`) but whose four reclaim sources sum to `0` is a real
"measured, reclaimed nothing" data point — it **contributes `0.0`** to
`tokens_reclaimed_mean` (pulling the mean down) and its peak to
`peak_context_pct_mean`. It is **not** excluded. Only `peak_context_pct == 0.0`
excludes a run. Do not couple "reclaimed is zero" to "unmeasured."

### 1. Add two fields to `ScorecardRow`

After the existing `Option<f64>` means, add:

```rust
/// Mean peak context-window utilization (a FRACTION in [0.0, 1.0]) over the
/// runs in this bucket that carry context telemetry (`peak_context_pct >
/// 0.0`). `None` when no run in the bucket is context-measured.
pub peak_context_pct_mean: Option<f64>,
/// Mean total tokens reclaimed (sum of all four M10 sources) over the same
/// context-measured runs. `None` when none are context-measured. A measured
/// run that reclaimed nothing contributes `0.0`, not exclusion.
pub tokens_reclaimed_mean: Option<f64>,
```

### 2. Add three fields to `Accumulator`

Additive (the struct derives `Default`):

```rust
peak_context_pct_sum: f64,
tokens_reclaimed_sum: f64,
context_measured_n: usize,
```

### 3. Accumulate inside the `for tag in &run.tags` loop of `aggregate`

Alongside the other conditional accumulations (e.g. the `bounces_to_approval`
block), add:

```rust
let eff = &run.context_efficiency;
if eff.peak_context_pct > 0.0 {
    acc.peak_context_pct_sum += eff.peak_context_pct;
    acc.tokens_reclaimed_sum += (eff.output_filtered_tokens
        + eff.read_evicted_tokens
        + eff.read_deduped_tokens
        + eff.compaction_tokens_reclaimed) as f64;
    acc.context_measured_n += 1;
}
```

### 4. Emit the two means in the `ScorecardRow { ... }` constructor

In the single `aggregate` constructor literal, add (matching the `Option`-mean
shape of `bounces_to_approval_mean`):

```rust
peak_context_pct_mean: if acc.context_measured_n > 0 {
    Some(acc.peak_context_pct_sum / acc.context_measured_n as f64)
} else {
    None
},
tokens_reclaimed_mean: if acc.context_measured_n > 0 {
    Some(acc.tokens_reclaimed_sum / acc.context_measured_n as f64)
} else {
    None
},
```

This is the only struct literal to update; after it, `cargo build` is green.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes (only files this phase touched).
- [ ] `cargo test` passes (existing + new).
- [ ] `ScorecardRow` has `peak_context_pct_mean: Option<f64>` and
      `tokens_reclaimed_mean: Option<f64>`.
- [ ] A bucket whose runs have `peak_context_pct` of `0.6` and `0.8` (both
      measured) reports `peak_context_pct_mean == Some(0.7)`.
- [ ] `tokens_reclaimed_mean` averages the **sum of all four** sources per run
      (`output_filtered_tokens + read_evicted_tokens + read_deduped_tokens +
      compaction_tokens_reclaimed`).
- [ ] A bucket containing only legacy runs (`ContextEfficiency::default()`, i.e.
      `peak_context_pct == 0.0`) reports `peak_context_pct_mean == None` **and**
      `tokens_reclaimed_mean == None`.
- [ ] A bucket mixing one measured run (`peak_context_pct == 0.5`) with one
      legacy run (`peak_context_pct == 0.0`) averages over the **measured run
      only** (`context_measured_n == 1`): the legacy run does not drag the mean
      toward zero.
- [ ] A measured run whose four reclaim sources sum to `0` still contributes
      (its bucket's `tokens_reclaimed_mean` is `Some(0.0)` if it is the only
      measured run — **not** `None`).

## Test plan

Unit tests in `mcp/src/scorecard.rs` `mod tests`. Use the existing `make_run`
helper (it already defaults `context_efficiency` to all-zeros via
`Default::default()`); set `context_efficiency` on the returned `PhaseRun`
before aggregating. All hermetic, no IO. Behavior pinned; exact test count and
names below are the floor, not a cap.

- `scorecard_peak_context_pct_mean_averages_measured_runs` — two runs in one
  (model, tag) bucket with `peak_context_pct` `0.6` and `0.8` → the row's
  `peak_context_pct_mean == Some(0.7)` (within `f64::EPSILON`).
- `scorecard_tokens_reclaimed_mean_sums_all_four_sources` — a single measured
  run (`peak_context_pct = 0.5`) with `output_filtered_tokens = 100`,
  `read_evicted_tokens = 50`, `read_deduped_tokens = 30`,
  `compaction_tokens_reclaimed = 20` → `tokens_reclaimed_mean == Some(200.0)`.
  **Mutation-resistant:** dropping any one source changes the rendered mean.
- `scorecard_context_efficiency_none_when_all_legacy` — a bucket whose only run
  is left at `ContextEfficiency::default()` → both `peak_context_pct_mean` and
  `tokens_reclaimed_mean` are `None` (the **must-render-`None`** negative case).
- `scorecard_context_measured_excludes_legacy_runs` — a bucket with one measured
  run (`peak_context_pct = 0.5`, reclaim sum `400`) and one legacy run
  (`peak_context_pct = 0.0`) → `peak_context_pct_mean == Some(0.5)` and
  `tokens_reclaimed_mean == Some(400.0)` (the legacy zero is excluded from both
  the numerator and the denominator — **not** averaged in as `(0.5+0.0)/2`).
- `scorecard_measured_run_with_zero_reclaim_contributes` — a bucket whose only
  run is measured (`peak_context_pct = 0.5`) but has all four reclaim sources
  `0` → `tokens_reclaimed_mean == Some(0.0)`, **not** `None` (the
  measured-but-reclaimed-nothing boundary).

## End-to-end verification

The model × tag scorecard has **no CLI binary path** — it is served only through
the `model_scorecard` MCP tool, which returns `ScorecardRow`s as JSON
(`Serialize`-derived). The real boundary this phase affects is therefore that
JSON wire output, not a terminal table. Verify it directly with a serialization
check (hermetic, single-file, no MCP client required):

- `scorecard_row_serializes_context_efficiency_means` in
  `mcp/src/scorecard.rs` `mod tests` — build one populated bucket via
  `aggregate` (a measured run with `peak_context_pct = 0.7` and reclaim sum
  `12288`), take the resulting `ScorecardRow`, `serde_json::to_string(&row)`,
  and assert the JSON string contains `"peak_context_pct_mean"` and
  `"tokens_reclaimed_mean"` with the expected numeric values. This proves the
  exact `model_scorecard` wire output carries the new fields. Quote the
  serialized JSON (or the asserted substrings) in the completion Update Log.

(There is no `cargo run` table to capture for model × tag — the `rexymcp
scorecard` CLI renders the *settings* scorecard, which is phase-08d. Do not add
a CLI path here.)

## Authorizations

None. (No new dependencies; no architecture-doc edit; no struct changes outside
`ScorecardRow` / `Accumulator`; the read field already exists.)

## Out of scope

What this phase must **not** do, even if tempted:

- **Do not touch the model × settings scorecard.** `SettingsScorecardRow`,
  `SettingsAccumulator`, `aggregate_by_settings`, and the
  `format_settings_scorecard` CLI renderer (`mcp/src/scorecard_cli.rs`) are
  **phase-08d**. That half carries three struct-literal sites across two files
  and is deliberately isolated from this single-literal, single-file phase.
- **Do not touch `mcp/src/server.rs`.** `model_scorecard_inner` builds a
  `ModelScorecardOutput` wrapper, not a `ScorecardRow` literal; the new fields
  flow through serialization with no server change.
- **Do not change `ContextEfficiency`, `PhaseRun`, or any executor-crate code.**
  This phase is `mcp/src/scorecard.rs`-only. The field is read-only here.
- **Do not add a CLI subcommand or renderer for the model × tag scorecard.** It
  is MCP-tool-only by design.
- **Do not fold reclaim variants into `StatusSummary` / `summarize` / the
  dashboard.** That is the later live-view phase (08e).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
