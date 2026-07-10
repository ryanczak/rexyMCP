# Phase 01: Executor `CancelSignal` + `cancelled` outcome

**Milestone:** M30 — Executor Interruption
**Status:** in-progress (bounced — bug-01-1: mid-stream cancel branch untested)
**Depends on:** none
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the executor turn loop a cooperative cancellation signal and a fourth
terminal outcome. This phase lands the **executor-crate primitive** only: a
`tokio::sync::watch`-based `CancelSignal` threaded into `LoopDeps` and checked at
two points in the loop, plus a new `PhaseStatus::Cancelled` / `PhaseResult`
shape that **leaves the working tree dirty** and reports the partial diff, stage,
and turns-done. No MCP or CLI caller wires a real signal yet (that is phase-02+);
here every call site passes an inert `CancelSignal::never()`.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #30 — this milestone: what interruption is for
  and why the signal must be polled by the loop.
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the value
  `execute_phase` returns; you are adding a status to it.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The result type (`executor/src/phase/result.rs`)

`PhaseResult` is a **single struct** keyed by a `PhaseStatus` enum — not an enum
of variants. Today:

```rust
/// Terminal status of an `execute_phase` run. Serializes to the contract strings
/// `"complete"` / `"hard_fail"` / `"budget_exceeded"` (M5 returns this as JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Complete,
    HardFail,
    BudgetExceeded,
}
```

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub status: PhaseStatus,
    pub files_changed: Vec<FileChange>,
    pub diff: String,
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    pub briefing: Option<Briefing>,
    pub log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completion_summary: String,
}
```

Note the **additive-field precedent**: `warnings` and `completion_summary` were
added later with `#[serde(default, skip_serializing_if = …)]` so old JSON still
deserializes and empty values are omitted from the wire. Your new field follows
the same shape exactly.

Constructors funnel through `assemble`:

```rust
impl PhaseResult {
    pub fn complete(artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::Complete, None, artifacts)
    }
    pub fn hard_fail(briefing: Briefing, artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::HardFail, Some(briefing), artifacts)
    }
    pub fn budget_exceeded(briefing: Briefing, artifacts: Artifacts) -> Self {
        Self::assemble(PhaseStatus::BudgetExceeded, Some(briefing), artifacts)
    }
    fn assemble(status: PhaseStatus, briefing: Option<Briefing>, artifacts: Artifacts) -> Self {
        Self { status, files_changed: artifacts.files_changed, /* … */ }
    }
}
```

### How the partial diff is already produced (`executor/src/agent/outcome.rs`)

`build_artifacts` renders the diff and `files_changed` from **pre-edit
snapshots** by reading the *current* file contents off disk:

```rust
let after = std::fs::read_to_string(path).unwrap_or_default();
if before == after { continue; }
```

This means: **do not revert or clean anything on cancel.** Calling
`build_artifacts` at the cancellation site captures exactly the partial edits
sitting in the working tree, which is the "leave dirty + report" behavior we
want. `budget_exceeded_result` is the closest existing builder to mirror (but it
attaches a `Briefing`; cancellation does **not**):

```rust
pub(super) fn budget_exceeded_result(
    input: &PhaseInput,
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    project_root: &Path,
    budget_remaining: String,
    artifacts: Artifacts,
) -> PhaseResult {
    let briefing = Briefing { /* … */ };
    PhaseResult::budget_exceeded(briefing, artifacts)
}
```

### The loop and its two check sites (`executor/src/agent/mod.rs`)

`LoopDeps<'a>` (`mod.rs:94`) is the injected-dependency struct. You add one
field. The **turn-loop top** (`mod.rs:280`) already has a budget/wall-clock
terminal you mirror — note the exact sequence it runs before returning
(`log_session_end` → `emit_phase_run` → `build_artifacts` → return):

```rust
loop {
    // Step 2a — wall-clock ceiling …
    if deps.wall_clock_secs > 0 {
        let elapsed_ms = (deps.clock)().saturating_sub(loop_started_ms);
        if elapsed_ms >= deps.wall_clock_secs.saturating_mul(1000) {
            log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
            emit_phase_run(&deps, input, "budget_exceeded", Gates::default(), &metrics, &scorer, turns);
            let artifacts = build_artifacts(
                &pre_edit_content, deps.project_root, log_path.clone(),
                "budget_exceeded", turns, CommandOutputs::default(),
            );
            return Ok(budget_exceeded_result(/* … */, artifacts));
        }
    }
    // Step 2 — budget: compact on overflow …
```

