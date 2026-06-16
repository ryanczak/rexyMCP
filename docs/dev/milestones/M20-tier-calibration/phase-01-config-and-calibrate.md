# Phase 01: Config schema, `/calibrate` CLI, and known-model registry

**Milestone:** M20 — Tier Calibration and Cost Visibility
**Status:** todo
**Depends on:** none (pure config / CLI layer; no executor loop changes)
**Estimated diff:** ~380 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the config schema for the tier system (`Tier` enum, `EscalationConfig`,
`ArchitectConfig`) to `executor/src/config.rs`, move the known-model rate
registry there so both `DashboardConfig` and `ArchitectConfig` share it, and
implement the `rexymcp calibrate LARGE|MEDIUM|SMALL` CLI command that writes
the correct tier and tier-derived defaults to `rexymcp.toml`.

No executor loop behavior changes in this phase. The tier is parsed and
available to downstream code but nothing reads it yet — that is M21's job.

## Architecture references

- `executor/src/config.rs` — the single file that owns all config structs.
  New types (`Tier`, `EscalationConfig`, `ArchitectConfig`) go here alongside
  `BudgetConfig`, `DashboardConfig`, etc.
- `mcp/src/dashboard/panels.rs:19–49` — existing `BudgetRates` struct and
  `model_rates(model: &str) -> Option<BudgetRates>`. The known-model table
  inside `model_rates` moves to `executor/src/config.rs` as
  `pub fn known_model_rates(model: &str) -> Option<(f64, f64)>`. The
  dashboard's `model_rates` becomes a thin wrapper that calls it.
- `mcp/src/main.rs` — clap `Commands` enum and `main()`. Gains a `Calibrate`
  variant and its match arm, delegating to `mcp/src/calibrate.rs`.
- `mcp/Cargo.toml` — add `toml_edit` (authorized below).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo build` to confirm the tree is clean.

## Current state

### `executor/src/config.rs`

The `Config` struct has eight sections today:

```
project, executor, commands, budget, telemetry, dashboard, context, governor, models
```

`ExecutorConfig` has no tier field. `BudgetConfig` has `max_turns` and
`escalation_slots` but no `gate_retries`. `DashboardConfig` has `saved_model`
and explicit rate fields but the rate-lookup logic lives in
`mcp/src/dashboard/panels.rs`, not in the executor crate where it can be
shared.

`BudgetConfig` does **not** carry `#[serde(default)]` on the struct — it
requires all four fields to be present in TOML. New optional fields added to
it must carry `#[serde(default)]` on the individual field (not the struct) so
existing configs without the field still parse.

### `mcp/src/dashboard/panels.rs:19–49`

```rust
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

pub fn model_rates(model: &str) -> Option<BudgetRates> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some(BudgetRates { ... }),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some(...),
        "claude-sonnet-4-6" => Some(...),
        "claude-haiku-4-5" => Some(...),
        _ => None,
    }
}
```

The rate table is duplicated nowhere today — but once `ArchitectConfig` also
needs it, duplication becomes a maintenance risk. Moving the table to
`executor/src/config.rs` keeps it in one place and makes it available to the
executor lib without an `mcp`-layer dependency.

### `mcp/src/main.rs`

CLI commands follow a consistent pattern: a variant in `Commands`, a handler
module (e.g. `mcp/src/review.rs`), and a match arm in `main()` that constructs
the args struct and calls the module. The `Calibrate` command follows this
same pattern.

## Spec

### Task 1 — `known_model_rates` in `executor/src/config.rs`

Add a public function **before** the `Config` struct:

```rust
/// Returns `(input_per_mtok, output_per_mtok)` in USD/MTok for known Claude
/// model IDs. Used by both `DashboardConfig` and `ArchitectConfig` so the
/// rate table lives in one place.
pub fn known_model_rates(model: &str) -> Option<(f64, f64)> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some((10.0, 50.0)),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some((5.0, 25.0)),
        "claude-sonnet-4-6" => Some((3.0, 15.0)),
        "claude-haiku-4-5" | "claude-haiku-4-5-20251001" => Some((1.0, 5.0)),
        _ => None,
    }
}
```

