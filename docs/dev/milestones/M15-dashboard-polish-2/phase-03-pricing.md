# Phase 03 — Model-aware `$ saved` pricing

**Status:** todo
**Milestone:** M15-dashboard-polish-2
**Depends on:** phase-02 (done)

## Goal

The `$ saved` Budget-panel line currently requires the user to manually look up
and enter `saved_input_per_mtok` / `saved_output_per_mtok` in `rexymcp.toml`;
both default to `0.0` (shown as `—`). Add a `saved_model` config field that
accepts a Claude model name and auto-fills the correct pricing.

Five files change:
- `executor/src/config.rs` — add `saved_model: Option<String>` to `DashboardConfig`;
  remove `Copy` from its derive (required — `String` is not `Copy`).
- `mcp/src/dashboard/panels.rs` — add `pub fn model_rates(model: &str) -> Option<BudgetRates>`.
- `mcp/src/dashboard/mod.rs` — re-export `model_rates`.
- `mcp/src/main.rs` — use `model_rates` when `saved_model` is set; fall back to
  numeric rates otherwise.
- `mcp/src/init.rs` — update the `[dashboard]` comment block with the new field.

~40 lines of diff. No new `SessionEvent`, no `Cargo.toml` change.

## Architecture references

- `executor/src/config.rs:8–24` — `DashboardConfig` struct + `Default` impl
- `mcp/src/dashboard/panels.rs:23–28` — `BudgetRates` struct
- `mcp/src/dashboard/panels.rs:364–375` — `dollars_saved_line`
- `mcp/src/dashboard/mod.rs:19` — existing `pub use panels::BudgetRates;`
- `mcp/src/main.rs:369–372` — `BudgetRates` construction from config
- `mcp/src/init.rs:41–42` — existing `[dashboard]` comment lines

## Pre-flight

- [ ] `cargo test` passes on the current tree.
- [ ] Confirm `config.rs:8–9` is `#[derive(Debug, Clone, Copy, Serialize, Deserialize)]` /
      `#[serde(default)]` for `DashboardConfig`.
- [ ] Confirm `config.rs:15` shows only `saved_input_per_mtok` and `saved_output_per_mtok`
      fields (no `saved_model` yet).
- [ ] Confirm `main.rs:369–372` is the `BudgetRates` struct literal reading from
      `cfg.dashboard.saved_input_per_mtok` / `saved_output_per_mtok`.
- [ ] Confirm `mod.rs:19` is `pub use panels::BudgetRates;`.

## Current state

### `config.rs:8–24` — `DashboardConfig` (current)

```rust
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
        Self {
            saved_input_per_mtok: 0.0,
            saved_output_per_mtok: 0.0,
        }
    }
}
```

### `main.rs:369–372` — BudgetRates construction (current)

```rust
let rates = dashboard::BudgetRates {
    input_per_mtok: cfg.dashboard.saved_input_per_mtok,
    output_per_mtok: cfg.dashboard.saved_output_per_mtok,
};
```

### `mod.rs:19` — existing re-export

```rust
pub use panels::BudgetRates;
```

### `init.rs:41–42` — existing dashboard comments

```
# saved_input_per_mtok = 3.0      # $/Mtok input tokens saved vs cloud baseline
# saved_output_per_mtok = 15.0    # $/Mtok output tokens saved vs cloud baseline
```

## Spec

### Task 1 — Add `saved_model` to `DashboardConfig` (`config.rs`)

**Critical gotcha:** `String` is not `Copy`. Removing `Copy` from the derive
is **required** — the compiler will reject `#[derive(Clone, Copy)]` the moment
`Option<String>` is added. The call sites in `config.rs` tests only access
individual `f64` fields (which remain `Copy`), and `main.rs:369–372` accesses
fields individually — neither relies on `DashboardConfig` itself being `Copy`.
Removing `Copy` is safe; no downstream code copies the struct by value.

