# Phase 05: per-model config-override resolution layer

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** todo
**Depends on:** phase-04 (thread 2 complete — the profile that *informs* which
per-model knobs to set is now surfaced; this phase builds the substrate that
*acts* on that judgment)
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Open thread 3 (model-conditioned runtime knobs). Today every tunable knob —
`task_tracking`, the three governor thresholds, sampling (`temperature`/`seed`) —
is **global**: one value in `[executor]`/`[governor]` applies to whatever model
is configured. Thread 2 now tells the architect *"Qwen first-passes rust/feature
work but stalls on multi-site changes; Gemma needs `temperature=0.2`"* — but
there is no way to encode that per-model. This phase adds the **resolution
layer**: a `[models."<id>"]` override table in `rexymcp.toml` and a pure
`Config::resolve_for_model(&mut self, model)` that applies the matching model's
overrides on top of the global defaults.

This phase is the **pure substrate**, mirroring how phase-03 shipped the
`aggregate_profiles` layer before phase-04 surfaced it. It adds the config types
and the resolution function with full unit coverage, but **does not call it from
the dispatch path** — wiring `resolve_for_model` into `mcp/src/runner.rs` (so the
live loop actually uses resolved values) plus the `rexymcp init` template
documentation is **phase-06**. The consumer is pinned and immediate (the very
next phase), so this is not orphaned state — it is the established 03→04 split.

## Architecture references

Read before starting:

- `docs/architecture.md#configuration` — the config knobs this phase makes
  per-model (the command set, budget, governor, sampling).
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "Threads → phases"
  — thread 3 is "make `task_tracking` / governor thresholds / router breadth /
  sampling resolve per-model defaults from config instead of global constants."
  (Router breadth is **out of scope here** — see Out of scope.)

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

All config lives in **`executor/src/config.rs`** (the `executor` **library**
crate). The two structs this phase reads overrides *into* already exist.

### `GovernorConfig` — the idiom to mirror for a new `[serde(default)]` struct

`executor/src/config.rs:51-74`. Note the `#[serde(default)]` + explicit `Default`
impl shape — copy it:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernorConfig {
    pub identical_call_threshold: usize,
    pub verifier_persistence_threshold: usize,
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

### The knobs the override targets (already global today)

- `ExecutorConfig.task_tracking: bool` (`config.rs:132-133`)
- `ExecutorConfig.temperature: Option<f64>` (`config.rs:123-124`)
- `ExecutorConfig.seed: Option<u64>` (`config.rs:126-127`)
- `GovernorConfig.{identical_call_threshold, verifier_persistence_threshold,
  runaway_output_bytes}` (above)

The active model id is **`Config.executor.model`** (`config.rs:108-109`).

### The top-level `Config` struct — where the new field lands

`config.rs:85-96`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub project: ProjectConfig,
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,
    pub dashboard: DashboardConfig,
    pub context: ContextConfig,
    pub governor: GovernorConfig,
}
```

`Config` already derives `Default` and is `#[serde(default)]`, so an added
`HashMap` field (whose `Default` is an empty map) needs **no** manual `Default`
edit and an absent `[models]` section parses to an empty map for free.

### Why in-place mutation (not a returned struct)

The dispatch path reads these knobs **directly off `Config`**
(`mcp/src/runner.rs:185, 226-228, 232-233`):

```rust
    let tasks = if inp.cfg.executor.task_tracking { ... };
    // ...
        generation_params: GenerationParams {
            temperature: inp.cfg.executor.temperature,
            seed: inp.cfg.executor.seed,
        },
        governor: inp.cfg.governor,
        task_tracking: inp.cfg.executor.task_tracking,
```

So `resolve_for_model(&mut self, model)` **mutating `self.executor`/`self.governor`
in place** means phase-06 wires it with a single call before these reads, and
every site above picks up the resolved value with zero further change. A function
that returned a separate "resolved" struct would force phase-06 to re-thread every
read site. In-place is the low-blast-radius shape. **Do not touch `runner.rs` in
this phase** — that wiring is phase-06.

