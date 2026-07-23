# Bug 1 on phase-07f: `header_band_height` inserted inside `render_dashboard`'s doc comment (doc misattribution)

**Severity:** minor
**Status:** open
**Filed:** 2026-07-22

## What's wrong

The new `header_band_height` helper (with its `///` doc) was placed **between
`render_dashboard`'s doc comment and its `fn`**, with no blank line (`render.rs:159–172`):

```rust
/// Render the dashboard into a three-panel header band (Session · Budget ·
/// Compactions) above a body (Activity wide-left · Files right), or a
/// single error pane when `data.error` is set.
/// Transcript is newest-first when `follow` is true (tail-pinned).
/// Rows for the header band: the tallest of the three header panels' content plus
/// 2 border rows. The panels share one horizontal band, so it fits the tallest;
/// a shorter panel shows a trailing blank equal to its shortfall.
fn header_band_height(session_len: usize, budget_len: usize, context_len: usize) -> u16 { … }

pub(crate) fn render_dashboard( … )
```

Rust attaches a doc comment to the **next item**. Because there is no blank line between
`render_dashboard`'s doc (`/// Render the dashboard … /// Transcript is newest-first …`)
and the helper's doc, the two blocks merge into **one** doc comment that attaches to
`header_band_height`. Result:

- `header_band_height` is documented with a garbled mix — "Render the dashboard into a
  three-panel header band … Transcript is newest-first … Rows for the header band: …" —
  most of which describes `render_dashboard`, not the helper.
- `render_dashboard` (the `pub(crate)` dashboard entry point) is now left **with no doc
  comment at all**.

Functionally correct (all gates green), but a real documentation regression — the wrong
description is attached to the helper and the main entry point lost its doc.

## What should happen

`render_dashboard` keeps its own doc comment; `header_band_height` has its own, separate
doc comment. The helper must not sit inside another item's doc block.

## How to fix

Move the `header_band_height` fn **together with its own 3-line `///` doc** out of the gap
— either:
- **(a)** below the `render_dashboard` function, or
- **(b)** above `render_dashboard`'s doc comment, with a **blank line** separating the
  helper's `fn` from `render_dashboard`'s `/// Render the dashboard …` block.

Restore `render_dashboard`'s original doc comment directly above `pub(crate) fn
render_dashboard` (the four lines: "Render the dashboard into a three-panel header band …",
"Compactions) above a body …", "single error pane when `data.error` is set.", "Transcript
is newest-first when `follow` is true (tail-pinned)."). **No functional change** — only the
placement of the helper + doc comments. Do not touch the reorder logic, the
`header_band_height` body, or its test.

## Verification

- [ ] `render_dashboard` has its doc comment directly above it (the four "Render the
      dashboard …" lines); `header_band_height` has its own separate doc comment.
- [ ] `header_band_height`'s doc no longer contains "Render the dashboard" or "Transcript
      is newest-first".
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass
      (incl. `header_band_height_fits_tallest_plus_borders`).
