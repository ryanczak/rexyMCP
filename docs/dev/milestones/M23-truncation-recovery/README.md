# M23 — Truncation & Empty-Completion Recovery

**Goal:** Give the executor a chance to *recover* from the truncation/empty-output
tail before it hard-fails. M22 made the empty-output spiral **terminate cleanly**
(a consecutive-empty counter → `EmptyCompletionStall` hard_fail at 3, instead of
burning 147 turns to the cap). M23 attacks the cause one layer up: raise the
per-turn output ceiling so a thinking model can finish its reasoning *and* emit a
tool call, and when the backend still cuts a turn off mid-stream, tell the model
exactly that instead of mis-reading the stub as a completion attempt.

**Status:** done (3/3 phases done, 2026-06-18)

**Depends on:** M22 (the `NoToolCall` empty branch + `consecutive_empty_completions`
counter this milestone extends), M18 (the `[models]` per-model override +
`resolve_for_model` path the new `max_tokens` knob rides), M19/M21 (the gate /
task-coverage feedback the truncation guard sits above).

## Why now

A fresh netviz e2e run (`google/gemma-4-26b-a4b-qat`, MEDIUM-tier, phase-03,
`session-phase-03-6a33e58c.jsonl`) hard-failed on the exact mechanism M22
phase-01 added the terminator for — but the session log shows the terminator is
firing on a *symptom*, not the root cause. Reconstructed from the per-turn
metrics (cumulative token deltas):

| turn | output Δ | raw | finish_reason |
|---|---|---|---|
| 12 | **+4096** | 11.8k chars of reasoning, no tool call | length (capped) |
| 14 | **+4096** | 12k chars reasoning, no tool call | length |
| 15 | **+4096** | 11k chars reasoning, no tool call | length |
| 16–18 | **+0** | `""` | stop (immediate EOS) |

Two findings:

1. **The model was being truncated mid-reasoning.** `max_tokens` is hardcoded to
   **4096** in `executor/src/ai/backends/openai.rs:110`. On turns 12/14/15 the
   model generated exactly 4096 output tokens of `<think>` reasoning and was cut
   off (`finish_reason == "length"`) *before* it reached the tool call. The
   context window was only **45% full** (`context_used ≈ 25k` of a 55,704 budget
   ceiling) — there was plenty of room; the per-turn output cap, not the context
   length, was the wall.

2. **A truncated turn is silently mis-read.** `finish_reason` is captured
   (`agent/mod.rs:414`) but only counted for the scorecard — nothing in the loop
   *acts* on it. A length-cut turn with non-empty reasoning text falls through the
   `NoToolCall` arm into the gate/completion path and is treated as a completion
   attempt, kicking off gate/coverage feedback. The model then collapses to
   0-token EOS responses (turns 16–18), which M22's `EmptyCompletionStall` finally
   terminates — three turns *after* the real failure.

The user's two prior observations corroborate the mechanism: a run that
"exhausted the turn budget" is the length-truncation arm running to the cap; a run
that "broke out after a compaction event, then fell back" is compaction evicting
the polluted truncated-reasoning turns (temporary recovery) before the same
unraised cap pulled it back in.

## Exit criteria

- The per-response output ceiling (`max_tokens`) is **configurable** in
  `rexymcp.toml` (`[executor] max_tokens`) and per-model overridable
  (`[models."<id>"] max_tokens`), defaulting to **8192** instead of the hardcoded
  4096.
- A `NoToolCall` turn the backend truncated at the output ceiling
  (`finish_reason == "length"`) is routed to a **truncation-specific** recovery
  nudge ("you were cut off — stop reasoning and emit a tool call"), **never**
  treated as a completion attempt.
- After ≥ 2 consecutive empty completions (below the M22 hard-fail threshold of
  3), the empty-recovery feedback **escalates** to a no-reasoning directive
  ("respond with exactly one tool call, no `<think>`").
