# Phase 04: surface the profile to the architect (`rexymcp profile` CLI + MCP tool)

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** todo
**Depends on:** phase-03 (the `mcp/src/profile.rs` aggregation layer is live —
`ModelProfile`, `aggregate_profiles`, `is_model_attributable`)
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Phase-03 built the pure `model_profile` aggregation but shipped **no binary
surface** — `aggregate_profiles` has no caller, which is why `profile.rs:1`
carries a `#![allow(dead_code)]`. This phase gives it the two surfaces the
architect queries at draft time: a **`rexymcp profile` CLI subcommand** (a human
table + `--json`) and a **`model_profile` MCP tool**. Both mirror the existing
`scorecard` pair (`scorecard_cli.rs` + `server.rs::model_scorecard`) almost
line-for-line. Wiring real callers lets the `#![allow(dead_code)]` come **out**
of `profile.rs` — its mandatory removal is part of this phase.

This is the payoff of thread 2: the architect runs `rexymcp profile --model
<m> --tag <t>` (or calls the MCP tool) before drafting and sees that model's
strengths (gate-pass / first-pass rates) **and** its ranked failure classes, so
pre-injection is data-driven instead of guessed.

## Architecture references

Read before starting:

- `docs/architecture.md#layer-2--mcp-crate-binary` — `model_scorecard`, the
  `rexymcp runs` / `rexymcp scorecard` CLIs; the surface this phase parallels.
- `docs/architecture.md#layer-3--plugin-package` — pre-injection is the
  architect's capability-transfer channel; this profile feeds it.
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "The failure-class
  taxonomy" — why `spec_bug`/`infra_blip` must be visually separated from a
  model's real weaknesses (that is what `is_model_attributable` is for, and the
  formatter must honor it).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read `mcp/src/profile.rs` end to end — you are giving its public items
   (`aggregate_profiles`, `ModelProfile`, `FailureClassCount`,
   `is_model_attributable`) their first callers.
4. Read this entire phase doc before touching any code.
5. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The CLI pair to mirror (`mcp/src/scorecard_cli.rs`)

`scorecard_cli.rs` is the exact template for the new `profile_cli.rs`. Its loader
resolves the telemetry file from config (or a `--telemetry-path` override),
reads runs + reviews, folds, aggregates, and returns rows
(`scorecard_cli.rs:12-37`):

```rust
pub fn load_settings_scorecard(
    config_path: &Path,
    telemetry_path: Option<&Path>,
    filter: &ScorecardFilter,
) -> Result<Vec<SettingsScorecardRow>, String> {
    let cfg =
        Config::load_with_env(config_path).map_err(|e| format!("failed to load config: {}", e))?;

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

    let runs: Vec<PhaseRun> =
        rexymcp_executor::store::telemetry::read(&telemetry_file).map_err(|e| e.to_string())?;
    let reviews = rexymcp_executor::store::telemetry::read_reviews(&telemetry_file)
        .map_err(|e| e.to_string())?;
    let runs = rexymcp_executor::store::telemetry::fold_reviews(runs, &reviews);
    Ok(aggregate_by_settings(&runs, filter))
}
```

> **⚠️ The one place `profile` differs from `scorecard`.** `aggregate_by_settings`
> takes **already-folded** runs, so `scorecard_cli` calls `fold_reviews` first.
> **`aggregate_profiles` folds internally** (it needs the raw `reviews` to read
> `failure_class`, which `fold_reviews` does *not* copy onto the run). So your
> `load_profiles` must **NOT** call `fold_reviews` — read `runs` and `reviews`,
> then pass **both raw** to `aggregate_profiles(&runs, &reviews, filter)`. Quoting
> `aggregate_profiles`'s doc comment (`profile.rs:106-111`): *"`runs` and
> `reviews` are the raw store reads … this function folds internally so callers
> pass both unmodified."* Double-folding would not crash but is wrong and wasteful
> — do not do it.

