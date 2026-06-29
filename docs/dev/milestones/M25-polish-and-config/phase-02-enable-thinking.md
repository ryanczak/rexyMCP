# Phase 02: Default thinking off (`enable_thinking`)

**Milestone:** M25 — Polish & Config Pass
**Status:** review
**Depends on:** none
**Estimated diff:** ~150 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Thinking models (Qwen3, Gemma) served over vLLM emit a `<think>` reasoning block
by default, burning output tokens before they reach a tool call. Add an
`enable_thinking` knob — a `[executor]` global defaulting to **false** (thinking
off), overridable per-model in `[models."<id>"]` — that, when `false`, suppresses
reasoning on the wire by sending `chat_template_kwargs.enable_thinking = false` in
the chat-completion body. This threads through the backend **exactly** like the
`max_tokens` knob added in M23, with one twist: the value is a plain `bool` (not
`Option`), and it controls whether a wire key is *emitted* rather than what value
a key carries.

## Architecture references

Read before starting:

- `docs/dev/milestones/M25-polish-and-config/README.md` — milestone overview and
  the issue-6 decision ("per-model overridable, default off").
- `docs/architecture.md` § Status #25 — milestone summary.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`max_tokens` (added in M23 phase-01) is the **exact precedent** for this phase —
the same config field → `SamplingParams` → `build_chat_body` → two call-site
path. Study how `max_tokens` is threaded and replicate the shape for
`enable_thinking`. The only structural differences are spelled out in the Spec.

### The wire contract

vLLM renders the served model's Jinja chat template with any `chat_template_kwargs`
the request supplies. The Qwen3 / Gemma templates read `enable_thinking`: when it
is `false`, the template omits the reasoning prelude and the model produces no
`<think>` block. So the wire change this phase ships is:

- `enable_thinking == false` → body gains `"chat_template_kwargs": { "enable_thinking": false }`.
- `enable_thinking == true` → body has **no** `chat_template_kwargs` key at all
  (the endpoint applies its own default, which for thinking models is
  thinking-on). This mirrors the `temperature`/`seed` idiom: omit the key to let
  the endpoint decide.

Because the default is `false`, the default config always emits the suppression
key — that is the whole point of the phase ("default thinking off").

### Touch points (five files)

**1. `executor/src/config.rs`** — two struct fields + a resolve arm.

`ExecutorConfig` (the `max_tokens` field is the model to copy, `config.rs:268-275`):

```rust
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
```

and its free-fn default (`config.rs:300-302`):

```rust
fn default_max_tokens() -> u32 {
    8192
}
```

and the `Default` impl line (`config.rs:315`): `max_tokens: default_max_tokens(),`.

`ModelOverride` (`config.rs:197-209`) currently ends:

```rust
pub struct ModelOverride {
    pub task_tracking: Option<bool>,
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub max_tokens: Option<u32>,
    pub identical_call_threshold: Option<usize>,
    ...
}
```

`resolve_for_model` (`config.rs:445-476`) has one arm per overridable field:

```rust
        if let Some(v) = over.max_tokens {
            self.executor.max_tokens = v;
        }
```

**2. `executor/src/ai/mod.rs`** — `SamplingParams` + `make_client`.

```rust
#[derive(Debug, Clone, Copy)]
pub struct SamplingParams {
    pub temperature: Option<f64>,
    pub seed: Option<u64>,
    pub max_tokens: u32,
}

impl Default for SamplingParams {
    fn default() -> Self {
        Self {
            temperature: None,
            seed: None,
            max_tokens: 8192,
        }
    }
}

pub fn make_client(cfg: &ExecutorConfig) -> Box<dyn AiClient> {
    Box::new(OpenAiClient::new(
        ...
        SamplingParams {
            temperature: cfg.temperature,
            seed: cfg.seed,
            max_tokens: cfg.max_tokens,
        },
    ))
}
```

**3. `executor/src/ai/backends/openai.rs`** — `build_chat_body` (lines 84-139).
The temperature/seed emission at the end is the shape to follow:

### Update — 2026-06-29 14:51 (started)
### Update — 2026-06-29 14:51 (started)

**Executor:** Claude (Sonnet)

**What I'm doing:** Implementing the `enable_thinking` config knob across 5 files: config.rs, ai/mod.rs, openai.rs, runner.rs, init.rs. Threading `enable_thinking: bool` through `ExecutorConfig`, `ModelOverride`, `SamplingParams`, and the wire body. Adding 7 new tests and extending 1 existing test.

### Update — 2026-06-29 14:55 (complete)

**Verification commands:**
- `cargo fmt --all --check`: ✓ passed
- `cargo build`: ✓ passed, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: ✓ passed
- `cargo test`: ✓ 871 passed, 0 failed, 2 ignored

**End-to-end verification:**
- The `rexymcp init` template now contains `enable_thinking` in both `[executor]` and `[models]` blocks (verified by the extended `init_documents_models_override_section` test which asserts `count >= 2` matches of `enable_thinking`).
- The wire behavior (`chat_template_kwargs` emission/suppression) is covered by the hermetic `build_chat_body` tests — no live endpoint call is required for this phase.

