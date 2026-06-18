# Phase 03: `SamplingParams` refactor + `format_no_match` fix

**Milestone:** M23 — Truncation & Empty-Completion Recovery
**Status:** done
**Depends on:** M23 phase-01 (which added `max_tokens` as the third sampling knob and
introduced the `#[allow(clippy::too_many_arguments)]` on `OpenAiClient::new`)
**Estimated diff:** ~90 lines
**Tags:** language=rust, kind=refactor, size=s

## Goal

Two cleanup items flagged in the M23 retrospective:

1. **Collapse the three sampling knobs into `SamplingParams`.** `OpenAiClient::new`
   now takes 8 positional arguments — one above clippy's `too_many_arguments`
   threshold (7) — and carries an authorized `#[allow]` that should be temporary.
   Introduce `SamplingParams { temperature, seed, max_tokens }` in
   `executor/src/ai/mod.rs`, replace the three trailing args on both
   `build_chat_body` and `OpenAiClient::new` with one `sampling: SamplingParams`
   arg, and remove the `#[allow]`. `OpenAiClient` stores the struct whole
   (`self.sampling`).

2. **Fix the `format_no_match` byte-slice panic.** `feedback.rs:45` uses
   `&response_excerpt[..200]`, which panics if byte 200 falls inside a multi-byte
   character. Replace with `response_excerpt.chars().take(200).collect::<String>()`,
   matching the char-safe pattern already used in `format_truncated`.

Both are pure refactors — no behavior change, no new dependency, no `Cargo.toml`
edit.

## Architecture references

Read before starting:

- `executor/src/ai/mod.rs` — `make_client` (**187–198**): currently passes
  `cfg.temperature, cfg.seed, cfg.max_tokens` as the last three args to
  `OpenAiClient::new`. After this phase it builds a `SamplingParams` inline and
  passes one arg. The struct is declared here, above `make_client`.
- `executor/src/ai/backends/openai.rs`:
  - `build_chat_body` signature (**83–91**): last three params are `temperature:
    Option<f64>`, `seed: Option<u64>`, `max_tokens: u32` → replaced by `sampling:
    SamplingParams`.
  - `build_chat_body` body: line **111** (`"max_tokens": max_tokens`) and lines
    **124–129** (temperature/seed conditionals) → reference `sampling.max_tokens`,
    `sampling.temperature`, `sampling.seed`.
  - `OpenAiClient` struct (**133–142**): fields `temperature: Option<f64>`,
    `seed: Option<u64>`, `max_tokens: u32` → replaced by `sampling: SamplingParams`.
  - `OpenAiClient::new` (**144–171**): remove `#[allow(clippy::too_many_arguments)]`
    (line 145); replace three params with `sampling: SamplingParams`; replace three
    field assignments with `sampling,`.
  - `chat` method — `build_chat_body` call (**184–192**): replace
    `self.temperature, self.seed, self.max_tokens` with `self.sampling`.
- `mcp/src/runner.rs` — prod call site (**275–283**): replace the three trailing
  args `client_cfg.executor.temperature, client_cfg.executor.seed,
  client_cfg.executor.max_tokens` with an inline `SamplingParams { … }` struct
  literal. Also add `SamplingParams` to the `use rexymcp_executor::ai::{…}` import
  on line **9**.
- `executor/src/parser/feedback.rs` — `format_no_match` (**43–55**): the
  `&response_excerpt[..200]` byte-slice on line **45**.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### `OpenAiClient::new` has an authorized `#[allow]` — `openai.rs:144–154`

```rust
impl OpenAiClient {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_key: String,
        model: String,
        base_url: String,
        first_token_timeout: Duration,
        stream_idle_timeout: Duration,
        temperature: Option<f64>,
        seed: Option<u64>,
        max_tokens: u32,
    ) -> Self {
```

8 args. `SamplingParams` collapses the last 3 to 1, dropping to 6 args and
retiring the `#[allow]`.

### `build_chat_body` has the same three trailing params — `openai.rs:83–91`

```rust
pub fn build_chat_body(
    model: &str,
    system: &str,
    messages: Vec<Value>,
    tools: Option<&[ToolSchema]>,
    temperature: Option<f64>,
    seed: Option<u64>,
    max_tokens: u32,
) -> Value {
```

After the phase: 5 params (the last three replaced by `sampling: SamplingParams`).

### `format_no_match` byte-slices — `feedback.rs:44–46`

```rust
let excerpt = if response_excerpt.len() > 200 {
    format!("{}...", &response_excerpt[..200])
} else {
```

