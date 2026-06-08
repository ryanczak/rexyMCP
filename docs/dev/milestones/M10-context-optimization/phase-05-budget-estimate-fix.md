# Phase 05: fix `Budget::estimate` — count tool exchange content

**Milestone:** M10 — Context optimization
**Status:** done
**Depends on:** phase-04 (surfaced the bug; see `bugs/bug-budget-estimate-1.md`)
**Estimated diff:** ~20 lines
**Tags:** language=rust, kind=fix, size=xs

## Goal

`Budget::estimate` (`executor/src/context/budget.rs:44`) counts only `msg.content`
for each message. But every tool-call/tool-result message has `content: String::new()`
— the actual payload lives in `msg.tool_calls[n].arguments` (the assistant turn) and
`msg.tool_results[n].content` (the tool turn). The result: `context_pct` is always
the system-prompt-only estimate (~15%), `would_overflow` never fires on real pressure,
and phase-07's context-efficiency metrics would aggregate wrong values from the JSONL.

This phase makes `estimate` count all three payload locations. No API change, no
dependency, no config. Executor-crate only.

## Architecture references

- `executor/src/context/budget.rs` — the file to fix: `estimate` (line 44), `mod
  tests` (line 68).
- `executor/src/context/tokens.rs` — `pub fn count(s: &str) -> usize` (the
  chars/4 heuristic already used in `estimate`). Already imported in `budget.rs`.
- `executor/src/ai/types.rs` — `AiToolCall { id, name, arguments, thought_signature }`
  and `AiToolResult { tool_call_id, tool_name, content }` — fields accessed in the fix.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `executor/src/context/budget.rs` in full.
3. Read `bugs/bug-budget-estimate-1.md` for the diagnosis.
4. Run `cargo test -p rexymcp-executor` and note the passing count.

## Current state

`budget.rs` line 44–50 today:

```rust
pub fn estimate(&self, system_prompt: &str, messages: &[Message]) -> usize {
    let mut total = tokens::count(system_prompt);
    for msg in messages {
        total = total.saturating_add(tokens::count(&msg.content));
    }
    total
}
```

`msg.content` is `String::new()` on every assistant tool-call message and every tool
result message (see `append_tool_exchange` in `executor/src/agent/tools.rs:195`).
The actual payload is in `msg.tool_calls[0].arguments` (a JSON string) and
`msg.tool_results[0].content` (the file / bash output / etc.). Neither is counted.

The existing test `estimate_sums_prompt_and_messages` (line 96) uses a plain
content-only message and passes. It does not exercise tool-exchange messages so it
does not detect this gap.

## Spec

### 1. Fix `estimate` in `executor/src/context/budget.rs`

Replace the body of `estimate` (lines 44–50) verbatim:

```rust
pub fn estimate(&self, system_prompt: &str, messages: &[Message]) -> usize {
    let mut total = tokens::count(system_prompt);
    for msg in messages {
        total = total.saturating_add(tokens::count(&msg.content));
        if let Some(tcs) = &msg.tool_calls {
            for tc in tcs {
                total = total.saturating_add(tokens::count(&tc.arguments));
            }
        }
        if let Some(trs) = &msg.tool_results {
            for tr in trs {
                total = total.saturating_add(tokens::count(&tr.content));
            }
        }
    }
    total
}
```

No other production code changes. No new imports needed in the production path —
`msg.tool_calls` and `msg.tool_results` are fields of `Message` (already imported);
their element types' fields are accessed directly.

### 2. Update `estimate_sums_prompt_and_messages` to cover tool-result content

The test at line 96 already passes. Extend it to also assert that a tool-result
message's content is counted. Replace the existing test with:

```rust
#[test]
fn estimate_sums_prompt_and_messages() {
    let budget = Budget::new(10_000);
    // Plain content message
    let messages = vec![Message {
        role: "user".to_string(),
        content: "world".to_string(),
        tool_calls: None,
        tool_results: None,
        turn: None,
    }];
    let result = budget.estimate("hello", &messages);
    let expected = tokens::count("hello") + tokens::count("world");
    assert_eq!(result, expected);
}
```

(Leave as-is; it remains a valid regression test for the content path.)

