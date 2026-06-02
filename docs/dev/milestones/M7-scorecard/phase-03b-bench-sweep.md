# Phase 03b: `rexymcp bench` multi-model sweep + one minimal fixture

> **⚠ ROLLED BACK 2026-06-02 — benchmarking deprecated.** Never landed: the one
> dispatch stalled mid-run and its partial work was discarded. The benchmark
> feature is dropped; M7 is being redirected toward detailed per-run statistics
> for regular rexyMCP runs. Retained for historical context only — do not
> dispatch.

**Milestone:** M7 — Model scorecard & routing
**Status:** rolled-back
**Depends on:** phase-03a (done). The stamped single-run primitive
(`run_phase` with `bench_suite: Some(name)`) exists; this phase drives it once
per (model, benchmark phase) over a copied fixture.
**Estimated diff:** ~380 lines (a new `mcp/src/bench.rs`, a `bench` CLI
subcommand, two scoped `Cargo.toml` edits, one minimal checked-in fixture, and
hermetic tests).
**Tags:** language=rust, kind=feature, size=l

## Goal

Add a `rexymcp bench` command that runs a **benchmark suite** — a set of
reference phase docs against a frozen fixture repo — across one or more
models-under-test, copying the pristine fixture into a fresh temp dir per run so
every model faces identical starting conditions, and emitting a `bench_suite`-
stamped `PhaseRun` for each. This is the controlled, apples-to-apples data
source the `model_scorecard`'s `SourceFilter::Suite` (phase-02) reads.

**Scope is deliberately one suite, one phase, one fixture** — enough to prove
the sweep mechanism end-to-end and ship a runnable `bench` command. The full
curated matrix (one phase per language × kind × size) is **phase-03c**; this
phase builds the engine + a single `smoke` fixture, not the breadth.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & routing" → "Benchmark
  vs. telemetry" — *"A small curated benchmark suite (the same phases run by
  each model) gives controlled head-to-head rankings… small models are
  high-variance, so a routing decision needs a minimum sample size, never one
  run."* The sweep is what runs *the same phase* across models.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### Reusable runner surface (`mcp/src/runner.rs`)

`run_phase` and its config are **public** and already accept everything the
sweep needs — a model override, a telemetry dir, a `bench_suite` stamp (added in
phase-03a), and an injectable `test_client` (so the sweep is hermetic-testable):

```rust
// runner.rs:202
pub struct RunPhaseConfig<'a> {
    pub cfg: &'a Config,
    pub phase_doc_path: &'a Path,
    pub repo_path: &'a Path,
    pub standards: &'a str,
    pub model_override: Option<&'a str>,
    pub telemetry_dir: Option<&'a Path>,
    pub bench_suite: Option<&'a str>,
    pub progress: Option<&'a dyn ProgressCallback>,
    pub test_client: Option<&'a dyn AiClient>,   // None → real OpenAiClient
}
// runner.rs:216
pub async fn run_phase(inp: &RunPhaseConfig<'_>) -> rexymcp_executor::error::Result<PhaseResult>
```

`derive_phase_id` is also public (`runner.rs:77`): `phase-01-foo.md` → `"phase-01"`.

### `PhaseStatus` (`executor/src/phase/result.rs:11`)

