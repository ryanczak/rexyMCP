# Phase 07a: SSE prefill-stall — first-token vs. inter-token timeout + retry

**Milestone:** M6 — Plugin + architect/review skills
**Status:** review
**Depends on:** phase-06a (done) — surfaced by the dogfood smoketest it prepared
**Estimated diff:** ~220 lines (timeout split + config + bounded retry + tests)
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The executor aborts a phase with a bare "SSE stream stalled" error when the
**local model takes longer than 90 s to produce its first token** — which is
normal *prefill* latency on a large/growing context, not a dropped connection.
A single `STREAM_CHUNK_TIMEOUT = 90s` (`executor/src/ai/mod.rs:171`) is applied
uniformly to every chunk, so the long wait before the **first** token is judged
by the same budget as the short gaps **between** tokens.

Split the one timeout into two — a generous **first-token** (prefill) budget and
a tight **inter-token** (idle) budget — make both configurable, and **retry the
completion** when the stall happens before any token has been emitted (the safe
case: nothing has been streamed to the consumer yet, so re-issuing is clean).
This removes the false-stall that ended dogfood session `6a1dd72e` at turn 17.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" step 3 (call the model and
  drain its event stream) and Layer 2 § "Long runs" (a phase can take minutes).
  This phase keeps that contract; it does not change what a turn does, only how
  patiently it waits for the model's first token.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom (esp. the error model: infra
   failures → `Error`; model-visible outcomes → a `ToolResult`-style value; no
   `unwrap`/`expect`/`panic!` in production; tests hermetic + deterministic, **no
   real `sleep`** — use `tokio::time::pause()` + `advance()` for time).
2. Read the architecture references above.
3. Read this entire phase doc before touching code.
4. Confirm the repo is on a clean branch with no uncommitted changes and the four
   gates are green at HEAD.

## Current state

The stall and abort path, end to end:

- `executor/src/ai/mod.rs:171` — `pub const STREAM_CHUNK_TIMEOUT: Duration =
  Duration::from_secs(90);`
- `executor/src/ai/mod.rs:173` — `stream_next_with_timeout(stream)` wraps
  `stream.next()` in `tokio::time::timeout(STREAM_CHUNK_TIMEOUT, …)` and, on
  elapse, returns `Some(Err(anyhow!("SSE stream stalled — no data received for
  90s …")))`. **The same 90 s applies to every chunk, including the first.**
- `executor/src/ai/backends/openai.rs:174` — the `'outer` loop calls
  `stream_next_with_timeout(&mut stream).await` and `let bytes = result?;`
  propagates the stall as `Err` out of `chat()`.
- `executor/src/ai/mod.rs:156` — `send_with_retry` only wraps the **initial
  POST**; once the byte-stream is open, a stall is **not** retried.
- `executor/src/agent/mod.rs:207-210` — `deps.client.chat(...).await.map_err(|e|
  Error::Backend(e.to_string()))?` propagates the stall straight out of
  `execute_phase`. (The `AiEvent::Error` branch at `:227-229` is the secondary
  route to the same `Err(Error::Backend(_))`.)

Observed in dogfood session `~/src/ms_pacman/.rexymcp/sessions/
session-phase-02-6a1dd72e.jsonl`: 17 turns completed in ~305 s; the last record
is `turn 17, stage:"verify"`; **no turn-18 completion exists**. The turn-18
request's first token didn't arrive within 90 s (prefill on the grown
transcript), the stall fired, and the phase aborted. A follow-up health check
succeeded — the endpoint was never down.

`ExecutorConfig` (`executor/src/config.rs:22-43`) currently carries only
`provider` / `model` / `base_url` / `api_key`; `OpenAiClient`
(`backends/openai.rs`) is built from those four via `make_client`
(`ai/mod.rs:188`). There is no timeout setting anywhere.

## Spec

Numbered tasks in execution order.

1. **Two configurable timeouts** — in `executor/src/config.rs`, add to
   `ExecutorConfig` two fields with `#[serde(default = "…")]` defaults:
   - `first_token_timeout_secs: u64` — budget for the wait *before the first
     token* of a completion (prefill). Default **600** (10 min), matching the
     architecture's minutes-long-runs envelope.
   - `stream_idle_timeout_secs: u64` — budget for the gap *between* tokens once
     streaming has begun. Default **90** (the current value — a real
     mid-generation gap this long is a genuine drop).

   Keep `ExecutorConfig`'s existing `Default` impl in sync (it is a hand-written
   `impl Default`, not a derive — add the two fields there too). Both must
   round-trip through TOML and fall back to the defaults when absent.

