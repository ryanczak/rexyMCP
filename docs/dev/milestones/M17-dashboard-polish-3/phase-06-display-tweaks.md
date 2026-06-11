# Phase 06: Dashboard display tweaks — layout, pan rate, label renames

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** done
**Depends on:** phase-05 (no code overlap)
**Estimated diff:** ~140 lines (panels.rs + render.rs + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Six visual refinements to the live dashboard (display-only; no new events, no
new deps, no new public APIs):

1. Session panel 4 columns wider by reducing the Budget panel's minimum width by 4.
2. Task title pan speed 50% faster; only the **active** task pans — done and
   pending tasks are frozen at their static truncated title.
3. Budget panel: `"$ saved"` → `"Savings:"`.
4. Move the context-usage line from the Budget panel to the top of the Reclaim
   panel; rename `"Context:"` → `"Usage:"`.
5. Rename the `" Reclaim "` panel title → `" Context "`.
6. Progress bar in Tasks panel spans the full panel inner width instead of a
   fixed 10 cells.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs:18` — `GAUGE_CELLS: usize = 10` (remove in §6).
- `mcp/src/dashboard/panels.rs:172–211` — `reclaim_lines` (gains Usage block in §4).
- `mcp/src/dashboard/panels.rs:235–258` — `tasks_gauge_line(done, total)` (gains `width` in §6).
- `mcp/src/dashboard/panels.rs:262–284` — `tasks_lines(summary, width, tick)` (two
  changes: pass `width` to gauge; active-only tick in §2).
- `mcp/src/dashboard/panels.rs:286–314` — `TASK_SCROLL_DELAY` const and `scrolled_title`
  (formula change in §2).
- `mcp/src/dashboard/panels.rs:388–444` — `budget_lines` (context block removed in §4).
- `mcp/src/dashboard/panels.rs:461–472` — `dollars_saved_line` (label change in §3).
- `mcp/src/dashboard/render.rs:135–166` — header-band layout (constraint + panel title).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read all architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

```rust
// render.rs:140–145
let [session_area, budget_area, compactions_area] = Layout::horizontal([
    Constraint::Fill(1),
    Constraint::Min(56),          // Budget minimum
    Constraint::Percentage(28),
]).areas::<3>(header);

// render.rs:163–166
frame.render_widget(
    panel(" Reclaim ", reclaim_lines(&data.summary)),
    compactions_area,
);

// panels.rs:18
const GAUGE_CELLS: usize = 10;

// panels.rs:235–258
pub(crate) fn tasks_gauge_line(done: usize, total: usize) -> Line<'static> {
    ...
    let filled = (((done as f64 / total as f64) * GAUGE_CELLS as f64).round() as usize)
        .min(GAUGE_CELLS);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(GAUGE_CELLS - filled));
    ...
    Line::from(Span::styled(format!("{bar} {done}/{total} ({pct}%)"), ...))
}

// panels.rs:271
let mut lines = vec![tasks_gauge_line(summary.tasks_done, summary.tasks_total)];

// panels.rs:278–281
lines.push(Line::from(vec![
    Span::styled(glyph, Style::new().fg(color)),
    Span::raw(format!(" {}", scrolled_title(&task.title, title_max, tick))),
]));

// panels.rs:288
const TASK_SCROLL_DELAY: usize = 2;

// panels.rs:303
let step = t / TASK_SCROLL_DELAY;

// panels.rs:422–442 (in budget_lines)
if let Some(pct) = summary.last_context_pct {
    if pct == 0.0 {
        lines.push(Line::from("Context: — (unmeasured)"));
    } else {
        let pct_int = (pct * 100.0).round() as u32;
        let color = ...;
        let label = match (...) { ... format!("Context: {pct_int}% ({used}/{window})") ... };
        lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
    }
}

