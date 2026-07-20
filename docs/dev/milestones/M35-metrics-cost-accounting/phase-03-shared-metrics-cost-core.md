# Phase 03: Shared metrics & cost core + `[models]` pricing

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-02
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=refactor, size=l

## Goal

Create one shared module that owns every *derived* metric number — reclaimed-token
sums, tok/s, sampling-settings labels, and per-model USD cost over all four token
classes — and route the four existing hand-rolled duplicates through it. Add
per-model executor pricing to the existing `[models]` config table so a local
model can carry `$/Mtok` rates (unpriced ⇒ $0). **This phase changes no CLI
output**: the migrations are behavior-preserving and the new cost/pricing/tok-s
functions have no display consumer yet — phases 04–06 render them.

**Committed consumers** (so nothing here is dead state): phase-04 renders
`token_cost` + `tokens_per_sec` as `runs` columns and in `runs show`; phase-05
renders them in the unified scorecard/profile. Each new pure function is
exercised by its own unit tests in this phase.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" —
  the `PhaseRun`/`ContextEfficiency` record shapes these helpers derive from.
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 3 (shared core + pricing); design-finding "tokens reclaimed is
  hand-summed in four places."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** every new `#[test]` fn goes **INSIDE** its file's
   existing `#[cfg(test)] mod tests { ... }` block (append before the closing
   `}`). For the new `metrics.rs` module, create the `#[cfg(test)] mod tests`
   block at the file bottom, `use super::*;`. Never place `#[test]` fns at file
   scope. (Two hard-fails earlier in M35 traced to this.)
6. **Execution order matters (cascade safety).** Do Part A fully (new module,
   all green), then Part B (behavior-preserving migrations, one file per turn,
   the compiler catches any missed site), then Part C (pricing — pre-adapt the
   two literals *first*, then add fields). Do not interleave parts.

## Current state

### The types these helpers derive from (all in the `executor` crate)

`ContextEfficiency` (`executor/src/store/telemetry.rs:46`) — the persisted
per-run efficiency record. Reclaimed tokens = the sum of its four reclaim
fields:

```rust
pub struct ContextEfficiency {
    pub peak_context_pct: f64,
    pub compaction_count: usize,
    pub compaction_tokens_reclaimed: usize,   // ← reclaim field
    pub output_filtered_tokens: usize,        // ← reclaim field
    pub read_evicted_tokens: usize,           // ← reclaim field
    pub read_deduped_tokens: usize,           // ← reclaim field
}
```

`GenerationParams` (`executor/src/store/telemetry.rs:24`) — `temperature:
Option<f64>`, `seed: Option<u64>` (both `#[serde(default)]`).

`TokenBreakdown` (`executor/src/ai/types.rs:43`) — `input_tokens`,
`output_tokens`, `cache_read_tokens`, `cache_write_tokens` (all `u32`).
Note **`cache_write_tokens` is the cache-creation class** (mirror of
`ArchitectTokens.cache_creation`).

`ArchitectRates` (`executor/src/store/telemetry.rs:443`) — the existing
per-class USD-per-Mtok rate type, and `ArchitectTokens::cost` (telemetry.rs:452)
is the worked template for the new `token_cost`:

```rust
pub struct ArchitectRates {
    pub input_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub output_per_mtok: f64,
}
impl ArchitectTokens {
    pub fn cost(&self, rates: &ArchitectRates) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.input, rates.input_per_mtok)
            + per_m(self.cache_creation, rates.cache_creation_per_mtok)
            + per_m(self.cache_read, rates.cache_read_per_mtok)
            + per_m(self.output, rates.output_per_mtok)
    }
}
```

### The four hand-rolled reclaimed-sum duplicates

1. `mcp/src/scorecard.rs:133-136` (inside `aggregate_by_settings`):
   ```rust
   acc.tokens_reclaimed_sum += (eff.output_filtered_tokens
       + eff.read_evicted_tokens
       + eff.read_deduped_tokens
       + eff.compaction_tokens_reclaimed) as f64;
   ```
   (`let eff = &run.context_efficiency;` a few lines above.)