The terminal status enum. It is `Serialize` with `#[serde(rename_all =
"snake_case")]` → `"complete"` / `"hard_fail"` / `"budget_exceeded"`. **Store it
directly on the outcome type** (derive `Serialize`) rather than restringifying:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus { Complete, HardFail, BudgetExceeded }
```

`PhaseResult.status` (`result.rs:50`) is a `PhaseStatus`.

### CLI subcommands (`mcp/src/main.rs`)

`enum Commands` (`main.rs:22`) defines variants (`Health`, `RunPhase`, `Serve`,
`Status`); each is handled in the `match` at `main.rs:83`. The `Status` handler
(`main.rs:163`) is the closest shape to copy: it loads, calls into a sibling
module, and prints either human text or `--json`. Modules are declared at the
top of `main.rs` (`mod server; mod runner; …`) — add `mod bench;`.

### No recursive-copy helper exists

`grep` confirms there is no `copy_dir`/`WalkDir` in `mcp/src`. Hand-roll a
`std::fs` recursive copy (spec'd below) — **do not** add `walkdir`/`fs_extra`.

### `tempfile` is dev-only in mcp (`mcp/Cargo.toml:22`)

It is under `[dev-dependencies]`. `bench.rs` uses `tempfile::TempDir` in
**production** code, so it must move to `[dependencies]` (authorized below — it
is already a workspace-tree crate via `executor`).

### The workspace manifest (root `Cargo.toml`)

```toml
[workspace]
members = ["executor", "mcp"]
resolver = "2"
```

This phase checks in a fixture crate at `benchmarks/smoke/fixture/` with its own
`Cargo.toml`. A nested package that is neither a member nor excluded makes the
workspace `cargo` gates (`build`/`clippy`/`fmt`/`test`) error or misbehave, so
the fixture dir **must be excluded** from the workspace (authorized below).

## Spec

### Task 0 — Scope the workspace away from the fixture, promote `tempfile` (do this FIRST)

**Gotcha — order matters.** Create the fixture crate's `Cargo.toml` only *after*
these manifest edits, or your own gate runs will choke on a nested package the
workspace doesn't know about.

1. Root `Cargo.toml` — add an `exclude` to `[workspace]`:

   ```toml
   [workspace]
   members = ["executor", "mcp"]
   exclude = ["benchmarks"]
   resolver = "2"
   ```

2. `mcp/Cargo.toml` — move `tempfile = "3"` from `[dev-dependencies]` to
   `[dependencies]` (it stays usable in tests). Leave `async-trait` in
   `[dev-dependencies]`.

### Task 1 — New module `mcp/src/bench.rs`

Declare `mod bench;` in `main.rs` alongside the other module declarations.

**1a. Types** (derive `Serialize` for `--json`):

```rust
use serde::Serialize;
use rexymcp_executor::phase::PhaseStatus;

#[derive(Debug, Clone, Serialize)]
pub struct BenchOutcome {
    pub model: String,
    pub phase_id: String,
    pub status: PhaseStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub suite: String,
    pub outcomes: Vec<BenchOutcome>,
}
```

**1b. `copy_dir_all`** — the standard `std::fs` recursive copy, verbatim:

```rust
use std::path::Path;

/// Recursively copy `src` into `dst` (creating `dst`). Files only + nested
/// dirs; no symlink special-casing (the fixture has none).
pub fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let dst_child = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &dst_child)?;
        } else {
            std::fs::copy(entry.path(), &dst_child)?;
        }
    }
    Ok(())
}
```

**1c. Phase discovery** — `*.md` files in `<suite_dir>/phases`, sorted:

```rust
use std::path::PathBuf;
use rexymcp_executor::error::Result;

fn discover_phase_docs(phases_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut docs: Vec<PathBuf> = std::fs::read_dir(phases_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    docs.sort();
    Ok(docs)
}
```

**1d. The sweep.** Group inputs in a config struct (project idiom — see
`RunPhaseConfig` / `Seams` in `runner.rs:202`, "grouped to stay under clippy's
argument limit"). Copy the fixture into a fresh `TempDir` for **every** run:

```rust
use tempfile::TempDir;
use rexymcp_executor::ai::AiClient;
use rexymcp_executor::config::Config;
use crate::runner::{self, derive_phase_id, RunPhaseConfig};

pub struct BenchConfig<'a> {
    pub cfg: &'a Config,
    pub suite: &'a str,
    pub suite_dir: &'a Path,
    pub models: &'a [String],
    pub telemetry_dir: Option<&'a Path>,
    /// Inject a test client for hermetic tests. `None` → real `OpenAiClient`.
    pub test_client: Option<&'a dyn AiClient>,
}

