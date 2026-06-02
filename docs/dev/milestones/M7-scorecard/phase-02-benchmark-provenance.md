# Phase 02: benchmark provenance on PhaseRun + scorecard source filter

**Milestone:** M7 — Model scorecard & routing
**Status:** done
**Depends on:** phase-01 (done). The telemetry store, `PhaseRun`, the scorecard aggregation, and the `model_scorecard` MCP tool all exist (M4/M5); this phase extends them.
**Estimated diff:** ~200 lines (one field + serde default + a filter enum + filter logic + MCP param mapping + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Make a `PhaseRun` record carry **provenance**: was it a normal production phase,
or a controlled **benchmark** run (and of which suite)? Then teach the scorecard
to filter on it. This is the foundational first slice of the M7 benchmark suite
(README exit criterion 2): benchmark records share the production telemetry store,
so they must be distinguishable, or the scorecard conflates *controlled*
head-to-head data with *confounded* production data — exactly the distinction
`docs/architecture.md` § "Benchmark vs. telemetry" says must hold.

This phase does **not** build the benchmark runner or any benchmark fixtures —
those are phase-03+. It builds the data model + the read-side filter they depend
on.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Benchmark vs. telemetry" — *"Both emit the same
  `PhaseRun` schema; small models are high-variance, so a routing decision needs
  a minimum sample size, never one run."* The provenance field is what lets one
  schema + one store serve both readers.
- `docs/architecture.md` § "Model effectiveness metrics & routing" — the
  `PhaseRun` schema and the `model_scorecard` matrix this feeds.
- `docs/dev/milestones/M7-scorecard/README.md` — exit criterion 2 (benchmark
  suite) and the phase plan.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### `PhaseRun` (`executor/src/store/telemetry.rs:36`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseRun {
    pub ts: u64,
    pub model: String,
    pub generation_params: GenerationParams,
    pub phase_id: String,
    pub tags: Vec<String>,
    pub status: String,
    pub escalated: bool,
    pub gates: Gates,
    pub parse_failure_rate: f64,
    pub repairs_per_call: f64,
    pub verifier_retries: usize,
    pub tool_success_rate: f64,
    pub turns: usize,
    pub wall_clock_s: f64,
    pub tokens: TokenBreakdown,
    pub warnings: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub bounces_to_approval: Option<u32>,
    pub architect_verdict: Option<String>,
}
```

**The store already contains real records written before this phase** (e.g. the
phase-01 run). `read()` (`telemetry.rs:83`) parses each line with
`serde_json::from_str::<PhaseRun>(l).ok()` inside a `filter_map`, so **a record
missing a newly-added field is silently dropped** unless the field has a serde
default. This is the load-bearing constraint of this phase (see the negative test
in the Test plan).

### Scorecard filter (`mcp/src/scorecard.rs:35`)

```rust
#[derive(Debug, Clone, Default)]
pub struct ScorecardFilter<'a> {
    /// Restrict runs to those whose `tags` contains **all** of these tags.
    pub tags: &'a [String],
    /// Restrict to one model. `None` = all models.
    pub model: Option<&'a str>,
    /// Drop output rows with `n_runs < min_runs`. `0` = no minimum.
    pub min_runs: usize,
}
```

The filter is applied at the top of `aggregate()` (`scorecard.rs:73`):

```rust
for run in runs {
    if let Some(model) = filter.model
        && run.model != model
    {
        continue;
    }
    if !filter.tags.is_empty() && !filter.tags.iter().all(|t| run.tags.contains(t)) {
        continue;
    }
    for tag in &run.tags { /* … accumulate … */ }
}
```

### Production emit site (`executor/src/agent/mod.rs:1189`)

```rust
let run = PhaseRun {
    ts: now,
    model: deps.model.to_string(),
    // …
    architect_verdict: None,
};
let _ = telemetry::append(dir, &run);
```

This is the **only production writer**. Every record it writes is a production
run, so it sets the new field to the production value.

### The MCP tool params (`mcp/src/server.rs:247`)

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelScorecardParams {
    pub tags: Option<Vec<String>>,
    pub model: Option<String>,
    pub min_runs: Option<usize>,
    pub telemetry_path: Option<String>,
}
```

Filter assembly (`server.rs:292`):

```rust
let filter = scorecard::ScorecardFilter {
    tags: params.tags.as_deref().unwrap_or(&[]),
    model: params.model.as_deref(),
    min_runs: params.min_runs.unwrap_or(0),
};
```

### `PhaseRun` construction sites

