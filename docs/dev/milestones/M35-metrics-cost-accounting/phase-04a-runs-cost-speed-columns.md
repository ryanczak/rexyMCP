# Phase 04a: `runs` cost/speed columns + stable run id

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-03
**Estimated diff:** ~180 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Surface the numbers phase-03's core made derivable: give every run a stable
short **id** and add **TOKENS**, **COST**, and **TOK/S** columns to the
`rexymcp runs` table. This is the first consumer of `metrics::token_cost` /
`metrics::tokens_per_sec`. The run id (a git-sha-style 8-hex handle) is what the
follow-on `rexymcp runs show <id>` (phase-04b) will reference.

**Split note:** phase-04 was split — this phase (04a) is the `runs` **list**
columns + the `run_id` helper; **04b** adds the `runs show <id>` detail
subcommand (deferred so its clap-subcommand restructuring stays isolated).

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" —
  the `PhaseRun` record and the pull-not-push discipline (the loop records,
  the surface derives).
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 4 (`runs` shows tokens, cost, tok/s).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** every new `#[test]` fn goes **INSIDE** its file's
   existing `#[cfg(test)] mod tests { ... }` block. Never at file scope.
6. **Cascade note:** Task 3 adds a required `config` parameter to `format_runs`,
   which breaks all 9 call sites at once. That is the one red window — fix
   **all** of them (the exact list is in Task 3) before expecting a green
   build. Every fix is the identical trivial edit.

## Current state

### The derivation helpers (phase-03, in `executor/src/store/metrics.rs`)

```rust
pub fn tokens_per_sec(output_tokens: u32, gen_time_s: f64) -> Option<f64>
pub fn token_cost(tokens: &TokenBreakdown, rates: &ModelRates) -> f64
```

`Config::model_rates(&self, model: &str) -> ModelRates` (executor/src/config.rs)
returns a model's per-class rates (all-zero when unpriced). `TokenBreakdown`
(`executor/src/ai/types.rs`) has `input_tokens`/`output_tokens`/
`cache_read_tokens`/`cache_write_tokens` (`u32`) and `pub fn total(&self) -> u32`.
`PhaseRun` (`executor/src/store/telemetry.rs:121`) carries `ts: u64`,
`model: String`, `phase_id: String`, `tokens: TokenBreakdown`, and
`gen_time_s: f64` (phase-02).

### The `runs` table today

`mcp/src/runs.rs` — `format_runs(runs: &[PhaseRun], now_ms: u64) -> String`
(runs.rs:60). It early-returns `"(no runs)"` for empty, pushes a header line,
then one `format!` row per run. Header (runs.rs:66-68):

```
AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC  CXT_WIN  PEAK_CXT  RECLAIMED
```

The row `format!` (runs.rs:131-146) ends with the `reclaimed` value. Existing
small helpers in the file to mirror: `humanize_age` (runs.rs:39), `gate_char`
(runs.rs:53), and the reclaimed `k`-suffix formatting (runs.rs:123-129:
`—` if 0, `{n/1024}k` if `>= 1024`, else `{n}`).

`format_runs` is imported and called at `mcp/src/main.rs:551` and in eight
runs.rs tests. `runs.rs` already imports `use rexymcp_executor::config::Config;`
and `use rexymcp_executor::store::metrics;` (phase-03).

## Spec

### Task 1 — `metrics::run_id`

In `executor/src/store/metrics.rs`, add `PhaseRun` to the telemetry import and
add a stable short-id function. Use FNV-1a (32-bit) — deterministic across
platforms and Rust versions (unlike `DefaultHasher`), no dependency:

```rust
use crate::store::telemetry::{ContextEfficiency, GenerationParams, ModelRates, PhaseRun};

/// Stable git-sha-style 8-hex-char handle for a run, derived from its identity
/// (`ts`, `model`, `phase_id`). Deterministic (FNV-1a/32, no dependency, stable
/// across platforms) so `rexymcp runs` and `runs show <id>` agree. Not
/// cryptographic — just a compact, copy-pasteable address.
pub fn run_id(run: &PhaseRun) -> String {
    let seed = format!("{}|{}|{}", run.ts, run.model, run.phase_id);
    let mut h: u32 = 0x811c_9dc5;
    for b in seed.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}
```

### Task 2 — Cost/speed/token column formatting helpers

In `mcp/src/runs.rs`, add three small render helpers next to `gate_char`
(mirroring its style — pure, private):

```rust
/// Total-tokens cell: `—` for 0, else a `k`-suffixed count (matches RECLAIMED).
fn fmt_tokens(total: u32) -> String {
    if total == 0 {
        "—".to_string()
    } else if total >= 1024 {
        format!("{}k", total / 1024)
    } else {
        format!("{total}")
    }
}

/// Cost cell: `—` when unpriced/zero, else `$` with 4 decimals.
fn fmt_cost(cost: f64) -> String {
    if cost == 0.0 {
        "—".to_string()
    } else {
        format!("${cost:.4}")
    }
}

/// Throughput cell: `—` when unmeasured, else whole tok/s.
fn fmt_tok_per_sec(tps: Option<f64>) -> String {
    match tps {
        Some(v) => format!("{v:.0}"),
        None => "—".to_string(),
    }
}
```

### Task 3 — Thread `&Config` into `format_runs` and add the columns

Cost needs per-model rates, which come from `Config`. Change the signature:

```rust
pub fn format_runs(runs: &[PhaseRun], now_ms: u64, config: &Config) -> String {
```

**This breaks all 9 call sites at once — fix every one before rebuilding:**

- Production: `mcp/src/main.rs:551` → `runs::format_runs(&selected, now_ms, &config)`
  (the `config` there is the loaded `Config`; if only the *path* is in scope,
  load it once with `Config::load_with_env(&config).map_err(...)?` before the
  call — mirror how `load_runs` loads it).
- The eight runs.rs tests calling `format_runs(&…, 5000)` (runs.rs:340, 353,
  363, 450, 484, 517, 558, 576) → add `, &Config::default()` as the third
  argument. `Config::default()` has an empty `[models]`, so cost renders `—`
  — which is exactly the unpriced path those tests should exercise. (Grep
  `format_runs(` in runs.rs to confirm you caught all eight.)

Then extend the table. Put **ID** first (leftmost, so it reads like `git log`)
and append the three metric columns after `RECLAIMED`. The new header line:

```
ID        AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC  CXT_WIN  PEAK_CXT  RECLAIMED  TOKENS  COST      TOK/S
```

In the per-run loop, compute the three new cells and prepend the id:

```rust
let id = metrics::run_id(run);
let tokens_cell = fmt_tokens(run.tokens.total());
let rates = config.model_rates(&run.model);
let cost_cell = fmt_cost(metrics::token_cost(&run.tokens, &rates));
let tps_cell = fmt_tok_per_sec(metrics::tokens_per_sec(
    run.tokens.output_tokens,
    run.gen_time_s,
));
```

Add `{:<9}` for the id at the **front** of the row `format!` (with `id` as the
first argument), and append `{:<7} {:<9} {}` for `tokens_cell`, `cost_cell`,
`tps_cell` at the end. Pin the exact column **content** (id / tokens / cost /
tok-s values); the exact padding widths are yours to make the header and rows
line up.

## Acceptance criteria

- [ ] `metrics::run_id` returns 8 lowercase-hex chars, is deterministic for a
      given `(ts, model, phase_id)`, and differs when any of the three differ.
- [ ] `rexymcp runs` output contains an `ID`, `TOKENS`, `COST`, and `TOK/S`
      column header, and each row shows the run's 8-char id.
- [ ] A run whose model has **no** `[models]` pricing shows `COST` = `—`; a
      priced model shows a `$`-prefixed value.
- [ ] A run with `gen_time_s > 0` and `output_tokens > 0` shows a numeric
      `TOK/S`; a run with `gen_time_s == 0` shows `—`.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `executor/src/store/metrics.rs` tests:

- `run_id_is_eight_hex_chars` — `run_id(&run).len() == 8` and every char is a
  hex digit.
- `run_id_is_deterministic` — two calls on equal runs return the same id.
- `run_id_differs_on_ts_model_or_phase` — changing `ts` alone, `model` alone,
  or `phase_id` alone each yields a different id (three sub-assertions). Build
  the base run however the existing metrics tests build fixtures, or via a
  local helper; a `PhaseRun` literal spread from a small base is fine.

In `mcp/src/runs.rs` tests (model on `format_runs_shows_context_efficiency_columns`
at runs.rs:504 for constructing runs with token/efficiency data):

- `format_runs_shows_id_tokens_cost_speed_columns` — a run with
  `tokens.total() > 0`, `output_tokens > 0`, `gen_time_s > 0`, priced via a
  `Config` whose `[models]` entry for that model sets `input_per_mtok`, renders
  a row whose `COST` cell starts with `$` and whose `TOK/S` cell is numeric.
  Also assert the header contains `TOKENS`, `COST`, `TOK/S`. Build the priced
  `Config` in-test (`let mut cfg = Config::default(); cfg.models.insert(...)`)
  — or load from a TOML `TempDir` if that reads cleaner.
- `format_runs_unpriced_cost_is_dash` — the same run with `Config::default()`
  (empty `[models]`) renders `COST` = `—` (negative pin: unpriced ⇒ no `$`).
- `format_runs_zero_gen_time_speed_is_dash` — a run with `gen_time_s == 0.0`
  renders `TOK/S` = `—`.

## End-to-end verification

The serve now runs the phase-02+ binary, so the real store has runs with
`gen_time_s` and token data. Build and inspect the live table (read-only):

```bash
cargo build
cargo run -p rexymcp -- runs --config rexymcp.toml
```

Expected: the table now has `ID`, `TOKENS`, `COST`, `TOK/S` columns; recent
runs show an 8-hex id, a `k`-style token count, `COST` = `—` (the executor
model is unpriced in `rexymcp.toml`), and a numeric `TOK/S` for runs recorded
since the phase-02 binary went live. Paste the output in the completion Update
Log.

## Authorizations

None. (No new dependencies — FNV-1a is hand-rolled; `Config` is already
imported in runs.rs. No `docs/architecture.md` edit.)

## Out of scope

- **`rexymcp runs show <id>`** — the detail subcommand is **phase-04b**
  (clap-subcommand restructuring + a per-run detail formatter). This phase only
  adds the id and the three list columns.
- Cost/tok-s in `scorecard` / `profile` — phase-05.
- The dashboard, `rexymcp costs` — phase-06.
- The `--json` output shape (`serde_json` of `Vec<PhaseRun>`) — unchanged; the
  new columns are render-only and derive from fields already serialized. Do not
  add `run_id`/cost to the JSON.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