The human formatter renders `Option<f64>` means as `—` and otherwise as a fixed
table (`scorecard_cli.rs:40-89`); mirror its `—`-for-`None` discipline:

```rust
let aft = row
    .approved_first_try_rate
    .map(|v| format!("{:.2}", v))
    .unwrap_or_else(|| "—".to_string());
```

### The MCP tool to mirror (`mcp/src/server.rs::model_scorecard`)

`ModelScorecardParams` / `ModelScorecardOutput` / `model_scorecard_inner` /
the `#[rmcp::tool]` method are the exact template. The params + output
(`server.rs:252-271`):

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelScorecardParams {
    /// Tags the run must contain (AND-ed). Empty = no filter.
    pub tags: Option<Vec<String>>,
    pub model: Option<String>,
    pub min_runs: Option<usize>,
    pub telemetry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ModelScorecardOutput {
    pub rows: Vec<scorecard::ScorecardRow>,
    pub total_runs_considered: usize,
    pub truncated: bool,
}
```

The inner fn (`server.rs:274-318`) resolves the telemetry file, reads runs +
reviews, computes `total_runs_considered = runs.len()`, builds a
`scorecard::ScorecardFilter`, aggregates, and caps to `scorecard::MAX_ROWS`. Note
its **scorecard-specific fold** at `server.rs:296` (`let runs = fold_reviews(runs,
&reviews);`) — the same `profile` caveat applies: **drop that fold line** in
`model_profile_inner` and pass raw `runs`/`reviews` to `aggregate_profiles`.
Compute `total_runs_considered` from the **raw** `runs.len()` before aggregation.

The tool method + router (`server.rs:364-372`):

```rust
    #[rmcp::tool(
        description = "Aggregate the cross-project PhaseRun telemetry into a model × tag competency matrix. …"
    )]
    async fn model_scorecard(
        &self,
        Parameters(params): Parameters<ModelScorecardParams>,
    ) -> Result<Json<ModelScorecardOutput>, String> {
        model_scorecard_inner(&self.config_path, &params).map(Json)
    }
```

`server.rs:20` already has `use crate::scorecard;` — add `use crate::profile;`
alongside it. `scorecard::MAX_ROWS` is `pub const … = 500` (`scorecard.rs:248`);
**reuse it** — do not add a new constant to `profile.rs`.

### The `Commands` enum + dispatch to mirror (`mcp/src/main.rs`)

The `Scorecard` clap variant (`main.rs:105-130`) and its dispatch arm
(`main.rs:373-410`) are the template for a new `Profile` variant. The dispatch
arm builds the borrowed filter and prints JSON or the formatter output:

```rust
        Commands::Scorecard { config, model, tags, min_runs, telemetry_path, json } => {
            let filter = scorecard::ScorecardFilter {
                model: model.as_deref(),
                tags: &tags,
                min_runs,
            };
            let rows = match scorecard_cli::load_settings_scorecard(
                &config, telemetry_path.as_deref(), &filter,
            ) {
                Ok(r) => r,
                Err(e) => { eprintln!("{e}"); std::process::exit(1); }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_else(|e| {
                    format!("{{\"error\": \"failed to serialize scorecard: {}\"}}", e)
                }));
            } else {
                println!("{}", scorecard_cli::format_settings_scorecard(&rows));
            }
            Ok(())
        }
