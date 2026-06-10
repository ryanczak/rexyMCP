# Phase 06c: Model-facing task flips — `update_task` tool + prompt injection

**Milestone:** M12 — Executor Tooling
**Status:** done
**Depends on:** phase-06a (substrate, done), phase-06b (gate, done)
**Estimated diff:** ~300 lines (≈190 prod + ≈110 test)
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the architect-seeded task list **model-facing**: give the executor a
`update_task` tool to flip a tracked task `pending → active → done` as it works,
and inject the task checklist + usage instructions into the system prompt so the
model knows the tool exists and when to call it. Every flip emits the
`SessionEvent::TaskUpdate` that 06a defined, so `rexymcp status` (and the future
dashboard panel, phase-07) reflect live progress.

This is the payoff of the M12 Arc A task-tracking feature: 06a built the
substrate (the event + the seeder), 06b put it behind the `task_tracking`
kill-switch and threaded the `LoopDeps` field, and **06c lets the model actually
drive it**. The whole feature stays gated: with `task_tracking` off there is no
`update_task` tool in the schema list and no task section in the prompt — the
model-facing surface is byte-identical to pre-M12 (the pinned A/B negative).

06b already did all the `LoopDeps`-struct-literal churn, so **06c carries none**.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #12 (M12) — Arc A: the executor "flips items
  pending → active → done"; "Config-gated … a clean A/B intervention … no
  measurement without an off-switch."
- `docs/dev/milestones/M12-executor-tooling/README.md` § "Pre-injection
  watch-items" — "The *prompt*-suppression half (no task section when off) is
  06c's" pinned negative.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom — especially §2.1 (the error
   model: bad model input → an advisory `ToolResult`, **not** `Result::Err`).
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

What 06a/06b already built that 06c builds on:

- **The event** (`executor/src/store/sessions/event.rs:124-128`):
  `SessionEvent::TaskUpdate { id: String, title: String, state: TaskState }`,
  where `TaskState` is `Pending | Active | Done` (derives `Serialize`/
  `Deserialize`, `#[serde(rename_all = "snake_case")]` → wire strings
  `"pending"`/`"active"`/`"done"`). **Do not touch this file.**
- **The seeder** (`executor/src/agent/tasks.rs`): the pure
  `seed_from_spec(phase_doc) -> Vec<Task>` parser and the
  `pub struct Task { pub id: String, pub title: String, pub state: TaskState }`.
  The module is declared `mod tasks;` (crate-private) at
  `executor/src/agent/mod.rs:18` — **task 1 makes it `pub`** so the mcp crate's
  `build_registry` can seed the tool. **Do not change the parsing logic.**
- **The gate** (`executor/src/agent/mod.rs:185-201`): the 06b
  `if deps.task_tracking { … }` block that emits one `pending` `TaskUpdate` per
  seeded item at turn 0. **Leave this block byte-untouched** — 06c adds new
  code around it, not inside it. (Its off-byte-identity is already tested.)
- **`deps.task_tracking`** (`LoopDeps`, `mod.rs:112`) and
  **`inp.cfg.executor.task_tracking`** (`mcp/src/runner.rs:200`): the flag is
  already threaded. 06c reads it; it adds no new field.

### Worked example A — the `Tool` trait and a real tool (mirror this shape)

The trait (`executor/src/tools/registry.rs:17-24`):

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;               // OpenAI function-calling shape
    async fn execute(&self, args: Value) -> Result<ToolResult>;
}
```

`ToolResult` (`registry.rs:10-15`):

```rust
pub struct ToolResult {
    pub output: String,          // model-visible text on success
    pub error: Option<String>,   // Some(_) = advisory failure the model sees
    pub metadata: Option<Value>, // structured side-channel for the loop
}
```

A real tool's struct + constructor (`executor/src/tools/read_file.rs:20-35`),
the exact shape to copy for `UpdateTask`:

```rust
#[derive(Deserialize)]
struct ReadFileArgs { path: String, start_line: Option<usize>, end_line: Option<usize> }

pub struct ReadFile { scope: Scope }