2. **Per-call timeout in the stream helper** — in `executor/src/ai/mod.rs`,
   change `stream_next_with_timeout` to take the timeout as a parameter
   (`timeout: Duration`) instead of reading the `STREAM_CHUNK_TIMEOUT` const.
   Keep the const **removed or repurposed** — no caller may read a single global
   chunk timeout after this phase. The stall error message must report the
   *actual* elapsed budget used (so a first-token stall reads `600s`, an idle
   stall reads `90s`).

3. **First-token vs. inter-token selection** — in `backends/openai.rs`'s `chat`
   streaming loop, track whether any token/tool-call delta has been emitted yet
   (`first_token_seen: bool`). Pass `first_token_timeout` to
   `stream_next_with_timeout` while `!first_token_seen`, and `idle_timeout`
   after. "First token seen" flips on the first non-empty `content` /
   `reasoning` / `tool_calls` delta — i.e., the first sign the model has started
   generating. (SSE keep-alive comments / empty deltas do **not** flip it.)

4. **Bounded retry on a first-token stall** — wrap the request-and-stream in a
   bounded retry (reuse the existing 2-retry shape from `send_with_retry_inner`
   for symmetry; **do not** add a dependency). If the stream stalls **with
   `first_token_seen == false`** — nothing has been sent to `tx` yet — re-issue
   the whole completion (new POST + new stream). Retry at most **twice**, then
   surface the stall as `Err`. If the stall happens **after** `first_token_seen`
   (mid-generation), **do not retry** — partial tokens were already pushed to the
   consumer, so a re-issue would duplicate them; surface it as `Err`
   immediately.

5. **Thread the timeouts into the client** — give `OpenAiClient` the two
   `Duration`s (new struct fields), set them in `OpenAiClient::new` /
   `make_client` from `ExecutorConfig`. Default-construct paths in tests may use
   the config defaults.

Do **not** change the agent loop's handling of a terminal `Err(Error::Backend)`
in this phase (see Out of scope).

## Acceptance criteria

- [ ] `ExecutorConfig` has `first_token_timeout_secs` (default 600) and
      `stream_idle_timeout_secs` (default 90); both load from TOML and fall back
      to defaults when the keys are absent.
- [ ] `stream_next_with_timeout` takes a `Duration` parameter; no global
      single-chunk timeout const is read by any caller.
- [ ] In `chat`, the first-token wait uses `first_token_timeout` and subsequent
      waits use `stream_idle_timeout`; the flip happens on the first non-empty
      content/reasoning/tool-call delta, **not** on empty/keep-alive lines.
- [ ] A first-token stall is retried (≤ 2 times) before failing; a mid-stream
      stall (after a token was emitted) is **not** retried.
- [ ] The stall error message reports the actual budget that elapsed (600 vs.
      90).
- [ ] All four gate commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. Use `tokio::time::pause()` + `tokio::time::advance()`
so no test sleeps in real time. Pin behavior and names; structure is the
executor's call.

- `config_defaults_first_token_and_idle_timeouts` in `executor/src/config.rs` —
  `ExecutorConfig::default()` yields `first_token_timeout_secs == 600` and
  `stream_idle_timeout_secs == 90`.
- `config_loads_overridden_timeouts` — a TOML with explicit values overrides the
  defaults; a TOML omitting them keeps the defaults (negative case).
- `stream_next_uses_supplied_timeout` in `executor/src/ai/mod.rs` — with paused
  time, a stream that yields nothing returns the stall `Err` only after the
  *supplied* `Duration` elapses (advance to just-before → still pending; advance
  past → stall), and the message contains the supplied seconds. Drive it with a
  generic `futures_util::stream` (e.g. `stream::pending()` and a hand-built
  stream), not a real reqwest body.
- `first_token_stall_retries_then_succeeds` — a fake stream that stalls on the
  first poll, then on re-issue yields a token + `[DONE]`, produces a successful
  completion (retry path). Pin the retry **cap** with a negative case:
  `first_token_stall_exhausts_retries_then_errors`.
- `midstream_stall_is_not_retried` — a fake stream that emits one content token
  then stalls returns `Err` without re-issuing (assert the request factory was
  invoked exactly once).

If driving the retry through `OpenAiClient::chat` proves to require reqwest
specifics, factor the timeout-selection + retry-decision into a stream-generic
helper (generic over item/error type) and unit-test that helper directly; pin
the helper's behavior, not its exact signature. A full live stall→retry against
a real endpoint may be added as an `#[ignore]`-gated test, per STANDARDS.

## End-to-end verification

> Not applicable — this phase ships no new runtime-loadable artifact (a CLI
> flag, a checked-in file, a config the binary surfaces to the user). It changes
> the internal streaming timeout/retry behavior of `OpenAiClient`, exercised by
> the hermetic tests above. The behavior is observable end-to-end only against a
> live slow-prefill endpoint, which the next real dogfood run will exercise.

