# Phase 03: Health-check entrypoint

**Milestone:** M1 — Foundations
**Status:** todo
**Depends on:** phase-02
**Estimated diff:** ~250 lines

## Goal

Ship the first **runnable real artifact**: a `rexymcp health` command that reads
rexyMCP config, connects to the configured OpenAI-compatible endpoint, and prints
the list of models it serves (or a clear error if it's unreachable). This is the
core of the `executor_health` MCP tool (M5) and the first end-to-end proof that
rexyMCP can reach a local LLM. After this phase you can point rexyMCP at a real
vLLM / LM Studio / Ollama instance and see its models.

## Architecture references

- `docs/architecture.md` — "Layer 2 — `mcp` crate": `executor_health` "pings the
  configured endpoint and lists available models." This phase builds that logic
  in the `executor` library so M5's MCP tool is a thin wrapper.
- `docs/architecture.md` — "Configuration": the endpoint + model come from
  `Config.executor`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the architecture references.
3. Read this entire phase doc.
4. Confirm phase-02 is `done`; `executor::ai` and `executor::config` exist and
   the workspace builds clean.
5. Have a local OpenAI-compatible endpoint available for the end-to-end check
   (LM Studio with a model loaded, or `ollama serve`, or a vLLM server). Note
   which one and its base URL in the Update Log.

## Current state

- `mcp/src/main.rs` is a stub that prints name + version (phase-01).
- `executor` exposes `config` (with `ExecutorConfig { provider, model, base_url,
  api_key }`), `error`, and `ai` (with `http()` and the OpenAI client) after
  phase-02.

## Spec

### 1. Health logic in the library — `executor/src/health.rs`

The OpenAI-compatible model-list endpoint is `GET {base_url}/models` returning
`{ "object": "list", "data": [ { "id": "<model>", … }, … ] }`.

Define:

```rust
use crate::config::ExecutorConfig;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    pub reachable: bool,
    pub base_url: String,
    pub models: Vec<String>,
}

/// GET {base_url}/models, parse the `data[].id` list.
/// Reuses the shared `ai::http()` client. A transport failure or non-2xx
/// returns Err(Error::Backend(...)); a 2xx with an unparseable body also
/// returns Err(Error::Backend(...)). Sets the Authorization header from
/// `api_key` when present.
pub async fn list_models(cfg: &ExecutorConfig) -> Result<Vec<String>>;

/// Convenience wrapper: never errors — on failure returns
/// Health { reachable: false, models: [], .. } so callers can render a
/// status without try/catch. Used by the binary and (M5) the MCP tool.
pub async fn check(cfg: &ExecutorConfig) -> Health;
```

`list_models` builds the URL by joining `base_url` + `/models` (handle a
trailing slash on `base_url` so `…/v1` and `…/v1/` both work). Add `pub mod
health;` to `executor/src/lib.rs`.

### 2. The `health` subcommand — `mcp/src/main.rs`

Turn the stub binary into a small CLI (use `clap`, authorized below):

- `rexymcp health [--config <path>] [--base-url <url>]` — load config
  (`Config::load_with_env`), apply the optional `--base-url` override, call
  `executor::health::check`, and print:
  - on success: the base URL and one model id per line (or "(no models
    reported)" if the list is empty but the endpoint answered);
  - on failure: a single clear line naming the base URL and that it's
    unreachable, and exit non-zero.
- With no subcommand, keep the phase-01 behavior (print name + version, exit 0).

Use `#[tokio::main]`. Keep `main` thin — it parses args, loads config, calls the
library, formats output. No business logic in `main`.

### 3. Output contract

`rexymcp health` stdout on success starts with a line containing the base URL,
then the models. Exact format is the executor's call, but the base URL and each
model id must each appear on their own line so the output is greppable. On
failure, exit code is non-zero and stderr/stdout names the unreachable URL.

## Acceptance criteria

- [ ] `executor/src/health.rs` exists with `list_models` and `check` per Spec §1;
      `pub mod health;` is in `lib.rs`.
- [ ] `list_models` parses `data[].id` from an OpenAI-compatible `/models`
      response (covered by a unit test against a fixture JSON).
- [ ] `list_models` returns `Error::Backend` for a non-2xx or unparseable body.
- [ ] `check` returns `reachable: false` (never errors) when the endpoint can't
      be reached.
- [ ] `rexymcp health --base-url <unreachable>` exits non-zero and names the URL.
- [ ] `rexymcp` with no subcommand still prints name + version and exits 0.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic unit tests in `executor/src/health.rs`:

- `parses_model_ids_from_openai_list_response` — feed a fixture
  `{"object":"list","data":[{"id":"qwen2.5-coder"},{"id":"gemma2"}]}` to the
  body-parsing helper; assert `["qwen2.5-coder","gemma2"]`. (Extract the parse
  step into a pure helper so it's testable without a socket.)
- `parse_rejects_non_list_body` — a body missing `data` → `Error::Backend`.
- `joins_base_url_and_models_path` — assert URL building handles both `…/v1` and
  `…/v1/` (pure helper).
- `check_returns_unreachable_on_connection_error` — point at a closed port
  (e.g. `http://127.0.0.1:1`); assert `reachable == false` and no panic. (This
  hits the loopback only and fails fast — acceptable as a hermetic test; mark
  `#[ignore]` only if it proves flaky in CI and explain why.)

Do **not** write a test that requires a real model server — that's the manual
end-to-end check below.

## End-to-end verification

This phase's whole point is the live connection. Against a real local endpoint:

- AC (live): start a local OpenAI-compatible server (note which: LM Studio /
  Ollama / vLLM) with at least one model, then run
  `cargo run -p rexymcp -- health --base-url <that url>` and **paste the actual
  stdout** showing the served model ids in the completion Update Log.
- AC (failure path): run `cargo run -p rexymcp -- health --base-url
  http://127.0.0.1:1` and paste the output + non-zero exit, showing the
  unreachable URL is named.

## Authorizations

- [x] **May add dependencies:** `clap` (with `derive`). `tokio`/`reqwest` are
      already present from phase-02.
- [x] **May create** `executor/src/health.rs` and rewrite `mcp/src/main.rs` into
      the `health` CLI.
- [ ] May **NOT** add the MCP server / `rmcp` — that's M5. This is a plain CLI
      that M5 will wrap.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Out of scope

- The `execute_phase` tool and the MCP server itself — **M5**. This phase only
  builds the `executor_health` *logic* + a CLI front-end.
- Any executor tooling, parser, or agent loop — **M2–M4**.
- Streaming a chat completion (only `/models` is needed to prove reachability);
  the chat round-trip is exercised once the loop exists in M4.

## Update Log

<!-- entries appended below this line -->
