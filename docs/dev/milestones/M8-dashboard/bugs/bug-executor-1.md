# Executor bug 1: think-only completion treated as clean exit

**Severity:** major
**Status:** open
**Filed:** 2026-06-02
**Observed during:** M8 phase-02 dispatch (Qwen/Qwen3.6-35B-A3B-FP8)

## What's wrong

When a thinking-mode model (Qwen3, DeepSeek-R1, etc.) produces a response that
is *entirely* a `<think>…</think>` block with nothing after the closing tag, the
executor agent loop treats it as a clean `complete` exit.

The execution path is `agent/mod.rs:410`:

```rust
match parse(&completion, deps.registry) {
    ParseResult::NoToolCall => {
        log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
        // runs the final command set and returns PhaseResult::complete(...)
    }
```

`parse()` (`parser/mod.rs:154`) strips think blocks internally (via
`strip_think_blocks`) during candidate detection, finds no tool-call candidates
in the empty remainder, and returns `ParseResult::NoToolCall`. The agent loop
interprets `NoToolCall` as "the model is done" — the only distinction it makes
between "genuine final answer" and "no output at all."

Session log evidence from `session-phase-02-6a1f7c64.jsonl`, turn 3: the model
emitted `</think>\n\n\n` (an enormous think block containing the full correct
implementation plan, followed by three newlines). `parse()` returned `NoToolCall`,
`session_end` fired with `status: "complete"`, and the command set ran against the
unchanged codebase. Zero files changed; phase reported complete after 3 turns.

`strip_think_blocks` (`parser/mod.rs:21`) **exists and is correct** — it is
simply never called at the agent-loop level to distinguish the two cases.

## What should happen

A response that consists entirely of a think block (nothing, or only
whitespace, after `</think>`) is not a clean exit — the model reasoned but
failed to act. It should be treated as a **recoverable parse failure** with
feedback prompting the model to emit a tool call, identical in shape to
`ParseResult::Failed`. After `max_turns` with this pattern it should degrade
to `hard_fail` / `budget_exceeded` like any other unresolved parse failure.

A genuine clean exit is a response where, *after* stripping think blocks, the
remaining text is non-empty and contains no tool-call candidates — the model
wrote a final prose message. A think-block-only response with empty remainder
is distinct and should not be conflated with it.

## How to fix

In `executor/src/agent/mod.rs`, in the `ParseResult::NoToolCall` branch
(currently at line ~411), check whether the raw completion was think-block-only
before treating it as `complete`:

```rust
ParseResult::NoToolCall => {
    // A completion that is *only* a think block (empty after stripping)
    // is not a clean exit — treat it as a parse failure so the model
    // gets feedback to emit a tool call.
    let post_think = crate::parser::strip_think_blocks(&completion).trim().to_owned();
    if post_think.is_empty() && completion.contains("</think>") {
        // Think-only: inject feedback and continue (same path as ParseResult::Failed).
        metrics.parse_failures += 1;
        let feedback = "Your response contained only a <think> block with no tool \
            call after it. You must emit a tool call after your reasoning. \
            Please produce a tool call now.";
        messages.push(assistant_text(&completion, turns));
        messages.push(user_text(feedback, turns));
        // fall through to turn budget check / next iteration
        ...
    } else {
        // Genuine clean exit: model wrote a final prose response.
        log_session_end(..., "complete", turns);
        ...
    }
}
```

The exact restructuring (early-continue vs. extracted helper) is the
implementor's choice. The key invariant: `NoToolCall` on a think-only completion
must not produce `PhaseResult::complete`.

Additionally, consider calling `strip_think_blocks` **before** passing
`completion` to `parse()` — the function already exists for exactly this purpose
and stripping upstream would simplify the parser's internal handling.

## Verification

- [ ] A `MockAiClient` test that returns a think-block-only response (e.g.
  `"<think>plan</think>\n\n"`) does **not** produce `PhaseResult::complete`
  after one turn; it produces a `ParseFailed` session event and the loop
  continues (or reaches `budget_exceeded` at turn cap).
- [ ] A `MockAiClient` test that returns a genuine non-tool-call prose response
  (e.g. `"I have finished all the tasks."`) **still** produces
  `PhaseResult::complete` (regression guard).
- [ ] `cargo test` passes.

## Notes

This is an executor-layer defect — the fix lives in `executor/src/agent/mod.rs`
and `executor/src/parser/mod.rs`. It is not specific to M8 and should be planned
as a standalone executor phase in a future milestone. Filed here because M8
phase-02 is where the failure was observed and diagnosed.