### 3. Add three new tests in `mod tests`

Add these after `estimate_sums_prompt_and_messages`. They require importing
`AiToolCall` and `AiToolResult` — add to the existing `use super::*;` line in
`mod tests`:

```rust
use crate::ai::types::{AiToolCall, AiToolResult};
```

The three tests:

```rust
#[test]
fn estimate_includes_tool_result_content() {
    let budget = Budget::new(10_000);
    // A tool-result message: content is empty, payload is in tool_results
    let messages = vec![Message {
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult {
            tool_call_id: "id1".to_string(),
            tool_name: "read_file".to_string(),
            content: "file content goes here".to_string(),
        }]),
        turn: Some(1),
    }];
    let estimated = budget.estimate("", &messages);
    assert!(
        estimated > 0,
        "estimate must count tool_result content, not just msg.content"
    );
    assert_eq!(estimated, tokens::count("file content goes here"));
}

#[test]
fn estimate_includes_tool_call_arguments() {
    let budget = Budget::new(10_000);
    // An assistant tool-call message: content is empty, payload is in arguments
    let messages = vec![Message {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(vec![AiToolCall {
            id: "tc1".to_string(),
            name: "patch".to_string(),
            arguments: r#"{"path":"foo.rs","old_str":"x","new_str":"y"}"#.to_string(),
            thought_signature: None,
        }]),
        tool_results: None,
        turn: Some(2),
    }];
    let estimated = budget.estimate("", &messages);
    assert!(
        estimated > 0,
        "estimate must count tool_call arguments, not just msg.content"
    );
    assert_eq!(
        estimated,
        tokens::count(r#"{"path":"foo.rs","old_str":"x","new_str":"y"}"#)
    );
}

#[test]
fn estimate_counts_all_payloads_in_a_tool_exchange() {
    // A two-message tool exchange: assistant call + tool result, both with empty
    // msg.content. The sum should equal arguments + result content.
    let budget = Budget::new(10_000);
    let args = r#"{"path":"src/lib.rs"}"#;
    let result_body = "pub fn hello() {}";
    let messages = vec![
        Message {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![AiToolCall {
                id: "tc2".to_string(),
                name: "read_file".to_string(),
                arguments: args.to_string(),
                thought_signature: None,
            }]),
            tool_results: None,
            turn: Some(1),
        },
        Message {
            role: "tool".to_string(),
            content: String::new(),
            tool_calls: None,
            tool_results: Some(vec![AiToolResult {
                tool_call_id: "tc2".to_string(),
                tool_name: "read_file".to_string(),
                content: result_body.to_string(),
            }]),
            turn: Some(1),
        },
    ];
    let estimated = budget.estimate("", &messages);
    let expected = tokens::count(args) + tokens::count(result_body);
    assert_eq!(estimated, expected);
}
```

## Acceptance criteria

- [ ] `grep -n 'tool_calls\|tool_results' executor/src/context/budget.rs` matches
      lines inside `estimate` (confirming the fix landed).
- [ ] `cargo test -p rexymcp-executor budget` passes — all existing + 3 new tests.
- [ ] `cargo test` passes (all existing tests unaffected — the fix is additive; it
      never decreases the estimate, only increases it for messages that were previously
      returning 0 for non-content payloads).
- [ ] The three new tests would **each fail** if the added `tool_calls` / `tool_results`
      branches were removed — confirming they actually exercise the new code paths.
- [ ] No new `unwrap()` / `expect()` / `panic!()` in production paths.

## Test plan

Covered entirely by §3 above — three unit tests in `executor/src/context/budget.rs`'s
existing `mod tests` block. No filesystem, no async, no `TempDir` needed — the function
is pure over its inputs.

There is no CLI surface or JSONL behavior change to exercise end-to-end: `estimate`
is an internal measurement function. The correctness of the JSONL `context_pct` values
in live sessions is confirmed by the fact that the tests assert the estimates grow as
expected for tool-exchange messages. The next session run after this phase ships will
show `context_pct` growing turn-over-turn in the dashboard — that is observable
verification but not required for phase completion.

## Authorizations

