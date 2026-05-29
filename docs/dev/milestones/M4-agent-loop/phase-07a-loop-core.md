# Phase 07a: executor turn-loop core

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** done
**Depends on:** phase-01–06 (all done). Composes: `ai` (`AiClient`, `AiEvent`,
`Message`, `make_client`), `parser::parse`, `tools` (`ToolRegistry`, `Tool`,
`ToolResult`), `governor::scorer::Scorer`, `context::{budget, compactor}`,
`phase` (`PhaseResult`, `Artifacts`, `Briefing`, `Blocker`, `summarize_attempts`,
`collect_working_files`), `config` (`CommandConfig`).
**Estimated diff:** ~450 lines (loop + prompt assembly + native seam + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

The **core `execute_phase` turn loop** — rexyMCP's first net-new orchestration.
It assembles the executor system prompt, then drives the local model through the
budget-bounded turn cycle: call the model → drain its event stream → turn the
output into a `ToolCall` (native event **or** forgiving-parser text) → dispatch
through the registry → record the outcome → repeat. It terminates **clean** when
the model stops calling tools, or **budget_exceeded** when it runs out of turns or
context, returning a `PhaseResult` either way.

This sub-phase ships the **control flow only**. The observability layer (session
log), the governance layer (verifier retry + hard-fail + read-before-edit), and
the completion artifacts (final command set + diff) are 07b/07c/07d — see § Out of
scope. The loop here is fully exercised by `MockAiClient` integration tests.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" (steps 1–5 are this phase;
  steps 6–8 are later sub-phases). Note line 131: "Logging is a side effect of the
  loop; it never changes what the loop returns" — the loop is correct *without*
  logging, which is why logging folds into 07b cleanly.