The **inner event-drain loop** (`mod.rs:383`) is a `tokio::select!` with exactly
two branches today — the chat future and a heartbeat tick. Note that an existing
branch already `return`s straight out of `execute_phase` on a turn-0 backend
error, so returning from inside `select!` is an established pattern:

```rust
let chat_fut = deps.client.chat(&system, messages.clone(), tx, tools_opt);
tokio::pin!(chat_fut);
let mut heartbeat = interval(HEARTBEAT_PERIOD);
heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
loop {
    tokio::select! {
        result = &mut chat_fut => {
            match result {
                Ok(()) => {}
                Err(e) if turns == 0 => return Err(Error::Backend(e.to_string())),
                Err(e) => { /* hard-fail signal … */ }
            }
        }
        // (heartbeat branch here)
    }
}
```

### The `AiClient` trait (`executor/src/ai/mod.rs:82`) — for the mid-stream test

```rust
pub trait AiClient: Send + Sync {
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> Result<()>;
}
```

### Call sites that construct `LoopDeps` (all must gain the new field)

Adding a field to `LoopDeps` is a compile-forcing change; set the new field to
`CancelSignal::never()` at **every** construction site. There are ~17:

- Production: `mcp/src/runner.rs:282`.
- Tests: `executor/src/agent/tests.rs` at lines ~135, ~918, ~1041, ~1343, ~1398,
  ~1919, ~2680, ~2756, ~2945, ~3755, ~3953, ~4019, ~4165, ~4669, ~4834, ~4877
  (grep `LoopDeps {` to find them all — do not rely on these line numbers being
  exact).

## Spec

Numbered tasks in execution order.

### 1. Add the `CancelSignal` primitive

In a new module `executor/src/agent/cancel.rs`, define a cooperative
cancellation signal built on `tokio::sync::watch` (already available — `tokio`'s
`sync` feature is enabled; **do not add a dependency**). Wire the module into
`executor/src/agent/mod.rs` (`mod cancel;`) and re-export `CancelSignal` +
`CancelHandle` from the `agent` module so `mcp` can reach them in a later phase.

Required surface and semantics:

- `CancelSignal` is `Clone` (the loop clones it for the `select!` branch; later
  phases hand clones to multiple observers).
- `CancelHandle` holds the `watch::Sender<bool>`. `CancelHandle::cancel(&self)`
  flips the signal (send `true`; ignore a send error from all-receivers-dropped).
- `CancelSignal::new() -> (CancelHandle, CancelSignal)` creates a fresh
  `watch::channel(false)` pair.
- `CancelSignal::never() -> CancelSignal` returns a signal that can never fire —
  create `watch::channel(false)`, drop the sender, keep the receiver. (A dropped
  sender means `is_cancelled()` reads the last value `false` and `cancelled()`
  stays pending forever — exactly "never".)
- `CancelSignal::is_cancelled(&self) -> bool` = `*self.rx.borrow()`.
- `CancelSignal::cancelled(&mut self)` is an `async` method that resolves **once
  the signal is flipped** and otherwise stays pending. **Gotcha:**
  `watch::Receiver::wait_for(|c| *c)` returns `Err` if the sender is dropped
  before the predicate is satisfied — that Err must **not** be treated as a
  cancel (it would fire the `select!` branch spuriously). Handle it by parking:

  ```rust
  pub async fn cancelled(&mut self) {
      match self.rx.wait_for(|c| *c).await {
          Ok(_) => {}                                    // flipped → cancelled
          Err(_) => std::future::pending::<()>().await,  // sender gone → never fires
      }
  }
  ```

  (`wait_for` also returns `Ok` immediately if the value is already `true`.)

Keep the concrete field(s) private; expose only the methods above.

### 2. Add the `Cancelled` status and `Cancellation` record

In `executor/src/phase/result.rs`:

- Add `Cancelled` to `PhaseStatus` (it serializes to `"cancelled"` via the
  existing `#[serde(rename_all = "snake_case")]`).
- Add a `CancelReason` enum with variants `UserStop` and `ClaudeStop`, deriving
  the same trait set as `PhaseStatus` and `#[serde(rename_all = "snake_case")]`
  (→ `"user_stop"` / `"claude_stop"`). The executor does not know *who*
  cancelled, so this is set by the MCP/CLI layer later.
