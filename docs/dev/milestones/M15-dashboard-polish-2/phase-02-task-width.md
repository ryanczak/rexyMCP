# Phase 02 — Width-aware task title truncation

**Status:** todo
**Milestone:** M15-dashboard-polish-2
**Depends on:** phase-01 (done)

## Goal

Task titles in the Tasks panel are currently truncated at 24 chars regardless
of actual panel width, leaving a gap between the ellipsis and the right border.
Replace the hardcoded `TASK_TITLE_MAX = 24` constant with a `width: usize`
parameter on `tasks_lines` so the available inner width is used instead.

Two files change:
- `mcp/src/dashboard/panels.rs` — add `width: usize` param to `tasks_lines`,
  remove the `TASK_TITLE_MAX` constant, derive title budget from `width`.
- `mcp/src/dashboard/render.rs` — compute `tasks_inner_width` from `tasks_area`
  and pass it to `tasks_lines`.

Three test call sites inside `panels.rs` need the new argument. One new test
pins the width-aware behaviour (mutation-resistant vs a hardcoded-constant impl).

~25 lines of diff. No config changes, no new `SessionEvent`, no `Cargo.toml`.

## Architecture references

- `mcp/src/dashboard/panels.rs:17–18` — `TASK_TITLE_MAX` constant (to remove)
- `mcp/src/dashboard/panels.rs:201–218` — `tasks_lines` (to change)
- `mcp/src/dashboard/panels.rs:161–168` — `truncate_title` (unchanged)
- `mcp/src/dashboard/panels.rs:758–861` — three test call sites (to update)
- `mcp/src/dashboard/render.rs:243` — sole production call site (to update)
- `mcp/src/dashboard/render.rs:145` — pattern precedent for computing inner width

## Pre-flight

- [ ] `cargo test` passes on the current tree.
- [ ] Confirm `panels.rs:18` is `const TASK_TITLE_MAX: usize = 24;`.
- [ ] Confirm `panels.rs:201` is `pub(crate) fn tasks_lines(summary: &StatusSummary) -> Vec<Line<'static>> {`.
- [ ] Confirm `render.rs:243` is `frame.render_widget(panel(" Tasks ", tasks_lines(&data.summary)), tasks_area);`.

## Current state

### `panels.rs:17–18` — constant to remove

```rust
/// Max chars for a task title in the Tasks panel (narrow ~28%-width right column).
const TASK_TITLE_MAX: usize = 24;
```

### `panels.rs:201–218` — `tasks_lines` current signature and body

```rust
pub(crate) fn tasks_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.tasks_total == 0 {
        return vec![Line::from("(no tasks tracked yet)")];
    }
    let mut lines = vec![tasks_gauge_line(summary.tasks_done, summary.tasks_total)];
    for task in &summary.tasks {
        let (glyph, color) = match task.state {
            TaskState::Done => ("☑", Color::Green),
            TaskState::Active => ("▶", Color::Yellow),
            TaskState::Pending => ("☐", Color::Rgb(200, 200, 200)),
        };
        lines.push(Line::from(vec![
            Span::styled(glyph, Style::new().fg(color)),
            Span::raw(format!(" {}", truncate_title(&task.title, TASK_TITLE_MAX))),
        ]));
    }
    lines
}
```

### `render.rs:243` — call site to update

```rust
frame.render_widget(panel(" Tasks ", tasks_lines(&data.summary)), tasks_area);
```

Immediately above (line 168–170), `tasks_area` is already in scope (from the
`Layout::vertical` split of `right_area`). The pattern for computing inner width
is already established at `render.rs:145`:
```rust
let session_inner_width = session_area.width.saturating_sub(2) as usize;
```
Use the same pattern with `tasks_area`.

### `panels.rs:758–861` — three test call sites

```
fn tasks_lines_empty_placeholder      — line 763: tasks_lines(&summary)
fn tasks_lines_lists_named_tasks...   — line 799: tasks_lines(&summary)
fn tasks_lines_truncates_long_title   — line 848: tasks_lines(&summary)
```

Each needs a `width` argument added.

## Spec

### Task 1 — Update `tasks_lines` signature and remove constant (`panels.rs`)

**Step A:** Delete the constant at `panels.rs:17–18`:
```rust
/// Max chars for a task title in the Tasks panel (narrow ~28%-width right column).
const TASK_TITLE_MAX: usize = 24;
```
(Both the doc-comment line and the `const` line.)

**Step B:** Replace the function at `panels.rs:201`:

```rust
pub(crate) fn tasks_lines(summary: &StatusSummary, width: usize) -> Vec<Line<'static>> {
    if summary.tasks_total == 0 {
        return vec![Line::from("(no tasks tracked yet)")];
    }
    let title_max = width.saturating_sub(2); // 1 glyph cell + 1 space
    let mut lines = vec![tasks_gauge_line(summary.tasks_done, summary.tasks_total)];
    for task in &summary.tasks {
        let (glyph, color) = match task.state {
            TaskState::Done => ("☑", Color::Green),
            TaskState::Active => ("▶", Color::Yellow),
            TaskState::Pending => ("☐", Color::Rgb(200, 200, 200)),
        };
        lines.push(Line::from(vec![
            Span::styled(glyph, Style::new().fg(color)),
            Span::raw(format!(" {}", truncate_title(&task.title, title_max))),
        ]));
    }
    lines
}
```

