# Phase 08d: Aggregate context-efficiency into the model × settings scorecard

**Milestone:** M10 — Context optimization
**Status:** review
**Depends on:** phase-08a (`PhaseRun.context_efficiency` capture — done),
phase-08c (the same two means on the model × tag `ScorecardRow` — done; this
phase mirrors it on the sibling settings row)
**Estimated diff:** ~110 lines (incl. tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Make M10's context-efficiency signal **comparable across sampling settings** in
the **model × settings** scorecard (the `SettingsScorecardRow` returned by the
`rexymcp scorecard` CLI). Add the same two aggregate fields phase-08c added to
the model × tag row — **`peak_context_pct_mean`** (mean peak context-window
utilization) and **`tokens_reclaimed_mean`** (mean total tokens reclaimed by all
four M10 levers) — computed over the runs in each (model, settings) bucket that
actually carry context telemetry, and surface them as two new columns in the
`rexymcp scorecard` human table.

Phase-08c rolled the signal up on the model × tag axis (MCP-tool-only, one struct
literal in one file). This phase does the **model × settings** axis so a user can
compare, e.g., "Qwen at `temp=0.2` averages 71% peak context and reclaims ~9k
tokens/run" against "Qwen at `temp=0.7`". It is the sibling half deliberately
split out from 08c because it carries **three `SettingsScorecardRow` struct
literals across two files** plus a CLI renderer change, whereas 08c touched
exactly one literal in one file. The fields, predicate, and boundary semantics
are **identical** to 08c — this phase reuses them verbatim for consistency.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `model × settings` slice answers "which settings work best for this model?";
  the `rexymcp scorecard` CLI serves it.
- `docs/dev/milestones/M10-context-optimization/README.md` § "Phases" (row 08d).

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
compaction) — the same sum phase-08b's `RECLAIMED` column and phase-08c's
`tokens_reclaimed_mean` use.

### The idiom you will mirror (already in this exact function)

**All aggregation changes are in `mcp/src/scorecard.rs`**, in
`aggregate_by_settings` and `SettingsScorecardRow` / `SettingsAccumulator`. This
file already holds the **identical optional-mean-over-the-present-subset idiom**
you will copy — `length_finish_rate_mean`. The three pieces of that idiom, quoted
so you can pattern-match them for the two new fields:

1. **The output field** (`SettingsScorecardRow`, `scorecard.rs:17-18`):

   ```rust
   /// Mean of `length_finish_rate` over runs where it is `Some`. `None` when none.
   pub length_finish_rate_mean: Option<f64>,
   ```

2. **The accumulator** (`SettingsAccumulator`, `scorecard.rs:55-56`) — derives
   `Default`, so **adding fields here is purely additive** (no literal to update;
   it is only ever built via `or_default()`):

   ```rust
   length_finish_rate_sum: f64,
   length_finish_n: usize,
   ```

3. **The conditional accumulation** inside the `for run in runs` loop of
   `aggregate_by_settings` (`scorecard.rs:102-105`) — the "only count present
   values" pattern, with a paired `_n` counter:

   ```rust
   if let Some(lr) = run.length_finish_rate {
       acc.length_finish_rate_sum += lr;
       acc.length_finish_n += 1;
   }
   ```

4. **The constructor** — the `SettingsScorecardRow { ... }` literal in the
   `filter_map` of `aggregate_by_settings` (`scorecard.rs:135-139`) — emits the
   `Option` mean, guarding the divide on the `_n` counter:

   ```rust
   length_finish_rate_mean: if acc.length_finish_n > 0 {
       Some(acc.length_finish_rate_sum / acc.length_finish_n as f64)
   } else {
       None
   },
   ```

Phase-08c added the exact same two fields to the **model × tag** `ScorecardRow`
in the same file using this same idiom — `scorecard.rs:196-203` (struct),
`scorecard.rs:235-237` (accumulator), `scorecard.rs:292-301` (accumulation),
`scorecard.rs:337-346` (constructor). You are doing the identical thing one
struct over. You may read those lines as a second worked example.

### The three struct literals this phase must update (grep-verified, complete)

Adding two non-`Default` fields to `SettingsScorecardRow` breaks **every**
`SettingsScorecardRow { ... }` literal at once (the struct won't compile until all
carry the new fields). `grep -n 'SettingsScorecardRow {' mcp/src/` returns the
struct definition plus exactly **three** constructor literals — this is the
complete list, in the order to edit them:

```
mcp/src/scorecard.rs:10        — the struct DEFINITION (add the 2 field decls here)
mcp/src/scorecard.rs:129       — LITERAL 1: the aggregate_by_settings constructor (add the 2 means)
mcp/src/scorecard_cli.rs:175   — LITERAL 2: test `format_settings_scorecard_shows_settings_and_signal`, the `rows` vec
mcp/src/scorecard_cli.rs:204   — LITERAL 3: same test, the `rows_none` vec
```

**There are no other `SettingsScorecardRow` literals.** `mcp/src/server.rs` and
`mcp/src/main.rs` do not build one (the CLI path goes through
`load_settings_scorecard` → `aggregate_by_settings` → `format_settings_scorecard`,
never a hand-built literal). The model × settings scorecard is **not** exposed as
an MCP tool, so `server.rs` needs no change.

> **Mechanical-multi-site note (READ THIS — it is why 08d is its own phase).**
> The struct-field add is a wide-blast-radius change: the moment you add the two
> field declarations to the struct definition (`scorecard.rs:10`), all three
> constructor literals stop compiling until each gets both fields. **Do the
> struct definition and all three literals as one contiguous sweep, then
> `cargo build` once** — do not `cargo build` after only the definition (it will
> fail with three `E0063 missing fields` errors, which is expected, not a
> blocker). The exact field text for every site is pre-injected verbatim in the
> Spec below; copy it, do not re-derive it. If a `cargo build` after the full
> sweep still reports a missing-field error, you missed one of the three literals
> above — re-grep `SettingsScorecardRow {` and fill the straggler. This is the
> dominant failure mode for this shape; the pre-injected verbatim text exists to
> eliminate the reasoning that stalls on it.

## Spec

Numbered tasks in execution order.

### The "context-measured" predicate (identical to 08c — pins both fields)

A run **carries context telemetry** iff `run.context_efficiency.peak_context_pct
> 0.0`. (Every run since the per-turn `Metrics` emit — phase-06a — has a nonzero
peak; legacy/pre-08a runs deserialize `context_efficiency` to all-zeros via
`#[serde(default)]` and so are excluded.) Both new means are computed over **only
the context-measured runs** in each (model, settings) bucket, sharing **one**
`_n` counter — exactly as phase-08c does for the tag bucket.

**Boundary case to preserve (pin a test on it):** a run that *is* context-measured
(`peak_context_pct > 0.0`) but whose four reclaim sources sum to `0` is a real
"measured, reclaimed nothing" data point — it **contributes `0.0`** to
`tokens_reclaimed_mean` (pulling the mean down) and its peak to
`peak_context_pct_mean`. It is **not** excluded. Only `peak_context_pct == 0.0`
excludes a run. Do not couple "reclaimed is zero" to "unmeasured."

### 1. Add two fields to `SettingsScorecardRow` (`scorecard.rs:10`, the struct definition)

After the existing `bounces_to_approval_mean: Option<f64>,` field, add (reuse the
exact field names and doc comments from 08c's `ScorecardRow` for cross-scorecard
consistency):

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

### 2. Add three fields to `SettingsAccumulator` (`scorecard.rs:44`)

Additive (the struct derives `Default`; no literal to update). Reuse the exact
field names 08c used in `Accumulator`:

```rust
peak_context_pct_sum: f64,
tokens_reclaimed_sum: f64,
context_measured_n: usize,
```

### 3. Accumulate inside the `for run in runs` loop of `aggregate_by_settings`

Alongside the other conditional accumulations (e.g. the `length_finish_rate`
block at `scorecard.rs:102-105` and the `bounces_to_approval` block), add:

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

### 4. Emit the two means in LITERAL 1 — the `SettingsScorecardRow { ... }` constructor (`scorecard.rs:129`)

After the `bounces_to_approval_mean: …,` line in the constructor, add (matching
the `Option`-mean shape):

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

### 5. Fill LITERAL 2 and LITERAL 3 — the two test literals in `scorecard_cli.rs`

These are hand-built `SettingsScorecardRow` literals in the existing test
`format_settings_scorecard_shows_settings_and_signal`. Add **both** new fields to
**each** so the struct compiles. Give them representative non-`None` and `None`
values so the renderer test (Task 6) has something to assert.

