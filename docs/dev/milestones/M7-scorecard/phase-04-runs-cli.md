# Phase 04: `rexymcp runs` — per-run statistics CLI view

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** review
**Depends on:** phase-01 (done). The `PhaseRun` telemetry store, `telemetry::read`,
and the `rexymcp` CLI all exist (M4/M5); this phase adds a read-only view over the
store.
**Estimated diff:** ~330 lines (a new `mcp/src/runs.rs` + a `runs` CLI subcommand
+ tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

Add a `rexymcp runs` CLI command that lists **individual** `PhaseRun` records —
each with its model, settings, gates, reliability/efficiency stats, and the
architect's verdict — filterable by model and tag, most-recent-first. This is the
first slice of the redirected M7: "see detailed statistics for each rexyMCP run."
Today the only consumer of the telemetry store is `model_scorecard`, which only
emits **aggregates**; there is no way to see the runs themselves. This phase fills
that gap.

This phase is **read-only over existing data**. It does **not** add settings
plumbing (the executor still records `generation_params` as default `None` —
that's phase-05) and does **not** add a settings slice to the scorecard
(phase-06). See Out of scope.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" →
  "Per-run detail" — *"Aggregates tell you which model/settings tend to win; the
  individual `PhaseRun` records tell you why a specific run went the way it did."*
  This command is that per-run view.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### The `PhaseRun` record (`executor/src/store/telemetry.rs:37`)

These are the fields available to display (all `pub`):

```rust
pub struct PhaseRun {
    pub ts: u64,                              // unix millis
    pub model: String,
    pub generation_params: GenerationParams,  // { temperature: Option<f64>, seed: Option<u64> }
    pub phase_id: String,
    pub tags: Vec<String>,
    pub status: String,                       // "complete" | "hard_fail" | "budget_exceeded"
    pub escalated: bool,
    pub gates: Gates,                         // { fmt, build, lint, test: Option<bool> }
    pub parse_failure_rate: f64,
    pub repairs_per_call: f64,
    pub verifier_retries: usize,
    pub tool_success_rate: f64,
    pub turns: usize,
    pub wall_clock_s: f64,
    pub tokens: TokenBreakdown,               // { prompt, completion, total }
    pub warnings: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub bounces_to_approval: Option<u32>,
    pub architect_verdict: Option<String>,
}
```

`PhaseRun` derives `Serialize` + `Deserialize`, so `--json` can print the records
directly.

### Reading the store (`executor/src/store/telemetry.rs:83`)

```rust
pub fn read(path: &Path) -> std::io::Result<Vec<PhaseRun>>
```

Reads the JSONL store, skipping malformed lines (`filter_map(...ok())`).

### Telemetry-path resolution — mirror this (`mcp/src/server.rs:276`)

`model_scorecard_inner` resolves the store path from config; do the **same** in
this phase's loader:

```rust
let telemetry_file = if let Some(ref p) = params.telemetry_path {
    PathBuf::from(p)
} else if let Some(ref dir) = cfg.telemetry.dir {
    dir.join("phase_runs.jsonl")
} else {
    return Err(
        "telemetry disabled: cfg.telemetry.dir not set and no telemetry_path provided"
            .to_string(),
    );
};
let runs = rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
```

### The module + CLI pattern to copy — `status` (`mcp/src/status.rs`, `mcp/src/main.rs:163`)

`status` is the closest analogue: a module with a **pure** fold + a `load_*`
function (resolves path, reads, folds) + a `format_*` function (human render
taking `now_ms`), and a thin CLI handler that prints JSON or human. Copy this
shape. The human formatter signature mirrors `format_status(&summary, now_ms)`:

```rust
// mcp/src/main.rs:163 — the Status handler shape to mirror
Commands::Status { repo, session, json } => {
    let summary = match status::load_status(&repo, session.as_deref()) {
        Ok(s) => s,
        Err(e) => { eprintln!("{e}"); std::process::exit(1); }
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap_or_else(|e| {
            format!("{{\"error\": \"failed to serialize status: {}\"}}", e)
        }));
    } else {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        println!("{}", status::format_status(&summary, now_ms));
    }
    Ok(())
}
```

The relative-age helper to reuse the *idea* of (`status.rs:147`):

```rust
fn humanize_age(age_ms: u64) -> String {
    let secs = age_ms / 1000;
    if secs < 60 { format!("{secs}s") }
    else if secs < 3600 { format!("{}m{:02}s", secs / 60, secs % 60) }
    else { format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60) }
}
```

For `runs`, extend it with a **day** bucket (runs can be days old):
`secs < 86400 → "{h}h"`, else `"{d}d"`. Keep it dep-free — **do not** add a date
crate (`chrono` etc. are not dependencies; adding one is a STANDARDS §2.6 blocker).

## Spec

### Task 1 — New module `mcp/src/runs.rs`

Declare `mod runs;` in `main.rs` alongside the other module declarations.

**1a. Filter struct:**

```rust
pub struct RunsFilter<'a> {
    /// Exact model match. `None` = all models.
    pub model: Option<&'a str>,
    /// Run's `tags` must contain **all** of these (AND). Empty = no tag filter.
    pub tags: &'a [String],
    /// Cap on rows after sorting (most recent first). `0` = no cap.
    pub limit: usize,
}
```

**1b. `select` — pure: filter + sort newest-first + cap.** Mirror the
`ScorecardFilter` guards (`scorecard.rs:76`): exact model, AND-tags.

```rust
use rexymcp_executor::store::telemetry::PhaseRun;

pub fn select(mut runs: Vec<PhaseRun>, filter: &RunsFilter) -> Vec<PhaseRun> {
    runs.retain(|r| {
        if let Some(m) = filter.model {
            if r.model != m { return false; }
        }
        if !filter.tags.is_empty() && !filter.tags.iter().all(|t| r.tags.contains(t)) {
            return false;
        }
        true
    });
    // newest first
    runs.sort_by(|a, b| b.ts.cmp(&a.ts));
    if filter.limit != 0 && runs.len() > filter.limit {
        runs.truncate(filter.limit);
    }
    runs
}
```

**1c. `format_runs(runs: &[PhaseRun], now_ms: u64) -> String`** — a human table,
one line per run. Pin **behavior, not exact spacing**: each line must surface, at
minimum, the run's **age** (relative, via the day-extended humanize), **model**,
**tags** (comma-joined), **settings** (`temp=…,seed=…`, or `default` when both are
`None`), **gates** (a 4-slot pass marker, e.g. `✓✓✓✓` / `✓✗✓✓`, mapping each
`Gates` field: `Some(true)`→pass, else fail), **turns**, **status**, and
**verdict** (`architect_verdict` or `—` when `None`). Include a header line and,
when `runs` is empty, a single `(no runs)` line. A run with `generation_params`
both `None` renders settings as `default` (this is the common case today — see the
note in Goal).

**1d. `load_runs` — the IO entry.** Resolve the path exactly like
`model_scorecard_inner` (quoted above), read, then `select`:

```rust
use std::path::{Path, PathBuf};

pub fn load_runs(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &RunsFilter,
) -> Result<Vec<PhaseRun>, String> {
    let cfg = rexymcp_executor::config::Config::load_with_env(config_path)
        .map_err(|e| format!("failed to load config: {}", e))?;
    let telemetry_file = if let Some(p) = telemetry_path {
        p.to_path_buf()
    } else if let Some(ref dir) = cfg.telemetry.dir {
        dir.join("phase_runs.jsonl")
    } else {
        return Err(
            "telemetry disabled: cfg.telemetry.dir not set and no --telemetry-path provided"
                .to_string(),
        );
    };
    let runs =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    Ok(select(runs, filter))
}
```

### Task 2 — `runs` CLI subcommand (`mcp/src/main.rs`)

Add a variant to `enum Commands`:

```rust
    /// List individual PhaseRun records with their per-run statistics
    Runs {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,
        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,
        /// Restrict to runs whose tags contain this tag; repeat for AND
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Max rows (most recent first); 0 = no limit
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
        /// Emit JSON instead of a human table
        #[arg(long)]
        json: bool,
    },
```

Handle it in the `match` (mirror the `Status` handler shape):

- Build `runs::RunsFilter { model: model.as_deref(), tags: &tags, limit }`.
- Call `runs::load_runs(&config, telemetry_path.as_deref(), &filter)`; on `Err`,
  `eprintln!` it and `std::process::exit(1)`.
- If `json`, print `serde_json::to_string_pretty(&selected)` with the same
  `unwrap_or_else` fallback the other handlers use.
- Else compute `now_ms` (as in the Status handler) and print
  `runs::format_runs(&selected, now_ms)`.
- `Ok(())`.

## Acceptance criteria

- [ ] `mcp/src/runs.rs` exists with `RunsFilter`, `select`, `format_runs`,
      `load_runs`; `mod runs;` declared in `main.rs`.
- [ ] `select` filters by exact model and AND-tags, sorts newest-first (by `ts`
      descending), and caps at `limit` (`0` = uncapped).
- [ ] `rexymcp runs --config <c> [--model m] [--tag t] [--limit n] [--json]`
      parses and lists matching runs; `--json` emits the `PhaseRun` records,
      human mode emits a table with the fields named in Task 1c.
- [ ] A store with no `cfg.telemetry.dir` and no `--telemetry-path` yields the
      "telemetry disabled" error (exit non-zero), not a panic.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

Construct test data by **writing JSONL lines and reading them back through
`telemetry::read`** (the pattern the scorecard server tests use — hand-written
records → a temp file → `read`). A valid `PhaseRun` line (copy/adapt; vary `ts`,
`model`, `tags`):

```json
{"ts":1717000000000,"model":"qwen","generation_params":{"temperature":null,"seed":null},"phase_id":"phase-01","tags":["rust","feature"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.1,"repairs_per_call":0.5,"verifier_retries":2,"tool_success_rate":0.9,"turns":7,"wall_clock_s":12.5,"tokens":{"prompt":0,"completion":0,"total":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":"approved_first_try"}
```

In `mcp/src/runs.rs` `#[cfg(test)] mod tests` (hermetic — `TempDir` for any file
IO):

- `select_filters_by_model_exact` — runs for `qwen` and `gemma`; `model:
  Some("qwen")` returns only the `qwen` ones (must-match), excludes `gemma`
  (must-NOT).
- `select_requires_all_tags` — a run tagged `[rust, feature]` and one tagged
  `[rust]`; `tags: ["rust","feature"]` returns only the first (AND semantics —
  must-NOT on the `[rust]`-only run).
- `select_sorts_newest_first` — three runs with `ts` 100/300/200 → returned order
  is 300, 200, 100.
- `select_limit_caps_after_sort` — five runs, `limit: 2` → the 2 newest; `limit:
  0` → all five (must-NOT-cap boundary).
- `format_runs_includes_model_and_verdict` — `format_runs` output for a run
  contains its model and its verdict string; a run with `architect_verdict: None`
  renders `—` (or the chosen sentinel). Assert **presence**, not spacing.
- `format_runs_renders_default_settings` — a run with `temperature: None, seed:
  None` shows `default` in its settings cell; a run with `temperature: Some(0.2)`
  shows `0.2`. (must-match + the default boundary.)
- `format_runs_empty_is_no_runs_line` — `format_runs(&[], now)` contains
  `(no runs)`.
- `load_runs_reads_and_selects` — write 2 JSONL lines to a temp
  `phase_runs.jsonl`, point a temp config's `[telemetry] dir` at it (or pass
  `telemetry_path`), assert `load_runs` returns both, newest first.
- `load_runs_telemetry_disabled_errors` — a config with no `[telemetry] dir` and
  `telemetry_path: None` returns the "telemetry disabled" `Err` (must-NOT panic).

In `mcp/src/main.rs` `#[cfg(test)] mod tests` (mirror the existing `cli_parse_*`
tests):

- `cli_parse_runs_collects_filters` — parse `runs --config rexymcp.toml --model
  qwen --tag rust --tag feature --limit 5 --json` → the `Runs` variant with
  `model == Some("qwen")`, `tags == ["rust","feature"]`, `limit == 5`,
  `json == true`.

Per `STANDARDS.md` §3: each filter branch gets a must-match and a must-NOT; the
`limit: 0` uncapped case and the empty/`default`-settings cases are the
boundaries.

## End-to-end verification

This ships a **real CLI surface**, so verify against the built binary:

1. `cargo run -p rexymcp -- runs --help` lists `--model`, `--tag`, `--limit`,
   `--telemetry-path`, `--json`. Quote the relevant lines in the completion
   Update Log.
2. Write a 2-line `phase_runs.jsonl` to a temp dir, point a temp `rexymcp.toml`'s
   `[telemetry] dir` at that dir, and run `cargo run -p rexymcp -- runs --config
   <tmp>/rexymcp.toml`. Quote the human table (showing both runs, newest first)
   in the completion Update Log. This exercises the real path resolution + read +
   format against an on-disk store, which the hermetic tests fake.

## Authorizations

- [x] May add the new file `mcp/src/runs.rs` and declare `mod runs;` in
      `mcp/src/main.rs`.
- [x] May add the `runs` subcommand + handler in `mcp/src/main.rs`.
- [ ] No new dependencies (no date crate — extend the dep-free humanize). No
      `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.
- [ ] Do **not** touch `executor/` — this is a read-only view over the existing
      store; the store schema and the emit site are unchanged.

## Out of scope

- **Settings plumbing + run provenance** — making sampling settings
  (temperature/seed/…) configurable, sent to the model, and recorded with real
  values, **and** capturing endpoint-reported provenance the AI client currently
  discards (served model id from the chat-response `model` field, `finish_reason`,
  context window). Today `generation_params` is always default `None`; making all
  of that real is **phase-05**. This phase only *displays* whatever is in the
  record (hence the `default` rendering) — do **not** touch `executor/` or the AI
  client.
- **A settings slice on the scorecard** (`model × settings` aggregation) —
  **phase-06**, and it depends on phase-05's real data.
- **An MCP tool** for per-run listing — the chosen surface is CLI-only. A
  Claude-facing `list_runs` tool can come later if wanted; do not add it or touch
  `mcp/src/server.rs`.
- **Per-run drill-down into the session log** — linking a run to its
  `.rexymcp/sessions/<…>.jsonl`. Useful later; not this phase. (`PhaseRun` does
  not currently carry a session-log path, so this would need a schema change.)
- **Sorting/filtering beyond model/tag/limit** (by verdict, by gate outcome,
  date ranges) — keep the first cut to the three filters specced.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-13 (started)

**Executor:** rexyMCP executor (headless)

Implementing phase-04: new `mcp/src/runs.rs` module with `RunsFilter`, `select`, `format_runs`, `load_runs`; new `runs` CLI subcommand in `mcp/src/main.rs`.

### Update — 2025-07-13 (complete)

**Executor:** rexyMCP executor (headless)

**Verification commands:**

- `cargo fmt --all --check` — passed clean
- `cargo build` — zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed clean
- `cargo test` — 542 passed, 0 failed, 2 ignored

**End-to-end verification:**

1. `cargo run -p rexymcp -- runs --help` lists all five filters:
   ```
       --model <MODEL>          Restrict to one model (exact match)
       --tag <TAGS>             Restrict to runs whose tags contain this tag; repeat for AND
       --limit <LIMIT>          Max rows (most recent first); 0 = no limit [default: 20]
       --telemetry-path <TELEMETRY_PATH>   Override the telemetry phase_runs.jsonl path
       --json                   Emit JSON instead of a human table
   ```

2. Human table against a 2-line on-disk store (both runs shown, newest first):
   ```
   AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT
   733d    gemma  python,refactor temp=0.2,seed=42 ✓✓✗✓  3      complete  —
   733d    qwen   rust,feature   default      ✓✓✓✓  7      complete  approved_first_try
   ```

**Literal grep proof** — spec-pinned "(no runs)" sentinel:
```
$ grep -n "no runs" mcp/src/runs.rs
61:        return "(no runs)".to_string();
304:        assert!(out.contains("(no runs)"));
```

**Files changed:**
- `mcp/src/runs.rs` — new module (RunsFilter, select, format_runs, load_runs, tests)
- `mcp/src/main.rs` — `mod runs;`, `Runs` CLI variant + handler, `cli_parse_runs_collects_filters` test

**Commit:** one conventional commit (`feat: add rexymcp runs CLI subcommand`)

**Notes for review:** None — implementation matches spec exactly.
