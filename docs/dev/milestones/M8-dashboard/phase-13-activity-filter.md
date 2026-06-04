# Phase 13: Activity pane event filter

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** phase-12 (Activity pane spinner) — the `transcript_lines`
signature must already carry `spinner: Option<usize>` as its second parameter.
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add a per-event-type filter to the Activity pane. Pressing `f` opens a
filter panel inside the Activity pane; the user navigates with `j`/`k`
(or arrow keys) and toggles event types with `Space`. `f` or `Esc` closes
the panel and returns to the transcript view.

**Defaults:** all event types **enabled** (visible) except `progress:` events,
which are **disabled** by default — they are frequent and rarely useful.

## Architecture references

All edits are in `mcp/src/dashboard.rs` (1 774 lines after phase-11b, growing
with phase-12). Do NOT read this file whole — use `start_line`/`end_line` for
any context beyond the pre-injected excerpts below.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.
4. Confirm phase-12 is `done`: `transcript_lines` already takes
   `(records: &[SessionRecord], spinner: Option<usize>)`.
5. **Do NOT read `mcp/src/dashboard.rs` whole.** It is >1 800 lines. Use
   `read_file` with `start_line`/`end_line` for sections you need beyond
   what is pre-injected below.

## Current state (post phase-12) — pre-injected excerpts

### `transcript_lines` (phase-12 form)

```rust
fn transcript_lines(records: &[SessionRecord], spinner: Option<usize>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = if records.is_empty() {
        vec![Line::from("(no activity yet)")]
    } else {
        records.iter().flat_map(record_lines).collect()
    };
    if let Some(frame) = spinner {
        let glyph = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
        lines.push(Line::from(glyph.to_string()));
    }
    lines
}
```

Call sites (post phase-12):
- **`render_dashboard` line ~729:**
  `let transcript = transcript_lines(&data.records, spinner);`
- **`run_loop` scroll clamp line ~787:**
  `offset = clamp_scroll(offset, transcript_lines(&data.records, None).len());`
- **Test `transcript_lines_empty_placeholder`:**
  `let lines = transcript_lines(&[], None);`
- **Test `transcript_lines_flatmaps_records`:**
  `let lines = transcript_lines(&records, None);`

### `render_dashboard` signature (post phase-12)

```rust
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    offset: u16,
    follow: bool,
    rates: BudgetRates,
    spinner: Option<usize>,
) {
```

Activity area rendering (post phase-12, lines ~729–750):
```rust
    let transcript = transcript_lines(&data.records, spinner);
    let viewport = activity_area.height.saturating_sub(2);
    let n = transcript.len();
    let (display_lines, scroll_rows) = if follow {
        let keep = (viewport as usize * 2).min(n);
        (transcript[n.saturating_sub(keep)..].to_vec(), 0u16)
    } else {
        let display = visible_offset(false, offset, n, viewport);
        (transcript, display)
    };
    let activity = Paragraph::new(display_lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_rows, 0))
        .block(Block::default().borders(Borders::ALL).title(" Activity "));
    frame.render_widget(activity, activity_area);
```

### `run_loop` key-handling block (post phase-12, within the `if event::poll...` arm)

```rust
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Up => {
                    follow = false;
                    offset = offset.saturating_sub(1);
                }
                KeyCode::Down => {
                    follow = false;
                    offset = offset.saturating_add(1);
                }
                KeyCode::PageUp => {
                    follow = false;
                    offset = offset.saturating_sub(10);
                }
                KeyCode::PageDown => {
                    follow = false;
                    offset = offset.saturating_add(10);
                }
                KeyCode::Home => {
                    follow = false;
                    offset = 0;
                }
                KeyCode::End => {
                    follow = true;
                }
                _ => {}
            }
```

And the local state at the top of `run_loop` (post phase-12):
```rust
    let mut offset: u16 = 0;
    let mut follow = true;
    let mut spinner_tick: usize = 0;
```

And the `terminal.draw` + scroll-clamp block (post phase-12):
```rust
        let spinner_active = data.summary.ended.is_none() && data.error.is_none();
        let spinner = if spinner_active {
            Some(spinner_tick % SPINNER_FRAMES.len())
        } else {
            None
        };
        terminal.draw(|frame| {
            render_dashboard(frame, frame.area(), &data, now_ms, offset, follow, rates, spinner)
        })?;
        offset = clamp_scroll(offset, transcript_lines(&data.records, None).len());
```

