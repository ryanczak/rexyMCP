# Phase 03: `stop_phase` MCP tool + real `CancelSignal` threading + reason stamping

**Milestone:** M30 — Executor Interruption
**Status:** done
**Depends on:** phase-01 (the executor `CancelSignal` primitive + `Cancelled` outcome), phase-02 (the job registry + async `execute_phase`)
**Estimated diff:** ~470 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

Give the **architect** a working mid-flight abort. Phase-02 made `execute_phase`
spawn a run under a `run_id`, but that run still uses `CancelSignal::never()`
(`mcp/src/runner.rs:306`) — nothing can stop it. This phase:

1. Threads a **real** `CancelSignal` from the spawned `execute_phase` run down to
   the executor loop's `LoopDeps.cancel`.
2. Stores each run's `CancelHandle` in the `JobRegistry` so it can be fired later.
3. Adds a new `stop_phase(run_id)` MCP tool that fires that handle and records the
   `ClaudeStop` reason.
4. Stamps that `CancelReason` into the run's terminal `cancelled` `PhaseResult`
   (phase-01 left `Cancellation.reason` `None` for the MCP layer to fill).

After this phase the architect can, between `get_run_status` polls, call
`stop_phase(run_id)` and the next poll returns a `PhaseResult` with
`status: "cancelled"`, `cancellation.reason: "claude_stop"`, and the partial diff.

This phase does **not** add the filesystem stop sentinel or the `rexymcp stop`
CLI (the human/`UserStop` path) — that is phase-04. The CLI `run-phase` keeps
`CancelSignal::never()` here.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #30 — the M30 async-job + cancel design; note
  "`stop_phase(run_id)` cancels the run's in-memory `CancelSignal` handle" and the
  `cancelled` result "leaves the working tree dirty" with partial diff + stage +
  turns-done.
- `docs/architecture.md` § Layer 2 "`mcp` crate" tools list — the `stop_phase`
  bullet was **already added by the architect** at draft time (do not edit
  architecture.md; the tool you build must match that description).
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the
  `cancelled` status and `cancellation` field (already present from phase-01).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Verify the `#[rmcp::tool]` macro shape against the working `get_run_status`
   tool** (`mcp/src/server.rs:580`) rather than inventing it. `stop_phase` is the
   *same* shape as `get_run_status`: a `#[rmcp::tool(description = "…")]` async
   method inside the existing `#[rmcp::tool_router] impl RexyMcpServer` block,
   taking `Parameters(params): Parameters<StopPhaseParams>` and returning
   `Result<Json<StopPhaseOutput>, String>`. Because it is a `#[rmcp::tool]` method
   it is **auto-listed** by `Self::tool_router().list_all()` — you do **not** edit
   `list_tools` / `get_tool` for it (those explicit branches exist only for
   `execute_phase` / `continue_phase`, which need roots corroboration). If the
   installed `rmcp` version's macro diverges from `get_run_status`'s shape, trust
   the working code over this sketch and note it in "Notes for review".

## Current state

### The job registry has no cancel handle (`mcp/src/jobs.rs`)

The registry tracks only run *state*; there is nothing to cancel a run with:

```rust
/// Per-run control block held in the registry.
struct RunEntry {
    state_tx: watch::Sender<RunState>,
}
```

`insert` creates the `RunEntry`, `spawn_run` registers + spawns the work and
publishes its terminal `RunState`:

```rust
pub fn insert(&self, run_id: &str) {
    let (state_tx, _rx) = watch::channel(RunState::Running);
    self.lock()
        .insert(run_id.to_string(), RunEntry { state_tx });
}
// …
pub fn spawn_run<F>(registry: Arc<JobRegistry>, run_id: String, work: F)
where
    F: std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'static,
{
    registry.insert(&run_id);
    tokio::spawn(async move {
        let state = match work.await {
            Ok(json) => RunState::Complete(json),
            Err(e) => RunState::Failed(e),
        };
        registry.publish(&run_id, state);
    });
}
```

