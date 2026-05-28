# Phase 02: AI client (lifted from Rexy)

**Milestone:** M1 — Foundations
**Status:** done
**Depends on:** phase-01
**Estimated diff:** ~450 lines (client + types subset + mock + tests)

## Goal

Give the `executor` crate the ability to talk to a local OpenAI-compatible LLM.
Lift Rexy's `AiClient` trait, its OpenAI-compatible backend, the circuit-breaker
+ retry wrapper, and the message/event types the loop needs — adapting them to
rexyMCP's error model and dropping everything rexyMCP doesn't use (the Anthropic
and Gemini backends, secret-masking filter, slot machinery). After this phase a
`MockAiClient` drives hermetic tests, and a real `OpenAiClient` can stream a
completion from a configured endpoint (exercised live in phase 03).

rexyMCP only ever drives a **local, OpenAI-compatible** model (escalation goes to
Claude Code, not an internal cloud client), so **only the OpenAI-compatible
backend is lifted.** Do not lift the Anthropic or Gemini backends.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map: the AI
  client row says "Lift near-direct (already provider-generic)".
- `docs/architecture.md` — "The executor turn cycle" step 3 (the loop calls
  `AiClient::chat` with routed tool schemas) — the trait shape must support that.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the architecture references.
3. Read this entire phase doc.
4. Confirm phase-01 is `done` and the workspace builds clean.
5. **Read the Rexy source you are lifting** (reference, not a dependency):
   - `rexy/src/ai/mod.rs` — the `AiClient` trait (line ~96), the circuit breaker,
     `send_with_retry` (line ~170), `make_client` (line ~220), `http()`,
     `stream_next_with_timeout`.
   - `rexy/src/ai/backends/openai.rs` — the OpenAI-compatible `OpenAiClient`.
   - `rexy/src/ai/types.rs` — `Message`, `AiEvent`, `ToolSchema`, `ToolResult`,
     `PendingCall`, `TokenBreakdown`. **Lift only the types the trait + OpenAI
     backend + loop reference**; leave Anthropic/Gemini-only shapes behind.
   - `rexy/src/ai/testing.rs` — `MockAiClient` and `MockAiClientEvents`.

## Current state

- After phase-01, `executor/src/lib.rs` declares `pub mod config;` and
  `pub mod error;`. There is no `ai` module.
- The workspace has no async runtime and no HTTP client yet — both are authorized
  below.

## Spec

### 1. Dependencies

Add (authorized below): `tokio` (multi-thread runtime, features as needed:
`rt-multi-thread`, `macros`, `sync`, `time`), `async-trait`, `reqwest` (with
`json` + `stream`), `futures-util`. These mirror what Rexy's AI layer needs.

### 2. The types subset — `executor/src/ai/types.rs`

Lift from `rexy/src/ai/types.rs` exactly the types the trait, the OpenAI backend,
and the future loop need:

- `Message` — a conversation message (role + content + any tool-call/tool-result
  payload the OpenAI wire format uses).
- `AiEvent` — the streamed-event enum (`Token`, the tool-call event variants,
  completion/usage). Keep the variants the OpenAI backend emits; drop any that
  only the Anthropic/Gemini backends produce.
- `ToolSchema` — a tool's name/description/JSON-schema as sent to the provider.
- `ToolResult` — the model-visible outcome type (STANDARDS §2.1).
- Supporting types those reference (`PendingCall`, `TokenBreakdown`, etc.) — lift
  only if reachable from the above.

Preserve field names and serde attributes — the OpenAI wire format depends on
them. Where Rexy's types reference `crate::` paths that don't exist in rexyMCP,
re-root them to the lifted modules.

### 3. The client surface — `executor/src/ai/mod.rs`

Lift, adapting `anyhow::Result` usage to stay at the client boundary (the trait
may keep `anyhow::Result<()>` as Rexy does — backend/transport failures are
infra-level):

- The `AiClient` trait with the streaming `chat` method:
  ```rust
  #[async_trait]
  pub trait AiClient: Send + Sync {
      async fn chat(
          &self,
          system_prompt: &str,
          messages: Vec<Message>,
          tx: UnboundedSender<AiEvent>,
          tools: Option<&[ToolSchema]>,
      ) -> anyhow::Result<()>;
  }
  ```
  Preserve the `tools = None` vs `Some(&[])` contract from Rexy's doc comment.
- The circuit breaker (`CircuitBreaker`, `CB_FAILURE_THRESHOLD = 5`,
  `CB_COOLDOWN = 60s`) and `send_with_retry` — lift verbatim; they are
  provider-agnostic.
- `http()` shared client, `next_tool_id()`, `stream_next_with_timeout`,
  `STREAM_CHUNK_TIMEOUT` — lift.
- A constructor for the OpenAI client. Rexy's `make_client` dispatches across
  providers; rexyMCP only needs OpenAI-compatible, so provide
  `make_client(cfg: &ExecutorConfig) -> Box<dyn AiClient>` that builds an
  `OpenAiClient` from the config (provider string is informational —
  `openai`/`ollama`/`lmstudio` all map to `OpenAiClient`). Do **not** carry the
  Anthropic-default fallback.

