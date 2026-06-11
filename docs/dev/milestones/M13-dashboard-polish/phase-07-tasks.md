# Phase 07: Tasks — named tasks with glyphs + done/total progress gauge

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none for code (independent of phases 01–06; touches the Tasks panel
builder and the `StatusSummary` task capture — neither changed by 01–06). The
`TaskUpdate` substrate and the `tasks_total/done/active` counts already exist (M12
Arc A).
**Estimated diff:** ~200 lines (new `TaskRow` struct + field ~10, `summarize`
HashMap→ordered-Vec ~12, `tasks_lines` rewrite ~30, new `tasks_gauge_line` helper
~20, tests ~120)
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the Tasks panel show **what** the executor is working on, not just **how many**
tasks there are (user item #7, enhancement R3). Today the panel renders three bare
count lines (`active: N` / `pending: N` / `done: N/T`); the task **titles** —
already carried on every `TaskUpdate` event — are discarded in `summarize`. This
phase:

1. **Captures task titles in order.** `summarize` keeps an ordered
   `Vec<TaskRow>` (id, title, state, first-seen order, last-write-wins on
   title+state) instead of the count-only `HashMap`. The existing
   `tasks_total/done/active` counts are **derived from that vec**, so they stay
   byte-identical.
2. **Renders named tasks with checkbox glyphs.** Each tracked task becomes a line
   `{glyph} {title}` — `☑` done, `▶` active, `☐` pending — with the title truncated
   to fit the narrow panel.
3. **Adds a done/total progress gauge** matching the Budget panel's context-gauge
   *style* (a single colored text line — see "Current state"): a small filled bar
   plus `done/total (pct%)`, colored by completion.

Pure presentation — no feed, config, or executor change. Everything rendered is
already in the JSONL log (`TaskUpdate.title` has been emitted since M12 phase-06a;
it was simply never read on the dashboard side).

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  **display-only** constraint and the phase table. This phase touches **only**
  `mcp/src/status.rs` (add an ordered title-bearing task capture) and
  `mcp/src/dashboard/panels.rs` (rewrite `tasks_lines`, add the gauge helper). It
  adds **no** `SessionEvent` variant, no config, and no `filter.rs`/`render.rs`
  change.
- `docs/architecture.md` § Status #12 (M12 Arc A — structured task tracking) and
  #13 (M13 thread "Panel polish" → "The **Tasks** panel shows named tasks with
  checkbox/check glyphs … and a done/total progress gauge matching the
  context-gauge style"). This phase implements that sentence.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### `summarize` discards task titles into a count-only `HashMap`

In `mcp/src/status.rs`, `summarize` (lines 101–255) folds `TaskUpdate` records into
a `HashMap<String, TaskState>` and then derives three counts, **ignoring the
title**:

```rust
// status.rs:107-108 — the only task accumulator today
let mut task_states: std::collections::HashMap<String, TaskState> =
    std::collections::HashMap::new();

// status.rs:211-213 — title is dropped (`..` pattern)
SessionEvent::TaskUpdate { id, state, .. } => {
    task_states.insert(id.clone(), *state);
}

// status.rs:244-252 — counts derived from the map
summary.tasks_total = task_states.len();
summary.tasks_done = task_states
    .values()
    .filter(|s| **s == TaskState::Done)
    .count();
summary.tasks_active = task_states
    .values()
    .filter(|s| **s == TaskState::Active)
    .count();
```

A `HashMap` has **no stable order**, so even if we read the titles we could not
render them in a sensible sequence. This phase replaces the map with an
insertion-ordered `Vec<TaskRow>`.

The `TaskUpdate` event (in `rexymcp_executor::store::sessions::event`) is:

```rust
SessionEvent::TaskUpdate { id: String, title: String, state: TaskState }
// TaskState: Pending | Active | Done  (Copy, PartialEq, Eq, Serialize)
```

`TaskState` is **already imported** in `status.rs` (line 12).

### `StatusSummary` is `Default`-built and mutated in `summarize` — field adds are cheap

`StatusSummary` (`status.rs:18-96`) derives `Default, Serialize` and is built via
`StatusSummary::default()` then mutated field-by-field. **There are no production
`StatusSummary { … }` struct literals** (grep-verified: production builds via
`::default()` + mutation in `summarize`; every test builds with
`..StatusSummary::default()`). So adding one field is a one-line struct add — **not**
the multi-site literal cascade flagged in the milestone README. The existing
`tasks_total/done/active: usize` fields (`status.rs:84-85`) stay; we only **add** an
ordered vec and **change how the same counts are computed** (from the vec instead of
the map — identical results).

### `tasks_lines` renders three bare count lines

In `mcp/src/dashboard/panels.rs`, `tasks_lines` (lines 153–169):

```rust
pub(crate) fn tasks_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.tasks_total == 0 {
        return vec![Line::from("(no tasks tracked yet)")];
    }
    let pending = summary
        .tasks_total
        .saturating_sub(summary.tasks_done + summary.tasks_active);
    vec![
        Line::from(format!("active:  {}", summary.tasks_active)),
        Line::from(format!("pending: {}", pending)),
        Line::from(format!(
            "done:    {}/{}",
            summary.tasks_done, summary.tasks_total
        )),
    ]
}
```

`render.rs:243` calls `tasks_lines(&data.summary)` — **the signature is unchanged by
this phase, so `render.rs` needs no edit.**

### The context gauge is a single colored text `Line` — this is the "style" to match

In `panels.rs`, `budget_lines` (lines 276–296) renders the context gauge **not** as a
ratatui `Gauge` widget but as one colored text line — a percentage plus a
green/yellow/red color by threshold:

```rust
if let Some(pct) = summary.last_context_pct {
    // ... (0.0 → "context: — (unmeasured)") ...
    let pct_int = (pct * 100.0).round() as u32;
    let color = if pct_int < 50 {
        Color::Green
    } else if pct_int < 80 {
        Color::Yellow
    } else {
        Color::Red
    };
    let label = /* "context: {pct_int}% ({used}/{window})" or "context: {pct_int}%" */;
    lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
}
```

The Tasks gauge matches this **style** (a single `Span::styled` text line, colored
by a percentage), rendered as a small filled bar + fraction + percent. **Do not
introduce a ratatui `Gauge` widget** — the Tasks panel is built as a `Vec<Line>`
handed to `panel()` (a `Paragraph`); a `Gauge` is a separate widget that would need
its own `Rect` and break the panel composition. Keep it a `Line`, exactly like the
context gauge.

**Gauge color is progress-oriented, not usage-oriented — a deliberate inversion.**
The context gauge means "more = worse" (red near full). Task completion means "more =
better," so the colors run the other way and a freshly-seeded list (0% done) is
**neutral**, not alarming red:

| Completion `pct` | Color |
|---|---|
| `pct >= 80` | `Color::Green` |
| `pct >= 40` | `Color::Yellow` |
| else (incl. 0%) | `Color::Rgb(200, 200, 200)` (the M13 legibility secondary-text color) |

This divergence from the context gauge's red floor is intentional; note it in "Notes
for review".

## Spec

All changes are in `mcp/src/status.rs` and `mcp/src/dashboard/panels.rs`. No other
files. No `SessionEvent`/config/`Cargo.toml` edit.

### 1. Add the `TaskRow` type and the ordered field — `status.rs`

Add a small serializable row type near `StatusSummary` (above it):

```rust
/// One tracked task as last seen in the log (M12 Arc A), in first-seen order.
/// The dashboard Tasks panel renders these by title + state; `summarize` keeps
/// the vec insertion-ordered with last-write-wins on title and state per id.
#[derive(Debug, Clone, Serialize)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub state: TaskState,
}
```

Add one field to `StatusSummary` (alongside the existing `tasks_total/done/active`,
status.rs:82-85). Keep the three count fields:

```rust
/// Tracked tasks (M12 Arc A) in first-seen order, with titles — drives the
/// dashboard Tasks panel's named-task list. The `tasks_total/done/active`
/// counts below are derived from this vec.
pub tasks: Vec<TaskRow>,
```

`Vec<TaskRow>` is `Default` (empty), so `StatusSummary::default()` is unaffected and
no `..Default::default()` test site changes. `TaskRow` needs **no** `Default` (it is
only ever pushed from real event data).

### 2. Replace the task `HashMap` with an ordered `Vec<TaskRow>` — `status.rs`

In `summarize`:

- Replace the `task_states` HashMap declaration (status.rs:107-108) with:

  ```rust
  let mut tasks: Vec<TaskRow> = Vec::new();
  ```

- Replace the `TaskUpdate` arm (status.rs:211-213) — now **read the title**, update
  in place by id (last-write-wins), else push (preserves first-seen order):

  ```rust
  SessionEvent::TaskUpdate { id, title, state } => {
      if let Some(row) = tasks.iter_mut().find(|r| &r.id == id) {
          row.title = title.clone();
          row.state = *state;
      } else {
          tasks.push(TaskRow {
              id: id.clone(),
              title: title.clone(),
              state: *state,
          });
      }
  }
  ```

- Replace the count derivation (status.rs:244-252) — derive the **same three counts**
  from the vec, then move the vec onto the summary:

  ```rust
  summary.tasks_total = tasks.len();
  summary.tasks_done = tasks.iter().filter(|r| r.state == TaskState::Done).count();
  summary.tasks_active = tasks.iter().filter(|r| r.state == TaskState::Active).count();
  summary.tasks = tasks;
  ```

The last-write-wins-per-id and count semantics are unchanged from the HashMap version
(`summarize_folds_task_states_last_write_wins` and the other existing
`status.rs` task tests must still pass without edits). The **only** new observable
behavior is that titles are now retained and order is first-seen.

### 3. Rewrite `tasks_lines` to render named tasks + the gauge — `panels.rs`

Replace `tasks_lines` (panels.rs:153-169) so it renders, when tasks exist: the
**gauge line first**, then one line per task. Keep the empty placeholder.

```rust
/// Tasks panel: a done/total progress gauge over a list of named tasks, or a
/// placeholder when none are tracked.
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

- Add `use rexymcp_executor::store::sessions::event::TaskState;` to `panels.rs`'s
  imports (it is not imported there today; `StatusSummary` and `TaskRow` come from
  `crate::status`). Import `TaskRow` too **only if** you reference the type name
  directly — iterating `&summary.tasks` and reading `task.state` does **not** require
  naming `TaskRow`, so the `TaskState` import alone suffices.
- `TASK_TITLE_MAX`: a module-level `const TASK_TITLE_MAX: usize = 24;` near the top of
  `panels.rs` (the Tasks panel is the narrow ~28%-width right column — pick a budget
  that keeps `{glyph} {title}` from overrunning; 24 chars of title is a reasonable fit
  and the panel/`Paragraph` clips any residual overflow).
- `truncate_title`: a small private helper that returns the title unchanged when it
  fits, else a `…`-suffixed prefix within budget:

  ```rust
  /// Truncate a task title to at most `max` chars, appending `…` when shortened.
  fn truncate_title(title: &str, max: usize) -> String {
      if title.chars().count() <= max {
          return title.to_string();
      }
      let keep = max.saturating_sub(1);
      let head: String = title.chars().take(keep).collect();
      format!("{head}…")
  }
  ```

### 4. Add the gauge helper — `panels.rs`

Add a pure `tasks_gauge_line` near `tasks_lines`. It renders a fixed-width filled bar
plus `done/total (pct%)`, colored per the progress table above. Only ever called when
`total > 0` (guarded by `tasks_lines`), but it must not panic at `total == 0` (return
a sane zero-progress line) so it is independently testable.

```rust
/// Cells in the Tasks progress bar.
const GAUGE_CELLS: usize = 10;

/// Done/total progress gauge for the Tasks panel — a filled bar plus
/// `done/total (pct%)`, colored by completion (progress-oriented: green = near/at
/// done, neutral grey = no progress). Matches the Budget context-gauge *style*
/// (a single colored text `Line`), not a ratatui `Gauge` widget.
pub(crate) fn tasks_gauge_line(done: usize, total: usize) -> Line<'static> {
    let pct = if total == 0 {
        0
    } else {
        ((done as f64 / total as f64) * 100.0).round() as u32
    };
    let filled = if total == 0 {
        0
    } else {
        (((done as f64 / total as f64) * GAUGE_CELLS as f64).round() as usize).min(GAUGE_CELLS)
    };
    let bar = format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(GAUGE_CELLS - filled)
    );
    let color = if pct >= 80 {
        Color::Green
    } else if pct >= 40 {
        Color::Yellow
    } else {
        Color::Rgb(200, 200, 200)
    };
    Line::from(Span::styled(
        format!("{bar} {done}/{total} ({pct}%)"),
        Style::new().fg(color),
    ))
}
```

### 5. Replace the obsolete `tasks_lines` count tests — `panels.rs`

The two existing tests `tasks_lines_shows_counts` (panels.rs:719) and
`tasks_lines_derives_pending` (panels.rs:740) assert on the old `active:`/`pending:`/
`done:` count-line format, which no longer exists. **Delete them** and replace with
the named-task + gauge tests in the Test plan below. Keep
`tasks_lines_empty_placeholder` (panels.rs:711) unchanged — the empty path is
unchanged.

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `StatusSummary` has a `tasks: Vec<TaskRow>` field; `TaskRow { id, title, state }`
      derives `Serialize`. `StatusSummary::default()` still compiles with no other
      test-site edits (the field defaults to an empty vec).
- [ ] `summarize` retains task **titles** and **first-seen order**, last-write-wins on
      title+state per id; `tasks_total/done/active` are derived from the vec and remain
      identical to today (all existing `status.rs` task tests pass unmodified).
- [ ] The Tasks panel, when tasks exist, renders a gauge line followed by one
      `{glyph} {title}` line per task: `☑` for `Done`, `▶` for `Active`, `☐` for
      `Pending`. Titles longer than `TASK_TITLE_MAX` are `…`-truncated.
- [ ] `tasks_gauge_line(done, total)` renders `done/total (pct%)` with a
      `GAUGE_CELLS`-wide bar (`█` filled / `░` empty) proportional to `done/total`,
      colored green (≥80%) / yellow (≥40%) / grey (else). `pct` and `filled` use the
      `done/total` ratio (a wrong divisor or off-by-one fill fails the pinned tests).
- [ ] The empty case is unchanged: `tasks_total == 0` →
      `vec![Line::from("(no tasks tracked yet)")]`, no gauge line.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test` all
      pass.