pub async fn run_bench(inp: &BenchConfig<'_>) -> Result<BenchReport> {
    let fixture = inp.suite_dir.join("fixture");
    let phase_docs = discover_phase_docs(&inp.suite_dir.join("phases"))?;
    let standards =
        std::fs::read_to_string(fixture.join("docs/dev/STANDARDS.md")).unwrap_or_default();

    let mut outcomes = Vec::new();
    for model in inp.models {
        for phase_doc in &phase_docs {
            let tmp = TempDir::new()?;
            let repo = tmp.path().join("repo");
            copy_dir_all(&fixture, &repo)?;

            let result = runner::run_phase(&RunPhaseConfig {
                cfg: inp.cfg,
                phase_doc_path: phase_doc,
                repo_path: &repo,
                standards: &standards,
                model_override: Some(model),
                telemetry_dir: inp.telemetry_dir,
                bench_suite: Some(inp.suite),
                progress: None,
                test_client: inp.test_client,
            })
            .await?;

            outcomes.push(BenchOutcome {
                model: model.clone(),
                phase_id: derive_phase_id(phase_doc),
                status: result.status,
            });
            // `tmp` drops here → the per-run copy is cleaned up.
        }
    }
    Ok(BenchReport { suite: inp.suite.to_string(), outcomes })
}
```

**1e. Human render** — a free function `pub fn format_report(report: &BenchReport)
-> String` that emits the suite name and one line per outcome
(`<model>  <phase_id>  <status>`) plus a one-line summary count. Pin *behavior*
(every outcome appears; the suite name appears), not exact spacing.

### Task 2 — `bench` CLI subcommand (`mcp/src/main.rs`)

Add a variant to `enum Commands`:

```rust
    /// Run a benchmark suite against one or more models
    Bench {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,
        /// Suite name — also the `bench_suite` stamp and the dir under --benchmarks-dir
        #[arg(long)]
        suite: String,
        /// Model under test; repeat for multiple (e.g. --model a --model b)
        #[arg(long = "model")]
        models: Vec<String>,
        /// Root dir containing `<suite>/`
        #[arg(long, default_value = "benchmarks")]
        benchmarks_dir: PathBuf,
        /// Emit the report as JSON instead of a human summary
        #[arg(long)]
        json: bool,
    },
```

Handle it in the `match` (follow the `Status` handler shape at `main.rs:163`):

- Load config via `Config::load_with_env(&config)?`.
- If `models.is_empty()`, `eprintln!` a clear message and
  `std::process::exit(1)` (a sweep needs at least one model).
- `let suite_dir = benchmarks_dir.join(&suite);`
- `let telemetry_dir = cfg.telemetry.dir.clone();`
- Call `bench::run_bench(&bench::BenchConfig { cfg: &cfg, suite: &suite,
  suite_dir: &suite_dir, models: &models, telemetry_dir:
  telemetry_dir.as_deref(), test_client: None }).await?`.
- Print: if `json`, `serde_json::to_string_pretty(&report)` (same
  `unwrap_or_else` fallback the other handlers use); else
  `bench::format_report(&report)`.
- `Ok(())`.

### Task 3 — One minimal checked-in fixture: `benchmarks/smoke/`

A standalone (non-workspace) crate the executor extends, plus one benchmark
phase. Write these files **verbatim**:

**`benchmarks/smoke/fixture/Cargo.toml`:**

```toml
[package]
name = "smoke-fixture"
version = "0.1.0"
edition = "2021"

[dependencies]
```

**`benchmarks/smoke/fixture/src/lib.rs`:**

```rust
//! Smoke benchmark fixture: a trivial crate the model-under-test extends.

