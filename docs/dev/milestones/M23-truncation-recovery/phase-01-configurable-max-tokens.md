# Phase 01: Configurable `max_tokens`

**Milestone:** M23 — Truncation & Empty-Completion Recovery
**Status:** review
**Depends on:** none in M23 (rides the M18 `[models]` override + `resolve_for_model` path)
**Estimated diff:** ~150 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the per-response output ceiling (`max_tokens`) a configuration knob instead
of a hardcoded constant. Add `[executor] max_tokens` to the config (default
**8192**, up from the hardcoded 4096), make it per-model overridable in
`[models."<id>"]`, and thread it through the OpenAI-compatible backend so the
value actually lands on the wire. This is the substrate fix for the netviz
truncation: 4096 cut the executor's reasoning off mid-`<think>` before it reached
a tool call.

`max_tokens` mirrors the existing `temperature`/`seed` knobs **exactly** — same
config field shape, same `ModelOverride` mirror, same `resolve_for_model` apply
line, same constructor-arg threading into `OpenAiClient`. Follow those three at
every site.

## Architecture references

Read before starting:

- `executor/src/config.rs` — `ExecutorConfig` (245–306), `ModelOverride`
  (199–208), `resolve_for_model` (431–459). `temperature`/`seed` are the pattern
  to copy.
- `executor/src/ai/backends/openai.rs` — `build_chat_body` (83–130, the hardcoded
  `"max_tokens": 4096` at line 110), `OpenAiClient` struct (132–140), `new`
  (142–166), the `build_chat_body` call in `chat` (~179–186).
- `executor/src/ai/mod.rs` — `make_client` (187–197), the second
  `OpenAiClient::new` call site.
- `mcp/src/runner.rs` — production `OpenAiClient::new` (275–283) and the two
  `ModelOverride` test literals (592–601, 710 region).
- `mcp/src/init.rs` — the `rexymcp.toml` template (the `# temperature`/`# seed`
  comment lines at 21 and 41–42 are where the `max_tokens` comments go).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### `build_chat_body` hardcodes the ceiling — `openai.rs:108–114`

```rust
let mut body = json!({
    "model": model,
    "max_tokens": 4096,
    "stream": true,
    "stream_options": { "include_usage": true },
    "messages": full_messages,
});
```

`temperature` and `seed` arrive as `Option` params (88–89) and are applied
conditionally (123–128). `max_tokens` is **not** a param — it is the literal
`4096`. This phase makes it the last param of `build_chat_body` and replaces the
literal.

### `OpenAiClient` carries sampling knobs as fields — `openai.rs:132–166`

The struct stores `temperature`/`seed` (138–139), `new` takes them as the last
two args (149–150), and `chat` passes `self.temperature`/`self.seed` into
`build_chat_body` (~184–185). `max_tokens` follows the same path: a new struct
field, a new `new` arg, passed into `build_chat_body`.

### Config — `executor/src/config.rs`

`ExecutorConfig` (245–306): `temperature`/`seed` are `#[serde(default)]
Option<…>` (260–266) and set to `None` in `Default` (300–301). `max_tokens` is a
concrete `u32` with a non-`None` default, so it uses the **`first_token_timeout_secs`
shape** instead: a `#[serde(default = "default_max_tokens")]` attribute + a
`default_max_tokens()` fn + a `default_max_tokens()` call in the `Default` impl
(see `default_first_token_timeout_secs` at 279–281 / 298).

`ModelOverride` (199–208) lists every overridable knob as `Option<…>`;
`resolve_for_model` (431–459) applies each present override. Add `max_tokens`
to both, following `temperature` (438–440) exactly.

## Spec

### Task 1 — `ExecutorConfig.max_tokens` + default (`config.rs`)

Add the field to `ExecutorConfig` (after `seed`, before `task_tracking`):

```rust
/// Per-response output-token ceiling (`max_tokens`) sent on every chat
/// request. Carved out of the remaining context window — `prompt + max_tokens`
/// must fit in the model's context length. The prior hardcoded 4096 truncated
/// thinking models mid-reasoning before they reached a tool call; 8192 leaves
/// headroom for a full reasoning block + tool call while keeping a runaway turn
/// bounded.
#[serde(default = "default_max_tokens")]
pub max_tokens: u32,
```

Add the default fn (next to `default_first_token_timeout_secs`):

```rust
fn default_max_tokens() -> u32 {
    8192
}
```

Add `max_tokens: default_max_tokens(),` to the `Default for ExecutorConfig` impl
(alongside `temperature: None,` etc.).

### Task 2 — `ModelOverride.max_tokens` + resolve (`config.rs`)

Add `pub max_tokens: Option<u32>,` to `ModelOverride` (after `seed`, matching the
sampling-knob grouping). Add the apply line to `resolve_for_model`, mirroring
`temperature`:

```rust
if let Some(v) = over.max_tokens {
    self.executor.max_tokens = v;
}
```

### Task 3 — Thread `max_tokens` through the backend (`openai.rs`)

1. Add `max_tokens: u32,` as the **last** param of `build_chat_body` (after
   `seed`).
2. Replace `"max_tokens": 4096,` with `"max_tokens": max_tokens,`.
3. Add `max_tokens: u32,` as a field on `OpenAiClient` (after `seed`).
4. Add `max_tokens: u32,` as the **last** param of `OpenAiClient::new` (after
   `seed`) and set the field in the constructor body.
5. In `chat`, pass `self.max_tokens` as the new last arg to `build_chat_body`.

### Task 4 — Update the two `OpenAiClient::new` call sites

- `executor/src/ai/mod.rs` `make_client` (~188): add
  `cfg.max_tokens,` as the new last arg (after `cfg.seed,`).
- `mcp/src/runner.rs` `prod_client` (~282): add
  `client_cfg.executor.max_tokens,` as the new last arg (after
  `client_cfg.executor.seed,`).

### Task 5 — Name the new field in the two `ModelOverride` test literals (`runner.rs`)

Both literals (592–601 and the sibling near 710) name **every** field. Add
`max_tokens: None,` to each (alongside `seed: None,`). This is the same
mechanical struct-literal consequence M22 phase-01 documented for
`empty_completion_threshold` — `ModelOverride` derives `Default` + carries
struct-level `#[serde(default)]` (so TOML parsing defaults the field), but a Rust
struct **literal** must name every field. Flag it in Notes-for-review as
mechanical, not a design change.

### Task 6 — Document the knob in the `rexymcp init` template (`init.rs`)

Add a commented line under `[executor]` (next to `# temperature`):

```
# max_tokens = 8192              # per-response output token ceiling (default 8192)
```

And in the `[models."<model-id>"]` override block (next to `# seed`):

```
# max_tokens = 8192                 # override [executor] max_tokens
```

If `init.rs` has an assertion test that pins the rendered template's line count or
specific substrings, update it to include the new lines.

### Task 7 — Update existing `build_chat_body` test call sites (`openai.rs`)

The new trailing arg breaks the 7 existing `build_chat_body` calls in
`#[cfg(test)] mod tests` (lines 621, 628, 631, 642, 649, 656, 663). Add a trailing
`8192` to each so they compile, then add the two new tests below.

### Task 8 — New tests

**`openai.rs`** (`#[cfg(test)] mod tests`):

- `build_chat_body_uses_configured_max_tokens` — `build_chat_body("m", "sys",
  vec![], None, None, None, 8192)` ⇒ `body["max_tokens"] == 8192`.
- `build_chat_body_max_tokens_reflects_arg_not_default` — call with `1234` and
  assert `body["max_tokens"] == 1234` (refutes a re-hardcoded literal).

**`config.rs`** (`#[cfg(test)] mod tests`):

- `loads_default_max_tokens_when_absent` — a `[executor]` block with no
  `max_tokens` ⇒ `cfg.executor.max_tokens == 8192`.
- `loads_max_tokens_from_toml` — `max_tokens = 2048` in `[executor]` ⇒
  `cfg.executor.max_tokens == 2048`.
- `resolve_for_model_applies_max_tokens_override` — global `max_tokens = 8192`, a
  `[models."m"] max_tokens = 2048`, then `resolve_for_model("m")` ⇒
  `cfg.executor.max_tokens == 2048`. Model the test on
  `resolve_for_model_applies_matching_override` (config.rs:1069).
- `resolve_for_model_leaves_max_tokens_when_override_absent` — a `[models."m"]`
  entry that overrides only `temperature` leaves `max_tokens` at the global value
  (mirror `resolve_for_model_leaves_unset_fields_global` at 1114).

## Acceptance criteria

- [ ] `[executor] max_tokens` loads from `rexymcp.toml`; absent ⇒ 8192.
- [ ] `[models."<id>"] max_tokens` overrides the global via `resolve_for_model`;
      absent ⇒ inherits the global.
- [ ] `build_chat_body` emits the configured `max_tokens` (no hardcoded 4096
      remains — `grep -n '4096' executor/src/ai/backends/openai.rs` returns
      nothing in production code).
- [ ] Both `OpenAiClient::new` call sites pass the configured value.
- [ ] The two new tests in `openai.rs` and the four in `config.rs` pass.
- [ ] All pre-existing tests pass (except the mechanical call-site edits in Tasks
      5 and 7).
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings), `cargo clippy
      --all-targets --all-features -- -D warnings`, `cargo test` all exit 0.

## Test plan

- Backend: two `build_chat_body` unit tests pinning the emitted `max_tokens` to the
  arg.
- Config: four unit tests — default, toml load, model-override resolve, and
  inherit-when-absent.

## End-to-end verification