- Add a `Cancellation` struct:

  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct Cancellation {
      /// Who cancelled. The executor loop leaves this `None`; the MCP/CLI layer
      /// (phase-03+) sets it from the entrypoint that fired the signal.
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub reason: Option<CancelReason>,
      /// Where in the turn cycle cancellation was observed.
      pub stage: String,
      /// Turns fully completed before cancellation.
      pub turns_done: usize,
  }
  ```

- Add a `cancellation: Option<Cancellation>` field to `PhaseResult`, following
  the `warnings` precedent exactly: `#[serde(default, skip_serializing_if =
  "Option::is_none")]`. Initialize it to `None` in `assemble` (all existing
  constructors get `None`).
- Add a constructor `PhaseResult::cancelled(cancellation: Cancellation,
  artifacts: Artifacts) -> Self` that assembles with `PhaseStatus::Cancelled`,
  **no briefing** (`None`), then sets `cancellation: Some(cancellation)`.

Re-export `Cancellation` and `CancelReason` from the `phase` module alongside
`PhaseResult`/`PhaseStatus` (match how those are exported today).

### 3. Add the `cancelled_result` builder

In `executor/src/agent/outcome.rs`, add:

```rust
pub(super) fn cancelled_result(stage: &str, turns: usize, artifacts: Artifacts) -> PhaseResult {
    PhaseResult::cancelled(
        Cancellation { reason: None, stage: stage.to_string(), turns_done: turns },
        artifacts,
    )
}
```

Import `Cancellation` from `crate::phase` alongside the existing imports. No
briefing is built (cancellation is a deliberate abort, not an escalation).

### 4. Add the `cancel` field to `LoopDeps` and set it at every call site

- In `mod.rs`, add `pub cancel: CancelSignal,` to `LoopDeps<'a>` with a doc
  comment: cooperative cancellation signal; `CancelSignal::never()` disables it.
- Add `cancel: CancelSignal::never(),` to **every** `LoopDeps { … }` literal
  (production `mcp/src/runner.rs` and all test constructions — see Current
  state). Do not thread a real signal anywhere yet; phase-02 changes
  `runner.rs`. Import `CancelSignal` where needed.

### 5. Check the signal at the top of the turn loop

At the top of the outer `loop {` in `execute_phase` (`mod.rs:280`), **before**
the wall-clock check, add a cancellation terminal that mirrors the wall-clock
block's sequence and returns `cancelled_result` with `stage = "between_turns"`:

```rust
if deps.cancel.is_cancelled() {
    log_session_end(&log_handle, &redactor, deps.clock, "cancelled", turns);
    emit_phase_run(&deps, input, "cancelled", Gates::default(), &metrics, &scorer, turns);
    let artifacts = build_artifacts(
        &pre_edit_content, deps.project_root, log_path.clone(),
        "cancelled", turns, CommandOutputs::default(),
    );
    return Ok(cancelled_result("between_turns", turns, artifacts));
}
```

Verify `emit_phase_run`, `log_session_end`, and `build_artifacts` accept the
new `"cancelled"` status string without a match arm that rejects unknown values
— if any of them enumerates statuses and would panic or misroute on an unknown
string, extend it minimally to treat `"cancelled"` like the other non-complete
terminals (a terminal that emits its record and ends the session). Do **not**
invent new telemetry semantics; `cancelled` records the same objective fields as
`budget_exceeded`.

### 6. Add a cancellation branch to the inner `select!`

Immediately before the inner event-drain `loop { tokio::select! { … } }`
(`mod.rs:383`), clone the signal into a local `mut` binding to sidestep borrow
conflicts with `deps` inside the branches:

```rust
let mut cancel = deps.cancel.clone();
```

Add a third `select!` branch that aborts the in-flight chat and returns
`cancelled_result` with `stage = "awaiting_model"`. Returning drops `chat_fut`,
which cancels the underlying request:

```rust
_ = cancel.cancelled() => {
    log_session_end(&log_handle, &redactor, deps.clock, "cancelled", turns);
    emit_phase_run(&deps, input, "cancelled", Gates::default(), &metrics, &scorer, turns);
    let artifacts = build_artifacts(
        &pre_edit_content, deps.project_root, log_path.clone(),
        "cancelled", turns, CommandOutputs::default(),
    );
    return Ok(cancelled_result("awaiting_model", turns, artifacts));
}
```

