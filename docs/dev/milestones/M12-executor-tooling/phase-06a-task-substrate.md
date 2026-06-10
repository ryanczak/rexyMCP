# Phase 06a: Task-tracking substrate — `SessionEvent::TaskUpdate` + Spec seeding

**Milestone:** M12 — Executor Tooling
**Status:** todo
**Depends on:** phase-05 (done)
**Estimated diff:** ~330 lines (≈150 prod + ≈180 test)
**Tags:** language=rust, kind=feature, size=m

## Goal

Lay the **substrate** for Arc A structured task tracking: a new
`SessionEvent::TaskUpdate { id, title, state }` log event, a pure parser that
seeds a task list from the phase doc's numbered `## Spec` section, and the loop
emitting one `TaskUpdate` (state `pending`) per seeded task at session start.
The seeded list is surfaced in `rexymcp status` (a `tasks: N/M done` line) so it
has a live, end-to-end-verifiable consumer.

This phase **does not** let the executor model flip task state, inject the list
into the prompt, or add a config gate — those are phase-06b. Splitting this way
isolates the **new-`SessionEvent`-variant match-arm wall** (the class that
hard-failed M10 phase-03/04/06) into a phase whose only *logic* is one pure
parser, with **no `LoopDeps`/`PhaseInput`/config struct-literal churn** (a new
`LoopDeps` field would touch 9 construction sites — the M10/phase-08a
struct-literal stall class; we avoid it by emitting unconditionally here and
gating in 06b).

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #12 (M12) — Arc A: "the loop seeds the list
  from the Spec and emits updates as the executor checks items off"; the list is
  "seeded from the phase doc's numbered Spec," the executor "does not generate
  it."
- `docs/dev/milestones/M12-executor-tooling/README.md` § "Pre-injection
  watch-items" — the new-variant match-arm blast radius is the known wall;
  enumerate every arm (this doc does).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`SessionEvent` is a serde-tagged enum in
