# Phase 01: Empty-completion routing + governor stall

**Milestone:** M22 — Bookkeeping-Loop Resilience
**Status:** todo
**Depends on:** none (extends the M19/M21 `NoToolCall` arm)
**Estimated diff:** ~180 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Stop the empty-output death spiral. Two coupled changes to the agent loop: (A1)
route a **truly-empty** model completion to the existing "emit a tool call"
recovery feedback instead of treating it as a completion attempt, and (A2)
terminate a run of consecutive empty completions as `hard_fail` with a named
governor signal instead of burning to the turn cap.

In `session-phase-04-6a32f806`, a blank completion (`raw: ""`) fell through the
`NoToolCall` guard, was treated as a completion signal, re-ran the gates, tripped
`task_coverage_retry`, and re-injected identical feedback — **147 times** — until
the 200-turn cap. The governor never saw it because empty completions produce no
tool call, so `recent_tool_calls` never grew.

## Architecture references

Read before starting:

- `executor/src/agent/mod.rs` — the `NoToolCall` arm (the parse-failure branch at
  lines ~510–569 is the A1 site; the Step-7 hard-fail emission block at
  ~1062–1103 is the shape A2 mirrors).
- `executor/src/governor/hard_fail.rs` — `HardFailSignal` enum + `describe()`
  (lines 14–54) and the pure per-signal checks (`check_identical_repetition` etc.,
  79–132); A2 adds a sibling variant + pure check here.
- `executor/src/config.rs` — `GovernorConfig` fields + defaults (lines ~166–180);
  A2 adds one threshold field.
- `executor/src/parser/mod.rs` — `strip_think_blocks` (line 21): returns the
  completion unchanged when there is no `<think>` block, so `post_think` is empty
  exactly when the post-think text is blank.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0 (no pre-existing failures).

## Current state

### The `NoToolCall` parse-failure branch — `executor/src/agent/mod.rs` ~510–569

```rust
match parse(&completion, deps.registry) {
    ParseResult::NoToolCall => {
        // A completion that is *only* a <think> block (empty after
        // stripping) is not a clean exit — the model reasoned but
        // emitted no action. Treat it as a recoverable parse failure
        // so it gets feedback to emit a tool call. bug-executor-1.
        let post_think = crate::parser::strip_think_blocks(&completion);
        if post_think.trim().is_empty() && completion.contains("</think>") {
            metrics.parse_failures += 1;
            let failure = crate::parser::ParseFailure { /* … */ };
            log_event(/* … SessionEvent::ParseFailed … */);
            messages.push(assistant_text(&completion, turns));
            messages.push(user_text(&failure.feedback, turns));
            if turns >= deps.max_turns {
                /* … log_session_end "budget_exceeded"; emit_phase_run; return budget_exceeded_result … */
            }
            continue;
        }
        // Step 8 — run the final gate set BEFORE declaring completion …
        let emit = EmitCtx { /* … */ };
        let (command_outputs, gates) =
            run_command_set(deps.runner, deps.commands, deps.project_root, &emit).await;
        // … gate_failure_feedback / task_coverage_feedback / bookkeeping_feedback …
        // … then "All configured gates passed — this is a true completion." …
    }
    // … ToolCall arm (native + parsed) below …
}
```

The bug: `post_think.trim().is_empty() && completion.contains("</think>")` fires
only when there **is** a `</think>` tag. A blank completion (`""`, no think tag)
has `post_think == ""` (empty) but no `</think>`, so it skips the recovery branch
and falls through to the gate/completion path.

### Counter declarations — `executor/src/agent/mod.rs` ~157–163

```rust
let mut recent_tool_calls: VecDeque<ToolCallSnapshot> = VecDeque::new();
let mut turns: usize = 0;

// Governor feedback state (07c).
let mut baseline = Baseline::new();
let mut baselined_exts: HashSet<String> = HashSet::new();
let mut recent_verifier_error_counts: Vec<usize> = Vec::new();
```

### Step-7 hard-fail emission block — `executor/src/agent/mod.rs` ~1062–1103

```rust
if let Some(signal) = evaluate(
    &recent_tool_calls,
    &recent_verifier_error_counts,
    Some((&tool_call.name, content.len())),
    &deps.governor,
) {
    log_event(/* … SessionEvent::HardFail { reason: signal.describe() } … */);
    log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
    emit_phase_run(&deps, input, "hard_fail", Gates::default(), &metrics, &scorer, turns);
    let artifacts = build_artifacts(
        &pre_edit_content, deps.project_root, log_path.clone(), "hard_fail", turns,
        CommandOutputs::default(),
    );
    return Ok(hard_fail_result(
        input, &recent_tool_calls, deps.project_root, last_author_diagnostics, signal, artifacts,
    ));
}
```

