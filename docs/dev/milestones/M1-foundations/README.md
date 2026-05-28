# M1 — Foundations

**Goal:** Stand up the rexyMCP Cargo workspace, its configuration and error
model, the lifted AI client, and a runnable health-check entrypoint that connects
to a live local OpenAI-compatible endpoint and lists its models.

**Status:** planning

**Depends on:** none

**Exit criteria:**
- `cargo build`, `cargo clippy -D warnings`, and `cargo test` all pass on a fresh
  checkout, with CI running the same.
- The `executor` library exposes a working `AiClient` trait + an
  OpenAI-compatible client (lifted from Rexy) with circuit-breaker/retry, covered
  by hermetic `MockAiClient` tests.
- A binary entrypoint reads rexyMCP config (endpoint, model, target-repo root,
  per-project commands) and, given a reachable endpoint, prints the list of
  models served by it — verified end-to-end against a real vLLM/LM Studio/Ollama
  instance.

## Architecture references

- `docs/architecture.md` — "The three layers" → "Layer 1 — `executor` crate"
  (the lift/drop map and turn cycle).
- `docs/architecture.md` — "Configuration".

## Phases

| #  | Phase                                                                        | Status |
|----|------------------------------------------------------------------------------|--------|
| 01 | workspace + config + error model ([phase-01-workspace-and-config.md](phase-01-workspace-and-config.md)) | todo   |
| 02 | AI client (lift from Rexy) ([phase-02-ai-client.md](phase-02-ai-client.md))  | todo   |
| 03 | health-check entrypoint ([phase-03-health-check.md](phase-03-health-check.md)) | todo   |

## Notes

This milestone deliberately defers all executor tooling (M2), the parser (M3),
and the agent loop (M4). M1's job is to make the workspace real, prove the
local-LLM connection works end-to-end, and establish the config + error
contracts every later milestone builds on.

The AI client, message/event types, and `MockAiClient` are **lifted from Rexy**
(`rexy/src/ai/`). They are copied and adapted, not depended on — rexyMCP does not
link against Rexy.
