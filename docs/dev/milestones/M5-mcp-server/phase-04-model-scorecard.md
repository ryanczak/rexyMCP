# Phase 04: model_scorecard — model × tag competency matrix

**Milestone:** M5 — MCP server
**Status:** todo
**Depends on:** M5 phase-02 (done) — same `RexyMcpServer` tool router, `[telemetry] dir` config field already wired. M4 phase-08 (done) — emits the `PhaseRun` records this aggregates. M4 telemetry module — `store::telemetry::{PhaseRun, Gates, read}` already `Serialize+Deserialize`.
**Estimated diff:** ~400 lines (scorecard module + handler + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add **`model_scorecard`** — the fourth MCP tool. Reads the cross-project
`phase_runs.jsonl` store and returns the **model × tag competency matrix**: for
each (model, tag) bucket, the aggregated gates pass rate, reliability metrics
(parse-failure / repair / tool-success / verifier-retry means), efficiency
(turns / wall-clock), escalation rate, and (where the architect has filled it
in) the supervision metrics (`approved_first_try_rate`,
`bounces_to_approval_mean`).

This is the **read** end of the M7 routing data pipeline. M7 will *use* this
matrix to pick a model per phase tag-set; M5 phase-04 just exposes it so the
architect can read it directly (`model_scorecard {}` → see the matrix; with a
`tags` filter → drill in on a tag combo). No routing here — that's M7. No
writes — `append` already lives in `store::telemetry` and runs via the loop.

## Architecture references

- `docs/architecture.md` — "Model effectiveness metrics & routing" (the
  `PhaseRun` schema; the model × tag matrix; the rationale for `bounces_to_approval`
  as "the single most telling number"); Layer 2 (`model_scorecard` — args:
  optional `tags` filter); Status §M5 ("Also exposes ... `model_scorecard`
  which aggregates the `PhaseRun` telemetry").
- M5 README Notes — "Telemetry dir is cross-project" (resolves via
  `cfg.telemetry.dir`).
- M4: `store::telemetry::{PhaseRun, Gates, GenerationParams, read}`; the
  store file is `<telemetry_dir>/phase_runs.jsonl` (append-only JSONL, forgiving
  reader: missing → empty, malformed lines skipped).
- M5 phase-02: the `cfg.telemetry.dir` config field; `pub(crate)` inner-fn
  factoring pattern.
- M5 phase-03: the `LogQueryOutput { records, truncated }` wrapping precedent —
  **not** repeated here because the scorecard's output schema is small and
  mcp-owned (see Adaptation 1).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README Notes (esp. "Telemetry dir
   is cross-project" and "Output capping is the boundary's whole point").
2. Read this entire phase doc.
3. Confirm M5 phase-03 is `done`; `cfg.telemetry.dir: Option<PathBuf>` is on
   `Config` (phase-02); `store::telemetry::{PhaseRun, Gates, read}` are public
   and `Serialize+Deserialize`; the store file convention is
   `<telemetry_dir>/phase_runs.jsonl`.
4. **No `executor/` edit is needed in this phase** — all telemetry types
   already derive what we need.

## Spec

### 1. New module — `mcp/src/scorecard.rs`

Pure aggregation over `&[PhaseRun]`. Declared `mod scorecard;` in
`mcp/src/main.rs`. No I/O — `read_session_log`-style separation: the handler
reads the file, the module aggregates the slice.

```rust
use rexymcp_executor::store::telemetry::PhaseRun;
use schemars::JsonSchema;
use serde::Serialize;

/// One row of the model × tag matrix.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScorecardRow {
    pub model: String,
    pub tag: String,
    pub n_runs: usize,
    /// Fraction of runs where every configured gate (fmt/build/lint/test)
    /// reported `Some(true)`. A `None` gate counts as a non-pass for the
    /// row's purposes — see Adaptation 3.
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
    pub repairs_per_call_mean: f64,
    pub tool_success_rate_mean: f64,
    pub verifier_retries_mean: f64,
    pub turns_mean: f64,
    pub wall_clock_s_mean: f64,
    /// Fraction of runs with `escalated == true`.
    pub escalation_rate: f64,
    /// Supervision metrics — derived only from runs whose `architect_verdict`
    /// is `Some`. `n_with_verdict` is the supervision sample size for this
    /// bucket; the two `*_rate` / `_mean` fields below are computed over
    /// those runs only. `None` when `n_with_verdict == 0`.
    pub n_with_verdict: usize,
    pub approved_first_try_rate: Option<f64>,
    pub bounces_to_approval_mean: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct ScorecardFilter<'a> {
    /// Restrict runs to those whose `tags` contains **all** of these tags
    /// (set intersection). Empty filter = no restriction.
    pub tags: &'a [String],
    /// Restrict to one model. `None` = all models.
    pub model: Option<&'a str>,
    /// Drop output rows with `n_runs < min_runs`. `0` = no minimum.
    pub min_runs: usize,
}

pub fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardRow>;
```

**Aggregation algorithm:**

1. **Filter runs:** keep `r` iff
   - `filter.model.is_none() || r.model == model`, **and**
   - every tag in `filter.tags` appears in `r.tags`.
2. **Explode by tag:** for each filtered run `r` and each `tag` in `r.tags`,
   contribute one observation to bucket `(r.model, tag)`. (A single run with
   tags `["language=rust", "kind=feature", "size=m"]` contributes to three
   buckets.)
3. **Aggregate per bucket:**
   - `n_runs` = count of observations
   - `gates_pass_rate` = (count of runs where `gates.fmt == Some(true) &&
     gates.build == Some(true) && gates.lint == Some(true) && gates.test ==
     Some(true)`) / `n_runs`. *All four required-`Some(true)*. Adaptation 3
     explains why.
   - `*_mean` fields = arithmetic mean of the corresponding `PhaseRun` field
   - `escalation_rate` = (count with `escalated == true`) / `n_runs`
   - `n_with_verdict` = count of runs where `architect_verdict.is_some()`
   - `approved_first_try_rate` = (count with `architect_verdict == Some("approved_first_try")`)
     / `n_with_verdict`, or `None` when `n_with_verdict == 0`
   - `bounces_to_approval_mean` = mean of `bounces_to_approval` over runs where
     it is `Some`, or `None` when no such runs in the bucket
4. **Drop low-sample buckets:** rows with `n_runs < filter.min_runs` are
   excluded.
5. **Sort:** by `(tag asc, n_runs desc, model asc)` — alphabetically by tag,
   then highest-evidence first within tag, with model as tie-break for
   determinism.

Bucket key is `(model: String, tag: String)`. Use a `BTreeMap<(String,
String), Accumulator>` for deterministic iteration (or aggregate into a
`HashMap` then sort — either is fine; the **output order** is what's pinned).

### 2. Per-tool result cap

Add to `scorecard.rs`:

```rust
pub const MAX_ROWS: usize = 500;
```

`aggregate` either always returns ≤ `MAX_ROWS` rows (truncating after sort), or
returns all rows and the *handler* truncates — pick one place and stick with
it. Recommended: **handler-side truncation**, so `aggregate` stays pure and the
truncation flag is at the same level as the cap.

### 3. Tool handler — extend `mcp/src/server.rs`

Same `pub(crate)` inner-fn pattern as phases 02–03:

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelScorecardParams {
    /// Tags the run must contain (AND-ed). Empty = no filter.
    pub tags: Option<Vec<String>>,
    /// Restrict to one model. `None` = all models.
    pub model: Option<String>,
    /// Drop buckets with fewer than this many runs. `None` = 0.
    pub min_runs: Option<usize>,
    /// Override the cross-project `phase_runs.jsonl` path. `None` = resolve
    /// from `cfg.telemetry.dir`.
    pub telemetry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelScorecardOutput {
    pub rows: Vec<ScorecardRow>,
    pub total_runs_considered: usize,
    /// True iff the row count was clipped by `MAX_ROWS`.
    pub truncated: bool,
}

pub(crate) fn model_scorecard_inner(
    config_path: &Path,
    params: &ModelScorecardParams,
) -> Result<ModelScorecardOutput, String>;
```

Handler logic:

1. Load `Config` from `config_path` (same as the other handlers).
2. **Resolve the telemetry file path:**
   - If `params.telemetry_path.is_some()` → use that.
   - Else if `cfg.telemetry.dir.is_some()` → `<dir>/phase_runs.jsonl`.
   - Else → return `Err("telemetry disabled: cfg.telemetry.dir not set and no
     telemetry_path provided".to_string())`. This is a usage error from the
     caller, not a runtime failure.
3. `let runs = store::telemetry::read(&path).map_err(|e| e.to_string())?;`
   (forgiving — missing file → `Ok(empty)`, malformed lines skipped).
4. Build the `ScorecardFilter` from `params.tags` (default empty),
   `params.model.as_deref()`, `params.min_runs.unwrap_or(0)`.
5. `let total_runs_considered = runs.len();` — *before* aggregation, so Claude
   sees the corpus size even when filters drop everything.
6. `let mut rows = scorecard::aggregate(&runs, &filter);`
7. **Truncate**: if `rows.len() > MAX_ROWS`, set `truncated = true` and
   `rows.truncate(MAX_ROWS)`. Else `truncated = false`.
8. Return `Ok(ModelScorecardOutput { rows, total_runs_considered, truncated })`.

Register as `#[rmcp::tool(description = "…")]` `model_scorecard` on
`RexyMcpServer`, mirroring the phase-03 tool methods.

### 4. `mcp/src/main.rs` — minimal

Declare `mod scorecard;` if not already present (it won't be). No new
subcommand — `model_scorecard` is pull-not-push debugging; the existing
`serve` subcommand exposes it via MCP.

## Adaptations / decisions

1. **`ScorecardRow` derives `JsonSchema` directly** — no `serde_json::Value`
   wrapper. The schema tree is small (one mcp-owned struct of primitives),
   the rule that matured across phases 02–03 says **derive when small, wrap
   when large**, and deriving gives Claude a typed schema for the matrix.
2. **No path confinement on `telemetry_path`** — same rationale as phase-03's
   `log_path` (architect is trusted; `Scope` confines the *model*).
3. **`gates_pass_rate` requires all four `Some(true)`** — a `None` gate (the
   command not configured for that project) counts against the pass rate. This
   is deliberate: in cross-project aggregation, we want *"the gates passed"* to
   mean *"the configured project's full check passed"*, not *"the gates we
   bothered to configure passed"*. A project that doesn't run `lint` is less
   well-tested than one that does, and the matrix should reflect that without
   needing per-project normalization. Adjustable later if dogfood shows this is
   too strict — for now, conservative is correct.
4. **Tags = `&[String]` with AND semantics** (intersection). OR was tempting
   but adds complexity (caller can issue multiple queries for OR). Document
   AND in the tool description.
5. **No regex / glob on tags** — exact-match only. Caller can iterate.
6. **No pagination/offset** — `min_runs` + tag filter + `model` filter cover
   the use cases; `MAX_ROWS = 500` is well above typical (tens to low
   hundreds) of (model, tag) buckets.
7. **`telemetry_path` override** — present primarily for tests (and the manual
   case where the architect points at a non-default store). `None` falls back
   to `cfg.telemetry.dir` per § 3 step 2.

## Acceptance criteria

- [ ] `mcp/src/scorecard.rs` exists; `mod scorecard;` is wired in
      `mcp/src/main.rs`; `ScorecardRow`, `ScorecardFilter`, `MAX_ROWS`, and
      `aggregate` are reachable.
- [ ] `ScorecardRow` derives `Serialize + JsonSchema`; **no `executor/` edit**
      (verify via `git diff --stat HEAD~1 HEAD -- executor/`).
- [ ] **No new dependency** (verify via `git diff -- mcp/Cargo.toml executor/Cargo.toml`).
- [ ] `aggregate` correctness:
  - filter by `model` → only that model contributes
  - filter by `tags` (AND) → only runs containing **all** filter tags
    contribute
  - explode-by-tag → a single run with `["a", "b"]` produces observations in
    `(model, "a")` and `(model, "b")`
  - `gates_pass_rate` is 1.0 iff every gate is `Some(true)` on every run in
    the bucket; 0.0 iff no run has all four `Some(true)`; intermediate fractions
    correctly computed
  - `*_mean` fields are arithmetic means of the matching `PhaseRun` fields
  - `escalation_rate` = fraction with `escalated == true`
  - `n_with_verdict` counts runs with `architect_verdict.is_some()`
  - `approved_first_try_rate` is `None` when `n_with_verdict == 0`; otherwise
    fraction of `architect_verdict == Some("approved_first_try")` over
    verdict-present runs
  - `bounces_to_approval_mean` is `None` when no run in the bucket has
    `bounces_to_approval.is_some()`; otherwise the mean over those
  - `min_runs` drops low-sample buckets
  - Output is sorted by `(tag asc, n_runs desc, model asc)`
- [ ] `model_scorecard` tool registered on `RexyMcpServer` with name exactly
      `"model_scorecard"`; thin macro wrapper over `model_scorecard_inner`
      (phase-02 pattern).
- [ ] `model_scorecard_inner`:
  - `telemetry_path` override used when `Some`
  - else `cfg.telemetry.dir` resolved → `<dir>/phase_runs.jsonl`
  - both unset → `Err(String)` (usage error, not runtime)
  - missing file at the resolved path → empty rows, `total_runs_considered = 0`,
    `truncated = false` (not an `Err`)
  - `total_runs_considered` reflects pre-aggregation corpus size
  - `truncated = true` iff post-aggregation row count was clipped at `MAX_ROWS`
- [ ] **Handler success-path test** (the phase-03 hardened requirement): write
      a `phase_runs.jsonl` fixture with several runs spanning two models and
      multiple tags into a `TempDir`, call `model_scorecard_inner` with both
      the explicit `telemetry_path` and the config-resolved path, assert the
      returned rows match expected aggregates. Not just error paths.
- [ ] Error paths covered: telemetry disabled (no `dir`, no `telemetry_path`),
      malformed JSONL lines (read should skip them — assert the matrix still
      builds from the survivors).
- [ ] No `#[allow]`; no `unwrap()` / `expect()` / `panic!()` in production
      paths; no Rexy phase references.
- [ ] **Calibration carry-forward (mandatory):** every scope deviation
      declared in "Notes for review", even defensible ones. Phase-03 had zero
      deviations; if this phase introduces any, they go in the Notes verbatim.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. In `mcp/src/scorecard.rs` `#[cfg(test)] mod tests`:

- **Filtering:**
  - `model` filter — only matching runs contribute
  - `tags` AND filter — only runs containing all tags contribute
  - combined `model` + `tags`
  - empty filter — every run × tag contributes
- **Aggregation:**
  - `gates_pass_rate` — fixture with 3 all-pass + 1 one-fail → 0.75
  - `gates_pass_rate` with a `None` gate (e.g. lint absent) → counts as fail
    (Adaptation 3 — pin the negative case)
  - `*_mean` fields — fixture with known values, assert means
  - `escalation_rate` — fixture with mixed `escalated`
  - `n_with_verdict` + `approved_first_try_rate` — fixtures: zero verdicts
    (None), partial verdicts (rate over verdict-present subset), all verdicts
  - `bounces_to_approval_mean` — None case, partial case
  - `min_runs` — exclude small buckets
- **Sort order:**
  - Multiple tags + multiple models → assert tuple ordering
- **Edge cases:**
  - Empty runs slice → empty Vec
  - One run, multiple tags → multiple rows for one model

In `mcp/src/server.rs` `#[cfg(test)] mod tests` (extend):

- **`model_scorecard_inner` success path** — TempDir + fixture
  `phase_runs.jsonl` + `Config` with `cfg.telemetry.dir = Some(tempdir)`;
  invoke without `telemetry_path` (config-resolved) → expected rows; invoke
  with explicit `telemetry_path` → same rows.
- **`telemetry_path` override** — point at a different fixture, assert it
  takes precedence over `cfg.telemetry.dir`.
- **Telemetry disabled** — `cfg.telemetry.dir = None`, no `telemetry_path`
  param → `Err(String)`.
- **Missing file** — config points at a nonexistent dir → `Ok` with empty
  rows, `total_runs_considered = 0`.
- **Malformed JSONL** — write a file with one good + one garbage line; assert
  the good record contributes, the garbage is skipped.
- **`truncated` flag** — synthesize a file with > `MAX_ROWS` unique (model,
  tag) combinations; assert `rows.len() == MAX_ROWS` and `truncated == true`.

## End-to-end verification

> Not applicable yet — same as phases 02 and 03. Handler logic exercised by
> unit tests over `TempDir` fixtures; rmcp transport is M6 dogfood.

## Authorizations

- [x] **May create** `mcp/src/scorecard.rs`; **may modify** `mcp/src/server.rs`
      (one new param struct, one new output struct, one new inner fn, one new
      `#[rmcp::tool]` method + handler tests), `mcp/src/main.rs` (declare
      `mod scorecard;`).
- [ ] **No new dependencies.** `mcp` already has `serde`, `serde_json`,
      `schemars`, `rmcp`, `tokio`.
- [ ] **No `executor/` edits.** All telemetry types already derive
      `Serialize+Deserialize`. If a genuine need arises, **stop and file a
      blocker.**
- [ ] May **NOT** add progress notifications (phase-05) or roots corroboration
      (phase-06).
- [ ] May **NOT** modify any other phase doc, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, `AGENTS.md`, `mcp/src/runner.rs`,
      `mcp/src/cap.rs`, `mcp/src/log_query.rs`, or `mcp/src/server.rs` beyond
      the four additions named above.
- [ ] **Calibration carry-forward from phases 02–03 (mandatory):** declare
      every scope deviation in "Notes for review", even defensible ones. The
      phase-03 verdict singled out "zero deviations" as the cleanest M5 phase
      — keep the bar.

## Out of scope

- **Routing logic** (mapping a phase's tags to the best-scoring model) — M7.
  Phase-04 is the read API; M7 builds the policy on top.
- **Writing supervision fields back** (`bounces_to_approval`,
  `architect_verdict`, `bugs_filed`, `warnings`) — also M7 / a future phase.
  Phase-04 reads what's already there; supervision fields are mostly `None`
  today (M4 phase-08 emits the record; nothing writes the verdict back yet).
- **Progress notifications / roots corroboration** — phases 05–06.
- **Regex / glob filters on `tags`** — exact-match only (Adaptation 5).
- **Pagination cursors** — `MAX_ROWS = 500` + filters cover it (Adaptation 6).
- **Per-row capping of field byte budgets** — rows are bounded primitives, no
  long strings; `MAX_ROWS` is the only cap needed.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