// panels.rs:468,471 (in dollars_saved_line)
return Some(Line::from("$ saved: —"));
...
Some(Line::from(format!("$ saved: ${saved:.2}")))
```

## Spec

### §1 — Session panel wider; Budget panel narrower (render.rs:142)

Change `Constraint::Min(56)` → `Constraint::Min(52)`.

```rust
let [session_area, budget_area, compactions_area] = Layout::horizontal([
    Constraint::Fill(1),
    Constraint::Min(52),          // was 56 — Budget 4 cols narrower
    Constraint::Percentage(28),
]).areas::<3>(header);
```

Update the comment above this block (line 135) to reflect the new minimum (52
still fits the longest tok/s stats line for reasonable session counts).

### §2 — 50% faster pan, active-only (panels.rs)

**2a. Remove `TASK_SCROLL_DELAY` constant (line 288).** Delete the line.

**2b. Update `scrolled_title` formula (line 303).** Change:

```rust
let step = t / TASK_SCROLL_DELAY;
```

to:

```rust
let step = t * 3 / 4;
```

Old rate: 1 char per 2 ticks (0.5 chars/tick). New rate: 3 chars per 4 ticks
(0.75 chars/tick) — exactly 50% faster. The triangle-wave logic below (`period`,
`phase`, `start`) is unchanged.

**2c. Active-only tick in `tasks_lines` (lines 278–281).** Change:

```rust
lines.push(Line::from(vec![
    Span::styled(glyph, Style::new().fg(color)),
    Span::raw(format!(" {}", scrolled_title(&task.title, title_max, tick))),
]));
```

to:

```rust
let task_tick = if task.state == TaskState::Active { tick } else { None };
lines.push(Line::from(vec![
    Span::styled(glyph, Style::new().fg(color)),
    Span::raw(format!(" {}", scrolled_title(&task.title, title_max, task_tick))),
]));
```

`tick == None` passed to `scrolled_title` returns `truncate_title(title, max)`
(the frozen/ellipsis path). Done and Pending tasks will show the static head
window at all ticks; only Active tasks animate.

### §3 — "$ saved" → "Savings:" (panels.rs:468,471)

In `dollars_saved_line`, change both string literals:

```rust
// line 468
return Some(Line::from("Savings: —"));
// line 471
Some(Line::from(format!("Savings: ${saved:.2}")))
```

### §4 — Move Context → Usage at top of Reclaim panel (panels.rs)

**4a. Remove the context block from `budget_lines`.** Delete lines 422–442:

```rust
    if let Some(pct) = summary.last_context_pct {
        if pct == 0.0 {
            lines.push(Line::from("Context: — (unmeasured)"));
        } else {
            // ... all the pct_int / color / label / push lines ...
        }
    }
```

The function now ends at `lines` (just tokens + tok/s lines, or the "No metrics
yet" placeholder).

**4b. Add Usage block at the top of `reclaim_lines`.** Insert the following
immediately after `let mut lines = Vec::new();` (before the
`if summary.compaction_count > 0` guard):

```rust
    if let Some(pct) = summary.last_context_pct {
        if pct == 0.0 {
            lines.push(Line::from("Usage: — (unmeasured)"));
        } else {
            let pct_int = (pct * 100.0).round() as u32;
            let color = if pct_int < 50 {
                Color::Green
            } else if pct_int < 80 {
                Color::Yellow
            } else {
                Color::Red
            };
            let label = match (summary.last_context_used, summary.last_context_window) {
                (Some(used), Some(window)) if window > 0 => {
                    format!("Usage: {pct_int}% ({used}/{window})")
                }
                _ => format!("Usage: {pct_int}%"),
            };
            lines.push(Line::from(Span::styled(label, Style::new().fg(color))));
        }
    }
