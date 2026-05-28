# Phase 03: Health-check entrypoint

**Milestone:** M1 — Foundations
**Status:** done
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

### Update — 2026-05-28 (progress)

Phase started. Implementing health logic in executor/src/health.rs, adding clap
to mcp/Cargo.toml, rewriting mcp/src/main.rs as a small CLI with health
subcommand. Executor: opencode (qwen3.6-plus).

### Update — 2026-05-28 (complete)

**Summary:** Built `executor/src/health.rs` with `list_models` (async, parses
`data[].id` from OpenAI-compatible `/models` endpoint, returns `Error::Backend`
on failure) and `check` (never errors, returns `Health { reachable: false, .. }`
on transport/parse failure). Added `clap` with `derive` to the workspace and
`mcp/Cargo.toml`. Rewrote `mcp/src/main.rs` into a clap CLI with `health`
subcommand (`--config`, `--base-url` flags) and default name+version behavior.
Five hermetic unit tests added. One `send_with_retry` error-mapping fix needed
(anyhow → Error::Backend).

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.61s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.66s

cargo test 2>&1 | tail -30
test health::tests::joins_base_url_no_trailing_slash ... ok
test health::tests::joins_base_url_trailing_slash ... ok
test health::tests::parse_rejects_non_list_body ... ok
test health::tests::parses_model_ids_from_openai_list_response ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok
...
test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

AC (failure path): `cargo run -p rexymcp -- health --base-url http://127.0.0.1:1`
```
unreachable: http://127.0.0.1:1
EXIT_CODE=1
```
Exits non-zero, names the unreachable URL.

AC (no subcommand): `cargo run -p rexymcp`
```
rexymcp 0.1.0
EXIT_CODE=0
```
Preserves phase-01 behavior.

AC (live): No local LLM endpoint available on this machine (checked :1234,
:11434 — neither responded). Requires manual verification by the user with
LM Studio / Ollama / vLLM running.

**Files changed:**
- `Cargo.toml` — added `clap` workspace dependency
- `mcp/Cargo.toml` — added `clap.workspace = true`, `tokio.workspace = true`
- `executor/src/lib.rs` — added `pub mod health;`
- `executor/src/health.rs` — new file: `Health` struct, `list_models`, `check`,
  `build_models_url`, `parse_models_list`, 5 unit tests
- `mcp/src/main.rs` — rewritten from stub to clap CLI with `health` subcommand

**New tests:**
- `parses_model_ids_from_openai_list_response` in `executor/src/health.rs`
- `parse_rejects_non_list_body` in `executor/src/health.rs`
- `joins_base_url_trailing_slash` in `executor/src/health.rs`
- `joins_base_url_no_trailing_slash` in `executor/src/health.rs`
- `check_returns_unreachable_on_connection_error` in `executor/src/health.rs`

**Commits:** pending

**Notes for review:** Live endpoint test deferred — no local LLM running.
Failure path and default behavior verified end-to-end.

### Update — 2026-05-28 (architect — live E2E completed)

The deferred live `/models` round-trip (AC live) was run by the architect against
two real OpenAI-compatible endpoints the project owner started. Both returned a
real model list with exit 0:

```
$ rexymcp health --base-url http://127.0.0.1:1234/v1   # LM Studio
http://127.0.0.1:1234/v1
qwen/qwen2.5-coder-14b
qwen/qwen3.6-35b-a3b
google/gemma-4-31b
google/gemma-4-26b-a4b
qwen/qwen3.6-27b
google/gemma-4-e4b
text-embedding-nomic-embed-text-v1.5
EXIT=0

$ rexymcp health --base-url http://brain:8000/v1       # vLLM
http://brain:8000/v1
Qwen/Qwen3.6-27B-FP8
EXIT=0
```

This proves the local-LLM connection end-to-end (M1's exit criterion) against two
distinct backends. All AC satisfied. **Approved → done.**