**Files changed:**
- `executor/src/config.rs` — Added `enable_thinking: bool` to `ExecutorConfig`, `enable_thinking: Option<bool>` to `ModelOverride`, `default_enable_thinking()` free function, resolve arm in `resolve_for_model`, and 5 new tests.
- `executor/src/ai/mod.rs` — Added `enable_thinking: bool` to `SamplingParams`, `Default` impl, `make_client` wiring, 3 struct-literal updates in tests, and 1 new test.
- `executor/src/ai/backends/openai.rs` — Added `chat_template_kwargs` emission in `build_chat_body`, 2 struct-literal updates in tests, and 2 new tests.
- `executor/src/health.rs` — Added `enable_thinking: false` to test struct literal.
- `mcp/src/runner.rs` — Added `enable_thinking` to `SamplingParams` literal, 2 struct-literal updates in tests.
- `mcp/src/init.rs` — Added `enable_thinking` comment lines in both `[executor]` and `[models]` blocks, extended existing test.
- `docs/dev/milestones/M25-polish-and-config/phase-02-enable-thinking.md` — Status → review, Update Log entries.
- `docs/dev/milestones/M25-polish-and-config/README.md` — Phase table row → review.

**Grep verification for pinned literal `enable_thinking`:**
```
grep -rn "enable_thinking" executor/src/config.rs executor/src/ai/mod.rs executor/src/ai/backends/openai.rs mcp/src/runner.rs mcp/src/init.rs | wc -l
→ 28 matches across all 5 files
```

**Notes for review:** None — clean implementation mirroring the `max_tokens` precedent.
    if let Some(t) = sampling.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(s) = sampling.seed {
        body["seed"] = json!(s);
    }
    body
```

**4. `mcp/src/runner.rs`** — the wire-client `SamplingParams` site (lines 281-285):

```rust
        SamplingParams {
            temperature: client_cfg.executor.temperature,
            seed: client_cfg.executor.seed,
            max_tokens: client_cfg.executor.max_tokens,
        },
```

**5. `mcp/src/init.rs`** — the generated-template docs (lines 16-47): the
`[executor]` comment block (after the `max_tokens` line at 22) and the `[models]`
comment block (after the `max_tokens` override line at 44).

## Spec

Numbered tasks in execution order.

1. **Add the `enable_thinking` config field** — in `executor/src/config.rs`, add
   to `ExecutorConfig` (place it directly after `max_tokens`):

   ```rust
   /// Whether the served model's chat template renders its reasoning block.
   /// Default **false** (thinking off) to stop thinking models from burning the
   /// output budget on `<think>` reasoning before reaching a tool call. When
   /// false, the backend sends `chat_template_kwargs.enable_thinking = false`;
   /// when true, the key is omitted and the endpoint applies its own default.
   #[serde(default = "default_enable_thinking")]
   pub enable_thinking: bool,
   ```

   Add the free-fn default next to `default_max_tokens`:

   ```rust
   fn default_enable_thinking() -> bool {
       false
   }
   ```

   Add `enable_thinking: default_enable_thinking(),` to the `Default for
   ExecutorConfig` impl.

2. **Add the per-model override** — in the same file, add to `ModelOverride`:

   ```rust
   pub enable_thinking: Option<bool>,
   ```

   and add the resolve arm to `resolve_for_model` (mirror the `max_tokens` arm):

   ```rust
   if let Some(v) = over.enable_thinking {
       self.executor.enable_thinking = v;
   }
   ```

3. **Thread it into `SamplingParams`** — in `executor/src/ai/mod.rs`, add
   `pub enable_thinking: bool,` to the struct, `enable_thinking: false,` to the
   `Default` impl, and `enable_thinking: cfg.enable_thinking,` to the
   `SamplingParams { … }` literal inside `make_client`.

4. **Emit the wire key** — in `executor/src/ai/backends/openai.rs`, in
   `build_chat_body`, after the `seed` block and before the final `body`, add:

   ```rust
   if !sampling.enable_thinking {
       body["chat_template_kwargs"] = json!({ "enable_thinking": false });
   }
   ```

   **Do not** emit `chat_template_kwargs` when `enable_thinking` is true — the
   key must be entirely absent in that case (see § "The wire contract" and the
   pinned negative test below).

5. **Wire the runner call site** — in `mcp/src/runner.rs`, add
   `enable_thinking: client_cfg.executor.enable_thinking,` to the
   `SamplingParams { … }` literal at lines 281-285.

6. **Document the knob in the init template** — in `mcp/src/init.rs`, add one
   commented line to the `[executor]` block (after the `max_tokens` line):

   ```
   # enable_thinking = false        # render the model's <think> reasoning block (default false = off)
   ```

   and one to the `[models]` override block (after its `max_tokens` line):

   ```
   # enable_thinking = false            # override [executor] enable_thinking
   ```

### Mechanically-required struct-literal updates

These are not new behavior — they are the compiler's E0063 consequence of adding
a field to `SamplingParams`, `ExecutorConfig`, and `ModelOverride`. Update each
to keep the build green. The complete grep-verified list:

- **`ExecutorConfig { … }` full literals** (add `enable_thinking: false,`):
  - `executor/src/ai/mod.rs:263` (`make_client_openai`)
  - `executor/src/ai/mod.rs:281` (`make_client_ollama`)
  - `executor/src/ai/mod.rs:299` (`make_client_lmstudio`)
  - `executor/src/health.rs:147` — **verify** whether this literal is full or uses
    `..Default::default()`; add the field only if the compiler flags it.
- **`SamplingParams { … }` full literals** (add `enable_thinking: false,` — these
  do **not** use `..Default::default()`):
  - `executor/src/ai/backends/openai.rs:660` (`build_chat_body_includes_temperature_and_seed_when_set`)
  - `executor/src/ai/backends/openai.rs:684` (`build_chat_body_omits_only_unset_key`)
  - (the literal at `openai.rs:707` already uses `..SamplingParams::default()` —
    leave it alone.)
- **`ModelOverride { … }` full literals** (add `enable_thinking: None,`):
  - `mcp/src/runner.rs` — the two test literals (around lines 595-605 and 714-724)
    that list every field explicitly.

After adding each field, run `cargo build` and let the compiler walk you to any
literal this list missed — a missing site is the only way this class of change
breaks. Do **not** add `#[allow(dead_code)]` or skip a literal; if you hit a
literal you cannot resolve from this doc, file a blocker.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `ExecutorConfig::default().enable_thinking == false`.
- [ ] A `[executor] enable_thinking = true` in TOML loads as `true`; absent loads
      as `false`.
