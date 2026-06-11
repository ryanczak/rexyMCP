# M14 phase-01 — Fix task seeder: `### N.` headings + empty-spec warning

**Status:** done

**Milestone:** [M14 — Cleanup](README.md)

**Size:** s

**Kind:** bugfix

## Goal

Fix two bugs in `executor/src/agent/tasks.rs` and `executor/src/agent/mod.rs`:

1. **Stop condition** — `seed_from_spec` breaks on `### N.` task-subheadings
   because `line.trim().starts_with('#')` fires on any heading. Change the
   stop to `starts_with("## ")` so only section boundaries end the scan.
2. **Heading-format parser** — add `parse_heading_task_line` so specs using
   `### N. Title` are seeded correctly (6 of 8 M13 phases used this format).
3. **Redundant call** — replace the second `seed_from_spec(&input.phase_doc)`
   call at lines 193-207 of `mod.rs` with iteration over `&seeded` (already
   computed at line 120-124).
4. **Observable empty-spec warning** — emit a `SessionEvent::Progress` at
   turn 0 when `task_tracking` is on but `seeded` is empty, so the failure is
   visible in the Activity panel rather than silent.

Fold the two accepted Spec formats into `WORKFLOW.md`'s phase-doc template
(explicitly authorized in the Authorizations section below).

## Architecture references

- `executor/src/agent/tasks.rs` — pure seeder, no I/O.
- `executor/src/agent/mod.rs` — loop entry point; turn-0 log events.
- `executor/src/store/sessions/event.rs` — `SessionEvent` enum (read-only;
  `Progress` variant already exists, no new variant needed).
- `docs/dev/WORKFLOW.md` — phase-doc template, `## Spec` section.

## Pre-flight

Confirm gates are green before touching any file:

```
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Current state

### `executor/src/agent/tasks.rs` (full file, 168 lines)

```rust
// tasks.rs:15-34 — seed_from_spec with the broken stop condition
pub fn seed_from_spec(phase_doc: &str) -> Vec<Task> {
    let Some(spec_start) = find_spec_section(phase_doc) else {
        return Vec::new();
    };

    let lines: Vec<&str> = phase_doc.lines().collect();
    let mut tasks = Vec::new();

    for line in lines.iter().skip(spec_start + 1) {
        // Stop at the next heading (trimmed line starts with '#')  ← BUG
        if line.trim().starts_with('#') {
            break;
        }
        if let Some(task) = parse_task_line(line) {
            tasks.push(task);
        }
    }

    tasks
}
```

```rust
// tasks.rs:50-73 — parse_task_line (handles `N. **Title**` and `N. plain`)
fn parse_task_line(line: &str) -> Option<Task> {
    let first_char = line.chars().next()?;
    if !first_char.is_ascii_digit() {
        return None;
    }
    let (digits, rest) = line.split_once('.')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    let title = extract_title(rest);
    Some(Task {
        id: digits.to_string(),
        title,
        state: TaskState::Pending,
    })
}
```

```rust
// tasks.rs:76-86 — extract_title helper
fn extract_title(rest: &str) -> String {
    let trimmed = rest.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("**")
        && let Some(title) = after_open.split_once("**")
    {
        return title.0.trim().to_string();
    }
    trimmed.trim_end().to_string()
}
```

### `executor/src/agent/mod.rs` (turn-0 seeding region)

```rust
// mod.rs:120-124 — first (correct) seed_from_spec call
let seeded: Vec<crate::agent::tasks::Task> = if deps.task_tracking {
    tasks::seed_from_spec(&input.phase_doc)
} else {
    Vec::new()
};
```

```rust
// mod.rs:191-207 — second (redundant) seed_from_spec call — to be replaced
if deps.task_tracking {
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
}
```

### `SessionEvent::Progress` variant shape (event.rs:61-66)

```rust
Progress {
    turn: usize,
    stage: String,
    files_changed: Vec<FileNumstat>,
    message: String,
},
```

### `WORKFLOW.md` Spec-section template (lines 201-206)

```markdown
## Spec