```

The existing `if lines.is_empty() { return vec![...] }` guard stays — if context
pct is `None` and no reclaim events occurred, the panel still shows "(No reclaim
yet)".

### §5 — Rename " Reclaim " → " Context " (render.rs:164)

```rust
frame.render_widget(
    panel(" Context ", reclaim_lines(&data.summary)),
    compactions_area,
);
```

### §6 — Full-width progress gauge (panels.rs)

**6a. Remove `GAUGE_CELLS` constant (line 18).** Delete the line.

**6b. Change `tasks_gauge_line` signature to accept `width`.** New implementation:

```rust
pub(crate) fn tasks_gauge_line(done: usize, total: usize, width: usize) -> Line<'static> {
    let pct = if total == 0 {
        0
    } else {
        ((done as f64 / total as f64) * 100.0).round() as u32
    };
    let suffix = format!(" {done}/{total} ({pct}%)");
    let gauge_cells = width.saturating_sub(suffix.len()).max(1);
    let filled = if total == 0 {
        0
    } else {
        (((done as f64 / total as f64) * gauge_cells as f64).round() as usize)
            .min(gauge_cells)
    };
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(gauge_cells - filled));
    let color = if pct >= 80 {
        Color::Green
    } else if pct >= 40 {
        Color::Yellow
    } else {
        Color::Rgb(200, 200, 200)
    };
    Line::from(Span::styled(format!("{bar}{suffix}"), Style::new().fg(color)))
}
```

`suffix` starts with a space (`" {done}/..."`) so `format!("{bar}{suffix}")` keeps
the space between bar and fraction without an extra separator.

**6c. Update the `tasks_lines` internal call site (line 271):**

```rust
let mut lines = vec![tasks_gauge_line(summary.tasks_done, summary.tasks_total, width)];
```

No call-site changes in `render.rs` — `tasks_lines` already receives `width` from
the caller and now forwards it to the gauge.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (all existing tests updated; new tests added).
- [ ] `render.rs` layout has `Constraint::Min(52)` for the Budget area.
- [ ] `GAUGE_CELLS` and `TASK_SCROLL_DELAY` constants are gone from `panels.rs`.
- [ ] `tasks_gauge_line` signature is `(done: usize, total: usize, width: usize)`.
- [ ] `scrolled_title` formula is `t * 3 / 4` (no reference to `TASK_SCROLL_DELAY`).
- [ ] Budget panel (`budget_lines`) contains no "Context:" or "Usage:" text.
- [ ] Context panel (`reclaim_lines`) emits "Usage:" as its first line when context
      pct is `Some`; "(No reclaim yet)" only when pct is `None` and no events.
- [ ] Panel title is `" Context "` (not `" Reclaim "`).
- [ ] `dollars_saved_line` emits `"Savings: —"` and `"Savings: $X.XX"`.

## Test plan

### §2 tests — pan rate and active-only

**Update `scrolled_title_pans_overflowing_title`** (line 1174–1183).
The test asserts that at a specific tick the window starts at char 3 ("defghijklm").
Old: `Some(TASK_SCROLL_DELAY * 3)` = `Some(6)` → step=3 (via `6/2`).
New: `Some(4)` → step=3 (via `4*3/4=3`). Change the tick literal only:

```rust
assert_eq!(scrolled_title(FIXTURE, max, Some(4)), "defghijklm");
```

**Rewrite `scrolled_title_ping_pongs`** (line 1185–1210).
Replace the `TASK_SCROLL_DELAY`-based period/step_by loop with a tick range
that covers a full cycle under the new formula:

```rust
#[test]
fn scrolled_title_ping_pongs() {
    let max = 10;
    let overflow = FIXTURE.len() - max; // 20
    let mut starts = Vec::new();
    for t in 0..=200usize {
        let window = scrolled_title(FIXTURE, max, Some(t));
        let start = FIXTURE.find(&window).unwrap_or(0);
        starts.push(start);
    }
    let max_start = *starts.iter().max().unwrap();
    assert_eq!(
        max_start, overflow,
        "max start ({max_start}) should equal overflow ({overflow})"
    );
    let descends = starts.windows(2).any(|w| w[1] < w[0]);
    assert!(descends, "ping-pong sequence must descend at some point");
}
```

Derivation: `step = t * 3 / 4`. Step=20 (== overflow) first occurs at t=27
(27 × 3 / 4 = 20); the window then descends. The range 0..=200 covers multiple
full cycles; `max_start == 20` confirms the pan reaches the far end.

**Add `tasks_lines_non_active_tasks_do_not_pan`:**

```rust
#[test]
fn tasks_lines_non_active_tasks_do_not_pan() {
    // 30-char title; title_max = 20, overflow = 10.
    // At tick=4: step = 4*3/4 = 3, so Active window shifts to chars[3..23].
    // Done/Pending receive tick=None → truncate_title → frozen at "abcdefghijklmnopqrst…".
    let long = "abcdefghijklmnopqrstuvwxyzABCD".to_string(); // 30 distinct chars
    let summary = StatusSummary {
        tasks_total: 3,
        tasks_done: 1,
        tasks: vec![
            crate::status::TaskRow {
                id: "a".into(),
                title: long.clone(),
                state: crate::status::TaskState::Done,
            },
            crate::status::TaskRow {
                id: "b".into(),
                title: long.clone(),
                state: crate::status::TaskState::Active,
            },
            crate::status::TaskRow {
                id: "c".into(),
                title: long.clone(),
                state: crate::status::TaskState::Pending,
            },
        ],
        ..StatusSummary::default()
    };
    let width = 22; // title_max = 20
    let lines_0 = tasks_lines(&summary, width, Some(0));
    let lines_4 = tasks_lines(&summary, width, Some(4));
    let text_0: Vec<String> = lines_0.iter().map(|l| format!("{l}")).collect();
    let text_4: Vec<String> = lines_4.iter().map(|l| format!("{l}")).collect();
    // Index 0 = gauge, 1 = done task, 2 = active task, 3 = pending task.
    assert_eq!(text_0[1], text_4[1], "done task must not pan");
    assert_eq!(text_0[3], text_4[3], "pending task must not pan");
    assert_ne!(text_0[2], text_4[2], "active task must pan at tick=4");
}
```

### §3 tests — "Savings:" label

**Update `dollars_saved_line_dash_when_rates_unset`** (line 1517–1527):

```rust
assert_eq!(format!("{}", line.unwrap()), "Savings: —");
```

**Update `dollars_saved_line_shows_dollars`** (line 1529–1543):

```rust
assert_eq!(format!("{}", line.unwrap()), "Savings: $10.50");
```

### §4 tests — Context → Usage migration

**Update `budget_lines_shows_tokens_and_context`** (line 1238–1251).
Remove the `assert!(text.iter().any(|s| s.contains("62%")))` line (context is
now in `reclaim_lines`). Rename to `budget_lines_shows_tokens`:

```rust
#[test]
fn budget_lines_shows_tokens() {
    let summary = StatusSummary {
        last_input_tokens: Some(1200),
        last_output_tokens: Some(340),
        last_context_pct: Some(0.62),
        ..StatusSummary::default()
    };
    let lines = budget_lines(&summary);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(text.iter().any(|s| s.contains("1200")));
    assert!(text.iter().any(|s| s.contains("340")));
    assert!(!text.iter().any(|s| s.contains("Context:") || s.contains("Usage:")),
        "context line must not appear in budget_lines");
}
```

**Remove these three budget tests** (context behavior they tested now lives in
`reclaim_lines`):
- `budget_lines_shows_context_used_and_window` (line 1253–1269)
- `budget_lines_context_omits_fraction_when_window_zero` (line 1271–1289)
- `budget_lines_unmeasured_when_zero_pct` (line 1291–1303)

**Add five new `reclaim_lines` tests** (place them after the existing seven
reclaim_lines tests, before the `// --- dollars_saved tests ---` comment):

