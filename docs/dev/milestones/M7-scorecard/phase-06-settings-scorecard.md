# Phase 06: `model × settings` scorecard slice — `rexymcp scorecard` CLI

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** todo
**Depends on:** phase-05a/05b/05c (done — `generation_params` and the provenance/
reliability signals are now real in `PhaseRun`) and phase-04 (done — the `runs.rs`
CLI module is the structural template to mirror).
**Estimated diff:** ~320 lines (a new `aggregate_by_settings` + a `scorecard` CLI
module + a `Scorecard` subcommand + tests).
**Tags:** language=rust, kind=feature, size=l

## Goal

Let a user answer **"which settings work best for this model?"** from the CLI. Add a
`rexymcp scorecard` command that aggregates `PhaseRun` records into a **`model ×
settings`** competency matrix — one row per (model, sampling-settings) bucket, with
the same quality / reliability / efficiency / supervision means the existing
`model × tag` scorecard reports, **plus** the new `length_finish_rate` reliability
signal (phase-05b). Settings only became real in phase-05a, so this is the first
phase where the slice carries signal.

This is the **milestone-closing** phase for M7. It is **additive**: it adds a new
aggregation function and a new CLI surface, and does **not** modify the existing
`aggregate` (model × tag) or the `model_scorecard` MCP tool.

## Architecture references

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  scorecard aggregates `PhaseRun` records into a competency matrix. The model × tag
  matrix exists (M5, the `model_scorecard` MCP tool); this adds the candidate
  **model × settings** dimension as a CLI view.
- M7 README § Phases, the 06 bullet: "Aggregate/compare `model × settings` (and the
  new provenance/reliability signals)."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching code. This phase is **purely
   additive** — a new struct, a new function, a new module, a new subcommand. It
   does **not** change `ScorecardRow`, `aggregate`, the `Accumulator`, or any
   existing struct/enum, so there is no caller cascade.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### The existing `model × tag` aggregation — the shape to mirror (`mcp/src/scorecard.rs:73`)

```rust
pub fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardRow> {
    let mut buckets: BTreeMap<(String, String), Accumulator> = BTreeMap::new();
    for run in runs {
        if let Some(model) = filter.model && run.model != model { continue; }
        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| run.tags.contains(t)) { continue; }
        for tag in &run.tags {
            let key = (run.model.clone(), tag.clone());
            let acc = buckets.entry(key).or_default();
            acc.n += 1;
            if gates_all_pass(&run.gates) { acc.gates_all_pass += 1; }
            acc.parse_failure_rate_sum += run.parse_failure_rate;
            // ... other sums ...
            if run.architect_verdict.is_some() {
                acc.n_with_verdict += 1;
                if run.architect_verdict.as_deref() == Some("approved_first_try") { acc.approved_first_try_count += 1; }
            }
            if let Some(b) = run.bounces_to_approval { acc.bounces_sum += b as f64; acc.bounces_n += 1; }
        }
    }
    // finalize: filter min_runs, divide sums by n, Option fields when their n == 0
}
```

`ScorecardFilter` (scorecard.rs:35) has `tags`, `model`, `min_runs` — **reuse it
unchanged** for the settings aggregation (same filtering semantics). `gates_all_pass`
(scorecard.rs:66) is reusable too.

### The settings label — must match what `rexymcp runs` shows (`mcp/src/runs.rs:74`)

`format_runs` already renders a run's settings as a label:

```rust
let settings = match (run.generation_params.temperature, run.generation_params.seed) {
    (None, None) => "default".to_string(),
    (Some(t), None) => format!("temp={t}"),
    (None, Some(s)) => format!("seed={s}"),
    (Some(t), Some(s)) => format!("temp={t},seed={s}"),
};
```

The settings **bucket key** must use this **exact** rendering, so a user sees the
same `temp=0.2,seed=42` / `default` label in `rexymcp runs` and `rexymcp scorecard`.
Duplicate this 4-arm match in `scorecard.rs` (a 4-line match is below the threshold
for extracting a shared helper per STANDARDS §2.2) and pin the format with a test.

### The CLI module to mirror — `runs.rs` (`mcp/src/runs.rs`) + the `Runs` subcommand (`mcp/src/main.rs:238`)

Phase-04's `runs` is the structural template: a module with a pure `select`/format +
a `load_*` (resolve telemetry path from config-or-override, read, aggregate) + a
thin CLI handler. `load_runs` (runs.rs:105) shows the path resolution to copy:

```rust
pub fn load_runs(config_path: &Path, telemetry_path: Option<&Path>, filter: &RunsFilter) -> Result<Vec<PhaseRun>, String> {
    let cfg = Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;
    let telemetry_file = if let Some(p) = telemetry_path {
        p.to_path_buf()
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.join("phase_runs.jsonl")
    } else {
        return Err("telemetry disabled: cfg.telemetry.dir not set and no --telemetry-path provided".to_string());
    };
    let runs = rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    Ok(select(runs, filter))
}
```