Numbered tasks in execution order. Each names the exact file to edit and the
change to make.

1. **<Task name>** — in `<path>`, <change>. <Why if non-obvious.>
```

## Spec

### 1. Fix `seed_from_spec` stop condition and add heading-format parser

In `executor/src/agent/tasks.rs`:

**a. Change the stop condition** from `starts_with('#')` to
`starts_with("## ")` so `### N.` task-subheadings do not terminate the scan:

```rust
// old (line 25)
if line.trim().starts_with('#') {
    break;
}

// new
if line.trim().starts_with("## ") {
    break;
}
```

**b. Add `parse_heading_task_line`** after `parse_task_line`. It handles
`### N. Title` lines (the format used in M13 phases 03–08):

```rust
/// Parse a `### N. Title` subheading as a task. Returns `None` for any other
/// line shape.
fn parse_heading_task_line(line: &str) -> Option<Task> {
    // Must start with exactly "### " (three hashes + space)
    let rest = line.trim().strip_prefix("### ")?;
    // After "### " must be `<digits>. <title>`
    let (digits, title_part) = rest.split_once('.')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !title_part.starts_with(' ') && !title_part.starts_with('\t') {
        return None;
    }
    Some(Task {
        id: digits.to_string(),
        title: title_part.trim().to_string(),
        state: TaskState::Pending,
    })
}
```

**c. Call both parsers in the loop** — try `parse_task_line` first, then
`parse_heading_task_line`:

```rust
for line in lines.iter().skip(spec_start + 1) {
    if line.trim().starts_with("## ") {
        break;
    }
    if let Some(task) = parse_task_line(line)
        .or_else(|| parse_heading_task_line(line))
    {
        tasks.push(task);
    }
}
```

### 2. Replace the redundant `seed_from_spec` call + add empty-spec warning

In `executor/src/agent/mod.rs`, replace lines 191-207 with:

```rust
// Task-tracking substrate (M12 Arc A). Gated by [executor] task_tracking
// (06b): off → no seeding, byte-identical to pre-06a.
if deps.task_tracking {
    for task in &seeded {
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            0,
            SessionEvent::TaskUpdate {
                id: task.id.clone(),
                title: task.title.clone(),
                state: task.state,
            },
        );
    }
    if seeded.is_empty() {
        log_event(
            &log_handle,
            &redactor,
            deps.clock,
            0,
            SessionEvent::Progress {
                turn: 0,
                stage: "task_seeding".to_string(),
                files_changed: vec![],
                message: "task tracking is on but 0 tasks were seeded \
                    from ## Spec — no `N. ` list items or `### N.` \
                    subheadings found; Tasks panel will be empty"
                    .to_string(),
            },
        );
    }
}
```

Note: the `state` field on `Task` is `Copy` (`TaskState` derives `Copy`);
`id` and `title` are `String` so they need `.clone()`.

### 3. Fold both accepted Spec formats into `WORKFLOW.md`

Replace the `## Spec` paragraph in the phase-doc template
(WORKFLOW.md lines 201-206, inside the `## Phase doc template` section)
from:

```
## Spec

Numbered tasks in execution order. Each names the exact file to edit and the
change to make.

1. **<Task name>** — in `<path>`, <change>. <Why if non-obvious.>
```

with:

```
## Spec

Numbered tasks in execution order. Each names the exact file to edit and the
change to make. Two formats are accepted by the task seeder and both populate
the executor's Tasks panel:

- **List item:** `N. **<Task name>** — in \`<path>\`, <change>.` — concise;
  good when each task fits on one line.
- **Subheading:** `### N. <Task name>` followed by detail paragraphs — good
  when a task needs code examples or sub-steps.

Both can coexist in the same `## Spec` section. The section ends at the next
`## ` heading (two hashes + space).