2. `mcp/src/scorecard.rs:326-329` — identical shape, a second aggregation.
3. `mcp/src/runs.rs:119-122`:
   ```rust
   let reclaimed_total = eff.output_filtered_tokens
       + eff.read_evicted_tokens
       + eff.read_deduped_tokens
       + eff.compaction_tokens_reclaimed;
   ```
   (`let eff = &run.context_efficiency;` at runs.rs:111.)
4. `mcp/src/status.rs:335-340` — **the odd one out**: it sums a live
   `StatusSummary`, not a `ContextEfficiency`, and its compaction term is
   `before − after` (StatusSummary stores `compaction_tokens_before` /
   `compaction_tokens_after` separately, not a pre-summed field):
   ```rust
   let reclaimed = summary.output_filtered_tokens
       + summary.read_evicted_tokens
       + summary.read_deduped_tokens
       + summary
           .compaction_tokens_before
           .saturating_sub(summary.compaction_tokens_after);
   ```
   The `format!` breakdown at status.rs:343-350 that renders the four terms
   individually **stays as is** — only the single `reclaimed` total routes
   through the helper.

### The two sampling-settings-label duplicates

- `mcp/src/scorecard.rs:39-49` — a `fn settings_label(run: &PhaseRun) -> String`
  matching on `(temperature, seed)`.
- `mcp/src/runs.rs:74-82` — the same match, inlined into `format_runs` as
  `let settings = match (...) { ... };`.

### The `[models]` config table (pricing goes here — do NOT add a new section)

`Config.models: HashMap<String, ModelOverride>` (config.rs:344) already exists;
`ModelOverride` (config.rs:303) holds per-model governor/sampling overrides,
all fields `Option<T>`, `#[derive(… Default)]` + `#[serde(default)]`. Add the
pricing fields **here**. Two full struct literals name every field and so must
be pre-adapted before the fields are added: `mcp/src/runner.rs:753` and
`mcp/src/runner.rs:884`.

## Spec

### Part A — the shared `metrics` module (fully additive)

### Task 1 — Add a `ModelRates` alias

In `executor/src/store/telemetry.rs`, directly after the `ArchitectRates`
definition (~line 448), add:

```rust
/// Per-class USD-per-Mtok rates for **any** model's token cost (executor or
/// architect). Structurally identical to the architect rate type; aliased so
/// call sites read as model-neutral.
pub type ModelRates = ArchitectRates;
```

### Task 2 — Create `executor/src/store/metrics.rs`

Register it: in `executor/src/store/mod.rs` add `pub mod metrics;` (the file is
just `pub mod sessions;` / `pub mod telemetry;` today).

Write the module with these four pure free functions. No struct, no state:

```rust
//! Shared derivations over telemetry records — the single home for every
//! *derived* metric number (reclaimed sums, tok/s, settings labels, cost).
//! Readers (`runs`/`scorecard`/`status`/dashboard) call these instead of
//! re-deriving; pinning the definition once.

use crate::ai::types::TokenBreakdown;
use crate::store::telemetry::{ContextEfficiency, GenerationParams, ModelRates};

/// Total tokens reclaimed in a run: boundary-filter + evicted + deduped +
/// compaction. The one definition of "reclaimed."
pub fn reclaimed_total(eff: &ContextEfficiency) -> usize {
    eff.output_filtered_tokens
        + eff.read_evicted_tokens
        + eff.read_deduped_tokens
        + eff.compaction_tokens_reclaimed
}

/// Generation throughput in output tokens per second. `None` when `gen_time_s`
/// is non-positive (no timed generation recorded) — callers render `—`.
pub fn tokens_per_sec(output_tokens: u32, gen_time_s: f64) -> Option<f64> {
    if gen_time_s > 0.0 {
        Some(output_tokens as f64 / gen_time_s)
    } else {
        None
    }
}

/// Sampling-settings label: `"default"` / `"temp=T"` / `"seed=S"` /
/// `"temp=T,seed=S"`. The exact strings `runs`/`scorecard` render.
pub fn settings_label(params: &GenerationParams) -> String {
    match (params.temperature, params.seed) {
        (None, None) => "default".to_string(),
        (Some(t), None) => format!("temp={t}"),
        (None, Some(s)) => format!("seed={s}"),
        (Some(t), Some(s)) => format!("temp={t},seed={s}"),
    }
}

/// USD cost of an executor `TokenBreakdown` at per-class rates. Mirrors
/// `ArchitectTokens::cost`; note `cache_write_tokens` is the cache-creation
/// class.
pub fn token_cost(tokens: &TokenBreakdown, rates: &ModelRates) -> f64 {
    let per_m = |toks: u32, rate: f64| (toks as f64 / 1_000_000.0) * rate;
    per_m(tokens.input_tokens, rates.input_per_mtok)
        + per_m(tokens.cache_write_tokens, rates.cache_creation_per_mtok)
        + per_m(tokens.cache_read_tokens, rates.cache_read_per_mtok)
        + per_m(tokens.output_tokens, rates.output_per_mtok)
}
```

Add the `#[cfg(test)] mod tests` block per the Test plan.

### Part B — route the duplicates through the module (behavior-preserving)

Do **one file per turn**; `cargo build` after each — the compiler flags any
site you miss. Import `use rexymcp_executor::store::metrics;` (mcp crate) at the
top of each file you touch.

### Task 3 — `mcp/src/runs.rs`

- Replace the reclaimed sum (runs.rs:119-122) with
  `let reclaimed_total = metrics::reclaimed_total(eff);`.
- Replace the inline `let settings = match (...) { ... };` (runs.rs:74-82) with
  `let settings = metrics::settings_label(&run.generation_params);`.

### Task 4 — `mcp/src/scorecard.rs`

- Both reclaimed sums (scorecard.rs:133-136 and :326-329) become
  `acc.tokens_reclaimed_sum += metrics::reclaimed_total(eff) as f64;`.
- Delete the local `fn settings_label(run: &PhaseRun) -> String`
  (scorecard.rs:38-49) and replace its call site(s) with
  `metrics::settings_label(&run.generation_params)`. (Grep
  `settings_label(` within the file to find every caller.)

### Task 5 — `mcp/src/status.rs`

Route the single `reclaimed` total (status.rs:335-340) through the helper by
building a `ContextEfficiency` from the summary's fields — the compaction term
maps `before − after` into `compaction_tokens_reclaimed`:

```rust
let reclaimed = metrics::reclaimed_total(&rexymcp_executor::store::telemetry::ContextEfficiency {
    output_filtered_tokens: summary.output_filtered_tokens,
    read_evicted_tokens: summary.read_evicted_tokens,
    read_deduped_tokens: summary.read_deduped_tokens,
    compaction_tokens_reclaimed: summary
        .compaction_tokens_before
        .saturating_sub(summary.compaction_tokens_after),
    ..Default::default()
});
```

Leave the `format!("reclaimed: … (filter {} evict {} dedupe {} compaction {})", …)`
detail breakdown (status.rs:343-350) **untouched** — it renders the individual
terms, which the helper does not change.

### Part C — per-model executor pricing on `[models]`

### Task 6 — Pre-adapt the two `ModelOverride` literals (stay green)

Before adding any field, append `..Default::default()` as the final entry of
the `ModelOverride { … }` literals at `mcp/src/runner.rs:753` and
`mcp/src/runner.rs:884`. `ModelOverride` derives `Default`, so this compiles
now and absorbs the new fields in Task 7. Build — green.

### Task 7 — Add pricing fields to `ModelOverride`

