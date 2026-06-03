# Phase 09: Dashboard redesign — header band, Compactions panel, Files trim

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** phase-07 (done — `SessionEvent::Compaction` is now in the JSONL, the
data this phase's Compactions panel renders) and phase-08 (done — the loop the new
layout renders in).
**Estimated diff:** ~150 lines (`mcp/src/status.rs` summarize + `mcp/src/dashboard.rs`
layout + two panels + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

First phase of the dashboard redesign (see the wireframe). Restructure the panel
layout into a **four-panel header band** (Session · Budget · Compactions · Heartbeat)
above a large **body** (Activity wide-left · Files right), and fill in two of the
wireframe's data panels that need no new plumbing:

- **Compactions panel** (new) — renders the `SessionEvent::Compaction` data that
  phase-07 started emitting (count of compactions + compression ratio). This is the
  render-half deferred from phase-07.
- **Files panel** — left-trim long paths so the filename tail stays visible.

**mcp-crate only.** No executor change, no new dependency, no interactivity beyond the
existing `q`/`Esc` quit. The scrollable Activity *transcript* and the Budget
*Tokens/Sec + $ saved* metrics are explicitly **later phases** (see Out of scope) —
this phase keeps the existing `activity_lines` and `budget_lines` content as-is, just
repositioned.

## Architecture references

Read before starting:

- M8 README § "Design decisions" — "Read-only, no side effects" and "Hermetic data
  layer" still hold: panels are pure line-builders tested without a terminal; the
  layout in `render_dashboard` is reviewed by inspection.
- `executor/src/store/sessions/event.rs` — the `SessionEvent::Compaction` variant
  (added phase-07) this phase reads: `{ tokens_before: usize, tokens_after: usize,
  messages_signaturized: usize, messages_evicted: usize }`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/status.rs` end to end (small) — you add three `StatusSummary` fields
   and one `summarize` match arm.
3. Read `mcp/src/dashboard.rs` end to end (small) — you rewrite `render_dashboard`'s
   layout and add one panel line-builder; you modify `files_lines`.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### `summarize` drops `Compaction` today (`mcp/src/status.rs`)

The fold ends with the `Metrics` arm and a catch-all that silently ignores
`Compaction`:

```rust
            SessionEvent::Metrics {
                input_tokens,
                output_tokens,
                context_pct,
            } => {
                summary.last_input_tokens = Some(*input_tokens);
                summary.last_output_tokens = Some(*output_tokens);
                summary.last_context_pct = Some(*context_pct);
            }
            _ => {} // Prompt, Completion, Parsed remain intentionally unread
```

`StatusSummary` (top of `status.rs`) is a flat struct of `Option`/`Vec` fields; the
most recent additions (06b) show the field+doc convention:

```rust
    /// Cumulative input tokens from the most recent `Metrics` record.
    pub last_input_tokens: Option<u32>,
    …
    pub last_context_pct: Option<f64>,
```

### The current layout (`mcp/src/dashboard.rs`, `render_dashboard`)

```rust
    // Outer split: fixed-height top row + filling middle region + fixed-height budget row.
    let [top, middle, budget_area] = Layout::vertical([
        Constraint::Length(8),
        Constraint::Min(0),
        Constraint::Length(4),
    ])
    .areas::<3>(area);

    // Top row: Session (left) | Heartbeat (right).
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas::<2>(top);
    frame.render_widget(panel(" Session ", session_lines(&data.summary)), left);
    frame.render_widget(panel(" Heartbeat ", heartbeat_lines(&data.summary, now_ms)), right);

    // Middle row: Files (left) | Activity (right).
    let [files_area, activity_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas::<2>(middle);
    frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
    frame.render_widget(panel(" Activity ", activity_lines(&data.summary)), activity_area);

    // Bottom row: Budget (full-width).
    frame.render_widget(panel(" Budget ", budget_lines(&data.summary)), budget_area);
```

The `panel(title, lines)` helper and the per-panel line-builders (`session_lines`,
`heartbeat_lines`, `files_lines`, `activity_lines`, `budget_lines`) stay; only their
*positions* change, plus the new `compactions_lines`.

### `files_lines` today (`mcp/src/dashboard.rs`)

```rust
fn files_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.files_changed.is_empty() {
        return vec![Line::from("(no files changed yet)")];
    }
    summary
        .files_changed
        .iter()
        .map(|f| Line::from(format!("  {} +{} -{}", f.path, f.added, f.removed)))
        .collect()
}
```

The error-pane early-return at the top of `render_dashboard` stays unchanged.

## Spec

Numbered tasks in execution order.

1. **Fold `Compaction` into `StatusSummary`** — in `mcp/src/status.rs`:

   a. Add three fields to `StatusSummary` (with doc comments, after the budget
      fields):
      ```rust
          /// Number of `Compaction` records seen so far.
          pub compaction_count: usize,
          /// Sum of `tokens_before` across all `Compaction` records.
          pub compaction_tokens_before: usize,
          /// Sum of `tokens_after` across all `Compaction` records.
          pub compaction_tokens_after: usize,
      ```
   b. Add a match arm **before** the `_ => {}` catch-all:
      ```rust
              SessionEvent::Compaction {
                  tokens_before,
                  tokens_after,
                  ..
              } => {
                  summary.compaction_count += 1;
                  summary.compaction_tokens_before += *tokens_before;
                  summary.compaction_tokens_after += *tokens_after;
              }
      ```
      (`messages_signaturized` / `messages_evicted` are intentionally unread for now —
      the panel shows count + ratio only.)

2. **Add `compactions_lines`** — in `mcp/src/dashboard.rs`, a new pure line-builder
   mirroring the shape of the other `*_lines` functions. Behavior:
   - When `compaction_count == 0`: return `vec![Line::from("(no compactions)")]`.
   - Otherwise three lines:
     - `format!("events: {}", summary.compaction_count)`
     - `format!("freed: {} tokens", before.saturating_sub(after))`
     - the **compression ratio** = `tokens_before / tokens_after`, displayed as
       `format!("ratio: {ratio:.1}x")`. **Guard divide-by-zero:** when
       `compaction_tokens_after == 0`, omit the ratio line (do not divide). Compute
       `ratio` as `before as f64 / after as f64`. A higher ratio means more freed
       (e.g. `before=1000, after=600` → `1.7x`).
   - No color requirement (keep it plain text for this phase).

3. **Add the Compactions panel + restructure the layout** — rewrite the non-error body
   of `render_dashboard` to the wireframe shape. Update the `render_dashboard` doc
   comment to describe the new layout (it currently says "2×2 grid with a full-width
   Budget row").

   - **Outer vertical split** into two regions: a fixed-height header band and a
     filling body.
     ```rust
     let [header, body] =
         Layout::vertical([Constraint::Length(7), Constraint::Min(0)]).areas::<2>(area);
     ```
     (Length 7 = ~5 content lines + borders; tune if a header panel clips.)
   - **Header band** split horizontally into **four** columns, left-to-right:
     **Session · Budget · Compactions · Heartbeat**. Pick reasonable widths — Budget
     and Compactions each need room for their numeric lines; e.g.
     `[Percentage(26), Percentage(20), Percentage(28), Percentage(26)]`, but the exact
     split is yours.
     ```rust
     frame.render_widget(panel(" Session ", session_lines(&data.summary)), session_area);
     frame.render_widget(panel(" Budget ", budget_lines(&data.summary)), budget_area);
     frame.render_widget(panel(" Compactions ", compactions_lines(&data.summary)), compactions_area);
     frame.render_widget(panel(" Heartbeat ", heartbeat_lines(&data.summary, now_ms)), heartbeat_area);
     ```
   - **Body** split horizontally into **Activity (wide-left) · Files (right)** —
     Activity gets the majority, e.g. `[Percentage(72), Percentage(28)]`:
     ```rust
     frame.render_widget(panel(" Activity ", activity_lines(&data.summary)), activity_area);
     frame.render_widget(panel(" Files ", files_lines(&data.summary)), files_area);
     ```
   - `now_ms` still flows only into `heartbeat_lines`. The error-pane early return is
     unchanged.

4. **Left-trim file paths** — in `mcp/src/dashboard.rs`, add a pure helper and use it
   in `files_lines` so a long path shows its tail with a leading ellipsis:
   ```rust
   /// Max display width for a file path in the Files panel. Longer paths are
   /// left-trimmed so the filename (the meaningful tail) stays visible.
   const FILE_PATH_MAX: usize = 40;

   fn trim_path_left(path: &str, max: usize) -> String {
       if path.chars().count() <= max {
           return path.to_string();
       }
       let tail: String = path
           .chars()
           .rev()
           .take(max.saturating_sub(1))
           .collect::<Vec<_>>()
           .into_iter()
           .rev()
           .collect();
       format!("…{tail}")
   }
   ```
   In `files_lines`, replace `f.path` in the format string with
   `trim_path_left(&f.path, FILE_PATH_MAX)`. Use `char` counts (not byte `len()`) so
   multibyte paths don't panic on a slice boundary. Keep the `+{added} -{removed}`
   suffix and the empty-placeholder branch unchanged.

## Acceptance criteria

- [ ] `StatusSummary` has `compaction_count`, `compaction_tokens_before`,
      `compaction_tokens_after`; `summarize` folds `Compaction` into them.
- [ ] `compactions_lines` returns the placeholder when count is 0, and
      events/freed/ratio lines otherwise, with no divide-by-zero when
      `compaction_tokens_after == 0`.
- [ ] `render_dashboard` renders a four-panel header band (Session · Budget ·
      Compactions · Heartbeat) over a body (Activity · Files); no full-width Budget
      row remains; its doc comment describes the new layout.
- [ ] `files_lines` left-trims paths longer than `FILE_PATH_MAX` with a leading `…`
      and leaves shorter paths unchanged.
- [ ] `cargo build` clean; `cargo clippy --all-targets --all-features -- -D warnings`
      clean; `cargo fmt --all --check` clean (use `rustfmt` only on touched files; do
      **not** run the writing form of `cargo fmt --all`); `cargo test -p rexymcp`
      passes (existing + new).

## Test plan

Add to the existing `#[cfg(test)] mod tests` blocks (`status.rs` for the fold,
`dashboard.rs` for the panels). Follow the existing `*_lines` test style (build a
`StatusSummary { … , ..StatusSummary::default() }`, render lines, assert on
`format!("{l}")` text). For the `Compaction` event constructor in `status.rs` tests,
mirror the existing `metrics(...)` helper shape but for the new variant.