```rust
#[test]
fn reclaim_lines_shows_usage_when_context_pct_set() {
    let summary = StatusSummary {
        last_context_pct: Some(0.62),
        ..StatusSummary::default()
    };
    let lines = reclaim_lines(&summary);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(text.iter().any(|s| s.contains("Usage:")),
        "Usage line must be present: {text:?}");
    assert!(text.iter().any(|s| s.contains("62%")),
        "percentage must appear: {text:?}");
}

#[test]
fn reclaim_lines_usage_is_first_line() {
    let summary = StatusSummary {
        last_context_pct: Some(0.55),
        compaction_count: 1,
        compaction_tokens_before: 1000,
        compaction_tokens_after: 600,
        ..StatusSummary::default()
    };
    let lines = reclaim_lines(&summary);
    let first = format!("{}", lines[0]);
    assert!(first.contains("Usage:"),
        "Usage must be the first line; got: {first}");
}

#[test]
fn reclaim_lines_usage_color_red_when_high() {
    let summary = StatusSummary {
        last_context_pct: Some(0.85),
        ..StatusSummary::default()
    };
    let lines = reclaim_lines(&summary);
    assert_eq!(
        lines[0].spans[0].style.fg,
        Some(Color::Red),
        "pct >= 80 must render red"
    );
}

#[test]
fn reclaim_lines_usage_shows_fraction_with_used_and_window() {
    let summary = StatusSummary {
        last_context_pct: Some(0.68),
        last_context_used: Some(31195),
        last_context_window: Some(45875),
        ..StatusSummary::default()
    };
    let lines = reclaim_lines(&summary);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    let usage_line = text.iter().find(|s| s.contains("Usage:")).unwrap();
    assert!(usage_line.contains("68%"), "pct in: {usage_line}");
    assert!(usage_line.contains("31195"), "used in: {usage_line}");
    assert!(usage_line.contains("45875"), "window in: {usage_line}");
}

#[test]
fn reclaim_lines_usage_unmeasured_when_zero_pct() {
    let summary = StatusSummary {
        last_context_pct: Some(0.0),
        ..StatusSummary::default()
    };
    let lines = reclaim_lines(&summary);
    let text: Vec<String> = lines.iter().map(|l| format!("{l}")).collect();
    assert!(text.iter().any(|s| s.contains("unmeasured")));
    assert!(!text.iter().any(|s| s.contains("0%")));
}
```