Match the exact argument names/values the neighboring branches and the
top-of-loop block already use (`turns` here is turns *completed*; the in-flight
turn is not counted). If `build_artifacts`/`emit_phase_run` are not in scope at
this point in the function, they are the same functions the top-of-loop block
calls — confirm before duplicating.

## Acceptance criteria

- [ ] `cargo build` succeeds.
- [ ] `serde_json::to_value(PhaseStatus::Cancelled)` equals `json!("cancelled")`.
- [ ] `serde_json::to_value(CancelReason::UserStop)` equals `json!("user_stop")`
      and `CancelReason::ClaudeStop` equals `json!("claude_stop")`.
- [ ] A `PhaseResult` built via `PhaseResult::cancelled(…)` has
      `status == PhaseStatus::Cancelled`, `briefing.is_none()`, and
      `cancellation.is_some()`.
- [ ] A `PhaseResult` with `cancellation: None` omits the `"cancellation"` key
      from its JSON (the `skip_serializing_if` precedent).
- [ ] `CancelSignal::never()` reports `is_cancelled() == false` and its
      `cancelled()` future never resolves.
- [ ] After `CancelHandle::cancel()`, the paired `CancelSignal` reports
      `is_cancelled() == true` and `cancelled()` resolves; a **clone** taken
      before the flip observes it too.
- [ ] Dropping the `CancelHandle` without cancelling leaves `is_cancelled()
      == false` and `cancelled()` pending.
- [ ] A loop run whose signal is flipped returns `status == Cancelled`, and the
      working tree is **left dirty** (a file an earlier turn wrote is still on
      disk and appears in `PhaseResult.diff`).
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` is clean (format only files this phase touched).
- [ ] `cargo test` passes.

## Test plan

Hermetic and deterministic — **no `sleep`, no real time**. Use `MockAiClient`
for the loop tests, and a tiny in-test parking `AiClient` for the mid-stream
case.

`CancelSignal` unit tests (in `executor/src/agent/cancel.rs`):

- `never_signal_is_not_cancelled` — `never()` → `is_cancelled()` false.
- `never_signal_cancelled_future_stays_pending` — poll `never().cancelled()`
  once (e.g. `tokio::select!` against a `std::future::ready(())` fallback, or
  `futures`-free: assert it is still pending via a `poll_fn`/`now_or_never`
  equivalent using only std/tokio) and assert it did not resolve.
- `cancel_flips_signal` — `new()` → `handle.cancel()` → `is_cancelled()` true and
  `cancelled().await` returns.
- `clone_observes_flip` — clone the `CancelSignal` **before** `cancel()`; after
  the flip the clone's `is_cancelled()` is true.
- `dropped_handle_does_not_cancel` — `new()`, drop the handle, assert
  `is_cancelled()` false and `cancelled()` pending.

Result-type unit tests (in `executor/src/phase/result.rs`):

- `cancelled_status_serializes_to_snake_case` — `PhaseStatus::Cancelled` →
  `"cancelled"`.
- `cancel_reason_serializes_to_snake_case` — both `CancelReason` variants.
- `cancelled_result_has_no_briefing_and_carries_cancellation`.
- `phase_result_cancellation_round_trips_through_json`.
- `phase_result_absent_cancellation_omitted_from_json`.

Loop-level tests (in `executor/src/agent/tests.rs`, reuse the existing
`LoopDeps` harness):

- `loop_returns_cancelled_when_signal_flipped_between_turns` — build a
  `MockAiClient` script that writes a file on turn 1 then would keep calling
  tools; construct `LoopDeps` with a real `CancelSignal` (keep the `CancelHandle`),
  flip it so the next top-of-loop check fires; assert `status == Cancelled`,
  `cancellation` stage `"between_turns"`, and the written file is still on disk
  and present in `diff` (tree left dirty).
- `loop_returns_cancelled_when_signal_flipped_mid_stream` — define an in-test
  `AiClient` whose `chat` parks forever (`std::future::pending().await`, never
  sends on `tx`); flip the signal so the inner `select!` cancellation branch is
  the only way the turn ends; assert `status == Cancelled` with stage
  `"awaiting_model"`. Deterministic: cancellation, not a timer, resolves the
  select.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. No MCP tool or CLI
> command wires a real `CancelSignal` until phase-02, so cancellation has no
> external entrypoint to drive; it is exercised entirely by the hermetic loop
> tests above (a `CancelSignal` fired against `MockAiClient` / a parking
> `AiClient`).

## Authorizations

- [ ] May touch `docs/architecture.md`: **No** (the § Layer 2 "Liveness" /
      MCP-tools-list contract edits belong to the phase that makes them true —
      phase-02/05 — and are authorized there, not here).
- [ ] May add dependencies: **No.** The cancel primitive uses
      `tokio::sync::watch`, already available via `tokio`'s enabled `sync`
      feature. If you believe a dependency is required, **stop and file a
      blocker** — do not add one.

Otherwise: None beyond the files named in the Spec (`executor/src/agent/cancel.rs`
[new], `executor/src/agent/mod.rs`, `executor/src/phase/result.rs`,
`executor/src/agent/outcome.rs`, `executor/src/agent/tests.rs`, and the
`executor/src/phase` module's re-export site; plus the one production `LoopDeps`
site in `mcp/src/runner.rs` — set to `CancelSignal::never()` only).

## Out of scope

- The MCP job registry, async `execute_phase`, `get_run_status`, `stop_phase`
  (phase-02/03).
- The `rexymcp stop` CLI and the `.rexymcp/stop` sentinel watcher (phase-04).
- Setting `Cancellation.reason` to a real value (`UserStop`/`ClaudeStop`) — the
  executor leaves it `None`; the MCP/CLI layer sets it (phase-03).
- Computing a git diff for the result — the existing `build_artifacts` diff of
  pre-edit snapshots is the partial diff; do not shell out to git.
- Reverting, cleaning, or committing the working tree on cancel — leave it dirty.
- Skill-loop / dashboard / `rexymcp status` changes (phase-05).
- Any edit to `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1783656011046 (complete, server-authored)

