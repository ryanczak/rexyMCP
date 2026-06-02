# Phase 01: terminal backend Err → hard_fail degradation

**Milestone:** M7 — Model scorecard & routing
**Status:** done
**Depends on:** M6 (done) — M6 retrospective (phase-06b) made this decision; the implementation sites are identified.
**Estimated diff:** ~80 lines (new enum variant + two loop sites + two new tests)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

A mid-phase terminal backend error (occurring after ≥1 completed turn) currently
aborts `execute_phase` with `Err(Error::Backend)`, discarding partial work and
the briefing. This phase makes it degrade to `Ok(PhaseResult::hard_fail(...))`
instead — preserving the partial diff + what-was-tried so the architect can
choose a lever (re-dispatch / takeover) with full context.

A backend error at turn 0 (before any work) stays `Err` — there is nothing to
preserve and the architect needs to learn the endpoint is down.

## Architecture references

- `docs/architecture.md` § Layer 1 "Escalation = Claude Code itself" — the
  whole escalation contract is "return a structured result, let the host
  re-invoke." An aborting MCP call violates it.
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the
  `hard_fail` status is the structured form for "executor got stuck"; this is
  exactly that.
- `executor/src/agent/mod.rs` — the two change sites and the `hard_fail_result`
  helper they should now call.
- `executor/src/governor/hard_fail.rs` — `HardFailSignal` enum; this phase adds
  a new variant.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc.
3. Confirm `cargo clippy --all-targets --all-features -- -D warnings` passes
   clean on the current tree before making any changes.

## Current state

Two sites in `executor/src/agent/mod.rs` currently propagate backend errors as
`Err`:

**Site A — `chat_fut` completion error (line ≈238):**
```rust
result = &mut chat_fut => {
    result.map_err(|e| Error::Backend(e.to_string()))?;
    break;
}
```
This is inside the inner `tokio::select!` that drives the model call + heartbeat.
`turns` at this point is the count of *completed* turns (incremented at line ≈277,
*after* this select loop finishes and the event loop drains).

**Site B — `AiEvent::Error` (line ≈271–273):**
```rust
AiEvent::Error(e) => {
    log_session_end(&log_handle, &redactor, deps.clock, "error", turns);
    return Err(Error::Backend(e));
}
```
This is inside the `while let Some(event) = rx.recv().await` event-drain loop
that runs after `chat_fut` resolves.

**Existing test that pins current behavior (line ≈1539):**
```rust
#[tokio::test]
async fn ai_event_error_propagates_as_err() {
    // ... MockAiClientScript with AiEvent::Error at turn 0 ...
    assert!(result.is_err(), "AiEvent::Error must surface as Err, not a PhaseResult");
}
```
This test is correct for the turn-0 case and must remain passing. A new
companion test covers the turn>0 case.

**`HardFailSignal` enum (`executor/src/governor/hard_fail.rs:17`):**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardFailSignal {
    IdenticalToolCallRepetition { tool: String, consecutive_count: u32 },
    VerifierFailurePersistent { consecutive_failures: u32 },
    RunawayOutput { tool: String, bytes: usize },
}
```
Add a new variant here. Implement `describe()` for it.

**`hard_fail_result` helper (`executor/src/agent/mod.rs:732`):**
```rust
fn hard_fail_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    diagnostics: Vec<Diagnostic>,
    signal: HardFailSignal,
    artifacts: Artifacts,
) -> PhaseResult { ... }
```
The new degradation path calls this with `HardFailSignal::BackendError { message }`,
`Vec::new()` for diagnostics (no verifier output on a connection failure), and
artifacts assembled via `build_artifacts` with status `"hard_fail"`.

The log + telemetry calls before the existing `hard_fail_result` invocation
(lines ≈522–556) are the exact pattern to follow for the new path:
```rust
log_event(..., SessionEvent::HardFail { reason: signal.describe() });
log_session_end(..., "hard_fail", turns);
emit_phase_run(..., "hard_fail", Gates::default(), ...);
let artifacts = build_artifacts(..., "hard_fail", turns, CommandOutputs::default());
return Ok(hard_fail_result(input, &recent_tool_calls, deps.project_root,
                            last_author_diagnostics, signal, artifacts));