### `SessionEvent` variants (from `executor`)

All 11 variants the filter must handle:
`SessionStart`, `Prompt`, `Completion`, `Parsed`, `ParseFailed`, `ToolResult`,
`Verify`, `HardFail`, `Progress`, `SessionEnd`, `Metrics`, `Compaction`.

## Spec

All edits in `mcp/src/dashboard.rs` only. No other file.

### 1. Add `ActivityFilter` + `FilterState`

Near the constants block (after `SPINNER_FRAMES`), add:

```rust
const FILTER_ITEM_COUNT: usize = 11;

/// Per-event-type visibility toggles for the Activity pane.
/// All enabled by default except `progress` (too noisy).
#[derive(Clone, Debug, PartialEq)]
struct ActivityFilter {
    session: bool,   // SessionStart + SessionEnd
    prompt: bool,
    completion: bool,
    tool_call: bool, // Parsed
    parse_failed: bool,
    tool_result: bool,
    verify: bool,
    hard_fail: bool,
    progress: bool,  // DEFAULT false
    metrics: bool,
    compaction: bool,
}

impl Default for ActivityFilter {
    fn default() -> Self {
        Self {
            session: true,
            prompt: true,
            completion: true,
            tool_call: true,
            parse_failed: true,
            tool_result: true,
            verify: true,
            hard_fail: true,
            progress: false,
            metrics: true,
            compaction: true,
        }
    }
}

impl ActivityFilter {
    fn allows(&self, event: &SessionEvent) -> bool {
        match event {
            SessionEvent::SessionStart { .. } | SessionEvent::SessionEnd { .. } => self.session,
            SessionEvent::Prompt { .. } => self.prompt,
            SessionEvent::Completion { .. } => self.completion,
            SessionEvent::Parsed { .. } => self.tool_call,
            SessionEvent::ParseFailed { .. } => self.parse_failed,
            SessionEvent::ToolResult { .. } => self.tool_result,
            SessionEvent::Verify { .. } => self.verify,
            SessionEvent::HardFail { .. } => self.hard_fail,
            SessionEvent::Progress { .. } => self.progress,
            SessionEvent::Metrics { .. } => self.metrics,
            SessionEvent::Compaction { .. } => self.compaction,
        }
    }

    fn toggle(&mut self, index: usize) {
        match index {
            0 => self.session = !self.session,
            1 => self.prompt = !self.prompt,
            2 => self.completion = !self.completion,
            3 => self.tool_call = !self.tool_call,
            4 => self.parse_failed = !self.parse_failed,
            5 => self.tool_result = !self.tool_result,
            6 => self.verify = !self.verify,
            7 => self.hard_fail = !self.hard_fail,
            8 => self.progress = !self.progress,
            9 => self.metrics = !self.metrics,
            10 => self.compaction = !self.compaction,
            _ => {}
        }
    }

    fn is_enabled(&self, index: usize) -> bool {
        match index {
            0 => self.session,
            1 => self.prompt,
            2 => self.completion,
            3 => self.tool_call,
            4 => self.parse_failed,
            5 => self.tool_result,
            6 => self.verify,
            7 => self.hard_fail,
            8 => self.progress,
            9 => self.metrics,
            10 => self.compaction,
            _ => false,
        }
    }

    fn item_label(index: usize) -> &'static str {
        match index {
            0 => "session start/end",
            1 => "prompt",
            2 => "completion",
            3 => "tool call",
            4 => "parse fail",
            5 => "tool result",
            6 => "verify",
            7 => "hard fail",
            8 => "progress",
            9 => "metrics",
            10 => "compaction",
            _ => "?",
        }
    }
}

/// Filter panel UI state — open/closed, cursor position, current settings.
struct FilterState {
    open: bool,
    cursor: usize,
    filter: ActivityFilter,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            open: false,
            cursor: 0,
            filter: ActivityFilter::default(),
        }
    }
}
```