## Spec

All code lands in **`executor/src/config.rs`**. Additive — no existing function
changes behavior.

### 1. The `ModelOverride` struct

Add near `GovernorConfig`. Every field is `Option` — **`None` means "inherit the
global default"**, the only override semantics (no separate "clear back to
endpoint default" — STANDARDS §2.2 "no fallbacks for if X is missing"). The field
names mirror their targets exactly.

```rust
/// Per-model knob overrides. Each `Some` field replaces the corresponding global
/// `[executor]`/`[governor]` default when this model is the active executor
/// model; each `None` field inherits the global value. Keyed by exact model id
/// in the `[models]` table (e.g. `[models."Qwen/Qwen3.6-27B-FP8"]`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ModelOverride {
    pub task_tracking: Option<bool>,
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub identical_call_threshold: Option<usize>,
    pub verifier_persistence_threshold: Option<usize>,
    pub runaway_output_bytes: Option<usize>,
}
```

### 2. The `models` field on `Config`

Add as the last field of `Config`:

```rust
    pub models: std::collections::HashMap<String, ModelOverride>,
```

(Add a `use std::collections::HashMap;` at the top and write `HashMap<...>` if you
prefer; either is fine. Keep the existing `use` grouping convention.)

### 3. `Config::resolve_for_model`

Add to the existing `impl Config` block (`config.rs:212+`, alongside `load`).

```rust
impl Config {
    /// Apply the per-model override for `model` (exact-match lookup in
    /// `[models]`) on top of the global `[executor]`/`[governor]` defaults,
    /// mutating `self` so downstream reads see resolved values. A model with no
    /// `[models]` entry leaves every global untouched.
    pub fn resolve_for_model(&mut self, model: &str) {
        let Some(over) = self.models.get(model).cloned() else {
            return;
        };
        if let Some(v) = over.task_tracking {
            self.executor.task_tracking = v;
        }
        if let Some(v) = over.temperature {
            self.executor.temperature = Some(v);
        }
        if let Some(v) = over.seed {
            self.executor.seed = Some(v);
        }
        if let Some(v) = over.identical_call_threshold {
            self.governor.identical_call_threshold = v;
        }
        if let Some(v) = over.verifier_persistence_threshold {
            self.governor.verifier_persistence_threshold = v;
        }
        if let Some(v) = over.runaway_output_bytes {
            self.governor.runaway_output_bytes = v;
        }
    }
}
```

### Gotchas (pin these — they are the easy mistakes here)

- **Use `.cloned()` on the lookup**, as shown. `self.models.get(model)` borrows
  `self.models` immutably; cloning the small `ModelOverride` out *ends that borrow*
  before you mutate `self.executor`/`self.governor`. Holding the borrow across the
  mutations risks a borrow-check stall — the `.cloned()` + `let-else` early return
  is the clean idiom. (`ModelOverride` derives `Clone`.)
- **Exact-match only.** `self.models.get(model)` is an exact string lookup. No
  prefix/substring/case-folding. `[models."qwen"]` must **NOT** apply to model
  `"qwen2.5-coder"`. Pin this negative.
- **`None` override field is a no-op, not a reset.** An override that sets only
  `temperature` must leave `task_tracking`/`seed`/the governor thresholds at their
  global values. Pin this.
- **Lib crate → no `#[allow(dead_code)]`.** `config.rs` is in the `executor`
  **library** crate, where `pub` items (`ModelOverride`, `resolve_for_model`) are
  public API and are **not** dead code even with no caller yet. Do **NOT** add
  `#![allow(dead_code)]` — that was needed only because phase-03's `profile.rs`
  lived in the `mcp` **binary** crate (bug-03-2). If `cargo clippy -D warnings`
  flags `resolve_for_model` as unused here, stop and file a blocker (it should
  not).

## Acceptance criteria

- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      and `cargo fmt --all --check` are clean (no new warnings, no `#[allow]`).
- [ ] `cargo test` passes (existing + new).
- [ ] A `rexymcp.toml` with a `[models."<id>"]` table loads via `Config::load`
      and populates `config.models` with a `ModelOverride` for that id.
