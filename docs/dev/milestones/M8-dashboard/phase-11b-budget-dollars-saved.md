# Phase 11b: Budget panel — "$ saved" (configurable $/Mtok)

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** phase-11a (done — the Budget panel and `last_input_tokens` /
`last_output_tokens` this reads).
**Estimated diff:** ~150 lines (`executor/src/config.rs` new section + `mcp/src/main.rs`
config load + `mcp/src/dashboard.rs` rates threading + line + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the wireframe's **"$ saved"** line to the Budget panel: the dollar cost the same
token usage *would* have incurred on a cloud model, which a local run avoids. The
baseline is a **configurable `$/Mtok` rate** in `rexymcp.toml` (locked decision —
model-agnostic, **not** a hardcoded named-model preset). The dashboard does not load
config today, so this phase also wires config into the `dashboard` CLI command.

The final redesign piece. After this, the Budget panel matches the wireframe (tokens
in/out · context · tok/s · $ saved).

## Architecture references

Read before starting:

- `executor/src/config.rs` — the `Config` struct (and its `#[serde(default)]` +
  per-section `Default` pattern) you extend with a `[dashboard]` section.
- `mcp/src/main.rs` — the `Dashboard` CLI command and how sibling commands (e.g.
  `Health`) load config via `Config::load_with_env`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `executor/src/config.rs` (the struct defs + the `#[cfg(test)]` config tests).
3. Read `mcp/src/dashboard.rs`'s `budget_lines` + `render_dashboard` + `run_loop` +
   `run_dashboard`, and `mcp/src/main.rs`'s `Dashboard` command.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### Config section pattern (`executor/src/config.rs`)

`Config` is `#[serde(default)]`, so a **missing** `[dashboard]` section falls back to
`Default` — purely additive. Each section is its own struct with a `Default` impl, e.g.:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig { … }
impl Default for BudgetConfig {
    fn default() -> Self { Self { context_length: 32768, max_context_pct: 70, … } }
}
```

`Config::load` returns `Config::default()` when the file is absent (`if path.exists()`),
so the dashboard still runs with no `rexymcp.toml` (rates default → "$ saved: —").

### The `Dashboard` command does not load config (`mcp/src/main.rs`)

```rust
    Dashboard {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        session: Option<String>,
    },
```
```rust
        Commands::Dashboard { repo, session } => {
            dashboard::run_dashboard(&repo, session.as_deref()).unwrap_or_else(|e| {
                eprintln!("dashboard error: {e}");
                std::process::exit(1);
            });
            Ok(())
        }
```

The `Health` command is the worked example for loading config:
```rust
        Commands::Health { config, base_url } => {
            let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
            let mut cfg = Config::load_with_env(&config_path)?;
            …
```

### The Budget panel + render assembly (`mcp/src/dashboard.rs`)

`budget_lines(summary)` builds tokens / tok-s / context lines and is called in
`render_dashboard` **and in 5 unit tests**. The dashboard fns thread to it:
`run_dashboard(repo, session)` → `run_loop(terminal, repo, session)` →
`render_dashboard(frame, area, data, now_ms, offset, follow)` →
`budget_lines(&data.summary)`. The `tokens_per_sec` helper (11a) shows the scalar-arg
shape for a testable pure computation.

## Spec

Numbered tasks. **Additive change shape — see task 4: do NOT change `budget_lines`'s
signature** (it has 6 call sites incl. 5 tests; mutating it is needless churn). Append
the "$ saved" line where the Budget panel is assembled instead.

1. **Add a `[dashboard]` config section** — in `executor/src/config.rs`:
   ```rust
   /// Live-dashboard settings. The "$ saved" baseline: cloud $/million-tokens the
   /// local run is priced against. Default 0.0 → the dashboard shows "— " (unset).
   #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
   #[serde(default)]
   pub struct DashboardConfig {
       /// USD per million **input** tokens for the cloud baseline.
       pub saved_input_per_mtok: f64,
       /// USD per million **output** tokens for the cloud baseline.
       pub saved_output_per_mtok: f64,
   }

   impl Default for DashboardConfig {
       fn default() -> Self {
           Self { saved_input_per_mtok: 0.0, saved_output_per_mtok: 0.0 }
       }
   }
   ```
   Add `pub dashboard: DashboardConfig,` as a field on `Config` (after `telemetry`).

2. **Add a `BudgetRates` carrier + `dollars_saved` helper** — in `mcp/src/dashboard.rs`:
   ```rust
   /// Cloud-baseline $/Mtok rates for the Budget panel's "$ saved" line.
   #[derive(Debug, Clone, Copy, Default)]
   pub struct BudgetRates {
       pub input_per_mtok: f64,
       pub output_per_mtok: f64,
   }

   /// Dollar cost the given cumulative token usage would incur at the cloud baseline.
   fn dollars_saved(input_tokens: u32, output_tokens: u32, in_per_mtok: f64, out_per_mtok: f64) -> f64 {
       (input_tokens as f64 / 1_000_000.0) * in_per_mtok
           + (output_tokens as f64 / 1_000_000.0) * out_per_mtok
   }
   ```

3. **Add a `dollars_saved_line` builder** — in `mcp/src/dashboard.rs`, a pure
   `Option<Line<'static>>` (returns `None` when there is no metric yet, so nothing is
   appended; shows `—` when the rates are unconfigured):
   ```rust
   fn dollars_saved_line(summary: &StatusSummary, rates: BudgetRates) -> Option<Line<'static>> {
       let in_tok = summary.last_input_tokens?; // None ⇒ no metrics yet ⇒ no line
       let out_tok = summary.last_output_tokens.unwrap_or(0);
       if rates.input_per_mtok == 0.0 && rates.output_per_mtok == 0.0 {
           return Some(Line::from("$ saved: —"));
       }
       let saved = dollars_saved(in_tok, out_tok, rates.input_per_mtok, rates.output_per_mtok);
       Some(Line::from(format!("$ saved: ${saved:.2}")))
   }
   ```
   **Pin the cases:** `None` when `last_input_tokens` is `None`; `"$ saved: —"` when
   both rates are exactly `0.0` (and there are metrics); `"$ saved: ${:.2}"` otherwise.

4. **Append the line where the Budget panel is assembled** — in `render_dashboard`,
   replace the single-expression budget render with an assemble-then-append (this is
   the **only** change to the panel; `budget_lines` itself is untouched):
   ```rust
   let mut budget = budget_lines(&data.summary);
   if let Some(line) = dollars_saved_line(&data.summary, rates) {
       budget.push(line);
   }
   frame.render_widget(panel(" Budget ", budget), budget_area);
   ```

5. **Thread `rates` through the dashboard entry points** — add a `rates: BudgetRates`
   parameter to `render_dashboard` (after `follow`), `run_loop` (after `session`), and
   `run_dashboard` (after `session`); pass it straight through. These three fns have
   **no unit-test callers** (they need a terminal), so this threading touches no tests.

6. **Load config in the `Dashboard` CLI command** — in `mcp/src/main.rs`:
   - Add a `--config` arg to the `Dashboard` variant (mirror `Health`'s `config:
     Option<PathBuf>`).
   - In the handler, load config and build the rates (mirror `Health`):
     ```rust
     Commands::Dashboard { repo, session, config } => {
         let config_path = config.unwrap_or_else(|| PathBuf::from("rexymcp.toml"));
         let cfg = Config::load_with_env(&config_path)?;
         let rates = dashboard::BudgetRates {
             input_per_mtok: cfg.dashboard.saved_input_per_mtok,
             output_per_mtok: cfg.dashboard.saved_output_per_mtok,
         };
         dashboard::run_dashboard(&repo, session.as_deref(), rates).unwrap_or_else(|e| {
             eprintln!("dashboard error: {e}");
             std::process::exit(1);
         });
         Ok(())
     }
     ```
   - Update the existing `cli_parse_dashboard_collects_args` test for the new `config`
     field (it destructures `Commands::Dashboard { repo, session }` — add `config` /
     `..`). This is the only existing test that needs touching.

## Acceptance criteria

- [ ] `Config` has a `dashboard: DashboardConfig` with `saved_input_per_mtok` /
      `saved_output_per_mtok`, both defaulting to `0.0`; a missing `[dashboard]` section
      loads the defaults.
- [ ] `dollars_saved` computes `(in/1e6)*in_rate + (out/1e6)*out_rate`.
- [ ] `dollars_saved_line` is `None` without metrics, `"$ saved: —"` when both rates are
      `0.0`, and `"$ saved: ${:.2}"` otherwise.
- [ ] The Budget panel shows the `$ saved` line (appended in `render_dashboard`);
      `budget_lines`'s signature is unchanged.
- [ ] The `dashboard` CLI command accepts `--config` and loads the rates.
- [ ] `cargo build` clean; clippy `-D warnings` clean; `cargo fmt --all --check` clean
      (use `rustfmt` only on touched files); `cargo test` passes (executor + mcp).

## Test plan

- `dashboard_config_defaults_to_zero` (executor `config.rs`) — `Config::default()
  .dashboard.saved_input_per_mtok == 0.0` and `…saved_output_per_mtok == 0.0`.
- `config_loads_dashboard_rates` (executor `config.rs`) — write a temp toml with a
  `[dashboard]\nsaved_input_per_mtok = 3.0\nsaved_output_per_mtok = 15.0` section
  (mirror `config_loads_overridden_timeouts`'s tempdir+`Config::load` style); assert
  both rates parse. Include the other required sections in the toml as that test does.
- `dollars_saved_computes_cost` (dashboard.rs) —
  `dollars_saved(1_000_000, 500_000, 3.0, 15.0)` → `10.5` (3.0 + 7.5).
- `dollars_saved_line_none_without_metrics` (dashboard.rs) — `StatusSummary::default()`
  + any rates → `None`.
- `dollars_saved_line_dash_when_rates_unset` (dashboard.rs) — summary with
  `last_input_tokens: Some(_)` + `BudgetRates::default()` → line text is `$ saved: —`
  (the must-NOT-show-a-dollar-figure negative case).
- `dollars_saved_line_shows_dollars` (dashboard.rs) — summary with
  `last_input_tokens: Some(1_000_000)`, `last_output_tokens: Some(500_000)` and rates
  `{3.0, 15.0}` → line text contains `$ saved: $10.50`.

Update `cli_parse_dashboard_collects_args` in `main.rs` for the new `config` field. The
live render is review-by-inspection (TTY).

## End-to-end verification

The cost math, the placeholder/figure branches, and config parsing are proven by unit
tests. Verify the real artifact:

1. Paste `cargo test` output (executor + mcp) covering the new `dollars_saved*`,
   `dashboard_config_*` / `config_loads_dashboard_rates` tests.
2. Add a `[dashboard]` section to this repo's `rexymcp.toml` (e.g.
   `saved_input_per_mtok = 3.0`, `saved_output_per_mtok = 15.0`) and launch against a
   session log with metrics:
   `cargo run -p rexymcp -- dashboard --repo .` — confirm the Budget panel shows a
   `$ saved: $…` figure; quote it. Then note that **without** the section it shows
   `$ saved: —`. (If you add the section for the test, leave it in `rexymcp.toml` as a
   working example, or revert it — your call; note which in "Notes for review".)

## Authorizations

- [x] May touch `executor/src/config.rs` (add the `[dashboard]` section) — this is a
      rexyMCP-internal config-schema change, in scope for this phase.

No new dependency. No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edit.

## Out of scope

- **A named-model price preset / price table** — the locked decision is a configurable
  `$/Mtok` rate only. Do not bake in any model's prices.
- **Changing `budget_lines`'s signature** — append the `$ saved` line in
  `render_dashboard` (Spec task 4). Leave `budget_lines` and its 5 tests alone.
- **Per-token cost breakdown, currency formatting/localization, cost-rate-over-time** —
  a single `$ saved: $X.XX` (USD, 2dp) line is the whole scope.
- **Reworking Tokens/Sec (11a), the Activity transcript, or any other panel.**
- **Loading config anywhere other than the `dashboard` CLI command.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
