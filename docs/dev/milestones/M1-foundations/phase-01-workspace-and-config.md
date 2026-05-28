# Phase 01: Workspace + config + error model

**Milestone:** M1 — Foundations
**Status:** todo
**Depends on:** none
**Estimated diff:** ~300 lines

## Goal

Turn the docs-only repo into a buildable Cargo workspace with two crates —
`executor` (library) and `mcp` (binary) — plus the configuration model and error
type every later phase depends on. After this phase `cargo build`, `cargo clippy
-D warnings`, and `cargo test` all pass, and the config loads from a TOML file +
environment overrides.

This phase ships **no** AI client and **no** MCP server — those are phases 02 and
M5. It establishes the skeleton and the two load-bearing contracts (`Config`,
the error enum) so phase 02 has something to build on.

## Architecture references

- `docs/architecture.md` — "Configuration". The config carries the executor
  endpoint + model, the target-repo root, the `{…_COMMAND}` resolution, and
  budget knobs. This phase defines that struct.
- `docs/architecture.md` — "Layer 1 — `executor` crate". The lift/drop map; this
  phase only creates the crate, no lifting yet.
- `docs/dev/STANDARDS.md` §2.1 (error handling) — the error type follows the
  "programmer/infra failures → enum; model-visible outcomes → ToolResult" split.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch (`git status` clean; only `docs/`
   exists).
5. For reference only, you may read `rexy/src/config/mod.rs` and
   `rexy/src/error/mod.rs` to see the shapes being adapted — but **do not** copy
   Rexy-specific config sections (`tui`, `planner`, `pricing`, etc.). rexyMCP's
   config is its own, smaller shape defined in Spec §3 below.

## Current state

- The repo contains only `docs/` (architecture + dev docs). There is no
  `Cargo.toml`, no `src/`, no CI.
- `git` is initialized; the default branch is `master`.

## Spec

### 1. Workspace layout

Create a Cargo **workspace** at the repo root:

1. **`Cargo.toml`** (workspace root) — declares members `["executor", "mcp"]`,
   `resolver = "2"`, and a `[workspace.package]` block with `edition = "2024"`,
   a shared version, and shared lints. Pin shared dependency versions in
   `[workspace.dependencies]` (at minimum: `serde`, `serde_json`, `toml`,
   `thiserror`, `anyhow`). Member crates reference them with
   `dep.workspace = true`.
2. **`executor/Cargo.toml`** — library crate `rexymcp-executor` (lib name
   `executor`). Depends on the workspace serde/toml/thiserror/anyhow deps.
3. **`mcp/Cargo.toml`** — binary crate `rexymcp` (the future MCP server; this
   phase ships only a stub `main`). Depends on `executor`.
4. **`executor/src/lib.rs`** — declares `pub mod config;` and `pub mod error;`.
5. **`mcp/src/main.rs`** — minimal `fn main()` that prints the crate name +
   version and exits 0. (The real server lands in M5; this is just a buildable
   placeholder.)

### 2. Error model

In `executor/src/error.rs`, define a `thiserror` enum following STANDARDS §2.1.
Model-visible outcomes do **not** go here. Minimum variants (add only what's
needed):

```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("backend: {0}")]
    Backend(String),

    #[error("internal: {0}")]
    Internal(String),
}
```

### 3. Configuration model (load-bearing — prescriptive)

In `executor/src/config.rs`, define the config the whole product reads. Field
names and the nesting are contract; later phases import these. Derive
`Debug, Clone, Serialize, Deserialize` and a `Default` that yields a usable
local-LLM config.

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
}

/// The local LLM the executor drives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// "openai" | "ollama" | "lmstudio" — all OpenAI-compatible.
    pub provider: String,
    /// Model identifier as the endpoint knows it (e.g. "qwen2.5-coder").
    pub model: String,
    /// OpenAI-compatible base URL, e.g. "http://localhost:1234/v1".
    pub base_url: String,
    /// Optional API key; local endpoints usually ignore it.
    pub api_key: Option<String>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            provider: "openai".into(),
            model: String::new(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: None,
        }
    }
}