- `docs/architecture.md` — "The `PhaseResult` / briefing contract" and "Escalation
  = Claude Code itself": `budget_exceeded` returns a briefing (`Blocker::
  BudgetExceeded`), never calls a cloud model.
- M4 README § Notes — the **native-call seam** (this phase owns it): the OpenAI
  backend already extracts native `tool_calls` from the SSE deltas and emits
  `AiEvent::ToolCallGeneric`; the loop turns that into a `parser::ToolCall {
  origin: Origin::Native }` and dispatches it through the *same* path as a
  text-extracted call.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. Read these surfaces (the loop calls every one):
   - `executor/src/ai/types.rs` — `AiEvent` (`Token` / `ToolCallGeneric` / `Done`
     / `Error`), `Message`, `ToolSchema`, `TokenBreakdown`.
   - `executor/src/ai/mod.rs` — the `AiClient` trait (`chat` streams events over
     an `UnboundedSender<AiEvent>` and returns `Result<()>`).
   - `executor/src/ai/testing.rs` — `MockAiClient` (one scripted string per call)
     and `MockAiClientEvents` (drains its whole event script in **one** call).
   - `executor/src/parser/mod.rs` — `parse(&str, &ToolRegistry) -> ParseResult`
     (`NoToolCall` / `Found(ToolCall)` / `Failed(ParseFailure)`); `ToolCall`,
     `Origin::Native`.
   - `executor/src/tools/registry.rs` — `ToolRegistry::get`, `Tool::execute`,
     `ToolResult { output, error, metadata }`.
   - `executor/src/governor/scorer.rs` — `Scorer::{new, record, score}`.
   - `executor/src/context/budget.rs` — `Budget::{from_context, would_overflow,
     estimate}`; `executor/src/context/compactor.rs` — `compact(&mut Vec<Message>,
     &Budget, &str) -> CompactionReport`.
   - `executor/src/phase/{result.rs,briefing.rs}` — `PhaseResult::{complete,
     budget_exceeded}`, `Artifacts`, `Briefing`, `Blocker`, `summarize_attempts`,
     `collect_working_files`.
5. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

There is no loop module. `execute_phase` does not exist. Every piece it composes
is built and tested (phase-01–06). `lib.rs` declares `ai`, `config`, `context`,
`error`, `governor`, `health`, `parser`, `phase`, `security`, `store`, `tools`.

`AiClient::chat(system_prompt, messages, tx, tools)` is **streaming**: it sends a
sequence of `AiEvent`s on `tx` and returns `Result<()>`. A single `chat` call
(one **turn**) emits zero-or-more `Token(String)`, optionally one
`ToolCallGeneric { id, name, args: Value, .. }`, and ends with `Done(TokenBreakdown)`
— or `Error(String)` on a backend failure.

`MockAiClient::new(Vec<String>)` pops **one** string per `chat` call and emits it
as a single `Token` (good for multi-turn text scripts). `MockAiClientEvents`
drains its **entire** script in one call, so it cannot script multiple turns. This
phase needs a per-call event script — see Spec task 7.

## Spec

Create an `executor/src/loop_/` module (name it to avoid the `loop` keyword —
`loop_`, `agent`, or `turn` is your call; `pub mod <name>;` in `lib.rs`). File
split within the module is your call. Re-export `execute_phase` and its
input/dependency types so they are reachable as `executor::<module>::*`.

### 1. System-prompt assembly (turn-cycle step 1)

A pure function composing the **three** prompt inputs **in this order**: the
embedded **executor contract**, the project **`STANDARDS.md`**, then the **phase
doc** (architect pre-injected). The local model reads none as files — the function
assembles them in-process from strings it is handed. The section delimiter /
heading rendering is your call; pin only the **inputs and their order**. Tested by
a pure unit test.

### 2. Inputs & dependencies

Group the loop's inputs and injected dependencies however reads cleanly (a
`PhaseInput` carrier for the prompt strings + `goal` + `acceptance_criteria`, and a
deps struct for the rest — your call). The loop must receive, explicitly (no
globals, no real clock):

- `&dyn AiClient`, `&ToolRegistry`.
- The prompt inputs (contract / standards / phase-doc strings) + the phase
  **`goal`** and **`acceptance_criteria`** (verbatim, for the briefing).
- A `Budget` (build via `Budget::from_context`).
- A **turn cap** (`max_turns: usize`).
- The **project root** (`&Path`) — passed to `collect_working_files`.
- The routed tool schemas to send to the model (`Vec<ToolSchema>` / `Option<&[…]>`).

`execute_phase` is `async` and returns `executor::error::Result<PhaseResult>`.

### 3. The turn cycle (steps 2–5)

Each loop iteration is one **turn** = one `chat` call and its (optional) single
tool dispatch:

1. **Budget (step 2).** Before the `chat` call, if `budget.would_overflow(system,
   &messages)`, run `compactor::compact(&mut messages, &budget, system)`. If it
   **still** overflows after compaction, stop with **`budget_exceeded`** (§5).
   Never evict the system prompt (the compactor already guarantees this — do not
   re-implement).
2. **Call the model (step 3).** Create an `mpsc::unbounded_channel::<AiEvent>()`,
   call `client.chat(system, messages.clone(), tx, tools)`, and **drain `rx`**:
   - `Token(s)` → append to this turn's completion text.
   - `ToolCallGeneric { name, args, .. }` → record the **native** call for this
     turn: `ToolCall { name, arguments: args, origin: Origin::Native }`. (At most
     one is expected; if several arrive, keep the **first**, mirroring the parser's
     first-call rule.)
   - `Done(_)` → the turn's model output is complete; stop draining.
   - `Error(e)` → a backend/infra failure: **return `Err`** (an
     `executor::error::Error`), not a `PhaseResult`. This is not a model-visible
     outcome.
   Await the `chat` future's `Result<()>` too; an `Err` there is likewise
   infra → propagate.
3. **Output → `ToolCall` (step 4).**
   - If a **native** call arrived this turn, use it directly — **do not** run the
     text parser (the backend already structured it).
   - Otherwise run `parser::parse(&completion, registry)`:
     - `NoToolCall` → the model produced a final answer with no tool call: the
       phase is **complete** (§5).
     - `Found(tool_call)` → dispatch it.
     - `Failed(failure)` → **do not dispatch**. Append `failure.feedback` to
       `messages` as a new `user` message (repair guidance) and **continue** to the
       next turn. (Also push the assistant's raw `completion` as an `assistant`
       message first, so the model sees its own attempt followed by the feedback.)
4. **Dispatch (step 5).** `registry.get(&tool_call.name)`:
   - Missing tool → synthesize a `ToolResult`-style failure message fed back to the
     model (a model-visible outcome, **not** `Err`); record it as a failed attempt
     and continue. (The parser validates names against the registry, so a *parsed*
     call is known; a **native** call is not parser-validated, so this guard is
     load-bearing for the native path — pin a test.)
   - Present tool → `tool.execute(tool_call.arguments.clone()).await`. Map the
     `Result<ToolResult>`: `Ok(r)` with `r.error == None` → success; `Ok(r)` with
     `Some(error)` or `Err(_)` → failed. Append the assistant tool-call message and
     the tool-result message to `messages` for the next turn.
5. **Record.** `scorer.record(&tool_call.name, succeeded)`. Push a
   `ToolCallSnapshot { tool, arguments, succeeded }` onto a `recent_tool_calls:
   VecDeque<ToolCallSnapshot>` (this is the same structure phase-06's
   `summarize_attempts` / `collect_working_files` consume, and what 07c's hard-fail
   detector will read). Increment the turn counter.
6. **Turn cap.** After a dispatched turn, if `turns >= max_turns`, stop with
   **`budget_exceeded`** (§5). (Turn exhaustion and context overflow are both
   `budget_exceeded` — architecture line 175.)

### 4. Native vs. text dispatch are one path

The only difference between a native and a text-extracted call is **how the
`ToolCall` is obtained** (event vs. parser); everything after (registry lookup,
`execute`, scoring, snapshot, message append) is identical code. Do not fork the
post-construction path. `origin` is recorded on the `ToolCall` for the session log
(07b) and telemetry (08); it does **not** branch dispatch.

### 5. Termination & `PhaseResult`

- **Complete:** `parser::parse` returned `NoToolCall`. Return
  `PhaseResult::complete(Artifacts { files_changed: vec![], diff: String::new(),
  command_outputs: CommandOutputs::default(), update_log: <minimal summary> })`.
  This phase does **not** populate `files_changed` / `diff` / `command_outputs`
  (07d) — leave them empty/default.
- **Budget exceeded:** context overflow after compaction, or `turns >= max_turns`.
  Assemble a **budget briefing** and return `PhaseResult::budget_exceeded(briefing,
  artifacts)` with the same empty artifacts. The briefing:
  - `goal` / `acceptance_criteria` — verbatim from the input.
  - `diagnostics: vec![]` — the verifier is 07c; a budget stop carries none yet.
  - `working_files: collect_working_files(&recent_tool_calls, project_root)`.
  - `what_was_tried: summarize_attempts(&recent_tool_calls)`.
  - `current_blocker: Blocker::BudgetExceeded`.
  - `budget_remaining` — a caller-rendered line, e.g. `"0 of {max_turns} turns
    remaining"` (turn cap) or `"context budget exhausted"` (overflow).