pub fn read_file(scope: Scope) -> Arc<dyn Tool> {
    Arc::new(ReadFile { scope })
}

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read a file's contents. …" }
    fn schema(&self) -> Value { json!({ "type": "object", "properties": { … } }) }
    async fn execute(&self, args: Value) -> Result<ToolResult> { … }
}
```

**The dispatch contract** (`executor/src/agent/tools.rs:298-302`) — how a tool's
return becomes the loop's `(succeeded, content, tool_meta)` tuple:

```rust
Ok(result) => match result.error {
    Some(error) => (false, error, None),          // advisory failure → no metadata
    None        => (true, result.output, result.metadata), // success → metadata flows
},
```

So: an invalid call returns `ToolResult { error: Some(msg), … }` and the model
sees `msg`; a valid call returns `ToolResult { error: None, metadata: Some(…) }`
and the loop receives the metadata. **This is how the flip reaches the event
emitter without the tool needing the session log.**

### Worked example B — the metadata→event emit the loop already does

The loop emits `OutputFiltered` by inspecting a tool's success metadata
(`executor/src/agent/mod.rs:697-717`). **Task 6b copies this exact shape** for
`TaskUpdate`:

```rust
if let Some(meta) = &tool_meta
    && let Some(of) = meta.get("output_filter")
    && let (Some(before), Some(after), Some(filter)) = (
        of.get("tokens_before").and_then(|v| v.as_u64()),
        of.get("tokens_after").and_then(|v| v.as_u64()),
        of.get("filter").and_then(|v| v.as_str()),
    )
    && after < before
{
    log_event(&log_handle, &redactor, deps.clock, turns,
        SessionEvent::OutputFiltered { … });
}
```

### Where the task list lives (the one design decision to internalize)

Tools are stateless `Arc<dyn Tool>` with no access to the session log or the
loop's locals — that is why the loop, not the tool, emits events. But the
`update_task` tool **does** need cross-turn state: to validate the `id` the model
sends and to look up that task's `title` for the event. The chosen design:

> **The `UpdateTask` tool owns the canonical live list** in a
> `std::sync::Mutex<Vec<Task>>`, seeded once from the phase doc at
> registry-build time. The registry wraps it in a single `Arc`, so the one
> instance persists across turns. `execute` locks the list, validates the `id`,
> flips the state in place, and returns `{id, title, state}` in `metadata`. The
> **loop** transcribes that metadata into a `SessionEvent::TaskUpdate`. The loop
> itself holds **no** task state.

The prompt's task section and the 06b initial-`pending` emit are seeded
**separately** from the same pure `seed_from_spec(&input.phase_doc)` (it is a
pure function over the same string, so the three copies never diverge — and
keeping them separate means the 06b block stays byte-untouched). Lock the mutex
only for the brief synchronous flip; never hold the guard across an `.await`.

## Spec

Numbered tasks in execution order.

1. **Expose the tasks module.** In `executor/src/agent/mod.rs:18`, change
   `mod tasks;` to `pub mod tasks;` so the mcp crate can call
   `rexymcp_executor::agent::tasks::seed_from_spec` / name `Task`. (One word.
   No logic change.)

2. **Create the `update_task` tool.** New file
   `executor/src/tools/update_task.rs`:
   - Imports: `anyhow::Result`, `async_trait::async_trait`,
     `serde::Deserialize`, `serde_json::{Value, json}`, `std::sync::{Arc, Mutex}`,
     `crate::agent::tasks::Task`, `crate::store::sessions::event::TaskState`,
     `super::registry::{Tool, ToolResult}`.
   - A `#[derive(Deserialize)] struct UpdateTaskArgs { id: String, state: String }`.
   - `pub struct UpdateTask { tasks: Mutex<Vec<Task>> }`.
   - `pub fn update_task(tasks: Vec<Task>) -> Arc<dyn Tool> { Arc::new(UpdateTask { tasks: Mutex::new(tasks) }) }`.
   - `impl Tool`:
     - `name` → `"update_task"`.
     - `description` → e.g. *"Record progress on a tracked task from the phase
       checklist. Set a task `active` when you start it and `done` when it is
       complete. `id` is the Spec item number; `state` is one of `active`,
       `done`, `pending`."*
     - `schema` → `json!({ "type": "object", "properties": {
       "id": { "type": "string", "description": "Task id (the Spec item number, e.g. \"2\")." },
       "state": { "type": "string", "enum": ["active", "done", "pending"], "description": "New state for the task." }
       }, "required": ["id", "state"] })`.
     - `execute(args)`:
       1. Parse: `serde_json::from_value::<UpdateTaskArgs>(args)`. On `Err` →
          return `Ok(advisory("update_task: invalid arguments — expected {id, state}"))`
          (see the helper note below). **Never return `Err` for bad model input.**
       2. Map `args.state` → `TaskState`: `"pending"→Pending`, `"active"→Active`,
          `"done"→Done`; anything else → advisory error naming the allowed
          values. (An explicit `match args.state.as_str()` is fine — do not pull
          in a parse crate.)
       3. Lock `self.tasks`. Find the task whose `id == args.id`. **Unknown id →
          advisory error** `format!("update_task: no task with id \"{}\"", args.id)`
          (this is the model-visible boundary validation STANDARDS §2.2 calls
          for). Found → set its `state` to the new state; capture its `title`.
       4. Return success: `ToolResult {
          output: format!("task {} \"{}\" → {}", id, title, state_str),
          error: None,
          metadata: Some(json!({ "task_update": { "id": id, "title": title, "state": state_value } })),
          }` where `state_value = serde_json::to_value(new_state)?` (serializes to
          the snake_case wire string — the loop deserializes it back with the same
          serde impl, so the two sides share one source of truth).
     - A tiny private `fn advisory(msg: &str) -> ToolResult { ToolResult { output: String::new(), error: Some(msg.to_string()), metadata: None } }` keeps the three error returns readable. (Three call sites justify it; do not over-abstract beyond this file.)