```

`main.rs:11` already declares `mod profile;` (added in phase-03). The module
declarations are alphabetized (`main.rs:6-19`); add `mod profile_cli;` directly
after `mod profile;`.

### The dead-code attribute to REMOVE (`mcp/src/profile.rs:1-3`)

```rust
#![allow(dead_code)]
// Removal note: this attribute must be removed in phase-04 when the CLI/MCP
// surface lands and `aggregate_profiles` has callers.
```

These three lines must be **gone** by the end of this phase. They are only safe
to remove once every public item has a non-test caller — which this phase
creates. After you wire both surfaces, delete lines 1-3 and confirm `clippy
-D warnings` stays clean (no `function … is never used`). If clippy then flags
`is_model_attributable` as never used, that means the formatter isn't calling it
— see §3, which requires it to.

## Spec

All changes are additive except the three-line deletion in `profile.rs`. New
file: `mcp/src/profile_cli.rs`. Edited files: `mcp/src/main.rs`,
`mcp/src/server.rs`, `mcp/src/profile.rs`.

1. **Remove the dead-code suppression** — in `mcp/src/profile.rs`, delete the
   `#![allow(dead_code)]` inner attribute and its two-line removal-note comment
   (lines 1-3). Do this *after* the callers below compile, or do it first and
   keep clippy red until they land — either order, but it must be gone at the
   end. No other change to `profile.rs`.

2. **Create `mcp/src/profile_cli.rs`** with two public functions mirroring
   `scorecard_cli.rs`:
   - `load_profiles(config_path: &Path, telemetry_path: Option<&Path>, filter:
     &ScorecardFilter) -> Result<Vec<ModelProfile>, String>` — copy
     `load_settings_scorecard`'s config/telemetry-file resolution verbatim
     (including the exact `"telemetry disabled: …"` error string), then read
     `runs` and `reviews` and return
     `Ok(crate::profile::aggregate_profiles(&runs, &reviews, filter))`. **Do not
     call `fold_reviews`** (see the ⚠️ note in Current state).
   - `format_profiles(rows: &[ModelProfile]) -> String` — a human table. Empty →
     a single `(no profiles)` line. Otherwise a header line + one row each. Pin
     these columns (label text and ordering are pinned; exact spacing/width is
     not): `MODEL`, `TAG`, `N` (`n_runs`), `GATES` (`gates_pass_rate`, `{:.2}`),
     `AFT` (`approved_first_try_rate`, `—` when `None`), `BOUNCES`
     (`bounces_to_approval_mean`, `—` when `None`), `TOOL`
     (`tool_success_rate_mean`), `PARSE` (`parse_failure_rate_mean`), `ESC`
     (`escalation_rate`), and `WEAKNESSES`.
     The `WEAKNESSES` cell renders `ranked_failure_classes` as
     `class×count` entries joined by spaces (empty → `—`). **Use
     `crate::profile::is_model_attributable` here:** a class for which it returns
     `false` (`spec_bug`/`infra_blip`) is rendered **parenthesized** —
     e.g. `prod_unwrap×2 (spec_bug×1)` — so the architect reads it as "not the
     model's fault." Attributable classes render bare. This call is what gives
     `is_model_attributable` a production caller (so the dead-code attribute can
     go); it is load-bearing, not cosmetic.

3. **Add the `Profile` CLI command** in `mcp/src/main.rs`:
   - Add `mod profile_cli;` after `mod profile;`.
   - Add a `Commands::Profile { config, model, tags, min_runs, telemetry_path,
     json }` variant whose fields and `#[arg(...)]` attributes are **identical**
     to `Commands::Scorecard` (same doc-comment style, same `--tag` repeat,
     same `--min-runs` default `0`, same `--json`). Give it the doc comment
     `/// Aggregate folded runs into a per-(model, tag) capability profile`.
   - Add a dispatch arm mirroring the `Scorecard` arm, calling
     `profile_cli::load_profiles` and `profile_cli::format_profiles`. On the
     `--json` error fallback string, use `"failed to serialize profile"`.