### The spawned run uses a dead cancel signal (`mcp/src/runner.rs:306`)

`run_phase_with` builds `LoopDeps` with a `never()` signal — the one line this
phase makes live:

```rust
    let deps = LoopDeps {
        // …
        task_tracking: cfg.executor.task_tracking,
        cancel: CancelSignal::never(),   // ← becomes inp.cancel.clone()
    };
```

### The `execute_phase` branch spawns work with no signal (`mcp/src/server.rs:664`)

```rust
let run_id = crate::jobs::new_run_id();
let config_path_owned = config_path.clone();
let params_owned = params.clone();
let work = async move {
    execute_phase_inner(
        &config_path_owned,
        &params_owned,
        progress_callback.as_deref(),
    )
    .await
    .map(|o| o.result)
};
crate::jobs::spawn_run(runs.clone(), run_id.clone(), work);
```

### The executor-crate cancel primitive (phase-01, `executor/src/agent/cancel.rs`)

Already shipped — **do not modify it**. The API you build on:

```rust
pub struct CancelHandle { /* … */ }
impl CancelHandle {
    pub fn cancel(&self) { /* flips the signal; ignores all-receivers-dropped */ }
}

#[derive(Clone)]
pub struct CancelSignal { /* … */ }
impl CancelSignal {
    pub fn new() -> (CancelHandle, CancelSignal) { /* handle + signal, starts false */ }
    pub fn never() -> CancelSignal { /* disabled: sender dropped */ }
    pub fn is_cancelled(&self) -> bool { /* … */ }
}
```

`CancelSignal` is `Clone` (so it can be handed to multiple config layers).
`CancelHandle` is **not** `Clone` — there is exactly one per run, and it lives in
the registry.

Exports (use these exact paths):
- `use rexymcp_executor::agent::{CancelHandle, CancelSignal};`
- `use rexymcp_executor::phase::CancelReason;`

### The cancellation reason type (phase-01, `executor/src/phase/result.rs:20`)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    UserStop,
    ClaudeStop,
}

pub struct Cancellation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CancelReason>,   // ← executor leaves None; this phase fills it
    pub stage: String,
    pub turns_done: usize,
}
```

`serde_json::to_value(CancelReason::ClaudeStop)` == `json!("claude_stop")`
(verified by `cancel_reason_serializes_to_snake_case` in `result.rs`). The
executor already emits a `cancelled` `PhaseResult` whose serialized JSON is
`{ "status": "cancelled", …, "cancellation": { "stage": "...", "turns_done": N } }`
(no `reason` key, because it is `None` and `skip_serializing_if`). This phase
inserts the `reason` key after a stop.

## Spec

Numbered tasks in execution order. **This phase adds two struct fields used at
many call sites; build after each site group as instructed to stay inside the
verifier's consecutive-failure budget.**

### 1. Give the registry a cancel handle + stop reason (`mcp/src/jobs.rs`)

Add `CancelHandle` + a recorded stop reason to `RunEntry`, and the methods to
fire and read them. Import at the top of `jobs.rs`:

```rust
use rexymcp_executor::agent::CancelHandle;
use rexymcp_executor::phase::CancelReason;
```

Change `RunEntry` and `insert`:

```rust
/// Per-run control block held in the registry.
struct RunEntry {
    state_tx: watch::Sender<RunState>,
    /// Fires the run's cooperative cancel signal. `None` is never stored — every
    /// registered run owns a handle (a `never()`-signal handle for runs that are
    /// not cancellable, e.g. tests).
    cancel: CancelHandle,
    /// Set by `request_stop`; read by `spawn_run` to stamp the terminal result.
    stop_reason: Option<CancelReason>,
}