In `executor/src/config.rs`, add four fields to `ModelOverride` (config.rs:303),
matching the architect field names, each `#[serde(default)]`:

```rust
    /// Per-Mtok USD executor pricing (M35). `None` ⇒ that class costs $0.
    #[serde(default)]
    pub input_per_mtok: Option<f64>,
    #[serde(default)]
    pub output_per_mtok: Option<f64>,
    #[serde(default)]
    pub cache_read_per_mtok: Option<f64>,
    #[serde(default)]
    pub cache_creation_per_mtok: Option<f64>,
```

### Task 8 — `Config::model_rates`

In `executor/src/config.rs`, add a method resolving a model id to its
`ModelRates` (all-zero when the model has no `[models]` entry or leaves a class
unset — **unpriced ⇒ $0**, per the milestone decision):

```rust
    /// Per-class executor rates for `model` from its `[models]` entry; every
    /// unset class is `0.0` (unpriced ⇒ $0). No fallback to `known_model_rates`
    /// — executor models are local, never the Claude pricing table.
    pub fn model_rates(&self, model: &str) -> crate::store::telemetry::ModelRates {
        let o = self.models.get(model);
        crate::store::telemetry::ModelRates {
            input_per_mtok: o.and_then(|o| o.input_per_mtok).unwrap_or(0.0),
            output_per_mtok: o.and_then(|o| o.output_per_mtok).unwrap_or(0.0),
            cache_read_per_mtok: o.and_then(|o| o.cache_read_per_mtok).unwrap_or(0.0),
            cache_creation_per_mtok: o.and_then(|o| o.cache_creation_per_mtok).unwrap_or(0.0),
        }
    }
```

### Task 9 — `rexymcp init` template comment

The `rexymcp init` template is a hardcoded string constant in `mcp/src/init.rs`;
the commented `[models."<model-id>"]` example block runs from init.rs:47 to the
blank line before `[commands]` (init.rs:~62, last override line
`# output_window_bytes = 524288`). Append four commented pricing lines to the
**end** of that block (after `# output_window_bytes = 524288`, before the blank
line):

```toml
# input_per_mtok = 0.0             # M35 executor pricing ($/Mtok); unpriced classes cost $0
# output_per_mtok = 0.0            # override per-model output pricing
# cache_read_per_mtok = 0.0        # override per-model cache-read pricing
# cache_creation_per_mtok = 0.0    # override per-model cache-creation pricing
```

Keep them **commented** — `mcp/src/init.rs`'s `init_documents_models_override_section`
test (init.rs:213-232) requires the whole `[models]` block to parse to an empty
`config.models`; an uncommented line would populate the map and fail it. Do not
create a second `[models]` block.

## Acceptance criteria

- [ ] `grep -rn "output_filtered_tokens +" mcp/src` returns **nothing** (all
      four reclaimed sums now route through `metrics::reclaimed_total`; the
      status.rs `before − after` compaction term moved into the struct-build).
- [ ] `grep -n "fn settings_label" mcp/src/scorecard.rs` returns nothing.
- [ ] `executor/src/store/metrics.rs` exists and `store/mod.rs` declares
      `pub mod metrics;`.
- [ ] `Config::model_rates("unknown-model")` returns all-zero rates; a
      `[models."m"]` entry with `input_per_mtok = 2.0` yields
      `model_rates("m").input_per_mtok == 2.0`.
- [ ] `token_cost` on `{input:1_000_000, output:1_000_000, cache_read:0,
      cache_write:0}` at rates `{input:3.0, output:15.0, …}` equals `18.0`.
- [ ] `cargo run -p rexymcp -- runs --config rexymcp.toml`,
      `scorecard`, and `status --repo .` produce the **same** output as before
      this phase (behavior-preserving) — see E2E.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `executor/src/store/metrics.rs` (`#[cfg(test)] mod tests`):