3. **Register the tool module.** In `executor/src/tools/mod.rs`, add
   `mod update_task;` and `pub use update_task::{UpdateTask, update_task};`
   (alongside the existing `pub use` lines, keep them alphabetical-ish to match).

4. **Add the router category.** The router is latent (only registry helpers and
   their tests consume `categorize`), but `update_task` must still categorize to
   `Some(_)`. In `executor/src/tools/router.rs`:
   - Add a `Meta` variant to `enum Category` (after `Run`).
   - Add the arm `"update_task" => Category::Meta,` to `categorize`.
   In `executor/src/tools/registry.rs`, append `Category::Meta,` after
   `Category::Run,` in the ordering array inside `categories()` (the
   `[Category::Read, Category::Write, Category::Search, Category::Run]` literal,
   `registry.rs:72-78`) so a registered `update_task` shows up in
   `categories()`/`tools_in(Category::Meta)`.

5. **Inject the prompt section.** In `executor/src/agent/prompt.rs`, add a pure
   `pub fn task_section(tasks: &[Task]) -> String` (import `use super::tasks::Task;`):
   - Empty slice → return `String::new()` (this is the off / no-Spec byte-identity
     guarantee).
   - Non-empty → return a section beginning with the exact heading
     `# Task tracking` (the test pins this literal), a short instruction
     paragraph telling the model to call `update_task` to set a task `active`
     when it starts and `done` when complete (and to update as it goes, not batch
     at the end), then one line per task: `- [<state>] <id> — <title>` (state
     rendered lowercase: `pending`/`active`/`done`).