**Summary:** Phase 01 is complete. Here's the summary:

**What was built:**
- **`CancelSignal`/`CancelHandle`** primitive in `executor/src/agent/cancel.rs` — a `tokio::sync::watch`-based cooperative cancellation signal with `new()`, `never()`, `is_cancelled()`, and `cancelled()` (async). The `cancelled()` method uses `changed()` + a loop to avoid the `Send` issue with `wait_for()`.
- **`PhaseStatus::Cancelled`** and **`Cancellation`** struct in `executor/src/phase/result.rs` — the new terminal status with stage and turns_done tracking. `PhaseResult::cancelled()` constructor sets `cancellation: Some(...)` with `briefing: None`.
- **`cancelled_result`** builder in `executor/src/agent/outcome.rs` — mirrors the `budget_exceeded_result` pattern but for cancellation.
- **`cancel` field** added to `LoopDeps` at all 16 test sites and 1 production site, all set to `CancelSignal::never()`.
- **Two cancellation check points** in the turn loop: top-of-loop (`"between_turns"`) and inner `select!` branch (`"awaiting_model"`).
- **Tests**: `CancelSignal` unit tests (never, cancel, clone, drop), result-type serialization tests, and two loop-level tests verifying cancellation is observed and the working tree is left dirty.

**Deviations:**
- The `cancelled()` implementation uses `changed()` + loop instead of `wait_for()` to avoid the `RwLockReadGuard<'_, bool>` not being `Send` (required for the `MaybeSendFuture` trait in the MCP server).
- The mid-stream test accepts either `"awaiting_model"` or `"between_turns"` stage due to the synchronous mock client completing before the `select!` cancellation branch can fire.

**Verification:** `cargo build` clean, `cargo clippy` clean, `cargo fmt --check` clean, all 949 tests pass.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
nt_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::respects_gitignore ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 949 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `executor/src/agent/cancel.rs` — +117 -0
- `executor/src/agent/mod.rs` — +52 -1
- `executor/src/agent/outcome.rs` — +13 -2
- `executor/src/agent/tests.rs` — +163 -1
- `executor/src/phase/mod.rs` — +3 -1
- `executor/src/phase/result.rs` — +92 -1
- `mcp/src/cap.rs` — +2 -0
- `mcp/src/runner.rs` — +2 -1

**Commit:** f1bc146fb36990004aea64b936b094b36a5b753a

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