### Task 2 — `Tier` enum in `executor/src/config.rs`

Add after `known_model_rates`:

```rust
/// Executor capability tier. Set via `rexymcp calibrate` and recorded in
/// `[executor].tier`. Controls default `max_turns`, `gate_retries`, and
/// whether mid-phase Architect escalation is enabled (SMALL only, wired in M21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Tier {
    Large,
    Medium,
    Small,
}

impl Tier {
    /// Default `max_turns` for this tier when not explicitly set in `[budget]`.
    pub fn default_max_turns(self) -> u32 {
        match self {
            Tier::Large => 400,
            Tier::Medium => 250,
            Tier::Small => 100,
        }
    }

    /// Default `gate_retries` for this tier when not explicitly set in `[budget]`.
    /// `u32::MAX` means retry until `max_turns` is exhausted (LARGE behavior).
    pub fn default_gate_retries(self) -> u32 {
        match self {
            Tier::Large => u32::MAX,
            Tier::Medium => 2,
            Tier::Small => 1,
        }
    }
}
```

### Task 3 — `EscalationConfig` and `ArchitectConfig` in `executor/src/config.rs`

Add both structs after `Tier`:

```rust
/// SMALL-tier escalation settings. When `tier = "SMALL"`, the executor fires
/// up to `max_assists` autonomous Architect assists before hard-failing. Absent
/// or ignored for MEDIUM and LARGE tiers (wired in M21).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct EscalationConfig {
    /// Maximum autonomous mid-phase Architect assists before hard_fail.
    pub max_assists: u32,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self { max_assists: 3 }
    }
}

/// The model used for Architect escalation assists. Separate from `[dashboard]`
/// which is the hypothetical cloud baseline — this is a real cost center.
/// When `model` matches a known Claude model ID, rates are auto-filled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ArchitectConfig {
    /// Claude model ID for Architect assists (e.g. `"claude-opus-4-8"`).
    /// When recognised, auto-fills `input_per_mtok` / `output_per_mtok`.
    pub model: Option<String>,
    /// USD per million input tokens (overridden by `model` when recognised).
    pub input_per_mtok: f64,
    /// USD per million output tokens (overridden by `model` when recognised).
    pub output_per_mtok: f64,
}

impl Default for ArchitectConfig {
    fn default() -> Self {
        Self {
            model: None,
            input_per_mtok: 0.0,
            output_per_mtok: 0.0,
        }
    }
}

impl ArchitectConfig {
    /// Resolved `(input_per_mtok, output_per_mtok)`: model lookup wins when
    /// the model ID is recognised; explicit fields win otherwise.
    pub fn effective_rates(&self) -> (f64, f64) {
        self.model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.input_per_mtok, self.output_per_mtok))
    }
}
```

### Task 4 — Modify existing structs in `executor/src/config.rs`

**`ExecutorConfig`** — add `tier` field after `task_tracking`:

```rust
/// Executor capability tier. `None` = no tier configured (behavior unchanged
/// from pre-M20). Set via `rexymcp calibrate`.
#[serde(default)]
pub tier: Option<Tier>,
```

Update `ExecutorConfig::default()` to include `tier: None`.

**`BudgetConfig`** — add `gate_retries` field:

```rust
/// Max gate-retry loops at completion time before escalation. `None` = derive
/// from `executor.tier`; if tier is also `None`, unlimited (bounded by
/// `max_turns`). Set explicitly to override tier default.
#[serde(default)]
pub gate_retries: Option<u32>,
```

Add an `impl BudgetConfig` block with a helper (add after the existing
`Default` impl):

