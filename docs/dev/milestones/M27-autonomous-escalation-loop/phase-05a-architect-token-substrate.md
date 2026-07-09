# Phase 05a: Architect token substrate — `ArchitectTokens` type + cache-aware cost + dormant dashboard wiring

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** done
**Depends on:** phase-02 (ArchitectActivity journal record), phase-02b (dashboard reads activities)
**Estimated diff:** ~430 lines
**Tags:** language=rust, kind=refactor, size=l

## Goal

Rebuild the *architect-side* token model into one coherent, cache-aware type so
the phase-05b harvester and the dashboard can account for architect (Claude Code)
token usage **accurately** — billing uncached input, cache-creation, cache-read,
and output at their real per-class rates instead of a flat input/output pair.

This phase ships **substrate only**: the new types, config rates, a read-time
fold, and the dashboard cost path — all **additive and dormant**. Every architect
token count is `0` until phase-05b's harvester writes real data, so the dashboard's
Architect row renders exactly as it does today (`$0.00`, hidden) after this phase.
The behavioral producer (transcript reader + `rexymcp harvest` CLI) is phase-05b.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § "Token/cost
  accounting — the honesty constraint" — why architect tokens are harvested,
  never estimated; the loop-journal + harvester split.
- `docs/architecture.md` § "Escalation = Claude Code itself" — the architect is a
  real cost center; `[architect]` rates already exist (M20 phase-03).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The architect-token representation today is **two flat `u64` fields**, duplicated
on two structs and — critically — **never populated with real data** (the only
non-test writer, `mcp/src/journal.rs:70-71`, hardcodes `0`). That is the wrong
shape for the 4-class accounting this milestone needs, and because there is no
real historical architect-token data, it is safe to replace outright.

**`executor/src/store/telemetry.rs:108-121` — `TierTelemetry`** (the two
`architect_*_tokens` fields here are dead once the dashboard reads from activities;
this phase **retires** them):

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TierTelemetry {
    pub tier: Option<Tier>,
    pub doc_level: Option<u8>,
    /// Architect input tokens attributed to this run's assists. `0` until the
    /// M27 phase-05 harvester fills it.
    pub architect_input_tokens: u64,
    /// Architect output tokens attributed to this run's assists. `0` until the
    /// M27 phase-05 harvester fills it.
    pub architect_output_tokens: u64,
}
```

**`executor/src/store/telemetry.rs:397-426` — `ArchitectActivity`** (the two flat
token fields become one nested `ArchitectTokens`):

```rust
    /// Architect input tokens for this activity. `0` until the phase-05 harvester fills it.
    #[serde(default)]
    pub architect_input_tokens: u64,
    /// Architect output tokens for this activity. `0` until the phase-05 harvester fills it.
    #[serde(default)]
    pub architect_output_tokens: u64,
