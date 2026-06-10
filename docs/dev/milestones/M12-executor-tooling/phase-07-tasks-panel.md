# Phase 07: Dashboard `Tasks` panel above Files

**Milestone:** M12 — Executor Tooling
**Status:** done
**Depends on:** phase-06a (substrate, done), phase-06c (model-facing flips, done)
**Estimated diff:** ~120 lines (≈70 prod + ≈50 test)
**Tags:** language=rust, kind=feature, size=s

## Goal

Surface the M12 Arc A task-tracking signal in the live dashboard: add a `Tasks`
panel (active / pending / done counts) **above** the existing Files panel in the
body's right column, halving the Files panel's height to make room. This is the
**render half** of Arc A — the data already flows (06a seeds + `rexymcp status`
folds `TaskUpdate` events into `StatusSummary.tasks_*`; 06c lets the model drive
the flips); this phase only *displays* it. It is the **last phase of M12**.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #12 (M12) — Arc A: "a dashboard `Tasks` panel."
- `docs/dev/milestones/M12-executor-tooling/README.md` § Exit criteria — "The
  dashboard shows a `Tasks` panel (active/pending/done) above the Files panel,
  with the Files panel's height halved to make room."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

This phase is **mcp-crate only** and **purely additive on the read side** — the
`StatusSummary` task fields already exist and are already populated. You touch
exactly two files: `mcp/src/dashboard/panels.rs` (add one panel-builder fn) and
`mcp/src/dashboard/render.rs` (split the right column, render the new panel).

### What already exists (do not re-build it)

`StatusSummary` already carries the task counts (`mcp/src/status.rs:79-83`),
folded last-write-wins from the `TaskUpdate` events 06a/06c emit:

```rust
/// Tracked-task counts (M12 Arc A): total distinct task ids seen, and how
/// many are currently `Done` / `Active` (last-write-wins per id).
pub tasks_total: usize,
pub tasks_done: usize,
pub tasks_active: usize,
```

There is **no** `tasks_pending` field — derive it as
`tasks_total - (tasks_done + tasks_active)` (use `saturating_sub` defensively;
the three counts are disjoint last-write-wins states so the subtraction is exact,
but never let it underflow-panic). **Do not add a field to `StatusSummary` or
touch `mcp/src/status.rs`** — the counts are complete.

The dashboard already hands `&data.summary` (a `StatusSummary`) to every panel
builder; `DashboardData.summary` is populated by `status::summarize` at
`mcp/src/dashboard/mod.rs:32`. You consume the existing summary; you add no
plumbing.

### Worked example A — the panel-builder shape to mirror (`files_lines`)

Your new `tasks_lines` is the exact same shape as the existing `files_lines`
(`mcp/src/dashboard/panels.rs:127-150`) and `reclaim_lines`
(`panels.rs:86-125`): a `pub(crate) fn` over `&StatusSummary` returning
`Vec<Line<'static>>`, with a single placeholder line for the empty case.

```rust
/// Files panel: one line per changed file, or a placeholder when none.
pub(crate) fn files_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.files_changed.is_empty() {
        return vec![Line::from("(no files changed yet)")];
    }
    summary
        .files_changed
        .iter()
        .map(|f| { /* … builds one Line per file … */ })
        .collect()
}
```

`reclaim_lines`'s empty case is the same idiom:
`return vec![Line::from("(no reclaim yet)")];` (`panels.rs:121-123`).

### Worked example B — the body layout you will modify (`render.rs:97-150`)

Today the body splits horizontally into Activity (wide-left) and a single Files
column (right). The two spots you change are the horizontal split and the final
`Files` render call:

```rust
// Body: Activity (wide-left) · Files (right).
let [activity_area, files_area] =
    Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
        .areas::<2>(body);

// … the filter-open / transcript branch renders into `activity_area` …

frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
```

`Layout::vertical([...]).areas::<N>(rect)` is the established split idiom in this
same file — the outer header/body split at `render.rs:64-65` uses it:

```rust
let [header, body] =
    Layout::vertical([Constraint::Length(9), Constraint::Min(0)]).areas::<2>(area);
```

`panel(title, lines)` (`panels.rs:285-287`) wraps lines in a bordered block; its
`title` is `&'static str`, so a literal like `" Tasks "` works directly.

## Spec

Numbered tasks in execution order.