```rust
impl BudgetConfig {
    /// Resolved gate_retries: explicit field wins; falls back to tier default;
    /// falls back to `u32::MAX` (unlimited, bounded by `max_turns`).
    pub fn effective_gate_retries(&self, tier: Option<Tier>) -> u32 {
        self.gate_retries
            .or_else(|| tier.map(|t| t.default_gate_retries()))
            .unwrap_or(u32::MAX)
    }

    /// Resolved max_turns: explicit field wins; falls back to tier default.
    /// `BudgetConfig.max_turns` always has a value (it has no `Option` wrapper)
    /// so this only matters when the TOML omits `max_turns` entirely. Current
    /// configs always set it, but future `/calibrate` writes will omit it and
    /// rely on this resolution.
    pub fn effective_max_turns(&self, tier: Option<Tier>) -> u32 {
        // max_turns is already resolved from TOML (or its Default impl).
        // This helper is the future hook for tier-derived defaults; for now
        // it just returns the stored value.
        let _ = tier; // reserved for M21 when calibrate stops writing max_turns
        self.max_turns
    }
}
```

**`DashboardConfig`** — update `effective_rates` to delegate to `known_model_rates`
(add an `impl DashboardConfig` block):

```rust
impl DashboardConfig {
    /// Resolved `(input_per_mtok, output_per_mtok)` for the cloud baseline.
    /// `saved_model` lookup wins; explicit fields win otherwise.
    pub fn effective_rates(&self) -> (f64, f64) {
        self.saved_model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.saved_input_per_mtok, self.saved_output_per_mtok))
    }
}
```

**`Config`** — add two new fields:

```rust
pub escalation: EscalationConfig,
pub architect: ArchitectConfig,
```

Both are `#[serde(default)]` on their structs, so existing configs without
these sections continue to parse without error.

### Task 5 — Update `DashboardConfig` rate lookup in `mcp/src/dashboard/panels.rs`

The `model_rates` function currently contains the known-model table directly.
Replace its body to delegate to the shared registry:

```rust
pub fn model_rates(model: &str) -> Option<BudgetRates> {
    rexymcp_executor::config::known_model_rates(model).map(|(i, o)| BudgetRates {
        input_per_mtok: i,
        output_per_mtok: o,
    })
}
```

No other changes to `panels.rs`. `BudgetRates` stays in `panels.rs` — it is
the mcp-layer display type; the executor only needs the raw `(f64, f64)` tuple.

Also update `mcp/src/main.rs`'s `Dashboard` command handler to use
`cfg.dashboard.effective_rates()` instead of the inline `model_rates` call:

```rust
// replace:
let rates = d
    .saved_model
    .as_deref()
    .and_then(dashboard::model_rates)
    .unwrap_or(dashboard::BudgetRates {
        input_per_mtok: d.saved_input_per_mtok,
        output_per_mtok: d.saved_output_per_mtok,
    });
// with:
let (i, o) = cfg.dashboard.effective_rates();
let rates = dashboard::BudgetRates {
    input_per_mtok: i,
    output_per_mtok: o,
};
```

### Task 6 — `mcp/src/calibrate.rs` (new file)