4. **Add the `model_profile` MCP tool** in `mcp/src/server.rs`:
   - Add `use crate::profile;` next to `use crate::scorecard;`.
   - Add `ModelProfileParams` (identical shape to `ModelScorecardParams`) and
     `ModelProfileOutput { rows: Vec<profile::ModelProfile>,
     total_runs_considered: usize, truncated: bool }`.
   - Add `pub(crate) fn model_profile_inner(config_path: &Path, params:
     &ModelProfileParams) -> Result<ModelProfileOutput, String>` mirroring
     `model_scorecard_inner`: resolve the telemetry file, read `runs` + `reviews`,
     set `total_runs_considered = runs.len()` (the **raw** count, before
     aggregation), build a `scorecard::ScorecardFilter`, call
     `profile::aggregate_profiles(&runs, &reviews, &filter)` (**no fold**), then
     cap to `scorecard::MAX_ROWS` exactly as `model_scorecard_inner` does
     (`truncated = rows.len() > MAX_ROWS; if truncated { rows.truncate(MAX_ROWS); }`).
   - Add a `#[rmcp::tool(description = "…")]` `model_profile` method in the
     `#[rmcp::tool_router] impl RexyMcpServer` block, after `model_scorecard`,
     delegating to `model_profile_inner`. Write a description in the same voice
     as `model_scorecard`'s: it aggregates the cross-project telemetry into a
     per-(model, tag) **capability profile** — strengths (gate-pass / first-pass
     rates, reliability means) **and** ranked failure classes with counts;
     filterable by tags (AND), model, min_runs; capped at 500 rows.

### Gotchas (pin these)

- **No double-fold.** Both `load_profiles` and `model_profile_inner` pass raw
  `runs`/`reviews` to `aggregate_profiles`. Deleting the `fold_reviews` line that
  the scorecard templates have is a *required* edit, not an omission.
- **The formatter MUST call `is_model_attributable`.** If it doesn't, removing
  `#![allow(dead_code)]` makes clippy fail (`function is_model_attributable is
  never used`). Don't "fix" that by re-adding the attribute — wire the call.
- **Reuse `scorecard::MAX_ROWS` and `scorecard::ScorecardFilter`.** Do not define
  a new `MAX_ROWS` const or a new filter type in `profile.rs`/`profile_cli.rs`.
- **Do not touch `profile.rs` beyond deleting lines 1-3.** The aggregation logic
  and its tests are approved and frozen.

## Acceptance criteria

- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      and `cargo fmt --all --check` are clean.
- [ ] `cargo test` passes (existing + new).
- [ ] `mcp/src/profile.rs` no longer contains `#![allow(dead_code)]` or its
      removal-note comment: `grep -c 'allow(dead_code)' mcp/src/profile.rs`
      prints `0`.
- [ ] `rexymcp profile --config rexymcp.toml` prints a table whose header
      includes `MODEL`, `TAG`, `AFT`, and `WEAKNESSES`; `--json` prints a JSON
      array deserializable as `Vec<ModelProfile>`.
- [ ] `format_profiles` renders a `spec_bug`/`infra_blip` class **parenthesized**
      and an attributable class (e.g. `prod_unwrap`) **bare**.
- [ ] `model_profile_inner` returns rows whose `total_runs_considered` equals the
      raw run count and whose row count is capped at `scorecard::MAX_ROWS`.
- [ ] `rexymcp profile` produces the same `(model, tag)` buckets the MCP tool
      returns for the same telemetry (both call `aggregate_profiles` on raw
      runs+reviews).

## Test plan

Hermetic. Pin behavior and names, not exact wording/spacing.

`mcp/src/profile_cli.rs` `#[cfg(test)] mod tests` — model the fixtures on
`scorecard_cli.rs`'s tests (write a `phase_runs.jsonl` in a `TempDir`; reviews
are `{"record":"review", …}` lines appended to the same file — see
`server_tests.rs:367-377` for the run-line JSON shape and append a review line):

- `load_profiles_reads_and_aggregates` — a run + a matching
  `approved_first_try` review in the JSONL; the returned profile has the
  expected `(model, tag)` and `approved_first_try_rate == Some(1.0)`.
  Mutation-resistant: fails if `aggregate_profiles` isn't called on the raw
  reviews (the verdict would never fold in).
- `load_profiles_telemetry_disabled_errors` — config with no `[telemetry] dir`
  and no override → `Err` containing `"telemetry disabled"`.