- `update_log` — a minimal factual line for now (e.g. `"Executor run: {status}
  after {turns} turn(s)."`). Rich Update-Log rendering is not this phase's job.

The status↔briefing invariant is already enforced by the phase-06 constructors —
use them; do not hand-build `PhaseResult`.

### 6. Error model

- Backend/infra failures (`AiEvent::Error`, a `chat` `Err`, a channel error) →
  `executor::error::Error` via `?`. Add a new `Error` variant only if none fits
  (note it in "Notes for review"; do **not** edit unrelated error sites).
- Model-visible outcomes (parse failure, unknown tool, failed tool execution) →
  fed back into `messages`, **never** `Err`. These are normal turns the model
  adapts to.
- No `.unwrap()` / `.expect()` / `panic!()` in the loop (test code exempt).

### 7. Test mock (authorized infra)

The existing `MockAiClientEvents` drains its whole script in one `chat` call, so it
cannot drive a multi-turn loop with native events. **You may add a per-call
event-scripted mock to `executor/src/ai/testing.rs`** (e.g. take `Vec<Vec<AiEvent>>`
— one inner vec per `chat` call — and pop one inner vec per call). This is test
support, not a dependency. Keep `MockAiClient` / `MockAiClientEvents` as they are
(other tests use them); add alongside.

