# Phase 03: Activity — line wrapping + tail-follow autoscroll over wrapped lines + scrollbar

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** phase-02 (done) — the transcript bodies it added are the long
lines this phase wraps; no code dependency, just the same `record_lines` surface.
**Estimated diff:** ~110 lines (helper ~35 prod, render branch ~20, event-loop ~6, tests ~50)
**Tags:** language=rust, kind=feature, size=m

## Goal

Long Activity-transcript lines currently run off the right edge of the panel
(ratatui truncates them) and the tail-follow autoscroll counts **pre-wrap**
lines, so when records wrap the view no longer pins to the true bottom. Make the
transcript **wrap** every line to the panel's inner width, fix the follow/scroll
math to count the **wrapped** lines, and add a position **scrollbar** on the
right edge. Pure presentation — no feed, config, or executor change (items #8,
#9, R1).

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  "display only" constraint and the phase table. This phase touches **only**
  `mcp/src/dashboard/render.rs` and `mcp/src/dashboard/event_loop.rs`; it adds
  no `SessionEvent`, no config, no `StatusSummary` field.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the milestone README above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The transcript is rendered in `render_dashboard` in
`mcp/src/dashboard/render.rs`. Today the **non-filter branch** (lines 140–153)
builds a `Paragraph` with **no wrapping** and scrolls it by a count taken from
`transcript_lines(...).len()` — the **pre-wrap** line count:

```rust
} else {
    let transcript = transcript_lines(&data.records, &filter_state.filter);
    let viewport = activity_area.height.saturating_sub(2); // minus top+bottom border
    let n = transcript.len();
    let scroll = visible_offset(state.follow, state.offset, n, viewport);
    frame.render_widget(
        Paragraph::new(transcript).scroll((scroll, 0)).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Activity [f=filter] "),
        ),
        activity_area,
    );
}
```

`render_dashboard` currently returns `()`.

The two scroll-math helpers are already correct and **stay unchanged** — they are
generic over whatever `total_lines` you feed them (`render.rs:26-38`):

```rust
pub(crate) fn clamp_scroll(scroll: u16, total_lines: usize) -> u16 {
    let max = total_lines.saturating_sub(1) as u16;
    scroll.min(max)
}
pub(crate) fn visible_offset(follow: bool, offset: u16, total_lines: usize, viewport: u16) -> u16 {
    let total = total_lines.min(u16::MAX as usize) as u16;
    let max = total.saturating_sub(viewport);
    if follow { max } else { offset.min(max) }
}
```

The **only** correctness bug is *which* total is passed in: pre-wrap today,
wrapped after this phase. A line that wraps to 3 rows must advance the follow
offset by 3, not 1.

The event loop (`mcp/src/dashboard/event_loop.rs:44-49`) draws, then clamps the
stored manual `offset` against the same pre-wrap count:

```rust
terminal
    .draw(|frame| render_dashboard(frame, frame.area(), &data, now_ms, &state, rates))?;
offset = clamp_scroll(
    offset,
    transcript_lines(&data.records, &filter_state.filter).len(),
);
```

The event loop has **no panel width**, so it cannot compute the wrapped count
itself — `render_dashboard` (which has `activity_area`) must compute it and hand
it back.

### Why we wrap ourselves and do NOT use `Paragraph::line_count` / ratatui `Wrap`

The "obvious" path is to set `.wrap(Wrap { trim: false })` on the Paragraph and
ask ratatui for the wrapped count via `Paragraph::line_count(width)`. **Do not do
this.** In the pinned `ratatui = "0.30"` (no extra features in `mcp/Cargo.toml`),
`Paragraph::line_count` / `line_width` are gated behind the **unstable** Cargo
feature `unstable-rendered-line-info`:

```rust
// ratatui-widgets-0.3.0/src/paragraph.rs
#[instability::unstable(feature = "rendered-line-info", ...)]
pub fn line_count(&self, width: u16) -> usize { ... }
```

Enabling that feature is a `mcp/Cargo.toml` edit **and** an unstable-API
dependency — both are out of bounds for M13 (display-only, no manifest churn) and
neither is authorized here. Instead, **pre-wrap the lines ourselves** with a pure
helper and render the already-fitting lines with **no** ratatui `Wrap`. Two
payoffs: the rendered rows are exactly the wrapped rows we counted (the follow
math is correct *by construction*, not by matching an opaque algorithm), and the
helper is unit-testable without a terminal.

## Spec

All changes are in `render.rs` and `event_loop.rs`. No other files.

### 1. Add a pure span-preserving wrap helper — `render.rs`

Add these two functions (place them near `clamp_scroll`/`visible_offset`, above
`render_dashboard`). `wrap_line` hard-wraps one styled `Line` to `width`
characters, **splitting spans at the wrap column so styles are preserved**;
`wrap_lines` flat-maps it over a slice. Copy this shape — it is the load-bearing
code of the phase:

```rust
/// Hard-wrap one styled line to `width` characters, preserving span styles by
/// splitting spans at the wrap column. A line that already fits returns a single
/// row; an empty line returns a single empty row. Char-count based (not unicode
/// display width) — a wide-glyph line may still clip by a cell, acceptable here.
pub(crate) fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line.clone()];
    }
    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in &line.spans {
        let mut buf = String::new();
        for ch in span.content.chars() {
            if col == width {
                if !buf.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut buf), span.style));
                }
                rows.push(Line::from(std::mem::take(&mut cur)));
                col = 0;
            }
            buf.push(ch);
            col += 1;
        }
        if !buf.is_empty() {
            cur.push(Span::styled(std::mem::take(&mut buf), span.style));
        }
    }
    rows.push(Line::from(cur));
    rows
}

/// Wrap every line in `lines` to `width` characters (see `wrap_line`).
pub(crate) fn wrap_lines(lines: &[Line<'static>], width: usize) -> Vec<Line<'static>> {
    lines.iter().flat_map(|l| wrap_line(l, width)).collect()
}
```

### 2. Wrap, fix the scroll total, and add the scrollbar — `render.rs`

Change `render_dashboard` to **return `usize`** — the wrapped line total of the
Activity transcript, so the event loop can clamp against it. Concretely:

- Add `let mut total_wrapped = 0usize;` near the top of the function body.
- In the **error** early-return branch, `return 0;` instead of `return;`.
- In the **filter-open** branch, before it ends, set `total_wrapped` so the
  manual offset stays clamped even while the filter overlay is up:
  ```rust
  total_wrapped = wrap_lines(
      &transcript_lines(&data.records, &filter_state.filter),
      activity_area.width.saturating_sub(2) as usize,
  )
  .len();
  ```
- Replace the **non-filter** branch (current lines 140–153) with:
  ```rust
  } else {
      let transcript = transcript_lines(&data.records, &filter_state.filter);
      let inner_width = activity_area.width.saturating_sub(2) as usize;
      let wrapped = wrap_lines(&transcript, inner_width);
      total_wrapped = wrapped.len();
      let viewport = activity_area.height.saturating_sub(2);
      let scroll = visible_offset(state.follow, state.offset, total_wrapped, viewport);
      frame.render_widget(
          Paragraph::new(wrapped).scroll((scroll, 0)).block(
              Block::default()
                  .borders(Borders::ALL)
                  .title(" Activity [f=filter] "),
          ),
          activity_area,
      );
      let mut sb_state = ScrollbarState::new(total_wrapped)
          .viewport_content_length(viewport as usize)
          .position(scroll as usize);
      frame.render_stateful_widget(
          Scrollbar::new(ScrollbarOrientation::VerticalRight)
              .begin_symbol(Some("↑"))
              .end_symbol(Some("↓")),
          activity_area.inner(Margin::new(0, 1)),
          &mut sb_state,
      );
  }
  ```
  Render the **pre-wrapped** `wrapped` lines with **no** `.wrap(...)` — they
  already fit `inner_width`, so ratatui will not truncate. Do **not** add a
  `Wrap`.
- End the function with `total_wrapped` (the return value).

`Margin::new(horizontal, vertical)` — `Margin::new(0, 1)` keeps the scrollbar on
the right border column, inset one row top and bottom so it sits inside the box
borders (verified shape: `ratatui-core` `Margin::new(horizontal: u16, vertical:
u16)`; `Rect::inner(self, margin: Margin)`). `ScrollbarState::new(content_length)
.viewport_content_length(viewport).position(scroll)` are all `const fn … -> Self`
in `ratatui-widgets-0.3.0`; chain them as shown.

### 3. New imports — `render.rs`

Extend the existing `use ratatui::{…}` block:

- `layout::{Constraint, Layout, Rect}` → add `Margin`.
- `widgets::{Block, Borders, Paragraph}` → add `Scrollbar, ScrollbarOrientation,
  ScrollbarState`.

(`Line`, `Span`, `Style`, `Color` are already imported. `render_stateful_widget`
is a `Frame` method — no trait import needed.)

### 4. Feed the wrapped total to the event-loop clamp — `event_loop.rs`

Capture the returned total from the draw closure and clamp with it:

```rust
let mut total_wrapped = 0usize;
terminal.draw(|frame| {
    total_wrapped = render_dashboard(frame, frame.area(), &data, now_ms, &state, rates);
})?;
offset = clamp_scroll(offset, total_wrapped);
```

`transcript_lines` is then no longer used in `event_loop.rs` — change its import
`use super::transcript::{SPINNER_FRAMES, transcript_lines};` to
`use super::transcript::SPINNER_FRAMES;` (leaving the unused import would fail the
zero-warnings gate). `SPINNER_FRAMES` is still used.

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `wrap_line` on a single-span line of 25 chars at width 10 returns **3**
      rows; concatenating their span text reproduces the original 25 chars; the
      first row's span `fg` equals the input span's `fg` (style preserved).
- [ ] `wrap_line` on a line shorter than `width` returns exactly **1** row,
      unchanged; on `width == 0` returns exactly 1 row (the clone, no panic /
      divide).
- [ ] `wrap_line` on a **multi-span** line splits across the wrap column with
      each fragment keeping its own span style.
- [ ] No wrapped row exceeds `width` characters (no horizontal overflow).
- [ ] `wrap_lines` total drives the follow offset: for a record set whose
      wrapped count is `W` and pre-wrap count is `P` (`W > P`), `visible_offset(
      true, 0, W, viewport)` returns `W - viewport`, which is **strictly greater**
      than the pre-wrap `visible_offset(true, 0, P, viewport)` — the headline fix.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test`
      all pass.
- [ ] `git diff --name-only` lists only `mcp/src/dashboard/render.rs` and
      `mcp/src/dashboard/event_loop.rs` (plus this phase doc). No `Cargo.toml`,
      no `filter.rs`, no `transcript.rs`.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` block of `render.rs`
(`use super::*` already brings `Line`, `Span`, `Style`, `Color`, `wrap_line`,
`wrap_lines`, `visible_offset` into scope). Names describe behavior; exact count
and placement are yours. The **load-bearing** test is
`wrap_lines_total_drives_follow_offset` — it pins the README's core behavior.

- `wrap_line_splits_long_line_into_rows` — `Line::from(Span::styled("a".repeat(25),
  Style::new().fg(Color::Red)))` at width 10 → `rows.len() == 3`; the joined span
  text equals `"a".repeat(25)`; `rows[0].spans[0].style.fg == Some(Color::Red)`.
- `wrap_line_keeps_short_line_intact` — a 5-char line at width 10 → exactly 1 row.
- `wrap_line_zero_width_is_noop` — `wrap_line(&line, 0).len() == 1` (guard; no
  panic).
- `wrap_line_preserves_multispan_styles` — a line of `Span("aa", Red)` +
  `Span("bbbb", Blue)` at width 3 → 2 rows; row 0 has a red span then a blue
  span; row 1's span is blue (proves spans are split, not dropped or recolored).
- `wrap_lines_no_row_exceeds_width` — over a mixed slice (short line + a long
  line), assert every returned row's total char count is `<= width`.
- `wrap_lines_total_drives_follow_offset` — `lines = [Line::from("hdr"),
  Line::from("x".repeat(25))]`; `let total = wrap_lines(&lines, 10).len();`
  assert `total == 4`; with `viewport = 1`, assert `visible_offset(true, 0, total,
  1) == 3` **and** `visible_offset(true, 0, lines.len(), 1) == 1` — the wrapped
  total advances the follow offset to 3, where the pre-wrap count would have
  stuck it at 1.

(Reading a span's char count: `row.spans.iter().map(|s| s.content.chars().count())
.sum::<usize>()`. Reading a span's style: `row.spans[i].style.fg`.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and
M13 phase-01/02). Verification is the pure-function `wrap_line` / `wrap_lines` /
`visible_offset` assertions above plus the `cargo` gates. The scrollbar and the
wrap *rendering* are exercised by the live binary, not a unit test; the wrapped
**count** that drives both is fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — `Scrollbar`/`ScrollbarState`/
      `ScrollbarOrientation`/`Margin` are already in `ratatui = "0.30"` (stable,
      no feature flag). **No `Cargo.toml` edit of any kind** (in particular, do
      **not** enable `unstable-rendered-line-info`).
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- **Edit `mcp/Cargo.toml`** — do not enable `unstable-rendered-line-info` or any
  feature, and do not use `Paragraph::line_count` / `Paragraph::line_width`
  (they are unstable and feature-gated; see "Why we wrap ourselves" above). If you
  believe you need them, **stop and file a blocker** — but you do not; `wrap_lines`
  replaces them.
- Add `.wrap(Wrap { trim: … })` to the transcript `Paragraph`. The lines are
  pre-wrapped to `inner_width`; adding ratatui wrapping on top would re-wrap and
  desynchronize the count from the render.
- Touch `filter.rs`, `transcript.rs`, `panels.rs`, or `highlight.rs`. The wrap
  operates on whatever `transcript_lines` returns; the per-record body cap
  (`TRANSCRIPT_CONTENT_MAX_LINES`) is unchanged and orthogonal.
- Change the **filter-overlay** Paragraph (the `if filter_state.open` branch's
  `Paragraph::new(filter_lines)`) — leave it unwrapped; only compute
  `total_wrapped` there for the clamp, as specified.
- Change `clamp_scroll` or `visible_offset` — they are already correct; only the
  `total` you pass them changes.
- Add new scroll/paging key bindings or any interactivity. The dashboard stays a
  monitoring view (M8 Non-goals).
- Switch from char-count to unicode display-width wrapping (would need a new
  dependency) — char-count is the accepted simplification; note it in the helper
  comment as shown.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 16:54 (progress)

Implementing wrap helpers, updating render_dashboard to return wrapped line count, adding scrollbar, updating event_loop clamp, and writing unit tests.

### Update — 2026-06-10 16:56 (complete)

**Summary:** Added `wrap_line` / `wrap_lines` pure helpers in `render.rs` that hard-wrap styled `Line`s to a given character width, preserving span styles by splitting spans at the wrap column. Updated `render_dashboard` to return `usize` (the wrapped line total), so the event loop can clamp the manual scroll offset against the correct wrapped count. Added a `Scrollbar` on the right edge of the Activity panel. Updated `event_loop.rs` to capture the returned wrapped total and removed the now-unused `transcript_lines` import. Six unit tests added covering all acceptance criteria.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.6s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.7s

cargo test 2>&1 | tail -30
test result: ok. 725 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13 phase-01/02). Verification is the pure-function `wrap_line` / `wrap_lines` / `visible_offset` assertions plus the `cargo` gates.

**Files changed:**
- `mcp/src/dashboard/render.rs` — added `wrap_line`/`wrap_lines` helpers, updated `render_dashboard` to return wrapped line count, added scrollbar rendering, added 6 unit tests
- `mcp/src/dashboard/event_loop.rs` — capture wrapped total from render, clamp against it, removed unused `transcript_lines` import
- `docs/dev/milestones/M13-dashboard-polish/phase-03-wrapping.md` — status flip to review, Update Log entries
- `docs/dev/milestones/M13-dashboard-polish/README.md` — phase table row updated to review

**New tests:**
- `wrap_line_splits_long_line_into_rows` in `mcp/src/dashboard/render.rs`
- `wrap_line_keeps_short_line_intact` in `mcp/src/dashboard/render.rs`
- `wrap_line_zero_width_is_noop` in `mcp/src/dashboard/render.rs`
- `wrap_line_preserves_multispan_styles` in `mcp/src/dashboard/render.rs`
- `wrap_lines_no_row_exceeds_width` in `mcp/src/dashboard/render.rs`
- `wrap_lines_total_drives_follow_offset` in `mcp/src/dashboard/render.rs`

**Commits:**
- pending — one commit expected

**Notes for review:** none — implementation follows the spec exactly.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — `git show 2a8b73b --name-only` lists exactly
  `mcp/src/dashboard/render.rs`, `mcp/src/dashboard/event_loop.rs`, and the two
  M13 docs. No `Cargo.toml`, no `filter.rs`/`transcript.rs`/`highlight.rs`/
  `panels.rs`. Both pinned gotchas held: no `Paragraph::line_count`/`line_width`,
  no `.wrap(Wrap …)`, no `unstable-rendered-line-info` feature. The `wrap_line`
  helper, the render branch, and the event-loop clamp match the spec's worked
  shapes; the pre-wrapped lines render via `.scroll((scroll, 0))` with no ratatui
  `Wrap`.
- **Independent re-run:** `cargo fmt --all --check` clean, `cargo build` zero
  warnings, `cargo clippy --all-targets --all-features -- -D warnings` clean,
  `cargo test` **725 passed / 0 failed / 2 ignored** (executor) + **312 passed**
  (mcp). All 6 new tests pass and are mutation-resistant — the load-bearing
  `wrap_lines_total_drives_follow_offset` pins `visible_offset(true, 0, 4, 1) == 3`
  against the pre-wrap `… == 1`, so reverting the wrapped-count fix fails it;
  `wrap_line_preserves_multispan_styles` pins the red/blue span split across the
  wrap column, so dropping or recoloring spans fails it. Production grep for
  `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]`/`TODO`/`dbg!`/`println!` is clean.
- **Calibration:** none. Cosmetic-only quirk: the Update Log's "Commits: pending"
  is stale (the executor did commit, `2a8b73b`), and its `16:54`/`16:56` stamps
  differ from the real commit time — the recurring local-LLM self-stamping quirk;
  machine records are correct.