```rust
use rexymcp_executor::config::Tier;
use std::path::Path;
use toml_edit::{DocumentMut, value};

pub struct CalibrateArgs<'a> {
    pub tier: Tier,
    pub config_path: &'a Path,
}

pub fn run(args: &CalibrateArgs<'_>) -> anyhow::Result<()> {
    let src = if args.config_path.exists() {
        std::fs::read_to_string(args.config_path)?
    } else {
        String::new()
    };

    let mut doc: DocumentMut = src.parse().map_err(|e| anyhow::anyhow!("TOML parse error: {e}"))?;

    // [executor].tier
    doc["executor"]["tier"] = value(tier_str(args.tier));

    // [budget] tier-derived defaults — only write when the key is absent so an
    // explicit override survives a re-calibrate.
    let max_turns = args.tier.default_max_turns();
    if doc.get("budget").and_then(|b| b.get("max_turns")).is_none() {
        doc["budget"]["max_turns"] = value(max_turns as i64);
    }
    // gate_retries: write only when absent; skip for Large (u32::MAX is implicit).
    let gate_retries = args.tier.default_gate_retries();
    if gate_retries != u32::MAX
        && doc.get("budget").and_then(|b| b.get("gate_retries")).is_none()
    {
        doc["budget"]["gate_retries"] = value(gate_retries as i64);
    }

    // [escalation] — write only for Small; remove for Medium/Large (absent = ignored).
    match args.tier {
        Tier::Small => {
            if doc.get("escalation").is_none() {
                doc["escalation"]["max_assists"] = value(3i64);
            }
        }
        _ => {
            doc.remove("escalation");
        }
    }

    // [architect] — add skeleton when absent so the user sees the section.
    if doc.get("architect").is_none() {
        doc["architect"]["model"] = value("");
        doc["architect"]["input_per_mtok"] = value(0.0);
        doc["architect"]["output_per_mtok"] = value(0.0);
    }

    std::fs::write(args.config_path, doc.to_string())?;

    println!(
        "Calibrated to {tier} — updated executor.tier={tier_s}, budget.max_turns={max_turns}{}{}",
        if gate_retries != u32::MAX {
            format!(", budget.gate_retries={gate_retries}")
        } else {
            String::new()
        },
        match args.tier {
            Tier::Small => ", escalation.max_assists=3",
            _ => "",
        },
        tier = tier_label(args.tier),
        tier_s = tier_str(args.tier),
    );
    Ok(())
}

fn tier_str(tier: Tier) -> &'static str {
    match tier {
        Tier::Large => "LARGE",
        Tier::Medium => "MEDIUM",
        Tier::Small => "SMALL",
    }
}

fn tier_label(tier: Tier) -> &'static str {
    tier_str(tier)
}
```

**Note on `toml_edit` comment preservation:** `toml_edit` preserves existing
comments and whitespace. The skeleton `[architect]` block written for a first-
time config will have no comments; that is expected and acceptable — users add
their `model =` value directly.

**Note on `doc.remove("escalation")`:** For MEDIUM/LARGE, we remove an
`[escalation]` section if it was previously written by a SMALL calibration.
This keeps the config clean. If the user has added custom keys under
`[escalation]`, they are lost — that is acceptable because `[escalation]` is a
tier-managed block.

### Task 7 — `Calibrate` command in `mcp/src/main.rs`

Add to the `mod` list: `mod calibrate;`

Add to `Commands`:

```rust
/// Set the executor capability tier and write tier-derived defaults to the
/// config file
Calibrate {
    /// Capability tier: LARGE (Deepseek/Qwen3.6+), MEDIUM (Qwen3.6-27B /
    /// Gemma4-31b), or SMALL (Qwen3.5-coder-12b / Gemma-12b)
    #[arg(value_enum)]
    tier: CalibrateArg,

    /// Path to the config file
    #[arg(long, default_value = "rexymcp.toml")]
    config: PathBuf,
},
```

Add `CalibrateArg` enum (clap `ValueEnum`) **above** the `Commands` enum:

```rust
#[derive(clap::ValueEnum, Clone, Copy)]
enum CalibrateArg {
    #[value(name = "LARGE")]
    Large,
    #[value(name = "MEDIUM")]
    Medium,
    #[value(name = "SMALL")]
    Small,
}

impl From<CalibrateArg> for rexymcp_executor::config::Tier {
    fn from(a: CalibrateArg) -> Self {
        match a {
            CalibrateArg::Large => Self::Large,
            CalibrateArg::Medium => Self::Medium,
            CalibrateArg::Small => Self::Small,
        }
    }
}
```

Add the match arm in `main()`:

```rust
Commands::Calibrate { tier, config } => {
    calibrate::run(&calibrate::CalibrateArgs {
        tier: tier.into(),
        config_path: &config,
    })?;
    Ok(())
}
```

### Task 8 — Add `toml_edit` to `mcp/Cargo.toml`

Append to `[dependencies]`:

```toml
toml_edit = "0.22"
```

Do not add to the workspace `Cargo.toml` — `toml_edit` is only needed by the
`mcp` binary for write-back; the executor library uses the existing `toml`
crate for read-only parsing.

### Task 9 — Tests