```

**`executor/src/config.rs:73-104` — `ArchitectConfig`** carries flat
`input_per_mtok`/`output_per_mtok` and `effective_rates() -> (f64, f64)`. The
shared `known_model_rates` (config.rs:10-18) returns `(input, output)`. **Do not
change `effective_rates` or `known_model_rates`** — `DashboardConfig::effective_rates`
(config.rs:134-143) shares them for the *cloud baseline*, which must stay a flat
input/output pair.

**`mcp/src/dashboard/panels.rs:14-29` — `ScopeCosts` / `BudgetRates`** hold flat
`architect_in`/`architect_out` (tokens) and `architect_input_per_mtok`/
`architect_output_per_mtok` (rates). The cost closures at `panels.rs:512-532`
(`architect_val`, `net_val`) apply the flat rate pair.

**`mcp/src/dashboard/mod.rs:52-123` — `load_data`** folds architect tokens from
`r.tier_telemetry.architect_input_tokens/output_tokens` (project scope 63-68,
milestone scope 109-114). It **already reads `ArchitectActivity` records** for the
assist count (mod.rs:76-85, via `read_architect_activities`) — that is the exact
pattern this phase extends to architect *tokens*.

**`mcp/src/main.rs:617-624`** builds `BudgetRates` from
`cfg.architect.effective_rates()`.

**`mcp/src/init.rs:73-79`** has a `[dashboard]` block but **no `[architect]`
block** — this phase adds one.

## Spec

Numbered tasks in execution order. Build after each of tasks 2, 3, 6, 7 (each
breaks a separate file until its dependents are updated).

### 1. Add `ArchitectTokens` + `ArchitectRates` + rate multipliers

In `executor/src/store/telemetry.rs`, near `ArchitectActivity` (after the
`ARCHITECT_ACTIVITIES` block, ~line 444), add the coherent architect-token type,
its rate sibling, the cost method, and the standard Anthropic cache multipliers.
Add verbatim:

```rust
/// Anthropic prompt-cache rate multipliers relative to the base input rate:
/// a 5-minute cache **write** costs 1.25× input; a cache **read** costs 0.1×
/// input. (1-hour cache writes cost 2× input — approximated here as the 1.25×
/// standard; a dedicated 1h rate can be added additively later.)
pub const CACHE_CREATION_RATE_MULTIPLIER: f64 = 1.25;
pub const CACHE_READ_RATE_MULTIPLIER: f64 = 0.1;

/// The four token classes an architect (Claude Code) request bills separately.
/// One coherent type threaded everywhere the architect touches tokens, replacing
/// the flat `architect_*_tokens` pairs. `#[serde(default)]` so a legacy
/// `ArchitectActivity` line (flat fields, or none) deserializes to all-zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchitectTokens {
    /// Uncached input tokens (`usage.input_tokens`).
    pub input: u64,
    /// Cache-creation input tokens (`usage.cache_creation_input_tokens`).
    pub cache_creation: u64,
    /// Cache-read input tokens (`usage.cache_read_input_tokens`).
    pub cache_read: u64,
    /// Output tokens (`usage.output_tokens`).
    pub output: u64,
}

/// Per-Mtok USD rates for each `ArchitectTokens` class.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArchitectRates {
    pub input_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl ArchitectTokens {
    /// Total USD cost of these tokens at the given per-class rates.
    pub fn cost(&self, rates: &ArchitectRates) -> f64 {
        let per_m = |toks: u64, rate: f64| (toks as f64 / 1_000_000.0) * rate;
        per_m(self.input, rates.input_per_mtok)
            + per_m(self.cache_creation, rates.cache_creation_per_mtok)
            + per_m(self.cache_read, rates.cache_read_per_mtok)
            + per_m(self.output, rates.output_per_mtok)
    }
}
```

### 2. Migrate `ArchitectActivity` to the nested token type

In `executor/src/store/telemetry.rs`, **replace** the two flat fields at
`telemetry.rs:420-425` with one nested field:

```rust
    /// Token usage for this activity, by class. All zero until the phase-05b
    /// harvester fills it; on non-Claude-Code clients they stay zero
    /// (counts-and-durations, never fabricated).
    #[serde(default)]
    pub tokens: ArchitectTokens,
```

Update the `ArchitectActivity` doc comment (telemetry.rs:396) to say the single
`tokens` field (not "the two `architect_*_tokens` fields") is filled by the
harvester.

Then fix the **only** production writer, `mcp/src/journal.rs:70-71`, replacing:

```rust
        architect_input_tokens: 0,
        architect_output_tokens: 0,
```

with:

```rust
        tokens: rexymcp_executor::store::telemetry::ArchitectTokens::default(),