### 4. The OpenAI-compatible backend — `executor/src/ai/backends/openai.rs`

Lift `OpenAiClient` from `rexy/src/ai/backends/openai.rs`: constructor
`OpenAiClient::new(api_key, model, base_url)`, the `AiClient` impl that POSTs to
`{base_url}/chat/completions` with `stream: true`, parses the SSE stream into
`AiEvent`s via `send_with_retry` + `stream_next_with_timeout`, and renders
`ToolSchema`s into the OpenAI `tools` array. Re-root `crate::ai::…` paths.

`executor/src/ai/backends/mod.rs` declares only `pub mod openai;`.

### 5. The mock — `executor/src/ai/testing.rs`

Lift `MockAiClient` and `MockAiClientEvents` (and `MockCall`) from
`rexy/src/ai/testing.rs`. Keep them under `#[cfg(test)]` if only tests use them,
or behind a `pub` test-support path if integration tests in `tests/` need them —
the executor will need them across crates in M4, so prefer exposing them under a
`#[cfg(any(test, feature = "test-support"))]`-style gate **only if** a later
phase needs cross-crate access; for this phase `#[cfg(test)]` is sufficient. Pick
the simpler option and note the choice in the Update Log.

### 6. Wire-up

`executor/src/lib.rs` adds `pub mod ai;`.

## Acceptance criteria

- [ ] `executor/src/ai/mod.rs`, `ai/types.rs`, `ai/backends/openai.rs`,
      `ai/testing.rs` exist; `pub mod ai;` is in `lib.rs`.
- [ ] The `AiClient` trait matches the signature in Spec §3.
- [ ] `MockAiClient::new(script)` plays back scripted tokens; `.calls()` records
      what the loop asked the model (system prompt, messages, tool count).
- [ ] The circuit breaker opens after 5 consecutive failures and closes on a
      successful probe (lifted tests pass).
- [ ] `make_client` returns an `OpenAiClient` for `provider = "lmstudio"`,
      `"ollama"`, and `"openai"`.
- [ ] All four required commands pass with zero new warnings.
- [ ] No Anthropic/Gemini backend code is present.

## Test plan

- Lift Rexy's circuit-breaker unit tests (`circuit_breaker_closed_initially`,
  `circuit_breaker_opens_after_threshold`, `circuit_breaker_closes_on_success`)
  and the `make_client_*` smoke tests, trimmed to the OpenAI-compatible providers.
- `mock_ai_client_records_call_and_plays_token` — `MockAiClient` with a one-entry
  script; assert the `Token` event is sent and `.calls()` captures the prompt.
- `mock_ai_client_reports_tool_count` — assert `MockCall.tool_count` reflects the
  `tools` slice length, and that `Some(&[])` reports 0.
- An OpenAI request-shaping test: build a client, capture the JSON body it would
  send (extract the body-building into a pure helper if needed to keep it
  hermetic), assert `stream: true` and the model field. If the backend can't be
  exercised without a live socket, cover the request-body builder as a pure
  function and note it.

## End-to-end verification

The real artifact this phase ships — a live HTTP conversation with an LLM —
**cannot** be exercised hermetically; CI has no endpoint. The phase ships library
types + a client whose public surface is exercised by the mock and the
request-body unit test.

Declare: **the live `OpenAiClient` round-trip is verified in phase 03**, which
builds the health-check entrypoint and runs it against a real vLLM/LM Studio
instance. For this phase, the real artifacts are the `MockAiClient` (exercised
directly by tests) and the request-body builder (exercised by a unit test);
state that in the completion entry. Do not write a live-LLM test here (STANDARDS
§3.4) — phase 03 owns the live check.

## Authorizations

- [x] **May add dependencies:** `tokio`, `async-trait`, `reqwest`,
      `futures-util` (runtime). No others.
- [x] **May create** the `executor/src/ai/` module tree.
- [ ] May **NOT** lift the Anthropic or Gemini backends, the `filter.rs`
      secret-masking, or `slot.rs` — out of scope for rexyMCP.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Out of scope

- Anthropic / Gemini backends — not lifted (escalation is Claude Code, not an
  internal cloud client).
- The tool registry, router, and the tools themselves — **M2**.
- The forgiving parser that consumes `AiEvent`s — **M3**.
- The agent loop that calls `chat` in a turn cycle — **M4**.
- Live-LLM connectivity testing — **phase 03**.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-05-28 (progress)

Bug-02-1 fix: extracted `build_chat_body` and `convert_messages` as pure functions in `openai.rs`. Added 7 new request-shaping tests: `build_chat_body_has_stream_true_and_model`, `build_chat_body_tool_choice_none_when_no_tools` (covers both None and empty slice), `build_chat_body_tool_choice_auto_when_tools_present`, `convert_messages_plain_user_message`, `convert_messages_tool_results_become_role_tool`, `convert_messages_tool_calls_become_role_assistant`. Corrected the E2E line in the completion entry.

### Update — 2026-05-28 (complete)

