# Phase 03: fix think-only completion treated as clean exit

**Milestone:** M8 — Live session dashboard
**Status:** in-progress (refined re-dispatch after RunawayOutput hard_fail — see Update Log)
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
2. Read this entire phase doc before touching code.
3. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

> ⚠️ **DO NOT `read_file` `executor/src/agent/mod.rs` in full.** It is ~140 KB
> and a whole-file read trips the executor's `RunawayOutput` safety limit — this
> is exactly what hard-failed the first dispatch of this phase. **Everything you
> need from that file is quoted verbatim below** (the `NoToolCall`/`Failed`
> match arms in "Current state", and the test harness in "Reference excerpts").
> Make your edits with the `patch` tool, anchoring on the quoted strings — you do
> **not** need to open the file. If you must look something up, use `search` with
> a narrow pattern (e.g. `search` for `ParseResult::Failed`), never a full read.
> The same applies to `bug-executor-1.md` — its content is already summarized in
> the Goal; you do not need to read it.

## Current state

### The `NoToolCall` / `Failed` match arms in `executor/src/agent/mod.rs` (VERBATIM)

This is the **exact, complete** code this phase modifies — copied verbatim from
the file so you do not need to open it. Use it as your `patch` anchor.

```rust
            metrics.parse_attempts += 1;
            match parse(&completion, deps.registry) {
                ParseResult::NoToolCall => {
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
                    // Step 8 — clean completion runs the final command set.
                    let emit = EmitCtx {
                        progress: deps.progress,
                        log_handle: &log_handle,
                        redactor: &redactor,
                        clock: deps.clock,
                        pre_edit_content: &pre_edit_content,
                        project_root: deps.project_root,
                        turn: turns,
                    };
                    let (command_outputs, gates) =
                        run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
                    emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
                    let artifacts = build_artifacts(
                        &pre_edit_content,
                        deps.project_root,
                        log_path.clone(),
                        "complete",
                        turns,
                        command_outputs,
                    );
                    return Ok(PhaseResult::complete(artifacts));
                }
                ParseResult::Found(tc) => tc,
                ParseResult::Failed(failure) => {
                    metrics.parse_failures += 1;
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
                        log_session_end(
                            &log_handle,
                            &redactor,
                            deps.clock,
                            "budget_exceeded",
                            turns,
                        );
                        emit_phase_run(
                            &deps,
                            input,
                            "budget_exceeded",
                            Gates::default(),
                            &metrics,
                            &scorer,
                            turns,
                        );
                        let artifacts = build_artifacts(
                            &pre_edit_content,
                            deps.project_root,
                            log_path.clone(),
                            "budget_exceeded",
                            turns,
                            CommandOutputs::default(),
                        );
                        return Ok(budget_exceeded_result(
                            input,
                            &recent_tool_calls,
                            deps.project_root,
                            turns_line(deps.max_turns),
                            artifacts,
                        ));
                    }
                    continue;
                }
            }
```

**The minimal edit:** change the `ParseResult::NoToolCall => { … }` arm so it
first checks for a think-only completion and, if so, falls through to the same
feedback-and-continue behaviour as `ParseResult::Failed`. The cleanest `patch`
replaces the **opening** of the `NoToolCall` arm — anchor on this exact slice:

```rust
                ParseResult::NoToolCall => {
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
```

and replace it with the think-only guard (see Spec Task 1) followed by the same
two lines for the genuine-clean-exit path. The `Found` and `Failed` arms are
shown only for context — **do not modify them.**

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

Modify the `ParseResult::NoToolCall` arm so it first checks whether the
completion was think-only and, if so, behaves exactly like `ParseResult::Failed`
(log `ParseFailed`, push feedback, budget-check, `continue`); otherwise it takes
the original clean-exit path unchanged.

**Make this edit with `patch`.** Anchor on the exact two lines that open the arm:

```rust
                ParseResult::NoToolCall => {
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
```

Replace that slice with the following (the rest of the original `NoToolCall` body
— `EmitCtx { … }` through `return Ok(PhaseResult::complete(artifacts));` — stays
exactly as it is, now guarded inside the `else`):

```rust
                ParseResult::NoToolCall => {
                    // A completion that is *only* a <think> block (empty after
                    // stripping) is not a clean exit — the model reasoned but
                    // emitted no action. Treat it as a recoverable parse failure
                    // so it gets feedback to emit a tool call. bug-executor-1.
                    let post_think = crate::parser::strip_think_blocks(&completion);
                    if post_think.trim().is_empty() && completion.contains("</think>") {
                        metrics.parse_failures += 1;
                        let failure = crate::parser::ParseFailure {
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
                            log_session_end(
                                &log_handle,
                                &redactor,
                                deps.clock,
                                "budget_exceeded",
                                turns,
                            );
                            emit_phase_run(
                                &deps,
                                input,
                                "budget_exceeded",
                                Gates::default(),
                                &metrics,
                                &scorer,
                                turns,
                            );
                            let artifacts = build_artifacts(
                                &pre_edit_content,
                                deps.project_root,
                                log_path.clone(),
                                "budget_exceeded",
                                turns,
                                CommandOutputs::default(),
                            );
                            return Ok(budget_exceeded_result(
                                input,
                                &recent_tool_calls,
                                deps.project_root,
                                turns_line(deps.max_turns),
                                artifacts,
                            ));
                        }
                        continue;
                    }
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
```