1. **Add the `tasks_lines` panel builder.** In `mcp/src/dashboard/panels.rs`,
   add a `pub(crate) fn tasks_lines(summary: &StatusSummary) -> Vec<Line<'static>>`
   next to `files_lines`:
   - **Empty case:** when `summary.tasks_total == 0`, return
     `vec![Line::from("(no tasks tracked yet)")]` (mirrors the `files_lines` /
     `reclaim_lines` placeholder idiom).
   - **Non-empty case:** compute
     `let pending = summary.tasks_total.saturating_sub(summary.tasks_done + summary.tasks_active);`
     and return exactly these three lines, in this order:
     ```rust
     vec![
         Line::from(format!("active:  {}", summary.tasks_active)),
         Line::from(format!("pending: {}", pending)),
         Line::from(format!("done:    {}/{}", summary.tasks_done, summary.tasks_total)),
     ]
     ```
     The `done` line carries the `/{total}` denominator so the panel reads as
     progress; `active` and `pending` are bare counts. Plain `Line::from`, no
     color styling (keep it simple — matches the `reclaim_lines` plain-line
     idiom; color is out of scope).

2. **Split the right column and render the Tasks panel.** In
   `mcp/src/dashboard/render.rs`:
   - Add `tasks_lines` to the `use super::panels::{…}` import list
     (`render.rs:10-12`), keeping it alphabetical-ish with the other builders.
   - Replace the horizontal body split (`render.rs:98-100`) so the right column
     is captured under a new name and then split vertically into a Tasks area
     (top) and a Files area (bottom) at **50/50 — this halves the Files panel's
     height per the exit criterion**:
     ```rust
     // Body: Activity (wide-left) · right column (Tasks over Files).
     let [activity_area, right_area] =
         Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
             .areas::<2>(body);
     let [tasks_area, files_area] =
         Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
             .areas::<2>(right_area);
     ```
     (`activity_area` keeps its name — the filter/transcript branch at
     `render.rs:102-149` still renders into it unchanged. `files_area` keeps its
     name — only its source rect changes.)
   - Immediately before the final Files render call (`render.rs:150`), render the
     Tasks panel into `tasks_area`:
     ```rust
     frame.render_widget(panel(" Tasks ", tasks_lines(&data.summary)), tasks_area);
     frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
     ```

That is the whole change. No new struct, no new field, no config, no event, no
match-arm churn.

## Acceptance criteria

- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.
- [ ] `tasks_lines` on a `StatusSummary` with `tasks_total == 0` returns a single
      placeholder line containing `no tasks tracked`.
- [ ] `tasks_lines` on a summary with `tasks_total: 3, tasks_done: 1,
      tasks_active: 1` renders `active: 1`, `pending: 1`, and `done: 1/3`
      (pending is derived, not stored).
- [ ] `render_dashboard` builds the body's right column as a vertical Tasks-over-
      Files split (50/50) and renders both panels; the existing Activity column
      and all header panels are unchanged.
- [ ] No change to `mcp/src/status.rs` (`StatusSummary` gains no field), no new
      `SessionEvent`, no config, no executor-crate change.

## Test plan

In `mcp/src/dashboard/panels.rs` (`#[cfg(test)] mod tests`, mirror the existing
`files_lines_*` tests at `panels.rs:409-443`, which build a `StatusSummary { … ,
..StatusSummary::default() }` and render lines to strings via
`lines.iter().map(|l| format!("{l}")).collect()`):

- `tasks_lines_empty_placeholder` — `StatusSummary::default()` (so
  `tasks_total == 0`) → the rendered text contains `no tasks tracked`. (Mirrors
  `files_lines_empty_placeholder`.)
- `tasks_lines_shows_counts` — `StatusSummary { tasks_total: 3, tasks_done: 1,
  tasks_active: 1, ..default() }` → rendered text contains `active:  1`,
  `pending: 1`, and `done:    1/3`. (Whitespace inside the assertion can be
  matched loosely with `contains("active:")` + `contains("1")` per line, or
  exactly — your call; pin the *counts and the `done` denominator*, not exact
  column padding.)
- `tasks_lines_derives_pending` — `StatusSummary { tasks_total: 2, tasks_done: 0,
  tasks_active: 0, ..default() }` → rendered text contains `pending: 2` and
  `done:    0/2`. (Mutation-resistant: an impl that stored or mis-derived pending
  fails this; a "show tasks_total as pending" bug would render `pending: 2` here
  but `tasks_lines_shows_counts` catches it — total 3 ≠ pending 1.)