/// Register a fresh run in `Running`, holding its cancel handle. Call before
/// spawning so a racing `get_run_status` / `stop_phase` always finds the id.
pub fn insert(&self, run_id: &str, cancel: CancelHandle) {
    let (state_tx, _rx) = watch::channel(RunState::Running);
    self.lock().insert(
        run_id.to_string(),
        RunEntry {
            state_tx,
            cancel,
            stop_reason: None,
        },
    );
}
```

Add two methods to `impl JobRegistry` (near `publish`):

```rust
/// Fire a run's cancel signal and record why. Returns `false` for an unknown id.
/// Firing an already-terminal run's handle is a harmless no-op (all receivers are
/// gone) — this returns `true` because the run existed, but nothing is re-stamped.
pub fn request_stop(&self, run_id: &str, reason: CancelReason) -> bool {
    if let Some(entry) = self.lock().get_mut(run_id) {
        entry.stop_reason = Some(reason);
        entry.cancel.cancel();
        true
    } else {
        false
    }
}

/// The reason recorded by a prior `request_stop`, if any. Read by `spawn_run`
/// when a run finishes so a `cancelled` result can be stamped.
fn recorded_reason(&self, run_id: &str) -> Option<CancelReason> {
    self.lock().get(run_id).and_then(|e| e.stop_reason.clone())
}
```

Change `spawn_run` to take the handle and stamp the reason on a `cancelled`
result:

```rust
/// Spawn `work` as run `run_id`, holding `cancel_handle` in the registry so
/// `request_stop` can fire it. Publishes the terminal state when `work` finishes;
/// if the run was stopped and came back `cancelled`, stamps the recorded reason
/// into the result JSON's `cancellation.reason`.
pub fn spawn_run<F>(
    registry: Arc<JobRegistry>,
    run_id: String,
    cancel_handle: CancelHandle,
    work: F,
) where
    F: std::future::Future<Output = Result<serde_json::Value, String>> + Send + 'static,
{
    registry.insert(&run_id, cancel_handle);
    tokio::spawn(async move {
        let state = match work.await {
            Ok(mut json) => {
                stamp_cancel_reason(&mut json, registry.recorded_reason(&run_id));
                RunState::Complete(json)
            }
            Err(e) => RunState::Failed(e),
        };
        registry.publish(&run_id, state);
    });
}

/// If `reason` is set and `json` is a `cancelled` PhaseResult, insert
/// `cancellation.reason`. No-op otherwise (a run that completed normally before
/// observing the stop keeps no reason — the status race is resolved in favor of
/// the observed terminal status).
fn stamp_cancel_reason(json: &mut serde_json::Value, reason: Option<CancelReason>) {
    let Some(reason) = reason else { return };
    if json.get("status").and_then(|s| s.as_str()) != Some("cancelled") {
        return;
    }
    if let Some(obj) = json.get_mut("cancellation").and_then(|c| c.as_object_mut()) {
        if let Ok(v) = serde_json::to_value(reason) {
            obj.insert("reason".to_string(), v);
        }
    }
}
```

**Build after Task 1** — `insert` and `spawn_run` signatures changed. Their only
non-test caller is the `execute_phase` branch (Task 3); the `jobs.rs` tests call
them directly and are updated in Task 6. Expect this build to fail only at the
`execute_phase` branch and the `jobs.rs` tests until those tasks land — that is
fine; do not "fix" it by reverting the signature. If you want a green build
between tasks, do Task 3 immediately after Task 1.

### 2. Thread a `CancelSignal` field through the config structs (`mcp/src/runner.rs`)

Add a `cancel: CancelSignal` field to **both** `RunPhaseConfig` and
`AssemblyInput`, resolve it into `LoopDeps.cancel`. `CancelSignal` is already
imported in `runner.rs` (line 8).

**2a.** `RunPhaseConfig` (the `pub struct` ~line 327) — add the field with a doc
line, at the end:

```rust
    /// Resume context for `continue_phase`. `None` on a normal `execute_phase`.
    pub resume: Option<crate::resume::ResumeContext>,
    /// Cooperative cancel signal for the spawned async run. `CancelSignal::never()`
    /// disables it (the CLI `run-phase` path and `continue_phase`, for now).
    pub cancel: CancelSignal,
