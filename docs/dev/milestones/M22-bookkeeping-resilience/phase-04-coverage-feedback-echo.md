# Phase 04: update_task result echoes remaining ids

**Milestone:** M22 — Bookkeeping-Loop Resilience
**Status:** done
**Depends on:** none (independent of phases 01–03 and 05)
**Estimated diff:** ~90 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Make the `update_task` tool result tell the model what work remains, so it can
self-correct instead of refixating. After a flip, the result names the
still-incomplete task ids and flags a redundant re-mark (marking a task that was
already in the target state).

In `session-phase-04-6a32f237`, the model called `update_task(id="1",
state="done")` eight times in a row — re-marking the same task while tasks 2–7 sat
pending — because the result (`task 1 "…" → done`) gave it no signal about what
was left. (The task-coverage *feedback* already lists incomplete tasks when the
model signals completion; this phase fixes the **per-call** tool result, which is
what the model sees on every `update_task`.)

## Architecture references

Read before starting:

- `executor/src/tools/update_task.rs` — `UpdateTask::execute` (lines 62–112). The
  `self.tasks` `Mutex<Vec<Task>>` is the live task list; the output string is
  built at line 102; the `task_update` metadata at 104–110 is consumed by the
  loop's task-states shadow (M21) and **must not change shape**.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0.

## Current state

```rust
let (id, title, state_value) = {
    let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
    let task = match tasks.iter_mut().find(|t| t.id == parsed.id) {
        Some(t) => t,
        None => {
            return Ok(advisory(&format!(
                "update_task: no task with id \"{}\"",
                parsed.id
            )));
        }
    };
    let title = task.title.clone();
    let id = task.id.clone();
    task.state = new_state;
    let state_value = serde_json::to_value(task.state)?;
    (id, title, state_value)
};

Ok(ToolResult {
    output: format!("task {} \"{}\" → {}", id, title, parsed.state),
    error: None,
    metadata: Some(json!({
        "task_update": { "id": id, "title": title, "state": state_value }
    })),
})
```

`TaskState` is `Copy` and is `crate::store::sessions::event::TaskState` with
variants `Pending` / `Active` / `Done`.

## Spec

### Task 1 — Echo remaining ids + flag redundant re-mark (`update_task.rs`)

Inside the locked block, **before** `task.state = new_state;`, capture whether
this is a no-op re-mark:

```rust
let was_already = task.state == new_state;
```

After `task.state = new_state;` and still **inside the lock**, compute the ids of
all tasks not yet `Done`, in their seeded order:

```rust
let remaining: Vec<String> = tasks
    .iter()
    .filter(|t| t.state != TaskState::Done)
    .map(|t| t.id.clone())
    .collect();
```

Return `(id, title, state_value, was_already, remaining)` from the block and build
the output to append guidance. The base `task {id} "{title}" → {state}` text and
the `task_update` **metadata stay exactly as today** (the loop's shadow depends on
them); only append a clause:

```rust
let remark = if was_already && new_state == TaskState::Done {
    format!("task {id} \"{title}\" was already done")
} else {
    format!("task {id} \"{title}\" → {}", parsed.state)
};
let tail = if remaining.is_empty() {
    " — all tasks complete".to_string()
} else {
    format!(" — still incomplete: {}", remaining.join(", "))
};

Ok(ToolResult {
    output: format!("{remark}{tail}"),
    error: None,
    metadata: Some(json!({
        "task_update": { "id": id, "title": title, "state": state_value }
    })),
})
```

(Keep the existing `advisory(...)` early returns for invalid args / invalid state
/ unknown id unchanged.)

### Task 2 — Tests (`update_task.rs` `#[cfg(test)] mod tests`)

- `result_lists_remaining_incomplete_ids` — three tasks (ids 1,2,3 pending), mark
  task 1 done; assert `result.output` contains `"still incomplete"` and `"2"` and
  `"3"` and not a "task 2 done" claim. (Build the tool with three tasks via the
  `make_tasks` helper, extended.)
- `result_flags_redundant_remark` — mark task 1 done, then call `update_task(id=1,
  state=done)` again; assert the second `result.output` contains `"was already
  done"`.
