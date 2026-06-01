# Phase 07b: executor liveness — `awaiting_model` heartbeat during the model wait

**Milestone:** M6 — Plugin + architect/review skills
**Status:** todo
**Depends on:** phase-07a (the model-call site is what both phases touch; land 07a first)
**Estimated diff:** ~190 lines (pre-call emit + heartbeat select + tests)
**Tags:** language=rust, kind=bugfix, size=m

## Goal

While the executor waits on the local model (especially a long prefill before
the first token), it emits **no progress at all**, so `rexymcp status` keeps
showing the *previous* turn's stage — in dogfood session `6a1dd72e` it sat on
`turn 17, stage:"verify"`. A human (or Claude, via the pull-based status path)
cannot tell "alive, waiting on a slow model" from "hung/dead."

Emit an `awaiting_model` progress event **before** each model call, and a
periodic **heartbeat** that re-emits it while the call is in flight. This keeps
the session log's last record (and `rexymcp status`'s "last sign of life")
fresh during the wait, so the pull-based status clearly distinguishes a busy
prefill from a hang.

## Architecture references

Read before starting:

- `docs/architecture.md` — Layer 2 § "Liveness" (the server emits progress so
  Claude and the user see motion) and § "Long runs". **Note:** the dogfood
  established that Claude Code sends no `progressToken`, so live MCP
  `notifications/progress` cannot fire; the **pull-based `rexymcp status`** is
  the human-liveness path. This phase strengthens that pull path. Whether the
  architecture's § "Liveness" wording should be amended to say so is a
  **06b-retrospective decision**, not this phase's — do not edit
  `docs/architecture.md`.
