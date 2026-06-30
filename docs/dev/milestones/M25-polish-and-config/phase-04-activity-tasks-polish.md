# Phase 04: Activity & Tasks panel polish

**Milestone:** M25 — Polish & Config Pass
**Status:** todo
**Depends on:** none
**Estimated diff:** ~80 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Two independent dashboard polish fixes, one per file:

- **Issue 4** — the Activity panel wraps long transcript lines on **word
  boundaries** instead of mid-word. A word that would fit on a row by itself is
  never split; it moves whole to the next row. A word longer than the wrap width
  has no fitting row, so it is still hard-split (the current behavior, now only the
  fallback). Lives in `wrap_line` (`mcp/src/dashboard/render.rs`).
- **Issue 5** — the Tasks panel title pan advances **twice as fast** per tick.
  Lives in `scrolled_title` (`mcp/src/dashboard/panels.rs`).

Both are display-only: no `SessionEvent`/telemetry schema change, no new
dependency, no change to any other panel or function.

## Architecture references

Read before starting:

- `docs/dev/milestones/M25-polish-and-config/README.md` — issues 4 & 5 and the
  locked decisions.
- `docs/architecture.md` § Status #25 — milestone summary.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### Issue 4 — `wrap_line` hard-wraps mid-word (`mcp/src/dashboard/render.rs:44-70`)

`wrap_line` walks each span char-by-char and forces a break the instant the
column reaches `width`, splitting whatever word straddles that column:

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
```

`wrap_line` is called **only** by `wrap_lines_hanging` (`render.rs:76-100`), which
passes `content_width = width − continuation_indent` and applies the hanging
indent to continuation rows. `wrap_lines_hanging` does **not** change in this
phase — it keeps calling `wrap_line` exactly as it does today; only `wrap_line`'s
internal break logic changes. (`grep -rn "wrap_line(" mcp/src` confirms
`render.rs` is the only caller.)

`ratatui::style::Style` derives `Copy + PartialEq + Eq` (relied on below to
coalesce equal-styled chars back into spans). `(char, Style)` is therefore `Copy`.

### Issue 5 — `scrolled_title` pan step (`mcp/src/dashboard/panels.rs:340-362`)

The pan speed is the `step` computation inside `scrolled_title`:

```rust
        Some(t) => {
            // Triangle wave over [0, overflow]: pan right, then back left.
            // 0.75 chars/tick (3 chars per 4 ticks).
            let step = t * 3 / 4;
            let period = overflow * 2;
            let phase = step % period;
            if phase <= overflow {
                phase
            } else {
                period - phase
            }
        }
