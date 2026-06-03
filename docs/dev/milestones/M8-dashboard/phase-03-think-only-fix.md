# Phase 03: fix think-only completion treated as clean exit

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** none (executor-crate fix; M8 phases 01–02 are independent)
**Estimated diff:** ~80 lines (`executor/src/agent/mod.rs` branch + new tests)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

Fix `bug-executor-1`: when a reasoning model (Qwen3, DeepSeek-R1, etc.)
produces a completion that is **entirely** a `<think>…</think>` block with
nothing after the closing tag, the agent loop treats it as a clean exit and
returns `PhaseResult::complete` with zero files changed. The fix detects
think-only completions in the `ParseResult::NoToolCall` branch and routes them
through the existing parse-failure feedback path instead, so the model gets
feedback prompting it to emit a tool call and the loop continues.

## Architecture references

- `executor/src/agent/mod.rs` — the turn loop; the `NoToolCall` branch at line
  ~411 is the site.
- `executor/src/parser/mod.rs` — `strip_think_blocks` (line 21) and
  `format_no_match` (line 43) already exist; both are reused here.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `docs/dev/milestones/M8-dashboard/bugs/bug-executor-1.md` — the filed
   bug; this phase closes it.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### The `NoToolCall` branch in `executor/src/agent/mod.rs` (lines ~409–435)

This is the exact code this phase modifies:

```rust
// agent/mod.rs ~line 409
metrics.parse_attempts += 1;
match parse(&completion, deps.registry) {
    ParseResult::NoToolCall => {
        log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
        // Step 8 — clean completion runs the final command set.
        let emit = EmitCtx { /* ... */ };
        let (command_outputs, gates) =
            run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
        emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
        let artifacts = build_artifacts(/* ... "complete" ... */);
        return Ok(PhaseResult::complete(artifacts));
    }
    ParseResult::Found(tc) => tc,
    ParseResult::Failed(failure) => {
        metrics.parse_failures += 1;
        log_event(/* SessionEvent::ParseFailed { failure } */);
        messages.push(assistant_text(&completion, turns));
        messages.push(user_text(&failure.feedback, turns));
        if turns >= deps.max_turns {
            // ... budget_exceeded path ...
        }
        continue;
    }
}
```

### `strip_think_blocks` — already exists, never called at agent level (`parser/mod.rs:21`)

```rust
pub fn strip_think_blocks(s: &str) -> String {
    // Strips <think>…</think> blocks and the trailing newline after </think>.
    // Returns the empty string when the input is entirely a think block.
}
```

### `format_no_match` — the right feedback function for this case (`parser/mod.rs:43`)

```rust
/// Used when no candidates were extracted at all. Short, generic guidance.
pub fn format_no_match(response_excerpt: &str) -> String {
    // Returns: "No tool call was found in your response. Emit a single tool
    // call in the expected format, or respond without a tool call if you are
    // done.\nExcerpt: <first 200 chars of response>"
}
```

### `ParseFailure` struct (`parser/mod.rs:112`)

```rust
pub struct ParseFailure {
    pub raw: String,
    pub detected_format: Option<Format>,
    pub candidates: Vec<Candidate>,
    pub feedback: String,
}
```

### Existing `ParseFailed` path for reference (same file, ~line 437–484)

The think-only branch must produce **the same observable effects** as this path:
log a `SessionEvent::ParseFailed`, push `assistant_text` + `user_text` onto
`messages`, check the turn budget, and `continue`. Do not return or log
`session_end` on the think-only path.

### Existing tests that must keep passing

- `no_tool_call_first_turn_completes_immediately` (~line 1364) — model returns
  `"All done, nothing to call."` → `PhaseStatus::Complete`. This is the
  genuine-clean-exit case; it must **not** be affected.
- `tool_call_then_no_tool_call_completes` (~line 1396) — tool call then
  `"now I'm done"` → `PhaseStatus::Complete`. Same.

## Spec

### Task 1 — Detect think-only in the `NoToolCall` branch

In `executor/src/agent/mod.rs`, replace the `ParseResult::NoToolCall` arm with a
two-way branch. Import `crate::parser::strip_think_blocks` at the top of the
function (or inline the call — the function is `pub`).

The detection predicate:

```rust
let post_think = crate::parser::strip_think_blocks(&completion);
let think_only = post_think.trim().is_empty() && completion.contains("</think>");
```

- `think_only == true` → treat as a recoverable parse failure (see below).
- `think_only == false` → existing clean-exit path, unchanged.

**Think-only path** (mirrors `ParseResult::Failed`, same structure):