### §6 tests — full-width gauge

**Update all five `tasks_gauge_line` call sites** to add `width = 40` and update
the bar-count assertions for the new `gauge_cells` values.

Derivation table (all at `width = 40`):

| call | pct | suffix | gauge_cells | filled | empty |
|------|-----|--------|------------|--------|-------|
| `(4, 4, 40)` | 100 | `" 4/4 (100%)"` = 11 chars | 29 | 29 | 0 |
| `(1, 2, 40)` | 50  | `" 1/2 (50%)"` = 10 chars  | 30 | 15 | 15 |
| `(0, 5, 40)` | 0   | `" 0/5 (0%)"` = 9 chars    | 31 | 0  | 31 |
| `(3, 8, 40)` | 38  | `" 3/8 (38%)"` = 10 chars  | 30 | 11 (`round(11.25)`) | 19 |
| `(0, 0, 40)` | 0   | `" 0/0 (0%)"` = 9 chars    | 31 | 0  | — |

Updated tests:

```rust
#[test]
fn tasks_gauge_line_full_is_green_and_complete() {
    let line = tasks_gauge_line(4, 4, 40);
    let text = format!("{line}");
    assert!(text.contains("4/4"), "should contain fraction: {text}");
    assert!(text.contains("100%"), "should contain 100%%: {text}");
    assert_eq!(text.matches('█').count(), 29, "should have 29 filled cells: {text}");
    assert_eq!(text.matches('░').count(),  0, "should have 0 empty cells: {text}");
    assert_eq!(line.spans[0].style.fg, Some(Color::Green), "should be green");
}

#[test]
fn tasks_gauge_line_half() {
    let line = tasks_gauge_line(1, 2, 40);
    let text = format!("{line}");
    assert!(text.contains("1/2"));
    assert!(text.contains("50%"));
    assert_eq!(text.matches('█').count(), 15, "should have 15 filled cells: {text}");
    assert_eq!(text.matches('░').count(), 15, "should have 15 empty cells: {text}");
    assert_eq!(line.spans[0].style.fg, Some(Color::Yellow), "should be yellow");
}

#[test]
fn tasks_gauge_line_zero_progress() {
    let line = tasks_gauge_line(0, 5, 40);
    let text = format!("{line}");
    assert!(text.contains("0/5"));
    assert!(text.contains("0%"));
    assert_eq!(text.matches('█').count(),  0, "should have 0 filled cells: {text}");
    assert_eq!(text.matches('░').count(), 31, "should have 31 empty cells: {text}");
    assert_eq!(line.spans[0].style.fg, Some(Color::Rgb(200, 200, 200)), "should be grey");
}

#[test]
fn tasks_gauge_line_fraction_and_fill() {
    let line = tasks_gauge_line(3, 8, 40);
    let text = format!("{line}");
    assert!(text.contains("3/8"));
    assert!(text.contains("38%"), "should contain 38%% (round(37.5)): {text}");
    assert_eq!(
        text.matches('█').count(),
        11,
        "should have 11 filled cells (round(11.25) at gauge_cells=30): {text}"
    );
}

#[test]
fn tasks_gauge_line_zero_total_does_not_panic() {
    let line = tasks_gauge_line(0, 0, 40);
    let text = format!("{line}");
    assert!(text.contains("0/0"));
    assert!(text.contains("0%"));
}
```