```

(Use whatever import path/alias the surrounding `journal.rs` code already uses for
`telemetry` symbols — match the existing style in that file.)

### 3. Retire the dead `TierTelemetry.architect_*_tokens` fields

In `executor/src/store/telemetry.rs`, **delete** both `architect_input_tokens`
and `architect_output_tokens` from `TierTelemetry` (telemetry.rs:115-120), leaving
only `tier` and `doc_level`. Update the `TierTelemetry` doc comment (telemetry.rs:99-107)
to drop the "`architect_*_tokens` are filled by the harvester" clause — architect
tokens now live on `ArchitectActivity`, not here.

Removing serialized fields is read-safe: serde ignores unknown keys, so legacy
JSONL lines carrying `architect_input_tokens`/`architect_output_tokens` inside
`tier_telemetry` still deserialize (the keys are dropped). Fix any test that
**constructs** `TierTelemetry { ... }` with these fields or **asserts** on them —
in this file the round-trip/back-compat tests around telemetry.rs:989-1016 and
1043 (drop the two fields from the struct literal; drop the
`run.tier_telemetry.architect_input_tokens == 1000` assertion; the JSON-string
fixture at ~1013 may keep the now-ignored keys to prove back-compat, or drop them —
either is fine, but do not assert a value that no longer has a field).

### 4. Add `fold_activities` — last-write-wins overlay by activity identity

In `executor/src/store/telemetry.rs`, add a read-time fold that collapses the
originals-plus-harvester-enriched-copies down to one record per activity, **latest
occurrence wins**. Unlike `fold_reviews` (which picks max `ts`), the harvester's
enriched copy shares the **same** `ts` as the original it enriches, so
disambiguation is by **append order** (the enriched copy is appended later, so it
appears later in the file, so last-occurrence-in-input-order wins). `read_architect_activities`
preserves file order, so input order == append order.

Add verbatim:

```rust
/// Collapse `ArchitectActivity` records to one per activity identity, keeping the
/// **last** occurrence in input order. The phase-05b harvester appends an enriched
/// copy (same `phase_id`/`activity`/`ts`, tokens filled) after the original
/// zero-token record; since `read_architect_activities` preserves file (append)
/// order, the later enriched copy wins. Identity key: `(phase_id, activity, ts)`.
pub fn fold_activities(activities: Vec<ArchitectActivity>) -> Vec<ArchitectActivity> {
    use std::collections::HashMap;
    // Index of the winning (latest) record per key, into a preserved-order Vec.
    let mut latest: HashMap<(String, String, u64), usize> = HashMap::new();
    let mut out: Vec<ArchitectActivity> = Vec::new();
    for act in activities {
        let key = (act.phase_id.clone(), act.activity.clone(), act.ts);
        if let Some(&idx) = latest.get(&key) {
            out[idx] = act;
        } else {
            latest.insert(key, out.len());
            out.push(act);
        }
    }
    out
}
```

### 5. Add cache rates + `effective_architect_rates` to `ArchitectConfig`

In `executor/src/config.rs`, add two fields to `ArchitectConfig` (after
`output_per_mtok`, config.rs:82):

```rust
    /// USD per million cache-**read** input tokens (overridden by `model` when
    /// recognised: 0.1× the input rate).
    pub cache_read_per_mtok: f64,
    /// USD per million cache-**creation** input tokens (overridden by `model`
    /// when recognised: 1.25× the input rate).
    pub cache_creation_per_mtok: f64,
```

Add both to `Default` (config.rs:85-93) as `0.0`.

Then add a method (below `effective_rates`, config.rs:104) that resolves the full
4-class `ArchitectRates`. When `model` is a recognised Claude ID, cache rates are
**derived** from the model's input rate via the multipliers (single-sourced —
`known_model_rates` stays the one rate table); otherwise the explicit config
fields are used:

```rust
    /// Resolved per-class architect rates. When `model` is recognised, cache
    /// rates derive from its input rate (0.1× read, 1.25× creation); otherwise the
    /// explicit `cache_*_per_mtok` fields apply. Reuses `effective_rates` for the
    /// input/output pair.
    pub fn effective_architect_rates(&self) -> crate::store::telemetry::ArchitectRates {
        use crate::store::telemetry::{
            ArchitectRates, CACHE_CREATION_RATE_MULTIPLIER, CACHE_READ_RATE_MULTIPLIER,
        };
        let (input, output) = self.effective_rates();
        let model_known = self.model.as_deref().and_then(known_model_rates).is_some();
        let (cache_read, cache_creation) = if model_known {
            (
                input * CACHE_READ_RATE_MULTIPLIER,
                input * CACHE_CREATION_RATE_MULTIPLIER,
            )
        } else {
            (self.cache_read_per_mtok, self.cache_creation_per_mtok)
        };
        ArchitectRates {
            input_per_mtok: input,
            cache_creation_per_mtok: cache_creation,
            cache_read_per_mtok: cache_read,
            output_per_mtok: output,
        }
    }
