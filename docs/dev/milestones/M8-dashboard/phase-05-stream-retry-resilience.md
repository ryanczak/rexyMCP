# Phase 05: executor stream-retry resilience — survive mid-stream connection drops

**Milestone:** M8 — Live session dashboard
**Status:** in-progress (bounced — see [bugs/bug-05-1.md](bugs/bug-05-1.md): non-hermetic test)
**Depends on:** none (executor-crate resilience fix; independent of the dashboard
phases). Closes `bug-executor-2`.
**Estimated diff:** ~190 lines (`executor/src/ai/backends/openai.rs` buffering +
retry + classifier + tests).
**Tags:** language=rust, kind=bugfix, size=m

## Goal

Fix `bug-executor-2`: a mid-stream connection drop currently aborts the whole run
to `hard_fail`, discarding all prior turns (phase-04 lost a 76-turn run to a
single `error decoding response body`). Make a transient transport failure during
the completion stream trigger a **bounded, backed-off retry** of the request —
the same resilience the pre-first-token stall path already has — instead of
aborting. Use the **Option A (buffer-then-flush)** design: the backend accumulates
the completion locally and emits it only once the stream completes `Ok`, so a
retry can cleanly discard the partial and re-issue.

This is an executor-resilience phase (like phase-03) that surfaced during M8. It
is independent of the dashboard work and the vLLM-side investigation of *why* the
stream drops — it is the client-side defense-in-depth: any OpenAI-compatible
backend can drop a connection, and the executor should not throw away completed
work over one transient read.

## Architecture references

- `executor/src/ai/backends/openai.rs` — the streaming `chat` impl; the inline
  drain loop (line ~192) and `should_retry_stall` (line ~385) are the change sites.
- `executor/src/ai/mod.rs:171` — `stream_next_with_timeout`; **no change needed**
  (see the downcast insight below), quoted here only for context.
