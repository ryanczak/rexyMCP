# Phase 12: Activity pane spinner

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Estimated diff:** ~80 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Add a "throbbing" spinner as the permanently-last row of the Activity pane.
The spinner cycles through paw-print frames while the session is running,
giving the user a live heartbeat and anchoring the auto-scroll: with a
guaranteed last line in the pane, tail-follow mode always shows the newest
content at the bottom.

The spinner frames, in order:

```
"🐾"  →  "🐾🐾"  →  "🐾🐾🐾"  →  "🐾🐾🐾🐾"  →  "🐾🐾🐾"  →  "🐾🐾"  →  "🐾"
```

That is 7 frames (0–6). Frame 0 and 6 are identical; the sequence grows then
shrinks — a "throb" rather than a spin. The frame advances on every loop
tick (~500 ms), completing one full throb cycle in ~3.5 seconds.

**Active condition:** `data.summary.ended.is_none()` — the spinner shows
while the session has no `SessionEnd` record and no load error. When the
session ends (or errors), the spinner disappears so the last real event is
the final visible line.

## Architecture references

All edits are in `mcp/src/dashboard.rs` (1 774 lines — do NOT read whole;
use `start_line`/`end_line` or rely on the pre-injected excerpts below).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.
4. **Do NOT read `mcp/src/dashboard.rs` whole** — it is 1 774 lines and will
   return a truncation notice at 500. Use `read_file` with `start_line`/
   `end_line` for any context beyond the pre-injected excerpts below. Never
   issue a whole-file read.

## Current state — pre-injected excerpts

All line numbers are from the post-phase-11b commit.

### Constants block (lines 60–68)

```rust
const TRANSCRIPT_PREVIEW_MAX: usize = 100;

/// Max content lines shown per record before collapsing the rest into a
/// "… (N more lines)" marker. Keeps one large tool output from flooding the panel.
const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

// TODO: activity filter — a key binding opens a config dialog to show/hide event
// types (Prompt, Completion, ToolCall, Verify, …). Deferred: requires a modal
// overlay / input handling that doesn't exist yet.
```

### `transcript_lines` (lines 246–251)

```rust
fn transcript_lines(records: &[SessionRecord]) -> Vec<Line<'static>> {
    if records.is_empty() {
        return vec![Line::from("(no activity yet)")];
    }
    records.iter().flat_map(record_lines).collect()
}
```

Call sites:
- **Production `render_dashboard` (line 729):**
  `let transcript = transcript_lines(&data.records);`
- **Production `run_loop` scroll clamp (line 787):**
  `offset = clamp_scroll(offset, transcript_lines(&data.records).len());`
- **Test `transcript_lines_empty_placeholder` (line 1366):**
  `let lines = transcript_lines(&[]);`
- **Test `transcript_lines_flatmaps_records` (line ~1566):**
  `let lines = transcript_lines(&records);`

### `render_dashboard` signature and activity section (lines 674–751)

```rust
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    offset: u16,
    follow: bool,
    rates: BudgetRates,
) {
```