- When `finish_reason` is absent (an endpoint that doesn't send it), behavior is
  unchanged — the corrective is best-effort, no regression.
- All pre-existing tests pass (every change additive / backward-compatible) except
  the mechanical call-site updates enumerated in each phase doc (the new
  `build_chat_body` / `OpenAiClient::new` / `ModelOverride` arguments).

## Architecture references

- `docs/architecture.md` § Status #23 (added at kickoff) and § Configuration
  (the `[executor] max_tokens` bullet).
- `executor/src/ai/backends/openai.rs` — `build_chat_body` (the hardcoded
  `"max_tokens": 4096` at line 110), `OpenAiClient` struct + `new` (132–166), the
  `build_chat_body` call in `chat` (~179).
- `executor/src/config.rs` — `ExecutorConfig` (245–306), `ModelOverride`
  (199–208), `resolve_for_model` (431–459).
- `executor/src/ai/mod.rs` — `make_client` (187), the second `OpenAiClient::new`
  call site.
- `mcp/src/runner.rs` — the production `OpenAiClient::new` (275) and the two
  `ModelOverride` test literals (592, 710).
- `executor/src/agent/mod.rs` — per-turn `completion`/`native_call` declaration
  (392–393), the `AiEvent::Completion` arm that captures `finish_reason`
  (407–420), the `NoToolCall` empty branch (516–623).
- `executor/src/parser/feedback.rs` — `format_no_match` (43); the home for the new
  truncation / escalating-empty feedback helpers.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Configurable `max_tokens` (config + backend + init template) ([phase-01-configurable-max-tokens.md](phase-01-configurable-max-tokens.md)) | done |
| 02 | Truncation-aware empty-completion recovery (finish_reason routing + no-think escalation) ([phase-02-truncation-recovery.md](phase-02-truncation-recovery.md)) | done |
| 03 | `SamplingParams` refactor + `format_no_match` fix ([phase-03-sampling-params-cleanup.md](phase-03-sampling-params-cleanup.md)) | done |

Dispatch in order and review-gate each. Phase 01 is config/backend plumbing
(mirrors the `temperature`/`seed` path); phase 02 edits the `NoToolCall` arm of
`agent/mod.rs`. They are independent in code (different files) but ordered so the
raised default ceiling from 01 reduces how often 02's truncation path fires in the
follow-up e2e run.

## Notes

### Scope decisions (2026-06-18, with the user)

- **Two phases, split on the config/loop seam** (the M18 phase-05/06 substrate→
  behavior precedent). `max_tokens` is pure config plumbing with a wide-but-shallow
  blast radius (every `build_chat_body`/`OpenAiClient::new` call site); the
  finish_reason recovery is a focused edit to one arm of the loop. Keeping them
  apart keeps each review surface a finite checklist.
- **Default `max_tokens` = 8192.** Doubles the prior 4096 — enough headroom for a
  full reasoning block + tool call on a thinking model, while keeping a runaway
  turn bounded. Per-project and per-model tunable. (`16384` was considered but lets
  a pathological/looping turn burn ~2× more tokens before being cut and grows
  context faster; `4096` kept the truncation-out-of-the-box.)
- **`max_tokens` is a ceiling carved out of the remaining context window** —
  `prompt + max_tokens` must fit in the model's context length. Phase 01 documents
  this for the operator but does **not** add runtime clamping/validation (the
  endpoint enforces its own limit; a clamp is a separate concern). Flagged as a
  possible follow-up if e2e shows a need.
- **Recover first, terminate later.** Phase 02 ships the *corrective* (route a
  truncated turn to a nudge, escalate empty feedback) but **no new terminator** for
  a truncation loop. Rationale: M22's `EmptyCompletionStall` already terminates the
  empty endgame, and a truncation loop now degrades to a bounded `budget_exceeded`
  at the turn cap (not the old unbounded 147× spiral, which was empty-driven and is
  already fixed). A dedicated `TruncationStall` signal mirroring
  `EmptyCompletionStall` is deferred until the follow-up e2e shows whether raising
  `max_tokens` + the nudge is enough on its own. (See phase-02 § Out of scope.)
- **No-think escalation is delivered as instruction text**, not a template flag.
  The executor talks to heterogeneous OpenAI-compatible endpoints (vLLM / LM Studio
  / Ollama) whose chat templates handle `<think>` server-side; the portable lever
  is the injected user message, not a per-backend prefill.

### Retrospective — 2026-06-18

**Outcome:** 3/3 phases **approved_first_try** (phases 01–02 executor
Qwen/Qwen3.6-27B-FP8; phase-03 executor Claude Code direct). Both gates of the
netviz truncation failure are now closed: the per-turn output ceiling is
configurable (default raised 4096 → 8192), and a `length`-truncated `NoToolCall`
turn is routed to a truncation nudge instead of being mis-read as a completion.
Phase-03 then retired the two calibration items the recovery work left behind (the
`too_many_arguments` allow and the `format_no_match` byte-slice panic). Commits
`5eec632` (phase-01 feat) / `6608df3` (phase-02 feat) / `eed0213` (phase-03 refactor).

**What worked.**
- **The config/loop seam split (M18 precedent) held again.** phase-01 was
  wide-but-shallow plumbing (every `build_chat_body`/`OpenAiClient::new` call
  site); phase-02 was a focused single-arm edit. Each review surface stayed a
  finite checklist, and neither phase's blast radius bled into the other's files
  — so phase-02's anchors were still exact at activation despite phase-01 landing
  between draft and dispatch.
- **Heavy pre-injection paid off on the loop edit.** phase-02 quoted the full
  `AiEvent::Completion` arm and the entire `NoToolCall` empty branch verbatim as
  the before/after shape, including the divergent `return hard_fail_result(…)`
  inside the `let feedback = …` initializer. The executor restructured a 50-line
  block (the largest single-arm churn this milestone) with zero bounces.
- **Pinned negatives did their job.** Both phases named the exact pre-existing
  tests that must pass unmodified (M22 empty-stall tests, gate tests) and the
  executor preserved the counter/stall logic verbatim — only the feedback string
  selection changed.

**Calibration data (no folds this milestone).**
- **`too_many_arguments` allow (phase-01, 1st occurrence).** The spec's "mirror
  `temperature`/`seed` **exactly**" instruction pushed `OpenAiClient::new` to 8
  positional args, tripping clippy's threshold (7) and requiring a function-scoped
  `#[allow(clippy::too_many_arguments)]`. Accepted as a spec-mandated consequence;
  the only alternative (a params-struct/builder refactor of the constructor) was
  out of the phase's authorized scope. **Data, not a trend** — but the pattern to
  watch: the sampling-knob constructor (`temperature`/`seed`/`max_tokens` and
  whatever knob M24+ adds next) is now at the lint ceiling, so the *next* knob
  added to `OpenAiClient::new` forces either a 2nd allow or the refactor. A future
  phase that collapses these into a `SamplingParams`/`GenerationConfig` struct
  would retire the allow and stop the recurrence pre-emptively. **Resolved in
  phase-03:** `SamplingParams { temperature, seed, max_tokens }` collapsed the
  three trailing args back to one, dropping `OpenAiClient::new` to 6 args and
  retiring the `#[allow]` (grep-confirmed gone). Never reached a 2nd occurrence.
- **`format_no_match` byte-slice panic, held out of scope a 2nd time.** Both
  M23 phases brushed `feedback.rs` and both correctly left the pre-existing
  `&response_excerpt[..200]` multibyte-boundary panic alone (phase-02's new
  `format_truncated` used char-safe `chars().take(200)`). **Resolved in phase-03:**
  replaced with `chars().take(200).collect::<String>()` and the `len()` guard with
  `chars().count()`, matching `format_truncated`; pinned by
  `format_no_match_handles_multibyte_boundary` (199 ASCII + `é` + more — panics
  under the old slice, passes after).

**Deferred / open.**
- **`TruncationStall` terminator** — deliberately not shipped (recover-first). The
  real test is the **follow-up live netviz e2e run** (user-driven): does the raised
  8192 ceiling + the truncation nudge keep the loop out of the truncation/empty
  endgame, or does a truncation loop still ride the turn cap to `budget_exceeded`?
  Add the terminator only if that run shows the loop persists.
- **`max_tokens` runtime clamp** vs the model's context length — noted in phase-01
  as a possible follow-up; the endpoint enforces its own limit today.
- **D8/D9 (server-authored bookkeeping)** — still deferred from M22, needs a
  design conversation before authoring.
