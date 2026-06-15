# Phase 06: wire the per-model override resolution into the dispatch path

**Milestone:** M18 — Capability-Aware Adaptation
**Status:** todo
**Depends on:** phase-05 (the pure `Config::resolve_for_model` substrate — done)
**Estimated diff:** ~130 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Make the per-model override table actually *take effect*. Phase-05 shipped
`Config::resolve_for_model(&mut self, model)` (pure, fully unit-tested) but with
**no caller** — the live dispatch path still reads global
`[executor]`/`[governor]` knobs. This phase wires the resolve call into
`mcp/src/runner.rs` so the active model's `[models."<id>"]` overrides reach both
the **live AI client** (the wire request's `temperature`/`seed`) and the **loop
deps** (`task_tracking`, the governor thresholds, and the telemetry
`generation_params`). It also documents the `[models]` section in the
`rexymcp init` template so users discover the knob.

This is the pinned consumer the 05 split promised: "wiring `resolve_for_model`
into `mcp/src/runner.rs` … plus the `rexymcp init` template documentation is
phase-06" (phase-05 § Goal).

## Architecture references

Read before starting:

- `docs/architecture.md#configuration` — the config knobs this phase resolves
  per-model.
- `docs/dev/milestones/M18-capability-adaptation/README.md` § "Threads → phases"
  — thread 3 (model-conditioned runtime knobs); exit criterion: "At least one
  runtime knob … resolves a **per-model** default derived from config, overriding
  the global default."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`Config::resolve_for_model` exists and is tested in
`executor/src/config.rs:275-297`. It mutates `self.executor.{task_tracking,
temperature, seed}` and `self.governor.{identical_call_threshold,
verifier_persistence_threshold, runaway_output_bytes}` in place for an
exact-match `[models]` key, and is a no-op for an unknown model. **Do not modify
`config.rs`** — it is complete.

The dispatch path is `mcp/src/runner.rs`. Two functions consume the knobs, and
**both read the global values today**:

### `run_phase` builds the live client (the wire authority)

`runner.rs:256-308`. The production `OpenAiClient` is constructed here, and its
`temperature`/`seed` are what the loop actually sends on every chat request
(`executor/src/ai/backends/openai.rs:184-185` reads `self.temperature` /
`self.seed`):

```rust
pub async fn run_phase(inp: &RunPhaseConfig<'_>) -> rexymcp_executor::error::Result<PhaseResult> {
    let model = inp.model_override.unwrap_or(&inp.cfg.executor.model);

    let prod_client = OpenAiClient::new(
        inp.cfg.executor.api_key.clone().unwrap_or_default(),
        model.to_string(),
        inp.cfg.executor.base_url.clone(),
        std::time::Duration::from_secs(inp.cfg.executor.first_token_timeout_secs),
        std::time::Duration::from_secs(inp.cfg.executor.stream_idle_timeout_secs),
        inp.cfg.executor.temperature,   // <-- global; must be resolved
        inp.cfg.executor.seed,          // <-- global; must be resolved
    );
    // ...
    let assembly = AssemblyInput {
        cfg: inp.cfg,
        // ...
        model,
        context_window: if inp.test_client.is_none() {
            rexymcp_executor::health::fetch_context_window(&inp.cfg.executor, model).await
        } else {
            None
        },
        // ...
    };
    run_phase_with(&assembly, &seams).await
}
```

### `run_phase_with` builds the loop deps (telemetry + governor + task tracking)

`runner.rs:173-237`. This is the hermetic, seam-injected assembler (the tests
drive it with `NoopVerifier`/`NoopRunner`/`MockAiClient`). It reads the same
knobs off `inp.cfg`:

```rust
    let tasks = if inp.cfg.executor.task_tracking {                 // line 185
        Some(rexymcp_executor::agent::tasks::seed_from_spec(&phase_doc))
    } else {
        None
    };
    // ...
    let deps = LoopDeps {
        // ...
        generation_params: GenerationParams {
            temperature: inp.cfg.executor.temperature,             // line 226
            seed: inp.cfg.executor.seed,                           // line 227
        },
        // ...
        governor: inp.cfg.governor,                                // line 232
        task_tracking: inp.cfg.executor.task_tracking,             // line 233
    };
```

`deps.generation_params` is **telemetry-only** — it is recorded verbatim into
the `PhaseRun` (`executor/src/agent/metrics.rs:107`,
`generation_params: deps.generation_params.clone()`), written to
`phase_runs.jsonl` by `telemetry::append` (`executor/src/store/telemetry.rs:161`)
whenever `telemetry_dir` is set. `deps.governor` and `deps.task_tracking` drive
loop behavior.

### The `rexymcp init` template

`mcp/src/init.rs:7-59` — `generate_config` emits a fully-commented
`rexymcp.toml`. There is a `[governor]` block (lines 32-35) but **no `[models]`
documentation**. Users have no way to discover the override table.

## Why two resolve calls (read this before coding)

`temperature`/`seed` are consumed in **two independent places that live in two
different functions**:

1. The **live client** is built in `run_phase` (`OpenAiClient::new`) and bakes
   `temperature`/`seed` at construction — this is what reaches the model on the
   wire.
2. The **loop deps / telemetry** are built in `run_phase_with` — this is what
   records `generation_params` and drives the governor/task-tracking.

`run_phase_with` receives an already-built client through `seams.client`; it
cannot retro-fix the client's baked sampling. So each function resolves the
config it owns:

- `run_phase` resolves a clone to build the client with resolved sampling.
- `run_phase_with` resolves a clone to read resolved `task_tracking` / governor /
  `generation_params` into the deps.

Both resolves are **necessary and non-redundant**: `run_phase` passes the
**unresolved** `inp.cfg` down to `AssemblyInput`, so `run_phase_with` must
resolve its own copy. Deleting either breaks correctness for its consumer. This
is deliberate — do **not** "simplify" it to a single resolve; the client and the
deps are assembled in different functions across the prod/test seam boundary.

(The resolve targeting only `executor.{task_tracking,temperature,seed}` and
`governor` means non-overridable fields — `base_url`, `api_key`, timeouts,
`budget`, `commands`, `context` — can keep reading `inp.cfg` unchanged. Only the
overridable knobs switch to the resolved clone.)

## Spec

Two files: `mcp/src/runner.rs` (the wiring + tests) and `mcp/src/init.rs` (the
template doc + a test). **`executor/src/config.rs` is not touched.**

### 1. Resolve for the live client in `run_phase`

In `mcp/src/runner.rs`, in `run_phase` (the `let model = …` / `OpenAiClient::new`
block, ~lines 257-267):

- Make `model` an owned `String` (the resolve call below borrows it, and it is
  later borrowed by `AssemblyInput.model`):

  ```rust
  let model = inp
      .model_override
      .map(str::to_string)
      .unwrap_or_else(|| inp.cfg.executor.model.clone());
  ```

- Resolve a clone for the client, then build the client from it:

  ```rust
  // Per-model overrides for the wire client's sampling. The loop deps resolve
  // independently in `run_phase_with` (see "Why two resolve calls" in the phase
  // doc) — `inp.cfg` is passed down unresolved.
  let mut client_cfg = inp.cfg.clone();
  client_cfg.resolve_for_model(&model);

  let prod_client = OpenAiClient::new(
      client_cfg.executor.api_key.clone().unwrap_or_default(),
      model.clone(),
      client_cfg.executor.base_url.clone(),
      std::time::Duration::from_secs(client_cfg.executor.first_token_timeout_secs),
      std::time::Duration::from_secs(client_cfg.executor.stream_idle_timeout_secs),
      client_cfg.executor.temperature,
      client_cfg.executor.seed,
  );
  ```

- In the `AssemblyInput` literal: leave `cfg: inp.cfg` (unresolved — see the
  rationale), and change `model` to a borrow of the owned string: `model: &model`.
  The `fetch_context_window(&inp.cfg.executor, model)` call becomes
  `fetch_context_window(&inp.cfg.executor, &model)` (provider/base_url/model are
  not overridable, so `inp.cfg.executor` is correct here).

`model` previously was `&str` borrowed from `inp`; making it an owned `String`
is required because `client_cfg` is dropped at end of scope but `model` is reused.
The `clock`/`seams`/rest of `run_phase` are unchanged.

### 2. Resolve for the loop deps in `run_phase_with`

In `mcp/src/runner.rs`, in `run_phase_with`, after the existing
`let phase = derive_phase_id(...)` / `let scope = …` setup and **before** the
`let tasks = …` block (~line 185), add:

```rust
let mut cfg = inp.cfg.clone();
cfg.resolve_for_model(inp.model);
```

Then switch the **overridable** reads from `inp.cfg` to the resolved `cfg`:

- `let tasks = if cfg.executor.task_tracking {` (was `inp.cfg.executor.task_tracking`)
- in `GenerationParams`: `temperature: cfg.executor.temperature` and
  `seed: cfg.executor.seed`
- `governor: cfg.governor`
- `task_tracking: cfg.executor.task_tracking`

Leave **all other** `inp.cfg.*` reads as-is: `inp.cfg.context.output_filter`
(line 191), `inp.cfg.budget.*` (lines 193-196, 217), `inp.cfg.commands` (line
223) — none of those are overridable, and `commands: &inp.cfg.commands` borrows
`inp.cfg`, which must stay valid (the resolved `cfg` is a separate owned clone
read only for its `Copy` scalar fields, so there is no borrow conflict).

### 3. Document `[models]` in the `rexymcp init` template

In `mcp/src/init.rs`, in `generate_config`, add a commented `[models]` block to
the emitted template. Place it after the `[governor]` block (it overrides
`[executor]`/`[governor]` knobs). Keep the whole block commented so the generated
config still parses to an empty `models` map. Use a realistic example id and
cover every override field with an inline "inherits global if omitted" note:

```toml

# [models."<model-id>"]              # per-model knob overrides; key is the exact
#                                    # active model id (no prefix/substring match).
#                                    # Any key omitted here inherits the global value above.
# task_tracking = false              # override [executor] task_tracking for this model
# temperature = 0.2                  # override [executor] temperature
# seed = 7                           # override [executor] seed
# identical_call_threshold = 8       # override [governor] identical_call_threshold
# verifier_persistence_threshold = 8 # override [governor] verifier_persistence_threshold
# runaway_output_bytes = 204800      # override [governor] runaway_output_bytes
```

(Exact comment wording is the executor's call — pin the **content**: a commented
`[models."…"]` section naming all six overridable fields and stating that omitted
keys inherit the global value.)

## Acceptance criteria

- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      and `cargo fmt --all --check` are clean (no new warnings, no `#[allow]`).
- [ ] `cargo test` passes (existing + new).
- [ ] `run_phase_with` resolves the active model's override before building the
      loop deps: with a global `temperature` and a matching `[models]` override of
      a different `temperature`, the recorded `PhaseRun.generation_params.temperature`
      equals the **override** value.
- [ ] `run_phase_with` with a model that has no `[models]` entry records the
      **global** sampling (resolution is a no-op).
- [ ] `executor/src/config.rs` is unchanged in this phase.
- [ ] The `rexymcp init`-generated `rexymcp.toml` contains a commented `[models]`
      override section and still loads via `Config::load` with an empty
      `config.models` map.

## Test plan

Hermetic tests. The resolution *logic* is already covered in `config.rs`
(phase-05); these tests pin the **wiring** — that the dispatch path calls
`resolve_for_model` with the active model and the resolved values reach the
recorded telemetry. Observe via the real `telemetry::append` →
`telemetry::read` round-trip (the deps' `generation_params` is the hermetically
observable resolved knob; governor/task_tracking are not surfaced in the
`PhaseResult`, but they resolve through the *same* `resolve_for_model` call this
test exercises, so one observable knob mutation-protects the call).

Mirror the existing `run_phase_with_assembles_and_returns_result` harness
(`runner.rs:463-513`): `TempDir` repo, a phase-doc file, `MockAiClient::new(vec!["Done.".to_string()])`,
`NoopVerifier`, `NoopRunner`, a fixed `clock`. Reference `ModelOverride` as
`rexymcp_executor::config::ModelOverride` (add it to the test `use` group, or use
the full path).

- `run_phase_with_resolves_per_model_sampling_into_telemetry` in `mcp/src/runner.rs`
  — build a `Config` with `cfg.executor.temperature = Some(0.8)` and
  `cfg.models.insert("override-model".into(), ModelOverride { temperature: Some(0.2), ..Default::default() })`;
  set `AssemblyInput.model = "override-model"` and `telemetry_dir = Some(tmp)`.
  After `run_phase_with`, read
  `rexymcp_executor::store::telemetry::read(&tmp.join("phase_runs.jsonl"))` and
  assert the single run's `generation_params.temperature == Some(0.2)`.
  **Mutation-resistant:** without the resolve call the value would be `Some(0.8)`.

- `run_phase_with_unknown_model_keeps_global_sampling` in `mcp/src/runner.rs` —
  same setup, but `AssemblyInput.model = "different-model"` (the only `[models]`
  key is `"override-model"`). Assert the recorded
  `generation_params.temperature == Some(0.8)` (global; resolution is a no-op for
  the unmatched id). **Pinned negative.**

- `init_documents_models_override_section` in `mcp/src/init.rs` — run
  `init::run(dir, false)`, read the generated `rexymcp.toml`, assert the content
  contains `[models."` (the documented override section) and that
  `Config::load(&path)` succeeds with `cfg.models.is_empty()` (the block is
  commented). **Pinned:** documents the knob without activating it.

## End-to-end verification

This phase ships two real artifacts; verify both against the running binary and
quote the output in the completion Update Log:

1. **The `rexymcp init` template** — run the real binary into a temp dir and
   confirm the `[models]` documentation is present in the generated file:

   ```
   cargo run -p rexymcp -- init --dir /tmp/m18p06-e2e
   grep -n 'models' /tmp/m18p06-e2e/rexymcp.toml
   ```

   Paste the matching lines. (Clean up the temp dir after.)

2. **The resolution wiring** — the live-loop resolution is only fully observable
   with a real LLM endpoint, which is out of scope for a hermetic phase. The
   `run_phase_with_resolves_per_model_sampling_into_telemetry` test is the
   real-artifact check: it writes an actual `phase_runs.jsonl` that the real
   `telemetry::read` parses, proving the resolved `generation_params` is
   persisted. Restate this in the Update Log; do **not** fabricate a live-dispatch
   transcript.

## Authorizations

- Edit **`mcp/src/runner.rs`** (the two resolve calls + the read switches + the
  two new tests) and **`mcp/src/init.rs`** (the template `[models]` block + one
  test).
- No new dependency (`OpenAiClient`, `Config`, `ModelOverride`, `telemetry::read`
  are all already in scope or in the `executor` crate).
- No edits to `executor/src/config.rs`, `architecture.md`, `Cargo.toml`,
  `STANDARDS.md`, or `WORKFLOW.md`.

## Out of scope

- **Changing `Config::resolve_for_model`** or anything in `config.rs` — it is
  complete (phase-05).
- **Profile-driven *automatic* knob selection** (reading `model_profile` to pick
  overrides). M18 keeps the human in the loop: the architect reads the profile
  and hand-writes the `[models]` table. No automated model→knob routing.
- **Router breadth** as a per-model knob — it has no global config home today
  (a router constant); exposing it is a separate concern (phase-05 § Out of
  scope).
- **Unifying the two `temperature`/`seed` sources** (client constructor vs
  `GenerationParams`) into one — that would change the `AiClient::chat` seam, a
  larger refactor. Keep both; resolve both.
- Any change to how knobs are *consumed* (the governor, the AI client's request
  builder, the loop) — this phase only resolves values into the client and deps.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
