# Phase 04: Scroll overflowing task titles in the Tasks panel

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** todo
**Depends on:** phase-02 (shares the `spinner` tick counter; no code overlap)
**Estimated diff:** ~120 lines (scroll math + signature thread + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Task titles wider than the Tasks panel are currently clipped with `…`. Instead,
**pan** an overflowing title back and forth within the available width so the
whole name is readable over time. Titles that already fit do not move.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs:226–246` — `tasks_lines(summary, width)` and the
  `truncate_title` call it uses today.
- `mcp/src/dashboard/panels.rs:187–195` — `truncate_title` (the static
  fits/`…`-truncate path, reused for non-scrolling titles).
- `mcp/src/dashboard/render.rs:245` — the **only** production call site:
  `panel(" Tasks ", tasks_lines(&data.summary, tasks_inner_width))`.
- `mcp/src/dashboard/render.rs` — `render_dashboard(…, state: &ViewState, …)`;
  `state.spinner: Option<usize>` is the per-loop tick (`Some` while running,
  `None` when ended). Reuse it as the scroll clock.
- `mcp/src/dashboard/event_loop.rs:19,26` — `spinner_tick` increments once per
  ~500 ms loop iteration.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

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

Every title goes through `truncate_title(&task.title, title_max)` — static clip
with `…`. There is no scroll/tick input.

## Spec

### 1. Thread a scroll tick into `tasks_lines`

Change the signature to:

```rust
pub(crate) fn tasks_lines(
    summary: &StatusSummary,
    width: usize,
    tick: Option<usize>,
) -> Vec<Line<'static>> {
```

`tick` is `state.spinner` from the caller — `Some(n)` while the session runs,
`None` once it ends (frozen, no scrolling).

### 2. Per-task scroll decision

Replace the per-task title rendering with a scroll-aware window. Add a const and
a pure helper:

```rust
/// Loop ticks per one-character scroll advance (the tick clock runs at ~2 Hz;
/// this slows the pan to a readable speed). The user may hand-tune later.
const TASK_SCROLL_DELAY: usize = 2;

/// Window of a task title to show within `max` chars. Titles that fit are
/// returned whole. Overflowing titles pan **back and forth** (ping-pong) driven
/// by `tick`: the visible window slides 0→overflow then overflow→0, repeating.
/// `tick == None` (session ended) or a fitting title → the static head window.
fn scrolled_title(title: &str, max: usize, tick: Option<usize>) -> String {
    let chars: Vec<char> = title.chars().collect();
    if chars.len() <= max || max == 0 {
        return truncate_title(title, max);
    }
    let overflow = chars.len() - max;
    let start = match tick {
        Some(t) => {
            // Triangle wave over [0, overflow]: pan right, then back left.
            let step = t / TASK_SCROLL_DELAY;
            let period = overflow * 2;
            let phase = step % period;
            if phase <= overflow {
                phase
            } else {
                period - phase
            }
        }
        None => 0,
    };
    chars[start..start + max].iter().collect()
}
```

Then in `tasks_lines`, swap the title render to:

```rust
Span::raw(format!(" {}", scrolled_title(&task.title, title_max, tick))),
```

**Notes the executor must honor:**
- Index by **chars**, not bytes (`title` may contain multibyte glyphs) — the
  reference uses a `Vec<char>` and slices it; keep that.
- A non-scrolling title (fits, or `tick == None`) renders exactly as today via
  `truncate_title` — so the existing static tests pass with `tick = None`.
- The window is always exactly `max` chars wide for an overflowing title, so the
  line width stays stable as it pans (no reflow).

### 3. Update all `tasks_lines` call sites

The signature change touches **6** call sites. Update each by adding the new
third argument. Enumerate them in one pass (compiler E0061 will list any missed):

1. `render.rs:245` (production) — pass the live tick:
   `tasks_lines(&data.summary, tasks_inner_width, state.spinner)`.
2. `panels.rs` test `tasks_lines_empty_placeholder` (~line 791) —
   `tasks_lines(&summary, 40, None)`.
3. `panels.rs` test `tasks_lines_lists_named_tasks_with_glyphs` (~line 827) —
   `tasks_lines(&summary, 40, None)`.
4. `panels.rs` test `tasks_lines_truncates_long_title` (~line 876) —
   `tasks_lines(&summary, 26, None)`.
5. `panels.rs` test `tasks_lines_uses_full_panel_width` (~line 905) —
   `tasks_lines(&summary, 60, None)`.
6. `panels.rs` test `tasks_lines_uses_full_panel_width` (~line 913, the second
   call in the same test) — `tasks_lines(&summary, 28, None)`.

Passing `None` preserves the static behavior those tests assert, so they keep
passing unchanged otherwise.

## Acceptance criteria

- [ ] `tasks_lines` takes a third `tick: Option<usize>` argument; all 6 call
      sites compile.
- [ ] A title that fits the panel width renders identically with `tick = Some(_)`
      or `None` (no movement).
- [ ] An overflowing title's visible window changes as `tick` advances, and the
      window is always exactly `title_max` chars wide.
- [ ] The pan is ping-pong: it reaches the title's tail and returns to the head
      (does not jump/wrap discontinuously).
- [ ] `tick = None` freezes an overflowing title at its head window.
- [ ] Char-indexed (a multibyte title does not panic or split a glyph).
- [ ] All four gates pass on an independent re-run.

## Test plan

In `panels.rs`'s test module:

- Keep the existing `tasks_lines_*` tests, adding `None` as the third arg.
  `tasks_lines_truncates_long_title` (width 26, `None`) still asserts the static
  `…` truncation — confirming the `tick = None` path equals today's behavior.
- `scrolled_title_returns_whole_when_fits` — `scrolled_title("short", 20,
  Some(5))` == `"short"` (no movement).
- `scrolled_title_pans_overflowing_title` — a title of 30 chars, `max = 10`:
  assert the window at `tick = Some(0)` starts at the head, and at a later tick
  (e.g. `Some(TASK_SCROLL_DELAY * 3)`) starts 3 chars in. Mutation-resistant: an
  impl that ignores `tick` (always head) fails the later-tick assertion.
- `scrolled_title_ping_pongs` — over a full `period` of ticks the start index
  rises to `overflow` then falls back to 0 (assert the max start reached equals
  `overflow` and the sequence is non-monotonic). Mutation-resistant vs a
  wrap-around impl (which would jump `overflow → 0` discontinuously and never
  produce the descending half).
- `scrolled_title_frozen_when_tick_none` — overflowing title, `tick = None`,
  start window is the head (== `truncate_title` head minus the `…`? — assert it
  equals the first `max` chars).
- `scrolled_title_char_indexed_multibyte` — a title with multibyte chars (e.g.
  `"日本語テスト"` repeated past `max`) does not panic and returns `max` chars.

## End-to-end verification

The pan is a live TUI animation; pin behavior via the `scrolled_title` unit tests
and declare the live render E2E-N/A (consistent with prior dashboard-panel
phases). If you run `cargo run -p rexymcp -- dashboard …` against a session whose
tasks have long titles, note that overflowing titles pan back and forth while
short ones stay still.

## Authorizations

None. No new dependencies. No `docs/architecture.md` change.

## Out of scope

- Scrolling the gauge line or the milestone line — titles only.
- A per-task independent phase offset (all overflowing titles share the same
  tick clock; staggering them is a later tweak).
- Pausing at the ends — the user will hand-tune cadence/pauses later.
- Changing `TASK_SCROLL_DELAY` semantics beyond the simple divisor.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