```

**2b.** `AssemblyInput` (the `struct` ~line 103) — add the field at the end:

```rust
    resume: Option<&'a crate::resume::ResumeContext>,
    cancel: CancelSignal,
```

**2c.** `run_phase` builds `AssemblyInput` (~line 392) — set it from the config
(clone, because `inp` is a shared `&RunPhaseConfig`):

```rust
    let assembly = AssemblyInput {
        // … existing fields …
        cancel: inp.cancel.clone(),
    };
```

**2d.** `run_phase_with` builds `LoopDeps` (line 306) — replace the dead signal:

```rust
        cancel: inp.cancel.clone(),
```

(`inp` here is `&AssemblyInput`, and `CancelSignal` is `Clone`.)

**Build after Task 2** — `RunPhaseConfig` and `AssemblyInput` now have a required
field, so **every** literal constructor of them stops compiling until set. The
complete grep-verified list (fix all before building):

- `RunPhaseConfig` literals (add `cancel: …`):
  - `mcp/src/server.rs:180` (in `execute_phase_inner_with_client`) → `cancel,`
    (the new param from Task 3 — until Task 3, temporarily `cancel: CancelSignal::never()`).
  - `mcp/src/server.rs:233` (`continue_phase_inner`) → `cancel: CancelSignal::never()`.
  - `mcp/src/main.rs:394` (CLI `run-phase`) → `cancel: CancelSignal::never()`.
- `AssemblyInput` literals:
  - `mcp/src/runner.rs:392` (prod `run_phase`) → `cancel: inp.cancel.clone()` (Task 2c).
  - `mcp/src/runner.rs` test sites — 7 of them, in the `#[cfg(test)] mod tests`
    block (near lines 597, 653, 709, 773, 827, 899, 1026): each gets
    `cancel: CancelSignal::never()`. (Verify with
    `grep -n "AssemblyInput {" mcp/src/runner.rs`; the count is the source of
    truth, not these line numbers.)

`main.rs` and `server.rs` need the import — add
`use rexymcp_executor::agent::CancelSignal;` where each pulls in executor types
(`main.rs`) / near the other `rexymcp_executor` imports (`server.rs`). The
`runner.rs` tests already have `CancelSignal` in scope via the module import.

### 3. Wire the real signal into the `execute_phase` branch (`mcp/src/server.rs`)

Import at the top of `server.rs`:

```rust
use rexymcp_executor::agent::CancelSignal;
use rexymcp_executor::phase::CancelReason;
```

**3a.** In the `execute_phase` branch (the `let run_id = …` block at ~line 664),
create the signal pair, call `execute_phase_inner_with_client` **directly** (so
the real signal reaches it without changing `execute_phase_inner`'s public
signature), and pass the handle to `spawn_run`:

```rust
let run_id = crate::jobs::new_run_id();
let (cancel_handle, cancel_signal) = CancelSignal::new();
let config_path_owned = config_path.clone();
let params_owned = params.clone();
let work = async move {
    execute_phase_inner_with_client(
        &config_path_owned,
        &params_owned,
        progress_callback.as_deref(),
        None,
        cancel_signal,
    )
    .await
    .map(|o| o.result)
};
crate::jobs::spawn_run(runs.clone(), run_id.clone(), cancel_handle, work);
```

**3b.** Add the `cancel: CancelSignal` parameter to
`execute_phase_inner_with_client` (~line 159) and pass it into the
`RunPhaseConfig` (this replaces the Task-2 placeholder at server.rs:180):

```rust
pub(crate) async fn execute_phase_inner_with_client(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
    test_client: Option<&dyn AiClient>,
    cancel: CancelSignal,
) -> Result<ExecutePhaseOutput, String> {
    // … unchanged up to the RunPhaseConfig literal …
    let result = runner::run_phase(&runner::RunPhaseConfig {
        // … existing fields …
        test_client,
        resume: None,
        cancel,
    })
    // …
}
```