```

## Spec

### Task 1 — Add `BackendError` variant to `HardFailSignal`

In `executor/src/governor/hard_fail.rs`, add to `HardFailSignal`:

```rust
BackendError { message: String },
```

In `impl HardFailSignal::describe()`, add the match arm:

```rust
Self::BackendError { message } => format!("backend error: {message}"),
```

### Task 2 — Degrade site A (chat_fut error)

In `executor/src/agent/mod.rs`, replace the `result.map_err(|e|
Error::Backend(e.to_string()))?` arm with a branch on `turns`:

```rust
result = &mut chat_fut => {
    match result {
        Ok(()) => {}
        Err(e) if turns == 0 => return Err(Error::Backend(e.to_string())),
        Err(e) => {
            let signal = HardFailSignal::BackendError { message: e.to_string() };
            log_event(&log_handle, &redactor, deps.clock, turns,
                      SessionEvent::HardFail { reason: signal.describe() });
            log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
            emit_phase_run(&deps, input, "hard_fail", Gates::default(),
                           &metrics, &scorer, turns);
            let artifacts = build_artifacts(
                &pre_edit_content, deps.project_root, log_path.clone(),
                "hard_fail", turns, CommandOutputs::default(),
            );
            return Ok(hard_fail_result(
                input, &recent_tool_calls, deps.project_root,
                Vec::new(), signal, artifacts,
            ));
        }
    }
    break;
}
```

### Task 3 — Degrade site B (AiEvent::Error)

In the event-drain loop (`while let Some(event) = rx.recv().await`), replace
the `AiEvent::Error` arm:

```rust
AiEvent::Error(e) => {
    if turns == 0 {
        log_session_end(&log_handle, &redactor, deps.clock, "error", turns);
        return Err(Error::Backend(e));
    }
    let signal = HardFailSignal::BackendError { message: e.clone() };
    log_event(&log_handle, &redactor, deps.clock, turns,
              SessionEvent::HardFail { reason: signal.describe() });
    log_session_end(&log_handle, &redactor, deps.clock, "hard_fail", turns);
    emit_phase_run(&deps, input, "hard_fail", Gates::default(),
                   &metrics, &scorer, turns);
    let artifacts = build_artifacts(
        &pre_edit_content, deps.project_root, log_path.clone(),
        "hard_fail", turns, CommandOutputs::default(),
    );
    return Ok(hard_fail_result(
        input, &recent_tool_calls, deps.project_root,
        Vec::new(), signal, artifacts,
    ));
}
```

### Task 4 — Update the existing test (site B, turn 0 case)

The test `ai_event_error_propagates_as_err` (line ≈1539) drives
`MockAiClientScript` with `AiEvent::Error` on the *first* script entry (turn 0).
The `turns == 0` branch keeps this path as `Err`, so the test assertion
(`result.is_err()`) stays valid. **Do not change the test body** — just add a
comment above it:

```rust
// Turn-0 case: backend error before any work stays Err (nothing to preserve).
```

### Task 5 — Add two new tests

**Test A** — mid-phase chat_fut error degrades to hard_fail:

Name: `backend_error_after_progress_degrades_to_hard_fail`

Setup: a `MockAiClientScript` that completes one full turn (produces a token
stream ending with `AiEvent::Done`), then on the second turn's `chat()` call
returns `Err("transient failure")`. Because the chat future is what errors here,
use a mock client where the second `chat()` call returns an error (consult
`executor/src/ai/testing.rs` for the `MockAiClient`/`MockAiClientScript` API;
look for how to inject an error-returning call). Assert:

- `result` is `Ok(phase_result)`
- `phase_result.status` is `hard_fail`
- `phase_result.briefing.is_some()`
- The briefing's `current_blocker` matches `Blocker::HardFail(HardFailSignal::BackendError { .. })`

**Test B** — mid-phase AiEvent::Error degrades to hard_fail:

Name: `ai_event_error_after_progress_degrades_to_hard_fail`

Setup: a `MockAiClientScript` where turn 1 completes cleanly (produces a `token`
+ `AiEvent::Done`), and turn 2's event stream includes `AiEvent::Error("mid-phase
error")`. Assert the same four conditions as test A.

**Pre-injection — MockAiClientScript API.** Look at `executor/src/ai/testing.rs`
for the current mock API. The script model is a `Vec<Vec<AiEvent>>` — one inner
`Vec` per `chat()` call. A `chat()` call that should *return* an error needs the
mock to surface it differently from an `AiEvent::Error` in the stream. Check how
`MockAiClientScript` handles this; if there is no "error return" mechanism, file
a blocker rather than adding one without authorization. Completing one clean turn
then injecting `AiEvent::Error` in the second turn's stream is the simpler path
and directly covers site B. Site A (chat_fut returning `Err`) may require the
client's `chat()` to return `Err` — verify before coding.

## Acceptance criteria

- [ ] `executor/src/governor/hard_fail.rs`: `HardFailSignal::BackendError { message: String }` variant exists with `describe()` returning `"backend error: {message}"`.
- [ ] Site A (`chat_fut` error): error after `turns > 0` returns `Ok(PhaseResult)` with `status == hard_fail` and a `briefing` carrying `Blocker::HardFail(HardFailSignal::BackendError { .. })`. Error at `turns == 0` returns `Err`.
- [ ] Site B (`AiEvent::Error`): same branching behavior as site A.
- [ ] Existing test `ai_event_error_propagates_as_err` still passes (turn-0 case unchanged).
- [ ] `backend_error_after_progress_degrades_to_hard_fail` passes.
- [ ] `ai_event_error_after_progress_degrades_to_hard_fail` passes.
- [ ] `cargo fmt --all --check` clean.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test` all pass.

## Test plan

- `backend_error_after_progress_degrades_to_hard_fail` in
  `executor/src/agent/mod.rs` tests block — asserts `Ok(hard_fail)` after one
  clean turn + a backend error return on turn 2.
- `ai_event_error_after_progress_degrades_to_hard_fail` in the same block —
  asserts `Ok(hard_fail)` with `AiEvent::Error` on turn 2's event stream.
- Existing `ai_event_error_propagates_as_err` — pins the turn-0 path is still `Err`.

No new `#[cfg(test)]` modules outside the existing bottom-of-file block.

## End-to-end verification

Not applicable — this phase ships a loop behavior change with no CLI-visible
surface (the change is inside `execute_phase`, not in `rexymcp serve` or any
tool output). The hermetic tests in the acceptance criteria are the definitive
verification. The reviewer will run `cargo test` and spot-check the three named
tests pass.

## Authorizations

- [x] May edit `executor/src/governor/hard_fail.rs` — add `BackendError` variant.
- [x] May edit `executor/src/agent/mod.rs` — change two error sites + add two tests.
- [ ] No other files touched.
- [ ] No new dependencies.
- [ ] No `Cargo.toml` edits.

## Out of scope

- Any other `HardFailSignal` variant changes.
- Changing the `Blocker` enum structure.
- Touching the MCP layer or any `mcp/src/` file — the MCP crate already handles
  `Err` from `execute_phase_inner` by surfacing it as an MCP tool error; once this
  phase lands, that path is hit only for turn-0 failures (correct).
- Implementing resume / `continue_phase` (M6 decision 2: not building this).
- Benchmark suite or routing policy (later M7 phases).

## Update Log

### Update — 2025-01-09 (started)

**Executor:** rexyMCP executor LLM
**Tasks:** Implementing all 5 spec tasks — new `HardFailSignal::BackendError` variant, degrade site A (chat_fut error) and site B (AiEvent::Error), add comment to existing test, add two new tests.

### Update — 2025-01-09 (complete)

**Tasks completed:**
- Task 1: Added `HardFailSignal::BackendError { message: String }` variant with `describe()` arm in `executor/src/governor/hard_fail.rs`.
- Task 2: Replaced site A (`chat_fut` error) with `turns`-branching logic — `turns == 0` returns `Err`, `turns > 0` degrades to `Ok(hard_fail_result(...))`.
- Task 3: Replaced site B (`AiEvent::Error`) with same branching pattern.
- Task 4: Added comment above `ai_event_error_propagates_as_err` test.
- Task 5: Added `backend_error_after_progress_degrades_to_hard_fail` (uses new `MockAiClientChatError` mock that returns `Err` from `chat()` on a configured call index) and `ai_event_error_after_progress_degrades_to_hard_fail` (uses `MockAiClientScript` with `AiEvent::Error` on turn 2).

**Verification commands:**
```
cargo fmt --all --check — clean
cargo clippy --all-targets --all-features -- -D warnings — clean
cargo test — 542 passed, 0 failed, 2 ignored
```

**Grep proof for spec-pinned literal:**
```
$ grep -rn 'BackendError' executor/src/
executor/src/governor/hard_fail.rs:29:    BackendError {
executor/src/governor/hard_fail.rs:53:            Self::BackendError { message } => {
executor/src/agent/mod.rs:245:                                HardFailSignal::BackendError { message: e.to_string() };
executor/src/agent/mod.rs:326:                    let signal = HardFailSignal::BackendError { message: e.clone() };
executor/src/agent/mod.rs:1731:                Blocker::HardFail(HardFailSignal::BackendError { .. })
executor/src/agent/mod.rs:1771:                Blocker::HardFail(HardFailSignal::BackendError { .. })
```

**Files changed:**
- `executor/src/governor/hard_fail.rs` — added `BackendError` variant + `describe()` arm
- `executor/src/agent/mod.rs` — degraded site A and site B, added comment to existing test, added `MockAiClientChatError` mock + two new tests
- `docs/dev/milestones/M7-scorecard/phase-01-backend-error-degradation.md` — status `todo` → `review`, update log
- `docs/dev/milestones/M7-scorecard/README.md` — phase table row `todo` → `review`

**Commit:** `fix: degrade mid-phase backend errors to hard_fail with briefing`

**Notes for review:**
- Created `MockAiClientChatError` inside the test module (not in `ai/testing.rs`) to avoid editing an unauthorized file. This mock returns `Err` from `chat()` on a configured call index, exercising the site A (`chat_fut`) error path.
- Both new tests use a `read_file` tool call on turn 0 to ensure `turns > 0` when the error hits on turn 1.
- End-to-end verification: N/A (no CLI-visible surface; hermetic tests are the definitive verification).

### Review verdict — 2026-06-01

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (local, via rexyMCP `execute_phase`)
- **Scope deviations:** none — `MockAiClientChatError` was correctly placed in the test module rather than editing the unauthorized `ai/testing.rs`, and declared in Notes for review (exactly the declare-deviations discipline).
- **Calibration:** none. Both sites match the spec verbatim; the two new tests are real (prime `turns > 0` with a turn-0 `read_file`, assert `Ok(HardFail)` + `BackendError` blocker — would fail under the old unconditional-`Err` behavior). Independent re-run: fmt ✓ · build ✓ (zero warnings) · clippy ✓ · test **542 executor + 131 mcp** ✓. Minor nit (not bounced): the executor's Update Log entries are dated `2025-01-09` — the local model lacks the real date; harmless.