6. **Wire the loop** (`executor/src/agent/mod.rs`, `execute_phase`):
   a. **Append the section to the system prompt.** At the `let system = …`
      assembly (`mod.rs:120-124`), append `prompt::task_section(&seeded)` where
      `seeded = if deps.task_tracking { tasks::seed_from_spec(&input.phase_doc) } else { Vec::new() }`.
      When off (or no Spec), `seeded` is empty → `task_section` returns `""` →
      `system` is byte-identical to its 06b value. Do **not** reuse this `seeded`
      to replace the 06b emit block — leave that block as-is.
   b. **Emit `TaskUpdate` from flip metadata.** Immediately after the
      `OutputFiltered` emit block (`mod.rs:697-717`), add the parallel block:
      ```rust
      // Model-driven task flip (M12 Arc A / phase-06c): the update_task tool
      // reports the flip in metadata; transcribe it to a TaskUpdate event.
      if let Some(meta) = &tool_meta
          && let Some(tu) = meta.get("task_update")
          && let (Some(id), Some(title)) = (
              tu.get("id").and_then(|v| v.as_str()),
              tu.get("title").and_then(|v| v.as_str()),
          )
          && let Some(state) = tu
              .get("state")
              .and_then(|v| serde_json::from_value::<crate::store::sessions::event::TaskState>(v.clone()).ok())
      {
          log_event(
              &log_handle,
              &redactor,
              deps.clock,
              turns,
              SessionEvent::TaskUpdate { id: id.to_string(), title: title.to_string(), state },
          );
      }
      ```
      This fires only when `tool_meta` carries `task_update`, which only the
      `update_task` tool produces, which is only registered when `task_tracking`
      is on — so the off path emits nothing here without an extra guard.

7. **Register the tool in `build_registry`** (`mcp/src/runner.rs:115-146`):
   - Add a parameter `tasks: Option<Vec<rexymcp_executor::agent::tasks::Task>>`
     to `build_registry`. When `Some(t)`, push `tools::update_task(t)` into the
     `tools` vec **before** the schema map (so its schema is included). When
     `None`, register nothing extra — the seven-tool set is unchanged.
   - At the prod call site (`runner.rs:161`), seed conditionally:
     ```rust
     let tasks = if inp.cfg.executor.task_tracking {
         Some(rexymcp_executor::agent::tasks::seed_from_spec(&phase_doc))
     } else {
         None
     };
     let (registry, tool_schemas) = build_registry(
         &scope, 30, inp.cfg.context.output_filter, tasks,
     );
     ```
     (`phase_doc` is already in scope at `runner.rs:153`.)
   - Update the existing `build_registry_has_seven_tools` test call
     (`runner.rs:377`) to pass `None` as the new arg (still seven tools).

## Acceptance criteria

- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.
- [ ] With `task_tracking` on and an N-item `## Spec`, `build_registry` registers
      `update_task` (the schema list contains it) and the assembled system prompt
      contains the `# Task tracking` section listing the seeded tasks.
- [ ] A model `update_task` call with `{id: "<seeded id>", state: "active"}`
      flips that task and writes one `task_update` record with `state: "active"`
      (id + title match the seeded task), via the loop's metadata→event path.
- [ ] An `update_task` call with an unknown `id`, an invalid `state`, or
      malformed args returns an advisory `ToolResult` error (model-visible) and
      writes **no** `task_update` record.
- [ ] With `task_tracking` **off**: `update_task` is absent from the schema list,
      the system prompt contains **no** `# Task tracking` section (the `Prompt`
      record is byte-identical to a tracking-off run), and the loop emits zero
      model-driven `task_update` records.
- [ ] No change to `event.rs`, the parsing logic in `tasks.rs` (only its module
      visibility), `status.rs`, the dashboard, or `LoopDeps` (no new field).

## Test plan

In `executor/src/tools/update_task.rs` (`#[cfg(test)] mod tests`, hermetic):

- `flips_pending_task_to_active` — seed one `Pending` task, call
  `execute({id, state:"active"})` → success, `metadata.task_update.state ==
  "active"`, id/title match.
- `flips_active_task_to_done` — same, `"done"`.
- `success_output_names_task` — the `output` string contains the id and title.
- `unknown_id_returns_advisory_error` — `{id:"99", state:"done"}` against a
  one-task list → `error` is `Some` naming the id, `metadata` is `None`. (Pins
  the boundary-validation negative; a "flip anyway" impl fails it.)
- `invalid_state_returns_advisory_error` — `{id:"1", state:"frobnicate"}` →
  `error` is `Some`, no flip.