- `result_reports_all_complete_when_last_done` — single task; mark it done; assert
  `result.output` contains `"all tasks complete"`.
- `metadata_shape_is_unchanged` — after a flip, assert
  `result.metadata["task_update"]["state"]` is `"done"` and `["id"]`/`["title"]`
  are present (pins that the loop's shadow contract is intact).

**Pinned negative / preserved behavior:**

- The existing tests (`flips_pending_task_to_active`, and the one asserting
  `result.output.contains("1")` / `contains("First task")`) must pass — the new
  output still contains the id and title. If any existing test pins the **exact**
  old output string `task 1 "…" → done` (equality, not `contains`), update it to
  the new format and note it in Notes-for-review.

## Acceptance criteria

- [ ] `result_lists_remaining_incomplete_ids` passes.
- [ ] `result_flags_redundant_remark` passes.
- [ ] `result_reports_all_complete_when_last_done` passes.
- [ ] `metadata_shape_is_unchanged` passes.
- [ ] All pre-existing `update_task` tests pass (updated only if they pinned the
      exact old output string by equality).
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy`, `cargo test` exit 0.

## Test plan

Four unit tests above, all hermetic (no I/O — `UpdateTask` over an in-memory task
vec). They assert the human-readable output guidance and the unchanged metadata
contract.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact beyond the tool result,
which the unit tests exercise directly. The result text reaches the model through
the normal `append_tool_exchange` path, unchanged.

## Authorizations

None. No new dependency, no `Cargo.toml`/`architecture.md` edit.

## Out of scope

- The task-coverage *feedback* string (`command::task_coverage_feedback`) — it
  already lists incomplete tasks by id+title; this phase only touches the
  per-call tool result.
- Changing the `task_update` metadata shape or the loop's shadow-map handling.
- Reordering or re-titling tasks (that is seeding — phase-03).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-18 01:36 (started)

**Executor:** claude-code
**Tasks:** Implementing Tasks 1 and 2 — echo remaining ids + flag redundant re-mark, and add four unit tests.

### Update — 2026-06-18 01:37 (complete)

**Executor:** claude-code
**Tasks completed:** 1 (echo remaining ids + flag redundant re-mark), 2 (four unit tests).

**Verification:**
- `cargo fmt --all --check` — exit 0
- `cargo build` — exit 0, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — exit 0
- `cargo test` — 836 passed, 0 failed, 2 ignored
- `cargo test -p rexymcp-executor -- update_task` — 11 passed (7 pre-existing + 4 new)

**Grep for pinned literal:** `grep "still incomplete" executor/src/tools/update_task.rs` → 2 matches (line 115 production, line 251 test).

**Files changed:**
- `executor/src/tools/update_task.rs` — added `was_already` / `remaining` capture inside lock, appended `{remark}{tail}` to output; added `make_three_tasks` helper and 4 new tests.
- `docs/dev/milestones/M22-bookkeeping-resilience/phase-04-coverage-feedback-echo.md` — status → review, update log entries.
- `docs/dev/milestones/M22-bookkeeping-resilience/README.md` — phase 04 row → review.

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact beyond the tool result, which the unit tests exercise directly.

**Notes for review:**
- No existing test pinned the exact old output string by equality, so no pre-existing test needed updating. All 7 pre-existing `update_task` tests passed unchanged.
- The `task_update` metadata shape is untouched — same `id`, `title`, `state` keys.

### Review verdict — 2026-06-18

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** claude-code (direct)
- **Scope deviations:** none — `update_task.rs` only (production + tests), plus the
  phase-doc/README status bookkeeping. Spec implemented byte-for-byte: `was_already`
  captured before the state flip, `remaining` computed inside the lock in seeded
  order, `{remark}{tail}` output, `task_update` metadata unchanged.
- **Independent re-run:** all four gates green — `cargo fmt --all --check` clean,
  `cargo build` zero warnings, `cargo clippy` clean, `cargo test` 836 passed / 2
  ignored. Four new tests present and mutation-resistant (the base remark for
  `result_lists_remaining_incomplete_ids` contains no `"2"`, so the id-echo
  assertions only pass via the `remaining` clause).
- **Calibration:** none.
