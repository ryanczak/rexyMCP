# Phase 02: Truncation-aware empty-completion recovery

**Milestone:** M23 — Truncation & Empty-Completion Recovery
**Status:** review
**Depends on:** M22 phase-01 (the `NoToolCall` empty branch + `consecutive_empty_completions` counter this phase extends). Independent of M23 phase-01 in code (different file), but dispatched after it so the raised `max_tokens` default reduces how often the truncation path fires.
**Estimated diff:** ~170 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Act on `finish_reason`. Two coupled changes to the `NoToolCall` arm of the agent
loop:

- **#1 (truncation routing):** a turn the backend cut off at the output-token
  ceiling (`finish_reason == "length"`) with no tool call is **not** a deliberate
  completion — the model ran out of budget mid-stream. Route it to a
  truncation-specific recovery nudge instead of letting it fall through to the
  gate/completion path (where it is mis-read as "the model is done").
- **#3 (no-think escalation):** after ≥ 2 consecutive empty completions (below the
  M22 hard-fail threshold of 3), escalate the empty-recovery feedback from the
  standard "emit a tool call" nudge to a no-reasoning directive — the model is
  burning the turn inside `<think>` and emitting nothing, so tell it to skip
  reasoning and act.

Both reduce to: in the existing empty branch, broaden the guard to also fire on a
truncated turn, and **select the recovery feedback by cause**. `finish_reason` is
already captured for metrics (`mod.rs:414`) but never acted on; this phase retains
it per-turn and uses it.

## Architecture references

Read before starting (re-locate by the quoted anchor text, not the line number —
M23 phase-01 does not touch this file, so the refs should be exact, but verify):

- `executor/src/agent/mod.rs`:
  - per-turn `completion` / `native_call` declaration (**392–393**) — where the
    new per-turn `turn_finish_reason` is declared.
  - the `AiEvent::Completion { finish_reason, model }` arm (**407–420**) — where
    `finish_reason` is captured; `turn_finish_reason` is set here.
  - the `NoToolCall` empty branch (**516–623**) — the guard
    `if post_think.trim().is_empty()` (523), the stall check (524–566), the
    parse-failure injection (568–585), the budget-exceeded block (586–619), and
    the post-branch reset `consecutive_empty_completions = 0;` (621–623).
- `executor/src/parser/feedback.rs` — `format_no_match` (43) is the pattern + the
  delegate for the first-empty case; the new `format_truncated` /
  `empty_recovery_feedback` helpers live here.
- `executor/src/ai/testing.rs` — `MockAiClientScript` (102): `new(Vec<Vec<AiEvent>>)`
  scripts one event vector per turn, so a turn can emit
  `AiEvent::Completion { finish_reason: Some("length".into()), model: None }`
  alongside its tokens. (See `length_finish_rate_is_fraction_of_length_finishes`
  in `agent/tests.rs` ~2907 for the scripting shape.)

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### `finish_reason` is captured but discarded after metrics — `mod.rs:407–420`

```rust
AiEvent::Completion {
    finish_reason,
    model,
} => {
    if let Some(m) = model {
        metrics.served_model = Some(m);
    }
    if let Some(fr) = finish_reason {
        metrics.total_finishes += 1;
        if fr == "length" {
            metrics.length_finishes += 1;
        }
    }
}
```

`finish_reason` is consumed by the `if let` and never seen by the `NoToolCall`
arm. This phase clones it into a per-turn variable first.

### The empty branch — `mod.rs:516–623`

```rust
match parse(&completion, deps.registry) {
    ParseResult::NoToolCall => {
        let post_think = crate::parser::strip_think_blocks(&completion);
        if post_think.trim().is_empty() {
            consecutive_empty_completions += 1;
            if let Some(signal) = crate::governor::hard_fail::check_empty_completion_stall(
                consecutive_empty_completions,
                deps.governor.empty_completion_threshold,
            ) {
                // … log HardFail; log_session_end; emit_phase_run; build_artifacts;
                //    return hard_fail_result(…) …
            }
            metrics.parse_failures += 1;
            let failure = crate::parser::ParseFailure {
                raw: completion.clone(),
                detected_format: None,
                candidates: vec![],
                feedback: crate::parser::feedback::format_no_match(&completion),
            };
            log_event(/* … SessionEvent::ParseFailed … */);
            messages.push(assistant_text(&completion, turns));
            messages.push(user_text(&failure.feedback, turns));
            if turns >= deps.max_turns {
                // … return budget_exceeded_result(…) …
            }
            continue;
        }
        // Reaching here means the completion had real post-think text
        // (not empty/think-only) — reset the empty-completion counter.
        consecutive_empty_completions = 0;
        // Step 8 — run the final gate set BEFORE declaring completion …
    }
    // … ToolCall arm below …
}
```

