# Phase 02: Restore the full-width dog-chasing-brain spinner

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** todo
**Depends on:** phase-01
**Estimated diff:** ~70 lines (one function + its tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

The liveness spinner used to be a dog chasing its own brain across the Session
panel, ending in a `💨` overtake burst. A refactor flattened it to a single dog
doing a plain triangle-wave walk. Restore the chase animation — but **parametric
on panel width** so the chase distance fills the whole Session panel inner width,
not a fixed set of frames.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs:117–143` — the current `spinner_line` and its
  `SPRITE_CELLS` constant + char-count-vs-display-width caveat.
- `mcp/src/dashboard/render.rs:148–151` — the single call site:
  `spinner_line(state.spinner, session_inner_width)`, pushed onto the Session
  panel after the other lines.
- `mcp/src/dashboard/event_loop.rs:19,26,40–45` — `spinner_tick` increments once
  per ~500 ms loop; the loop passes `Some(tick)` while running, `None` once the
  session ends.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

```rust
const SPRITE_CELLS: usize = 2;

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

A single dog, triangle-wave offset, no brain, no chase. The **signature stays
exactly this** — `spinner_line(spinner: Option<usize>, width: usize) ->
Option<Line<'static>>`. Only the body changes.

The previous chase animation (from commit `e0b3663`, a fixed 9-frame constant)
read like this — a dog gaining on its brain at the right, overtaking with a
dust puff, then ending alone:

```
"🐕       🧠"   " 🐕     🧠"   "  🐕   🧠   "   "   🐕 🧠  "
"    🐕🧠 "      "  🧠🐕💨"      " 🧠🐕"         "🧠🐕"        "🐕"
```

The restore generalizes that to any width.

## Spec

### 1. Rewrite `spinner_line` as a width-parametric chase

Replace the `spinner_line` body (keep the signature). Reference implementation —
transcribe this; the per-cell positions are tuned so the rendered line never
exceeds `width` display cells:

```rust
const DOG: char = '🐕';
const BRAIN: char = '🧠';
const DASH: char = '💨';

/// Display cells each emoji sprite occupies (one code point, two terminal cells).
const SPRITE_CELLS: usize = 2;

/// Liveness spinner: a dog chasing its own brain across the Session panel. While
/// the session runs, `spinner` is `Some(tick)` (a monotonic per-loop counter);
/// once it ends, `None` (→ no spinner line). `width` is the panel inner width.
///
/// One cycle: the dog walks left→right (`track + 1` steps) closing on the brain
/// pinned at the right edge, catches it, then one overtake-burst frame
/// (`🧠🐕💨`) before resetting. The chase distance scales with `width`.
///
/// Char-count vs display-width caveat (unchanged from the prior impl): each emoji
/// is one `char` but two display cells; positions are computed in display cells so
/// the rendered line is bounded by `width` cells, while its `chars().count()` is
/// smaller. A wide-glyph terminal rounding may leave the line a cell short of the
/// border — acceptable.
pub(crate) fn spinner_line(spinner: Option<usize>, width: usize) -> Option<Line<'static>> {
    let tick = spinner?;
    // Reserve SPRITE_CELLS for the dog and SPRITE_CELLS for the brain so neither
    // sprite runs past `width`. `track` is the range the dog's left edge sweeps.
    let track = width.saturating_sub(SPRITE_CELLS * 2);
    if track == 0 {
        return Some(Line::from(format!("{DOG}{BRAIN}")));
    }
    let period = track + 2; // track+1 chase steps + 1 overtake-burst frame
    let phase = tick % period;
    if phase <= track {
        // Chase: dog at `phase`; brain pinned so its right edge is the panel edge.
        let dog_off = phase;
        let brain_off = track + SPRITE_CELLS;
        let gap = brain_off.saturating_sub(dog_off + SPRITE_CELLS);
        Some(Line::from(format!(
            "{}{DOG}{}{BRAIN}",
            " ".repeat(dog_off),
            " ".repeat(gap),
        )))
    } else {
        // Overtake burst: brain, dog, dust — pinned to the right edge.
        let lead = width.saturating_sub(SPRITE_CELLS * 3);
        Some(Line::from(format!("{}{BRAIN}{DOG}{DASH}", " ".repeat(lead))))
    }
}
```

**Width-bound check (why it fits):** in the chase frame the rightmost content is
the brain whose right edge sits at `brain_off + SPRITE_CELLS = track + 2 *
SPRITE_CELLS = width`. In the overtake frame the content is `lead + 3 *
SPRITE_CELLS = width`. Both land exactly on the panel edge, never past it.

### 2. Replace the spinner tests

The old tests pin the single-dog triangle wave (`spinner_line_never_exceeds_width`,
`spinner_line_bounces_at_right_edge`, the frame-content assertions). Replace them
with tests for the chase. Use the **char-count** convention the existing tests
use (`format!("{}", line).chars().count()`), consistent with the module caveat —
do not try to measure true display width.

Required tests:

- `spinner_line_none_when_ended` — `spinner_line(None, 40)` is `None`.
- `spinner_line_contains_dog_and_brain_during_chase` — for a chase-phase tick,
  the rendered string contains both `🐕` and `🧠`.
- `spinner_line_emits_overtake_burst_once_per_cycle` — over one full `period`
  of ticks at a fixed width, exactly one frame contains `💨` (the burst), and
  that frame also contains `🧠🐕` adjacent. Mutation-resistant: an impl that
  never emits the burst, or emits it every frame, fails this.
- `spinner_line_scales_with_width` — the dog's leftmost-space count (or the
  chase `period`) differs between `width = 20` and `width = 60`, proving the
  animation is width-parametric, not fixed-frame. (E.g. assert the number of
  distinct dog offsets over a cycle is larger at width 60 than at width 20.)