- [ ] A `[models."<id>"] enable_thinking = true` override flips the global `false`
      to `true` after `resolve_for_model("<id>")`; an override that omits
      `enable_thinking` leaves the global value untouched.
- [ ] `build_chat_body` with `enable_thinking: false` produces a body where
      `body["chat_template_kwargs"]["enable_thinking"] == false`.
- [ ] `build_chat_body` with `enable_thinking: true` produces a body with **no**
      `chat_template_kwargs` key (`body.get("chat_template_kwargs").is_none()`).
- [ ] The `rexymcp init` template documents `enable_thinking` in both the
      `[executor]` and `[models]` blocks, and the generated config still loads.

## Test plan

Mirror the `max_tokens` tests already in the tree.

- `enable_thinking_defaults_false` in `config.rs` — `ExecutorConfig::default()`.
- `loads_enable_thinking_from_toml` in `config.rs` — `[executor] enable_thinking
  = true` → `Some`-style assert `cfg.executor.enable_thinking`.
- `enable_thinking_absent_keeps_default_false` in `config.rs` — TOML with no
  `enable_thinking` → `false`.
- `resolve_for_model_applies_enable_thinking_override` in `config.rs` — global
  `false`, `[models."m"] enable_thinking = true`, resolve → `true`.
- `resolve_for_model_leaves_enable_thinking_when_override_absent` in `config.rs` —
  global `true` (set it explicitly in the TOML), `[models."m"]` overrides only
  `temperature`, resolve → still `true`.
- `sampling_params_default_enable_thinking_is_false` in `ai/mod.rs` — mirrors
  `sampling_params_default_max_tokens_is_8192`.
- `build_chat_body_suppresses_thinking_when_disabled` in `openai.rs` — asserts
  `body["chat_template_kwargs"]["enable_thinking"] == false`.
- `build_chat_body_omits_chat_template_kwargs_when_thinking_enabled` in
  `openai.rs` — the **pinned negative**: builds with `enable_thinking: true` and
  asserts `body.get("chat_template_kwargs").is_none()`. This is mutation-resistant
  — removing the `if !sampling.enable_thinking` guard (always emitting) fails it.
- An `init.rs` assertion that the generated template contains `enable_thinking`
  (extend the existing template-content test rather than adding a brittle new one
  if one already covers the `[models]` block).

## End-to-end verification

The runtime-loadable artifact is the `rexymcp init` template. After implementing,
run `rexymcp init --config /tmp/et-check.toml` (or the workspace equivalent),
confirm the generated file contains the two `enable_thinking` comment lines, then
`Config::load` it (the existing init test already round-trips the template) and
quote the relevant template lines in the completion Update Log.

The wire behavior (`chat_template_kwargs` on the actual HTTP body) is covered by
the hermetic `build_chat_body` tests — no live endpoint call is required for this
phase. State that explicitly under End-to-end verification.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `docs/architecture.md` edit.

## Out of scope

- **Response-side reasoning parsing** — the `in_reasoning` / `</think>` handling
  in `OpenAiClient::chat` (`openai.rs:253-282`) is untouched. With thinking off
  the model simply emits no reasoning deltas, so that path stays dormant; do not
  modify it.
- **Any other M25 issue** (the dashboard panels, the dep bumps) — those are
  separate phases.
- **Forcing thinking off via a system-prompt directive or `/no_think` token** —
  the suppression is purely the `chat_template_kwargs` wire field; do not add
  prompt-level instructions.
- **Changing `max_tokens`, `temperature`, or `seed` semantics** — only add the
  new field alongside them.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