- `bug-executor-2` (this milestone's `bugs/`) — the filed defect.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `executor/src/ai/backends/openai.rs` — specifically the `chat` impl
   (the `loop` at ~192 through ~356) and the test module's stream-mock helpers.
   **This file is moderately large; if a full read risks `RunawayOutput`, read the
   `chat` function range specifically (lines ~169–356) and the test module
   (~470–860), not the whole file in one call.**
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Key insight — classify without changing the seam

`stream_next_with_timeout` (`ai/mod.rs:178`) maps a stream item's
`Some(Err(e))` via `Some(Err(e.into()))`, turning the `reqwest::Error` into an
`anyhow::Error`. **`anyhow` preserves the original type for `downcast_ref`** — so
a real transport error is recoverable as a `reqwest::Error`, while the *synthetic*
errors (the stall `anyhow::anyhow!("SSE stream stalled …")` at `ai/mod.rs:180`,
and the `MAX_LEFTOVER_BYTES` abort in the drain) are plain `anyhow` errors that do
**not** downcast. That is exactly the discriminator we need — no signature change
to `stream_next_with_timeout`:

- `e.downcast_ref::<reqwest::Error>().is_some()` → a transport/body failure during
  streaming → **retriable** (the new behavior).
- downcast fails → it is the stall or the runaway-buffer abort → **existing**
  handling (the runaway abort must NOT be retried).

## Current state

### The eager token-emit sites (the buffering blocker), `openai.rs`

The drain sends tokens to the consumer *as they arrive* (4 sites). These are what
Option A must replace with appends to a local buffer:

```rust
// ~256 — open the reasoning section
let _ = tx.send(AiEvent::Token("</think>".to_string()));
// ~259 — a reasoning chunk
let _ = tx.send(AiEvent::Token(chunk.to_string()));
// ~266 — close reasoning when content starts
let _ = tx.send(AiEvent::Token("</think>\n".to_string()));
// ~270 — a content chunk
let _ = tx.send(AiEvent::Token(content.to_string()));
```

Tool-call args are already accumulated locally (`tool_id` / `tool_name` /
`tool_args`); `emit_tool_call_generic` is called mid-stream at ~284 (when a second
tool id appears) and at ~337 (stream end). The consumer
(`agent/mod.rs`) keeps only the **first** `ToolCallGeneric` it receives.

### The drain error arm + retry decision, `openai.rs` (~326, ~331-353, ~385)

```rust
                    Some(Err(e)) => break Err(e),
                    None => break Ok(()),
                }
            };

            match stall_result {
                Ok(()) => {
                    if in_reasoning {
                        let _ = tx.send(AiEvent::Token("</think>\n".to_string()));
                    }
                    if !tool_id.is_empty() {
                        emit_tool_call_generic(&tx, &tool_id, &tool_name, &tool_args);
                    }
                    let _ = tx.send(AiEvent::Completion { finish_reason, model: served_model });
                    let _ = tx.send(AiEvent::Done(usage));
                    return Ok(());
                }
                Err(e) => {
                    if should_retry_stall(first_token_seen, retries, MAX_FIRST_TOKEN_RETRIES) {
                        retries += 1;
                        continue;
                    }
                    return Err(e);
                }
            }

// elsewhere:
fn should_retry_stall(first_token_seen: bool, retries: u32, max_retries: u32) -> bool {
    !first_token_seen && retries < max_retries
}
```

`AiEvent` (`ai/types.rs`): `Token(String)`, `ToolCallGeneric { … }`,
`Done(TokenBreakdown)`, `Completion { finish_reason, model }`, `Error(String)`.

## Spec

### Task 1 — Buffer the completion instead of emitting eagerly (Option A)

In the `chat` drain, accumulate output into per-attempt local state instead of
sending it to `tx`:

- Replace the four eager `tx.send(AiEvent::Token(...))` sites with appends to a
  local `String` (e.g. `out`), building the **same** text — including the
  `"</think>"` open marker, the reasoning chunks, the `"</think>\n"` close marker,
  and the content chunks, in the same order the eager path produced them.
- Defer tool-call emission: instead of calling `emit_tool_call_generic` mid-stream
  (~284) and at stream end (~337), collect completed tool calls into a local
  `Vec` (id, name, args) in arrival order.
- On `stall_result == Ok(())` **only**: if `in_reasoning`, append the trailing
  `"</think>\n"` to `out`; then emit `AiEvent::Token(out)` (one consolidated token
  — the consumer concatenates, so the result is identical), then emit each
  buffered tool call via `emit_tool_call_generic` **in arrival order** (preserving
  "first tool call wins" downstream), then `Completion`, then `Done`, then
  `return Ok(())`.

Net effect: **nothing is sent to `tx` until the stream completes successfully**,
so a retry that discards the per-attempt buffer is clean. (Per-attempt locals are
already declared inside the retry `loop`, so `continue` resets them; the only
thing that previously leaked across a retry was eager `tx` output, now removed.)

### Task 2 — Classify the error and retry transient transport drops

Add a small pure helper:

```rust
/// A stream error worth retrying: a transport/body failure (the connection
/// dropped mid-stream), as opposed to a stall timeout or our own runaway-buffer
/// abort, which are synthetic `anyhow` errors that don't downcast.
fn is_retriable_transport(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>().is_some()
}
```

In the `Err(e)` arm of the `stall_result` match, branch:

```rust
Err(e) => {
    if is_retriable_transport(&e) && stream_retries < MAX_STREAM_RETRIES {
        stream_retries += 1;
        tokio::time::sleep(stream_retry_backoff(stream_retries)).await;
        continue;   // per-attempt buffer is discarded; request re-issued
    }
    if should_retry_stall(first_token_seen, retries, MAX_FIRST_TOKEN_RETRIES) {
        retries += 1;
        continue;
    }
    return Err(e);
}
```

- Add `let mut stream_retries = 0;` alongside `retries` (before the `loop`).
- `const MAX_STREAM_RETRIES: u32 = 3;`
- `fn stream_retry_backoff(attempt: u32) -> Duration` — a short bounded backoff,
  e.g. `Duration::from_millis(250 * 2u64.pow(attempt - 1))` capped at ~2 s (250ms,
  500ms, 1s). Keep it a pure function so it is unit-testable.

The runaway-buffer abort and the stall both stay non-transport (don't downcast),
so they keep their exact current behavior — the runaway abort is **not** retried.

### Task 3 — Tests (hermetic)

`is_retriable_transport` and `stream_retry_backoff` are pure and directly
unit-testable. For `is_retriable_transport`, construct a real `reqwest::Error`
(e.g. by `reqwest`-getting an unroutable URL inside the test, or reuse any helper
the test module already has) wrapped in `anyhow`, and assert it returns `true`;
assert a plain `anyhow::anyhow!("SSE stream stalled …")` returns `false`.

Add unit tests:
- `is_retriable_transport_true_for_reqwest_error` — a downcastable reqwest error → true.
- `is_retriable_transport_false_for_synthetic_stall` — `anyhow::anyhow!("stalled")` → false.
- `is_retriable_transport_false_for_runaway_abort` — the `anyhow` runaway-buffer
  message → false.
- `stream_retry_backoff_is_bounded_and_increasing` — backoff grows with attempt
  and never exceeds the cap.

**Note on full end-to-end retry tests:** the production `chat` drain consumes a
`reqwest` `bytes_stream`, which is awkward to drive hermetically. The existing
`drain_stream_with_retry` (test-only today) plus the pure helpers above give
adequate coverage of the decision logic without a live backend. **Do not** add a
live-network test. If you find a clean way to exercise the buffer-then-flush +
retry against a scripted in-memory stream (mirroring the existing
`drain_stream_with_retry` tests), add one; otherwise the pure-helper tests plus
the unchanged existing stream tests are sufficient for this phase.

## Acceptance criteria

- [ ] Nothing is sent to `tx` until the completion stream finishes `Ok` (eager
      `AiEvent::Token` sends in the drain are gone; output is buffered and flushed
      once on success).
- [ ] A transient transport error mid-stream (a `reqwest::Error`, identified via
      downcast) triggers a bounded, backed-off retry (`MAX_STREAM_RETRIES`), not an
      immediate `hard_fail`.
- [ ] A stall timeout and the `MAX_LEFTOVER_BYTES` runaway abort keep their
      current behavior (stall: pre-first-token retry only; runaway: no retry).
- [ ] The consumer still receives one well-formed completion (text + tool calls in
      the same order, `Completion`, `Done`) on success — `<think>` markers and
      "first tool call wins" semantics unchanged.
- [ ] `is_retriable_transport` and `stream_retry_backoff` are pure and unit-tested.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

- `is_retriable_transport_true_for_reqwest_error`, `_false_for_synthetic_stall`,
  `_false_for_runaway_abort` — the classifier.
- `stream_retry_backoff_is_bounded_and_increasing` — the backoff curve.
- Existing `should_retry_stall_*`, `midstream_stall_is_not_retried`, and
  `drain_stream_with_retry` tests must keep passing (the stall path is unchanged).

## End-to-end verification

No CLI surface change; the fix is internal to the AI backend. Verify:

1. `cargo test -p rexymcp-executor` passes, including the new classifier/backoff
   tests; quote the new test names and pass status in the Update Log.
2. Confirm the four eager `tx.send(AiEvent::Token(...))` sites in the drain are
   gone (grep): `grep -n 'tx.send(AiEvent::Token' executor/src/ai/backends/openai.rs`
   should show only the single consolidated flush on the success path.

## Authorizations

- [x] May modify `executor/src/ai/backends/openai.rs` (buffering, retry, classifier,
      tests).
- [ ] `executor/src/ai/mod.rs` does **not** need changing — the downcast insight
      avoids touching `stream_next_with_timeout`. If you believe a seam change is
      unavoidable, **stop and file a blocker** rather than widening scope. No
      `Cargo.toml` (reqwest/anyhow/tokio are already deps), no `docs/architecture.md`.

## Out of scope

- Routing production `chat` through the existing `drain_stream_with_retry`
  abstraction (a larger refactor; the inline loop is modified in place here).
- Retrying non-transport failures (4xx/5xx response *status* errors — those are
  caught at send time, not in the body stream), or the runaway-buffer abort.
- A configurable retry policy / surfacing retries in telemetry — a later phase if
  wanted. (`RunMetrics` could gain a `stream_retries` counter eventually.)
- Any change to the agent loop's `AiEvent::Error` → `hard_fail` handling.
- Live-network tests.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-02 (review bounce)

**Executor:** Qwen/Qwen3.6-27B-FP8 (implementation); architect committed the work
the run left uncommitted (`136713c`) and reviewed.

**Outcome:** bounced — [bug-05-1](bugs/bug-05-1.md) (major). The production code
(Tasks 1 & 2: buffer-then-flush, `is_retriable_transport`, `stream_retry_backoff`,
the retry branch) is **correct and complete** — independent fmt/clippy/test all
green, the buffering preserves text + tool-call ordering and "first tool call wins,"
the classifier and bounded backoff are right. The single defect is test hygiene:
`is_retriable_transport_true_for_reqwest_error` makes a **real network call**
(`reqwest::get("http://10.255.255.1:1")`), violating STANDARDS §3.3/§4 (hermetic,
no real network) and inflating the executor suite to ~30 s wall-clock (0.67 s CPU)
on a connection timeout. Fix is pre-injected in bug-05-1: construct the
`reqwest::Error` synchronously from an unparseable URL (`.get("not-a-url").build()`),
drop `#[tokio::test]`. On re-dispatch, also flip status → `review`, fill the
completion Update Log, and commit (the run skipped its closeout).

**Re-dispatch note:** the implementation is done — **only fix the one test**; do not
re-do the production logic.