- `spinner_line_never_exceeds_width` — for several widths (e.g. 10, 20, 40, 80)
  and several ticks spanning a full cycle, `chars().count() <= width`. (Char
  count ≤ width is the looser bound the module already commits to.)
- `spinner_line_degenerate_narrow_width` — at `width <= SPRITE_CELLS * 2`,
  `track == 0`, so the line is `"🐕🧠"` and is `Some`.

## Acceptance criteria

- [ ] `spinner_line`'s signature is unchanged
      (`spinner_line(Option<usize>, usize) -> Option<Line<'static>>`).
- [ ] The animation shows a dog (`🐕`) and a brain (`🧠`); the dog closes on the
      brain and an overtake frame with `💨` appears once per cycle.
- [ ] The chase distance scales with `width` (not a fixed frame set).
- [ ] The rendered line's char count never exceeds `width` for widths 10/20/40/80
      across a full cycle.
- [ ] `spinner_line(None, _)` returns `None`.
- [ ] All four gates pass on an independent re-run.

## Test plan

All in `panels.rs`'s `#[cfg(test)] mod tests`, replacing the old spinner tests:

- `spinner_line_none_when_ended`
- `spinner_line_contains_dog_and_brain_during_chase`
- `spinner_line_emits_overtake_burst_once_per_cycle` (load-bearing,
  mutation-resistant on burst-once)
- `spinner_line_scales_with_width` (load-bearing, pins width-parametric)
- `spinner_line_never_exceeds_width`
- `spinner_line_degenerate_narrow_width`

## End-to-end verification

The spinner is a live TUI animation; pin behavior via the unit tests above and
declare the live render E2E-N/A (consistent with prior dashboard-panel phases
M13/M15). One sentence in the completion log restating that is sufficient. If you
do run `cargo run -p rexymcp -- dashboard …` against a running session, note that
the dog chases the brain across the full Session-panel width.

## Authorizations

None. No new dependencies. No `docs/architecture.md` change.

## Out of scope

- Tuning the animation speed (cells-per-tick) — the dog advances one cell per
  loop tick; the user will hand-tune cadence later. Do not add a speed divisor.
- The `render.rs` call site — it already passes `(state.spinner,
  session_inner_width)`; leave it unchanged.
- Any other panel.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