/// Returns the sum of two numbers.
pub fn add(a: i64, b: i64) -> i64 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sums() {
        assert_eq!(add(2, 3), 5);
    }
}
```

**`benchmarks/smoke/fixture/docs/dev/STANDARDS.md`** (a short, self-contained DoD
stub for the benchmarked phase — not the full rexyMCP STANDARDS):

```markdown
# Smoke fixture standards

- Implement the phase's Goal; satisfy its Acceptance criteria.
- `cargo build`, `cargo clippy -- -D warnings`, `cargo test`, and
  `cargo fmt --check` all pass.
- New behavior has a unit test (a must-match and, where relevant, a must-NOT).
- No `unwrap`/`expect`/`panic!` in non-test code; no `unsafe`; no `#[allow]`.
```

**`benchmarks/smoke/phases/phase-01-subtract.md`** (a trivial, real phase the
sweep dispatches — it needs `## Goal`, `## Acceptance criteria`, and a `**Tags:**`
line, since `parse_phase_doc` reads those):

```markdown
# Phase 01: add a `subtract` function

**Tags:** language=rust, kind=feature, size=s

## Goal

Add a `subtract(a: i64, b: i64) -> i64` function to `src/lib.rs` returning
`a - b`, mirroring the existing `add`, with a unit test.

## Acceptance criteria

- [ ] `subtract(5, 3)` returns `2`.
- [ ] A unit test covers `subtract`.
- [ ] `cargo test` passes.
```

## Acceptance criteria

- [ ] `mcp/src/bench.rs` exists with `copy_dir_all`, `discover_phase_docs`,
      `run_bench`, `BenchConfig`, `BenchReport`, `BenchOutcome`, `format_report`.
- [ ] `run_bench` copies the fixture into a fresh `TempDir` for **each**
      (model, phase) run — the source fixture is never used as the run repo and
      is unmodified after a sweep.
- [ ] A sweep over N models × M phases emits N×M `bench_suite`-stamped
      `PhaseRun` records (stamp == the suite name; each record's `model` == the
      model it ran under).
- [ ] `discover_phase_docs` returns only `*.md`, sorted; non-`.md` files are
      excluded.
- [ ] `rexymcp bench --config <c> --suite smoke --model a --model b` parses
      (repeated `--model` collected) and runs; `--model`-less invocation exits
      non-zero with a message.
- [ ] Root `Cargo.toml` excludes `benchmarks`; `mcp` depends on `tempfile` in
      `[dependencies]`.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

In `mcp/src/bench.rs` `#[cfg(test)] mod tests` (hermetic — `TempDir` for all
filesystem state, `MockAiClient` for the model, `Config::default()` so the empty
command set means no real `cargo` runs, and `test_client: Some(&mock)` so no
network):

- `copy_dir_all_replicates_nested_tree` — a src with a nested subdir + files →
  dst has the same structure and file contents. (must-match)
- `copy_dir_all_copy_is_independent_of_source` — copy, then modify a file in the
  **dst**; assert the matching **src** file is unchanged. (the isolation
  workhorse — must-NOT-affect-source)
- `discover_phase_docs_returns_only_sorted_markdown` — a `phases/` dir with
  `b.md`, `a.md`, and `notes.txt` → `[a.md, b.md]`; the `.txt` is excluded.
  (must-match + must-NOT)
- `run_bench_stamps_each_model_and_phase` — build a temp suite: `fixture/` with
  `docs/dev/STANDARDS.md` + a `src/lib.rs`, and `phases/` with one phase doc
  (with `## Goal` / `## Acceptance criteria` / `**Tags:**`). `MockAiClient::new(
  vec!["Done.".to_string()])`, models `["m1", "m2"]`, `telemetry_dir` set. Assert:
  `report.outcomes.len() == 2`, both `PhaseStatus::Complete`; read the telemetry
  back via `rexymcp_executor::store::telemetry::read` → 2 records, each
  `bench_suite == Some("smoke".to_string())`, and the set of `model` values is
  `{"m1", "m2"}`. (the core integration must-match)
