# M1 — Foundations

**Goal:** Stand up the rexyMCP Cargo workspace, its configuration and error
model, the lifted AI client, and a runnable health-check entrypoint that connects
to a live local OpenAI-compatible endpoint and lists its models.

**Status:** in-progress

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
| 01 | workspace + config + error model ([phase-01-workspace-and-config.md](phase-01-workspace-and-config.md)) | done |
| 02 | AI client (lift from Rexy) ([phase-02-ai-client.md](phase-02-ai-client.md))  | done |
| 03 | health-check entrypoint ([phase-03-health-check.md](phase-03-health-check.md)) | todo   |

## Notes

This milestone deliberately defers all executor tooling (M2), the parser (M3),
and the agent loop (M4). M1's job is to make the workspace real, prove the
local-LLM connection works end-to-end, and establish the config + error
contracts every later milestone builds on.

The AI client, message/event types, and `MockAiClient` are **lifted from Rexy**
(`rexy/src/ai/`). They are copied and adapted, not depended on — rexyMCP does not
link against Rexy.

### Calibration data (first occurrences — noted, not yet folded)

Phase-01 review surfaced two spec-induced issues. Per WORKFLOW.md § "Calibration"
a first occurrence is noted, not folded. If either recurs, fold into
STANDARDS/WORKFLOW:

1. **Test plan mandated real env mutation** ("set/remove env vars within the
   test"), which is `unsafe` in edition 2024 and non-hermetic — see
   `bugs/bug-01-1.md`. Lesson candidate: spec test plans should require
   injectable/faked external state, never real-process-env or wall-clock
   mutation. (Code-quality angle → STANDARDS §3.3; spec-writing angle →
   WORKFLOW.)
2. **rustfmt.toml carried nightly-only options** (`imports_granularity`,
   `group_imports`) under a stable toolchain, so they warn and no-op. Lesson
   candidate: don't spec nightly-only formatter options without pinning a nightly
   fmt step. Fixed at the principal-engineer level (dropped the two lines).