The bug this phase fixes: a **truncated** turn (`finish_reason == "length"`)
usually has *non-empty* reasoning text, so `post_think.trim().is_empty()` is false,
the branch is skipped, and the turn falls through to "run the final gate set …
declare completion" — exactly the mis-read seen on netviz turns 12/14/15.

## Spec

### Task 1 — Retain `finish_reason` per turn (`mod.rs`)

Declare a per-turn variable alongside `completion` / `native_call` (~392–393):

```rust
let mut completion = String::new();
let mut native_call: Option<ToolCall> = None;
let mut turn_finish_reason: Option<String> = None;
```

In the `AiEvent::Completion` arm, capture it **before** the metrics `if let`
consumes `finish_reason`:

```rust
AiEvent::Completion {
    finish_reason,
    model,
} => {
    if let Some(m) = model {
        metrics.served_model = Some(m);
    }
    if finish_reason.is_some() {
        turn_finish_reason = finish_reason.clone();
    }
    if let Some(fr) = finish_reason {
        metrics.total_finishes += 1;
        if fr == "length" {
            metrics.length_finishes += 1;
        }
    }
}
```

(Guard with `is_some()` so a later `Completion` event carrying `None` in the same
turn — see the two-event mock in `length_finish_rate_is_fraction_of_length_finishes`
— does not clobber a `length` already seen. The **last non-None** finish_reason of
the turn wins, which matches how `length_finishes` already counts.)

### Task 2 — Recovery feedback helpers (`parser/feedback.rs`)

Add two helpers next to `format_no_match`:

```rust
/// Feedback for a turn the backend cut off at the output-token ceiling
/// (`finish_reason == "length"`) before a tool call appeared — the model ran out
/// of output budget mid-stream, so its stub is not a deliberate completion.
pub fn format_truncated(response_excerpt: &str) -> String {
    // char-safe truncation (do not byte-slice — multibyte boundaries panic).
    let excerpt: String = response_excerpt.chars().take(200).collect();
    format!(
        "Your previous response was cut off at the output-token limit before you \
         emitted a tool call. Do not keep reasoning — emit a single tool call in \
         the expected format now, and keep any reasoning brief.\n\
         Excerpt: {excerpt}"
    )
}

/// Escalating feedback for consecutive empty completions: the first empty gets the
/// standard "emit a tool call" nudge; a second or later empty escalates to a
/// no-reasoning directive, since the model is spending the turn inside `<think>`
/// and emitting nothing.
pub fn empty_recovery_feedback(consecutive_empty: usize, response_excerpt: &str) -> String {
    if consecutive_empty >= 2 {
        "You have returned multiple empty responses in a row. Do NOT write any \
         <think> reasoning this turn. Respond with exactly one tool call in the \
         expected format and nothing else."
            .to_string()
    } else {
        format_no_match(response_excerpt)
    }
}
```

**Do not** modify `format_no_match`. (It has a pre-existing
`&response_excerpt[..200]` byte-slice that can panic on a multibyte boundary —
note it in Notes-for-review as an adjacent latent bug, but **do not fix it** here;
that is out of scope per the hard rules. The new `format_truncated` uses a
char-safe `chars().take(200)` so it does not add a second panic path.)

### Task 3 — Route truncated + empty turns by cause (`mod.rs`)

Modify the empty branch so it fires on **either** a truncated turn **or** an empty
one, and picks feedback by cause. Replace the
`if post_think.trim().is_empty() { … }` block with:

```rust
let truncated = turn_finish_reason.as_deref() == Some("length");
if truncated || post_think.trim().is_empty() {
    // Unproductive turn — never a completion. Pick recovery feedback by cause.
    let feedback = if truncated {
        // Cut off at the output ceiling mid-stream. NOT an empty turn — leave the
        // empty-completion counter untouched (it tracks blank/think-only turns).
        crate::parser::feedback::format_truncated(&completion)
    } else {
        consecutive_empty_completions += 1;
        if let Some(signal) = crate::governor::hard_fail::check_empty_completion_stall(
            consecutive_empty_completions,
            deps.governor.empty_completion_threshold,
        ) {
            // … unchanged: log HardFail; log_session_end; emit_phase_run;
            //    build_artifacts; return hard_fail_result(…) …
        }
        crate::parser::feedback::empty_recovery_feedback(
            consecutive_empty_completions,
            &completion,
        )
    };
    metrics.parse_failures += 1;
    let failure = crate::parser::ParseFailure {
        raw: completion.clone(),
        detected_format: None,
        candidates: vec![],
        feedback,
    };
    log_event(/* … SessionEvent::ParseFailed { failure: failure.clone() } … */);
    messages.push(assistant_text(&completion, turns));
    messages.push(user_text(&failure.feedback, turns));
    if turns >= deps.max_turns {
        // … unchanged budget_exceeded block …
    }
    continue;
}
// Reaching here means a productive, non-truncated completion with real post-think
// text — reset the empty-completion counter (unchanged).
consecutive_empty_completions = 0;
```