## Authorizations

- [x] **May modify** `executor/src/config.rs` (add two `ExecutorConfig` fields +
      their defaults), `executor/src/ai/mod.rs` (timeout param on
      `stream_next_with_timeout`; remove/repurpose `STREAM_CHUNK_TIMEOUT`),
      `executor/src/ai/backends/openai.rs` (first-vs-inter-token selection +
      bounded first-token retry + `OpenAiClient` fields).
- [ ] **No new dependencies.** `tokio` (with `time`) and `futures-util` are
      already present; reuse them.
- [ ] May **NOT** edit `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `AGENTS.md`, `Cargo.toml`, or any phase doc other than this one.

## Out of scope

- **Converting a terminal `Err(Error::Backend)` into a structured `PhaseResult`
  (preserving the partial diff) at `agent/mod.rs:210/229`.** This touches the
  `execute_phase` contract and the error model (infra failure → `Error`), and is
  a deliberate architect decision deferred to the M6 06b retrospective. Note it
  in "Notes for review" if you touch nearby code, but do not change it.
- **Any resume/"continue if phase status ≠ done" mechanism.** A cross-dispatch
  resume is a separate design question for 06b, not part of the timeout fix.
- **The liveness/idle-signal gap** (Claude can't tell "busy prefilling" from
  "dead") — that is phase-07b.
- Touching `send_with_retry` / the circuit breaker beyond reusing the retry
  shape.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-01 (progress)

Added `first_token_timeout_secs` (default 600) and `stream_idle_timeout_secs` (default 90) to `ExecutorConfig` with serde defaults. Changed `stream_next_with_timeout` to take a `Duration` parameter and removed the `STREAM_CHUNK_TIMEOUT` const. Added timeout fields to `OpenAiClient`, threaded through `make_client` and `mcp/src/runner.rs`. Implemented first-token vs inter-token selection in `chat()` with `first_token_seen` tracking on non-empty content/reasoning/tool_calls deltas. Implemented bounded retry (≤2) on first-token stalls via a `drain_stream_with_retry` helper generic over stream type. Added tests for config defaults, TOML load/omit, stream timeout behavior, first-token stall retry, retry exhaustion, and mid-stream no-retry.

### Update — 2026-06-01 (complete)

**Summary:** Split the uniform 90s SSE stream timeout into two configurable timeouts (first-token: 600s, inter-token: 90s) with bounded retry on first-token stalls. The `chat()` loop tracks `first_token_seen` on the first non-empty delta and selects the appropriate timeout. First-token stalls retry up to 2 times; mid-stream stalls error immediately. Stall error messages report the actual budget that elapsed.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.1 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.80s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.1 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.15s

cargo test 2>&1 | tail -10
test result: ok. 525 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**
Not applicable — phase ships no new runtime-loadable artifact (a CLI flag, a checked-in file, a config the binary surfaces to the user). It changes the internal streaming timeout/retry behavior of `OpenAiClient`, exercised by the hermetic tests.

**Files changed:**
- `executor/src/config.rs` — added `first_token_timeout_secs` (default 600) and `stream_idle_timeout_secs` (default 90) to `ExecutorConfig` + `Default` impl + 3 tests
- `executor/src/ai/mod.rs` — `stream_next_with_timeout` takes `Duration` param; removed `STREAM_CHUNK_TIMEOUT` const; updated `make_client`; added 1 test
- `executor/src/ai/backends/openai.rs` — added timeout fields to `OpenAiClient`; implemented first-token vs inter-token selection + bounded retry in `chat()`; added `drain_stream_with_retry` helper + 3 tests
- `mcp/src/runner.rs` — updated `OpenAiClient::new` call with timeout args
- `executor/src/health.rs` — added timeout fields to test `ExecutorConfig`

**New tests:**
- `config_defaults_first_token_and_idle_timeouts` in `executor/src/config.rs`
- `config_loads_overridden_timeouts` in `executor/src/config.rs`
- `config_omits_timeouts_keeps_defaults` in `executor/src/config.rs`
- `stream_next_uses_supplied_timeout` in `executor/src/ai/mod.rs`
- `first_token_stall_retries_then_succeeds` in `executor/src/ai/backends/openai.rs`
- `first_token_stall_exhausts_retries_then_errors` in `executor/src/ai/backends/openai.rs`
- `midstream_stall_is_not_retried` in `executor/src/ai/backends/openai.rs`

**Notes for review:** None.

**verification:** fmt OK · clippy OK · tests 525 passed · build OK