**Add `tasks_gauge_line_fills_panel_width`** (the mutation-resistant test — an
implementation that still uses a fixed GAUGE_CELLS would produce `text.chars().count()
< width`):

```rust
#[test]
fn tasks_gauge_line_fills_panel_width() {
    // pct = round(3/7*100) = 43; suffix = " 3/7 (43%)" = 10 chars;
    // gauge_cells = 40-10 = 30; text.chars().count() = 30 + 10 = 40.
    let width = 40;
    let line = tasks_gauge_line(3, 7, width);
    let text = format!("{line}");
    assert_eq!(
        text.chars().count(),
        width,
        "gauge line must fill panel width {width}: got {} chars in {text:?}",
        text.chars().count()
    );
}
```

## End-to-end verification

After all gates pass (`cargo fmt --all --check`, `cargo build`, `cargo clippy`,
`cargo test`), do a visual spot-check if a live session is available:

1. Confirm the dashboard renders without panic.
2. The Budget panel is ~4 columns narrower; the Session panel has gained that
   space (visible when milestone name is long enough to use it).
3. Active task titles animate; done/pending task titles remain frozen.
4. The gauge spans the full Tasks inner width.
5. The right-column third panel title reads "Context", not "Reclaim".
6. The "Usage:" line appears in the Context panel, not in Budget.
7. The savings line in Budget reads "Savings: …".

If no live session is available, the gate suite is sufficient.

## Authorizations

- Edit `mcp/src/dashboard/panels.rs`.
- Edit `mcp/src/dashboard/render.rs`.
- No other files.

## Out of scope

- Phase-07 savings scope expansion (session / milestone / project) — separate
  phase.
- Any change to `SessionEvent` variants or session-log schema.
- Any new Cargo dependency.
- Any change outside `mcp/src/dashboard/`.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-06-11 22:25 (started)

**By:** executor

Implementing all six dashboard display tweaks: layout constraint, pan rate, label renames, context→usage migration, panel rename, full-width gauge.

### Update — 2026-06-11 22:25 (complete)

**By:** executor

**Summary:** All six display tweaks implemented — layout, pan rate, label renames, context→usage migration, panel rename, full-width gauge.