### 2. Change `transcript_lines` to accept `&ActivityFilter`

Add `filter: &ActivityFilter` as the **second** parameter (before `spinner`):

```rust
fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
    spinner: Option<usize>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = {
        let visible: Vec<_> = records.iter()
            .filter(|r| filter.allows(&r.event))
            .collect();
        if visible.is_empty() {
            vec![Line::from("(no activity yet)")]
        } else {
            visible.iter().flat_map(|r| record_lines(r)).collect()
        }
    };
    if let Some(frame) = spinner {
        let glyph = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
        lines.push(Line::from(glyph.to_string()));
    }
    lines
}
```

Update **all 4 call sites** to pass the filter as the second argument:

- `render_dashboard`: `transcript_lines(&data.records, &filter_state.filter, spinner)` (see §3)
- `run_loop` scroll clamp: `transcript_lines(&data.records, &filter_state.filter, None).len()`
- Test `transcript_lines_empty_placeholder`: `transcript_lines(&[], &ActivityFilter::default(), None)`
- Test `transcript_lines_flatmaps_records`: `transcript_lines(&records, &ActivityFilter::default(), None)`

### 3. Add `filter_state: &FilterState` to `render_dashboard`

New signature (after `spinner`):

```rust
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    offset: u16,
    follow: bool,
    rates: BudgetRates,
    spinner: Option<usize>,
    filter_state: &FilterState,
) {
```

In the Activity pane section, branch on `filter_state.open`:

```rust
    if filter_state.open {
        // Filter panel replaces the transcript while open.
        let mut filter_lines: Vec<Line<'static>> = (0..FILTER_ITEM_COUNT)
            .map(|i| {
                let check = if filter_state.filter.is_enabled(i) { "✓" } else { "✗" };
                let label = ActivityFilter::item_label(i);
                let text = format!(" {check}  {label}");
                if i == filter_state.cursor {
                    Line::from(Span::styled(
                        text,
                        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(text)
                }
            })
            .collect();
        filter_lines.push(Line::from(Span::styled(
            " ↑↓/jk move · space toggle · f/Esc close",
            Style::new().fg(Color::DarkGray),
        )));
        frame.render_widget(
            Paragraph::new(filter_lines)
                .block(Block::default().borders(Borders::ALL).title(" Activity [filter] ")),
            activity_area,
        );
    } else {
        let transcript = transcript_lines(&data.records, &filter_state.filter, spinner);
        let viewport = activity_area.height.saturating_sub(2);
        let n = transcript.len();
        let (display_lines, scroll_rows) = if follow {
            let keep = (viewport as usize * 2).min(n);
            (transcript[n.saturating_sub(keep)..].to_vec(), 0u16)
        } else {
            let display = visible_offset(false, offset, n, viewport);
            (transcript, display)
        };
        frame.render_widget(
            Paragraph::new(display_lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll_rows, 0))
                .block(Block::default().borders(Borders::ALL).title(" Activity [f=filter] ")),
            activity_area,
        );
    }
```

**Title change:** when filter is closed, the Activity title shows
`" Activity [f=filter] "` to hint the `f` key. When the filter panel is
open, the title shows `" Activity [filter] "`.

### 4. Update `run_loop` to carry `FilterState` and new key handling

Add `let mut filter_state = FilterState::default();` to the local state block.

Replace the key-handling `match key.code { … }` with a mode-aware branch:

```rust
            if filter_state.open {
                match key.code {
                    KeyCode::Char('f') | KeyCode::Esc => filter_state.open = false,
                    KeyCode::Char('j') | KeyCode::Down => {
                        filter_state.cursor =
                            (filter_state.cursor + 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        filter_state.cursor =
                            (filter_state.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        filter_state.filter.toggle(filter_state.cursor);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('f') => {
                        filter_state.open = true;
                        filter_state.cursor = 0;
                    }
                    KeyCode::Up => {
                        follow = false;
                        offset = offset.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        follow = false;
                        offset = offset.saturating_add(1);
                    }
                    KeyCode::PageUp => {
                        follow = false;
                        offset = offset.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        follow = false;
                        offset = offset.saturating_add(10);
                    }
                    KeyCode::Home => {
                        follow = false;
                        offset = 0;
                    }
                    KeyCode::End => {
                        follow = true;
                    }
                    _ => {}
                }
            }
```