- **LITERAL 2** (`scorecard_cli.rs:175`, the `rows` vec — the populated row):
  after its `bounces_to_approval_mean: None,` line, add:

  ```rust
  peak_context_pct_mean: Some(0.71),
  tokens_reclaimed_mean: Some(9216.0),
  ```

- **LITERAL 3** (`scorecard_cli.rs:204`, the `rows_none` vec — the all-`None`
  row): after its `bounces_to_approval_mean: None,` line, add:

  ```rust
  peak_context_pct_mean: None,
  tokens_reclaimed_mean: None,
  ```

After Tasks 1–5 the workspace compiles. `cargo build` now.

### 6. Add two columns to the `format_settings_scorecard` CLI renderer (`scorecard_cli.rs:37`)

This is the one user-visible new behavior. Add a **`PEAK_CXT`** column (peak
context utilization) and a **`RECLAIMED`** column (mean tokens reclaimed) to the
table, mirroring the per-run rendering already in `mcp/src/runs.rs` so the two
human views read consistently:

- **Header** (`scorecard_cli.rs:43-46`): append `PEAK_CXT` and `RECLAIMED` to the
  header string after `TURNS_MEAN`.
- **Per-row cells** (inside the `for row in rows` loop): compute two strings from
  the `Option<f64>` means, using the same `.map(...).unwrap_or_else(|| "—")`
  idiom the renderer already uses for `length_finish` and `aft`:

  ```rust
  let peak_cxt = row
      .peak_context_pct_mean
      .map(|v| format!("{:.0}%", v * 100.0))
      .unwrap_or_else(|| "—".to_string());

  let reclaimed = match row.tokens_reclaimed_mean {
      None => "—".to_string(),
      Some(v) if v >= 1024.0 => format!("{:.0}k", v / 1024.0),
      Some(v) => format!("{:.0}", v),
  };
  ```

  The `peak_cxt` rendering (`{:.0}%` of `v * 100.0`, `None`/unmeasured → `—`) and
  the `reclaimed` compact-`k` rendering (`>= 1024` → `{N}k`, else `{N}`, `None` →
  `—`) match `runs.rs:113-129` exactly so a reader sees the same forms in
  `rexymcp runs` and `rexymcp scorecard`.

- **Format row** (`scorecard_cli.rs:59-69`): append the two cells to the `format!`
  call's template and argument list. Pick reasonable column widths consistent
  with the existing columns (e.g. `{:>9}` for each, mirroring `LENGTH_FIN`). Exact
  width is your call — the tests pin **content**, not column alignment.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes (only files this phase touched).
- [ ] `cargo test` passes (existing + new).
- [ ] `SettingsScorecardRow` has `peak_context_pct_mean: Option<f64>` and
      `tokens_reclaimed_mean: Option<f64>`.
- [ ] A (model, settings) bucket whose runs have `peak_context_pct` of `0.6` and
      `0.8` (both measured) reports `peak_context_pct_mean == Some(0.7)`.
- [ ] `tokens_reclaimed_mean` averages the **sum of all four** sources per run
      (`output_filtered_tokens + read_evicted_tokens + read_deduped_tokens +
      compaction_tokens_reclaimed`).
- [ ] A bucket containing only legacy runs (`ContextEfficiency::default()`, i.e.
      `peak_context_pct == 0.0`) reports `peak_context_pct_mean == None` **and**
      `tokens_reclaimed_mean == None`.
- [ ] A bucket mixing one measured run (`peak_context_pct == 0.5`) with one legacy
      run (`peak_context_pct == 0.0`) averages over the **measured run only**
      (`context_measured_n == 1`): the legacy run does not drag the mean toward
      zero.
- [ ] A measured run whose four reclaim sources sum to `0` still contributes (its
      bucket's `tokens_reclaimed_mean` is `Some(0.0)` if it is the only measured
      run — **not** `None`).
- [ ] `format_settings_scorecard` output contains a `PEAK_CXT` header and a
      `RECLAIMED` header; a row with `peak_context_pct_mean == Some(0.71)` renders
      `71%`; a row with both means `None` renders `—` for both columns.

## Test plan