## Acceptance criteria

- [ ] `executor/src/<module>/` exists with `execute_phase` (async, returns
      `Result<PhaseResult>`) and the prompt-assembly function; `pub mod` in
      `lib.rs`; `execute_phase` reachable as `executor::<module>::execute_phase`.
- [ ] System-prompt assembly emits the three inputs in **contract → standards →
      phase-doc** order (pure-function test).
- [ ] A turn whose model output is `NoToolCall` returns `status == Complete` with
      `briefing == None`.
- [ ] A **native** `AiEvent::ToolCallGeneric` is dispatched as `Origin::Native`
      **without** invoking the text parser, through the same path as a parsed call.
- [ ] A **text** tool call is parsed and dispatched; a `ParseFailure` feeds
      `failure.feedback` back as a user message and the loop continues (no dispatch
      that turn).
- [ ] An **unknown tool name** (reachable via the native path) yields a
      model-visible failure fed back into `messages`, **not** an `Err`.
- [ ] `turns >= max_turns` returns `status == BudgetExceeded` with `briefing ==
      Some(_)`, `Blocker::BudgetExceeded`, `goal` echoed, `what_was_tried`
      non-empty.
- [ ] A budget that overflows after compaction returns `BudgetExceeded`; a budget
      with headroom runs compaction-free.
- [ ] `AiEvent::Error` (or a `chat` `Err`) makes `execute_phase` return `Err`, not
      a `PhaseResult`.
- [ ] No new dependency; no `tracing`; no session-log / verifier / hard-fail /
      diff / final-command-set code (those are 07b–07d).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic: `MockAiClient` / the new per-call events mock for the