- [ ] `git diff --name-only` lists only `mcp/src/status.rs`,
      `mcp/src/dashboard/panels.rs` (plus this phase doc and the README row). No
      `render.rs`, `filter.rs`, `transcript.rs`, `Cargo.toml`, `SessionEvent`, or config
      edit.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` blocks (`status.rs` for the
capture, `panels.rs` for the rendering; `use super::*` is in scope in both). Names
describe behavior; exact count and placement are yours. The **load-bearing** tests are
`summarize_captures_task_titles_in_order` (pins title retention + order — the whole
point of the phase) and `tasks_gauge_line_fraction_and_fill` (pins the gauge math,
mutation-resistant).

### `status.rs`

- `summarize_captures_task_titles_in_order` — records
  `task_update("1","Read config",Pending)`, `task_update("2","Write tests",Pending)`,
  `task_update("1","Read config",Done)` → `summary.tasks` has len 2, `tasks[0].id ==
  "1"` with `state == Done` and `title == "Read config"`, `tasks[1].id == "2"`. (Pins
  first-seen order + last-write-wins + title retention. A HashMap-backed impl would
  not guarantee order; a `..`-dropping arm would lose the title.) Note: the existing
  test helper `task_update(id, state)` hardcodes `title: "Test task"` (status.rs:507);
  either extend it to take a title, or add a local 3-arg variant for this test — your
  call, but the surviving `task_update`-based tests must keep compiling.
- Confirm `summarize_folds_task_states_last_write_wins` and
  `summarize_no_tasks_when_absent` still pass **unmodified** (counts unchanged).

### `panels.rs`

- `tasks_lines_lists_named_tasks_with_glyphs` — build a summary (via
  `..StatusSummary::default()`) with `tasks: vec![ TaskRow done "Read config",
  TaskRow active "Write tests", TaskRow pending "Refactor" ]` and the matching
  `tasks_total: 3, tasks_done: 1, tasks_active: 1`. Assert the rendered lines contain
  a line with `☑` and `"Read config"`, a line with `▶` and `"Write tests"`, and a line
  with `☐` and `"Refactor"`.
- `tasks_lines_truncates_long_title` — one task whose title exceeds `TASK_TITLE_MAX`;
  assert the rendered line contains `…` and its title portion is within budget, and a
  short title is **not** truncated (no `…`).
- `tasks_lines_empty_placeholder` — keep as-is (`tasks_total == 0` → placeholder; no
  gauge line, assert no `/` or `█` present).
- `tasks_gauge_line_full_is_green_and_complete` — `tasks_gauge_line(4, 4)` →
  text contains `"4/4"` and `"100%"`, 10 `█` and 0 `░`. (Color green — assert via the
  line's first span `.style.fg == Some(Color::Green)`.)
- `tasks_gauge_line_half` — `tasks_gauge_line(1, 2)` → contains `"1/2"` and `"50%"`,
  exactly 5 `█` and 5 `░`, color `Color::Yellow`.
- `tasks_gauge_line_zero_progress` — `tasks_gauge_line(0, 5)` → contains `"0/5"` and
  `"0%"`, 0 `█` and 10 `░`, color `Color::Rgb(200, 200, 200)`.
- `tasks_gauge_line_fraction_and_fill` — `tasks_gauge_line(3, 8)` → contains `"3/8"`
  and `"38%"` (`round(37.5) = 38`) and exactly 4 `█` (`round(3.75) = 4`).
  Mutation-resistant: a wrong divisor or `floor` fill fails.

(Reading a line's text: `format!("{l}")`. Counting a glyph: `format!("{l}").matches('█').count()`.
Reading the gauge color: the gauge `Line` has a single styled span —
`line.spans[0].style.fg`.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13
phases 01–06). Verification is the `tasks_gauge_line`/`tasks_lines` pure-function
assertions plus the `summarize` title-capture assertions and the `cargo` gates. The
panel-composition path is exercised by the live binary; the line-builder functions
that drive it are fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — only `status.rs` and `panels.rs` change. **No
      `Cargo.toml` edit.**
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- Add a new `SessionEvent` variant, a config field, or change the `TaskUpdate` event.
  If you think you need one, **stop and file a blocker**: you have left M13's
  display-only scope. (Titles are already on `TaskUpdate`; this phase only stops
  discarding them.)
- Introduce a ratatui `Gauge` widget. The gauge is a `Line` matching the context-gauge
  style — see "Current state". A `Gauge` widget needs its own `Rect` and would break
  the `Vec<Line>` → `panel()` composition.
- Touch `render.rs` (the `tasks_lines(&data.summary)` call is unchanged),
  `filter.rs` (the `TaskUpdate` filter toggle already exists),
  `transcript.rs` (its own `TaskUpdate` rendering at line 169 stays as-is), or
  `mcp/src/status.rs`'s `format_status` (the `rexymcp status` **CLI text** path keeps
  its `tasks: D/T done (A active)` summary line — this phase is the **dashboard**
  panel only).
- Add scrolling, selection, or any interactivity to the Tasks panel (M8/M13
  Non-goals — the dashboard is a monitoring view). The panel clips overflow, as the
  Files panel does.
- Reorder, recolor, or restyle any other panel. Only `tasks_lines` and the `summarize`
  task capture change.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-11 00:49 (complete)

**Summary:** Implemented named tasks with checkbox glyphs and a done/total progress gauge in the Tasks panel. Replaced the count-only `HashMap` in `summarize` with an ordered `Vec<TaskRow>` that retains task titles and first-seen order. Rewrote `tasks_lines` to render a gauge line followed by `{glyph} {title}` lines per task, and added `tasks_gauge_line` matching the Budget context-gauge style (a single colored `Line`, not a ratatui `Gauge` widget). Gauge color is progress-oriented (green ≥80%, yellow ≥40%, grey else) — a deliberate inversion of the context gauge's usage-oriented coloring; noted in spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean — no output)

cargo build 2>&1 | tail -20
(clean — compiled successfully)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(clean — no warnings)

cargo test 2>&1 | tail -30
test result: ok. 725 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13 phases 01–06). Verification is the `tasks_gauge_line`/`tasks_lines` pure-function assertions plus the `summarize` title-capture assertions and the `cargo` gates.

**Grep for spec-pinned literals:**
```
$ grep -n '☑\|☐\|▶\|█\|░' mcp/src/dashboard/panels.rs | head -20
185:    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(GAUGE_CELLS - filled));
208:            TaskState::Done => ("☑", Color::Green),
209:            TaskState::Active => ("▶", Color::Yellow),
210:            TaskState::Pending => ("☐", Color::Rgb(200, 200, 200)),
... (15 more matches in tests)
```

**Files changed:**
- `mcp/src/status.rs` — added `TaskRow` struct, `tasks: Vec<TaskRow>` field on `StatusSummary`, replaced `HashMap` with ordered `Vec` in `summarize`, added `summarize_captures_task_titles_in_order` test, updated `task_update` helper to accept title
- `mcp/src/dashboard/panels.rs` — added `TASK_TITLE_MAX`, `GAUGE_CELLS` consts, `truncate_title` helper, `tasks_gauge_line` helper, rewrote `tasks_lines`, replaced old count tests with named-task + gauge tests
- `docs/dev/milestones/M13-dashboard-polish/README.md` — phase 07 row status `todo` → `review`
- `docs/dev/milestones/M13-dashboard-polish/phase-07-tasks.md` — status `todo` → `review`, completion entry appended

**New tests:**
- `summarize_captures_task_titles_in_order` in `mcp/src/status.rs`
- `tasks_lines_lists_named_tasks_with_glyphs` in `mcp/src/dashboard/panels.rs`
- `tasks_lines_truncates_long_title` in `mcp/src/dashboard/panels.rs`
- `tasks_gauge_line_full_is_green_and_complete` in `mcp/src/dashboard/panels.rs`
- `tasks_gauge_line_half` in `mcp/src/dashboard/panels.rs`
- `tasks_gauge_line_zero_progress` in `mcp/src/dashboard/panels.rs`
- `tasks_gauge_line_fraction_and_fill` in `mcp/src/dashboard/panels.rs`
- `tasks_gauge_line_zero_total_does_not_panic` in `mcp/src/dashboard/panels.rs`

**Commits:**
- (pending — one commit to be made)

**Notes for review:**
- Gauge color is **progress-oriented** (green = near/at done, grey = no progress), which is a deliberate inversion of the context gauge's **usage-oriented** colors (red = near full). This was intentional per the spec's color table — a freshly-seeded task list (0% done) is neutral, not alarming red.
- `task_update` test helper in `status.rs` was changed from 2-arg `(id, state)` to 3-arg `(id, title, state)` to support the new title-capture test. All existing callers were updated.
- Deleted `tasks_lines_shows_counts` and `tasks_lines_derives_pending` tests (old format), kept `tasks_lines_empty_placeholder` (unchanged behavior).

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — only `mcp/src/status.rs` + `mcp/src/dashboard/panels.rs`
  (plus this phase doc + the README row) touched; no `render.rs`/`filter.rs`/
  `transcript.rs`/`Cargo.toml`/`SessionEvent`/config edit, and `format_status`'s CLI
  text path was correctly left alone.
- **Calibration:** none. Clean 65-turn first-try with full bookkeeping (status flip +
  Update Log + single `feat:` commit `9e50f24`). All four gates re-run green
  independently (725 executor + 339 mcp pass, 0 failed). Production clean of new
  `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]` (the `status.rs:125`
  `prev_rec_ts.unwrap()` is pre-existing and `is_some_and`-guarded; all flagged
  `panels.rs` unwraps are test-only). Load-bearing tests confirmed mutation-resistant:
  `tasks_gauge_line_fraction_and_fill` (3/8 → "38%" + 4 `█`; a `floor` fill or wrong
  divisor fails) and `summarize_captures_task_titles_in_order` (pins first-seen order +
  last-write-wins + title retention — a HashMap-backed or `..`-dropping impl fails it).
  The progress-oriented gauge color inversion (vs the context gauge's red floor) is
  intentional per spec and noted by the executor. E2E correctly declared N/A (TUI, no
  headless harness — consistent with prior M8/M10/M12 + M13 dashboard-panel phases).