- `reclaimed_total_sums_all_four_reclaim_fields` — a `ContextEfficiency` with
  `output_filtered_tokens: 100, read_evicted_tokens: 50, read_deduped_tokens:
  30, compaction_tokens_reclaimed: 20` (other fields default) sums to `200`.
  **Must-NOT pin:** set exactly one field to a distinct value and assert the
  total reflects it (guards a dropped term).
- `tokens_per_sec_divides_output_by_time` — `tokens_per_sec(1000, 2.0) ==
  Some(500.0)`.
- `tokens_per_sec_none_when_time_zero` — `tokens_per_sec(1000, 0.0) == None`
  (negative pin against a divide-by-zero / spurious `Some`).
- `settings_label_covers_all_four_shapes` — the four `(temp, seed)`
  combinations produce `"default"`, `"temp=0.2"`, `"seed=42"`,
  `"temp=0.2,seed=42"`.
- `token_cost_prices_each_class` — `TokenBreakdown { input_tokens: 1_000_000,
  output_tokens: 1_000_000, cache_read_tokens: 1_000_000, cache_write_tokens:
  1_000_000 }` at `ModelRates { input_per_mtok: 1.0, output_per_mtok: 2.0,
  cache_read_per_mtok: 3.0, cache_creation_per_mtok: 4.0 }` equals `10.0`
  (pins that each class uses its own rate and `cache_write → cache_creation`).
- `token_cost_zero_when_unpriced` — any tokens at
  `ModelRates::default()` cost `0.0`.

In `executor/src/config.rs` (`#[cfg(test)] mod tests`):

- `model_rates_zero_for_unconfigured_model` — a default `Config` returns
  all-zero `model_rates("anything")`.
- `model_rates_reads_configured_pricing` — load a TOML with
  `[models."m"]\ninput_per_mtok = 2.0\noutput_per_mtok = 9.0` and assert
  `model_rates("m")` has `input_per_mtok == 2.0`, `output_per_mtok == 9.0`,
  and the two cache classes `== 0.0` (unset ⇒ 0).
- `model_override_pricing_absent_parses` — a `[models."m"]` table with **only**
  governor keys (no pricing) still loads and `model_rates("m")` is all-zero
  (negative: pricing fields are optional, old `[models]` tables unaffected).

## End-to-end verification

This phase ships **no new runtime-loadable output** (cost/tok-s/pricing have no
CLI surface until phase-04). Its runtime-visible contract is **behavior
preservation** of the three readers that were refactored. Build the real binary
and confirm each still runs clean against the real store/logs (read-only):

```bash
cargo build
cargo run -p rexymcp -- runs --config rexymcp.toml
cargo run -p rexymcp -- scorecard --config rexymcp.toml
cargo run -p rexymcp -- status --repo .
```

Expected: each command runs without error and prints its usual table/summary
(an empty store prints the usual empty-state line — fine). The reclaimed and
settings columns are unchanged from before the refactor. Paste all three
outputs in the completion Update Log.

## Authorizations

None. (No new dependencies; `HashMap` is already imported in config.rs. No
`docs/architecture.md` edit — the record shapes are unchanged; only derived
helpers and additive config fields are added.)

## Out of scope

- **Any CLI/dashboard output change** — no new columns, no cost display, no
  tok/s column. Phases 04 (`runs`), 05 (scorecard/profile), 06 (`costs` +
  dashboard) render these. If a migration would change a rendered string, it is
  wrong — the refactor must be byte-for-byte behavior-preserving.
- Consuming `token_cost` / `tokens_per_sec` anywhere beyond their unit tests.
- The architect cost path (`ArchitectTokens::cost`,
  `effective_architect_rates`, dashboard architect costs) — unchanged; do not
  refactor it into `metrics.rs` this phase.
- `known_model_rates` and the Claude pricing table — executor pricing is
  `[models]`-only; no fallback.
- The live `status` tok/s (`tok_per_sec_*`, computed from `Metrics` intervals)
  — a different signal from per-run `gen_time_s`; leave it entirely.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