1. **<Task name>** — in `<path>`, <change>. <Why if non-obvious.>
```

Use `Edit` / `patch` with the exact old text quoted above. Do **not** reformat
surrounding WORKFLOW.md content.

## Acceptance criteria

- [ ] `cargo test -p rexymcp-executor` passes with all new tests present.
- [ ] Test `seed_from_spec_parses_heading_format_tasks` passes: a doc with
  `### 1. Fix stop condition` and `### 2. Add parser` under `## Spec` seeds
  two tasks with `id = "1"` / `id = "2"` and the correct titles.
- [ ] Test `seed_from_spec_stop_condition_does_not_fire_on_task_subheading`
  passes: a doc with `### 1. Task` followed by `### 2. Task` under `## Spec`
  (and the next section starting with `## `) seeds **2** tasks (not 0).
- [ ] Test `seed_from_spec_mixed_formats` passes: a doc mixing `1. List item`
  and `### 2. Heading item` under `## Spec` seeds 2 tasks in order.
- [ ] Test `mod_emits_progress_warning_when_task_tracking_on_and_no_tasks`
  passes: calling `execute_phase` on a `LoopDeps` with
  `task_tracking = true` and a phase doc with no `## Spec` items causes the
  mock AI to receive a request, the session log contains a `Progress` record
  with `stage = "task_seeding"` and a non-empty message.
  (See § Test plan for the exact test shape.)
- [ ] The redundant `seed_from_spec(&input.phase_doc)` call at the old
  `mod.rs:194` site is gone; `&seeded` is used instead.
- [ ] `WORKFLOW.md`'s phase-doc template `## Spec` paragraph documents both
  list-item and subheading formats.
- [ ] All four gates pass: `cargo fmt --all --check`, `cargo build` (zero
  warnings), `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test`.
- [ ] No `unwrap()` / `expect()` / `panic!()` in the new production paths.
- [ ] No `TODO` / `FIXME` / `dbg!` / `println!` / commented-out code.

## Test plan

### New unit tests in `executor/src/agent/tasks.rs` (append to the existing
`#[cfg(test)] mod tests` block)

**`seed_from_spec_parses_heading_format_tasks`**

```rust
#[test]
fn seed_from_spec_parses_heading_format_tasks() {
    let doc = "## Spec\n\n### 1. Fix stop condition\n\nSome detail.\n\n### 2. Add parser\n\n## Acceptance criteria\n";
    let tasks = seed_from_spec(doc);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "1");
    assert_eq!(tasks[0].title, "Fix stop condition");
    assert_eq!(tasks[1].id, "2");
    assert_eq!(tasks[1].title, "Add parser");
    for t in &tasks {
        assert_eq!(t.state, TaskState::Pending);
    }
}
```

**`seed_from_spec_stop_condition_does_not_fire_on_task_subheading`**

```rust
#[test]
fn seed_from_spec_stop_condition_does_not_fire_on_task_subheading() {
    let doc = "## Spec\n\n### 1. First\n\n### 2. Second\n\n## Other\n\n### 3. Not in spec\n";
    let tasks = seed_from_spec(doc);
    assert_eq!(tasks.len(), 2, "### headings in ## Spec must not stop the scan");
    assert_eq!(tasks[0].id, "1");
    assert_eq!(tasks[1].id, "2");
}
```

**`seed_from_spec_mixed_formats`**

```rust
#[test]
fn seed_from_spec_mixed_formats() {
    let doc = "## Spec\n\n1. **List item task** — do this\n\n### 2. Heading item task\n\nSome detail.\n\n## Acceptance criteria\n";
    let tasks = seed_from_spec(doc);
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "1");
    assert_eq!(tasks[0].title, "List item task");
    assert_eq!(tasks[1].id, "2");
    assert_eq!(tasks[1].title, "Heading item task");
}
```

**`parse_heading_task_line_rejects_non_heading_lines`** (negative case)

```rust
#[test]
fn parse_heading_task_line_rejects_non_heading_lines() {
    assert!(parse_heading_task_line("## Not a task heading").is_none());
    assert!(parse_heading_task_line("#### 1. Too many hashes").is_none());
    assert!(parse_heading_task_line("###1. No space after hashes").is_none());
    assert!(parse_heading_task_line("1. List item not heading").is_none());
    assert!(parse_heading_task_line("  ### 1. Indented").is_none(),
        "parse_heading_task_line calls line.trim() first so leading space is fine — \
         but strip_prefix works on the trimmed result, so this is OK; \
         adjust if the impl trims or not");
}
```

