# Phase 05a-i: Unified scorecard aggregation core (behind wrappers)

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-04b
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=refactor, size=m

## Goal

Collapse the two near-duplicate scorecard aggregations — `aggregate_by_settings`
(model×settings, CLI) and `aggregate` (model×tag, MCP) — into **one** dimension-
parameterised core, `aggregate_scorecard(runs, dimension, filter)`, and re-express
both existing functions as thin wrappers over it. **Fully behavior-preserving:**
every existing scorecard test passes unchanged. This is step 1 of 3 in the
scorecard unification (05a-ii migrates the MCP tool onto the core; 05a-iii adds
the `scorecard --by` CLI + dropped columns and deletes the wrappers). No CLI or
MCP behavior changes here.

**Committed consumers** (so the new code is not dead): 05a-ii points the MCP
`model_scorecard` tool at `aggregate_scorecard`; 05a-iii adds `rexymcp scorecard
--by model|tag|settings`. The new `Model` dimension + `ScorecardBucket` are
exercised by this phase's own unit tests.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 5 (`scorecard --by model|tag|settings` unifies the CLI and MCP
  aggregations).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** every new `#[test]` fn goes **INSIDE**
   `mcp/src/scorecard.rs`'s existing `#[cfg(test)] mod tests { ... }` block.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view
   a file or a line range, use `read_file` (`start_line`/`end_line`), not
   `sed -n`/`cat` — `bash` refuses `sed -i` and repeated identical reads trip the
   governor. On a `patch` "0 matches"/"changed on disk", `read_file` again then
   re-patch.

## Current state

`mcp/src/scorecard.rs` has two aggregations that are **identical except for the
bucket key and one extra metric**:

- **`aggregate_by_settings(runs, filter) -> Vec<SettingsScorecardRow>`**
  (scorecard.rs:66). Buckets by `(model, metrics::settings_label(&run.generation_params))`
  — **one bucket per run**. Uses `SettingsAccumulator`, which additionally tracks
  `length_finish_rate` (`length_finish_rate_sum`/`length_finish_n`). Row carries a
  `settings: String` field and `length_finish_rate_mean: Option<f64>`. Sorts by
  `a.settings.cmp(b.settings).then(b.n_runs.cmp(a.n_runs)).then(a.model.cmp(b.model))`.
- **`aggregate(runs, filter) -> Vec<ScorecardRow>`** (scorecard.rs:266). Buckets by
  `(model, tag)` **for each tag on the run** — a **tag-explosion** (`for tag in
  &run.tags { … }`), so a run with two tags contributes to two buckets. Uses
  `Accumulator` (no `length_finish`). Row carries a `tag: String` field, **no**
  `length_finish_rate_mean`. Sorts by
  `a.tag.cmp(b.tag).then(b.n_runs.cmp(a.n_runs)).then(a.model.cmp(b.model))`.

Everything else — the per-run filter (`filter.model`, `filter.tags`), the metric
sums (`gates_all_pass`, `parse_failure_rate_sum`, `repairs_per_call_sum`,
`tool_success_rate_sum`, `verifier_retries_sum`, `turns_sum`, `wall_clock_s_sum`,
`escalated_count`, verdict counts, `bounces_*`, `peak_context_pct_sum`,
`tokens_reclaimed_sum`, `context_measured_n`), the `min_runs` drop, and the
row-field derivations (`_mean = _sum / n`, the `Option` guards) — is **character-
for-character the same** between the two. `SettingsAccumulator` is a strict
superset of `Accumulator` (only the two `length_finish_*` fields extra).

Both sorts have the **same shape**: `key.cmp().then(n_runs desc).then(model)`.
So a unified core sorted by its bucket key reproduces **both** orders exactly.

Consumers to keep working unchanged: `mcp/src/scorecard_cli.rs`
(`aggregate_by_settings`), `mcp/src/server.rs` `model_scorecard_inner`
(`aggregate`), and every test in `scorecard.rs`/`scorecard_cli.rs`/
`server_tests.rs`.

## Spec

### Task 1 — The dimension enum + unified row

In `mcp/src/scorecard.rs`, add:

```rust
/// Which secondary dimension the scorecard buckets by (model is always the
/// primary key). `Model` = no secondary (one row per model); `Tag` = one row per
/// (model, tag) with tag-explosion; `Settings` = one row per (model, settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScorecardDimension {
    Model,
    Tag,
    Settings,
}

/// One aggregated bucket — the dimension-neutral superset of `ScorecardRow` and
/// `SettingsScorecardRow`. `key` is the secondary-dimension label (`""` for
/// `Model`, the tag for `Tag`, the settings label for `Settings`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ScorecardBucket {
    pub model: String,
    pub key: String,
    pub n_runs: usize,
    pub gates_pass_rate: f64,
    pub parse_failure_rate_mean: f64,
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
    pub peak_context_pct_mean: Option<f64>,
    pub tokens_reclaimed_mean: Option<f64>,
}
```

(`Serialize`/`JsonSchema` are added now because 05a-ii makes this an MCP tool
output; all fields are primitives/`Option`/`String` and already satisfy both.)

### Task 2 — `aggregate_scorecard` (the unified core)

Add `pub fn aggregate_scorecard(runs: &[PhaseRun], dimension: ScorecardDimension,
filter: &ScorecardFilter) -> Vec<ScorecardBucket>`. Build it by **moving the body
of `aggregate_by_settings` in** (it already uses the superset `SettingsAccumulator`
and tracks `length_finish`), and generalising only the **bucket-key step**:

- The per-run `filter.model` / `filter.tags` guards: unchanged (copy verbatim).
- Replace the single `let key = (run.model.clone(), settings_label(…));
  let acc = buckets.entry(key).or_default(); …accumulate…` with a per-dimension
  key selection that runs the **same accumulation body** once per bucket the run
  belongs to:

  ```rust
  let keys: Vec<String> = match dimension {
      ScorecardDimension::Model => vec![String::new()],
      ScorecardDimension::Settings => {
          vec![metrics::settings_label(&run.generation_params)]
      }
      ScorecardDimension::Tag => run.tags.clone(), // tag-explosion: 0..n buckets
  };
  for key in keys {
      let acc = buckets.entry((run.model.clone(), key)).or_default();
      // …the identical accumulation body (n, gates, sums, verdict, bounces,
      //   length_finish, context) copied from aggregate_by_settings…
  }
  ```

  Note `Tag` with an empty `run.tags` contributes **no** bucket (matches today's
  `for tag in &run.tags`). `Model`'s `""` key gives one bucket per model.
- The `min_runs` filter + the `_mean = _sum / n` derivations + the `Option`
  guards: unchanged, emitting `ScorecardBucket { model, key, … }` (was
  `settings`/`tag` → now `key`).
- Sort: `rows.sort_by(|a, b| a.key.cmp(&b.key).then(b.n_runs.cmp(&a.n_runs)).then(a.model.cmp(&b.model)));`
  — the shared shape both old sorts used.

### Task 3 — Reduce the two existing fns to wrappers

Rewrite `aggregate_by_settings` and `aggregate` to delegate. They keep their
**exact signatures and return types** (so every consumer + test is untouched):

```rust
pub fn aggregate_by_settings(
    runs: &[PhaseRun],
    filter: &ScorecardFilter,
) -> Vec<SettingsScorecardRow> {
    aggregate_scorecard(runs, ScorecardDimension::Settings, filter)
        .into_iter()
        .map(|b| SettingsScorecardRow {
            model: b.model,
            settings: b.key,
            n_runs: b.n_runs,
            gates_pass_rate: b.gates_pass_rate,
            parse_failure_rate_mean: b.parse_failure_rate_mean,
            length_finish_rate_mean: b.length_finish_rate_mean,
            repairs_per_call_mean: b.repairs_per_call_mean,
            tool_success_rate_mean: b.tool_success_rate_mean,
            verifier_retries_mean: b.verifier_retries_mean,
            turns_mean: b.turns_mean,
            wall_clock_s_mean: b.wall_clock_s_mean,
            escalation_rate: b.escalation_rate,
            n_with_verdict: b.n_with_verdict,
            approved_first_try_rate: b.approved_first_try_rate,
            bounces_to_approval_mean: b.bounces_to_approval_mean,
            peak_context_pct_mean: b.peak_context_pct_mean,
            tokens_reclaimed_mean: b.tokens_reclaimed_mean,
        })
        .collect()
}

pub fn aggregate(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardRow> {
    aggregate_scorecard(runs, ScorecardDimension::Tag, filter)
        .into_iter()
        .map(|b| ScorecardRow {
            model: b.model,
            tag: b.key,                       // ScorecardRow has no length_finish — drop it
            n_runs: b.n_runs,
            gates_pass_rate: b.gates_pass_rate,
            parse_failure_rate_mean: b.parse_failure_rate_mean,
            repairs_per_call_mean: b.repairs_per_call_mean,
            tool_success_rate_mean: b.tool_success_rate_mean,
            verifier_retries_mean: b.verifier_retries_mean,
            turns_mean: b.turns_mean,
            wall_clock_s_mean: b.wall_clock_s_mean,
            escalation_rate: b.escalation_rate,
            n_with_verdict: b.n_with_verdict,
            approved_first_try_rate: b.approved_first_try_rate,
            bounces_to_approval_mean: b.bounces_to_approval_mean,
            peak_context_pct_mean: b.peak_context_pct_mean,
            tokens_reclaimed_mean: b.tokens_reclaimed_mean,
        })
        .collect()
}
```