The `Runs` subcommand + handler (main.rs:238 / :219) is the shape for the new
`Scorecard` subcommand.

### Available `PhaseRun` fields (`executor/src/store/telemetry.rs:37`)

All fields used by `aggregate` plus the phase-05 additions: `generation_params`
(temperature/seed — the settings key), `length_finish_rate: Option<f64>` (the new
reliability mean to add), `served_model`, `context_window`.

## Spec

### Task 1 — `SettingsScorecardRow` + `aggregate_by_settings` (`mcp/src/scorecard.rs`)

Add (do **not** modify the existing `ScorecardRow`/`aggregate`/`Accumulator`):

```rust
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SettingsScorecardRow {
    pub model: String,
    /// Sampling-settings label, e.g. "temp=0.2,seed=42" or "default".
    pub settings: String,
    pub n_runs: usize,
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
    /// Mean of `length_finish_rate` over runs where it is `Some`. `None` when none.
    pub length_finish_rate_mean: Option<f64>,
    pub repairs_per_call_mean: f64,
    pub tool_success_rate_mean: f64,
    pub verifier_retries_mean: f64,
    pub turns_mean: f64,
    pub wall_clock_s_mean: f64,
    pub escalation_rate: f64,
    pub n_with_verdict: usize,
    pub approved_first_try_rate: Option<f64>,
    pub bounces_to_approval_mean: Option<f64>,
}
```

`aggregate_by_settings(runs: &[PhaseRun], filter: &ScorecardFilter) ->
Vec<SettingsScorecardRow>`:

- Apply the **same** model + AND-tags filtering as `aggregate` (reuse the guards).
- Bucket by `(run.model.clone(), settings_label(run))` where `settings_label` is the
  4-arm match quoted above. **One bucket per (model, settings)** — unlike the tag
  aggregation, do **not** explode per tag; each run contributes to exactly one
  settings bucket.
- Accumulate the same sums as `Accumulator` **plus** a `length_finish_rate_sum: f64`
  and `length_finish_n: usize` (increment only when `run.length_finish_rate` is
  `Some`).
- Finalize like `aggregate`: drop buckets with `n < filter.min_runs`; means are
  `sum / n`; `length_finish_rate_mean` is `Some(sum / length_finish_n)` when
  `length_finish_n > 0` else `None`; `approved_first_try_rate` / `bounces_to_approval_mean`
  follow the existing `None`-when-their-`n`-is-0 rule.
- Sort: `settings` asc, then `n_runs` desc, then `model` asc (mirror the existing
  sort, swapping `tag`→`settings`).

You may add a private accumulator struct for this function (e.g.
`SettingsAccumulator`) rather than extending the shared `Accumulator`.

### Task 2 — `scorecard` CLI module (`mcp/src/scorecard_cli.rs`, new; `mod scorecard_cli;` in main.rs)

Mirror `runs.rs`'s load + format split:

- `pub fn load_settings_scorecard(config_path: &Path, telemetry_path: Option<&Path>,
  filter: &ScorecardFilter) -> Result<Vec<SettingsScorecardRow>, String>` — resolve
  the telemetry path **exactly** as `load_runs` (config `telemetry.dir` or
  `--telemetry-path` override; the same "telemetry disabled" error string), read via
  `telemetry::read`, return `scorecard::aggregate_by_settings(runs, filter)`.
- `pub fn format_settings_scorecard(rows: &[SettingsScorecardRow]) -> String` — a
  human table, one line per row, header included; `(no runs)` when empty. Each line
  must surface at least: **model**, **settings**, **n_runs**, **gates_pass_rate**,
  **length_finish_rate_mean** (the new signal; render `—` when `None`),
  **approved_first_try_rate** (`—` when `None`), and **turns_mean**. Pin behavior,
  not spacing.

### Task 3 — `Scorecard` subcommand (`mcp/src/main.rs`)

Add a `Commands::Scorecard` variant mirroring `Commands::Runs` (main.rs:238):

```rust
    /// Aggregate runs into a model × settings competency matrix
    Scorecard {
        #[arg(long)]
        config: PathBuf,
        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,
        /// Restrict to runs whose tags contain this tag; repeat for AND
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Drop buckets with fewer than this many runs
        #[arg(long, default_value_t = 0)]
        min_runs: usize,
        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
        /// Emit JSON instead of a human table
        #[arg(long)]
        json: bool,
    },
```

Handle it like the `Runs` arm (main.rs:219): build
`scorecard::ScorecardFilter { model: model.as_deref(), tags: &tags, min_runs }`, call
`scorecard_cli::load_settings_scorecard(...)`, `eprintln!`+`exit(1)` on `Err`, then
`--json` → `serde_json::to_string_pretty` (same `unwrap_or_else` fallback) else
`println!` the `format_settings_scorecard` output.

## Acceptance criteria