Note on the last assertion: `parse_heading_task_line` calls `line.trim()`
before `strip_prefix("### ")`, so `"  ### 1. Indented"` **will** match.
If that is intentional (task headings may be indented), remove the last
`assert!` and add a positive test instead. If it is not intended, do not
call `.trim()` — use `line.strip_prefix("### ")` directly.

The phase intent is: **task subheadings are not indented** in real phase
docs, so either behavior is acceptable. Choose the simpler implementation
(`.trim().strip_prefix`) and adjust the test accordingly.

### Integration test for the `mod.rs` warning event

Add to `executor/src/agent/tests.rs` (the split-out test file):

**`mod_emits_progress_warning_when_task_tracking_on_and_no_tasks`**

Pattern: use the existing `mock_loop_deps` / `MockAiClient` infrastructure
(look for `fn mock_loop_deps` or similar in `tests.rs`). Run `execute_phase`
with `task_tracking = true` and a phase doc that has **no** `## Spec` items.
After the call, load the session log and assert it contains a `Progress`
record with `stage = "task_seeding"` and a non-empty `message`.

Shape (adapt field names to the actual test helpers in scope):

```rust
#[tokio::test]
async fn mod_emits_progress_warning_when_task_tracking_on_and_no_tasks() {
    use crate::store::sessions::event::SessionEvent;

    let tmp = tempfile::tempdir().unwrap();
    // Phase doc with no ## Spec items
    let phase_doc = "# Phase\n\n## Goal\n\nNo spec items here.\n\n## Acceptance criteria\n\n- [ ] passes\n";
    let mut deps = mock_loop_deps(&tmp, /* task_tracking= */ true);
    // MockAiClient: respond immediately with a final message (no tool calls)
    deps.ai = /* mock that returns a non-tool-call completion on the first turn */;

    let input = PhaseInput {
        phase: "test-01".to_string(),
        phase_doc: phase_doc.to_string(),
        standards: String::new(),
    };
    let _ = execute_phase(&input, deps).await;

    // Read the session log and find the warning
    let records = crate::store::sessions::jsonl::read_session_log(
        &tmp.path().join(".rexymcp/sessions"),
        "test-01",
    )
    .unwrap_or_default();
    let warning = records.iter().find(|r| {
        matches!(&r.event, SessionEvent::Progress { turn: 0, stage, .. } if stage == "task_seeding")
    });
    assert!(warning.is_some(), "expected a task_seeding Progress warning at turn 0");
    if let Some(rec) = warning {
        if let SessionEvent::Progress { message, .. } = &rec.event {
            assert!(!message.is_empty(), "warning message must not be empty");
        }
    }
}
```

Adapt the mock setup to use the exact helpers already present in `tests.rs`.
Do not invent new helper functions — reuse what exists.

## End-to-end verification

N/A for this phase — the fix is in a pure parser and a turn-0 log event.
The integration test above covers the observable warning. A live dispatch
would be the final validation, but that is the architect's post-approval
smoke test, not the executor's gate.

## Authorizations

The following edits are explicitly authorized for this phase (CLAUDE.md hard
rules otherwise prohibit them):

- **`docs/dev/WORKFLOW.md`** — the `## Spec` paragraph in the phase-doc
  template section only. Do not touch any other WORKFLOW.md content.

## Out of scope

- Phase-02 items: `eprintln!` ×2 in `mcp/src/server.rs`, stale doc-comment
  in `read_file.rs:17`, `symbols` copy bug.
- Any change to `SessionEvent` variants, `filter.rs`, `transcript.rs`,
  dashboard rendering, or config.
- Any `Cargo.toml` edit.
- No new crate dependencies.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-06-11 02:03 (started)

**Executor:** rexyMCP executor LLM
**Working on:** Spec tasks 1–3 (stop condition fix, heading parser, redundant call, empty-spec warning, WORKFLOW.md template)