Notes for the executor:

- The `return hard_fail_result(…)` inside the `else` arm of `let feedback = …`
  diverges — a `return` in a `let` initializer is valid Rust. Keep that block
  byte-for-byte as it is today (including `last_author_diagnostics.clone()`); only
  its surrounding context moves.
- A turn that is **both** truncated and empty takes the `truncated` arm (cause
  precedence: truncation), so it does not increment the empty stall counter. This
  is intentional — a length-cut turn is a different failure mode than a blank EOS.
- The truncation path deliberately has **no new hard-fail terminator** (see § Out
  of scope). A repeated-truncation loop is bounded by `deps.max_turns`
  (`budget_exceeded`), and the empty endgame is still caught by the M22 stall.

### Task 4 — Unit tests (`parser/feedback.rs`)

In the `#[cfg(test)] mod tests` block:

- `format_truncated_tells_model_it_was_cut_off` — contains `"cut off"` and
  `"tool call"`.
- `empty_recovery_feedback_first_empty_is_standard_nudge` —
  `empty_recovery_feedback(1, "x")` contains `"No tool call was found"` (i.e. it
  delegates to `format_no_match`).
- `empty_recovery_feedback_escalates_after_two` — `empty_recovery_feedback(2, "x")`
  contains `"Do NOT write"` and `"nothing else"`, and does **not** equal the
  count-1 message (refutes a non-escalating impl).

### Task 5 — Integration tests (`executor/src/agent/tests.rs`)

Use `MockAiClientScript` with per-turn `AiEvent::Completion { finish_reason, model }`.
Model the harness wiring on the existing `length_finish_rate_*` tests and the M22
empty-completion tests (`empty_completions_hard_fail_at_threshold`).