- `malformed_args_returns_advisory_error` — `execute(json!({"id":1}))` (wrong
  type / missing field) → `error` is `Some`, **not** an `Err`.

In `executor/src/tools/router.rs`: `categorize_update_task` —
`categorize("update_task") == Some(Category::Meta)`.

In `executor/src/agent/prompt.rs`:

- `task_section_lists_tasks_with_state` — a 2-item list → output contains
  `# Task tracking`, both titles, and the `pending` state markers.
- `task_section_empty_for_no_tasks` — `task_section(&[])` returns `""` exactly.

In `executor/src/agent/tests.rs` (loop integration, MockAiClient; mirror 06b's
on/off task tests and how they read `records(dir.path())`):

- `loop_emits_task_update_when_model_flips_task` — register `update_task` (seeded
  from a 2-item Spec) in the test registry, script a MockAiClient turn that calls
  `update_task` with `{id:"1", state:"active"}`, run the loop, and assert the
  session log contains a `task_update` record with `state == Active` for id `"1"`
  **beyond** the two turn-0 `pending` seeds. (Mutation-resistant: removing the
  metadata→event block drops the `active` record and fails.)
- `loop_prompt_omits_task_section_when_tracking_off` — run the loop over a Spec
  doc with `task_tracking` off (`deps()` override as in 06b), read the `Prompt`
  record, assert its `rendered` does **not** contain `# Task tracking`.
- `loop_prompt_includes_task_section_when_tracking_on` — same doc, default-on
  `deps()`, assert the `Prompt` record **does** contain `# Task tracking` and a
  seeded title.

In `mcp/src/runner.rs` tests:

- `build_registry_includes_update_task_when_tasks_present` — pass
  `Some(vec![Task{…}])` → the schema list contains `update_task` (eight tools).
- `build_registry_excludes_update_task_when_none` — pass `None` → seven tools,
  no `update_task`.

The 06a/06b tests routing through `deps()` and the existing seven-tool
`build_registry` test must stay green at their adjusted counts.

## End-to-end verification

The model-facing behavior is exercised hermetically by
`loop_emits_task_update_when_model_flips_task` — it drives the full
tool → `dispatch` → success-metadata → loop emit → JSONL `task_update(active)`
path with a `MockAiClient` (the same "the loop test is the behavioral E2E"
framing as 06b, since the gate ships no new CLI surface). The on/off prompt-section
difference is proven by the paired `loop_prompt_{includes,omits}_task_section_*`
tests reading the real `Prompt` record. Quote the asserted `active` record (or
the relevant `cargo test update_task`/`task_section` run tail) in the completion
Update Log.