### Update — 2026-06-11 02:03 (complete)

**All spec tasks implemented.**

**Files changed:**
- `executor/src/agent/tasks.rs` — stop condition `starts_with('#')` → `starts_with("## ")`, added `parse_heading_task_line`, both parsers chained in loop, 4 new unit tests
- `executor/src/agent/mod.rs` — redundant `seed_from_spec(&input.phase_doc)` replaced with `&seeded`, empty-spec `SessionEvent::Progress` warning added
- `executor/src/agent/tests.rs` — `mod_emits_progress_warning_when_task_tracking_on_and_no_tasks` integration test
- `docs/dev/WORKFLOW.md` — `## Spec` template documents both list-item and subheading formats

**Verification commands:**
```
cargo fmt --all --check → clean
cargo build → zero warnings
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test -p rexymcp-executor → 730 passed, 0 failed, 2 ignored
```

**Grep proof — `task_seeding` literal landed:**
```
executor/src/agent/mod.rs:215:                    stage: "task_seeding".to_string(),
executor/src/agent/tests.rs:3961:            SessionEvent::Progress { turn: 0, stage, .. } if stage == "task_seeding"
executor/src/agent/tests.rs:3966:        "expected a task_seeding Progress warning at turn 0"
```

**Grep proof — `parse_heading_task_line` literal landed:**
```
executor/src/agent/tasks.rs:29:            .or_else(|| parse_heading_task_line(line))
executor/src/agent/tasks.rs:92:fn parse_heading_task_line(line: &str) -> Option<Task> {
executor/src/agent/tasks.rs:225:        assert!(parse_heading_task_line("## Not a task heading").is_none());
```

**Acceptance criteria ticked:**
- [x] `cargo test -p rexymcp-executor` passes (730 passed)
- [x] `seed_from_spec_parses_heading_format_tasks` passes
- [x] `seed_from_spec_stop_condition_does_not_fire_on_task_subheading` passes
- [x] `seed_from_spec_mixed_formats` passes
- [x] `mod_emits_progress_warning_when_task_tracking_on_and_no_tasks` passes
- [x] Redundant `seed_from_spec(&input.phase_doc)` call gone; `&seeded` used
- [x] `WORKFLOW.md` documents both formats
- [x] All four gates pass
- [x] No `unwrap()`/`expect()`/`panic!()` in new production paths
- [x] No `TODO`/`FIXME`/`dbg!`/`println!`/commented-out code

**End-to-end verification:** N/A — pure parser fix + turn-0 log event; integration test covers the observable warning.

**Notes for review:** None — straightforward bugfix, no adaptations needed.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — touched only the authorized files (`tasks.rs`,
  `mod.rs`, `tests.rs`, `WORKFLOW.md` `## Spec` template).
- **Gates (independent re-run):** fmt clean · build zero warnings · clippy
  clean · 730 executor + 344 mcp pass / 0 failed / 2 ignored.
- **Verification:** stop condition now `starts_with("## ")`;
  `parse_heading_task_line` chained via `.or_else`; the redundant
  `seed_from_spec(&input.phase_doc)` call is gone (one call remains at
  `mod.rs:121`), loop iterates `&seeded`; empty-spec `Progress` warning at
  turn 0 (`stage = "task_seeding"`). All 5 new tests pass and are
  mutation-resistant — `seed_from_spec_stop_condition_does_not_fire_on_task_subheading`
  asserts 2 tasks where the old `starts_with('#')` would have seeded 0.
  `WORKFLOW.md` `## Spec` template documents both list-item and subheading
  formats. No `unwrap`/`expect`/`panic!`/`dbg!`/`println!`/`TODO` in the new
  production paths.
- **Calibration:** none. Cosmetic-only quirk: the executor's Update Log
  self-stamps "2026-06-11 02:03" / "rexyMCP executor LLM" (the recurring
  local-LLM clock/identity quirk; machine records correct — still pending the
  `rexymcp serve` restart that activates M11 phase-06's datetime injection).