```

(Confirm the module path `crate::store::telemetry` resolves `ArchitectRates` from
config.rs; adjust the `use` path if the crate lays it out differently, but do
**not** move the types out of `telemetry.rs`.)

### 6. Cache-aware dashboard cost path (`panels.rs`)

In `mcp/src/dashboard/panels.rs`:

- **`ScopeCosts` (panels.rs:14-20):** replace `architect_in`/`architect_out`
  (both `u64`) with a single `pub architect: ArchitectTokens` field. Keep
  `executor_in`/`executor_out` unchanged (executor side is out of scope). Import
  `ArchitectTokens`/`ArchitectRates` from `rexymcp_executor::store::telemetry`.
  `ScopeCosts` must stay `Copy` (`ArchitectTokens` is `Copy`).
- **`BudgetRates` (panels.rs:24-29):** replace `architect_input_per_mtok`/
  `architect_output_per_mtok` with a single `pub architect: ArchitectRates` field.
- **Cost closures (panels.rs:512-532):** rewrite `architect_val` and `net_val` to
  take an `ArchitectTokens` and use `.cost(&rates.architect)`:

```rust
    let architect_val =
        |t: ArchitectTokens| -> String { fmt_dollars(t.cost(&rates.architect)) };
    let net_val = |b_in: u64, b_out: u64, a: ArchitectTokens| -> String {
        if no_baseline {
            return "—".to_string();
        }
        let baseline = cost(b_in, b_out, rates.input_per_mtok, rates.output_per_mtok);
        fmt_dollars(baseline - a.cost(&rates.architect))
    };
```

- **Row call sites (panels.rs:588-611):** the Architect row and Net row now pass
  `ArchitectTokens` values: session scope passes `ArchitectTokens::default()`
  (per-session architect tokens are not tracked); milestone passes `mile.architect`;
  project passes `project_costs.architect`. Update `debit_row`'s Architect call and
  both `net_val` calls accordingly.
- Update the fixture `BudgetRates`/`ScopeCosts` literals in this file's tests
  (panels.rs ~1806-1944) to the new field shapes. A known-model architect rate
  such as opus (`architect_input_per_mtok: 5.0, architect_output_per_mtok: 25.0`)
  becomes `architect: ArchitectRates { input_per_mtok: 5.0, cache_creation_per_mtok: 6.25, cache_read_per_mtok: 0.5, output_per_mtok: 25.0 }`.

### 7. Dashboard reads architect tokens from folded activities (`mod.rs`)

In `mcp/src/dashboard/mod.rs`, `load_data`, stop reading architect tokens from
`tier_telemetry` and instead sum them from **folded** `ArchitectActivity` records,
scoped the same way the executor tokens are (project = `project_id`; milestone =
`project_id` + `milestone_id`).

- Read + fold activities once (reuse the existing
  `telemetry::read_architect_activities(&dir.join("phase_runs.jsonl"))` call — you
  already load them for the assist count at mod.rs:76-85; fold them with
  `telemetry::fold_activities(...)`).
- In the **project** fold (mod.rs:56-70), delete the two
  `costs.architect_in/out.saturating_add(r.tier_telemetry.architect_*)` lines. The
  per-run fold now sums executor tokens only.
- Compute `project_costs.architect` by summing `a.tokens` across folded activities
  where `a.project_id.as_deref() == Some(pid)`, into a single `ArchitectTokens`
  (saturating adds per class).
- Do the same for the **milestone** fold (mod.rs:102-116): drop the
  `tier_telemetry.architect_*` lines from the per-run fold; sum
  `mile.architect` from folded activities where `a.project_id == pid && a.milestone_id == Some(milestone_dir)`.
- The final milestone `.filter` at mod.rs:118-123 checks "any cost non-zero" —
  update it to reference the new `architect` field's classes (e.g. treat the
  milestone as having architect cost when any of `architect.input`,
  `architect.cache_creation`, `architect.cache_read`, `architect.output` is
  non-zero, alongside the executor checks).

A small private helper to sum a filtered activity iterator into `ArchitectTokens`
is fine (avoids duplicating the four saturating adds twice). Keep it in `mod.rs`.

- **Rewrite the affected dashboard test(s)** (mod.rs ~462-482 seeds architect
  tokens via `tier_telemetry`): seed an `ArchitectActivity` JSONL line (record tag
  `"architect_activity"`, matching `project_id`, non-zero `tokens`) instead, and
  assert the Architect/Net rows reflect its cost. This is the load-bearing proof
  that the activity→dashboard path works.

### 8. Wire `main.rs` to the new rates

In `mcp/src/main.rs:617-624`, replace the architect-rate wiring. `cfg.architect.effective_architect_rates()`
now returns an `ArchitectRates`; assign it to `BudgetRates.architect`:

```rust
            let (i, o) = cfg.dashboard.effective_rates();
            let rates = dashboard::BudgetRates {
                input_per_mtok: i,
                output_per_mtok: o,
                architect: cfg.architect.effective_architect_rates(),
            };