Replace the struct definition and `Default` impl:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]    // ← Copy removed
#[serde(default)]
pub struct DashboardConfig {
    /// USD per million **input** tokens for the cloud baseline (used when
    /// `saved_model` is not set or not recognised).
    pub saved_input_per_mtok: f64,
    /// USD per million **output** tokens for the cloud baseline (used when
    /// `saved_model` is not set or not recognised).
    pub saved_output_per_mtok: f64,
    /// Optional Claude model name; when set and recognised, auto-fills
    /// `saved_input_per_mtok` / `saved_output_per_mtok` with current pricing.
    /// Recognised values: `claude-fable-5`, `claude-mythos-5`,
    /// `claude-opus-4-8`/`4-7`/`4-6`, `claude-sonnet-4-6`, `claude-haiku-4-5`.
    pub saved_model: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            saved_input_per_mtok: 0.0,
            saved_output_per_mtok: 0.0,
            saved_model: None,
        }
    }
}
```

The struct has `#[serde(default)]` at the struct level, so existing `rexymcp.toml`
files that omit `saved_model` will deserialize correctly with `None`.

### Task 2 — Add `model_rates` lookup (`panels.rs`)

Add a new public function after the `BudgetRates` struct definition
(around `panels.rs:28`):

```rust
/// Return hardcoded cloud baseline rates for a known Claude model name.
/// Returns `None` for unrecognised names (caller falls back to configured rates).
///
/// Pricing as of 2026-06-04 ($/MTok input / $/MTok output):
/// - Fable 5 / Mythos 5: $10.00 / $50.00
/// - Opus 4.8 / 4.7 / 4.6: $5.00 / $25.00
/// - Sonnet 4.6: $3.00 / $15.00
/// - Haiku 4.5: $1.00 / $5.00
pub fn model_rates(model: &str) -> Option<BudgetRates> {
    match model {
        "claude-fable-5" | "claude-mythos-5" => Some(BudgetRates {
            input_per_mtok: 10.0,
            output_per_mtok: 50.0,
        }),
        "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => Some(BudgetRates {
            input_per_mtok: 5.0,
            output_per_mtok: 25.0,
        }),
        "claude-sonnet-4-6" => Some(BudgetRates {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
        }),
        "claude-haiku-4-5" => Some(BudgetRates {
            input_per_mtok: 1.0,
            output_per_mtok: 5.0,
        }),
        _ => None,
    }
}
```

### Task 3 — Re-export `model_rates` from dashboard (`mod.rs`)

At `mcp/src/dashboard/mod.rs`, after the existing `pub use panels::BudgetRates;`
line (~line 19), add:

```rust
pub use panels::model_rates;
```

This lets `main.rs` call `dashboard::model_rates(…)` matching the pattern
already established for `dashboard::BudgetRates`.

### Task 4 — Use `model_rates` in `BudgetRates` construction (`main.rs`)

Replace `main.rs:369–372`:

```rust
// Before:
let rates = dashboard::BudgetRates {
    input_per_mtok: cfg.dashboard.saved_input_per_mtok,
    output_per_mtok: cfg.dashboard.saved_output_per_mtok,
};

// After:
let d = &cfg.dashboard;
let rates = d
    .saved_model
    .as_deref()
    .and_then(dashboard::model_rates)
    .unwrap_or(dashboard::BudgetRates {
        input_per_mtok: d.saved_input_per_mtok,
        output_per_mtok: d.saved_output_per_mtok,
    });
```

`as_deref()` converts `&Option<String>` → `Option<&str>` so `model_rates` can
accept it. If `saved_model` is `None`, or is `Some` but unrecognised, the numeric
rates are used as before.

### Task 5 — Update `init.rs` template

Replace `init.rs:41–42`:
```
# saved_input_per_mtok = 3.0      # $/Mtok input tokens saved vs cloud baseline
# saved_output_per_mtok = 15.0    # $/Mtok output tokens saved vs cloud baseline
```