If you want an additional real-binary check, none is required: there is no new
`init`/CLI artifact in this phase (unlike 06b's template line). Write the
"loop test is the behavioral E2E" note rather than inventing a CLI repro.

## Authorizations

None. (No new dependencies — `Mutex`/`serde_json` are already in use. No
`Cargo.toml`/architecture/STANDARDS/WORKFLOW edits. No `LoopDeps`/config field.
`rexymcp.toml` is not edited.)

## Out of scope

Do **not**, in this phase:

- Let the model **append discovered sub-tasks** to the list (the milestone's
  "may append" stretch goal). 06c flips existing seeded tasks only; appending is
  a later phase if the data justifies it.
- Touch the dashboard `Tasks` panel / `panels.rs` / `render.rs` — that is
  **phase-07**.
- Change `event.rs`, the `tasks.rs` parsing logic (only its `mod` visibility),
  `status.rs`, or any other `SessionEvent` match arm (06a already landed the
  variant blast radius).
- Add a `LoopDeps` field or a config field (06b already threaded
  `task_tracking`; 06c only reads it).
- Re-seed or re-emit the turn-0 `pending` list — leave the 06b emit block
  (`mod.rs:185-201`) byte-untouched; 06c's emit covers only model-driven flips.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 (complete)

**Executor:** rexyMCP executor

**Summary:** Implemented the `update_task` tool, wired it into the loop via metadata→event emit, injected the `# Task tracking` prompt section (gated by `task_tracking`), and registered the tool in `build_registry` conditionally.

**Files changed:**
- `executor/src/agent/mod.rs` — exposed `pub mod tasks`; appended `task_section` to system prompt; added metadata→`TaskUpdate` emit block after `OutputFiltered`
- `executor/src/agent/prompt.rs` — added `task_section()` pure function + 2 unit tests
- `executor/src/tools/update_task.rs` — new file: `UpdateTask` tool with `Mutex<Vec<Task>>` state, 6 unit tests
- `executor/src/tools/mod.rs` — registered `update_task` module
- `executor/src/tools/router.rs` — added `Category::Meta` + `categorize_update_task` test
- `executor/src/tools/registry.rs` — added `Category::Meta` to ordering array
- `executor/src/agent/tests.rs` — 3 integration tests (flip emit, prompt off, prompt on)
- `mcp/src/runner.rs` — added `tasks` param to `build_registry`, conditional seed at call site, 2 new tests
- `docs/dev/milestones/M12-executor-tooling/phase-06c-update-task-tool.md` — status `todo` → `review`
- `docs/dev/milestones/M12-executor-tooling/README.md` — phase table row `todo` → `review`

**Verification commands:**
```
cargo fmt --all --check: PASS
cargo build: PASS (zero warnings)
cargo clippy --all-targets --all-features -- -D warnings: PASS
cargo test: 722 passed, 0 failed, 2 ignored
```

**Grep for pinned literal `# Task tracking`:**
```
$ grep -rn '# Task tracking' executor/src/
executor/src/agent/prompt.rs:56:    let mut out = String::from("# Task tracking\n\n");
executor/src/agent/prompt.rs:188:        assert!(section.contains("# Task tracking"));
executor/src/agent/tests.rs:3896:                !rendered.contains("# Task tracking"),
executor/src/agent/tests.rs:3922:            rendered.contains("# Task tracking") && rendered.contains("First task")
```

**End-to-end verification:** The loop test `loop_emits_task_update_when_model_flips_task` is the behavioral E2E — it drives the full tool → dispatch → success-metadata → loop emit → JSONL `task_update(active)` path with a `MockAiClient`. The on/off prompt-section difference is proven by the paired `loop_prompt_{includes,omits}_task_section_*` tests reading the real `Prompt` record. No new CLI artifact exists in this phase.

**Notes for review:** None — implementation follows the spec exactly.

### Update — 2026-06-10 10:00 (fix — bug-06c-1)

**Executor:** rexyMCP executor

**Bug fixed:** `executor/src/tools/update_task.rs:84` had `.lock().unwrap()` in the production path, violating STANDARDS §2.1. Replaced with `.lock().unwrap_or_else(|e| e.into_inner())` following the existing poison-tolerant idiom established in `ai/mod.rs` and `jsonl.rs`.

**Files changed:**
- `executor/src/tools/update_task.rs` — `.lock().unwrap()` → `.lock().unwrap_or_else(|e| e.into_inner())`

**Verification commands:**
```
cargo fmt --all --check: PASS
cargo build: PASS (zero warnings)
cargo clippy --all-targets --all-features -- -D warnings: PASS
cargo test: 722 passed, 0 failed, 2 ignored
```

**Grep for unwrap/expect/panic in prod path:**
```
$ grep -nE '\.(unwrap|expect)\(|panic!\(' executor/src/tools/update_task.rs
```
All matches are inside `#[cfg(test)]` block (line 115+), zero in production code.

### Review verdict — 2026-06-10

- **Verdict:** approved_after_1
- **Bounces:** 1 (bug-06c-1 — major: production-path `.lock().unwrap()`, fixed in commit 2648cbb)
- **Executor:** rexyMCP executor (Qwen/Qwen3.6-27B-FP8)
- **Scope deviations:** none
- **Calibration:** none (first `Mutex`-lock-unwrap bounce — a data point, not yet a trend; the poison-tolerant idiom is already established in `ai/mod.rs`/`jsonl.rs`)