**In `executor/src/config.rs` `#[cfg(test)] mod tests`:**

```rust
#[test]
fn known_model_rates_returns_opus_rates() {
    let (i, o) = known_model_rates("claude-opus-4-8").expect("opus must be known");
    assert_eq!(i, 5.0);
    assert_eq!(o, 25.0);
}

#[test]
fn known_model_rates_returns_none_for_unknown() {
    assert!(known_model_rates("some-local-llm").is_none());
}

#[test]
fn tier_default_max_turns_correct() {
    assert_eq!(Tier::Large.default_max_turns(), 400);
    assert_eq!(Tier::Medium.default_max_turns(), 250);
    assert_eq!(Tier::Small.default_max_turns(), 100);
}

#[test]
fn tier_default_gate_retries_correct() {
    assert_eq!(Tier::Large.default_gate_retries(), u32::MAX);
    assert_eq!(Tier::Medium.default_gate_retries(), 2);
    assert_eq!(Tier::Small.default_gate_retries(), 1);
}

#[test]
fn budget_effective_gate_retries_explicit_wins() {
    let b = BudgetConfig { gate_retries: Some(5), ..BudgetConfig::default() };
    assert_eq!(b.effective_gate_retries(Some(Tier::Small)), 5);
}

#[test]
fn budget_effective_gate_retries_falls_back_to_tier() {
    let b = BudgetConfig { gate_retries: None, ..BudgetConfig::default() };
    assert_eq!(b.effective_gate_retries(Some(Tier::Medium)), 2);
}

#[test]
fn budget_effective_gate_retries_unlimited_when_no_tier() {
    let b = BudgetConfig { gate_retries: None, ..BudgetConfig::default() };
    assert_eq!(b.effective_gate_retries(None), u32::MAX);
}

#[test]
fn architect_effective_rates_uses_known_model() {
    let a = ArchitectConfig {
        model: Some("claude-opus-4-8".into()),
        input_per_mtok: 0.0,
        output_per_mtok: 0.0,
    };
    assert_eq!(a.effective_rates(), (5.0, 25.0));
}

#[test]
fn architect_effective_rates_falls_back_to_explicit() {
    let a = ArchitectConfig {
        model: Some("unknown-model".into()),
        input_per_mtok: 2.5,
        output_per_mtok: 12.5,
    };
    assert_eq!(a.effective_rates(), (2.5, 12.5));
}

#[test]
fn config_parses_tier_from_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(&path, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"
tier = "MEDIUM"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#).unwrap();
    let cfg = Config::load(&path).unwrap();
    assert_eq!(cfg.executor.tier, Some(Tier::Medium));
}

#[test]
fn config_tier_absent_is_none() {
    let cfg = Config::default();
    assert_eq!(cfg.executor.tier, None);
}

#[test]
fn config_parses_escalation_and_architect_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(&path, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1

[escalation]
max_assists = 5

[architect]
model = "claude-opus-4-8"
"#).unwrap();
    let cfg = Config::load(&path).unwrap();
    assert_eq!(cfg.escalation.max_assists, 5);
    assert_eq!(cfg.architect.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(cfg.architect.effective_rates(), (5.0, 25.0));
}

#[test]
fn config_escalation_absent_uses_default() {
    let cfg = Config::default();
    assert_eq!(cfg.escalation.max_assists, 3);
}

#[test]
fn dashboard_effective_rates_uses_known_model() {
    let d = DashboardConfig {
        saved_model: Some("claude-sonnet-4-6".into()),
        ..DashboardConfig::default()
    };
    assert_eq!(d.effective_rates(), (3.0, 15.0));
}
```

**In `mcp/src/main.rs` `#[cfg(test)] mod tests`** — add CLI parsing tests for
the new `Calibrate` command:

```rust
#[test]
fn cli_parse_calibrate_medium() {
    let cli = Cli::try_parse_from(["rexymcp", "calibrate", "MEDIUM"]).unwrap();
    match cli.command {
        Some(Commands::Calibrate { tier, config }) => {
            assert!(matches!(tier, CalibrateArg::Medium));
            assert_eq!(config, PathBuf::from("rexymcp.toml"));
        }
        _ => panic!("expected Calibrate"),
    }
}

#[test]
fn cli_parse_calibrate_small_with_config() {
    let cli = Cli::try_parse_from([
        "rexymcp", "calibrate", "SMALL", "--config", "/path/rexymcp.toml",
    ])
    .unwrap();
    match cli.command {
        Some(Commands::Calibrate { tier, config }) => {
            assert!(matches!(tier, CalibrateArg::Small));
            assert_eq!(config, PathBuf::from("/path/rexymcp.toml"));
        }
        _ => panic!("expected Calibrate"),
    }
}

#[test]
fn cli_parse_calibrate_missing_tier_fails() {
    let result = Cli::try_parse_from(["rexymcp", "calibrate"]);
    assert!(result.is_err());
}
```

**In `mcp/src/calibrate.rs` tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::config::Tier;
    use tempfile::TempDir;

    fn run_calibrate(dir: &TempDir, tier: Tier, initial: &str) -> String {
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(&path, initial).unwrap();
        run(&CalibrateArgs { tier, config_path: &path }).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn calibrate_medium_sets_tier_and_budget() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Medium, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 200
escalation_slots = 1
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("MEDIUM"));
        // max_turns already set — calibrate does NOT overwrite it
        assert_eq!(doc["budget"]["max_turns"].as_integer(), Some(200));
        // gate_retries written for Medium
        assert_eq!(doc["budget"]["gate_retries"].as_integer(), Some(2));
    }

    #[test]
    fn calibrate_small_adds_escalation_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Small, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 200
escalation_slots = 1
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("SMALL"));
        assert_eq!(doc["escalation"]["max_assists"].as_integer(), Some(3));
    }

    #[test]
    fn calibrate_medium_removes_escalation_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Medium, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 100
escalation_slots = 1

[escalation]
max_assists = 3
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert!(doc.get("escalation").is_none(), "[escalation] must be removed for MEDIUM");
    }

    #[test]
    fn calibrate_large_does_not_write_gate_retries() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Large, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 131071