That is the whole change to the arm: the think-only guard is inserted, and the
original `log_session_end(... "complete" ...)` line now follows the guard (so the
genuine clean-exit path is unchanged). Everything after it in the original arm
(`let emit = EmitCtx { … }` … `return Ok(PhaseResult::complete(artifacts));`) and
the closing `}` stay exactly as before.

**Paths/visibility — all already usable, no import or visibility changes needed:**
- `crate::parser::strip_think_blocks` — `pub` (parser/mod.rs:21).
- `crate::parser::ParseFailure` — `pub` struct (parser/mod.rs:112).
- `crate::parser::feedback::format_no_match` — `pub` (parser/feedback.rs:43).
- `SessionEvent`, `assistant_text`, `user_text`, `Gates`, `CommandOutputs`,
  `budget_exceeded_result`, `turns_line`, `build_artifacts`, `emit_phase_run` are
  all already in scope in this function (the `Failed` arm uses them).

The duplicated budget-exceeded block is acceptable (it mirrors `ParseResult::Failed`);
do **not** refactor it into a shared helper — that is out of scope.

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

### Reference excerpts — the test harness (VERBATIM, so you need not read the file)

The `#[cfg(test)] mod tests` block in `executor/src/agent/mod.rs` already defines
every helper your two tests need. Quoted here so you can write the tests by
pattern-matching the existing `no_tool_call_first_turn_completes_immediately`
test (shown last) without opening the 140 KB file. **Add your two `#[tokio::test]`
functions immediately after that existing test** — `patch`-anchor on its closing
`}` plus the next test's `#[tokio::test]` line.

Module imports already present (top of the test module):

```rust
    use super::*;
    use crate::ai::testing::{MockAiClientScript, MockCall};
    use crate::phase::PhaseStatus;
    use crate::security::scope::Scope;
    use crate::tools::{patch, read_file, write_file};
    use serde_json::json;
    use tempfile::TempDir;
```

Helpers already defined in that module:

```rust
    fn registry_over(scope: Scope) -> ToolRegistry { /* read_file + write_file + patch */ }
    fn input() -> PhaseInput { /* ... */ }
    fn token(s: &str) -> AiEvent { AiEvent::Token(s.to_string()) }

    // deps(client, registry, budget, max_turns, root) -> LoopDeps
    fn deps<'a>(
        client: &'a dyn AiClient,
        registry: &'a ToolRegistry,
        budget: &'a Budget,
        max_turns: usize,
        root: &'a Path,
    ) -> LoopDeps<'a> { /* ... */ }
```

The existing test to mirror (verbatim):

```rust
    #[tokio::test]
    async fn no_tool_call_first_turn_completes_immediately() {
        let dir = TempDir::new().unwrap();
        let scope = Scope::new(dir.path()).unwrap();
        let registry = registry_over(scope);
        let client = MockAiClientScript::new(vec![vec![token("All done, nothing to call.")]]);
        let budget = Budget::new(1_000_000);

        let result = execute_phase(&input(), deps(&client, &registry, &budget, 8, dir.path()))
            .await
            .unwrap();

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(result.briefing.is_none());
        assert_eq!(client.calls().len(), 1);
    }
```

Your two new tests follow the same shape. For
`think_only_completion_at_budget_is_budget_exceeded`, pass `2` as the `max_turns`
argument to `deps(&client, &registry, &budget, 2, dir.path())`. (`Budget::new`,
`Scope`, `execute_phase`, `PhaseStatus`, `MockAiClientScript`, `token`, `input`,
`deps`, `registry_over` are all already imported/defined — your tests need no new
imports.)

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

### Notes for executor — 2026-06-02

This phase was refined after a first dispatch hard-failed with
`RunawayOutput` (the executor `read_file`'d the 140 KB `executor/src/agent/mod.rs`
in one shot and tripped the safety limit). **Do not read that file whole** — the
⚠️ callout in Pre-flight, the verbatim match-arm quote in "Current state", and the
verbatim test harness in "Reference excerpts" now contain everything you need.
Make the source edit with `patch`, anchoring on the quoted `ParseResult::NoToolCall
=> { log_session_end(... "complete" ...)` slice. Add the two tests right after the
quoted `no_tool_call_first_turn_completes_immediately`. If you need to confirm a
detail, use `search` with a narrow pattern — never a full-file read.

### Update — 2026-06-02 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the hard_fail was a spec gap (the Pre-flight told the executor to
read a 140 KB file that trips `RunawayOutput`), not a model failure — fixed by
pre-injecting the needed code verbatim and forbidding the whole-file read.