- [ ] `aggregate_by_settings` buckets by `(model, settings-label)`, one bucket per
      run (must-NOT explode per tag); two runs with different temperatures land in
      **different** buckets; two with identical settings land in the **same** bucket.
- [ ] The settings label matches `rexymcp runs` exactly: both-`None` → `default`;
      `temperature=0.2, seed=42` → `temp=0.2,seed=42`.
- [ ] `length_finish_rate_mean` is the mean over runs where it is `Some`, and `None`
      when no run in the bucket has it (must-NOT divide by zero / report `0.0`).
- [ ] `min_runs` drops low-sample buckets; model + AND-tags filters behave as in
      `aggregate`.
- [ ] `rexymcp scorecard --config <c> [--model m] [--tag t] [--min-runs n] [--json]`
      parses and prints the matrix; `--json` emits the `SettingsScorecardRow`
      records, human mode the table.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

In `mcp/src/scorecard.rs` `#[cfg(test)] mod tests` (extend the existing `make_run`
helper usage — note it now takes the phase-05 fields; build runs with explicit
`generation_params` and `length_finish_rate` via a small local helper or struct
update):

- `by_settings_buckets_distinct_settings` — runs with `temp=0.2` and `temp=0.7`
  (same model) produce two rows; identical-settings runs share one row (`n_runs`
  counts them). must-match + must-NOT-merge.
- `by_settings_default_label_for_none` — a run with `temperature: None, seed: None`
  buckets under `settings == "default"`.
- `by_settings_does_not_explode_per_tag` — one run with three tags produces **one**
  settings row with `n_runs == 1` (contrast with the per-tag `aggregate`).
- `by_settings_length_finish_rate_mean` — two runs in a bucket with
  `length_finish_rate` `Some(0.2)` and `Some(0.4)` → mean `Some(0.3)`; a bucket whose
  runs are all `None` → `None` (the divide-by-zero boundary).
- `by_settings_min_runs_drops_low_sample` — mirror the existing `min_runs` test.

In `mcp/src/scorecard_cli.rs` `#[cfg(test)] mod tests` (hermetic, `TempDir`):

- `load_settings_scorecard_reads_and_aggregates` — write 2 JSONL lines to a temp
  store, point a temp config's `[telemetry] dir` at it (or pass `telemetry_path`),
  assert the returned rows aggregate them.
- `load_settings_scorecard_telemetry_disabled_errors` — config without
  `[telemetry] dir` and `telemetry_path: None` → the "telemetry disabled" `Err`
  (must-NOT panic).
- `format_settings_scorecard_shows_settings_and_signal` — a row with
  `settings: "temp=0.2,seed=42"`, `length_finish_rate_mean: Some(0.25)` renders both
  (and a `None` length-finish renders `—`). `format_settings_scorecard(&[])` contains
  `(no runs)`.

In `mcp/src/main.rs` `#[cfg(test)] mod tests`:

- `cli_parse_scorecard_collects_filters` — parse `scorecard --config rexymcp.toml
  --model qwen --tag rust --min-runs 3 --json` → the `Scorecard` variant with the
  expected fields.

## End-to-end verification

Ships a real CLI surface — verify against the built binary and quote in the Update
Log:

1. `cargo run -p rexymcp -- scorecard --help` lists `--model`, `--tag`, `--min-runs`,
   `--telemetry-path`, `--json`.
2. Write a 3-line `phase_runs.jsonl` for one model with two distinct settings (e.g.
   two runs `temp=0.2,seed=42` and one `default`), point a temp `rexymcp.toml`'s
   `[telemetry] dir` at it, and run `cargo run -p rexymcp -- scorecard --config
   <tmp>/rexymcp.toml`. Quote the human table showing the two settings buckets with
   their `n_runs` and means. This exercises real path resolution + read + aggregate +
   format.

## Authorizations

- [x] May add `SettingsScorecardRow` + `aggregate_by_settings` to
      `mcp/src/scorecard.rs`; a new `mcp/src/scorecard_cli.rs` module (declared in
      `main.rs`); and a `Scorecard` subcommand + handler in `mcp/src/main.rs`.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.
- [ ] Do **not** modify the existing `ScorecardRow`, `aggregate`, `Accumulator`, or
      the `model_scorecard` MCP tool (`mcp/src/server.rs`). This phase is additive.

## Out of scope

- **A `model × tag` CLI** — that matrix is already available via the
  `model_scorecard` MCP tool; this phase adds only the new settings slice. A `--by
  tag|settings` selector can come later.
- **An MCP tool for the settings matrix** — the chosen surface is CLI (matches
  `rexymcp runs`/`status`). A Claude-facing tool can be added later if wanted.
- **`context_window` / `served_model` aggregation** — these are per-run identity
  values (already visible in `rexymcp runs`), not settings-comparison means. Only
  `length_finish_rate` is added as a new aggregated signal.
- **Realigning `docs/architecture.md`'s benchmark/routing language** — a known M7
  follow-up for the milestone retrospective; not this phase (and architecture.md is
  not editable here).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
