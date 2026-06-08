# Phase 08b: Surface context-efficiency in `rexymcp runs`

**Milestone:** M10 — Context optimization
**Status:** todo
**Depends on:** phase-08a (`PhaseRun.context_efficiency` capture — done)
**Estimated diff:** ~75 lines (incl. tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Make the per-run context-efficiency signal that phase-08a captured onto
`PhaseRun` **visible to the user** in the `rexymcp runs` table. Add two columns:
**`PEAK_CXT`** (peak context-window utilization for the run) and **`RECLAIMED`**
(total tokens the M10 levers reclaimed across all four sources). This is the
first surfacing of 08a's data — the most direct consumer (one row per run, read
straight off the field). It is deliberately **single-file** (`mcp/src/runs.rs`)
and **purely additive** (it only *reads* `run.context_efficiency`; no struct
changes), to land the rendering decisions cleanly before they fan out into the
cross-run scorecard aggregations (phase-08c).

Why now: 08a closed the capture gap (the field is on every new `PhaseRun`), but
nothing displays it yet, so M10's effect is still invisible to the user. This
phase makes a single run's reclaim visible; 08c makes it comparable across runs.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` record is the per-run substrate; `rexymcp runs` is its per-run view.
- `docs/dev/milestones/M10-context-optimization/README.md` § "Phases" (row 08b)
  and § "What is novel to rexyMCP (Arc B)" item 4 ("Scorecard-measured
  optimization").

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The field this phase reads (already exists — do not change it)

`executor/src/store/telemetry.rs` defines `ContextEfficiency`, nested on
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

`peak_context_pct` is a **fraction** (`Budget::fraction_used` returns `0.0..=1.0`)
— render it as a percentage by multiplying by 100. The four `*_tokens` fields are
chars/4 estimates; "total reclaimed" for this phase is their **sum** (all four:
the three per-lever reclaim sources **plus** compaction).

### The function to modify — `mcp/src/runs.rs`

`format_runs` (`mcp/src/runs.rs:59`) builds the table: a header line, then one
formatted line per run. The current header (`:65`):

```rust
lines.push(
    "AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC  CXT_WIN".to_string(),
);
```

The current per-run row, including the **compact-number pattern** you will mirror
for `RECLAIMED` (the `cxt_win` block, `:100`) and the final `format!` (`:111`):

```rust
let cxt_win = run
    .context_window
    .map(|n| {
        if n >= 1024 {
            format!("{}k", n / 1024)
        } else {
            format!("{}", n)
        }
    })
    .unwrap_or_else(|| "—".to_string());

lines.push(format!(
    "{:<7} {:<6} {:<14} {:<12} {}  {:<6} {:<9} {:<11} {:<13} {:<7} {}",
    age,
    run.model,
    tags,
    settings,
    gates,
    run.turns,
    run.status,
    verdict,
    served_model,
    trunc,
    cxt_win
));
```

Note the table's sentinel convention: missing/absent values render as `"—"`
(em-dash, U+2014), not `"0"` or empty. The new columns follow it.

This phase touches **only `format_runs` and the `runs.rs` test module**. No
struct literal changes — `PhaseRun` already carries `context_efficiency`, so
`make_run` / `make_run_with_params` compile unchanged.

## Spec

Numbered tasks in execution order. All in `mcp/src/runs.rs`.

### 1. Extend the header line

Append two column headers after `CXT_WIN`: `PEAK_CXT` and `RECLAIMED`. Keep the
existing fixed-width style; the exact spacing/width is your call as long as the
header tokens `PEAK_CXT` and `RECLAIMED` appear and align with the row values
(task 3). Behavior pinned, rendering (widths) yours.

### 2. Compute the two cell values inside the per-run loop

Just before the final `lines.push(format!(...))`, bind a reference to the run's
context-efficiency and compute the two rendered strings:

```rust
let eff = &run.context_efficiency;

// Peak context-window utilization: fraction → percentage. 0.0 means no
// per-turn Metrics were recorded (legacy/unmeasured run) → sentinel.
let peak_cxt = if eff.peak_context_pct == 0.0 {
    "—".to_string()
} else {
    format!("{:.0}%", eff.peak_context_pct * 100.0)
};

// Total tokens reclaimed by ALL four M10 sources. 0 → sentinel (no lever
// fired, or a legacy record).
let reclaimed_total = eff.output_filtered_tokens
    + eff.read_evicted_tokens
    + eff.read_deduped_tokens
    + eff.compaction_tokens_reclaimed;
let reclaimed = if reclaimed_total == 0 {
    "—".to_string()
} else if reclaimed_total >= 1024 {
    format!("{}k", reclaimed_total / 1024)
} else {
    format!("{}", reclaimed_total)
};
```

The two columns are **independent**: a measured run that triggered no levers
renders `PEAK_CXT = "68%"` but `RECLAIMED = "—"`, and that is correct — do not
couple them.

### 3. Append both cells to the row `format!`

Add two trailing fields to the `format!` (matching the header order: `PEAK_CXT`
then `RECLAIMED`), passing `peak_cxt` and `reclaimed`. Keep the fixed-width style
consistent with the existing columns.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes (only files this phase touched).
- [ ] `cargo test` passes (existing + new).
- [ ] `format_runs` output header contains both `PEAK_CXT` and `RECLAIMED`.
- [ ] A run with `peak_context_pct == 0.68` renders `68%` in its line.
- [ ] A run whose four reclaim figures sum to `12288` renders `12k`; a run whose
      figures sum to `200` renders `200` (sub-1024 path).
- [ ] `RECLAIMED` sums **all four** sources (`output_filtered_tokens` +
      `read_evicted_tokens` + `read_deduped_tokens` + `compaction_tokens_reclaimed`).
- [ ] A run with `ContextEfficiency::default()` (all zeros) renders `—` in
      **both** new columns on its line — **not** `0`, `0%`, or empty.

## Test plan

Unit tests in `mcp/src/runs.rs` `mod tests`. Build `PhaseRun`s with the existing
`make_run` helper and set `context_efficiency` on the returned value (the helper
defaults it to all-zeros; mutate the field on the run before formatting). All
hermetic, no IO.

- `format_runs_shows_context_efficiency_columns` — header contains `PEAK_CXT`
  and `RECLAIMED`; a run with `peak_context_pct = 0.68` and reclaim figures
  summing to `12288` (e.g. `output_filtered_tokens = 10000`,
  `read_evicted_tokens = 2000`, `read_deduped_tokens = 288`,
  `compaction_tokens_reclaimed = 0`) → output contains `68%` and `12k`.
- `format_runs_reclaimed_sums_all_four_sources` — a run with
  `output_filtered_tokens = 100`, `read_evicted_tokens = 50`,
  `read_deduped_tokens = 30`, `compaction_tokens_reclaimed = 20` (sum `200`,
  sub-1024) → output contains `200`. This is the **must-sum-all-four** case:
  mutation-resistant because dropping any one source changes the rendered total.
- `format_runs_context_efficiency_dashes_when_zero` — a run left at
  `ContextEfficiency::default()` → its line shows `—` for both new columns and
  **not** `0%` / `0`. This is the **must-render-sentinel** negative case. (Find
  the run's line via a model/tag substring as the other tests in this module do,
  then assert on that line so unrelated `—` sentinels elsewhere don't mask it.)

## End-to-end verification

`rexymcp runs` is a runtime-loadable CLI artifact, so verify against the real
binary, not just the unit tests.

1. Write a single `PhaseRun` JSONL line with a populated `context_efficiency` to
   a temp file. Use this exact line (it deserializes against the current
   `PhaseRun`; total reclaimed = `8000 + 3000 + 1000 + 288 = 12288` → `12k`,
   `peak_context_pct 0.68` → `68%`):

   ```
   {"ts":1717000000000,"model":"qwen","generation_params":{"temperature":null,"seed":null},"phase_id":"phase-08b","tags":["rust"],"status":"complete","escalated":false,"gates":{"fmt":true,"build":true,"lint":true,"test":true},"parse_failure_rate":0.0,"repairs_per_call":0.0,"verifier_retries":0,"tool_success_rate":1.0,"turns":5,"wall_clock_s":10.0,"tokens":{"input_tokens":0,"output_tokens":0,"cache_read_tokens":0,"cache_write_tokens":0},"warnings":null,"bugs_filed":null,"bounces_to_approval":null,"architect_verdict":null,"served_model":null,"length_finish_rate":null,"context_window":262144,"context_efficiency":{"peak_context_pct":0.68,"compaction_count":2,"compaction_tokens_reclaimed":8000,"output_filtered_tokens":3000,"read_evicted_tokens":1000,"read_deduped_tokens":288}}
   ```

2. Run the CLI against it (config is required by the loader even when
   `--telemetry-path` overrides the store; the repo's own `rexymcp.toml` works):

   ```
   cargo run -p rexymcp -- runs --config rexymcp.toml --telemetry-path <tmpfile>
   ```

3. Confirm the printed table has the `PEAK_CXT` and `RECLAIMED` headers and the
   `qwen` row shows `68%` and `12k`. Quote the actual table output in the
   completion Update Log.

## Authorizations

None. (No new dependencies; no architecture-doc edit; no struct changes — the
field already exists.)

## Out of scope

What this phase must **not** do, even if tempted:

- **Do not touch the scorecards.** Adding aggregate means
  (`peak_context_pct_mean`, `tokens_reclaimed_mean`, …) to `ScorecardRow` /
  `SettingsScorecardRow`, their accumulators, the two `aggregate*` functions, and
  the `scorecard` CLI renderer is **phase-08c** (the multi-site struct-literal
  work, deliberately isolated from this single-file phase). Do not edit
  `mcp/src/scorecard.rs` or `mcp/src/scorecard_cli.rs`.
- **Do not fold the reclaim variants into `StatusSummary` / `summarize` or the
  dashboard.** That is **phase-08d** (mcp-only live view).
- **Do not change `ContextEfficiency`, `PhaseRun`, or any executor-crate code.**
  This phase is `mcp/src/runs.rs`-only. The field is read-only here.
- **Do not add a `--json`-path change.** The JSON output already serializes
  `context_efficiency` automatically (it's a `Serialize` field on `PhaseRun`);
  this phase only adds the human-table columns.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