With:
```
# saved_model = "claude-opus-4-8"   # auto-fill rates for a known Claude model
#   known models: claude-fable-5 ($10/$50/MTok), claude-opus-4-8 ($5/$25/MTok),
#   claude-sonnet-4-6 ($3/$15/MTok), claude-haiku-4-5 ($1/$5/MTok)
# Or set rates directly (saved_model overrides these when both are set):
# saved_input_per_mtok = 5.0        # $/MTok input tokens saved vs cloud baseline
# saved_output_per_mtok = 25.0      # $/MTok output tokens saved vs cloud baseline
```

## Acceptance criteria

- [ ] `DashboardConfig` no longer derives `Copy` (required for `Option<String>` field).
- [ ] `DashboardConfig` has a new `saved_model: Option<String>` field with `Default` → `None`.
- [ ] Existing `config.rs` tests (`dashboard_config_defaults` / `dashboard_config_from_toml`)
      still pass without changes.
- [ ] `panels::model_rates("claude-opus-4-8")` returns `Some(BudgetRates { 5.0, 25.0 })`.
- [ ] `panels::model_rates("claude-fable-5")` returns `Some(BudgetRates { 10.0, 50.0 })`.
- [ ] `panels::model_rates("unknown")` returns `None`.
- [ ] `dashboard::model_rates` is accessible from `main.rs` via the `mod.rs` re-export.
- [ ] Setting `saved_model = "claude-opus-4-8"` in `rexymcp.toml` causes the `$ saved`
      line to compute against $5/$25 per MTok without needing numeric rate fields.
- [ ] All four gates pass on an independent re-run.

## Test plan

Add three tests to `panels.rs` test module under a `// --- model_rates tests ---`
section:

```rust
#[test]
fn model_rates_opus_48_returns_correct_pricing() {
    let rates = model_rates("claude-opus-4-8").expect("opus-4-8 should have rates");
    assert_eq!(rates.input_per_mtok, 5.0);
    assert_eq!(rates.output_per_mtok, 25.0);
}

#[test]
fn model_rates_fable_5_returns_correct_pricing() {
    let rates = model_rates("claude-fable-5").expect("fable-5 should have rates");
    assert_eq!(rates.input_per_mtok, 10.0);
    assert_eq!(rates.output_per_mtok, 50.0);
}

#[test]
fn model_rates_unknown_model_is_none() {
    assert!(model_rates("gpt-4").is_none(), "unknown model should return None");
    assert!(model_rates("").is_none(), "empty string should return None");
}
```

**Mutation-resistance:** an implementation that returns a non-`None` default for
all models would fail `model_rates_unknown_model_is_none`. An implementation that
swaps Opus / Fable rates would fail the pricing assertions.

The existing config tests (`dashboard_config_defaults` and
`dashboard_config_from_toml` in `config.rs`) should pass unchanged — they only
assert `saved_input_per_mtok` and `saved_output_per_mtok` field values, which
are not modified.

## End-to-end verification

With `rexymcp.toml` containing:
```toml
[dashboard]
saved_model = "claude-opus-4-8"
```

Running `rexymcp dashboard` against any session with recorded token metrics
should show `$ saved: $X.XX` computed at $5/$25 per MTok, without requiring
`saved_input_per_mtok` / `saved_output_per_mtok` to be set.

## Authorizations

- **Authorized: edit `executor/src/config.rs`** to add `saved_model: Option<String>`
  to `DashboardConfig` and remove `Copy` from its derive. This is the only
  cross-crate change; it is display-only (no new `SessionEvent`, no loop/governor
  change).
- **Authorized: edit `mcp/src/init.rs`** to update the `[dashboard]` template comment.

All other files (`panels.rs`, `mod.rs`, `main.rs`) are in the `mcp` crate, within
the dashboard's normal scope.

## Out of scope

- Pricing for non-Claude models.
- Auto-detecting the comparison model from the session (the session records the
  *executor* model, which is a local LLM — not the cloud baseline).
- A `[dashboard] compare_model` dropdown or interactive rate selection.
- Updating pricing values in the lookup — add a phase when pricing changes.

## Update Log

<!-- entries appended below this line -->