```

## Spec

Numbered tasks in execution order.

1. **Word-boundary-wrap `wrap_line`** — in `mcp/src/dashboard/render.rs`, replace
   the body of `wrap_line` (and update its doc comment) with the word-aware
   version below, and add the `row_to_line` coalescing helper directly beneath it.
   The signature is unchanged.

   This shape is **pinned** — it is a strict superset of the current behavior
   (identical output for any space-free line, so every existing `wrap_line` /
   `wrap_lines_hanging` test passes unmodified) and only changes where breaks land
   when spaces are present. Implement it as written:

   ```rust
   /// Wrap one styled line to `width` columns on **word boundaries**, preserving
   /// span styles. A word — a maximal run of non-space chars — is never split
   /// across rows when it would fit on a row by itself; it moves whole to the next
   /// row instead. A word longer than `width` has no fitting row, so it is
   /// hard-split to fill each row (the prior mid-word behavior, now only the
   /// fallback). Spaces are placed as encountered, so no characters are dropped and
   /// concatenating all rows reproduces the input. `width == 0` or an empty line
   /// returns a single row unchanged. Char-count based (not unicode display width).
   pub(crate) fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
       if width == 0 {
           return vec![line.clone()];
       }
       let chars: Vec<(char, Style)> = line
           .spans
           .iter()
           .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
           .collect();
       if chars.is_empty() {
           return vec![line.clone()];
       }

       let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
       let mut cur: Vec<(char, Style)> = Vec::new();
       let mut col = 0usize;
       let mut i = 0usize;
       while i < chars.len() {
           if chars[i].0 == ' ' {
               // Space: place it; break only when the row is already full.
               if col == width {
                   rows.push(std::mem::take(&mut cur));
                   col = 0;
               }
               cur.push(chars[i]);
               col += 1;
               i += 1;
               continue;
           }
           // Measure the next word (a run of non-space chars).
           let start = i;
           while i < chars.len() && chars[i].0 != ' ' {
               i += 1;
           }
           let word = &chars[start..i];
           if word.len() <= width {
               // Word fits on a row: break before it if it won't fit on this one.
               if col + word.len() > width {
                   rows.push(std::mem::take(&mut cur));
                   col = 0;
               }
               cur.extend_from_slice(word);
               col += word.len();
           } else {
               // Word longer than any row: hard-split to fill each row.
               for &c in word {
                   if col == width {
                       rows.push(std::mem::take(&mut cur));
                       col = 0;
                   }
                   cur.push(c);
                   col += 1;
               }
           }
       }
       rows.push(cur);

       rows.into_iter().map(row_to_line).collect()
   }

   /// Coalesce a row of styled chars into a `Line`, merging adjacent equal-styled
   /// chars into a single span (so the span count matches the prior behavior).
   fn row_to_line(row: Vec<(char, Style)>) -> Line<'static> {
       let mut spans: Vec<Span<'static>> = Vec::new();
       let mut buf = String::new();
       let mut cur_style: Option<Style> = None;
       for (ch, style) in row {
           if cur_style != Some(style) {
               if let Some(s) = cur_style {
                   spans.push(Span::styled(std::mem::take(&mut buf), s));
               }
               cur_style = Some(style);
           }
           buf.push(ch);
       }
       if let Some(s) = cur_style {
           spans.push(Span::styled(buf, s));
       }
       Line::from(spans)
   }
   ```

2. **Double the Tasks pan speed** — in `scrolled_title`
   (`mcp/src/dashboard/panels.rs`), change the step computation from
   `t * 3 / 4` to `t * 3 / 2` and update the inline comment to match:

   ```rust
           Some(t) => {
               // Triangle wave over [0, overflow]: pan right, then back left.
               // 1.5 chars/tick (3 chars per 2 ticks).
               let step = t * 3 / 2;
   ```

   Nothing else in `scrolled_title` changes (`period`, `phase`, the triangle-wave
   branch, the `None` and `chars.len() <= max` early returns all stay).

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `wrap_line` on a line with spaces never splits a word that fits within
      `width`; such a word appears whole in exactly one row.
- [ ] `wrap_line` on a single word longer than `width` still hard-splits it, and
      **no** output row exceeds `width` chars (the existing
      `wrap_lines_hanging_no_row_exceeds_width` invariant holds).
- [ ] `wrap_line` preserves span styles across the new word breaks.
- [ ] `scrolled_title(FIXTURE, 10, Some(4))` returns the window starting at char 6
      (`"ghijklmnop"`), confirming the doubled step.

## Test plan

All `wrap_line` tests live in `mcp/src/dashboard/render.rs`; `scrolled_title`
tests in `mcp/src/dashboard/panels.rs`.

**Existing tests that pass unmodified** (do not change them — they are the
regression guarantee that the no-space path is byte-identical and over-long words
still hard-split): `wrap_line_splits_long_line_into_rows`,
`wrap_line_keeps_short_line_intact`, `wrap_line_zero_width_is_noop`,
`wrap_line_preserves_multispan_styles`, `wrap_lines_hanging_no_row_exceeds_width`,
`wrap_lines_hanging_total_drives_follow_offset`,
`wrap_lines_hanging_first_row_has_no_indent`,
`wrap_lines_hanging_continuations_are_indented`. Also `scrolled_title_ping_pongs`
(range-based: max start still reaches `overflow` — at t=40, `step = 60`,
`phase = 60 % 40 = 20 = overflow`) and `scrolled_title_char_indexed_multibyte`
(asserts length only).

Add (in `render.rs`):

- `wrap_line_breaks_on_word_boundary` — `wrap_line(&Line::from("hello world foo"), 8)`;
  format each row with `format!("{r}")`; assert each of `"hello"`, `"world"`,
  `"foo"` is a substring of some row, and that the 5-char word `"world"` occupies
  its own row (a row whose trimmed text equals `"world"`) — i.e. it was moved
  whole, not split after `"hello "` (`6 + 5 > 8`).
- `wrap_line_hard_splits_word_longer_than_width` — `wrap_line(&Line::from("supercalifragi"), 8)`
  (14 chars, no space) returns 2 rows and concatenating their text reproduces
  `"supercalifragi"` (the over-long fallback still fires).
- `wrap_line_word_boundary_preserves_styles` — build
  `Line::from(vec![Span::styled("hello", red), Span::raw(" "), Span::styled("world", blue)])`,
  wrap at width 8; assert the row containing `"world"` carries the blue style on
  that word (the break does not lose the span style).

Update (in `panels.rs`):

- `scrolled_title_pans_overflowing_title` — the `tick = 4` expectation changes
  from `"defghijklm"` (old start 3) to `"ghijklmnop"` (new `step = 4*3/2 = 6`,
  start 6); update the accompanying comment. The `tick = 0` case (`"abcdefghij"`)
  is unchanged.
- `tasks_lines_non_active_tasks_do_not_pan` — its assertions still hold (active
  task still pans at tick 4: start 0 vs start 6), but the explanatory comment
  (`step = 4*3/4 = 3`) is now stale; update it to the doubled math
  (`step = 4*3/2 = 6`, window `chars[6..26]`).

## End-to-end verification

The runtime artifact is the live dashboard TUI, which is not headlessly
assertable; `wrap_line` and `scrolled_title` are **pure functions** fully covered
by the hermetic unit tests above. Per the phase-doc template's guidance for a
phase that ships no separately-loadable artifact, in the completion Update Log
paste the rendered rows of one representative `wrap_line` call (a line with spaces
that wraps, showing each word kept whole) and one `scrolled_title` call at
`tick = 4` (showing the doubled `"ghijklmnop"` window) — captured by formatting
the returned values in a scratch test, not a committed `println!`/`dbg!`. Leave no
`println!`/`dbg!` in the committed code.

## Authorizations

None. No new dependency, no `Cargo.toml` edit, no `docs/architecture.md` edit.

## Out of scope

- **`wrap_lines_hanging`** — unchanged; it keeps calling `wrap_line` as it does
  today. Only `wrap_line`'s internal logic changes.
- **The hanging-indent width math, the 4-char gutter, the scrollbar, follow/offset
  logic** in `render.rs` — untouched.
- **Unicode display-width measurement** — wrapping stays char-count based, as
  documented; wide glyphs may still clip by a cell. Do not pull in a width crate.
- **Trimming trailing spaces from wrapped rows** — not required; placing spaces as
  encountered is intentional (keeps the concatenation-equals-input property).
- **Any other `scrolled_title` behavior** — only the `step` multiplier and its
  comment change; the triangle-wave ping-pong, the `None`/fits early returns, and
  `truncate_title` are untouched.
- **Issues 1–3** (phase-03, done) and **the dependency bumps** (phases 05–09).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