Note: this `evaluate` call is reached **only after a tool call is dispatched** —
the `NoToolCall` arm `continue`s before it. So the empty-completion stall cannot
be detected here; it must be checked **inline in the `NoToolCall` arm** (Task 4),
reusing the same emission shape.

### Governor — `executor/src/governor/hard_fail.rs`

`HardFailSignal` (14–30) and `describe()` (32–54); pure checks like
`check_identical_repetition` (79–98). `evaluate` (56–77) aggregates the
tool-call/verifier/runaway checks. The new empty-completion check is a **pure
sibling** (`check_empty_completion_stall`), **not** added to `evaluate` — it is
called from the loop where the empty-completion count lives.

### Config — `executor/src/config.rs` ~166–180

```rust
pub struct GovernorConfig {
    pub identical_call_threshold: usize,        // default 6
    pub verifier_persistence_threshold: usize,  // default 6
    pub runaway_output_bytes: usize,            // default 100 * 1024
}
impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            identical_call_threshold: 6,
            verifier_persistence_threshold: 6,
            runaway_output_bytes: 100 * 1024,
            // …
        }
    }
}
```

There is a `GovernorConfigOverride` mirror immediately below (`Option<usize>`
fields + an apply block ~434). Add the new field to **both** the struct + default
**and** the override mirror + its apply line, following the existing three fields'
exact shape.

## Spec

### Task 1 — Add the `EmptyCompletionStall` signal + pure check (`hard_fail.rs`)

Add a variant to `HardFailSignal` (after `RunawayOutput`):

```rust
EmptyCompletionStall {
    consecutive_count: u32,
},
```

Add its `describe()` arm:

```rust
Self::EmptyCompletionStall { consecutive_count } => {
    format!("model emitted {consecutive_count} consecutive empty completions")
}
```

Add the pure check (sibling to `check_identical_repetition`; **not** wired into
`evaluate`):

```rust
/// Empty-completion stall: the model emitted `consecutive_empty` blank/think-only
/// completions in a row (no tool call, no answer text). Distinct from
/// `IdenticalToolCallRepetition`, which only sees turns that produced a tool call.
pub fn check_empty_completion_stall(
    consecutive_empty: usize,
    threshold: usize,
) -> Option<HardFailSignal> {
    if consecutive_empty >= threshold {
        Some(HardFailSignal::EmptyCompletionStall {
            consecutive_count: threshold as u32,
        })
    } else {
        None
    }
}
```

### Task 2 — Add `empty_completion_threshold` to `GovernorConfig` (`config.rs`)

Add `pub empty_completion_threshold: usize,` to `GovernorConfig`, defaulting to
**3** (a run of 3 blank completions is unambiguously stuck; the spiral was 147).
Mirror the field into `GovernorConfigOverride` as `Option<usize>` and add its
apply line, exactly as the other three governor thresholds do.

### Task 3 — A1: broaden the empty-completion guard (`mod.rs`)

Change the guard condition so a **truly-empty** completion also routes to the
recovery branch. Replace:

```rust
if post_think.trim().is_empty() && completion.contains("</think>") {
```

with:

```rust
if post_think.trim().is_empty() {
```

Rationale: `strip_think_blocks` returns the input unchanged when there is no think
block, so `post_think.trim().is_empty()` is true for **both** a `<think>`-only
completion **and** a blank completion. Dropping the `&& contains("</think>")`
conjunct folds the blank case into the same recovery path. A completion with real
post-think answer text (e.g. `"All done."`) still has a non-empty `post_think` and
still falls through to the gate/completion path — **this must not change** (pinned
negative below).

### Task 4 — A2: maintain the empty-completion counter + stall (`mod.rs`)

**Declare the counter** alongside the governor feedback state (~163, after
`recent_verifier_error_counts`):

```rust
let mut consecutive_empty_completions: usize = 0;
```

**Inside the (now broadened) empty branch**, at the top of the `if
post_think.trim().is_empty() { … }` block, before `metrics.parse_failures += 1;`,
increment and check for the stall:

```rust
consecutive_empty_completions += 1;
if let Some(signal) = crate::governor::hard_fail::check_empty_completion_stall(
    consecutive_empty_completions,
    deps.governor.empty_completion_threshold,
) {
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        turns,
        SessionEvent::HardFail {
            reason: signal.describe(),
        },
    );
    log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
    emit_phase_run(
        &deps, input, "hard_fail", Gates::default(), &metrics, &scorer, turns,
    );
    let artifacts = build_artifacts(
        &pre_edit_content,
        deps.project_root,
        log_path.clone(),
        "hard_fail",
        turns,
        CommandOutputs::default(),
    );
    return Ok(hard_fail_result(
        input,
        &recent_tool_calls,
        deps.project_root,
        last_author_diagnostics.clone(),
        signal,
        artifacts,
    ));
}
```