Activity pane section (lines 729–750):
```rust
    let transcript = transcript_lines(&data.records);
    let viewport = activity_area.height.saturating_sub(2); // minus top+bottom border
    // Word-wrap is enabled for the Activity panel. Paragraph::scroll counts
    // *visual* rows (post-wrap), not logical lines, so scroll-to-bottom is
    // unsolvable without measuring actual rendered heights. When tail-following,
    // we instead truncate to the last (viewport * 2) logical lines and skip
    // scrolling — the newest content naturally lands at the bottom. When the user
    // scrolls manually we keep all lines and apply the offset (which is a logical-
    // line approximation, fine for navigation but not pixel-perfect with wrapping).
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

### `run_loop` (lines 768–827)

```rust
fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use std::time::Duration;

    let mut offset: u16 = 0;
    let mut follow = true;

    loop {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let data = load_data(repo, session);
        offset = clamp_scroll(offset, transcript_lines(&data.records).len());
        terminal.draw(|frame| {
            render_dashboard(frame, frame.area(), &data, now_ms, offset, follow, rates)
        })?;

        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Up => {
                    follow = false;
                    offset = offset.saturating_sub(1);
                }
                // ... (Down, PageUp, PageDown, Home, End) ...
                KeyCode::End => {
                    follow = true;
                }
                _ => {}
            }
        }
    }

    Ok(())
}
```

## Spec

All edits in `mcp/src/dashboard.rs` only. No other file.

### 1. Add the `SPINNER_FRAMES` constant

After `TRANSCRIPT_CONTENT_MAX_LINES`:

```rust
const SPINNER_FRAMES: &[&str] = &[
    "🐾",
    "🐾🐾",
    "🐾🐾🐾",
    "🐾🐾🐾🐾",
    "🐾🐾🐾",
    "🐾🐾",
    "🐾",
];
```

### 2. Change `transcript_lines` signature — add `spinner: Option<usize>`

New signature and body:

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

Update the 4 call sites (2 production + 2 tests) to pass the appropriate
second argument. Production sites get the real `spinner` value; test sites
pass `None`:

- `render_dashboard` line 729: `transcript_lines(&data.records, spinner)`
  (add `spinner: Option<usize>` parameter to `render_dashboard` — see §3)
- `run_loop` scroll clamp line 787: `transcript_lines(&data.records, None)`
  (scroll clamp does not need the spinner line counted)
- Test `transcript_lines_empty_placeholder`: `transcript_lines(&[], None)`
- Test `transcript_lines_flatmaps_records`: `transcript_lines(&records, None)`

### 3. Add `spinner: Option<usize>` to `render_dashboard`

New signature:

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

The single call site in `run_loop` (the `terminal.draw` closure) passes the
new `spinner` argument.

### 4. Thread spinner state through `run_loop`

Add `spinner_tick: usize = 0` to the local state. Advance it on every
iteration — before the `terminal.draw` call:

```rust
let mut spinner_tick: usize = 0;

loop {
    spinner_tick = spinner_tick.wrapping_add(1);
    // ...
    let spinner_active = data.summary.ended.is_none() && data.error.is_none();
    let spinner = if spinner_active {
        Some(spinner_tick % SPINNER_FRAMES.len())
    } else {
        None
    };
    terminal.draw(|frame| {
        render_dashboard(frame, frame.area(), &data, now_ms, offset, follow, rates, spinner)
    })?;
    // scroll clamp: pass None (spinner not counted for scroll math)
    offset = clamp_scroll(offset, transcript_lines(&data.records, None).len());
```

**Note:** the scroll clamp uses `None` so the spinner row is not counted in
the total height used for offset clamping — the clamp operates on real
record lines only.

## Acceptance criteria

- [ ] The spinner appears as the **last** line of the Activity pane while the
      session is running (`summary.ended.is_none()`).
- [ ] The spinner cycles through all 7 frames in order, advancing once per
      loop tick (~500 ms per frame).
- [ ] The spinner disappears (no spinner line) when the session has ended
      (`summary.ended.is_some()`).
- [ ] The spinner disappears when `data.error` is `Some(_)`.
- [ ] The scroll-clamp calculation (`transcript_lines(..., None).len()`) does
      **not** count the spinner row, so the offset is not off by one.
- [ ] `tool.description()` / public API: no changes (this is an internal
      rendering change only).
- [ ] All 4 updated `transcript_lines` call sites pass the correct second arg.
- [ ] All previously-passing tests continue to pass.
- [ ] No new `unwrap`/`expect`/`panic!` in production paths; no `unsafe`;
      no `#[allow]`; no `println!`/`dbg!`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

New tests in `#[cfg(test)] mod tests`:

- `spinner_appended_when_active` — call `transcript_lines(&records, Some(0))`
  with a non-empty record slice; assert last line text equals `"🐾"`.
- `spinner_frame_cycles_through_all_frames` — for each frame index 0..=6
  (and also index 7, which wraps to 0), call `transcript_lines(&records, Some(i))`
  and assert the last line equals the expected frame string.
- `spinner_absent_when_none` — call `transcript_lines(&records, None)`; assert
  the last line does **not** contain `"🐾"`.
- `spinner_appended_to_empty_records` — call `transcript_lines(&[], Some(3))`;
  assert the output has **two** lines: `"(no activity yet)"` and `"🐾🐾🐾🐾"`.

## End-to-end verification

`cargo test spinner` — paste the output in the Update Log.

## Authorizations

- [x] **May modify** `mcp/src/dashboard.rs` only (source + tests).
- [ ] **No new dependencies.**
- [ ] May **NOT** modify any other file.

## Out of scope

- Configuring the spinner speed or frames.
- A different spinner in the header or other panels.
- The activity pane event filter (phase-13).

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->