`[..200]` panics when byte 200 falls inside a multi-byte character (e.g. a string
of 199 ASCII bytes followed by a 2-byte UTF-8 character — `&s[..200]` splits the
character). `format_truncated` (added in phase-02) already uses the char-safe form;
this phase brings `format_no_match` in line.

## Spec

### Task 1 — `SamplingParams` struct in `executor/src/ai/mod.rs`

Insert before `make_client` (line 187):

```rust
/// Sampling knobs forwarded verbatim to every chat request.
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
```

`Copy` is derivable because all three fields are `Copy` (`Option<f64>`,
`Option<u64>`, `u32`). With `Copy`, `build_chat_body` takes `sampling:
SamplingParams` by value and `chat` passes `self.sampling` directly — no `.clone()`
needed anywhere.

The `Default` impl is manual because the derived `Default` for `u32` is `0`, not
`8192`. This matches the `default_max_tokens()` fn in `config.rs` (which is
private; just repeat the literal here — it's documented prominently).

### Task 2 — Update `make_client` in `executor/src/ai/mod.rs`

Replace the three trailing args in the `OpenAiClient::new` call:

```rust
pub fn make_client(cfg: &ExecutorConfig) -> Box<dyn AiClient> {
    Box::new(OpenAiClient::new(
        cfg.api_key.clone().unwrap_or_default(),
        cfg.model.clone(),
        cfg.base_url.clone(),
        Duration::from_secs(cfg.first_token_timeout_secs),
        Duration::from_secs(cfg.stream_idle_timeout_secs),
        SamplingParams {
            temperature: cfg.temperature,
            seed: cfg.seed,
            max_tokens: cfg.max_tokens,
        },
    ))
}
```

### Task 3 — Update `build_chat_body` in `openai.rs`

Replace the three trailing params with `sampling: SamplingParams` (by value —
`Copy` makes this free):

```rust
pub fn build_chat_body(
    model: &str,
    system: &str,
    messages: Vec<Value>,
    tools: Option<&[ToolSchema]>,
    sampling: SamplingParams,
) -> Value {
```

In the body, replace the three references:
- `"max_tokens": max_tokens,` → `"max_tokens": sampling.max_tokens,`
- `if let Some(t) = temperature {` → `if let Some(t) = sampling.temperature {`
- `if let Some(s) = seed {` → `if let Some(s) = sampling.seed {`

`build_chat_body` is in `openai.rs` and `SamplingParams` is in `ai/mod.rs`. The
import needed at the top of `openai.rs`:

```rust
use crate::SamplingParams;
```

(The `executor/src/ai/backends/openai.rs` module path is
`executor::ai::backends::openai`, so `crate::` is the executor lib root and
`crate::SamplingParams` is not right — `SamplingParams` lives in `crate::ai`, so
the import is `use crate::ai::SamplingParams;`. Verify the existing imports in
`openai.rs` for the path pattern.)

### Task 4 — Update `OpenAiClient` struct and `new` in `openai.rs`

Replace the three fields in the struct:

```rust
pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String,
    first_token_timeout: Duration,
    stream_idle_timeout: Duration,
    sampling: SamplingParams,
}
```

Remove `#[allow(clippy::too_many_arguments)]`. Replace the three params in `new`
with `sampling: SamplingParams`:

```rust
impl OpenAiClient {
    pub fn new(
        api_key: String,
        model: String,
        base_url: String,
        first_token_timeout: Duration,
        stream_idle_timeout: Duration,
        sampling: SamplingParams,
    ) -> Self {
        let resolved_url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url
        };
        OpenAiClient {
            api_key,
            model,
            base_url: resolved_url,
            first_token_timeout,
            stream_idle_timeout,
            sampling,
        }
    }
}
```

In the `chat` method, replace the three args in the `build_chat_body` call with
`self.sampling` (copied by value — no `.clone()` needed):

```rust
let body = build_chat_body(
    &self.model,
    system,
    converted,
    tools,
    self.sampling,
);
```

### Task 5 — Update `mcp/src/runner.rs` prod call site

Add `SamplingParams` to the existing import on line 9:

```rust
use rexymcp_executor::ai::{AiClient, OpenAiClient, SamplingParams, ToolSchema};
```

Replace the three trailing args in the `OpenAiClient::new` call (lines 281–283):

```rust
let prod_client = OpenAiClient::new(
    client_cfg.executor.api_key.clone().unwrap_or_default(),
    model.clone(),
    client_cfg.executor.base_url.clone(),
    std::time::Duration::from_secs(client_cfg.executor.first_token_timeout_secs),
    std::time::Duration::from_secs(client_cfg.executor.stream_idle_timeout_secs),
    SamplingParams {
        temperature: client_cfg.executor.temperature,
        seed: client_cfg.executor.seed,
        max_tokens: client_cfg.executor.max_tokens,
    },
);
```

### Task 6 — Update 9 `build_chat_body` test call sites in `openai.rs`

All are inside `#[cfg(test)] mod tests`. Each currently has three trailing args;
replace them with a single `SamplingParams` value:

| Old trailing args | New trailing arg |
|---|---|
| `None, None, 8192` | `SamplingParams::default()` |
| `Some(&[]), None, None, 8192` | `Some(&[]), SamplingParams::default()` |
| `Some(&tools), None, None, 8192` | `Some(&tools), SamplingParams::default()` |
| `None, Some(0.2), Some(42), 8192` | `None, SamplingParams { temperature: Some(0.2), seed: Some(42), max_tokens: 8192 }` |
| `None, Some(0.7), None, 8192` | `None, SamplingParams { temperature: Some(0.7), seed: None, max_tokens: 8192 }` |
| `None, None, None, 1234` | `None, SamplingParams { max_tokens: 1234, ..SamplingParams::default() }` |

Lines (verify before editing — use the line numbers as hints, not guarantees):
627, 634, 637, 648, 655, 662, 669, 676, 682.

Flag this in Notes-for-review as the same mechanical struct-literal churn pattern
as M23 phase-01's `ModelOverride` test literals and M22 phase-01's
`empty_completion_threshold`.

### Task 7 — Fix `format_no_match` in `feedback.rs`

Replace lines 44–46:

```rust
// before
let excerpt = if response_excerpt.len() > 200 {
    format!("{}...", &response_excerpt[..200])
} else {
    response_excerpt.to_string()
};

// after
let excerpt = if response_excerpt.chars().count() > 200 {
    format!("{}...", response_excerpt.chars().take(200).collect::<String>())
} else {
    response_excerpt.to_string()
};
```

Note: the `len()` guard becomes `chars().count()` to stay consistent — `len()` is
byte length and `chars().count()` is char length. The semantics shift slightly
(previously truncated at 200 bytes, now at 200 chars), but the intent was always
"at most ~200 characters of context."

### Task 8 — Tests

**`executor/src/ai/mod.rs`** (`#[cfg(test)] mod tests`):

- `sampling_params_default_max_tokens_is_8192` — `SamplingParams::default().max_tokens == 8192`
  (pins the non-zero default against a future derive-only change).

**`executor/src/parser/feedback.rs`** (`#[cfg(test)] mod tests`):

- `format_no_match_handles_multibyte_boundary` — fixture: `"a".repeat(199) + "é" + "bbb"`
  (199 ASCII bytes + 2-byte `é` + 3 bytes = 204 bytes; with the old `[..200]` this
  would panic mid-`é`). Assert the call does not panic and the result contains
  `"No tool call"`. This test would panic before the Task 7 fix and passes after.

## Acceptance criteria

- [ ] `SamplingParams` is declared in `executor/src/ai/mod.rs`; `SamplingParams::default().max_tokens == 8192`.
- [ ] `OpenAiClient::new` takes 6 args (no `sampling: SamplingParams` split); `#[allow(clippy::too_many_arguments)]` is gone from `openai.rs`.
- [ ] `build_chat_body` takes 5 args; `SamplingParams` fields wire correctly to the JSON body.
- [ ] `mcp/src/runner.rs` prod call site builds and passes `SamplingParams`.
- [ ] `grep -n '#\[allow(clippy::too_many_arguments)\]' executor/src/ai/backends/openai.rs` returns nothing.
- [ ] `format_no_match_handles_multibyte_boundary` passes (no panic on 199 ASCII + `é` + more).
- [ ] All pre-existing tests pass unmodified (9 `build_chat_body` test call sites updated mechanically — not a behavior change).
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings), `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all exit 0.

## Test plan

- `sampling_params_default_max_tokens_is_8192` — pins the manual `Default` impl.
- `format_no_match_handles_multibyte_boundary` — mutation-resistant: the old
  `[..200]` byte-slice panics on this input; the fix does not.
- Pre-existing `build_chat_body_includes_temperature_and_seed_when_set` and
  `build_chat_body_max_tokens_reflects_arg_not_default` pass with the new struct
  call shape — they already pin the wire behavior; only the call site changes.

## End-to-end verification

Not applicable — pure refactor, no new runtime-loadable artifact. The
`build_chat_body` wire-behavior tests (`build_chat_body_max_tokens_reflects_arg_not_default`,
`build_chat_body_includes_temperature_and_seed_when_set`) serve as the end-to-end
check that the sampling fields still land correctly in the JSON body.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `architecture.md` edit. Pure
refactor — `SamplingParams` consolidates three existing fields, `format_no_match`
replaces a byte-slice with a char-safe equivalent.

## Out of scope

- **Sharing `default_max_tokens()` between `config.rs` and `SamplingParams::default`**
  — `default_max_tokens()` is a private serde helper in `config.rs`; making it
  pub or moving it to a shared location is a separate design decision. The literal
  `8192` appears in both places with a comment in each; this is acceptable.
- **`GenerationParams` / telemetry** — still carries `temperature`/`seed` for the
  scorecard (see M23 phase-01 § Out of scope); `max_tokens` is not added here.
  `GenerationParams` is a separate struct with different lifecycle concerns.
- **Adding further knobs to `SamplingParams`** — `top_p`, `top_k`, etc. are not in
  scope; add them when a phase specifically requires them.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-18 15:55 (started)

**Executor:** Claude executor

Implementing `SamplingParams` refactor and `format_no_match` byte-slice fix.

### Update — 2026-06-18 15:56 (complete)

**Summary:** Introduced `SamplingParams { temperature, seed, max_tokens }` in `executor/src/ai/mod.rs`, collapsed the three trailing args on `build_chat_body`, `OpenAiClient::new`, and both call sites (`make_client` and `mcp/src/runner.rs`) into a single `sampling: SamplingParams` arg. Removed the authorized `#[allow(clippy::too_many_arguments)]`. Fixed `format_no_match` in `feedback.rs` to use char-safe truncation instead of a byte-slice that panics on multi-byte boundaries.