The only changes from the current body are:
- New `width: usize` parameter.
- `let title_max = width.saturating_sub(2);` replaces `TASK_TITLE_MAX` at the use site.
- `truncate_title(&task.title, title_max)` instead of `truncate_title(&task.title, TASK_TITLE_MAX)`.

### Task 2 — Update the render.rs call site

Replace `render.rs:243`:
```rust
// Before:
frame.render_widget(panel(" Tasks ", tasks_lines(&data.summary)), tasks_area);

// After:
let tasks_inner_width = tasks_area.width.saturating_sub(2) as usize;
frame.render_widget(panel(" Tasks ", tasks_lines(&data.summary, tasks_inner_width)), tasks_area);
```

`tasks_area` is already in scope (from the `Layout::vertical` split above).
The `saturating_sub(2)` accounts for the panel border (1px each side).

### Task 3 — Update three test call sites (`panels.rs` test module)

Three tests call `tasks_lines` without a width argument. Add the argument to each:

**`tasks_lines_empty_placeholder` (~line 763):**
```rust
// Before:
let lines = tasks_lines(&summary);
// After:
let lines = tasks_lines(&summary, 40);
```
Width 40 is irrelevant — the function returns early before reaching any
truncation logic when `tasks_total == 0`.

**`tasks_lines_lists_named_tasks_with_glyphs` (~line 799):**
```rust
// Before:
let lines = tasks_lines(&summary);
// After:
let lines = tasks_lines(&summary, 40);
```
Width 40 gives `title_max = 38`. All task names in this test are short; no
truncation occurs.

**`tasks_lines_truncates_long_title` (~line 848):**
```rust
// Before:
let lines = tasks_lines(&summary);
// After:
let lines = tasks_lines(&summary, 26);
```
Width 26 gives `title_max = 24` — matching the old `TASK_TITLE_MAX = 24` so
the long title "This is a very long task title that should be truncated" (55
chars) still triggers truncation and the short title "Short" (5 chars) still
fits. Both existing assertions remain valid.

### Task 4 — Add mutation-resistant test `tasks_lines_uses_full_panel_width`

Add this test to the `tasks_lines` test block in `panels.rs`:

```rust
#[test]
fn tasks_lines_uses_full_panel_width() {
    use crate::status::TaskRow;
    let title_50 = "A".repeat(50);
    let summary = StatusSummary {
        tasks_total: 1,
        tasks: vec![TaskRow {
            id: "1".into(),
            title: title_50.clone(),
            state: TaskState::Pending,
        }],
        ..StatusSummary::default()
    };
    // width=60: title_max=58, 50-char title fits without truncation.
    let lines = tasks_lines(&summary, 60);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(
        text.iter().any(|s| s.contains(&title_50) && !s.contains('…')),
        "50-char title should not be truncated at width=60: {text:?}"
    );
    // width=28: title_max=26, 50-char title is truncated.
    let lines_narrow = tasks_lines(&summary, 28);
    let text_narrow: Vec<String> = lines_narrow.iter().map(|l| format!("{l}")).collect();
    assert!(
        text_narrow.iter().any(|s| s.contains('…')),
        "50-char title should be truncated at width=28: {text_narrow:?}"
    );
}
```

**Why this is mutation-resistant:** an implementation that ignores `width` and
uses a hardcoded `TASK_TITLE_MAX = 24` would truncate the 50-char title even at
`width=60`, failing the first assertion. The second assertion (`width=28` →
truncation) also fails a naïve "always show full title" implementation.

## Acceptance criteria

- [ ] `tasks_lines` signature is `pub(crate) fn tasks_lines(summary: &StatusSummary, width: usize)`.
- [ ] `const TASK_TITLE_MAX` is deleted from `panels.rs`.
- [ ] `render.rs` computes `tasks_inner_width` from `tasks_area.width.saturating_sub(2) as usize`
      and passes it to `tasks_lines`.
- [ ] All three existing `tasks_lines` test call sites compile with the new signature.
- [ ] New test `tasks_lines_uses_full_panel_width` passes and is mutation-resistant.
- [ ] All four gates pass on an independent re-run.

## Test plan

- **Updated call sites (3):** `tasks_lines_empty_placeholder`, `tasks_lines_lists_named_tasks_with_glyphs`,
  `tasks_lines_truncates_long_title` — add `width` arg, behavior unchanged.
- **New test:** `tasks_lines_uses_full_panel_width` — pins width-aware truncation.
  Mutation-resistant on both arms (no-truncate at wide width; truncate at narrow width).

## End-to-end verification

E2E is N/A (TUI rendering — no headless harness, consistent with all prior
dashboard phases). Verify visually: run `rexymcp dashboard` with a session that
has tasks with long names; confirm titles extend further toward the right border
than before (previously capped at 24 chars; now fills the panel minus 2 for the
glyph/space overhead).

## Authorizations

No cross-scope changes. Both files are in `mcp/src/dashboard/`. No config,
no executor crate, no `SessionEvent`, no `Cargo.toml`.

## Out of scope

- Wrapping task titles across multiple lines.
- Changing the gauge or any other part of `tasks_lines` output.
- Any change to `truncate_title` itself.

## Update Log

<!-- entries appended below this line -->