This phase ships a runtime-loadable artifact (a `rexymcp.toml` knob the running
binary reads and sends on the wire), so per STANDARDS § DoD it must be verified
against the real artifact, not just a fake. Verify the two halves and quote the
output in the Update Log:

1. **Config-load half:** write a real `rexymcp.toml` on disk with a non-default
   `[executor] max_tokens = 2048`, load it through the real `Config` loader, and
   show `cfg.executor.max_tokens == 2048`. (A `loads_max_tokens_from_toml`-style
   assertion driven from an on-disk `TempDir` file satisfies this; quote the value
   read back.)
2. **Wire-serialization half:** show `build_chat_body(…, 2048)` puts
   `"max_tokens": 2048` in the request body (quote the serialized fragment).

Together these prove the value travels config-file → `Config` → request body. The
live network send itself is unchanged streaming infrastructure (the same path that
already carries `temperature`/`seed`) and is not separately exercised here.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `architecture.md` edit (the
§ Configuration bullet + § Status #23 entry are added by the architect at kickoff,
not the executor). The new config field is additive and defaulted.

## Out of scope

- **Runtime validation/clamping** of `prompt + max_tokens` against the model's
  context length — the endpoint enforces its own limit; a clamp is a separate
  concern (noted as a possible follow-up in the README).
- **Recording `max_tokens` in `GenerationParams`/telemetry** — that struct carries
  `temperature`/`seed` for the scorecard; adding `max_tokens` is a telemetry
  schema change, not needed for the wire fix. Leave `GenerationParams`
  (`store/telemetry.rs:19`) untouched.
- **The finish_reason recovery / no-think escalation** — that is phase-02.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-18 13:17 (started)

**Executor:** rexyMCP executor
**Summary:** Implementing configurable `max_tokens` across config, backend, call sites, init template, and tests.

### Update — 2026-06-18 13:25 (complete)

**Executor:** rexyMCP executor

**Verification commands:**
- `cargo fmt --all --check` — exit 0, clean
- `cargo build` — exit 0, zero new warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0, clean
- `cargo test` — 850 passed, 0 failed, 2 ignored
- `cargo test max_tokens` — 6 passed (2 openai.rs + 4 config.rs)
- `grep -n '4096' executor/src/ai/backends/openai.rs` — exit 1 (no matches), confirming no hardcoded 4096 remains

**End-to-end verification:**
- **Config-load half:** `loads_max_tokens_from_toml` writes a real `rexymcp.toml` on disk with `max_tokens = 2048` in `[executor]`, loads it through `Config::load`, and asserts `cfg.executor.max_tokens == 2048`.
- **Wire-serialization half:** `build_chat_body_uses_configured_max_tokens` calls `build_chat_body(…, 8192)` and asserts `body["max_tokens"] == 8192`; `build_chat_body_max_tokens_reflects_arg_not_default` calls with `1234` and asserts `body["max_tokens"] == 1234`, refuting a re-hardcoded literal.

**Files changed:**
- `executor/src/config.rs` — added `max_tokens: u32` to `ExecutorConfig`, `default_max_tokens()`, `Default` impl, `ModelOverride.max_tokens: Option<u32>`, `resolve_for_model` apply line, 4 new tests
- `executor/src/ai/backends/openai.rs` — added `max_tokens` param to `build_chat_body`, replaced hardcoded `4096`, added `max_tokens` field to `OpenAiClient`, added `max_tokens` arg to `new`, threaded through `chat`, `#[allow(clippy::too_many_arguments)]` on `new`, updated 7 existing test call sites, 2 new tests
- `executor/src/ai/mod.rs` — added `cfg.max_tokens` to `make_client` call, 3 test struct literals updated
- `executor/src/health.rs` — 1 test struct literal updated
- `mcp/src/runner.rs` — added `client_cfg.executor.max_tokens` to `prod_client` call, 2 `ModelOverride` test literals updated with `max_tokens: None`
- `mcp/src/init.rs` — added `# max_tokens = 8192` comment lines in `[executor]` and `[models."<model-id>"]` blocks
- `docs/dev/milestones/M23-truncation-recovery/phase-01-configurable-max-tokens.md` — status flip + Update Log
- `docs/dev/milestones/M23-truncation-recovery/README.md` — phase table status flip

**Commit:** `feat: make max_tokens configurable via rexymcp.toml`

**Notes for review:**
- Added `#[allow(clippy::too_many_arguments)]` on `OpenAiClient::new` — the function now has 8 args (7 → 8). This is the same pattern the existing code already had at 7 args (the clippy threshold is 7), and the new arg follows the exact same constructor-arg threading pattern as `temperature`/`seed`. A struct builder refactor would be out of scope for this phase.
- Task 5 (`ModelOverride` test literals in `runner.rs`) is a mechanical struct-literal consequence — same pattern documented in M22 phase-01 for `empty_completion_threshold`.