Three total — all must set the new field:
1. `executor/src/agent/mod.rs:1189` (production — sets the production value).
2. `executor/src/store/telemetry.rs:102` — the `sample()` test helper.
3. `mcp/src/scorecard.rs:180` — the `make_run()` test helper.

## Spec

### Task 1 — Add the `bench_suite` field to `PhaseRun`

In `executor/src/store/telemetry.rs`, add to `PhaseRun` (place it adjacent to the
identity fields, after `tags`):

```rust
    /// Provenance. `None` = a normal production phase run. `Some(name)` = a
    /// controlled benchmark run belonging to suite `name`. Serde-defaults to
    /// `None` so records written before this field existed still deserialize
    /// (as production).
    #[serde(default)]
    pub bench_suite: Option<String>,
```

The `#[serde(default)]` is **mandatory** — without it, every pre-existing record
in `phase_runs.jsonl` fails to deserialize and is dropped by `read()`'s
`filter_map(...ok())`.

Update the `sample()` test helper (`telemetry.rs:102`) to set
`bench_suite: None`.

### Task 2 — Production emit sets `bench_suite: None`

In `executor/src/agent/mod.rs:1189`, add `bench_suite: None,` to the `PhaseRun`
construction. The production loop always emits production runs. **Do not** thread
a benchmark-suite value through `LoopDeps` in this phase — the writer of a
non-`None` value is the phase-03 benchmark runner; adding the plumbing now would
be wired-in state with no caller (see Out of scope).

### Task 3 — Add `SourceFilter` to the scorecard

In `mcp/src/scorecard.rs`, add an enum and a field on `ScorecardFilter`:

```rust
/// Which provenance of run the scorecard should aggregate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SourceFilter {
    /// Both production and benchmark runs contribute.
    #[default]
    Any,
    /// Only production runs (`bench_suite == None`).
    ProductionOnly,
    /// Only benchmark runs from this exact suite (`bench_suite == Some(name)`).
    Suite(String),
}
```

Add to `ScorecardFilter`:

```rust
    /// Restrict by run provenance. Default `Any`.
    pub source: SourceFilter,
```

In `aggregate()`, add the source check alongside the existing model/tags guards
(before the `for tag in &run.tags` accumulation):

```rust
match &filter.source {
    SourceFilter::Any => {}
    SourceFilter::ProductionOnly => {
        if run.bench_suite.is_some() {
            continue;
        }
    }
    SourceFilter::Suite(name) => {
        if run.bench_suite.as_deref() != Some(name.as_str()) {
            continue;
        }
    }
}
```

Update the `make_run()` test helper (`scorecard.rs:180`) to set
`bench_suite: None` (keeping every existing scorecard test a production run, so
their expectations are unchanged under the default `Any` filter).

### Task 4 — Expose the source filter through the MCP tool

In `mcp/src/server.rs`, add two optional params to `ModelScorecardParams`:

```rust
    /// Restrict to benchmark runs of this suite. Wins over `production_only`
    /// when both are set.
    pub bench_suite: Option<String>,
    /// When `Some(true)` (and `bench_suite` unset), restrict to production runs.
    pub production_only: Option<bool>,
```

In `model_scorecard_inner`, derive the `source` before building the filter, with
`bench_suite` taking precedence:

```rust
let source = if let Some(suite) = params.bench_suite.clone() {
    scorecard::SourceFilter::Suite(suite)
} else if params.production_only == Some(true) {
    scorecard::SourceFilter::ProductionOnly
} else {
    scorecard::SourceFilter::Any
};
```

Then set `source` in the `ScorecardFilter` literal. Existing `ModelScorecardParams`
constructions in the server tests must add `bench_suite: None, production_only:
None,` (they currently set `tags`/`model`/`min_runs`/`telemetry_path`).

Extend the tool's `description` string (`server.rs:357`) with one clause noting
the new filter, e.g. append: `Filter by run provenance with production_only or
bench_suite (bench_suite wins).`

## Acceptance criteria

- [ ] `PhaseRun` has `#[serde(default)] pub bench_suite: Option<String>`.
- [ ] A `phase_runs.jsonl` line written **without** a `bench_suite` field
      deserializes via `telemetry::read` as a record with `bench_suite == None`
      (back-compat — the pinned negative case).
- [ ] `SourceFilter { Any, ProductionOnly, Suite(String) }` exists; `Any` is the
      `Default`.
- [ ] `aggregate()` with `SourceFilter::ProductionOnly` excludes every run whose
      `bench_suite.is_some()`.
