# Phase 05a: settings plumbing — make sampling settings configurable, sent, and recorded

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** todo
**Depends on:** phase-04 (done — `rexymcp runs` displays `generation_params`, which
are always `default`/`None` today; this phase makes them real). No dependency on
phase-05b.
**Estimated diff:** ~190 lines (config fields + client threading + request-body
insertion + emit-site population + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the executor's **sampling settings** (`temperature`, `seed`) real across all
three points where they matter: **configurable** in `rexymcp.toml`'s `[executor]`
block, **sent** to the model on every chat request, and **recorded** with their
actual values in each `PhaseRun.generation_params`. Today `generation_params` is
hard-wired to `GenerationParams::default()` (both `None`) at the emit site, and the
request body never sends `temperature`/`seed` at all — so the "which settings work
best" axis the M7 redesign is built around carries no signal. This phase closes
that gap for the two knobs the `GenerationParams` struct already models.

This is the **high-value half** of the original phase-05 (settings are the lever
*we* choose and vary, so they drive "which settings work best"). The
endpoint-reported provenance half — served model id, `finish_reason`, context
window — is **phase-05b** and is explicitly out of scope here (see Out of scope).

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` schema's `generation_params` is the "how the model was asked" axis.
  This phase populates it from config and sends the same values on the wire.
- The M7 README (`docs/dev/milestones/M7-scorecard/README.md`) § Phases, phase 05
  bullet: *"make sampling settings (temperature/seed/…) configurable, sent to the
  model, and recorded with real values."* This phase implements the
  `temperature`/`seed` subset.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### `GenerationParams` — the record shape (already exists, unchanged) (`executor/src/store/telemetry.rs:16`)

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
}
```

This struct is correct as-is. **Do not change it.** This phase only makes its
values real; the field set is fixed at `temperature` + `seed` for this phase.

### `ExecutorConfig` — where the knobs must become configurable (`executor/src/config.rs:23`)

```rust
pub struct ExecutorConfig {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(default = "default_first_token_timeout_secs")]
    pub first_token_timeout_secs: u64,
    #[serde(default = "default_stream_idle_timeout_secs")]
    pub stream_idle_timeout_secs: u64,
}
```

Note the existing `#[serde(default = "...")]` idiom for optional fields with a
default, and that `ExecutorConfig` has a hand-written `impl Default` (config.rs:48)
— **both must be updated** when you add fields (the struct has no `#[derive(Default)]`).

### The request body — where settings must be *sent* (`executor/src/ai/backends/openai.rs:83`)

```rust
pub fn build_chat_body(
    model: &str,
    system: &str,
    messages: Vec<Value>,
    tools: Option<&[ToolSchema]>,
) -> Value {
    // ... assembles full_messages ...
    let mut body = json!({
        "model": model,
        "max_tokens": 4096,
        "stream": true,
        "stream_options": { "include_usage": true },
        "messages": full_messages,
    });
    // ... tool_choice ...
    body
}
```

Today the body never carries `temperature` or `seed`. `build_chat_body` is called
once in production (`openai.rs:165`, inside `OpenAiClient::chat`) and in several
`build_chat_body_*` unit tests (openai.rs:551–575).

### The client — where settings are held and threaded (`executor/src/ai/backends/openai.rs:124`)

```rust
pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String,
    first_token_timeout: Duration,
    stream_idle_timeout: Duration,
}

impl OpenAiClient {
    pub fn new(
        api_key: String,
        model: String,
        base_url: String,
        first_token_timeout: Duration,
        stream_idle_timeout: Duration,
    ) -> Self { /* ... */ }
}
```

`temperature`/`seed` belong on this struct (carried from config to the request).
The `AiClient` **trait** and `chat()` signature do **not** change — settings live
on the concrete `OpenAiClient`, so `MockAiClient` is unaffected.

### The construction site (`executor/src/ai/mod.rs:187`)

```rust
pub fn make_client(cfg: &ExecutorConfig) -> Box<dyn AiClient> {
    Box::new(OpenAiClient::new(
        cfg.api_key.clone().unwrap_or_default(),
        cfg.model.clone(),
        cfg.base_url.clone(),
        Duration::from_secs(cfg.first_token_timeout_secs),
        Duration::from_secs(cfg.stream_idle_timeout_secs),
    ))
}
```

This is where `cfg.executor.temperature`/`cfg.executor.seed` get handed to the
client.

### The emit site — where settings must be *recorded* (`mcp/src/runner.rs:190`)

```rust
let deps = LoopDeps {
    // ...
    generation_params: GenerationParams::default(),   // <-- always None today
    telemetry_dir: inp.telemetry_dir,
    progress: inp.progress,
};
```

`LoopDeps.generation_params` flows straight into the emitted record
(`executor/src/agent/mod.rs:1192`: `generation_params: deps.generation_params.clone()`),
which is written to `PhaseRun`. So populating this one field from
`inp.cfg.executor` is the entire "recorded" half. **`inp.cfg` is the loaded
`Config`** (see `RunPhaseConfig` at runner.rs:200) — reach the knobs at
`inp.cfg.executor.temperature` / `inp.cfg.executor.seed`.

## Spec

The invariant this phase must hold: **the values sent on the wire and the values
recorded in `generation_params` are the same configured values.** A run recorded
as `temp=0.2,seed=42` must have been asked with `temperature: 0.2, seed: 42`; a run
recorded as `default` must have sent neither key.

### Task 1 — Add the knobs to `ExecutorConfig` (`executor/src/config.rs`)

Add two fields to `ExecutorConfig`:

```rust
    /// Sampling temperature sent on every chat request. `None` omits the key,
    /// letting the endpoint apply its own default.
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Deterministic sampling seed sent on every chat request. `None` omits it.
    #[serde(default)]
    pub seed: Option<u64>,
```

Update the hand-written `impl Default for ExecutorConfig` (config.rs:48) to set both
to `None`.

### Task 2 — Carry the knobs on `OpenAiClient` and into the request body (`executor/src/ai/backends/openai.rs`)

- Add `temperature: Option<f64>` and `seed: Option<u64>` fields to `OpenAiClient`,
  and two corresponding parameters to `OpenAiClient::new` (append them after
  `stream_idle_timeout` — argument order is the executor's call, but keep `new`'s
  params positional and documented).
- Thread the two values into the request body. **Insert each key only when
  `Some`** — when `None`, the key must be **absent** from the body (not
  `null`, not a hardcoded default). The cleanest seam is to extend
  `build_chat_body` with two trailing `Option` params and insert inside it:

  ```rust
  if let Some(t) = temperature {
      body["temperature"] = json!(t);
  }
  if let Some(s) = seed {
      body["seed"] = json!(s);
  }
  ```

  Update the one production call site (`openai.rs:165`) and the existing
  `build_chat_body_*` test call sites to pass the new params (pass `None, None`
  where a test doesn't care).

### Task 3 — Pass config knobs through `make_client` (`executor/src/ai/mod.rs`)

Hand `cfg.temperature` and `cfg.seed` to `OpenAiClient::new`.

### Task 4 — Record the configured knobs at the emit site (`mcp/src/runner.rs`)

Replace `generation_params: GenerationParams::default()` (runner.rs:190) with a
value built from config:

```rust
generation_params: GenerationParams {
    temperature: inp.cfg.executor.temperature,
    seed: inp.cfg.executor.seed,
},
```

(`GenerationParams` is already imported here.)

## Acceptance criteria

- [ ] `ExecutorConfig` has `temperature: Option<f64>` and `seed: Option<u64>`,
      both defaulting to `None`, parsed from `[executor]` in `rexymcp.toml`.
- [ ] When `temperature`/`seed` are set in config, the chat request body contains
      `"temperature"` / `"seed"` with those values; when unset, **neither key is
      present** in the body.
- [ ] `OpenAiClient` carries the two settings and `make_client` populates them
      from `ExecutorConfig`.
- [ ] A phase run executed with configured settings records them in
      `PhaseRun.generation_params` (the same values that were sent); with no
      settings configured, `generation_params` is both-`None` and `rexymcp runs`
      renders that run's settings cell as `default`.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

Hermetic unit tests; no real network. Mirror the existing `config.rs` and
`openai.rs` test patterns.

In `executor/src/config.rs` `#[cfg(test)] mod tests`:

- `config_defaults_sampling_settings_to_none` — `ExecutorConfig::default()` has
  `temperature == None` and `seed == None`.
- `config_loads_sampling_settings` — a `[executor]` block with
  `temperature = 0.2` and `seed = 42` parses to `Some(0.2)` / `Some(42)`.
- `config_omits_sampling_settings_keeps_none` — an `[executor]` block **without**
  those keys parses to `None`/`None` (must-NOT default to a number — the boundary
  that keeps "unset" distinct from "set to 0").

In `executor/src/ai/backends/openai.rs` `#[cfg(test)] mod tests`:

- `build_chat_body_includes_temperature_and_seed_when_set` — with
  `Some(0.2)`, `Some(42)`, `body["temperature"] == 0.2` and `body["seed"] == 42`.
- `build_chat_body_omits_sampling_keys_when_none` — with `None, None`,
  `body.get("temperature").is_none()` **and** `body.get("seed").is_none()`
  (must-NOT-emit boundary — assert *absence*, not `== null`).
- `build_chat_body_omits_only_unset_key` — with `Some(0.7), None`,
  `body["temperature"] == 0.7` and `body.get("seed").is_none()` (independent
  per-key omission).

In `mcp/src/runner.rs` (or wherever the emit-site population is unit-testable):
prefer asserting the mapping directly. If `run_phase` is not unit-testable without
a live client, instead add a focused test that a `GenerationParams` built from an
`ExecutorConfig` with `temperature = Some(0.2), seed = Some(42)` carries those
values — the point is to pin the config→record mapping, not to spin up the loop.
If neither is cleanly reachable, cover the mapping via the end-to-end step below
and note in "Notes for review" that the emit population is verified E2E rather than
by unit test.

Per `STANDARDS.md` §3: each new config field and each body-insertion branch gets a
must-match and a must-NOT (the `None`→absent boundary is the one that matters most
— it's how "unset" stays distinguishable from "explicitly zero").

## End-to-end verification

This phase changes a real config surface and a real wire request. Verify both
halves against real artifacts and quote the output in the completion Update Log:

1. **Recorded half (no live model needed).** Write a `rexymcp.toml` with
   `[executor] temperature = 0.2` and `seed = 42` (and a `[telemetry] dir`), and a
   matching config without those keys. Demonstrate — by whatever real path is
   reachable without a live endpoint — that a `PhaseRun` emitted under the first
   config has `generation_params = {temperature: 0.2, seed: 42}` and under the
   second has both `None`. Then run `cargo run -p rexymcp -- runs --config <that
   toml>` and quote the table: the configured run's SETTINGS cell shows
   `temp=0.2,seed=42`; the unset run shows `default`. (If you cannot emit a real
   `PhaseRun` without a live model, construct the two `GenerationParams` from the
   two loaded configs and quote `rexymcp runs` against a hand-written store whose
   records carry those exact params — the point is to show config→record→display
   end-to-end.)
2. **Sent half.** Quote a `build_chat_body` result (from a test or a tiny harness)
   for the configured case showing `"temperature": 0.2` and `"seed": 42` present
   in the JSON, and for the unset case showing both keys absent. (`build_chat_body`
   is pure, so a unit-test assertion quoted in the log satisfies this.)

## Authorizations

- [x] May add the `temperature`/`seed` fields to `ExecutorConfig`, thread them
      through `OpenAiClient` / `make_client` / `build_chat_body`, and populate the
      emit site in `mcp/src/runner.rs`.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.
- [ ] Do **not** change the `GenerationParams` struct, the `PhaseRun` schema, the
      `AiClient` trait, or `MockAiClient`.

## Out of scope

- **Endpoint-reported provenance — phase-05b.** Served model id (chat-response
  `model` field), `finish_reason` (and the `length`-truncation rate), and the
  model's context window (`max_model_len` from `/v1/models`) are all phase-05b.
  Do **not** parse the streaming response for `finish_reason`/`model`, do **not**
  touch `executor/src/health.rs`, and do **not** add fields to `PhaseRun` here.
- **Additional sampling knobs** (`top_p`, `top_k`, `min_p`, `repetition_penalty`,
  `max_tokens` overrides, …). `GenerationParams` models exactly `temperature` and
  `seed`; widening it is a separate decision. Keep this phase to those two.
- **Env-var overrides for the new knobs** (a `REXYMCP_TEMPERATURE` /
  `REXYMCP_SEED` in `Config::apply_overrides`). The config-file path is the lever
  for this phase; env overrides can be added later if wanted.
- **A `model × settings` scorecard slice** — phase-06, depends on this phase's
  real data.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