model, a real `ToolRegistry` over a `tempfile::TempDir` `Scope` (mirror
`parser/mod.rs`'s `test_registry`), an injected turn cap and `Budget`. No network,
no real clock. Assert **both** what the loop returned **and** what it sent the
model (`messages` / `MockCall` inspection), per STANDARDS §3.1. Pin negatives.

**Prompt assembly:**
- `assembles_system_prompt_in_contract_standards_phase_order`.

**Termination:**
- `no_tool_call_first_turn_completes_immediately` — text-only first turn → Complete,
  zero dispatches.
- `tool_call_then_no_tool_call_completes` — one dispatched turn, then a text-only
  turn → Complete.
- `complete_result_has_no_briefing` (invariant at the loop level).
- `turn_cap_returns_budget_exceeded_with_briefing` — model always calls a tool →
  BudgetExceeded, `Blocker::BudgetExceeded`, briefing present.
- `budget_overflow_after_compaction_returns_budget_exceeded` — tiny ceiling.
- `budget_with_headroom_runs_without_compaction`.

**Native seam:**
- `native_tool_call_event_dispatches_as_origin_native`.
- `native_tool_call_skips_text_parser` — the completion text is *also* present but
  the native event wins; assert the native args were dispatched.
- `native_unknown_tool_feeds_failure_not_err` (**negative** — the native path is
  unvalidated, so the loop's own guard must catch it).

**Text seam:**
- `text_tool_call_is_parsed_and_dispatched`.
- `parse_failure_feeds_feedback_and_continues` — assert a `user` message containing
  `failure.feedback` was appended and the loop kept going.

**Dispatch / scoring:**
- `scorer_records_success_and_failure` — drive one succeeding and one failing tool
  call; assert `scorer.score` reflects both.
- `failed_tool_execution_is_model_visible_not_err`.

**Error model:**
- `ai_event_error_propagates_as_err`.

**Briefing assembly:**
- `budget_briefing_carries_goal_and_attempts` — `briefing.goal == input.goal`;
  `what_was_tried` non-empty.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. `execute_phase` is a
> library entry the MCP server exposes in M5; here it is composition logic
> exercised by `MockAiClient` integration tests. (The first real end-to-end —
> a live local model over the MCP boundary — lands in M5.)

## Authorizations

- [x] **May create** `executor/src/<loop-module>/**`; **may modify**
      `executor/src/lib.rs` (the `pub mod`) and `executor/src/ai/testing.rs` (add
      the per-call events mock — test infra only).
- [ ] **No new dependencies**; no `tracing`. (`tokio::sync::mpsc` and
      `async_trait` are already in the workspace via `ai`.)
- [ ] May **NOT** edit `verifier.rs`, `hard_fail.rs`, `scorer.rs`, the parser, the
      tools, the phase types, `Cargo.toml`, `docs/architecture.md`, `STANDARDS.md`,
      `WORKFLOW.md`, or another phase doc.
- [ ] May **NOT** add an `executor::error::Error` variant without noting it in
      "Notes for review" (a borderline new-variant call the reviewer should see).

## Out of scope

- **Session log** (redact → log, the `SessionEvent` kinds) — **07b**. The loop runs
  correct and silent here; logging is a side effect woven in next.
- **Verifier retry** (post-edit `verifier::verify`, `Baseline` author/ambient
  partition, diagnostic feedback), **hard-fail detection** (`hard_fail::evaluate`,
  the `recent_verifier_error_counts`), the **hard_fail briefing** (`Blocker::
  HardFail`, populated `diagnostics`), and the **read-before-edit invariant** —
  **07c**. (Build `recent_tool_calls` here; 07c reads it.)
- **Final command set** (`{FORMAT,BUILD,LINT,TEST}_COMMAND`), **diff generation**,
  `files_changed` / `command_outputs` population — **07d**. Leave them
  empty/default.
- **`PhaseRun` telemetry** — phase-08.
- **The MCP server wiring** of `execute_phase` — M5.
- **Resume / `continue_phase`** — architecture candidate, not committed.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md (test
fixtures embed `<tool_call>`/`<think>` close-tags + escaped quotes inside
JSON-in-Rust string literals).

Creating `executor/src/agent/` with `prompt.rs` (system-prompt assembly) and
`mod.rs` (`PhaseInput` + `LoopDeps` + `execute_phase` turn loop). Adding a
per-call event-scripted mock (`MockAiClientScript`) to `ai/testing.rs` for
multi-turn native-event tests. Backend errors map to `Error::Backend` (no new
error variant). Then the integration test suite per the phase doc's test plan.

### Update — 2026-05-29 (complete)

**Summary:** Added `executor/src/agent/` (`pub mod agent;` in `lib.rs`).
`prompt.rs`: `assemble_system_prompt` composing contract → standards → phase-doc.
`mod.rs`: `PhaseInput` (prompt strings + verbatim goal/acceptance), `LoopDeps`
(client / registry / tools / budget / max_turns / project_root), and the async
`execute_phase` turn loop — budget check + `compactor::compact` on overflow, a
`chat` call drained over an unbounded channel (`Token` accumulates completion
text, the first `ToolCallGeneric` becomes a `ToolCall { origin: Origin::Native }`,
`Done` ends the turn, `Error` → `Err(Error::Backend)`), then native-or-`parse`
into a `ToolCall` dispatched through one shared path (`registry.get` →
`Tool::execute`, mapping a missing tool / `error` / `Err` to a model-visible
failure fed back into `messages`), `Scorer::record` + a `ToolCallSnapshot`, and
termination: `NoToolCall` → `complete`; `turns >= max_turns` or post-compaction
overflow → `budget_exceeded` with a `Blocker::BudgetExceeded` briefing built from
`summarize_attempts` / `collect_working_files`. `files_changed` / `diff` /
`command_outputs` left empty/default (07d); no logging (07b); no
verifier/hard-fail (07c). Added `MockAiClientScript` (one event script per `chat`
call) to `ai/testing.rs`. No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.64s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.01s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 440 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(440 = 425 prior + 14 loop tests + 1 prompt test.)

**Spec-pinned literal grep** (the native seam):

```
grep -n 'Origin::Native\|ToolCallGeneric' executor/src/agent/mod.rs
100:  AiEvent::ToolCallGeneric { name, args, .. } => {
105:      origin: Origin::Native,
313:  AiEvent::ToolCallGeneric { ...quad (test fixture)
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. `execute_phase` is a
library entry the MCP server exposes in M5; here it is composition logic
exercised by `MockAiClient*` integration tests (first live end-to-end is M5).

**Files changed:**
- `executor/src/lib.rs` — added `pub mod agent;`
- `executor/src/agent/prompt.rs` — new: `assemble_system_prompt` + 1 test
- `executor/src/agent/mod.rs` — new: `PhaseInput`, `LoopDeps`, `execute_phase`,
  helpers + 14 tests
- `executor/src/ai/testing.rs` — added `MockAiClientScript` (per-call event mock)

**New tests:** `assembles_system_prompt_in_contract_standards_phase_order`;
`no_tool_call_first_turn_completes_immediately`, `complete_result_has_no_briefing`,
`tool_call_then_no_tool_call_completes`,
`native_tool_call_event_dispatches_as_origin_native`,
`native_tool_call_skips_text_parser`, `native_unknown_tool_feeds_failure_not_err`,
`text_tool_call_is_parsed_and_dispatched`,
`parse_failure_feeds_feedback_and_continues`,
`turn_cap_returns_budget_exceeded_with_briefing`,
`budget_briefing_carries_goal_and_attempts`,
`budget_overflow_after_compaction_returns_budget_exceeded`,
`budget_with_headroom_runs_without_compaction`,
`tool_outcomes_distinguish_success_and_failure`, `ai_event_error_propagates_as_err`.

**Commits:** (pending — committed below)

**Notes for review:**
- **Scorer has no observable surface in 07a.** `scorer.record` is wired per spec
  step 5, but nothing consumes the score yet (routing/governor is a later concern)
  and `execute_phase` does not expose the `Scorer`. So the success/failure
  computation is tested via its *other* consumer — the `ToolCallSnapshot` →
  `what_was_tried` path (`tool_outcomes_distinguish_success_and_failure`), which is
  observable through the briefing. Flagging in case the architect would rather drop
  the premature `scorer.record` until its consumer lands.
- The drain awaits `chat` to completion, then reads buffered events
  (`rx.recv().await` until the sender is dropped). Correct for the unbounded
  channel; if 07b/M5 needs *live* progress mid-stream it can drain concurrently —
  not needed here.
- `serde_json::to_string(&tc.arguments)` keeps an `unwrap_or_else(|| "{}")`
  fallback for the infallible `Value` serialization (STANDARDS §2.1, same pattern
  as phase-06 `compact_args`).
- Native and text dispatch share one post-construction path (`dispatch` +
  `append_tool_exchange`); `origin` is recorded but does not branch dispatch.

verification: fmt OK · clippy OK · tests 440 passed · build OK

### Review verdict — 2026-05-29

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude Code (direct) — pre-routed off opencode (`<tool_call>` /
  escaped-quote test fixtures).
- **Scope deviations:** none. Implemented exactly to spec; `diff` /
  `files_changed` / `command_outputs` correctly left empty/default (07d), no
  logging (07b), no verifier/hard-fail (07c).
- **Calibration:** one note, not yet a fold (one occurrence). The spec mandated
  `scorer.record` in step 5, but 07a has no consumer of the score and does not
  expose the `Scorer`, so it is currently dead computation (the executor flagged
  this honestly and covered success/failure via the observable `ToolCallSnapshot`
  → `what_was_tried` path). **Decision: keep it** — the `Scorer` is loop-running
  state whose natural write-site is where outcomes occur, and **07c will wire the
  reader** (governor tool-selection biasing) so it stops being dead. Action carried
  to 07c's spec. If 07c does not consume it, drop it there. (If a *second*
  spec mandates a write with no consumer, fold a "derive/record intentionally"
  rule into STANDARDS § "Derive intentionally", which already warns the analogous
  case for serde derives.)
- Re-ran all four gates independently (fmt/build/clippy clean, 440 passed);
  spot-checked tests are real — `budget_overflow…` asserts zero model calls,
  `native_unknown_tool…` inspects the fed-back tool-result content,
  `ai_event_error…` asserts `Err`.