**3c.** `execute_phase_inner` (~line 150) keeps its signature and forwards a dead
signal (so its own callers — two `server_tests.rs` sites + the doc's testable
path — are untouched):

```rust
pub(crate) async fn execute_phase_inner(
    config_path: &Path,
    params: &ExecutePhaseParams,
    progress: Option<&dyn ProgressCallback>,
) -> Result<ExecutePhaseOutput, String> {
    execute_phase_inner_with_client(config_path, params, progress, None, CancelSignal::never()).await
}
```

**3d.** The two `server_tests.rs` callers of `execute_phase_inner_with_client`
(currently `…, None).await` at ~lines 649 and 692) gain the signal argument:
`…, None, CancelSignal::never()).await`. Add the import to that test module.

**Build after Task 3** — the whole non-`stop_phase` surface should compile now.

### 4. Add the `stop_phase` tool + its param/output types (`mcp/src/server.rs`)

Mirror `GetRunStatusParams` / `GetRunStatusOutput` (search `server.rs` for them)
for the derives — same `#[derive(...)]` set, same `Parameters`/`Json` wrapping.

Add the param + output types near `GetRunStatusParams`:

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct StopPhaseParams {
    /// The `run_id` returned by `execute_phase`.
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StopPhaseOutput {
    /// `true` if a run with that id existed and its cancel was fired; `false`
    /// for an unknown `run_id`.
    pub stopped: bool,
}
```

(Use whatever derive/attribute set `GetRunStatusParams`/`GetRunStatusOutput`
actually use — copy it exactly; the block above is the intent, not a licence to
diverge from the working types.)

Add the tool method inside the `#[rmcp::tool_router] impl RexyMcpServer` block,
right after `get_run_status` (~line 590):

```rust
    #[rmcp::tool(
        description = "Stop a spawned execute_phase run by run_id: fires the run's cooperative cancel signal so it aborts at the next turn boundary (or mid model-stream) and returns a PhaseResult with status \"cancelled\", cancellation.reason \"claude_stop\", and the partial diff (working tree left dirty). Returns {stopped:true} if the run_id was known, {stopped:false} if not. The cancel is cooperative and asynchronous — poll get_run_status to observe the terminal cancelled result."
    )]
    async fn stop_phase(
        &self,
        Parameters(params): Parameters<StopPhaseParams>,
    ) -> Result<Json<StopPhaseOutput>, String> {
        let stopped = self
            .runs
            .request_stop(&params.run_id, CancelReason::ClaudeStop);
        Ok(Json(StopPhaseOutput { stopped }))
    }
```

Because this is a `#[rmcp::tool]` method it is auto-listed — **no** `list_tools`
or `get_tool` edit.

### 5. (Architect-owned — nothing to do) `architecture.md` tools list

The `stop_phase` bullet in `docs/architecture.md` § Layer 2 was added by the
architect at draft time. **Do not edit `architecture.md`.**

### 6. Update `jobs.rs` tests for the new `insert` / `spawn_run` signatures

The existing `jobs.rs` tests call `registry.insert("r1")` and
`spawn_run(registry, id, work)` — update them to the new signatures using a
throwaway handle, e.g.:

```rust
let (handle, _signal) = CancelSignal::new();
registry.insert("r1", handle);
```

For `spawn_run` tests, thread a handle in as the third argument. Keep every
existing assertion (`await_terminal_*`, `spawn_run_publishes_*`) — they still
hold. `use rexymcp_executor::agent::CancelSignal;` in the test module.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean, with
      **no** `#[allow(...)]` added anywhere.
- [ ] `cargo fmt --all --check` is clean.
- [ ] `cargo test` passes (existing + new).
- [ ] `grep -n "CancelSignal::never()" mcp/src/runner.rs` shows the signal only in
      **test** constructions, **not** in `run_phase_with`'s `LoopDeps` (line ~306
      now reads `cancel: inp.cancel.clone()`).
- [ ] `request_stop` on a known `run_id` returns `true` and fires the handle; on
      an unknown id returns `false`.