- `truncated_turn_is_not_treated_as_completion` (the #1 pin) — turn 1 emits real
  reasoning **tokens** + `Completion { finish_reason: Some("length"), model: None }`
  and **no** tool call; turn 2 emits a real tool call that drives a clean finish.
  Assert the run does **not** finish on turn 1 (it is re-prompted) and reaches the
  turn-2 completion — e.g. the model was called **twice** and the result is the
  clean-completion status, not a turn-1 `Complete`. This test **fails before** the
  Task 3 change (turn 1 falls through to completion) and passes after.
- `repeated_truncation_reaches_turn_cap_not_completion` — every scripted turn is
  truncated (`finish_reason == "length"`, no tool call) with `max_turns` small;
  assert the result is `budget_exceeded` (bounded loop), **not** `Complete`.

**Pinned negatives:**

- `finish_reason == None` (or `Some("stop")`) on a non-empty completion must still
  reach the gate/completion path — covered by the **unmodified** pre-existing
  completion tests (e.g. `gate_failure_loops_until_gates_pass`, which scripts a
  plain completion with no `length` finish). Those must pass unchanged; if any
  needs editing, that signals Task 3 broke the normal completion path — stop and
  file a blocker.
- The M22 empty-completion tests (`empty_completions_hard_fail_at_threshold`,
  `single_empty_completion_then_recovers_does_not_hard_fail`) must pass unchanged —
  the empty arm's counter + stall logic is preserved verbatim, only the feedback
  string is now selected via `empty_recovery_feedback` (count 1 → the same
  `format_no_match` text those tests' empties produce). If a counter/stall test
  breaks, the empty arm was altered beyond feedback selection — stop and file a
  blocker.

## Acceptance criteria

- [ ] `format_truncated_tells_model_it_was_cut_off` passes.
- [ ] `empty_recovery_feedback_first_empty_is_standard_nudge` passes.
- [ ] `empty_recovery_feedback_escalates_after_two` passes.
- [ ] `truncated_turn_is_not_treated_as_completion` passes (turn 1 truncation is
      re-prompted, not completed).
- [ ] `repeated_truncation_reaches_turn_cap_not_completion` passes
      (`budget_exceeded`, not `Complete`).
- [ ] All pre-existing tests pass **unmodified** — in particular the M22
      empty-completion tests and the gate/completion tests.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings), `cargo clippy
      --all-targets --all-features -- -D warnings`, `cargo test` all exit 0.

## Test plan

- Three `feedback.rs` unit tests (truncation text, first-empty delegate, escalation).
- Two `tests.rs` integration tests (truncation-not-completion, repeated-truncation-
  bounded) plus the unmodified M22 + gate tests as the pinned negatives.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. `execute_phase` is the
library entry point; the integration tests exercise the truncation/empty recovery
path end-to-end against `MockAiClientScript` (scripting `finish_reason` per turn,
asserting both what the loop did — re-prompt vs complete — and that it re-asked the
model). The follow-up live netviz e2e run (post-M23) is the real-world check the
user drives, not a phase artifact.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `architecture.md` edit. All
changes are additive (a new per-turn variable, two new pure feedback fns, a
broadened guard with cause-selected feedback).

## Out of scope

- **A `TruncationStall` hard-fail terminator** mirroring `EmptyCompletionStall`.
  Deferred deliberately (README § Scope): recover first. A repeated-truncation loop
  is already bounded by the turn cap (`budget_exceeded`), and the empty endgame is
  caught by the M22 stall. Add the terminator only if the post-M23 e2e shows
  truncation loops persist after the raised `max_tokens` ceiling + the nudge.
- **Fixing `format_no_match`'s byte-slice panic risk** — adjacent latent bug; note
  it in Notes-for-review, do not fix (hard rule: don't widen scope).
- **Native (structured) empty tool calls** — the observed failure is the
  text/parse path; native-call emptiness is not in evidence (same boundary M22
  phase-01 drew).
- **`max_tokens` config** — that is M23 phase-01.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-18 14:05 (started)

Executor started phase 02: truncation-aware empty-completion recovery. Implementing Tasks 1–5: retain `finish_reason` per turn, add `format_truncated` / `empty_recovery_feedback` helpers, route truncated + empty turns by cause, unit tests, integration tests.

### Update — 2026-06-18 14:15 (complete)

**Summary:** Implemented truncation-aware empty-completion recovery. Two coupled changes to the `NoToolCall` arm: (1) a turn with `finish_reason == "length"` and no tool call is now routed to a truncation-specific recovery nudge instead of falling through to the gate/completion path; (2) after ≥ 2 consecutive empty completions, the empty-recovery feedback escalates from the standard "emit a tool call" nudge to a no-reasoning directive. `finish_reason` is retained per-turn in a new `turn_finish_reason` variable, and recovery feedback is selected by cause via `format_truncated` / `empty_recovery_feedback` helpers in `parser/feedback.rs`.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
(no output — clean)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(no output — clean)

cargo test 2>&1 | tail -30
test result: ok. 855 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.06s
```

**End-to-end verification:**

N/A — phase ships no runtime-loadable artifact. `execute_phase` is the library entry point; the integration tests exercise the truncation/empty recovery path end-to-end against `MockAiClientScript` (scripting `finish_reason` per turn, asserting both what the loop did — re-prompt vs complete — and that it re-asked the model). The follow-up live netviz e2e run (post-M23) is the real-world check the user drives, not a phase artifact.

**Files changed:**
- `executor/src/agent/mod.rs` — declared per-turn `turn_finish_reason`, captured it in `AiEvent::Completion` arm, broadened the empty-branch guard to also fire on truncated turns, selected feedback by cause
- `executor/src/parser/feedback.rs` — added `format_truncated` and `empty_recovery_feedback` helpers, added 3 unit tests
- `executor/src/agent/tests.rs` — added 2 integration tests (`truncated_turn_is_not_treated_as_completion`, `repeated_truncation_reaches_turn_cap_not_completion`)
- `docs/dev/milestones/M23-truncation-recovery/phase-02-truncation-recovery.md` — status flip to `review`, Update Log entries
- `docs/dev/milestones/M23-truncation-recovery/README.md` — phase table row status flip to `review`

**New tests:**
- `format_truncated_tells_model_it_was_cut_off` in `executor/src/parser/feedback.rs`
- `empty_recovery_feedback_first_empty_is_standard_nudge` in `executor/src/parser/feedback.rs`
- `empty_recovery_feedback_escalates_after_two` in `executor/src/parser/feedback.rs`
- `truncated_turn_is_not_treated_as_completion` in `executor/src/agent/tests.rs`
- `repeated_truncation_reaches_turn_cap_not_completion` in `executor/src/agent/tests.rs`

**Commits:**
- `6608df3` — feat: truncation-aware empty-completion recovery

**Notes for review:**
- Adjacent latent bug noted (not fixed per hard rules): `format_no_match` uses `&response_excerpt[..200]` byte-slicing which can panic on multibyte boundaries. The new `format_truncated` uses char-safe `chars().take(200)` to avoid adding a second panic path.
- All pre-existing tests pass unmodified: M22 empty-completion tests (`empty_completions_hard_fail_at_threshold`, `single_empty_completion_then_recovers_does_not_hard_fail`) and gate/completion tests all pass.
- Grep verification: `format_truncated` and `empty_recovery_feedback` confirmed present in `feedback.rs` at lines 60 and 75 respectively.