```

### 9. Document `[architect]` rates in the init template

In `mcp/src/init.rs`, add a commented `[architect]` block after the `[dashboard]`
block (init.rs:79), documenting the model auto-fill and all four explicit rate
keys:

```
[architect]
# model = "claude-opus-4-8"        # auto-fill architect rates for a known Claude model
#   (cache rates derive from input: read = 0.1×, creation = 1.25×)
# Or set rates directly (model overrides these when set & recognised):
# input_per_mtok = 5.0             # $/MTok uncached input tokens
# output_per_mtok = 25.0           # $/MTok output tokens
# cache_read_per_mtok = 0.5        # $/MTok cache-read input tokens
# cache_creation_per_mtok = 6.25   # $/MTok cache-creation input tokens
```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `ArchitectTokens` and `ArchitectRates` exist in `executor/src/store/telemetry.rs`;
      `ArchitectTokens::cost` costs each class at its own rate.
- [ ] `ArchitectActivity` carries a single `tokens: ArchitectTokens` field; the flat
      `architect_input_tokens`/`architect_output_tokens` are gone from both
      `ArchitectActivity` and `TierTelemetry`.
- [ ] `ArchitectConfig::effective_architect_rates()` returns opus cache rates
      `(read 0.5, creation 6.25)` when `model = "claude-opus-4-8"`, and the explicit
      `cache_*_per_mtok` fields when the model is unknown.
- [ ] The dashboard sums architect tokens from folded `ArchitectActivity` records,
      not `tier_telemetry`; with **no** architect activities the Architect row is
      still hidden (`$0.00`) — dormant.
- [ ] `rexymcp init` output contains an `[architect]` block with the four rate keys.

## Test plan

- `architect_tokens_cost_bills_each_class_at_its_own_rate` (telemetry.rs) — an
  `ArchitectTokens { input:1_000_000, cache_creation:1_000_000, cache_read:1_000_000, output:1_000_000 }`
  against `ArchitectRates { 5.0, 6.25, 0.5, 25.0 }` costs `5.0+6.25+0.5+25.0 = 36.75`.
- `architect_tokens_default_is_zero_cost` (telemetry.rs) — default tokens cost `0.0`.
- `fold_activities_enriched_copy_wins` (telemetry.rs) — two records same
  `(phase_id, activity, ts)`, first `tokens` zero, second `tokens.input = 500`;
  fold returns one record with `input == 500`. **Mutation check:** reversing the
  input order must make the zero record win (proves it is order-based, not max).
- `fold_activities_distinct_ts_keeps_both` (telemetry.rs) — same phase/activity,
  different `ts` → both retained (negative: not over-collapsed).
- `fold_activities_distinct_activity_keeps_both` (telemetry.rs) — same phase/ts,
  `activity` `"draft"` vs `"review"` → both retained.
- `architect_activity_roundtrips_nested_tokens` (telemetry.rs) — serialize an
  `ArchitectActivity` with non-zero `tokens`, read it back via
  `read_architect_activities`, assert the class counts survive.
- `legacy_activity_line_without_tokens_defaults_zero` (telemetry.rs) — a JSON line
  with the **old** flat `architect_input_tokens`/`architect_output_tokens` keys (and
  no `tokens` object) deserializes with `tokens == ArchitectTokens::default()`
  (back-compat: old keys ignored, no panic).
- `effective_architect_rates_derives_cache_from_known_model` (config.rs) — `model =
  "claude-opus-4-8"` → `ArchitectRates { input:5.0, cache_creation:6.25, cache_read:0.5, output:25.0 }`.
- `effective_architect_rates_uses_explicit_when_model_unknown` (config.rs) — no
  model, explicit `cache_read_per_mtok:2.0, cache_creation_per_mtok:9.0`,
  `input_per_mtok:8.0, output_per_mtok:40.0` → those exact values pass through.
- Dashboard: rewrite the existing architect-cost test (mod.rs) to seed an
  `ArchitectActivity` with non-zero `tokens` and assert the Architect/Net rows
  reflect its cache-aware cost; add a **negative** — an activity with a different
  `project_id` contributes nothing to `project_costs.architect`.
- `panels.rs` cost-row tests: update fixtures to the new `ArchitectTokens`/
  `ArchitectRates` shapes; keep at least one asserting a non-zero Architect row and
  one asserting the row is hidden at all-zero.

## End-to-end verification

`rexymcp init` ships a real artifact (the config template). Run it against a temp
path and confirm the `[architect]` block is present, then quote the output:

```
cargo run -p rexymcp -- init --config /tmp/rexymcp-05a-check.toml
grep -A6 '\[architect\]' /tmp/rexymcp-05a-check.toml
```

The dashboard cost path is **dormant** this phase (no harvester writes tokens yet),
so it cannot be exercised end-to-end against real architect data here — it is
covered by the seeded-`ArchitectActivity` unit test in task 7, and its full
end-to-end proof (harvest a real transcript → dashboard shows non-zero architect
cost) lands in phase-05b. State this in the completion Update Log.

## Authorizations

- [ ] May touch `docs/architecture.md`: **No.**
- [ ] May add dependencies: **No.**

None. (No new dependency — the ISO-timestamp parsing and all `~/.claude`
transcript reading are phase-05b; this phase is pure Rust type/config/dashboard
work using only what the workspace already links.)

## Out of scope

- **The harvester itself** — the Claude Code transcript reader, `message.id`
  dedup, ISO-8601→epoch-ms parsing, the per-phase time-window join, and the
  `rexymcp harvest` CLI subcommand are **all phase-05b**. Do not read
  `~/.claude`, do not parse timestamps, do not add a CLI command here.
- **Executor tokens (`TokenBreakdown`)** — untouched. Do not add cache classes to
  executor telemetry; the local model's cost is `$0.00` and its history is
  populated. Only the architect side changes.
- **Distinguishing 5-minute vs 1-hour cache-creation rates** — this phase uses one
  `cache_creation` rate (the 1.25× standard); a separate 1h class is a possible
  additive follow-up, not this phase.
- **Populating `PhaseRun.escalation_count` / assist counts** — already handled
  (phase-02b); do not touch the assist-count logic beyond reusing the existing
  `read_architect_activities` call as a fold source.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-09 14:00 (started)

**Executor:** Claude (executor LLM)

Implemented all 9 spec tasks: added `ArchitectTokens`/`ArchitectRates`/cache multipliers to `telemetry.rs`, migrated `ArchitectActivity` to nested `tokens` field, retired `TierTelemetry.architect_*_tokens`, added `fold_activities`, wired `ArchitectConfig::effective_architect_rates()`, updated dashboard cost path in `panels.rs` and `mod.rs` to sum from folded activities, wired `main.rs` to new rates, and documented `[architect]` block in init template. All tests pass including new ones for cost, fold, roundtrip, legacy compat, and effective rates.
### Update — ts=1783608155764 (complete, server-authored)

**Summary:** All verification gates pass. The working tree is clean with one conventional commit covering all 9 files.

**Summary + Notes for review:**

This phase rebuilt the architect-side token model from two flat `u64` fields into a coherent 4-class `ArchitectTokens` type with `ArchitectRates`, cache rate multipliers (1.25× creation, 0.1× read), and a `cost()` method. `TierTelemetry.architect_*_tokens` fields were retired (dead — architect tokens now live exclusively on `ArchitectActivity.tokens`). The dashboard cost path in `panels.rs` and `mod.rs` was updated to use `ArchitectTokens.cost()` and sum from folded `ArchitectActivity` records via the new `fold_activities()` function. `ArchitectConfig::effective_architect_rates()` derives cache rates from the known model's input rate. The `rexymcp init` template now documents the `[architect]` block with all four rate keys. The dashboard cost path is **dormant** this phase (no harvester writes tokens yet), so it is covered by the seeded-`ArchitectActivity` unit test in task 7; the full end-to-end proof (harvest a real transcript → dashboard shows non-zero architect cost) lands in phase-05b.

**Gates:** format=run, build=run, lint=run, test=run

### Review verdict — 2026-07-09

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none
- **Calibration:** none

Independent re-run of all four gates green (fmt/build/clippy clean; 926 executor
+ 472 mcp tests pass, 2 ignored). Diff matches the phase doc's pre-injected code
verbatim across all 9 tasks (`ArchitectTokens`/`ArchitectRates`/cost/
`fold_activities` in `telemetry.rs`; `effective_architect_rates` in `config.rs`;
the cache-aware cost closures + `ScopeCosts`/`BudgetRates` reshape in
`panels.rs`; the folded-activity sum + negative test in `mod.rs`;
`main.rs`/`journal.rs` wiring; the `[architect]` init block). No production
`unwrap`/`expect`/`panic!`/`TODO`/`dbg!`/`#[allow]`/`#[ignore]`/`unsafe` added
(all `.unwrap()` hits are test code). `architecture.md`/`Cargo.toml` untouched,
matching the phase doc's "No" authorizations. `fold_activities_enriched_copy_wins`
spot-checked as a real mutation-resistant test (reversed-order assertion proves
order-based, not max-based, selection). End-to-end verified independently: `cargo
run -p rexymcp -- init --dir <tmp> --force` produces a `[architect]` block with
all four rate keys (the phase doc's own example command uses a nonexistent
`--config` flag on `init` — a phase-doc typo, not an executor defect; `init` only
accepts `--dir`/`--force`, confirmed via `--help`). Dormant-scope claim holds: no
architect activities exist yet, so the dashboard Architect row stays hidden.

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.09s


TEST
st tools::symbols::tests::single_file_unsupported_extension_advisory_error ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::flips_active_task_to_done ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 926 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` — +1 -1
- `docs/dev/milestones/M27-autonomous-escalation-loop/phase-05a-architect-token-substrate.md` — +7 -1
- `executor/src/config.rs` — +70 -0
- `executor/src/store/telemetry.rs` — +256 -27
- `mcp/src/dashboard/mod.rs` — +105 -51
- `mcp/src/dashboard/panels.rs` — +62 -50
- `mcp/src/init.rs` — +9 -0
- `mcp/src/journal.rs` — +1 -2
- `mcp/src/main.rs` — +1 -3

**Commit:** 2334084c38391775bcc5e5c64ea6890f7c9ec0b6

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