- `docs/architecture.md` — "The executor turn cycle" step 3 (call the model and
  drain its event stream).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` (esp. hermetic + deterministic tests: **no real
   `sleep`** — use `tokio::time::pause()` + `advance()`; inject the clock, never
   call `Utc::now()`).
2. Read the architecture references above.
3. Read this entire phase doc.
4. Confirm phase-07a is `done` and the four gates are green at HEAD.

## Current state

- `executor/src/agent/mod.rs:206-210` — `deps.client.chat(...).await` runs the
  **entire** streaming completion (sending `AiEvent`s into `tx`) and returns
  only after `[DONE]`; the drain loop at `:214` runs **afterward**. So the
  model's prefill + generation all happen inside one `chat().await` with **no
  emission point during the wait**.
- `executor/src/agent/mod.rs:233-244` — `turns += 1` then the first
  `emit_progress(&emit, "turn_start")` fire **after** the model has already
  responded. The model-wait for the upcoming turn happens while `turns` still
  holds the *previous* turn's count.
- `executor/src/agent/mod.rs:888` — `emit_progress(ctx, stage)` is the single
  emission helper: it fires the optional live callback and **always** logs a
  `SessionEvent::Progress` (so `rexymcp status` and post-return queries see
  liveness even with no live watcher).
- Canonical stage tags are documented in `executor/src/agent/progress.rs:27-28`
  (`"turn_start"`, `"tool:<name>"`, `"verify"`, `"command:<name>"`).
- `mcp/src/status.rs:63-71` — `rexymcp status` folds the log: each
  `SessionEvent::Progress` updates `latest_turn` / `latest_stage` /
  `latest_message`, and **every** record advances `last_ts`. `format_status`
  renders "turn N, stage <stage>" and the age of `last_ts` ("last sign of
  life"). A stale `last_ts` is what makes a long wait look like a hang.

## Spec

Numbered tasks in execution order.

1. **Add `awaiting_model` to the stage vocabulary** — extend the canonical-stage
   doc comment in `executor/src/agent/progress.rs:27-28` to include
   `"awaiting_model"`. (The stage is a free string; this is documentation of the
   set, not an enum.)

2. **Emit `awaiting_model` before the model call** — in
   `executor/src/agent/mod.rs`, immediately before the `deps.client.chat(...)`
   call (step 3), emit a progress event with stage `"awaiting_model"` for the
   **upcoming** turn (`turns + 1`, since `turns` is incremented only after the
   response drains). This flips `rexymcp status` off the previous turn's
   (misleading) stage as soon as the wait begins. Reuse the existing `EmitCtx` /
   `emit_progress` machinery.

3. **Heartbeat during the wait** — drive the `chat(...)` future concurrently
   with a `tokio::time::interval` (heartbeat period: a module const, **15 s**)
   via `tokio::select!`: on each tick while `chat` is still pending, re-emit the
   `awaiting_model` progress event (same turn number); when `chat` resolves,
   stop ticking and proceed to drain `rx` exactly as today. Each re-emit logs a
   fresh `SessionEvent::Progress`, so `last_ts` stays current and `rexymcp
   status` shows the executor is alive and waiting. Do **not** change the drain
   loop, the turn counter, or any post-response behavior.

   Keep this confined to the agent loop — do **not** add a progress channel or a
   new `AiEvent` variant into `OpenAiClient::chat` (the drain runs after `chat`
   returns, so a channel event would not be live anyway).

4. **No behavior change on the model's decisions.** The heartbeat is pure
   observability: it must not alter turn counting, the completion buffer, tool
   dispatch, hard-fail/budget logic, or the `PhaseResult`. If `chat` returns
   `Err` (e.g. a real 07a-classified stall), propagate exactly as before.

## Acceptance criteria

- [ ] `"awaiting_model"` is listed in the canonical-stage doc comment in
      `progress.rs`.
- [ ] An `awaiting_model` `SessionEvent::Progress` (turn = upcoming turn) is
      logged immediately before the model call, on every turn.
- [ ] While the model call is in flight, an `awaiting_model` event is re-emitted
      every heartbeat period (15 s), and stops once the call resolves.
- [ ] `rexymcp status` against a session log whose latest record is an
      `awaiting_model` Progress renders `stage awaiting_model`.
- [ ] Turn counting, the completion buffer, and the `PhaseResult` are unchanged
      (existing agent-loop tests still pass unmodified).
- [ ] All four gate commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. Use a `MockAiClient` (`ai/testing.rs`) whose `chat`
awaits a controllable gate so the test holds the call "in flight," and
`tokio::time::pause()` + `advance()` to step the heartbeat interval. Pin
behavior + names; structure is the executor's.

- `awaiting_model_emitted_before_model_call` — run one loop turn against a mock
  client; assert the session log contains an `awaiting_model` Progress for the
  upcoming turn **ordered before** that turn's `Completion`/`turn_start`
  records.
- `heartbeat_reemits_awaiting_model_while_in_flight` — with paused time and a
  `chat` that stays pending across two heartbeat periods, assert ≥ 2
  `awaiting_model` Progress records are logged before the call resolves, and
  their timestamps advance (fresh `last_ts`).
- `heartbeat_stops_when_model_responds` — once `chat` resolves, no further
  `awaiting_model` records appear (negative case: advancing time past more
  heartbeat periods adds none).
- `status_renders_awaiting_model_stage` in `mcp/src/status.rs` — fold a log
  whose latest Progress has stage `"awaiting_model"`; `format_status` output
  contains `stage awaiting_model`.

## End-to-end verification

Build a temp repo with a `.rexymcp/sessions/<name>.jsonl` whose last record is
an `awaiting_model` `SessionEvent::Progress`, then run the real CLI:

```
rexymcp status --repo <tmpdir>
```

Quote the actual output in the completion Update Log; it must show
`stage awaiting_model` and a "last sign of life" age. (This exercises the real
`rexymcp status` artifact end-to-end; the live in-flight heartbeat itself is
covered by the hermetic paused-time tests above and will be observed in the next
real dogfood run.)

## Authorizations

- [x] **May modify** `executor/src/agent/mod.rs` (pre-call emit + heartbeat
      select around the model call), `executor/src/agent/progress.rs` (stage
      doc comment). May add hermetic tests in those files and in
      `mcp/src/status.rs`.
- [ ] **No new dependencies.** `tokio` (with `time`) is already present; the
      `MockAiClient` lives in `executor/src/ai/testing.rs`.
- [ ] May **NOT** edit `docs/architecture.md` (the § "Liveness" amendment is a
      06b decision), `STANDARDS.md`, `WORKFLOW.md`, `AGENTS.md`, `Cargo.toml`, or
      any phase doc other than this one.

## Out of scope

- **Anything making live MCP `notifications/progress` fire** — Claude Code sends
  no `progressToken`, so the live push path is closed at the client. This phase
  improves the **pull** path (`rexymcp status`) only.
- **Restructuring `chat` to stream tokens concurrently with the drain loop** —
  the buffered-then-drained structure stays; only a heartbeat wraps the await.
- **The first-token-stall timeout/retry** — that is phase-07a.
- **Amending the architecture's Liveness section** or any `rexymcp status`
  output-format redesign — 06b retrospective territory.
- **Encoding elapsed-seconds into the progress message** — the freshened
  `last_ts` already conveys liveness; do not change `format_message`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