Update the `terminal.draw` call to pass `&filter_state`:

```rust
        terminal.draw(|frame| {
            render_dashboard(
                frame, frame.area(), &data, now_ms, offset, follow, rates,
                spinner, &filter_state,
            )
        })?;
```

And the scroll clamp:

```rust
        offset = clamp_scroll(
            offset,
            transcript_lines(&data.records, &filter_state.filter, None).len(),
        );
```

### 5. Remove the stale TODO comment

The TODO at lines ~66–68 (`// TODO: activity filter — …`) can now be deleted:

```
// TODO: activity filter — a key binding opens a config dialog to show/hide event
// types (Prompt, Completion, ToolCall, Verify, …). Deferred: requires a modal
// overlay / input handling that doesn't exist yet.
```

## Acceptance criteria

- [ ] `ActivityFilter::default()` has `progress: false` and all other fields
      `true`.
- [ ] `filter.allows(SessionEvent::Progress { … })` returns `false` for the
      default filter (**negative**).
- [ ] `filter.allows(SessionEvent::Completion { … })` returns `true` for the
      default filter (**positive**).
- [ ] Toggling `progress` (index 8) via `filter.toggle(8)` flips `false` →
      `true` and `true` → `false`.
- [ ] `transcript_lines` with the default filter excludes `Progress` records;
      a slice containing only `Progress` records returns `"(no activity yet)"`.
- [ ] `transcript_lines` with a filter where `progress: true` includes
      `Progress` records.
- [ ] Pressing `f` opens the filter panel (Activity title changes to
      `" Activity [filter] "`; `Esc` or `f` closes it — **tested via the
      `FilterState` + key-handling logic, not a TUI integration test**).
- [ ] Filter cursor wraps: from item 10 `Down`/`j` → item 0; from item 0
      `Up`/`k` → item 10.
- [ ] When filter is open, `Esc` closes it; it does **not** quit the
      dashboard (only `q`/`Esc` when filter is closed quits).
- [ ] All 4 `transcript_lines` call sites pass the filter as the second arg.
- [ ] All four required commands pass with zero new warnings.
- [ ] All previously-passing tests continue to pass (updated call sites use
      `&ActivityFilter::default()`).

## Test plan

New tests in `#[cfg(test)] mod tests`:

- `filter_default_disables_progress` — `ActivityFilter::default().progress == false`.
- `filter_allows_progress_when_enabled` — set `progress: true`, assert
  `filter.allows(&progress_record.event) == true`.
- `filter_blocks_progress_by_default` — default filter, assert
  `filter.allows(&progress_record.event) == false`.
- `filter_toggle_flips_field` — toggle index 8 twice; assert `false → true → false`.
- `transcript_lines_excludes_filtered_events` — build a record slice with
  one `Progress` and one `SessionStart`; call with default filter; assert
  output does **not** contain `"progress:"` and **does** contain
  `"session start"`.
- `transcript_lines_all_filtered_shows_placeholder` — record slice with only
  `Progress` records, default filter; assert output contains `"no activity yet"`.
- `filter_cursor_wraps_forward` — start cursor at `FILTER_ITEM_COUNT - 1`,
  apply the "j" increment: `(cursor + 1) % FILTER_ITEM_COUNT == 0`.
- `filter_cursor_wraps_backward` — start cursor at 0, apply the "k"
  decrement: `(cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT == FILTER_ITEM_COUNT - 1`.

## End-to-end verification

`cargo test filter` — paste the output in the Update Log.

## Authorizations

- [x] **May modify** `mcp/src/dashboard.rs` only (source + tests).
- [x] **May delete** the stale `// TODO: activity filter …` comment block.
- [ ] **No new dependencies.**
- [ ] May **NOT** modify any other file.

## Out of scope

- Persisting filter settings across dashboard restarts (would require config
  file I/O).
- Filtering inside the Files or Budget panels.
- A `--hide-progress` CLI flag (a future phase could add that).
- Any change to `record_lines` — it renders a single record without
  awareness of the filter; the filter operates at the `transcript_lines`
  level.

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