- [ ] `aggregate()` with `SourceFilter::Suite("x")` includes only runs with
      `bench_suite == Some("x")` — excludes production runs **and** runs from
      other suites (`Some("y")`).
- [ ] `aggregate()` with `SourceFilter::Any` (default) includes both production
      and benchmark runs (existing scorecard tests still pass unchanged).
- [ ] The production emit site sets `bench_suite: None`.
- [ ] `ModelScorecardParams` has `bench_suite` and `production_only`; `bench_suite`
      takes precedence when both are set.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

In `executor/src/store/telemetry.rs` `#[cfg(test)] mod tests`:

- `record_without_bench_suite_field_deserializes_as_production` — write a JSON
  line that is a valid `PhaseRun` **minus** the `bench_suite` key (build it by
  serializing a `sample()` then removing the key, or hand-write the object), pass
  it through `read()` (or `serde_json::from_str`), assert one record returned with
  `bench_suite == None`. **This is the load-bearing back-compat test** — it must
  fail if the `#[serde(default)]` is dropped.
- `round_trip_preserves_some_bench_suite` — a `PhaseRun` with
  `bench_suite: Some("smoke".into())` survives `to_string` → `from_str` with the
  value intact.

In `mcp/src/scorecard.rs` `#[cfg(test)] mod tests` (extend `make_run` with a
`bench_suite` argument, or add a sibling helper — executor's structural call):

- `production_only_excludes_benchmark_runs` — a production run and a
  `Some("smoke")` run with the same tag; `ProductionOnly` yields a bucket with
  `n_runs == 1` (the production one).
- `suite_filter_excludes_production_and_other_suites` — three runs same tag:
  `None`, `Some("smoke")`, `Some("perf")`; `Suite("smoke")` yields `n_runs == 1`.
- `any_source_includes_all` — the same three runs under `SourceFilter::Any`
  (default) yield `n_runs == 3` in that tag bucket.

In `mcp/src/server.rs` `#[cfg(test)] mod tests`:

- `scorecard_params_bench_suite_takes_precedence` — a `ModelScorecardParams` with
  both `bench_suite: Some("smoke")` and `production_only: Some(true)` resolves to
  `SourceFilter::Suite("smoke")`. (Assert via `model_scorecard_inner` over a
  fixture store containing a production run + a `smoke` run + a `perf` run: the
  result rows reflect only the `smoke` run. Reuse the existing telemetry-fixture
  pattern from the surrounding scorecard tests.)

Per `STANDARDS.md` §3: every new filter branch gets a test with both a
must-match and a must-NOT-match example (the negative cases are the point).

## End-to-end verification

> Not applicable — this phase ships a telemetry-schema + aggregation change with
> no new CLI surface or runtime artifact a user loads directly. The hermetic
> tests above (especially the serde back-compat test against a field-less record,
> which mirrors the real records already in `phase_runs.jsonl`) are the
> definitive verification. The benchmark **runner** that exercises this field
> end-to-end against a live model lands in phase-03.

## Authorizations

- [x] May edit `executor/src/store/telemetry.rs` — add field + helper + tests.
- [x] May edit `executor/src/agent/mod.rs` — set `bench_suite: None` at the emit
      site.
- [x] May edit `mcp/src/scorecard.rs` — add `SourceFilter`, filter logic, tests.
- [x] May edit `mcp/src/server.rs` — add the two params, the mapping, tests, and
      the tool description clause.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.

## Out of scope

- **The benchmark runner** (`rexymcp bench` or equivalent) — phase-03. It is the
  writer of non-`None` `bench_suite` values; this phase only adds the field and
  the production (`None`) writer.
- **Threading `bench_suite` through `LoopDeps` / `emit_phase_run`** so a live run
  can be stamped with a suite — phase-03, where the runner that needs it lives.
  Adding it now is wired-in state with no caller (`WORKFLOW.md` § "Derive
  intentionally").
- **Benchmark fixtures** (a frozen target repo + benchmark phase docs) — phase-03+.
- **Routing policy** (`recommend_model`) — phase-04.
- **A separate benchmark store file.** The architecture pins a *shared* store;
  this phase keeps one store and distinguishes by field, not by file.
- **Backfilling provenance onto the existing production records** — they are
  already production; `None` via serde default is correct, no migration needed.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-06-01

The previous dispatch **hard-failed on a build error** (`VerifierFailurePersistent`,
3 consecutive identical failures) and never reached the Test plan. The working
tree already holds most of the change — **continue against it, do not start over.**
What is already on disk and correct:

- `executor/src/store/telemetry.rs` — `#[serde(default)] pub bench_suite:
  Option<String>` is on `PhaseRun` (Task 1).
- `executor/src/agent/mod.rs` — the production emit sets `bench_suite: None`
  (Task 2).
- `mcp/src/scorecard.rs` — `SourceFilter` enum, the `source` field on
  `ScorecardFilter`, and the `aggregate()` match arm are present (Task 3).
- `mcp/src/server.rs` — the `source` derivation in `model_scorecard_inner`
  (`params.bench_suite` / `params.production_only`) is present.

**The single blocker:** `model_scorecard_inner` reads `params.bench_suite`
(`server.rs:292`) and `params.production_only` (`server.rs:294`), but those two
fields were **never added to the `ModelScorecardParams` struct** (`server.rs:247`).
The build fails with `E0609` on both. **Do this edit FIRST**, then confirm the
build, then finish the rest:

1. **(Unblocks the build)** Add the two fields to `ModelScorecardParams` per
   Task 4 — `pub bench_suite: Option<String>` and `pub production_only:
   Option<bool>` with the doc comments shown in the spec.
2. Append the description clause to the `model_scorecard` tool (`server.rs` tool
   `description = ...`, the `model_scorecard` attribute, not `get_turn`) per
   Task 4's last paragraph.
3. Update **every existing** `ModelScorecardParams { … }` construction in
   `server.rs` tests to add `bench_suite: None, production_only: None,` (they
   currently set only `tags`/`model`/`min_runs`/`telemetry_path` — they will
   fail `E0063` once the fields exist).
4. Confirm the `make_run()` helper (`scorecard.rs:200`) and the `sample()`
   helper (`telemetry.rs`) each set `bench_suite: None`; same `E0063` risk.
5. **Write the Test plan in full** — none of it was reached last run. All five
   named tests across `telemetry.rs`, `scorecard.rs`, and `server.rs`,
   including the load-bearing back-compat test
   `record_without_bench_suite_field_deserializes_as_production` and every
   negative case.

Run the build after step 1 to clear the blocker, then proceed; run the full
command set (`fmt --check`, `build`, `clippy`, `test`) before returning.

### Update — 2026-06-01 18:37 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** first failure, single-class build blocker (two missing struct
fields the consumer edit depended on) that a tighter, ordered spec prevents —
takeover would forfeit the model-scorecard data point.

### Completion (reconstructed by reviewer) — 2026-06-01

The executor flipped `Status:` to `review` but its completion Update Log entry
never landed — the `execute_phase` SSE stream stalled (90s no-data timeout on a
long local-LLM inference) during finalization, so the result payload + log write
were truncated. The status flip persisted; the narrative did not. Command set
re-run independently by the reviewer (all green):

- `cargo fmt --all --check` — clean.
- `cargo build` — clean, zero warnings.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean.
- `cargo test` — **679 passed, 0 failed, 2 ignored** (544 executor + 135 mcp).

All six Test-plan tests present and green, including the load-bearing back-compat
case `record_without_bench_suite_field_deserializes_as_production` and the
precedence case `scorecard_params_bench_suite_takes_precedence`.

**End-to-end verification:** N/A per the phase doc — telemetry-schema +
aggregation change with no new CLI surface; the hermetic serde back-compat test
against a field-less record is the definitive verification.

### Review verdict — 2026-06-01

- **Verdict:** approved_after_1
- **Bounces:** 1 — the initial `hard_fail` (`VerifierFailurePersistent`: the
  `source` mapping in `model_scorecard_inner` referenced `ModelScorecardParams`
  fields that were never added → `E0609` ×2). Resolved by refined re-dispatch
  (no review bug filed). All four Spec tasks and all eight acceptance criteria
  verified met on re-run.
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — diff is exactly the field + serde default +
  `SourceFilter` + filter logic + MCP param mapping + tests, as specced. No
  production `unwrap`/`expect`/`panic`, no `unsafe`, no `#[allow]`/`#[ignore]`,
  no `TODO`/`dbg!`/`println!`.
- **Calibration:** two consecutive `execute_phase` SSE stalls (90s no-data
  timeout) on long local-LLM inference truncated executor finalization — the
  first lost the result entirely (continued on re-dispatch), the second lost
  only the completion-log write (work intact, reviewer reconstructed above).
  Infrastructure pattern, not an executor defect; two occurrences = a trend to
  watch, not yet a fold.
