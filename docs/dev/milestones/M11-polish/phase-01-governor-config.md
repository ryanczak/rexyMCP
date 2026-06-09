# Phase 01: Governor thresholds → `[governor]` config

**Milestone:** M11 — Polish
**Status:** review
**Depends on:** nothing (pure config plumbing)
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Move the three hard-coded governor constants out of the source into `rexymcp.toml`
so users can tune them without recompiling. The constants today:

```rust
// executor/src/governor/hard_fail.rs
pub const IDENTICAL_CALL_THRESHOLD: usize = 6;
pub const VERIFIER_PERSISTENCE_THRESHOLD: usize = 6;
pub const RUNAWAY_OUTPUT_BYTES: usize = 100 * 1024;
```

After this phase they are read from `[governor]` in `rexymcp.toml` (with the
same values as defaults). No behaviour changes; only the mechanism changes.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).

## Spec

### 1. New `GovernorConfig` in `executor/src/config.rs`

Add the struct **above** the existing `BudgetConfig`:

```rust
/// Governor hard-fail thresholds. Tune these to match your model's cadence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernorConfig {
    /// Consecutive identical tool calls before `IdenticalToolCallRepetition`
    /// hard-fail. Default 6.
    pub identical_call_threshold: usize,
    /// Consecutive turns with author-attributed verifier errors before
    /// `VerifierFailurePersistent` hard-fail. Default 6.
    pub verifier_persistence_threshold: usize,
    /// Single tool-output bytes before `RunawayOutput` hard-fail.
    /// Default 102400 (100 KB).
    pub runaway_output_bytes: usize,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            identical_call_threshold: 6,
            verifier_persistence_threshold: 6,
            runaway_output_bytes: 100 * 1024,
        }
    }
}
```

Add `governor: GovernorConfig` to the `Config` struct (after `context`):

```rust
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
    pub dashboard: DashboardConfig,
    pub context: ContextConfig,
    pub governor: GovernorConfig,   // ← add this line
}
```

`Config` derives `Default` and has `#[serde(default)]`; the new field picks up
both automatically. No other changes to `config.rs`.

### 2. Update `executor/src/governor/hard_fail.rs`

**Remove** the three `pub const` lines at the top of the file.

**Change `evaluate`'s signature** to accept a `GovernorConfig`:

```rust
use crate::config::GovernorConfig;

pub fn evaluate(
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    recent_verifier_error_counts: &[usize],
    output: Option<(&str, usize)>,
    config: &GovernorConfig,
) -> Option<HardFailSignal>
```

Inside `evaluate`, forward to the three helpers passing the config values:

```rust
pub fn evaluate(..., config: &GovernorConfig) -> Option<HardFailSignal> {
    if let Some(s) = check_identical_repetition(recent_tool_calls, config.identical_call_threshold) {
        return Some(s);
    }
    if let Some(s) = check_verifier_persistence(recent_verifier_error_counts, config.verifier_persistence_threshold) {
        return Some(s);
    }
    if let Some(s) = check_runaway_output(output, config.runaway_output_bytes) {
        return Some(s);
    }
    None
}
```

Update the three private helpers to accept their threshold as a `usize` parameter
instead of using the removed constants. Example for `check_identical_repetition`:

```rust
fn check_identical_repetition(
    recent: &VecDeque<ToolCallSnapshot>,
    threshold: usize,
) -> Option<HardFailSignal> {
    if recent.len() < threshold {
        return None;
    }
    let last_n: Vec<_> = recent.iter().rev().take(threshold).collect();
    // ... rest unchanged, replace IDENTICAL_CALL_THRESHOLD with threshold
    Some(HardFailSignal::IdenticalToolCallRepetition {
        tool: last_n[0].tool.clone(),
        consecutive_count: threshold as u32,
    })
}
```

Apply the same pattern to `check_verifier_persistence` (use `threshold` instead of
`VERIFIER_PERSISTENCE_THRESHOLD`) and `check_runaway_output` (use `limit` instead
of `RUNAWAY_OUTPUT_BYTES`).

**Update `hard_fail.rs` tests** — every `evaluate(...)` call in the `#[cfg(test)]`
block needs a fourth argument. Replace all bare `evaluate(&recent, &counts, output)`
calls with `evaluate(&recent, &counts, output, &GovernorConfig::default())`. Add
`use crate::config::GovernorConfig;` to the test imports. Also update any direct
calls to the private helpers if they appear in tests (they appear in doc-tests but
not unit tests — check carefully).

### 3. Add `governor: GovernorConfig` to `LoopDeps` in `executor/src/agent/mod.rs`

In the `LoopDeps` struct definition (around line 75), add one field:

```rust
/// Governor hard-fail thresholds — read from `[governor]` in rexymcp.toml.
pub governor: GovernorConfig,
```

`GovernorConfig` derives `Copy`, so this is an owned field (no lifetime needed).