**Acceptance criteria:** all ticked.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.0
   Compiling rexymcp-mcp v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in Xs

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.0
    Checking rexymcp-mcp v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in Xs

cargo test 2>&1 | tail -30
test result: ok. 857 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — pure refactor, no new runtime-loadable artifact. The `build_chat_body` wire-behavior tests (`build_chat_body_max_tokens_reflects_arg_not_default`, `build_chat_body_includes_temperature_and_seed_when_set`) serve as the end-to-end check that the sampling fields still land correctly in the JSON body.

**Files changed:**
- `executor/src/ai/mod.rs` — Added `SamplingParams` struct + `Default` impl, updated `make_client` to pass `SamplingParams`, added test
- `executor/src/ai/backends/openai.rs` — Updated imports, `build_chat_body` signature/body, `OpenAiClient` struct/`new`/`chat`, 9 test call sites
- `mcp/src/runner.rs` — Added `SamplingParams` to import, updated `OpenAiClient::new` call site
- `executor/src/parser/feedback.rs` — Fixed `format_no_match` byte-slice to char-safe truncation, added test

**New tests:**
- `sampling_params_default_max_tokens_is_8192` in `executor/src/ai/mod.rs`
- `format_no_match_handles_multibyte_boundary` in `executor/src/parser/feedback.rs`

**Commits:**
- (to be committed)

**Notes for review:** The 9 `build_chat_body` test call site updates are the same mechanical struct-literal churn pattern as M23 phase-01's `ModelOverride` test literals and M22 phase-01's `empty_completion_threshold`. The `#[allow(clippy::too_many_arguments)]` grep returns nothing (exit 1 = no matches), confirming it was fully removed.

### Review verdict — 2026-06-18

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude Code (direct)
- **Scope deviations:** none — both flagged retrospective items (the `too_many_arguments` allow and the `format_no_match` byte-slice) retired exactly as drafted; no `architecture.md`/`Cargo.toml` edit, no new dependency.
- **Calibration:** none. All four gates green on independent re-run (857 executor + 431 mcp pass, 2 ignored). `#[allow(clippy::too_many_arguments)]` grep-confirmed gone from `openai.rs`; the two new tests are mutation-resistant (`format_no_match_handles_multibyte_boundary` panics under the old `[..200]` byte-slice; `sampling_params_default_max_tokens_is_8192` pins the manual `Default` against a derive regression). Closes the 1st-occurrence `too_many_arguments` and the 2nd-deferral `format_no_match` panic, both flagged in the M23 retrospective — no remaining calibration debt for this milestone.
