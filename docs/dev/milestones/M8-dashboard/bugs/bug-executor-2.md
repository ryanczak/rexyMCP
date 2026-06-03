# Executor bug 2: completed work discarded on a mid-stream connection drop

**Severity:** major
**Status:** open
**Filed:** 2026-06-02
**Observed during:** M8 phase-04 dispatch (Qwen/Qwen3.6-27B-FP8) — `hard_fail` at
turn 77 after the implementation and all command gates had already passed.

## What's wrong

When the backend drops the HTTP response stream **mid-generation**, the executor
aborts the entire run to `hard_fail` — even when 76 turns of correct work are
already on disk. There is no retry for a mid-stream transport failure.

The chain:

- `executor/src/ai/backends/openai.rs:201` streams the completion via
  `response.bytes_stream()`.
- The drain loop's error arm, `openai.rs:326`: `Some(Err(e)) => break Err(e)`.
- The retry decision, `openai.rs:385`:
  ```rust
  fn should_retry_stall(first_token_seen: bool, retries: u32, max_retries: u32) -> bool {
      !first_token_seen && retries < max_retries
  }
  ```
  Retries happen **only before the first token**. Once `first_token_seen == true`,
  any stream failure aborts: `should_retry_stall` returns `false` → `return Err(e)`
  → the producer emits `AiEvent::Error(...)` → `agent/mod.rs:340-342` (turns > 0)
  converts it to `HardFailSignal::BackendError` → `hard_fail`.

Observed message: `BackendError: "error decoding response body"` — reqwest's
`Display` for a body-stream read interrupted by a connection close (not a JSON
parse error in our code).

Two design facts make this worse than it looks:

1. **The error type is erased at the seam.** `executor/src/ai/mod.rs:178`,
   `stream_next_with_timeout`, does `Some(Err(e)) => Some(Err(e.into()))`,
   converting the typed `reqwest::Error` into `anyhow::Error`. A *stall* (idle
   timeout) is *also* mapped to an `anyhow::Error` (line 180). So at the drain
   site, a transient connection drop is **indistinguishable** from a stall or any
   other failure without string-matching.
2. **Tokens are emitted eagerly.** The drain calls `tx.send(AiEvent::Token(...))`
   per chunk (`openai.rs:256, 259, 270, 334`). The consumer
   (`agent/mod.rs`) accumulates them into `completion` as they arrive. This is
   *why* `should_retry_stall` forbids post-first-token retry — a naive re-issue
   would make the consumer accumulate attempt-1 + attempt-2 into one garbled
   string. The eager-emit design is the blocker to safe mid-stream retry.

This is the third infrastructure-induced abort of otherwise-complete work in M8
(phase-03 `RunawayOutput`; this; and the earlier model-swap no-ops). The server
-side cause (a vLLM stream drop) is being investigated separately, but the client
should not throw away a completed run over a single transient read error — any
OpenAI-compatible backend can drop a connection.

## What should happen

A transient mid-stream transport failure (connection reset / incomplete body)
should trigger a **bounded, backed-off retry** of the request, transparently, the
same way a pre-first-token stall already does — instead of aborting to
`hard_fail`. Only when retries are exhausted (or the error is non-transient)
should it degrade to `BackendError`/`hard_fail`.

## How to fix

Drafted as **phase-05** (executor stream-retry resilience). The minimal shape:

1. **Preserve/classify the error at the seam** — change `stream_next_with_timeout`
   (or add a classifier) so the drain loop can distinguish `Stalled` vs.
   `Transport(reqwest::Error)` and inspect `is_connect()` / `is_body()` /
   `is_decode()` / `is_timeout()`.
2. **Buffer-then-flush (Option A)** — accumulate the completion in the backend and
   emit `AiEvent::Token`/`Completion`/`Done` only once the stream completes `Ok`,
   so a retry can discard the partial buffer and re-issue cleanly. (Live per-token
   emission is not consumed by anything today — the agent loop only accumulates —
   so the liveness cost is ~zero.)
3. **Broaden the retry decision** — retry on a transient transport error (bounded,
   with short backoff), not just on a pre-first-token stall.

## Verification

- [ ] A hermetic test (using the existing scripted-stream seam) where the stream
      yields content tokens then a transient transport error, then succeeds on
      re-issue: the consumer receives **one** clean completion and the run does
      **not** `hard_fail`.
- [ ] A test where every attempt errors: after the bounded cap, the run degrades
      to `BackendError`/`hard_fail` with a clear message (no infinite loop).
- [ ] A non-transient error (e.g. our `MAX_LEFTOVER_BYTES` runaway abort) is **not**
      retried.
- [ ] `cargo test` passes.