- [ ] An absent `[models]` section yields an empty `config.models` map (default).
- [ ] `resolve_for_model` with a matching id overrides each set field
      (`task_tracking`, `temperature`, `seed`, and all three governor thresholds)
      and leaves each unset field at its global value.
- [ ] `resolve_for_model` with a non-matching id (or empty `[models]`) leaves all
      global values untouched.
- [ ] Exact-match only: a key that is a prefix/substring of the model id does not
      apply.

## Test plan

Hermetic tests in the existing `executor/src/config.rs` `#[cfg(test)] mod tests`
block. Pin **behavior and names**, not exact assertion wording:

- `model_override_section_parses_from_toml` — a written `rexymcp.toml` with
  `[models."Qwen/Qwen3.6-27B-FP8"]` setting `temperature` + `task_tracking` loads
  via the real `Config::load`; `config.models` contains that key and the override
  fields are `Some(...)`. (Exercises the quoted-dotted-key TOML parse path.)
- `models_section_absent_is_empty_map` — a config with no `[models]` section has
  `config.models.is_empty()`. (Pinned default.)
- `resolve_for_model_applies_matching_override` — globals set to known values, a
  `[models]` entry overriding `task_tracking=false`, `temperature=0.2`, `seed=7`,
  `identical_call_threshold=8`; after `resolve_for_model("<id>")` each target field
  equals the override value. Mutation-resistant: fails if any branch is dropped.
- `resolve_for_model_leaves_unset_fields_global` — an override that sets **only**
  `temperature`; after resolution `temperature` is overridden but `task_tracking`,
  `seed`, and all three governor thresholds retain their global values. (Pinned —
  the `None`-is-a-no-op boundary.)
- `resolve_for_model_unknown_model_is_noop` — `[models]` has one entry, but
  `resolve_for_model` is called with a **different** id; every executor/governor
  field is byte-identical to before the call. (Pinned negative.)
- `resolve_for_model_is_exact_match` — `[models."qwen"]` present; calling
  `resolve_for_model("qwen2.5-coder")` changes nothing (prefix must not match).
  (Pinned negative.)

## End-to-end verification

Not applicable — this phase ships **no** runtime-loadable artifact that exercises
the resolution: `resolve_for_model` has no caller in any binary until phase-06
wires it into `runner.rs`. The config *parse* half is real and is covered by the
`model_override_section_parses_from_toml` test going through the actual
`Config::load` (file → TOML → struct). State this in the completion Update Log; do
not fabricate a CLI transcript.

## Authorizations

- Edit **`executor/src/config.rs`** only: add `ModelOverride`, the `models` field
  on `Config`, `resolve_for_model`, and the new tests.
- No new dependency (`serde`/`toml`/`std::collections::HashMap` already in use).
- No edits to `mcp/src/runner.rs`, `mcp/src/init.rs`, `architecture.md`,
  `Cargo.toml`, `STANDARDS.md`, or `WORKFLOW.md`.
- **No `#[allow]`/`#![allow]` anywhere** — see the lib-crate gotcha above.

## Out of scope

- **Wiring `resolve_for_model` into the dispatch path** (`mcp/src/runner.rs`) so
  the live loop uses resolved knobs — **phase-06**. This phase must not touch
  `runner.rs`.
- **`rexymcp init` template documentation** of the `[models]` section —
  **phase-06** (it ships with the wiring that makes it live).
- **Router breadth** as a per-model knob. Unlike the other thread-3 knobs, router
  breadth has **no global config home today** (it is a constant in the router);
  exposing it would mean adding a *new* global config section first, a separate
  concern. Note it as a thread-3 candidate; do not add it here.
- Any change to how knobs are *consumed* (the governor, the loop, the AI client).
  This phase only resolves values into `Config`; consumers are unchanged.
- Profile-driven *automatic* knob selection (reading `model_profile` to pick
  overrides). M18 keeps the human in the loop — the architect reads the profile
  and writes the `[models]` table by hand. No automated model→knob routing.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