- `summarize_folds_compaction_counts_and_tokens` (status.rs) — feed two `Compaction`
  records (e.g. before/after 1000/600 and 800/500); assert `compaction_count == 2`,
  `compaction_tokens_before == 1800`, `compaction_tokens_after == 1100`.
- `compactions_lines_empty_placeholder` (dashboard.rs) — default summary →
  contains "no compactions".
- `compactions_lines_shows_events_and_ratio` (dashboard.rs) — `compaction_count = 2`,
  before `1000`, after `600` → lines contain "events: 2", "freed: 400", and "1.7x".
- `compactions_lines_omits_ratio_when_after_zero` (dashboard.rs) — `compaction_count
  = 1`, before `500`, after `0` → has "events: 1" and "freed: 500" but **no** "x"
  ratio line (the must-NOT divide-by-zero negative case).
- `files_lines_trims_long_path_left` (dashboard.rs) — a `FileNumstat` whose path is
  longer than `FILE_PATH_MAX` → the rendered line starts the path with `…`, ends with
  the original path's tail, and the trimmed path is `FILE_PATH_MAX` chars.
- `files_lines_keeps_short_path_untrimmed` (dashboard.rs) — a short path (e.g.
  `src/a.rs`) → rendered unchanged, **no** `…` (the negative case).