Aggregation tests go in `mcp/src/scorecard.rs` `mod tests` (alongside 08c's
`scorecard_*` tests, using the same `make_run` helper, which defaults
`context_efficiency` to all-zeros; set it on the returned `PhaseRun` before
aggregating, and set `generation_params` so the runs land in one settings
bucket). Renderer tests go in `mcp/src/scorecard_cli.rs` `mod tests`. All
hermetic, no IO. Behavior pinned; exact names below are the floor, not a cap.

**Aggregation (`scorecard.rs`, via `aggregate_by_settings`):**

- `by_settings_peak_context_pct_mean_averages_measured_runs` — two runs in one
  (model, settings) bucket with `peak_context_pct` `0.6` and `0.8` → the row's
  `peak_context_pct_mean == Some(0.7)` (within `f64::EPSILON`).
- `by_settings_tokens_reclaimed_mean_sums_all_four_sources` — a single measured
  run (`peak_context_pct = 0.5`) with `output_filtered_tokens = 100`,
  `read_evicted_tokens = 50`, `read_deduped_tokens = 30`,
  `compaction_tokens_reclaimed = 20` → `tokens_reclaimed_mean == Some(200.0)`.
  **Mutation-resistant:** dropping any one source changes the result.
- `by_settings_context_efficiency_none_when_all_legacy` — a bucket whose only run
  is left at `ContextEfficiency::default()` → both means are `None`
  (must-render-`None` negative case).
- `by_settings_context_measured_excludes_legacy_runs` — a bucket with one measured
  run (`peak_context_pct = 0.5`, reclaim sum `400`) and one legacy run
  (`peak_context_pct = 0.0`) → `peak_context_pct_mean == Some(0.5)` and
  `tokens_reclaimed_mean == Some(400.0)` (the legacy zero is excluded from both
  numerator and denominator — **not** averaged in as `(0.5+0.0)/2`). This is the
  strongly mutation-resistant test: it distinguishes the correct measured-only
  mean (`0.5`/`400`) from the naive all-runs mean (`0.25`/`200`).
- `by_settings_measured_run_with_zero_reclaim_contributes` — a bucket whose only
  run is measured (`peak_context_pct = 0.5`) but has all four reclaim sources `0`
  → `tokens_reclaimed_mean == Some(0.0)`, **not** `None`.

**Renderer (`scorecard_cli.rs`, via `format_settings_scorecard`):** extend the
existing `format_settings_scorecard_shows_settings_and_signal` test (or add a
sibling) to assert:

- The output contains `PEAK_CXT` and `RECLAIMED` (the new headers).
- The populated row (LITERAL 2, `peak_context_pct_mean = Some(0.71)`) renders
  `71%`.
- The `rows_none` row (LITERAL 3, both means `None`) renders `—` for the two new
  columns (the must-render-sentinel negative case). The test already asserts a
  `—` is present for `length_finish`; make the new assertion specific enough that
  it would fail if the new columns rendered `0%`/`0` instead of `—` — e.g. assert
  the `RECLAIMED`-position value for that row is `—`, not just that some `—`
  exists.

## End-to-end verification

Unlike 08c (MCP-tool-only, no CLI), this phase **does** ship a runtime CLI
artifact: the `rexymcp scorecard` table. Verify against the real binary, the same
way phase-08b verified its `rexymcp runs` columns.

1. Build: `cargo build -p rexymcp`.
2. Write a small JSONL telemetry fixture with at least one **measured** run
   (nonzero `peak_context_pct` and at least one nonzero reclaim source) and a set
   `generation_params` (e.g. `temperature = 0.2`) so it lands in a named settings
   bucket. (A serialized `PhaseRun` line — you can produce one with
   `serde_json::to_string` in a scratch test, or hand-write the JSON; the
   `#[serde(default)]` fields may be omitted.)
3. Run:
   `cargo run -p rexymcp -- scorecard --config rexymcp.toml --telemetry-path <fixture.jsonl>`
4. Confirm the printed table has the `PEAK_CXT` and `RECLAIMED` headers and the
   row shows the expected `NN%` peak and compact reclaim value (or `—` for an
   unmeasured/legacy fixture row).

Quote the actual table output in the completion Update Log under "End-to-end
verification."

## Authorizations

None. (No new dependencies; no architecture-doc edit; no struct changes outside
`SettingsScorecardRow` / `SettingsAccumulator`; the read field already exists.)

## Out of scope

What this phase must **not** do, even if tempted:

- **Do not touch the model × tag scorecard.** `ScorecardRow`, `Accumulator`, and
  `aggregate` already carry these two means (phase-08c, done). Leave them alone.
- **Do not touch `mcp/src/server.rs` or the `model_scorecard` MCP tool.** The
  model × settings scorecard is CLI-only; the MCP tool serves the tag matrix,
  which is already done.
- **Do not change `ContextEfficiency`, `PhaseRun`, or any executor-crate code.**
  This phase is `mcp/src/scorecard.rs` + `mcp/src/scorecard_cli.rs` only. The
  field is read-only here.
- **Do not change the per-run `rexymcp runs` columns** (`mcp/src/runs.rs`,
  phase-08b). You read its rendering as a worked example, but you do not edit it.
- **Do not fold reclaim variants into `StatusSummary` / `summarize` / the
  dashboard.** That is the later live-view phase (08e).
- **Do not add `peak_context_pct_mean` / `tokens_reclaimed_mean` to the
  `model_scorecard` MCP output as new explicit columns** — they already serialize
  through `ScorecardRow` automatically (08c). No server-side work.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-08 (complete)

**Summary:** Added `peak_context_pct_mean` and `tokens_reclaimed_mean` to
`SettingsScorecardRow` (struct definition) + `SettingsAccumulator` (3 fields) +
`aggregate_by_settings` (accumulation block) in `mcp/src/scorecard.rs`, and two
new `PEAK_CXT` / `RECLAIMED` columns to `format_settings_scorecard` in
`mcp/src/scorecard_cli.rs`. The executor (Qwen/Qwen3.6-27B-FP8) completed the
struct definition, accumulator fields, and accumulation logic, then
hard-failed `VerifierFailurePersistent` (3× E0063 on the constructor literal
before filling it). Architect session takeover closed out: the constructor
literal (`scorecard.rs:150`), both test literals in `scorecard_cli.rs`, the
CLI columns + rendering, and 5 aggregation tests + 1 extended renderer test.

**Acceptance criteria:** all ticked.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.57s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -10
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.62s

cargo test 2>&1 | tail -10
test result: ok. 664 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
(mcp) test result: ok. 257 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.01s
```

**End-to-end verification:**

```
cargo run -p rexymcp -- scorecard --config rexymcp.toml --telemetry-path /tmp/fixture_08d.jsonl

MODEL  SETTINGS          N  GATES  PARSE_FAIL  LENGTH_FIN  AFT_RATE  TURNS_MEAN  PEAK_CXT  RECLAIMED
Qwen/Qwen3.6-27B-FP8 temp=0.2         1   1.00       0.05       0.02      1.00      42.00       68%        13k
```

Fixture run: `peak_context_pct = 0.68` → `68%`; total reclaim = 8000+3500+1200+500 = 13200 → `13k`. Both columns present in header and row.

**Files changed:**
- `mcp/src/scorecard.rs` — 2 fields on `SettingsScorecardRow`, 3 fields on `SettingsAccumulator`, accumulation block, constructor literal, 5 new tests
- `mcp/src/scorecard_cli.rs` — `PEAK_CXT`/`RECLAIMED` headers + row cells in `format_settings_scorecard`, 2 test literals fixed, renderer test extended

**New tests:**
- `by_settings_peak_context_pct_mean_averages_measured_runs` in `mcp/src/scorecard.rs`
- `by_settings_tokens_reclaimed_mean_sums_all_four_sources` in `mcp/src/scorecard.rs`
- `by_settings_context_efficiency_none_when_all_legacy` in `mcp/src/scorecard.rs`
- `by_settings_context_measured_excludes_legacy_runs` in `mcp/src/scorecard.rs`
- `by_settings_measured_run_with_zero_reclaim_contributes` in `mcp/src/scorecard.rs`
- `format_settings_scorecard_shows_settings_and_signal` (extended) in `mcp/src/scorecard_cli.rs`

**Notes for review:** Executor completed struct def + accumulator + accumulation (the additive parts), stalled on the constructor literal — the predicted mechanical-multi-site-churn stall (5th occurrence, controlled comparison arm). This is the calibration data point confirming literal-count as the stall driver (08c: 1 literal, clean first-try; 08d: 3 literals, stall before literal 1 of 3). Architect session takeover closed the remaining 3 literal sites + renderer + tests.