**Update the `evaluate()` call** (around line 806 inside `execute_phase`):

```rust
// Before:
if let Some(signal) = evaluate(&recent_tool_calls, &recent_verifier_error_counts, ...) {

// After:
if let Some(signal) = evaluate(&recent_tool_calls, &recent_verifier_error_counts, ..., &deps.governor) {
```

**Update all `LoopDeps { ... }` construction sites in the test module.** Use `grep`
to find them all:

```bash
grep -n "LoopDeps {" executor/src/agent/mod.rs
```

There are approximately 8 sites. Add `governor: GovernorConfig::default()` to each.
Add `use crate::config::GovernorConfig;` to the test-module imports. You do NOT need
to add it to the module-level imports at the top of `mod.rs` (it's already imported
transitively via `config` if needed, or add it to the top-level `use` block).

Actually, add it to the top-level `use` block at the top of `mod.rs` since
`GovernorConfig` is now a field of `LoopDeps` (a public struct in the same file):

```rust
use crate::config::{CommandConfig, GovernorConfig};
```

### 4. Update `mcp/src/runner.rs`

In the `LoopDeps { ... }` construction (around line 179), add:

```rust
governor: inp.cfg.governor,
```

`GovernorConfig: Copy` so no borrow needed.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero warnings after the changes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test` passes — all existing tests green (the new `GovernorConfig::default()`
  produces identical threshold values to the removed constants, so no test behaviour
  should change).
- [ ] `cargo fmt --all --check` passes.
- [ ] The three compile-time constants no longer exist in `hard_fail.rs`.
- [ ] `GovernorConfig` appears in `config.rs` with a `Default` impl matching the old
  constant values (6, 6, 102400).
- [ ] A `[governor]` section in `rexymcp.toml` with any of the three keys overrides the
  default — verified by reading the config test that parses a full TOML and asserting
  the non-default values round-trip.

## Notes for executor

- Do NOT change the test for `no_runaway_at_exact_threshold` — it asserts the
  boundary condition using the old constant value via `RUNAWAY_OUTPUT_BYTES`. After
  your change it will reference `GovernorConfig::default().runaway_output_bytes`
  instead. Update the reference but keep the logic.
- Read `hard_fail.rs` in full before editing it — the file is under 300 lines and
  within the read cap.
- Read `config.rs` in full before editing — it is under 700 lines.
- Read `runner.rs` in full before editing — check its actual line count first with a
  shell command.
- For `agent/mod.rs`, do NOT read the full file. Use `grep` to find:
  1. The `LoopDeps` struct definition (line range)
  2. The `evaluate(` call site
  3. All `LoopDeps {` construction sites
  Then range-read only the surrounding context (~20 lines each) before patching.
- Add one new config test in `config.rs` asserting that a TOML with
  `[governor]\nidentical_call_threshold = 10` round-trips to
  `cfg.governor.identical_call_threshold == 10`. Model it on the existing `BudgetConfig`
  round-trip tests in the same file.

## Update Log

### Update — 2025-07-18 (started)

**Executor:** rexyMCP executor LLM

Implementing Spec tasks: adding `GovernorConfig` to config, wiring through `hard_fail.rs`, `LoopDeps`, and `runner.rs`.

### Update — 2025-07-18 (complete)

**Executor:** rexyMCP executor LLM

**Summary:** Moved three hard-coded governor constants from `hard_fail.rs` into `[governor]` config section of `rexymcp.toml`. Defaults match previous constant values (6, 6, 102400).

**Files changed:**
- `executor/src/config.rs` — added `GovernorConfig` struct with `Default` impl, added `governor` field to `Config`, added `governor_config_round_trips_from_toml` test
- `executor/src/governor/hard_fail.rs` — removed 3 `pub const`, added `GovernorConfig` parameter to `evaluate()`, updated helpers to accept threshold params, updated all test calls
- `executor/src/agent/mod.rs` — added `GovernorConfig` import, added `governor` field to `LoopDeps`, updated `evaluate()` call site, updated all 8 `LoopDeps { ... }` construction sites in tests
- `mcp/src/runner.rs` — added `governor: inp.cfg.governor` to `LoopDeps` construction

**Verification summary:**
- `cargo fmt --all --check`: passed
- `cargo build`: passed, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: passed
- `cargo test`: 665 passed, 0 failed, 2 ignored

**End-to-end verification:**
- `grep "pub const IDENTICAL_CALL_THRESHOLD" executor/src/governor/hard_fail.rs` → 0 matches (constants removed)
- `grep "GovernorConfig" executor/src/config.rs` → struct + Default impl + Config field present
- `cargo test governor_config_round_trips_from_toml` → passed (non-default values 10, 8, 204800 round-trip through TOML)

**Notes for review:**
- No behavioral changes — `GovernorConfig::default()` produces identical values to the removed constants
- All existing tests pass unchanged (only the call sites needed the 4th argument added)