None. No new dependency. No `Cargo.toml` change. No `docs/architecture.md` change.
`budget.rs` is not in the list of protected files.

## Out of scope

- Changing the compactor trigger threshold or `TARGET_FRACTION`. This phase fixes the
  instrument; whether `would_overflow` trips more frequently after the fix is correct
  behavior, not a new design decision.
- Aggregating `context_pct` onto `PhaseRun` — that is phase-08.
- Any change to how `context_pct` is rendered in the dashboard or logged — those
  consumers read the JSONL value directly; fixing `estimate` fixes what they see.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-16 (started)

**Executor:** rexyMCP executor

### Update — 2025-07-16 (complete)

**Summary:** Fixed `Budget::estimate` to count `tool_calls[n].arguments` and `tool_results[n].content` in addition to `msg.content`. The fix is purely additive — it never decreases the estimate, only increases it for messages that were previously returning 0 for non-content payloads. No API change, no dependency, no config. Note: the actual type names in the codebase are `ToolCall` and `ToolResult` (not `AiToolCall`/`AiToolResult` as the spec sketch used); adapted cleanly.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in 1.2s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.0
    Finished dev [unoptimized + debuginfo] target(s) in 1.4s

cargo test 2>&1 | tail -30
test result: ok. 628 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

N/A — `estimate` is an internal measurement function with no CLI surface or JSONL behavior change. Correctness verified by unit tests asserting estimates grow as expected for tool-exchange messages.

**Grep verification:**
```
grep -n 'tool_calls\|tool_results' executor/src/context/budget.rs
48:            if let Some(tcs) = &msg.tool_calls {
53:            if let Some(trs) = &msg.tool_results {
```
Lines 48 and 53 are inside `estimate`, confirming the fix landed in the production path.

**Files changed:**
- `executor/src/context/budget.rs` — extended `estimate` to count tool call arguments and tool result content; added 3 new unit tests

**New tests:**
- `estimate_includes_tool_result_content` in `executor/src/context/budget.rs`
- `estimate_includes_tool_call_arguments` in `executor/src/context/budget.rs`
- `estimate_counts_all_payloads_in_a_tool_exchange` in `executor/src/context/budget.rs`

**Commits:**
- (pending) — `fix: count tool exchange content in Budget::estimate`

**Notes for review:** The spec referenced types as `AiToolCall`/`AiToolResult` but the actual types are `ToolCall`/`ToolResult` in `executor/src/ai/types.rs`. Adapted the test imports accordingly.

### Review verdict — 2026-06-07

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (40 turns, clean — code, 3 tests, Update Log, and commit all landed)
- **Independent re-run:** `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` — all green, 628 pass (625 + 3 new).
- **DoD:** all boxes met. The fix matches the spec verbatim (`estimate` now counts `tool_calls[n].arguments` + `tool_results[n].content`); the 3 new tests are real (each asserts `estimate == tokens::count(payload)`, which would be `0` and fail if the new branches were removed); no production `unwrap`/`expect`/`panic`/`unsafe`; one conventional commit (`43fa08b`) carrying code + tests + doc flips.
- **Scope deviations:** none. The executor left `estimate_sums_prompt_and_messages` as-is per the spec's "leave as-is" instruction.
- **Calibration:** **architect pre-injection naming error (1 occurrence — data, not a trend).** The spec sketch named the types `AiToolCall`/`AiToolResult`; the canonical types are `ToolCall`/`ToolResult` (`executor/src/ai/types.rs:4`). The `Ai*` names are a **file-local import alias** — `tools.rs:6` does `use crate::ai::types::{ToolCall as AiToolCall, ToolResult as AiToolResult}` — so they exist only inside `agent/tools.rs` and `agent/mod.rs` (where phase-04's tests live, which is why that spec's `Ai*` names compiled there). `budget.rs` has no such alias, so the executor correctly imported the canonical `ToolCall`/`ToolResult`. Clean adaptation, no bounce, but a less capable model could have copied the alias blindly and hit an unresolved-import error. Watch-item: when pre-injecting struct-construction code, cite the type by its canonical name and note any file-local aliases, rather than carrying a name over from whichever file the pattern was lifted from. Not worth a fold.