**Acceptance criteria verified:**
- [x] `cargo build` succeeds with zero warnings
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [x] `cargo fmt --all --check` passes
- [x] `cargo test` passes (734 passed; 0 failed; 2 ignored)
- [x] `render.rs` layout has `Constraint::Min(52)` for the Budget area
- [x] `GAUGE_CELLS` and `TASK_SCROLL_DELAY` constants are gone from `panels.rs`
- [x] `tasks_gauge_line` signature is `(done: usize, total: usize, width: usize)`
- [x] `scrolled_title` formula is `t * 3 / 4` (no reference to `TASK_SCROLL_DELAY`)
- [x] Budget panel (`budget_lines`) contains no "Context:" or "Usage:" text
- [x] Context panel (`reclaim_lines`) emits "Usage:" as its first line when context pct is `Some`
- [x] Panel title is `" Context "` (not `" Reclaim "`)
- [x] `dollars_saved_line` emits `"Savings: —"` and `"Savings: $X.XX"`

**Grep verification:**
- `grep -c 'GAUGE_CELLS' panels.rs` → 0
- `grep -c 'TASK_SCROLL_DELAY' panels.rs` → 0
- `grep -n 'Constraint::Min(52)' render.rs` → 142
- `grep -n '" Context "' render.rs` → 164
- `grep -n 'Savings: —' panels.rs` → 3 matches (doc, impl, test)
- `grep -n 'Usage:' panels.rs` → 3 production matches + test matches
- `grep -n 't \* 3 / 4' panels.rs` → 333
- `grep -n 'fn tasks_gauge_line(done: usize, total: usize, width: usize)' panels.rs` → 254

**Files changed:**
- `mcp/src/dashboard/panels.rs` — all 6 spec items (§1-§6)
- `mcp/src/dashboard/render.rs` — §1 (Min 52), §5 (" Context " panel title)
- `docs/dev/milestones/M17-dashboard-polish-3/phase-06-display-tweaks.md` — status flip + Update Log
- `docs/dev/milestones/M17-dashboard-polish-3/README.md` — phase table status flip

**Commit:** `feat: dashboard display tweaks — layout, pan rate, labels, context panel`

**Verification commands:**
```
cargo fmt --all --check → 0
cargo build → 0
cargo clippy --all-targets --all-features -- -D warnings → 0
cargo test → 734 passed; 0 failed; 2 ignored
```

**End-to-end verification:** N/A — no live dashboard session available; gate suite sufficient per phase doc.

**Notes for review:** None — all changes match the spec exactly.

### Review verdict — 2026-06-11

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — only `mcp/src/dashboard/panels.rs` and
  `render.rs` touched (plus the phase-doc/README bookkeeping); no
  `SessionEvent`, config, or `Cargo.toml` change.
- **Calibration:** none. Clean 91-turn first-try. All six spec items
  (§1 `Min(52)` · §2 `t * 3 / 4` + active-only pan · §3 `Savings:` · §4
  Context→Usage migration to `reclaim_lines` · §5 `" Context "` title · §6
  full-width `tasks_gauge_line(done, total, width)`) match the spec verbatim;
  `GAUGE_CELLS`/`TASK_SCROLL_DELAY` removed; the `reclaim_lines` "(No reclaim
  yet)" empty-guard preserved. All four gates green on independent re-run
  (fmt/build/clippy clean; **734 executor + 374 mcp** pass, 2 ignored).
  Production paths clean of `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]`
  (all five `.expect()` hits are in the test module, ≥ line 504). New tests
  mutation-resistant: `tasks_gauge_line_fills_panel_width` (asserts
  `chars().count() == 40` — fails any fixed-cell impl) and
  `tasks_lines_non_active_tasks_do_not_pan` (done/pending frozen across ticks,
  active pans). E2E is a TUI render — N/A per the established dashboard-panel
  precedent; the unit tests render the real `Line`/`Span` output. The
  local-LLM Update-Log identity self-stamp ("By: executor") is cosmetic; the
  date is correct (`2026-06-11`). **M17 phase-07 remains in scope — do not
  close M17.**
