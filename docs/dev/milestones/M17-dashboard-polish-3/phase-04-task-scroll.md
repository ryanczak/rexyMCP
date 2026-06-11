# Phase 04: Scroll overflowing task titles in the Tasks panel

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** in-progress
**Depends on:** phase-02 (shares the `spinner` tick counter; no code overlap)
**Estimated diff:** ~120 lines (scroll math + signature thread + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Task titles wider than the Tasks panel are currently clipped with `…`. Instead,
**pan** an overflowing title back and forth within the available width so the
whole name is readable over time. Titles that already fit do not move.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs:262–280` — `tasks_lines(summary, width)` and the
  `truncate_title` call it uses today.
- `mcp/src/dashboard/panels.rs:214–221` — `truncate_title` (the static
  fits/`…`-truncate path, reused for non-scrolling titles).
- `mcp/src/dashboard/render.rs:249` — the **only** production call site:
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
  `truncate_title` — so the existing static tests pass with `tick = None`. For an
  **overflowing** title this means the frozen (`None`) window is the **ellipsized**
  head (`max-1` chars + `…`), while a **scrolling** window (`tick = Some`) is a
  **raw** `max`-char slice with no `…`. The two head windows differ deliberately;
  both are pinned in the Test plan.
- The window is always exactly `max` chars wide for an overflowing title, so the
  line width stays stable as it pans (no reflow). (`truncate_title` also yields
  `max` display chars — `max-1` + the single `…` glyph.)

### 3. Update all `tasks_lines` call sites

The signature change touches **6** call sites. Update each by adding the new
third argument. Enumerate them in one pass (compiler E0061 will list any missed):

1. `render.rs:249` (production) — pass the live tick:
   `tasks_lines(&data.summary, tasks_inner_width, state.spinner)`.
2. `panels.rs` test `tasks_lines_empty_placeholder` (~line 906) —
   `tasks_lines(&summary, 40, None)`.
3. `panels.rs` test `tasks_lines_lists_named_tasks_with_glyphs` (~line 942) —
   `tasks_lines(&summary, 40, None)`.
4. `panels.rs` test `tasks_lines_truncates_long_title` (~line 991) —
   `tasks_lines(&summary, 26, None)`.
5. `panels.rs` test `tasks_lines_uses_full_panel_width` (~line 1020) —
   `tasks_lines(&summary, 60, None)`.
6. `panels.rs` test `tasks_lines_uses_full_panel_width` (~line 1028, the second
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
- [ ] `tick = None` freezes an overflowing title at its head window — the static
      `truncate_title` form (`max-1` chars + `…`), matching today's behavior.
- [ ] Char-indexed (a multibyte title does not panic or split a glyph).
- [ ] All four gates pass on an independent re-run.

## Test plan

In `panels.rs`'s test module.

**Fixture — use an all-distinct-character title for any test that recovers a
window's start index by substring search.** A repeating fixture is what bounced
the first dispatch: with `"012345678901234567890123456789"`, a 10-char window
occurs at multiple indices, so `title.find(&window)` returns the *first* match
and can never observe a start ≥ 10 — the ping-pong test then reads a max start of
9 instead of `overflow = 20` and fails a correct impl. Pin a 30-distinct-char
fixture so every 10-char window is unique:

```rust
const FIXTURE: &str = "abcdefghijklmnopqrstuvwxyzABCD"; // 30 distinct chars
// max = 10 → overflow = 20; each 10-char window appears exactly once, so
// FIXTURE.find(&window) recovers the true start index unambiguously.
```

- Keep the existing `tasks_lines_*` tests, adding `None` as the third arg.
  `tasks_lines_truncates_long_title` (width 26, `None`) still asserts the static
  `…` truncation — confirming the `tick = None` path equals today's behavior.
- `scrolled_title_returns_whole_when_fits` — `scrolled_title("short", 20,
  Some(5))` == `"short"` (no movement).
- `scrolled_title_pans_overflowing_title` — `FIXTURE`, `max = 10`. The **scrolling**
  window is a raw `max`-char slice (no `…`): `tick = Some(0)` → `"abcdefghij"`
  (start 0); `tick = Some(TASK_SCROLL_DELAY * 3)` → start 3 → `"defghijklm"`.
  Mutation-resistant: an impl that ignores `tick` (always head) fails the
  later-tick assertion.
- `scrolled_title_ping_pongs` — `FIXTURE`, `max = 10`, `overflow = 20`. Collect the
  recovered start index (`FIXTURE.find(&window)`) across a full period of ticks
  (`0..overflow * 2 * TASK_SCROLL_DELAY` stepping by `TASK_SCROLL_DELAY`). Assert
  the **max start reached equals `overflow`** (20 — the tail is reached) and the
  sequence is **non-monotonic** (descends at some point). Mutation-resistant vs a
  wrap-around impl (which jumps `overflow → 0` discontinuously and never produces
  the descending half). **This passes only with the distinct `FIXTURE`** — see the
  fixture note above.
- `scrolled_title_frozen_when_tick_none` — overflowing `FIXTURE`, `tick = None`.
  The **frozen** window uses `truncate_title`, so it is the **ellipsized** head
  (`max-1` chars + `…`), *not* the raw first `max` chars: assert it equals
  `truncate_title(FIXTURE, max)` == `"abcdefghi…"`. (This is the same static form
  `tasks_lines_truncates_long_title` already asserts, and matches §2's
  `None → truncate_title`. The scrolling head from `Some(0)` is the raw
  `"abcdefghij"` — the two head windows differ deliberately; pin both.)
- `scrolled_title_char_indexed_multibyte` — a title with multibyte chars (e.g.
  `"日本語テスト"` repeated past `max`) does not panic and returns `max` chars. (A
  repeated fixture is fine *here* — this test checks only char count + no panic,
  it does not recover a start index.)

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

### Notes for executor — 2026-06-11

The first dispatch (session `phase-04-6a2b00fc`) **hard-failed on a transient
backend decode error** (`BackendError: "error decoding response body"`) after 34
turns — an infrastructure blip on the LLM endpoint, **not** a problem with your
work. Your `scrolled_title` implementation logic was actually **correct** (the
triangle-wave window reaches `overflow` and the early `Some(t) => t` unwrap is the
right shape — keep that approach). The run left two **test-plan** defects that I
have since fixed in this doc; re-implement against the corrected Spec/Test plan and
you'll pass clean:

1. **Frozen (`tick = None`) head window is the *ellipsized* `truncate_title` form**
   (`max-1` chars + `…`), e.g. `"abcdefghi…"` — **not** the raw first `max` chars.
   The prior test asserted `&title[..max]` and failed against §2's
   `None → truncate_title`. The Test plan and acceptance criteria now state this
   explicitly. A **scrolling** window (`tick = Some`) is still the raw `max`-char
   slice with no `…` — the two head windows differ deliberately.
2. **The ping-pong test must use an all-distinct-character fixture.** The prior run
   used a repeating-digit title, so `title.find(&window)` returned the first match
   and could never observe a start ≥ 10 — it read max-start 9 instead of
   `overflow = 20` and failed a correct impl. Use the pinned
   `FIXTURE = "abcdefghijklmnopqrstuvwxyzABCD"` (see the Test plan fixture note).

Also: **run `cargo fmt --all` on the files you touch before reporting** — the prior
render.rs call-site edit was left un-formatted (the multi-line `tasks_lines(...)`
call rustfmt wants reflowed), which would fail the `{FORMAT_COMMAND}` gate. After
the four gates pass, **stop and report** per STANDARDS §8 — do not keep editing.

### Update — 2026-06-11 12:33 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the hard_fail was a transient backend decode error, but the run also
exposed two architect-authored test-plan defects (a §2-vs-Test-plan contradiction
on the frozen-head ellipsis, and a repeating-fixture that breaks `find`-based start
recovery) that a bare retry would reproduce; refining the spec is the cheap fix and
the executor's impl logic was already correct, so takeover is unwarranted.