`executor/src/store/sessions/event.rs:21-111`. It currently has 15 variants
ending at `ReadDeduped`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum SessionEvent {
    // ... 14 variants ...
    ReadDeduped {
        path: String,
        tokens_saved: usize,
        prior_turn: usize,
    },
}
```

The phase-doc markdown is available in the loop at runtime as
`input.phase_doc` (`executor/src/agent/mod.rs:63`, threaded from
`mcp/src/runner.rs:153` `std::fs::read_to_string`). The system prompt is
assembled at `mod.rs:115-119`; the session log is opened and the
`SessionStart` + `Prompt` events are emitted at `mod.rs:159-178`, immediately
before `loop {` at `mod.rs:180`.

The `agent` module already has sibling submodules (`prompt`, `tools`, `log`,
`metrics`, …). `log_event(&log_handle, &redactor, deps.clock, turn, event)` is
the emit helper (`mod.rs`, e.g. the `ReadDeduped` emit at `mod.rs:731-741`).

`rexymcp status` reads the JSONL log and folds it via `summarize` into a
`StatusSummary`, rendered by `format_status` (`mcp/src/status.rs`).

### Worked example — the full blast radius of one variant (`ReadDeduped`)

`ReadDeduped` was the last variant added. Mirror its shape exactly. Every site
it touches (this is the **complete** site list for a new variant; do the same
for `TaskUpdate`):

| # | File | Site |
|---|---|---|
| 1 | `executor/src/store/sessions/event.rs:106-110` | the variant definition |
| 2 | `mcp/src/dashboard/filter.rs:3` | `FILTER_ITEM_COUNT` const |
| 3 | `mcp/src/dashboard/filter.rs:22` | `ActivityFilter` struct field |
| 4 | `mcp/src/dashboard/filter.rs:41` | `Default` impl field |
| 5 | `mcp/src/dashboard/filter.rs:62` | `allows()` match arm |
| 6 | `mcp/src/dashboard/filter.rs:81` | `toggle()` index arm |
| 7 | `mcp/src/dashboard/filter.rs:101` | `is_enabled()` index arm |
| 8 | `mcp/src/dashboard/filter.rs:121` | `item_label()` index arm |
| 9 | `mcp/src/dashboard/transcript.rs:161-170` | `record_lines()` match arm |
| 10 | `mcp/src/log_query.rs:30` | `event_type_str()` match arm |
| 11 | `executor/src/agent/tests.rs:876` | `event_kind()` test-helper arm |
| 12 | `mcp/src/status.rs:73-76`, `:190-193` | `StatusSummary` fields + `summarize` arm |

Sites that are **non-exhaustive** (a `_ =>` fallthrough) and must **NOT** get a
`TaskUpdate` arm — leave them exactly as they are:
`mcp/src/cap.rs` (`other => other`),
`mcp/src/log_query.rs:34-40` `matches_tool_name_filter` (`_ => false`),
`executor/src/store/telemetry.rs:62-96` `aggregate_context_efficiency` (`_ => {}`
— `TaskUpdate` is not a context-efficiency signal).

## Spec

Numbered tasks in execution order. **`cargo build` after each of tasks 3–8** —
they are the exhaustive-match wall; each unmatched arm is a hard compile error,
so building per site keeps you inside the verifier's runway (per STANDARDS /
WORKFLOW § "Prefer additive change shapes").

1. **Add `TaskState`** — in `executor/src/store/sessions/event.rs`, directly
   **above** the `SessionEvent` enum (so the serialized type co-locates with the
   other serialized types, and `agent` depends on `store` as it already does —
   not the reverse):

   ```rust
   /// State of one tracked task in the architect-seeded TODO list (M12 Arc A).
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "snake_case")]
   pub enum TaskState {
       Pending,
       Active,
       Done,
   }
   ```

2. **Add the `TaskUpdate` variant** — append it **after** `ReadDeduped` in the
   `SessionEvent` enum (additive; keeps every prior variant byte-identical):

   ```rust
   /// Emitted when a tracked task is seeded or changes state (M12 Arc A).
   /// At seed time the loop emits one `pending` update per numbered Spec item;
   /// 06b emits `active`/`done` as the executor flips them. Consumers
   /// reconstruct current state per `id` with last-write-wins.
   TaskUpdate {
       id: String,
       title: String,
       state: TaskState,
   },
   ```

3. **New module `executor/src/agent/tasks.rs`** — the pure seeder. Declare it in
   `executor/src/agent/mod.rs` with `pub mod tasks;` (it must be reachable as
   `crate::agent::tasks` from the loop). Contents:

   ```rust
   use crate::store::sessions::event::TaskState;

   /// One architect-seeded task. `id` is the Spec item's number ("1", "2", …);
   /// `title` is its short name.
   #[derive(Debug, Clone, PartialEq, Eq)]
   pub struct Task {
       pub id: String,
       pub title: String,
       pub state: TaskState,
   }

   /// Parse the phase doc's `## Spec` section into a seeded task list, all
   /// `Pending`. Pure; no I/O. Returns empty when there is no `## Spec` section
   /// or it has no top-level numbered items.
   pub fn seed_from_spec(phase_doc: &str) -> Vec<Task> { /* … */ }
   ```

   **`seed_from_spec` behavior (pin exactly):**
   - Find the **first** line whose trimmed text equals `## Spec` exactly
     (case-sensitive). If none → return `Vec::new()`.
   - Collect the lines **after** it, stopping (exclusive) at the next line whose
     **trimmed** text starts with `#` (any heading) or at EOF.
   - Within that body, a **task line** is a **raw** line (do *not* trim leading
     whitespace first) whose first char is an ASCII digit and that matches the
     shape `<digits>. <rest>` — i.e. `line.split_once('.')` gives a left side of
     one-or-more ASCII digits and a right side that **starts with a space or
     tab**. For each task line:
     - `id` = the digits.
     - `title` = from `rest.trim_start()`: if it starts with `**`, the text
       between that `**` and the next `**` (the bold span), trimmed; otherwise
       the whole `rest.trim_start().trim_end()`.
     - `state` = `TaskState::Pending`.
   - Preserve document order.
   - **Parse with std string methods only — do NOT add the `regex` crate**
     (that needs authorization; STANDARDS §2.6). No `.unwrap()`/`.expect()` in
     this production path (STANDARDS §2.1 — bug-05-1 fired on exactly this);
     use `if let` / `?`-on-`Option` patterns.

   **Pinned positives** (must produce these):
   - `1. **Add the variant** — in event.rs, append after ReadDeduped.`
     → `Task { id: "1", title: "Add the variant", Pending }`
   - `2. Wire the filter sites` (no bold)
     → `Task { id: "2", title: "Wire the filter sites", Pending }`
   - `12. **Last** thing` → `id: "12", title: "Last"`

   **Pinned negatives** (must **NOT** become tasks):
   - An **indented** numbered line `    1. a sub-step` (leading spaces/tabs
     before the digit) — it is a continuation/sub-item, not a top-level task.
   - `1.5x speedup` — the char after `.` is not a space/tab, so it is not a task
     line (no fabricated `id: "1"`).
   - A numbered line that appears **before** the `## Spec` heading, or **after**
     the next `#` heading following Spec — out of section, excluded.
   - A bullet (`- foo`) or prose paragraph — not numbered, excluded.
   - A phase doc with no `## Spec` heading → empty `Vec`.