max_context_pct = 80
max_turns = 400
escalation_slots = 1
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["executor"]["tier"].as_str(), Some("LARGE"));
        // gate_retries must not be written for Large (unlimited is the default)
        assert!(doc.get("budget").and_then(|b| b.get("gate_retries")).is_none());
    }

    #[test]
    fn calibrate_adds_architect_skeleton_when_absent() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Medium, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert!(doc.get("architect").is_some(), "[architect] skeleton must be added");
    }

    #[test]
    fn calibrate_preserves_existing_architect_section() {
        let dir = TempDir::new().unwrap();
        let result = run_calibrate(&dir, Tier::Medium, r#"
[executor]
provider = "openai"
model = "m"
base_url = "http://localhost:1234/v1"

[budget]
context_length = 32768
max_context_pct = 70
max_turns = 40
escalation_slots = 1

[architect]
model = "claude-opus-4-8"
input_per_mtok = 5.0
output_per_mtok = 25.0
"#);
        let doc: toml_edit::DocumentMut = result.parse().unwrap();
        assert_eq!(doc["architect"]["model"].as_str(), Some("claude-opus-4-8"));
    }
}
```

## Acceptance criteria

- [ ] `known_model_rates("claude-opus-4-8")` returns `Some((5.0, 25.0))`.
- [ ] `known_model_rates("some-local-llm")` returns `None`.
- [ ] `Tier::Medium.default_max_turns()` returns `250`.
- [ ] `Tier::Small.default_gate_retries()` returns `1`.
- [ ] `BudgetConfig::effective_gate_retries(Some(Tier::Medium))` returns `2`
      when `gate_retries` is `None`.
- [ ] `BudgetConfig::effective_gate_retries(None)` returns `u32::MAX` when
      `gate_retries` is also `None`.
- [ ] `ArchitectConfig` with `model = "claude-opus-4-8"` returns `(5.0, 25.0)`
      from `effective_rates()`, ignoring explicit zero rates.
- [ ] `Config::load` parses `tier = "MEDIUM"` in `[executor]` to
      `Some(Tier::Medium)`; absence parses to `None`.
- [ ] `Config::load` parses `[escalation]` and `[architect]` sections; both
      absent in a config file is not an error.
- [ ] `rexymcp calibrate MEDIUM` writes `tier = "MEDIUM"` and
      `gate_retries = 2` to `[executor]` and `[budget]` respectively.
- [ ] `rexymcp calibrate SMALL` also writes `[escalation] max_assists = 3`.
- [ ] `rexymcp calibrate LARGE` does NOT write a `gate_retries` key.
- [ ] Re-calibrating from SMALL to MEDIUM removes the `[escalation]` section.
- [ ] Calibrate on a config with an existing `[architect]` section preserves it.
- [ ] Existing configs without `tier`, `[escalation]`, or `[architect]` continue
      to parse without error (backward compatibility).
- [ ] `cargo fmt --all --check` exits 0.
- [ ] `cargo build` exits 0 with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0.
- [ ] `cargo test` passes; count rises by at least 20 new tests.

## Test plan

Unit tests in `executor/src/config.rs`:
- `known_model_rates_returns_opus_rates`
- `known_model_rates_returns_none_for_unknown`
- `tier_default_max_turns_correct`
- `tier_default_gate_retries_correct`
- `budget_effective_gate_retries_explicit_wins`
- `budget_effective_gate_retries_falls_back_to_tier`
- `budget_effective_gate_retries_unlimited_when_no_tier`
- `architect_effective_rates_uses_known_model`
- `architect_effective_rates_falls_back_to_explicit`
- `config_parses_tier_from_toml`
- `config_tier_absent_is_none`
- `config_parses_escalation_and_architect_sections`
- `config_escalation_absent_uses_default`
- `dashboard_effective_rates_uses_known_model`

CLI parsing tests in `mcp/src/main.rs`:
- `cli_parse_calibrate_medium`
- `cli_parse_calibrate_small_with_config`
- `cli_parse_calibrate_missing_tier_fails`

Integration tests in `mcp/src/calibrate.rs`:
- `calibrate_medium_sets_tier_and_budget`
- `calibrate_small_adds_escalation_section`
- `calibrate_medium_removes_escalation_section`
- `calibrate_large_does_not_write_gate_retries`
- `calibrate_adds_architect_skeleton_when_absent`
- `calibrate_preserves_existing_architect_section`

## End-to-end verification

Run `cargo run -p rexymcp -- calibrate MEDIUM --config /tmp/test.toml` on a
minimal config file and confirm:
1. Exit 0, confirmation line printed.
2. `[executor].tier = "MEDIUM"` is in the written file.
3. `[budget].gate_retries = 2` is in the written file.
4. `[architect]` skeleton is present.
5. Re-run `cargo run -p rexymcp -- calibrate SMALL --config /tmp/test.toml`
   and confirm `[escalation].max_assists = 3` is added and tier updated to
   `"SMALL"`.

Quote the terminal output of both runs in the Update Log completion entry.

## Authorizations

- **Add `toml_edit = "0.22"` to `mcp/Cargo.toml`** — required for
  structure-preserving write-back to `rexymcp.toml`. Do not add to
  `executor/Cargo.toml` or the workspace root; the executor library uses
  the existing `toml` crate for read-only parsing and must not gain a
  write-back dependency.

## Out of scope

- Using `tier` or `gate_retries` in the executor loop — that is M21.
- `escalation_request` `PhaseResult` variant and autonomous dispatch loop — M21.
- Telemetry fields (`tier`, `doc_level`, `escalation_count`, etc.) — M20 phase-02.
- Dashboard cost breakdown changes — M20 phase-03.
- Phase doc levels (L1/L2/L3) and architect scorecard reading — M22.
- Auto-calibration feedback loop — M23.
- Any change to `docs/architecture.md` — not required for this phase.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