- [ ] A `cancelled` result JSON that had `request_stop(…, ClaudeStop)` called gets
      `cancellation.reason == "claude_stop"`; a `complete` result never gets a
      `reason` stamped even if `request_stop` was called (the race case).
- [ ] `stop_phase` is present in the server's advertised tool list (auto-listed).

## Test plan

Hermetic unit tests only (no real network, no sleeps). Names describe behavior.

- `mcp/src/jobs.rs`:
  - `request_stop_unknown_id_returns_false` — `request_stop` on an empty registry
    is `false`.
  - `request_stop_known_id_fires_and_returns_true` — `insert` a run with a handle
    whose `CancelSignal` is observed; after `request_stop` the signal
    `is_cancelled()` and the call returned `true`.
  - `stamp_cancel_reason_sets_reason_on_cancelled` — a `json!({"status":"cancelled",
    "cancellation":{"stage":"between_turns","turns_done":2}})` gains
    `cancellation.reason == "claude_stop"` after `stamp_cancel_reason(&mut j,
    Some(ClaudeStop))`.
  - `stamp_cancel_reason_noop_on_complete` — a `json!({"status":"complete"})` is
    unchanged even with `Some(ClaudeStop)` (pins the race negative).
  - `stamp_cancel_reason_noop_when_reason_none` — unchanged with `None`.
  - `spawn_run_stamps_reason_when_stopped_and_cancelled` — end-to-end: `spawn_run`
    a work future that returns a `cancelled` JSON; call `request_stop` before it
    resolves; `await_terminal` yields `Complete(json)` whose
    `cancellation.reason == "claude_stop"`.
  - Keep the existing `await_terminal_*` / `spawn_run_publishes_*` tests (updated
    signatures).
- `mcp/src/server_tests.rs` (or wherever the server tool tests live):
  - `stop_phase_unknown_run_returns_not_stopped` — build a `RexyMcpServer`, call
    the `stop_phase` path (via `self.runs.request_stop`, or the tool if a harness
    exists) with a random id → `stopped == false`.
  - A test that a real spawned run threads a **live** signal: assert that after
    `run_phase` with a `RunPhaseConfig { cancel: <a fired signal>, … }` and a
    `MockAiClient` scripted to loop, the loop terminates as `cancelled`. If the
    existing runner test harness makes this heavy, a lighter unit test that
    `run_phase_with` copies `inp.cancel` into `LoopDeps` (observable via a
    pre-cancelled signal producing an immediate `cancelled` result) is acceptable —
    pin the **behavior** (a fired input signal ⇒ `cancelled` status), not the
    plumbing.

**Do not** write a test that sleeps or races on wall-clock time. Use a
pre-fired `CancelSignal` (`let (h, s) = CancelSignal::new(); h.cancel();`) so
`is_cancelled()` is deterministically true, mirroring `cancel_flips_signal` in
`executor/src/agent/cancel.rs`.

## End-to-end verification

The real artifact is the running MCP server's tool surface. After the gates pass:

- Run `cargo run -p rexymcp -- doctor` (or the server's tool-list path if doctor
  doesn't enumerate tools) and confirm `stop_phase` appears alongside
  `get_run_status`. If no CLI surfaces the tool list, quote the
  `list_tools`/`tool_router` test output that proves `stop_phase` is registered,
  and state that the live `rexymcp serve` must be **restarted** to pick up the new
  binary (a rebuilt binary does not hot-swap a running serve process).
- Paste the actual output. If you cannot exercise the live server tool list from
  the headless loop, write one line naming that limitation and rely on the
  registration test instead.

## Authorizations

- [ ] May touch `docs/architecture.md`: **No** — the architect already added the
      `stop_phase` tools-list bullet. The executor must not edit it.
- [ ] May add dependencies: **No.** `tokio` (`sync`), `uuid`, and the executor
      crate already provide everything (`watch`, `CancelHandle`, `CancelReason`).
- [ ] May add `#[allow(...)]`: **No.** If a `dead_code`/unused warning appears,
      the fix is to remove the speculative surface or add its real caller in this
      phase — not to silence it (this is exactly what bug-02-2 bounced).

## Out of scope

- **The `UserStop` / human path** — `rexymcp stop`, the `.rexymcp/stop` sentinel,
  and the serve-side watcher are **phase-04**. Do not add a filesystem watcher or
  a `stop` CLI subcommand. `CancelReason::UserStop` stays unused by this phase
  (phase-04 introduces its producer); do not add a caller just to avoid a
  dead-variant note (the enum is already public and consumed via `Deserialize`).
- **CLI `run-phase` cancellation** — it keeps `CancelSignal::never()` here; it
  gains sentinel honoring in phase-04.
- **`stop_phase` blocking until the run is actually cancelled** — `stop_phase`
  fires the signal and returns immediately; the caller polls `get_run_status` for
  the terminal `cancelled` result. Do not make `stop_phase` await the run.
- **Cancelling an already-terminal run** — `request_stop` on a finished run
  returns `true` but changes nothing (the terminal state is already published).
  Do not add re-stamping or resurrection logic.
- **The async-polling skill-loop rewrite** — that is phase-05.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### 🔴 RE-DISPATCH REFINEMENT — 2026-07-10 (read this first)

**The prior dispatch hard-failed** (`VerifierFailurePersistent`, 6 consecutive
verifier failures) with a `mismatched closing delimiter: }` in `mcp/src/jobs.rs`.
The substantive changes were *correct* — the failure was purely a **patch-tangling
mechanics problem**: a sequence of many small overlapping `patch`/`patch_lines`
edits to `mcp/src/jobs.rs` corrupted the `#[cfg(test)] mod tests` block's brace
structure (one test lost its closing `);\n    }`; another left orphaned fragments
from the old `spawn_run_publishes_failed` test dangling after the new tests). The
executor then spent 6 turns unable to surgically repair the braces.

**The working tree has been reset to a clean HEAD** — `jobs.rs` and `runner.rs`
are back to their committed state. Start fresh; do not go hunting for partial work.

**Edit-mechanics directive for this re-dispatch (this is what changed):**

- **`mcp/src/jobs.rs` — apply Tasks 1 and 6 with `write_file` on the WHOLE file,
  in a single write, not with many `patch` calls.** Tasks 1 (registry additions)
  and 6 (test-module rewrite) together touch most of the file; incremental patches
  to adjacent regions are exactly what tangled the braces last time. Compose the
  complete intended `jobs.rs` (existing content + the Task-1 struct/method changes
  + the Task-6 test updates) and write it once.
- **After any write, if the verifier reports a delimiter/brace error, do NOT patch
  around it.** Re-read the entire file and re-`write_file` a clean version. One
  bad brace is faster to fix by rewriting the file than by patching near it.
- For `runner.rs` / `server.rs` / `main.rs` (Task 2/3), the edits are small and
  localized — normal `patch` is fine there. It is specifically `jobs.rs`'s
  large, multi-region change that must go through `write_file`.
- Everything else in the Spec below is unchanged and still correct. Follow it as
  written; only the *how you edit `jobs.rs`* changed.

### Update — 2026-07-10 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the spec was complete and the content was right; the hard_fail was a
mechanical patch-tangle producing a brace mismatch the executor couldn't surgically
repair — so the fix is an edit-mechanics directive (write_file the whole jobs.rs)
plus a clean-tree reset, not a spec change or a takeover on first failure.

### Update — 2026-07-10 (complete, architect takeover)

**Summary:** Completed via **session takeover** after two consecutive executor
hard_fails (both `VerifierFailurePersistent`). The executor's `mcp/src/jobs.rs`
work (Task 1: `RunEntry.cancel`/`stop_reason`, `insert`/`spawn_run` handle,
`request_stop`, `recorded_reason`, `stamp_cancel_reason`, and all Task-6 tests)
landed **verbatim-correct** via the write_file directive from the first refinement,
as did the runner.rs struct fields + prod-site wiring. The architect finished the
mechanical remainder the executor could not complete within the verifier's
6-strike budget: the load-bearing `run_phase_with` `LoopDeps.cancel:
inp.cancel.clone()` (the executor had left it `never()` — a latent bug that would
have made `stop_phase` a no-op), the 7 test `AssemblyInput` literals, all of
`server.rs` Task 3/4 (imports, `cancel` param threaded through
`execute_phase_inner`/`_with_client`, the `execute_phase` branch signal-pair +
`spawn_run(handle)`, `continue_phase_inner`/CLI `never()`, the `stop_phase` tool +
`StopPhaseParams`/`StopPhaseOutput`), `main.rs`, and the `server_tests.rs` callers.
Two clippy fixes on the executor's tests (`bool_assert_comparison`; and a
`collapsible_if`/let-chain on the architect's own pre-injected `stamp_cancel_reason`).

**Acceptance criteria:** all met (below).

**Commands (independent architect re-run):**

```
cargo fmt --all --check      → clean (rustfmt applied to the 5 touched files only)
cargo build                  → Finished, zero warnings
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test                   → 503 passed (mcp) + 949 passed / 2 ignored (executor); 0 failed
```

**End-to-end verification:** `stop_phase` is registered via the
`#[rmcp::tool_router]` auto-list (same mechanism as `get_run_status`); the running
`rexymcp serve` must be **restarted** to expose it live (a rebuilt binary does not
hot-swap a running serve process — the known stale-serve caveat). Acceptance pin
verified: `grep "CancelSignal::never()" mcp/src/runner.rs` shows it only in the 7
test `AssemblyInput` literals + one doc comment — **not** in `run_phase_with`'s
`LoopDeps`, which now reads `cancel: inp.cancel.clone()`.

**Files changed:**
- `mcp/src/jobs.rs` — registry cancel handle + `request_stop`/`recorded_reason`/`stamp_cancel_reason`; `spawn_run` stamps `ClaudeStop` on a `cancelled` result (executor-authored, correct).
- `mcp/src/runner.rs` — `cancel: CancelSignal` on `RunPhaseConfig`/`AssemblyInput`; `LoopDeps.cancel` now honors the config signal (the `inp.cancel.clone()` fix was architect-authored).
- `mcp/src/server.rs` — `cancel` threaded through the inner fns + `execute_phase` branch; new `stop_phase` tool + param/output types (architect-authored).
- `mcp/src/main.rs`, `mcp/src/server_tests.rs` — cascade completions (architect-authored).

**Notes for review:** the two hard_fails are a real calibration signal —
see the Review verdict.

### Review verdict — 2026-07-10

- **Verdict:** escalated (session takeover after 2 hard_fails)
- **Bounces:** 0 review bounces; 2 executor hard_fails before takeover (no bug docs — infrastructure/mechanical failures, not defects in shipped code)
- **Executor:** AEON-7/Qwen3.6-27B-AEON (LARGE) authored jobs.rs + runner.rs correctly; Claude (direct) finished the multi-site cascade + `stop_phase` tool
- **Scope deviations:** none — the shipped code matches the spec; only *who* finished it changed
- **Calibration:** **1st occurrence (flag for the user).** A **required-field cascade** — adding a non-defaultable `cancel: CancelSignal` field to two structs used at ~14 call sites — collides with the governor's 6-consecutive-verifier-failure limit: the crate cannot compile until *every* site is updated, so the verifier strikes out before the executor finishes the cascade (WORKFLOW § "Prefer additive change shapes; avoid wide-blast-radius breaking changes"). The first hard_fail was a *different* class (patch-tangling a brace, fixed by a write_file directive); the second was this cascade-vs-strike wall. Lesson for future specs: a many-site required-field addition should either (a) be split so intermediate states compile, or (b) instruct the executor to make **all** cascade edits before *any* build/verify, or (c) be pre-flagged as architect-takeover-likely. Not yet a fold (1 occurrence); note for the milestone retrospective.