4. **Seed-and-emit in the loop** — in `executor/src/agent/mod.rs`, immediately
   **after** the `Prompt` log event (`mod.rs:170-178`) and **before** `loop {`
   (`mod.rs:180`), add:

   ```rust
   // Task-tracking substrate (M12 Arc A / phase-06a): seed the TODO list from
   // the phase doc's Spec and broadcast it as one `pending` TaskUpdate each.
   for task in tasks::seed_from_spec(&input.phase_doc) {
       log_event(
           &log_handle,
           &redactor,
           deps.clock,
           0,
           SessionEvent::TaskUpdate {
               id: task.id,
               title: task.title,
               state: task.state,
           },
       );
   }
   ```

   Emit **unconditionally** (no config gate in this phase — the `[executor]
   task_tracking` gate and the `LoopDeps` field land in 06b, where the
   prompt-injection that actually changes model behavior lives). This is the
   deliberate choice that keeps 06a free of the 9-site `LoopDeps`-literal churn:
   do **not** add any field to `LoopDeps` or `PhaseInput` in this phase.

5. **`mcp/src/dashboard/filter.rs` — the 7 sites.** `TaskUpdate` becomes filter
   index **14**. Apply all seven verbatim:
   - line 3: `FILTER_ITEM_COUNT` `14` → `15`.
   - struct field (after `read_deduped: bool,`): `pub(crate) task_update: bool,`
   - `Default` (after `read_deduped: true,`): `task_update: true,`
   - `allows()` (after the `ReadDeduped` arm):
     `SessionEvent::TaskUpdate { .. } => self.task_update,`
   - `toggle()` (after `13 => …read_deduped…`):
     `14 => self.task_update = !self.task_update,`
   - `is_enabled()` (after `13 => self.read_deduped,`):
     `14 => self.task_update,`
   - `item_label()` (after `13 => "read deduped",`):
     `14 => "task update",`

6. **`mcp/src/dashboard/transcript.rs` — `record_lines()` arm.** After the
   `ReadDeduped` arm (ends `mcp/src/dashboard/transcript.rs:170`), add a match
   arm following the single-line-header shape (mirror the `OutputFiltered`/
   `ReadDeduped` arms — `(summary, color, bold, body)` tuple, `body = None`):

   ```rust
   SessionEvent::TaskUpdate { id, title, state } => (
       format!("task {id} [{state:?}]: {title}"),
       Color::Yellow,
       false,
       None,
   ),
   ```

   (`TaskState` derives `Debug`, so `{state:?}` renders `Pending`/`Active`/`Done`.
   `Color::Yellow` is fine — pick any existing `Color`; this is rendering, not
   pinned behavior.)

7. **`mcp/src/log_query.rs` — `event_type_str()` arm.** After the `ReadDeduped`
   arm (`log_query.rs:30`):
   `SessionEvent::TaskUpdate { .. } => "task_update",`

8. **`executor/src/agent/tests.rs` — `event_kind()` test helper.** After the
   `ReadDeduped` arm (`tests.rs:876`):
   `SessionEvent::TaskUpdate { .. } => "task_update",`

9. **`mcp/src/status.rs` — the `rexymcp status` consumer.** Three edits:
   - Add three fields to `StatusSummary` (after `read_deduped_tokens`,
     `status.rs:76`):
     ```rust
     /// Tracked-task counts (M12 Arc A): total distinct task ids seen, and how
     /// many are currently `Done` / `Active` (last-write-wins per id).
     pub tasks_total: usize,
     pub tasks_done: usize,
     pub tasks_active: usize,
     ```
   - In `summarize` (`status.rs:91-196`): before the `for rec` loop, add
     `let mut task_states: std::collections::HashMap<String, TaskState> =
     std::collections::HashMap::new();` (import `TaskState` alongside the
     existing `SessionEvent` import at `status.rs:11`). Add a match arm (before
     the `_ => {}` at `status.rs:194`):
     ```rust
     SessionEvent::TaskUpdate { id, state, .. } => {
         task_states.insert(id.clone(), *state);
     }
     ```
     After the loop (before `summary` is returned at `status.rs:224`), set:
     ```rust
     summary.tasks_total = task_states.len();
     summary.tasks_done = task_states
         .values()
         .filter(|s| **s == TaskState::Done)
         .count();
     summary.tasks_active = task_states
         .values()
         .filter(|s| **s == TaskState::Active)
         .count();
     ```
   - In `format_status` (`status.rs:256-304`): after the `reclaimed:` block,
     when `summary.tasks_total > 0`, push one line:
     ```rust
     if summary.tasks_total > 0 {
         lines.push(format!(
             "tasks: {}/{} done ({} active)",
             summary.tasks_done, summary.tasks_total, summary.tasks_active
         ));
     }
     ```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy
      --all-targets --all-features -- -D warnings` passes; `cargo fmt --all
      --check` passes; `cargo test` passes (existing + new).
- [ ] `SessionEvent::TaskUpdate { id, title, state }` exists and serializes with
      `"event_type": "task_update"` and `"state": "pending"|"active"|"done"`.
- [ ] `seed_from_spec` parses top-level numbered `## Spec` items into `Pending`
      tasks and honors every pinned positive and negative above.
