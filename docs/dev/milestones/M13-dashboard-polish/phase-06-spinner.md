# Phase 06: Session — full-width spinner on its own bottom line

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none for code (independent of phases 01–05; touches the Session
panel layout, the spinner, and the header height — none of which 01–05 changed).
Builds on the phase-05 "optional-line pushed in `render.rs`" pattern, copied here.
**Estimated diff:** ~140 lines (new `spinner_line` helper ~18, `session_lines`
spinner removal + param drop ~8, render wiring + header height ~12, event_loop +
transcript const removal ~16, tests ~85)
**Tags:** language=rust, kind=feature, size=m

## Goal

Turn the cramped, fixed-window liveness spinner into a **full-width spinner on its
own bottom line** of the Session panel (user items #10, R5), with the
`turn N, stage X` status text on the line directly above it (left-aligned with the
rest of the panel, as today).

Two concrete changes, decided with the user (2026-06-10):

1. **The spinner becomes a full-width line** that spans the Session panel: a dog
   that trots back and forth across the **entire** panel width, instead of the
   current ~12-character fixed-window animation. It is the **bottom** line of the
   panel; the `turn N, stage X` line sits directly above it (unchanged).
2. **The header band grows by one row** (`Length(9)` → `Length(10)`) to give the
   Session / Budget / Reclaim panels breathing room for the dedicated spinner line.
   The body (Activity + the Tasks/Files column) is `Min(0)`, so it **automatically**
   yields that one row — no body-layout edit is needed.

Pure presentation — no feed, config, or executor change. The spinner is liveness
chrome; `turn`/`stage` already exist on `StatusSummary`. **No `StatusSummary` field
is added in this phase** (unlike phase-05).

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  **display-only** constraint and the phase table. This phase touches **only**
  `mcp/src/dashboard/render.rs`, `mcp/src/dashboard/panels.rs`,
  `mcp/src/dashboard/event_loop.rs`, and `mcp/src/dashboard/transcript.rs` (the last
  two only to retire the now-unused fixed-window `SPINNER_FRAMES`). It adds **no**
  `SessionEvent` variant, no config, and no `StatusSummary` field.
- `docs/dev/milestones/M13-dashboard-polish/phase-05-timing.md` § "Budget is
  composed in `render.rs`, with an optional line appended" — the
  `dollars_saved_line` / `last_update_line` **optional-line-pushed-in-render**
  pattern you copy here for `spinner_line` (with one extra `width` argument).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The spinner lives inside `session_lines` as a fixed-window frame

In `mcp/src/dashboard/panels.rs`, `session_lines` (lines 41–91) takes a
`spinner: Option<usize>` **frame index** and, when `Some`, pushes a bare glyph line
from `SPINNER_FRAMES`:

```rust
pub(crate) fn session_lines(
    summary: &StatusSummary,
    now_ms: u64,
    spinner: Option<usize>,
) -> Vec<Line<'static>> {
    // ... phase / session / model / state / duration lines ...

    let stage = summary.latest_stage.as_deref().unwrap_or("<none>");
    lines.push(Line::from(format!(
        "turn {}, stage {stage}",
        summary.latest_turn
    )));

    if let Some(frame) = spinner {
        let glyph = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
        lines.push(Line::from(glyph.to_string()));
    }

    lines
}
```

`SPINNER_FRAMES` (a fixed set of ~12-char walk frames) is defined in
`mcp/src/dashboard/transcript.rs:14–27` and imported by `panels.rs:7` and
`event_loop.rs:6`.

### The spinner frame index is computed in `event_loop.rs`

`event_loop.rs` (lines 26–46) increments a `spinner_tick` each poll and hands
`session_lines` a frame **index** modulo the frame count:

```rust
spinner_tick = spinner_tick.wrapping_add(1);
// ...
let spinner_active = data.summary.ended.is_none() && data.error.is_none();
let spinner = if spinner_active {
    Some(spinner_tick % SPINNER_FRAMES.len())
} else {
    None
};
let state = ViewState { offset, follow, spinner, filter: filter_state.clone() };
```

### The header band is `Length(9)`; the Session panel is rendered in `render.rs`

In `mcp/src/dashboard/render.rs`, the outer split fixes the header at 9 rows
(line 128–129) and the Session panel is built by calling `session_lines` with
`state.spinner` (lines 137–150):

```rust
let [header, body] =
    Layout::vertical([Constraint::Length(9), Constraint::Min(0)]).areas::<2>(area);

let [session_area, budget_area, compactions_area] = Layout::horizontal([
    Constraint::Fill(1),
    Constraint::Min(56),
    Constraint::Percentage(28),
])
.areas::<3>(header);

frame.render_widget(
    panel(" Session ", session_lines(&data.summary, now_ms, state.spinner)),
    session_area,
);
```

Note `session_area` (the Session panel's `Rect`) is in scope at the render call, so
its width is available — this is what lets `spinner_line` be full-width.

### The blessed pattern to copy: optional line pushed in `render.rs`

Immediately below, the Budget panel composes optional lines by **pushing** helper
results onto the vec in `render.rs` (lines 151–159), rather than computing them
inside the panel builder:

```rust
let mut budget = Vec::new();
if let Some(line) = last_update_line(&data.summary, now_ms) {
    budget.push(line);
}
budget.extend(budget_lines(&data.summary));
if let Some(line) = dollars_saved_line(&data.summary, rates) {
    budget.push(line);
}
frame.render_widget(panel(" Budget ", budget), budget_area);
```

`spinner_line` is the same shape, plus a `width` argument (the spinner is the only
panel line that depends on the panel's width).

## The chosen change shape (read this — it avoids the known stall)

The spinner must become **full-width**, which needs the panel width. The panel
width lives in `render.rs` (`session_area`), **not** in `session_lines`. So the
spinner moves **out** of `session_lines` into a new pure
`spinner_line(spinner, width) -> Option<Line>` that `render.rs` pushes onto the
Session vec — exactly the `dollars_saved_line` / `last_update_line` precedent, with
one extra `width` arg.

Moving the spinner out means **`session_lines` no longer needs the `spinner`
parameter**. Dropping that parameter is a signature change with **one production
call site and nine test call sites, all in `panels.rs`/`render.rs`** — the kind of
multi-site mechanical edit that has stalled this executor before. It is bounded and
single-module, so it is safe **if you do every site**. The complete, grep-verified
list is enumerated in Spec step 3; do all ten, then `cargo build` once.

Do **not** instead keep a now-unused `spinner` parameter on `session_lines` (dead
code; clippy will reject it) or thread `width` *into* `session_lines` (that is the
same ten-site churn plus it drags width through a function that needs nothing else
from it). The clean shape is: spinner out, parameter gone, new width-aware helper
pushed in `render.rs`.

## Spec

All changes are in `render.rs`, `panels.rs`, `event_loop.rs`, and `transcript.rs`.
No other files. No `StatusSummary` change, no `Cargo.toml`.

### 1. Add the full-width spinner helper — `panels.rs`

Add this pure helper near `session_lines` (and a module-level `const`). It renders a
dog at a horizontal offset that is a **triangle wave** across the panel width, so the
dog trots right to the edge, then back, indefinitely, and **never overflows the
panel**:

```rust
/// Display cells the spinner sprite occupies (one wide dog glyph). The dog's
/// horizontal offset is bounded so `offset + SPRITE_CELLS <= width`, keeping the
/// rendered line inside the panel.
const SPRITE_CELLS: usize = 2;

/// Full-width liveness spinner: a dog that trots back and forth across the Session
/// panel. `spinner` is `Some(tick)` — a monotonic counter from the event loop —
/// while the session runs, and `None` once it ends (→ `None`, no spinner line, same
/// as today's ended behavior). `width` is the Session panel's inner width. The dog's
/// offset is a triangle wave over `[0, width − SPRITE_CELLS]`, so the dog walks the
/// full width and the line never exceeds it.
///
/// Char-count vs display-width caveat: the dog glyph is one code point but two
/// display cells; `SPRITE_CELLS` budgets for that. Wide-glyph rounding may leave the
/// line a cell short of the border — acceptable, matching `wrap_line`'s existing
/// char-count approach.
pub(crate) fn spinner_line(spinner: Option<usize>, width: usize) -> Option<Line<'static>> {
    let tick = spinner?;
    let span = width.saturating_sub(SPRITE_CELLS);
    if span == 0 {
        return Some(Line::from("🐕"));
    }
    let period = span * 2;
    let phase = tick % period;
    let offset = if phase <= span { phase } else { period - phase };
    Some(Line::from(format!("{}🐕", " ".repeat(offset))))
}
```

You may keep the `🐕` glyph alone (simplest) or pair it with one trailing motion
glyph — but if you add a second glyph, **raise `SPRITE_CELLS` to match its display
width** so the overflow bound still holds. The pinned behavior is: bounded offset,
dog present, `None` when ended (see Acceptance + Test plan). The exact emoji art is
yours.

### 2. Drop the spinner from `session_lines` — `panels.rs`

Change the signature to remove the `spinner` parameter:

```rust
pub(crate) fn session_lines(summary: &StatusSummary, now_ms: u64) -> Vec<Line<'static>> {
```

Delete the trailing spinner block (the `if let Some(frame) = spinner { … }` push,
panels.rs:85–88). **Keep everything else, including the `turn {}, stage {stage}`
line** — that line is unchanged and remains the last line `session_lines` emits, so
the spinner pushed after it in `render.rs` sits directly below it.

Remove the now-unused `use crate::dashboard::transcript::SPINNER_FRAMES;` import
(panels.rs:7).

### 3. Update every `session_lines` call site — `panels.rs` + `render.rs`

Dropping the parameter breaks ten call sites. **Update all of them** (grep-verified
`grep -rn "session_lines(" mcp/src`):

Production:
- `render.rs:147` — handled in step 4 (the whole Session composition changes).

Tests in `panels.rs` — drop the third argument (`None` / `Some(0)` / `Some(i)`):
- `panels.rs:347` `session_lines(&summary, 0, None)` → `session_lines(&summary, 0)`
- `panels.rs:361` `session_lines(&summary, 0, None)` → `session_lines(&summary, 0)`
- `panels.rs:373` `session_lines(&summary, 4000, None)` → `session_lines(&summary, 4000)`
- `panels.rs:386` `session_lines(&summary, 4000, None)` → `session_lines(&summary, 4000)`
- `panels.rs:394` `session_lines(&summary, 9999, None)` → `session_lines(&summary, 9999)`
- `panels.rs:406` `session_lines(&summary, 4000, None)` → `session_lines(&summary, 4000)`

The three **spinner** tests (`panels.rs:415` `session_lines_shows_spinner_when_active`,
`panels.rs:423` `session_lines_spinner_cycles_frames`, `panels.rs:436`
`session_lines_omits_spinner_when_none`) tested the old in-`session_lines` spinner.
**Delete them** and replace with `spinner_line` tests (Test plan below) — the spinner
no longer lives in `session_lines`, so asserting on its output for spinner behavior is
meaningless. After this step, no `session_lines` test references `SPINNER_FRAMES` or a
third argument.

Then `cargo build` once — all ten sites are now consistent.

### 4. Compose the full-width spinner into the Session panel — `render.rs`

Replace the single `session_lines` render call (render.rs:144–150) with the
push-the-optional-line shape, using `session_area`'s inner width:

```rust
let mut session = session_lines(&data.summary, now_ms);
let session_inner_width = session_area.width.saturating_sub(2) as usize;
if let Some(line) = spinner_line(state.spinner, session_inner_width) {
    session.push(line);
}
frame.render_widget(panel(" Session ", session), session_area);
```

Add `spinner_line` to the `use super::panels::{…}` import block (render.rs:10–13).

### 5. Grow the header band by one row — `render.rs`

Change the outer vertical split (render.rs:128–129) from `Length(9)` to
`Length(10)`:

```rust
let [header, body] =
    Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas::<2>(area);
```

The body is `Constraint::Min(0)`, so it shrinks by exactly one row automatically;
**do not** touch the body's horizontal/vertical splits (Activity, Tasks, Files). Add
or adjust the nearby comment to note the header is 10 rows to fit the spinner line.

### 6. Pass a raw tick, not a frame index — `event_loop.rs`

`spinner_line` does its own modulo against `width`, so the event loop should hand it
the raw monotonic counter, not `tick % SPINNER_FRAMES.len()`. Change the spinner
computation (event_loop.rs:42–46):

```rust
let spinner = if spinner_active {
    Some(spinner_tick)
} else {
    None
};
```

Remove the now-unused `use super::transcript::SPINNER_FRAMES;` import
(event_loop.rs:6). `spinner_tick` keeps incrementing each loop (event_loop.rs:27),
unchanged. `ViewState.spinner` stays `Option<usize>`; only its meaning changes
(tick, not frame index) — no struct change.

### 7. Retire the fixed-window frames — `transcript.rs`

Delete the `SPINNER_FRAMES` constant (transcript.rs:14–27). After steps 2 and 6 it
has no remaining references (`grep -rn "SPINNER_FRAMES" mcp/src` returns nothing).
Leave `TRANSCRIPT_PREVIEW_MAX` and everything else in `transcript.rs` untouched.

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `spinner_line(None, w)` is `None` for any `w` (ended / inactive → no spinner
      line).
- [ ] `spinner_line(Some(0), w)` for `w >= SPRITE_CELLS` is `Some` with the dog at
      offset 0 (no leading spaces) and contains `🐕`.
- [ ] For **any** `tick` and any `width`, the rendered spinner line never overflows:
      the leading-space count is `<= width − SPRITE_CELLS` (and `0` when
      `width < SPRITE_CELLS`). A large tick (e.g. `999_999`) does not overflow — the
      bound is a triangle wave, not `tick` unbounded.
- [ ] The dog **moves** between consecutive ticks near the start (e.g. offset at
      `tick = 0` differs from offset at `tick = 1` for a non-tiny width) and
      **reverses** at the right edge (offset stops growing past `width − SPRITE_CELLS`
      and comes back).
- [ ] `session_lines` no longer takes a `spinner` parameter and no longer emits any
      `🐕`/`SPINNER_FRAMES` line; it still emits the `turn N, stage X` line
      unchanged.
- [ ] The Session panel in `render.rs` pushes the `spinner_line` result (when `Some`)
      as the **last** line, below the `turn N, stage X` line; the header band is
      `Length(10)`.
- [ ] `grep -rn "SPINNER_FRAMES" mcp/src` returns nothing.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test` all
      pass.
- [ ] `git diff --name-only` lists only `mcp/src/dashboard/render.rs`,
      `mcp/src/dashboard/panels.rs`, `mcp/src/dashboard/event_loop.rs`,
      `mcp/src/dashboard/transcript.rs` (plus this phase doc and the README row). No
      `Cargo.toml`, no `status.rs`, no `filter.rs`, no `SessionEvent`/config edit.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` block in `panels.rs`
(`use super::*` is in scope). Names describe behavior; exact count and placement are
yours. The **load-bearing** test is `spinner_line_never_exceeds_width` (pins the
overflow bound — a mutation to `tick % width` or an unbounded offset fails it) paired
with `spinner_line_none_when_ended` (pins the spinner genuinely disappears when the
session ends).

New `spinner_line` tests (replacing the three deleted `session_lines` spinner tests):

- `spinner_line_none_when_ended` — `spinner_line(None, 40) == None`.
- `spinner_line_starts_flush_left` — `spinner_line(Some(0), 40)` is `Some`; the
  rendered text has no leading space and contains `🐕`.
- `spinner_line_never_exceeds_width` — for `width = 20` and every `tick` in, say,
  `0..200` (and a couple of huge ticks like `999_999`), assert the leading-space
  count is `<= width - SPRITE_CELLS` (= 18). Mutation-resistant: an unbounded or
  `tick % width` offset overflows and fails.
- `spinner_line_bounces_at_right_edge` — for a known small width (e.g. `width = 5` →
  `span = 3`, `period = 6`), assert the offset sequence over `tick = 0..7` is
  `0,1,2,3,2,1,0,1` (i.e. it rises to `span` then falls). Reading the offset = the
  leading-space count of the rendered line.
- `spinner_line_tiny_width_does_not_panic` — `spinner_line(Some(7), 1)` is `Some` and
  contains `🐕` (no panic, no overflow when `width < SPRITE_CELLS`).

Revise the **surviving** `session_lines` tests only by dropping the third argument
(step 3); their assertions (phase/session/state/duration/turn-stage) are unchanged.
Confirm `session_lines_shows_turn_stage` (panels.rs:367) still passes — the
`turn N, stage X` line is untouched.

(Reading a line's text: `format!("{l}")`. Counting leading spaces:
`format!("{l}").chars().take_while(|c| *c == ' ').count()`.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13
phases 01–05). Verification is the `spinner_line` pure-function assertions plus the
`session_lines` parameter-drop assertions and the `cargo` gates. The header-height
and panel-composition changes are exercised by the live binary; the line-builder
function that drives the spinner is fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — only `render.rs`, `panels.rs`, `event_loop.rs`,
      and `transcript.rs` change. **No `Cargo.toml` edit.**
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- Add a new `SessionEvent` variant, a config field, or any `StatusSummary` field. If
  you think you need one, **stop and file a blocker**: you have left M13's
  display-only scope. (Everything the spinner needs — `latest_turn`,
  `latest_stage` — already exists; and the spinner itself derives only from the
  event-loop tick and the panel width.)
- Thread `width` into `session_lines`, or keep an unused `spinner` parameter on it.
  Use the `spinner_line` helper + `render.rs` push instead — this is deliberate, to
  avoid the multi-site signature churn.
- Touch `mcp/src/status.rs`'s `format_status` (the `rexymcp status` **CLI text**
  path) or `summarize`. This phase is the **dashboard** panel only; the CLI text
  renderer has no spinner and stays as-is.
- Touch the body layout (Activity / Tasks / Files splits), the wrap/scroll machinery,
  `filter.rs`, or `transcript.rs` beyond removing the `SPINNER_FRAMES` const. The
  body shrinks by one row for free because it is `Constraint::Min(0)`.
- Change the `turn N, stage X` line's text or position, or any other Session-panel
  line. Only the spinner moves (out, to full-width) and the header gains a row.
- Restyle or recolor anything. The spinner is plain (no `Style`); keep it plain.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 23:38 (started)

**Executor:** Claude (headless)

Implementing full-width spinner on its own bottom line of the Session panel.

### Update — 2026-06-10 23:40 (complete)

**Summary:** Moved the spinner out of `session_lines` into a new `spinner_line(spinner, width) -> Option<Line>` helper in `panels.rs`, composed in `render.rs` using the panel's inner width. The dog (`🐕`) now trots across the full panel width via a triangle-wave offset. Dropped the `spinner` parameter from `session_lines` (10 call sites updated), retired `SPINNER_FRAMES` from `transcript.rs`, switched `event_loop.rs` to pass a raw monotonic tick instead of a frame index, and grew the header band from `Length(9)` to `Length(10)`.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.62s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.70s

cargo test 2>&1 | tail -30
test result: ok. 725 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.06s
```

**End-to-end verification:**

N/A — phase ships no runtime-loadable artifact (TUI rendering has no headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13 phases 01–05). Verification is the `spinner_line` pure-function assertions plus the `session_lines` parameter-drop assertions and the `cargo` gates.

**Files changed:**
- `mcp/src/dashboard/panels.rs` — removed `spinner` param from `session_lines`, added `SPRITE_CELLS` const and `spinner_line` helper, replaced 3 old spinner tests with 5 new `spinner_line` tests, updated 6 surviving `session_lines` test call sites
- `mcp/src/dashboard/render.rs` — composed `spinner_line` into Session panel vec, grew header to `Length(10)`, added `spinner_line` import
- `mcp/src/dashboard/event_loop.rs` — pass raw tick instead of `tick % SPINNER_FRAMES.len()`, removed `SPINNER_FRAMES` import
- `mcp/src/dashboard/transcript.rs` — deleted `SPINNER_FRAMES` constant
- `docs/dev/milestones/M13-dashboard-polish/phase-06-spinner.md` — status flip to `review`, Update Log entries
- `docs/dev/milestones/M13-dashboard-polish/README.md` — phase table row updated to `review`

**New tests:**
- `spinner_line_none_when_ended` in `panels.rs`
- `spinner_line_starts_flush_left` in `panels.rs`
- `spinner_line_never_exceeds_width` in `panels.rs`
- `spinner_line_bounces_at_right_edge` in `panels.rs`
- `spinner_line_tiny_width_does_not_panic` in `panels.rs`

**Deleted tests:**
- `session_lines_shows_spinner_when_active` in `panels.rs`
- `session_lines_spinner_cycles_frames` in `panels.rs`
- `session_lines_omits_spinner_when_none` in `panels.rs`

**Commits:**
- `002f148` — `feat: full-width spinner on Session panel bottom line`

**Notes for review:**
- `grep -rn "SPINNER_FRAMES" mcp/src` returned exit code 1 (zero matches) — confirmed fully retired.
- `grep -rn "spinner_line" mcp/src` confirms the helper is defined in `panels.rs` and imported/used in `render.rs`.
- One-line verification summary: `cargo fmt --all --check` clean, `cargo build` zero warnings, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo test` 725 passed 0 failed, `grep SPINNER_FRAMES` zero matches.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (Update Log self-stamps "Claude (headless)" — the
  recurring local-LLM identity self-stamping quirk; cosmetic, machine records correct).
- **Scope deviations:** none in the source. Diff is exactly the four authorized files
  (`panels.rs`, `render.rs`, `event_loop.rs`, `transcript.rs`); no `Cargo.toml`,
  `status.rs`, `filter.rs`, `SessionEvent`, or config touched. The spinner moved out of
  `session_lines` into the width-aware `spinner_line` helper pushed in `render.rs`
  (phase-05 precedent), all ten `session_lines` call sites updated, `SPINNER_FRAMES`
  retired, header grown `Length(9)`→`Length(10)` — spec-exact.
- **Independent re-run:** all four gates green (`cargo fmt --all --check` clean,
  `cargo build` zero warnings, `cargo clippy` clean, `cargo test` 725 passed / 0 failed
  / 2 ignored). The 5 `spinner_line` tests pass; the load-bearing
  `spinner_line_never_exceeds_width` (tick 0..200 + 999_999 ≤ width−SPRITE_CELLS) and
  `spinner_line_bounces_at_right_edge` (pins the `0,1,2,3,2,1,0,1` triangle wave) are
  mutation-resistant against an unbounded / `tick % width` / sawtooth offset.
- **Calibration:** the executor committed (good — full bookkeeping first-try) but the
  commit swept in the at-dispatch dirty tree: modified `docs/dev/NEXT.md` (architect's
  activation edits) + the untracked phase doc. **2nd-plus occurrence of the
  dirty-tree-at-dispatch pattern** (prior: M9/phase-01, M11/phase-03) — commit ambient
  activation edits *before* dispatch next time. Cosmetic, no code impact.