The old `SettingsAccumulator` / `Accumulator` structs are now used only inside
`aggregate_scorecard` (keep `SettingsAccumulator`, the superset); if `Accumulator`
becomes unused after the rewrite, **delete it** (STANDARDS: no dead code). Verify
with a grep — do not leave an unused struct or an `#[allow(dead_code)]`.

## Acceptance criteria

- [ ] `aggregate_scorecard` exists with the `ScorecardDimension` param; both
      `aggregate_by_settings` and `aggregate` keep their signatures and are now
      wrappers over it.
- [ ] Every existing scorecard test passes **unchanged** (behavior-preserving —
      the wrappers reproduce the old output byte-for-byte, including sort order).
- [ ] `grep -n "struct Accumulator" mcp/src/scorecard.rs` returns nothing if it
      became unused (else it is still referenced) — no dead struct, no
      `#[allow(dead_code)]`.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `mcp/src/scorecard.rs` tests (reuse the existing `make_run`/`make_run_with_*`
helpers; add ones only if a shape is missing):

- `aggregate_scorecard_settings_matches_wrapper` — for a fixture of ≥3 runs
  across ≥2 settings, `aggregate_scorecard(runs, Settings, filter)` mapped to the
  same fields equals `aggregate_by_settings(runs, filter)` element-for-element
  (same `key`↔`settings`, same `n_runs`, same means, **same order**). (Pins the
  wrapper is a faithful delegate.)
- `aggregate_scorecard_tag_explodes_runs_across_tags` — a run with tags
  `["rust","feature"]` contributes to **both** the `rust` and `feature` buckets
  (assert both keys present and each `n_runs` counts the run). A run with **no**
  tags contributes **no** Tag bucket (negative pin).
- `aggregate_scorecard_model_dimension_one_bucket_per_model` — two runs of model
  `m1` (different tags/settings) + one run of `m2`, `--by Model` yields exactly
  two buckets (`m1` with `n_runs==2`, `m2` with `n_runs==1`), each `key == ""`.
- `aggregate_scorecard_respects_min_runs` — a bucket with fewer than
  `filter.min_runs` runs is dropped (negative pin: it must not appear).

## End-to-end verification

Not applicable — phase ships no new runtime-loadable artifact. It is a pure
internal refactor: `aggregate_scorecard` has no CLI/MCP surface yet (05a-ii/iii
wire it), and the two public wrappers are behavior-preserving. The `cargo test`
gate over the unchanged existing scorecard/CLI/server tests **is** the
verification that behavior is preserved; state that in the completion Update Log.

## Authorizations

None. (No new dependencies; `serde`/`schemars` are already used in this file. No
`docs/architecture.md` edit.)

## Out of scope

- **Any CLI or MCP behavior change** — no `scorecard --by` flag (05a-iii), no MCP
  `model_scorecard` migration (05a-ii). The wrappers must produce identical
  output to today.
- Deleting `SettingsScorecardRow` / `ScorecardRow` — they stay until 05a-iii.
- Displaying the dropped columns (wall-clock, verifier-retries, repairs) — 05a-iii
  (a format change).
- `profile` tokens/cost — phase-05b.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