(`last_author_diagnostics` is moved into `hard_fail_result` at Step 7; here it is
still live and used again later, so clone it — Step 7's call can keep its move.)

**Reset the counter on any productive turn.** Two reset sites, both setting
`consecutive_empty_completions = 0;`:

1. In the gate/completion fall-through — immediately after the empty branch closes
   (before `// Step 8 — run the final gate set …`), since reaching here means the
   completion had real answer text.
2. In the tool-dispatch path — right after the `SessionEvent::Parsed { tool_call }`
   is logged (~810), since a real tool call is a productive turn.

### Task 5 — Unit tests for `check_empty_completion_stall` (`hard_fail.rs`)

In the `#[cfg(test)] mod tests` block:

- `empty_completion_stall_fires_at_threshold` — `check_empty_completion_stall(3, 3)`
  returns `Some(EmptyCompletionStall { consecutive_count: 3 })`.
- `empty_completion_stall_silent_below_threshold` —
  `check_empty_completion_stall(2, 3)` returns `None`.
- `describe_empty_completion_stall` — the `describe()` string contains
  `"empty completions"` and the count.

### Task 6 — Integration tests in `executor/src/agent/tests.rs`

Use `MockAiClientScript` to feed empty completions. Model the setup on the
existing gate-retry tests (`gate_failure_loops_until_gates_pass` region).

- `empty_completions_hard_fail_at_threshold` — a client scripted to return
  `vec![token("")]` (or an empty token vec) on every turn, `max_turns` well above
  the threshold, governor default (threshold 3). Assert
  `result.status == PhaseStatus::HardFail` and that the model was called exactly
  `empty_completion_threshold` times (the stall fires on the 3rd empty, not the
  turn cap).
- `single_empty_completion_then_recovers_does_not_hard_fail` — client returns one
  empty completion, then a `write_file`/`update_task` tool call, then a clean
  completion; assert the run does **not** hard_fail on the empty (the counter
  resets after the productive turn). Use whatever minimal tool the test harness
  already wires (see the M21 integration tests for the `update_task` registry
  pattern).

**Pinned negatives:**

- `single_empty_completion_then_recovers_does_not_hard_fail` — one stray empty
  followed by a real action must NOT hard_fail; the reset is load-bearing.
- A completion with real answer text must still reach the gate/completion path —
  covered by the **unmodified** pre-existing completion tests (e.g.
  `gate_failure_loops_until_gates_pass`, which scripts `token("All done.")`). Those
  must pass unchanged; if any needs editing, that is a signal A1 broke the
  non-empty path — stop and file a blocker.

## Acceptance criteria

- [ ] `empty_completion_stall_fires_at_threshold` passes.
- [ ] `empty_completion_stall_silent_below_threshold` passes.
- [ ] `describe_empty_completion_stall` passes.
- [ ] `empty_completions_hard_fail_at_threshold` passes (hard_fail at the
      threshold, not budget_exceeded at the cap).
- [ ] `single_empty_completion_then_recovers_does_not_hard_fail` passes.
- [ ] All pre-existing tests pass **unmodified**.
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo test` all exit 0.

## Test plan

- `check_empty_completion_stall` unit tests in `hard_fail.rs` (3 above).
- Two `execute_phase` integration tests in `tests.rs` (above), asserting both the
  hard_fail-at-threshold behavior and the reset-on-recovery behavior.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. `execute_phase` is the
library entry point; the integration tests exercise the empty-completion path
end-to-end against `MockAiClientScript`. (The config field is loaded by the same
`Config` machinery already covered by existing config tests.)

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `architecture.md` edit. The new
config field extends `GovernorConfig` (additive, defaulted) — no `rexymcp.toml`
schema doc is required for an internally-defaulted governor knob.

## Out of scope

- The stuck *non-empty* gate-feedback loop (A3) — that is phase-02.
- Changing `evaluate`'s signature or the existing tool-call/verifier/runaway
  checks. The empty-completion check is a separate pure fn called from the loop.
- Detecting empty **native** tool calls — the observed failure is the text/parse
  path; native-call emptiness is not in evidence.
- Documenting `empty_completion_threshold` in the `rexymcp init` template — defer
  unless a later phase surfaces a need to tune it per-model.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