Keep the existing panel/summarize tests green — they assert content of unchanged
line-builders and are layout-agnostic.

## End-to-end verification

`render_dashboard`'s layout is not unit-testable (it needs a terminal); it is reviewed
by inspection per the M8 hermetic-data-layer decision. The data-backed parts are
proven by the unit tests above. Verify:

1. The new layout by inspection of `render_dashboard` (four header panels + body), and
   paste the `cargo test -p rexymcp` output covering the new `compactions_*` and
   `files_lines_*` tests.
2. Build the binary and launch it against this repo to confirm the redesigned panels
   render without panicking and the Compactions panel appears:
   `cargo run -p rexymcp -- dashboard --repo .` — observe the four-panel header band,
   then quit with `q`. Quote what you observed in one line (panels present, no panic).
   (If no session log exists, the error pane is expected — note that instead.)

## Authorizations

None. No new dependency. No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md`
edit. mcp-crate only.

## Out of scope

- **The scrollable Activity transcript** (raw-record history, JSON pretty-printing,
  color, tool-output rendering, scroll keys). This phase keeps the existing
  `activity_lines` signal content in the new wide Activity position. The transcript is
  the next phase (phase-10).
- **Budget Tokens/Sec and "$ saved".** `budget_lines` stays as today (tokens in/out +
  context %). Those metrics need new data / a pricing decision and are a later phase
  (phase-11). Do not add them.
- **Any interactivity beyond the existing `q`/`Esc` quit.** No scrolling, no focus, no
  key handling changes in `run_loop`.
- **Color in the Compactions panel.** Plain text for this phase.
- **Reading `messages_signaturized` / `messages_evicted`** from `Compaction`. Count +
  token ratio only.
- **Any executor-crate change.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