- `run_bench_leaves_source_fixture_untouched` — snapshot the fixture dir's file
  set before the sweep; assert it is unchanged after (no files leaked into the
  source — guards against running in-place). (isolation at the sweep level)
- `run_bench_empty_phases_yields_no_outcomes` — a suite whose `phases/` holds
  only a non-`.md` file → `report.outcomes` is empty, no error. (graceful
  empty / must-NOT)

In `mcp/src/main.rs` `#[cfg(test)] mod tests` (mirror the existing
`cli_parse_*` tests):

- `cli_parse_bench_collects_repeated_models` — `Cli::try_parse_from(["rexymcp",
  "bench", "--config", "rexymcp.toml", "--suite", "smoke", "--model", "m1",
  "--model", "m2"])` → the `Bench` variant with `suite == "smoke"` and
  `models == ["m1", "m2"]`.

Per `STANDARDS.md` §3: every behavior gets a must-match and a must-NOT (the
non-`.md` exclusion, the source-untouched isolation, the empty-phases case).

## End-to-end verification

`run_bench`'s real artifacts are the `bench` CLI and the checked-in `smoke`
fixture. Verify both without a live model:

1. **CLI surface:** `cargo run -p rexymcp -- bench --help` lists `--suite`,
   `--model`, `--benchmarks-dir`, `--json`. Quote the relevant lines in the
   completion Update Log.
2. **The fixture is a valid standalone crate:** copy it out of the workspace and
   build it — `cp -r benchmarks/smoke/fixture /tmp/smoke-fix && (cd
   /tmp/smoke-fix && cargo build && cargo test)` — and quote the `add_sums`
   test passing. (This proves the fixture compiles on its own, which a live
   `bench` run depends on.)

A full live sweep (`bench` against the real `brain:8000` endpoint) needs a
running model and is **not** part of this phase's hermetic verification — the
user exercises it post-merge, the same boundary phase-03a drew for live runs.

## Authorizations

- [x] May edit root `Cargo.toml` — **only** to add `exclude = ["benchmarks"]` to
      `[workspace]`. No other workspace changes.
- [x] May edit `mcp/Cargo.toml` — **only** to move the existing `tempfile = "3"`
      from `[dev-dependencies]` to `[dependencies]`. No new crates.
- [x] May add the new file `mcp/src/bench.rs` and declare `mod bench;` in
      `mcp/src/main.rs`.
- [x] May add the `bench` subcommand + handler in `mcp/src/main.rs`.
- [x] May add the checked-in fixture under `benchmarks/smoke/`.
- [ ] No other new dependencies. No edits to `clippy.toml` / `rustfmt.toml` /
      `.github/workflows/*` / `docs/architecture.md` / `STANDARDS.md` /
      `WORKFLOW.md`.

## Out of scope

- **The curated breadth** — more than one suite, more than one fixture, more
  than one phase, or the language × kind × size matrix. That is **phase-03c**.
  Build exactly the `smoke` suite with its one `phase-01` here.
- **Per-suite command config** (a suite declaring its own `[commands]`
  independent of the host `rexymcp.toml`) — phase-03c. This phase's runs use the
  host `cfg` as-is; the `smoke` fixture is a Rust crate compatible with a `cargo`
  command set.
- **Per-model endpoints / base URLs** — every model shares the one configured
  `base_url`. Multi-endpoint routing is later.
- **`model: "auto"` routing / `recommend_model`** — phase-04.
- **Touching `mcp/src/server.rs` or the `execute_phase` MCP tool** — `bench` is
  a CLI-only path; do not add a `bench` MCP tool.
- **Parallelism** — run the sweep sequentially. Concurrency is a later
  optimization, not needed for correctness.
- **Retry / resume of a failed benchmark run** — record the terminal status in
  the outcome and move on; a `hard_fail` outcome is valid data, not an error.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