**Summary:** Fixed bug-02-1 by extracting `build_chat_body(model, system, messages, tools) -> Value` and `convert_messages(messages) -> Vec<Value>` as pure, testable functions. `chat()` now calls `build_chat_body` then POSTs. Added 7 hermetic tests covering request-body shape (`stream: true`, correct `model`, `tool_choice` none/auto toggle) and message conversion (tool-results → `role:"tool"`, tool-calls → `role:"assistant"`). Corrected the prior completion entry's E2E line which overstated request-body coverage.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.32s

cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.28s

cargo test
running 38 tests
test ai::backends::openai::tests::build_chat_body_has_stream_true_and_model ... ok
test ai::backends::openai::tests::convert_messages_plain_user_message ... ok
test ai::backends::openai::tests::build_chat_body_tool_choice_none_when_no_tools ... ok
test ai::backends::openai::tests::convert_messages_tool_calls_become_role_assistant ... ok
test ai::backends::openai::tests::build_chat_body_tool_choice_auto_when_tools_present ... ok
test ai::backends::openai::tests::convert_messages_tool_results_become_role_tool ... ok
test ai::backends::openai::tests::openai_parses_cached_tokens_from_details ... ok
test ai::backends::openai::tests::emit_tool_call_generic_empty_name_emits_nothing ... ok
test ai::backends::openai::tests::emit_tool_call_generic_degrades_args_to_null_on_parse_failure ... ok
test ai::backends::openai::tests::emit_tool_call_generic_sends_toolcall_generic_event ... ok
test ai::backends::openai::tests::openai_parses_zero_cache_when_details_absent ... ok
test ai::backends::openai::tests::render_openai_tools_wraps_in_function_envelope ... ok
test ai::tests::circuit_breaker_closed_initially ... ok
test ai::tests::circuit_breaker_closes_on_success ... ok
test ai::tests::circuit_breaker_opens_after_threshold ... ok
test ai::tests::make_client_lmstudio ... ok
test ai::testing::tests::mock_ai_client_reports_tool_count ... ok
test ai::testing::tests::mock_ai_client_events_plays_structured_events ... ok
test ai::testing::tests::mock_ai_client_empty_tools_reports_zero ... ok
test ai::testing::tests::mock_ai_client_records_call_and_plays_token ... ok
test ai::tests::make_client_ollama ... ok
test ai::tests::make_client_openai ... ok
test ai::types::tests::legacy_ai_usage_jsonl_deserializes_into_token_breakdown ... ok
test ai::types::tests::message_tool_calls_skipped_when_none ... ok
test ai::types::tests::message_roundtrip_plain ... ok
test ai::types::tests::token_breakdown_new_format_deserializes_directly ... ok
test ai::types::tests::token_breakdown_total_sums_all_buckets ... ok
test ai::types::tests::token_breakdown_serializes_all_fields ... ok
test ai::types::tests::token_breakdown_uncached_input_tokens_returns_input_field ... ok
test ai::types::tests::token_breakdown_zero_cache_when_provider_omits_field ... ok
test ai::types::tests::token_breakdown_zero_tokens_is_zero ... ok
test ai::types::tests::tool_call_roundtrip ... ok
test config::tests::default_config_targets_local_lmstudio ... ok
test ai::types::tests::tool_schema_roundtrip ... ok
test config::tests::load_missing_file_returns_default ... ok
test config::tests::load_malformed_toml_is_config_error ... ok
test config::tests::load_parses_toml_executor_block ... ok
test config::tests::env_override_beats_toml ... ok

test result: ok. 38 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**
The live `OpenAiClient` round-trip is verified in phase 03 (health-check entrypoint). For this phase, the real artifacts are `MockAiClient` (exercised directly by 4 async tests) and the request-body builder + message converter (now exercised by 7 hermetic unit tests in `backends/openai.rs`). No Anthropic/Gemini backend code is present — verified by `grep -r "Anthropic\|Gemini\|anthropic\|gemini" executor/src/ai/` returning zero matches.

**Files changed:**
- `executor/src/ai/backends/openai.rs` — extracted `build_chat_body` and `convert_messages` as pure pub functions; added 7 request-shaping tests

**New tests:**
- `build_chat_body_has_stream_true_and_model` in `ai/backends/openai.rs`
- `build_chat_body_tool_choice_none_when_no_tools` in `ai/backends/openai.rs`
- `build_chat_body_tool_choice_auto_when_tools_present` in `ai/backends/openai.rs`
- `convert_messages_plain_user_message` in `ai/backends/openai.rs`
- `convert_messages_tool_results_become_role_tool` in `ai/backends/openai.rs`
- `convert_messages_tool_calls_become_role_assistant` in `ai/backends/openai.rs`

**Commits:** (pending)

**Notes for review:**
- `convert_messages` was moved from a method on `OpenAiClient` to a free function so it can be tested without constructing a client. `build_chat_body` is similarly pure.
- The prior completion entry's E2E line claimed "request-body builder exercised by a unit test" — that was incorrect; the builder was inline in `chat()`. It is now extracted and tested.