/// Resolves the {FORMAT,BUILD,LINT,TEST}_COMMAND placeholders for the
/// target project the executor works in.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandConfig {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// % of the model's context window the loop may fill before compacting.
    pub max_context_pct: u8,
    /// Hard cap on executor turns in one phase before budget_exceeded.
    pub max_turns: u32,
    /// Escalation slots (briefings returned to the architect) per phase.
    pub escalation_slots: u32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self { max_context_pct: 70, max_turns: 40, escalation_slots: 1 }
    }
}
```

Note: the **target-repo root** is **not** in `Config` — it is per-call
(an `execute_phase` argument, M5), not global config. Do not add it here.

### 4. Config loading

`config.rs` also exposes loading with this precedence (later overrides earlier):

1. `Config::default()`.
2. A TOML file at a path the caller passes (`Config::load(path: &Path)`); if the
   file is absent, that is **not** an error — return defaults. If present but
   malformed, return `Error::Config`.
3. Environment overrides applied by a separate `Config::apply_env(&mut self)`:
   `REXYMCP_PROVIDER`, `REXYMCP_MODEL`, `REXYMCP_BASE_URL`, `REXYMCP_API_KEY`.
   An env var that fails to parse (e.g. a future numeric field) returns
   `Error::Config`; a string field never fails to parse.

Provide `Config::load_with_env(path: &Path) -> Result<Config>` that does (2)
then (3).

### 5. Tooling config + CI

1. **`rustfmt.toml`** — `max_width = 100`, `imports_granularity = "Module"`,
   `group_imports = "StdExternalCrate"`, `edition = "2024"`.
2. **`.github/workflows/ci.yml`** — on push/PR, runs (as separate steps, not
   chained) `cargo fmt --all --check`, `cargo clippy --all-targets
   --all-features -- -D warnings`, and `cargo test`, with `RUSTFLAGS=-D warnings`.
3. **`.gitignore`** — at minimum `/target`.

## Acceptance criteria

- [ ] `cargo build` succeeds from a clean checkout with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (the new config unit tests).
- [ ] `cargo run -p rexymcp` prints the crate name + version and exits 0.
- [ ] `Config::default()` yields `provider = "openai"`, `base_url =
      "http://localhost:1234/v1"`, `max_context_pct = 70`.
- [ ] `Config::load` on a non-existent path returns defaults (not an error).
- [ ] `Config::load_with_env` applies `REXYMCP_MODEL` over the TOML value.
- [ ] `Config::load` on a malformed TOML file returns `Error::Config`.

## Test plan

Unit tests in `executor/src/config.rs` (`#[cfg(test)] mod tests`). Hermetic —
use `tempfile::TempDir` for TOML fixtures and set/remove env vars within the
test (serialize env-mutating tests so they don't race).

- `default_config_targets_local_lmstudio` — asserts the three default values
  above.
- `load_missing_file_returns_default` — `load` on a path that doesn't exist
  returns defaults.
- `load_parses_toml_executor_block` — a TOML fixture sets `model`/`base_url`;
  asserts they load.
- `load_malformed_toml_is_config_error` — garbage TOML → `Err(Error::Config(_))`.
- `env_override_beats_toml` — TOML sets `model = "a"`, `REXYMCP_MODEL = "b"`;
  `load_with_env` yields `"b"`.

Authorize one new dev-dependency (`tempfile`) in §Authorizations.

## End-to-end verification

The phase ships a runnable binary and a config loader — both real artifacts.

- AC: `cargo run -p rexymcp` → paste stdout (expect crate name + version).
- AC: write a TOML fixture under a temp dir, point a tiny throwaway check at it
  via `Config::load_with_env`, and confirm an env override wins — but this is
  covered by `env_override_beats_toml`; for the E2E entry, run the binary with
  `REXYMCP_MODEL` set if you wire it into the stub `main`'s output, otherwise
  declare the config-loader's real-artifact check satisfied by the unit test
  that exercises the public `load_with_env` API directly and say so.

## Authorizations

- [x] **May add dependencies:** `serde` (+`derive`), `serde_json`, `toml`,
      `thiserror`, `anyhow` (runtime); `tempfile` (dev). These are the workspace
      foundation; no others without a new authorization.
- [ ] May **NOT** add an async runtime, `reqwest`, or `rmcp` yet — those arrive
      with the AI client (phase 02) and the server (M5).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Out of scope

- The AI client, `AiClient` trait, message/event types — **phase 02**.
- The MCP server and `execute_phase` — **M5**. `mcp/src/main.rs` is a stub.
- Any executor tooling, parser, governor — **M2–M4**.
- The target-repo-root field — it is a per-call argument, not config.
- A `.env`-file loader — env vars are read directly; a dotenv loader is not
  required this phase.

## Update Log

<!-- entries appended below this line -->