```rust
// think-only: model reasoned but emitted no action — treat as parse failure.
metrics.parse_failures += 1;
let failure = ParseFailure {
    raw: completion.clone(),
    detected_format: None,
    candidates: vec![],
    feedback: crate::parser::feedback::format_no_match(&completion),
};
log_event(
    &log_handle,
    &redactor,
    deps.clock,
    turns,
    SessionEvent::ParseFailed {
        failure: failure.clone(),
    },
);
messages.push(assistant_text(&completion, turns));
messages.push(user_text(&failure.feedback, turns));
if turns >= deps.max_turns {
    // ... same budget_exceeded path as ParseResult::Failed ...
}
continue;
```

The budget-exceeded block is a verbatim copy of the one in `ParseResult::Failed`
(lines ~450–483). Do not factor it out — the spec does not authorize a
refactor, and two copies in the same match arm is acceptable duplication for
now.

**Note on `feedback::format_no_match` visibility:** it is currently `pub` in
`parser/feedback.rs` — no visibility change needed.

### Task 2 — Two new unit tests

In `executor/src/agent/mod.rs`'s `#[cfg(test)] mod tests`, add:

**`think_only_completion_is_not_complete`** — a `MockAiClientScript` that
returns turn 1 as a think-only token stream (`"<think>I will read the
file</think>\n\n"`) and turn 2 as a genuine clean-exit token stream
(`"All done."`). Assert:
- `result.status == PhaseStatus::Complete` (the loop recovers on turn 2).
- `client.calls().len() == 2` (the loop did not exit after turn 1).

Use the same setup shape as `no_tool_call_first_turn_completes_immediately`
(line ~1364):

```rust
let client = MockAiClientScript::new(vec![
    vec![token("<think>I will read the file</think>\n\n")],
    vec![token("All done.")],
]);
```

**`think_only_completion_at_budget_is_budget_exceeded`** — a
`MockAiClientScript` whose every turn is a think-only response. Set `max_turns`
to 2. Assert `result.status == PhaseStatus::BudgetExceeded`.

```rust
let client = MockAiClientScript::new(vec![
    vec![token("<think>plan</think>\n\n")],
    vec![token("<think>still thinking</think>\n")],
]);
// ... deps with max_turns = 2 ...
```

**Regression guard** — confirm `no_tool_call_first_turn_completes_immediately`
still passes (it will if the non-think-only path is unchanged; the existing
test serves as the guard, no new test needed for this).

## Acceptance criteria

- [ ] A think-only completion (`<think>…</think>` with empty remainder) does
      **not** produce `PhaseResult::complete`; it produces a `ParseFailed`
      session event and the loop continues.
- [ ] A genuine non-tool-call prose response (`"All done, nothing to call."`)
      still produces `PhaseResult::complete` (regression: existing
      `no_tool_call_first_turn_completes_immediately` passes).
- [ ] After a think-only completion, the model receives feedback via
      `user_text` (the loop pushes `assistant_text(&completion) +
      user_text(&failure.feedback)`).
- [ ] At `max_turns` with only think-only responses, `PhaseResult` status is
      `BudgetExceeded`.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

- `think_only_completion_is_not_complete` in `executor/src/agent/mod.rs` —
  think-only turn 1 followed by prose turn 2; asserts `Complete` and 2 client
  calls.
- `think_only_completion_at_budget_is_budget_exceeded` — two think-only turns,
  `max_turns = 2`; asserts `BudgetExceeded`.
- `no_tool_call_first_turn_completes_immediately` (existing, ~line 1364) —
  serves as the non-think-only regression guard; must keep passing unchanged.

## End-to-end verification

The fix is internal to the executor loop — no CLI surface change. Verify:

1. `cargo test -p rexymcp-executor` passes including the two new tests.
2. Quote the two new test names and their pass status in the Update Log.

## Authorizations

- [x] May modify `executor/src/agent/mod.rs` (the turn loop).
- [ ] No other files touched. `parser/mod.rs` needs no changes —
      `strip_think_blocks` and `format_no_match` are already `pub` and usable
      as-is. No `Cargo.toml` edits. No `docs/architecture.md` edits.

## Out of scope

- Calling `strip_think_blocks` before `parse()` to simplify the parser's
  internal handling — that is a separate refactor not authorized here.
- Detecting partially-empty post-think responses (e.g. `</think>\nSome prose`
  with no tool call) — those already hit `ParseResult::NoToolCall` and exit
  cleanly, which is correct behaviour.
- Any changes to how `strip_think_blocks` itself works.
- Any changes to the `ParseResult::Found` or `ParseResult::Failed` branches.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