- [ ] A loop run (MockAiClient, no tool calls) over a phase doc with an N-item
      `## Spec` writes exactly N `task_update` records, all `pending`, at turn 0.
- [ ] `rexymcp status` over a log containing `task_update` records shows a
      `tasks: D/T done (A active)` line; a log with none shows no `tasks:` line.
- [ ] No `LoopDeps`, `PhaseInput`, or `Config`/`*.toml` change in this phase.

## Test plan

In `executor/src/agent/tasks.rs` (`#[cfg(test)] mod tests`):

- `seeds_top_level_numbered_items` — a 3-item Spec → 3 `Pending` tasks, ids
  `"1"/"2"/"3"`, correct titles.
- `seeds_bold_title_strips_to_bold_span` — `1. **Name** — rest` → title `Name`.
- `seeds_plain_title_keeps_whole_remainder` — `2. plain text` → title
  `plain text`.
- `ignores_indented_sub_items` — an indented `    1. sub` line is not a task
  (assert the parent count is unchanged / `len` excludes it).
- `ignores_decimal_like_numbers` — `1.5x speedup` produces no task.
- `ignores_items_outside_spec_section` — a numbered line before `## Spec` and
  one after the next `#` heading are both excluded.
- `empty_when_no_spec_section` — no `## Spec` → `[]`.
- `parses_multi_digit_ids` — `10. Tenth` → id `"10"`.

In `executor/src/agent/tests.rs` (loop integration, mirror the existing
`MockAiClient`-driven loop tests + the `event_kind` helper):

- `loop_seeds_task_updates_from_spec` — run the loop over a phase doc with a
  3-item Spec and a model that completes immediately; assert the session log has
  3 `event_kind == "task_update"` records, all decode to `state: Pending`, with
  the expected ids/titles. Mutation-resistant: assert the **count** is 3 (not
  ≥1) and the titles match.
- `loop_emits_no_task_updates_when_spec_absent` — a phase doc with no `## Spec`
  → zero `task_update` records (pinned negative).

In `mcp/src/status.rs` (`mod tests`, mirror the `summarize_folds_*` tests; add a
`task_update(id, state)` helper):

- `summarize_folds_task_states_last_write_wins` — `[update("1", Pending),
  update("2", Pending), update("1", Done)]` → `tasks_total == 2`,
  `tasks_done == 1`, `tasks_active == 0`. (Mutation-resistant: distinguishes the
  correct last-write-wins `1` done from a naive count-all-Done.)
- `summarize_no_tasks_when_absent` — a log with no `TaskUpdate` →
  `tasks_total == 0`.
- `format_status_shows_tasks_line_when_present` — output contains `tasks: 1/2
  done`.
- `format_status_omits_tasks_line_when_absent` — output does **not** contain
  `tasks:`.

In `mcp/src/dashboard/filter.rs` (`mod tests`): extend
`filter_default_disables_progress` to also assert `f.task_update`, and verify
`FILTER_ITEM_COUNT == 15` via the existing cursor-wrap tests (they already use
the const).

## End-to-end verification

The real artifact is the `rexymcp status` CLI. Verify against the built binary:
create a temp repo dir with `.rexymcp/sessions/session-phase-99-x.jsonl`
containing a `session_start` record plus two `task_update` records (one
`pending`, one `done`) — write the JSONL by hand or via `serde_json` — then run
`cargo run -p rexymcp -- status --repo <tmpdir>` and confirm the output includes
a `tasks: 1/2 done (0 active)` line. Quote the actual output in the completion
Update Log.

## Authorizations

None. (No new dependencies; no `Cargo.toml`/`*.toml`/architecture/STANDARDS/
WORKFLOW edits.)

## Out of scope

Do **not**, in this phase:

- Add `[executor] task_tracking` config, any `LoopDeps`/`PhaseInput` field, or
  any gate on the seeding — that is **phase-06b** (and it is what makes the
  9-site `LoopDeps`-literal change a *separate* concern).
- Add a tool the model calls to flip task state, or inject the task list into
  the system prompt — **phase-06b**.
- Touch the dashboard `Tasks` panel / `panels.rs` / `render.rs` layout — that is
  **phase-07**.
- Add a `TaskUpdate` arm to the non-exhaustive `_ =>` matches in `cap.rs`,
  `log_query::matches_tool_name_filter`, or `telemetry::aggregate_context_efficiency`.
- Fold `TaskUpdate` into the `PhaseRun`/`ContextEfficiency` telemetry — it is not
  a context-efficiency signal.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