No test is added for `render.rs`: `render_dashboard` draws into a ratatui `Frame`
and has no headless unit harness (the existing `render.rs` tests cover only the
pure `visible_offset` / `clamp_scroll` helpers). The layout wiring is covered by
the compiler (the `areas::<2>` destructuring must type-check) and by inspection;
the *content* it renders is fully tested through `tasks_lines`.

## End-to-end verification

Not applicable as an automated repro — this phase ships a TUI layout change with
no headlessly-observable runtime artifact (the dashboard is an interactive
`ratatui` view that requires a live session and a terminal; consistent with the
prior M8/M10 dashboard-panel phases, whose panel placement was likewise verified
by inspection, not by a headless command). The **behavioral** surface — the panel
contents — is fully exercised by the `tasks_lines_*` unit tests. Quote the
`cargo test tasks_lines` run tail in the completion Update Log.

If a live terminal is available, the optional manual check is: run
`cargo run -p rexymcp -- dashboard --config rexymcp.toml` against a session log
containing `task_update` records and confirm the right column shows a `Tasks`
panel (with `active/pending/done`) above a half-height `Files` panel. This is
**optional** — do not block completion on it.

## Authorizations

None. (No new dependencies — `ratatui` `Layout`/`Constraint` are already in use.
No `Cargo.toml`/architecture/STANDARDS/WORKFLOW edits. No new struct field, event,
or config.)

## Out of scope

Do **not**, in this phase:

- Add a `tasks_pending` field to `StatusSummary` or otherwise touch
  `mcp/src/status.rs` — derive pending in `tasks_lines` from the existing counts.
- Color-style the Tasks panel lines (no green-when-all-done, etc.) — plain lines
  only; styling is a polish item for a future phase if wanted.
- Make the panel scrollable, sortable, or list individual task titles — the panel
  shows the three aggregate counts only. (Per-task title listing, if it ever
  proves useful, is a separate phase.)
- Change the header band, the Activity/transcript column, the filter panel, or the
  72/28 horizontal ratio. Only the right column's internal vertical split changes.
- Touch the executor crate, `event.rs`, `tasks.rs`, or the `update_task` tool.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 00:00 (started)

**Executor:** claude-code

Added `tasks_lines` panel builder in `panels.rs`, split right column in `render.rs` into Tasks-over-Files 50/50 vertical split, added 3 unit tests.

### Update — 2026-06-10 00:00 (complete)

**Executor:** claude-code

**Verification commands:**
- `cargo fmt --all --check` — passed (clean)
- `cargo build` — passed (zero new warnings)
- `cargo clippy --all-targets --all-features -- -D warnings` — passed (clean)
- `cargo test` — passed (722 passed; 0 failed; 2 ignored)
- `cargo test tasks_lines` — passed (3 passed)

**Verification summary:** All four gates clean (fmt, build, clippy, test). Three `tasks_lines_*` unit tests pass.

**Files changed:**
- `mcp/src/dashboard/panels.rs` — added `tasks_lines` panel builder + 3 unit tests
- `mcp/src/dashboard/render.rs` — split right column vertically (Tasks over Files, 50/50), render Tasks panel

**Commit:** `feat: add Tasks panel to dashboard above Files`

**End-to-end verification:** N/A — TUI layout change with no headlessly-observable runtime artifact. Behavioral surface fully exercised by `tasks_lines_*` unit tests (quoted `cargo test tasks_lines` output above).

**Notes for review:**
- Grep confirms literal landed: `grep "no tasks tracked" mcp/src/dashboard/panels.rs` → 2 matches (line 130 impl, line 569 test).
- No change to `mcp/src/status.rs`; pending derived via `saturating_sub` as specified.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** rexyMCP executor (Qwen/Qwen3.6-27B-FP8)
- **Scope deviations:** none — `tasks_lines` + the 50/50 right-column split landed
  exactly as specified; `status.rs` untouched, pending derived via `saturating_sub`,
  no new field/event/config/match-arm churn.
- **Calibration:** none. All four gates green on independent re-run (722 executor +
  298 mcp, 0 failed, 2 ignored); the 3 `tasks_lines_*` tests are mutation-resistant
  (the `total≠pending` distinction between `shows_counts` and `derives_pending`
  catches a naive "render total as pending" impl). Cosmetic-only quirk (no fold):
  the Update Log self-stamps `2026-06-10 00:00` / "claude-code" — the recurring
  local-LLM clock/identity quirk pending the `rexymcp serve` restart for phase-06's
  datetime injection.