- `format_profiles_shows_strengths_and_weaknesses` — a row with a
  `prod_unwrap×2` weakness and `approved_first_try_rate = Some(0.5)`; output
  contains `MODEL`, the model name, `prod_unwrap`, and `0.50`.
- `format_profiles_marks_non_attributable` — a row whose
  `ranked_failure_classes` holds both `prod_unwrap` and `spec_bug`; assert the
  rendered cell contains `(spec_bug` (parenthesized) and contains `prod_unwrap`
  **not** immediately preceded by `(`. Mutation-resistant on the
  `is_model_attributable` branch.
- `format_profiles_empty_is_no_profiles` — `&[]` → output contains
  `(no profiles)`.

`mcp/src/server_tests.rs` — mirror the `model_scorecard_*` tests using the same
`make_config_with_telemetry` / fixture helpers already in that file:

- `model_profile_success_via_config_telemetry_dir` — fixture present →
  `total_runs_considered` matches the fixture run count, `!truncated`, rows
  non-empty.
- `model_profile_telemetry_path_override_takes_precedence` — an alt
  `--telemetry-path` fixture wins over the config dir.
- `model_profile_telemetry_disabled_returns_error` — no telemetry dir → `Err`.

`mcp/src/main.rs` `#[cfg(test)] mod tests`:

- `cli_parse_profile_collects_filters` — mirror `cli_parse_scorecard_collects_filters`
  (`main.rs:668`): `--model`, repeated `--tag`, `--min-runs`, `--json` parse into
  the `Profile` variant.

## End-to-end verification

This phase ships **two** runtime-loadable artifacts — verify both against the
real binary before reporting complete, and quote the output in the Update Log:

1. **CLI:** run `cargo run -p rexymcp -- profile --config rexymcp.toml` against
   the repo's own telemetry. Quote the printed table (or the `(no profiles)`
   line if the store is empty). Then run it with `--json` and confirm the output
   parses as a JSON array.
2. **MCP tool inner:** the `model_profile_*` server tests exercise
   `model_profile_inner` against a real on-disk fixture — that is the inner-logic
   E2E. (The full stdio MCP round-trip is not separately harnessed here, matching
   how `model_scorecard` is verified.)

If the live telemetry store has no reviews yet, the table may show empty
`WEAKNESSES` columns — that is correct, not a failure; say so rather than
fabricating data.

## Authorizations

- New file `mcp/src/profile_cli.rs`; new `mod profile_cli;` in `main.rs`.
- Edits to `mcp/src/main.rs` (`Commands::Profile` + dispatch arm),
  `mcp/src/server.rs` (`use crate::profile;`, `ModelProfileParams`,
  `ModelProfileOutput`, `model_profile_inner`, the `model_profile` tool method,
  and tests in `server_tests.rs`).
- **Deletion of `#![allow(dead_code)]` + its removal-note comment from
  `mcp/src/profile.rs:1-3`** — required by this phase (phase-03 authorized the
  attribute *only* until this surface landed). No other `profile.rs` change.
- Reuse `crate::scorecard::ScorecardFilter` and `crate::scorecard::MAX_ROWS`.
- No new dependency (`clap`/`schemars`/`serde`/`rmcp` already in use). No edits
  to `telemetry.rs`, `scorecard.rs`, `scorecard_cli.rs`, `profile.rs` aggregation
  logic, `architecture.md`, `Cargo.toml`, `STANDARDS.md`, or `WORKFLOW.md`.

## Out of scope

- Any change to `aggregate_profiles` / `ModelProfile` / `is_model_attributable`
  logic — phase-03 is frozen. This phase only *calls* them.
- Folding the profile into the dashboard, `runs`, or `scorecard` outputs.
- Per-model config-override resolution and runtime-knob wiring — thread 3
  (phases 05-06).
- Auto-invoking the profile from the architect skill / pre-injection tooling —
  a plugin-side concern, not this phase.
- Adding `failure_class` to `PhaseRun` (deliberately not done since phase-01).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
