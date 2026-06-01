# Bug 1 on phase-07a: retry/timeout logic tested via a `#[cfg(test)]` duplicate, not the shipping path

**Severity:** major (correctness-critical behavior in the shipped path has zero
test coverage; the tests validate a divergent copy with different semantics)
**Status:** verified
**Filed:** 2026-06-01
**Verified:** 2026-06-01 — fixed in `33d2497` (Option B: shared production
decision fns `select_timeout` / `should_retry_stall` / `delta_carries_token`
called by `chat()`; keep-alive negatives tested directly). Architect re-ran all
gates green (537 executor + 130 mcp tests, +12 new).

## What's wrong

The phase's whole point — first-token-vs-inter-token timeout selection and
bounded retry on a pre-token stall — is implemented **twice**, and the tests
exercise the wrong copy.

- **Production** lives in `OpenAiClient::chat`, `executor/src/ai/backends/openai.rs`
  lines ~167–331 (the `loop { … }` with `first_token_seen` / `retries` /
  `MAX_FIRST_TOKEN_RETRIES`, the inner `stall_result = loop { … }`, the
  `has_content || has_reasoning || has_tool_calls` flip at :235–238, and the
  retry decision at :326–330).
- **A second copy** lives in `drain_stream_with_retry`
  (`openai.rs:362-410`), gated `#[cfg(test)]` and **never called by production**
  — confirmed: every reference (`:416`, `:620`, `:646`, `:670`) is inside the
  `#[cfg(test)] mod tests` block.

All three behavioral tests — `first_token_stall_retries_then_succeeds`,
`first_token_stall_exhausts_retries_then_errors`, `midstream_stall_is_not_retried`
— drive `drain_stream_with_retry`, **not** `chat`. So:

1. **The shipping retry/timeout path is untested.** The `chat` loop can drift
   from the helper with no test catching it.

2. **The two copies have different `first_token_seen` semantics**, so the helper
   cannot stand in for production:
   - Production flips `first_token_seen` only on a **non-empty content /
     reasoning / tool-call delta** (`openai.rs:221-238`) — the correct rule per
     the spec.
   - The helper flips it on **any stream item received**
     (`openai.rs:390-393`), regardless of whether that chunk carried a real
     token.

   `midstream_stall_is_not_retried` therefore proves "a stall after the first
   *stream chunk* isn't retried" — not "a stall after the first *token* isn't
   retried," which is what production does. The test passes against semantics
   production doesn't have.

3. **Acceptance criterion 3's negative case is unverified.** The criterion
   reads: "the flip happens on the first non-empty content/reasoning/tool-call
   delta, **not** on empty/keep-alive lines." No test asserts that an empty
   `data:` line or an SSE keep-alive comment leaves `first_token_seen == false`
   (and so keeps the long first-token budget in force). The helper *can't* test
   this — it has no SSE parsing and flips on any item. Per WORKFLOW §
   "Calibration → Pin negative cases, not just positive ones," this boundary is
   exactly where the bug would live.

The production logic reads correct on inspection; this is not a known runtime
defect. But for a subtle, correctness-critical retry/timeout boundary, shipping
it with the real path untested and the tests pointing at a divergent double is a
DoD failure, not a nit.

## What should happen

`chat` and the tests share **one** implementation of the timeout-selection +
stall/retry decision, and the tests exercise that shipping implementation —
including the keep-alive negative case. Per the phase doc's test-plan escape
hatch, the intent was to *factor* (extract) the decision into a stream-generic
helper that **production calls**, not to add a parallel `#[cfg(test)]` copy.

## How to fix

1. **Make `drain_stream_with_retry` real (drop `#[cfg(test)]`) and have `chat`
   call it.** Give it a per-item processing callback so the SSE parsing stays in
   `chat` while the loop/timeout/retry structure lives in one place. Sketch:

   ```
   async fn drain_stream_with_retry<S, F, P>(
       mut open_stream: F,      // FnMut() -> S : (re)issues the request
       mut process_chunk: P,    // FnMut(&Bytes) -> Result<ChunkOutcome>
       first_token_timeout: Duration,
       stream_idle_timeout: Duration,
       max_first_token_retries: u32,
   ) -> Result<()>
   ```

   where `ChunkOutcome` reports `{ token_seen: bool, done: bool }`. The helper
   owns: pick timeout by `first_token_seen`; on `Ok(Some(Ok(bytes)))` call
   `process_chunk`, OR-in `token_seen`, stop on `done`; on stall, retry iff
   `!first_token_seen && retries < cap` (re-issue via `open_stream`) else return
   the budget-reporting error. `chat`'s closure does the leftover-buffer + line
   split + delta parsing + `tx.send`, and decides `token_seen` from the same
   non-empty content/reasoning/tool-call check it uses today.

   If a single closure proves awkward (borrowing `tx`, `leftover`, `usage`
   across calls), an acceptable alternative is to extract just the
   **decision functions** as pure, non-test helpers and call them from both
   `chat` and the tests: `select_timeout(first_token_seen, first, idle)` and
   `should_retry_stall(first_token_seen, retries, cap)`. Either way, no
   behavioral logic may live **only** under `#[cfg(test)]`.

2. **Add the missing negative test** (production semantics): feed a chunk
   sequence containing an SSE keep-alive / empty `data:` line *before* any real
   token and assert `first_token_seen` stays `false` (so a subsequent stall is
   still treated as first-token and retried / still uses the 600 s budget), then
   a real content delta flips it. This is the criterion-3 negative the phase
   requires.

3. Keep the existing positive tests, but re-point them at the shared
   implementation so they cover the shipping path.

## Verification

- [ ] `grep -n "cfg(test)" executor/src/ai/backends/openai.rs` shows no
      behavioral function (only the `mod tests` block) — the retry/timeout
      decision is reachable from production.
- [ ] A test exercises the path `chat` actually runs (shared helper / extracted
      decision fns), not a parallel copy.
- [ ] A test asserts an empty/keep-alive line does **not** flip
      `first_token_seen` (criterion-3 negative).
- [ ] `first_token_stall_retries_then_succeeds`,
      `first_token_stall_exhausts_retries_then_errors`, and
      `midstream_stall_is_not_retried` still pass against the shared
      implementation.
- [ ] `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D
      warnings`, and `cargo test` all clean.

## Notes

- **Out of scope for this bounce** (do not widen): everything else in 07a is
  approved on inspection — the two config fields + defaults + their three tests
  (incl. the omit-keeps-default negative), the `Duration` parameter on
  `stream_next_with_timeout` + const removal + its budget-reporting test, and
  the threading through `make_client` / `runner.rs` / `health.rs`. Don't touch
  the agent loop (the `Err`→`PhaseResult` question stays deferred to 06b).
- **Estimated-diff overrun is not a defect.** 417 insertions in `openai.rs` vs.
  the ~220 estimate is almost entirely the re-indentation from wrapping the
  existing parse block in the new outer `loop`; the genuinely new logic is
  small.
