# Phase 05b: split `dashboard/mod.rs` — extract `panels`, `render`, `event_loop`

**Milestone:** M9 — Executor runtime hardening
**Status:** todo
**Depends on:** phase-05a
**Estimated diff:** ~600 lines (new files) + ~700 lines deleted from mod.rs
**Tags:** language=rust, kind=refactor, size=l

## Goal

Continue the `dashboard/` structural split started in phase-05a. Extract the
remaining three groups — panel content formatters, the renderer, and the event
loop — into `panels.rs`, `render.rs`, and `event_loop.rs`. After this phase,
`mod.rs` contains only the public API (~80 lines) and `load_data` tests. No
logic or behavior changes; all tests pass unchanged.

## Architecture references

Read before starting:

- `docs/architecture.md` — dashboard is the live TUI; this is a structural
  refactor only.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm `git status` is clean.
4. Confirm phase-05a is committed and `cargo test` passes. Capture test count.

## Current state

After phase-05a, `mcp/src/dashboard/mod.rs` contains:

| Lines (approx.) | Content → target module |
|-----------------|------------------------|
| 1–56 | imports, `mod` decls, `use` statements, `DashboardData`, `load_data` |
| ~57–100 | `clamp_scroll`, `visible_offset` → `render.rs` |
| ~100–350 | `session_lines`, `compactions_lines`, `files_lines`, `trim_path_left`, `tokens_per_sec`, `budget_lines`, `BudgetRates`, `dollars_saved`, `dollars_saved_line`, `panel` → `panels.rs` |
| ~350–480 | `ViewState`, `render_dashboard` → `render.rs` |
| ~480–590 | `run_dashboard`, `run_loop` → `event_loop.rs`; `run_dashboard` stays in `mod.rs` |
| ~590–650 | `mod tests` with `load_data_*` tests (stays in `mod.rs`) |

**Exact line numbers in phase-05a's `mod.rs` output:**
Read `mcp/src/dashboard/mod.rs` to verify current line numbers before
patching. The file is ~650 lines after phase-05a.

`main.rs` still uses `dashboard::BudgetRates` and `dashboard::run_dashboard` —
both must remain `pub` in `mod.rs`. `run_dashboard` stays in `mod.rs` as the
public entry point; it calls `event_loop::run_loop` internally.

## Spec

Tasks run in this exact order.

### Task 1 — Create `mcp/src/dashboard/panels.rs`

Read `mod.rs` lines covering `session_lines` through `panel` (the `// --- Panel
helpers ---` section). The skeleton with read instructions:

```rust
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::status::StatusSummary;
use crate::status;

pub(crate) const FILE_LINE_MAX: usize = 28;

/// Cloud-baseline $/Mtok rates for the Budget panel's "$ saved" line.
#[derive(Debug, Clone, Copy, Default)]
pub struct BudgetRates {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

// ── read from mod.rs: session_lines fn ──────────────────────────────────────
/// Session panel: phase / session / model / state / turn / stage / freshness.
pub(crate) fn session_lines(summary: &StatusSummary, now_ms: u64) -> Vec<Line<'static>> {
    // paste verbatim
}

// ── read from mod.rs: compactions_lines fn ──────────────────────────────────
pub(crate) fn compactions_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    // paste verbatim
}

// ── read from mod.rs: files_lines + trim_path_left ──────────────────────────
pub(crate) fn files_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    // paste verbatim
}

fn trim_path_left(path: &str, max: usize) -> String {
    // paste verbatim
}

// ── read from mod.rs: tokens_per_sec ────────────────────────────────────────
pub(crate) fn tokens_per_sec(
    prev_ts: Option<u64>,
    prev_out: Option<u32>,
    last_ts: Option<u64>,
    last_out: Option<u32>,
) -> Option<f64> {
    // paste verbatim
}

// ── read from mod.rs: budget_lines ──────────────────────────────────────────
pub(crate) fn budget_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    // paste verbatim (references tokens_per_sec in same file ✓)
}

// ── read from mod.rs: dollars_saved + dollars_saved_line ────────────────────
fn dollars_saved(
    input_tokens: u32,
    output_tokens: u32,
    in_per_mtok: f64,
    out_per_mtok: f64,
) -> f64 {
    // paste verbatim
}

pub(crate) fn dollars_saved_line(summary: &StatusSummary, rates: BudgetRates) -> Option<Line<'static>> {
    // paste verbatim
}

// ── read from mod.rs: panel helper ──────────────────────────────────────────
/// Wrap lines in a bordered `Block` with the given title.
pub(crate) fn panel(title: &'static str, lines: Vec<Line<'static>>) -> Paragraph<'static> {
    // paste verbatim
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::sessions::event::FileNumstat;

    // paste verbatim from mod.rs tests: session_lines_*, files_lines_*,
    // budget_lines_*, tokens_per_sec_*, compactions_lines_*,
    // dollars_saved_* tests
}
```

**Move `BudgetRates` to `panels.rs`** (it currently lives in `mod.rs`).
In `mod.rs`, re-export it:
```rust
pub use panels::BudgetRates;
```

**Visibility:**
- `BudgetRates` → `pub` (public API re-exported from `mod.rs`)
- `session_lines`, `compactions_lines`, `files_lines`, `budget_lines`,
  `dollars_saved_line`, `panel`, `tokens_per_sec` → `pub(crate)`
- `trim_path_left`, `dollars_saved`, `FILE_LINE_MAX` const → private
  (`files_lines` uses `FILE_LINE_MAX` in the same file; no external access needed)

Wait — `FILE_LINE_MAX` is referenced by tests in `panels.rs`. Keep it `pub(crate)` if
tests reference it; otherwise leave private.

### Task 2 — Create `mcp/src/dashboard/render.rs`

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::filter::{ActivityFilter, FilterState, FILTER_ITEM_COUNT};
use super::panels::{BudgetRates, budget_lines, compactions_lines, dollars_saved_line,
    files_lines, panel, session_lines};
use super::transcript::transcript_lines;
use crate::dashboard::DashboardData;

/// View-state for the dashboard activity pane.
pub(crate) struct ViewState {
    pub(crate) offset: u16,
    pub(crate) follow: bool,
    pub(crate) spinner: Option<usize>,
    pub(crate) filter: FilterState,
}

// ── body from mod.rs: clamp_scroll ──────────────────────────────────────────
/// Clamp a scroll offset so it can't run past the last line.
pub(crate) fn clamp_scroll(scroll: u16, total_lines: usize) -> u16 {
    // paste verbatim
}

// ── body from mod.rs: visible_offset ────────────────────────────────────────
/// Resolve the scroll offset to display.
pub(crate) fn visible_offset(follow: bool, offset: u16, total_lines: usize, viewport: u16) -> u16 {
    // paste verbatim
}

// ── body from mod.rs: render_dashboard ──────────────────────────────────────
pub(crate) fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    state: &ViewState,
    rates: BudgetRates,
) {
    // paste verbatim
}

#[cfg(test)]
mod tests {
    use super::*;

    // paste verbatim from mod.rs tests: visible_offset_*, clamp_scroll_* tests
}
```

**Visibility:** `ViewState` and all its fields → `pub(crate)`.
`clamp_scroll`, `visible_offset`, `render_dashboard` → `pub(crate)`.

### Task 3 — Create `mcp/src/dashboard/event_loop.rs`

```rust
use std::path::Path;

use super::filter::{FilterState, FILTER_ITEM_COUNT};
use super::render::{ViewState, clamp_scroll, render_dashboard};
use super::transcript::{transcript_lines, SPINNER_FRAMES};
use super::panels::BudgetRates;
use crate::dashboard::load_data;

// ── body from mod.rs: run_loop ───────────────────────────────────────────────
pub(crate) fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    // paste verbatim
}
```

No tests (run_loop is an I/O-bound event loop; not unit-testable without a
terminal mock, and no test existed before).

### Task 4 — Patch `mod.rs` to the final public-API-only form

After the three new files exist, patch `mod.rs`:

**Step 4a — Update `mod` declarations and `use` statements.**

Replace the current import + mod block at the top of `mod.rs` with:

```rust
//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Continuously refreshes a `ratatui` terminal with a header band (Session ·
//! Budget · Compactions) above a body (Activity · Files).

use std::path::Path;

use rexymcp_executor::store::sessions::event::SessionRecord;

use crate::status::{self, StatusSummary};

mod event_loop;
mod filter;
mod highlight;
mod panels;
mod render;
mod transcript;

pub use panels::BudgetRates;
```

Remove the `use filter::…`, `use transcript::…` lines (they were needed when
render_dashboard and run_loop were inline; now those live in their own modules).

**Step 4b — Delete extracted code from `mod.rs` (bottom-up).**

**Deletion A — remove all panel formatters + scroll helpers + renderer
(from `// --- Per-panel content formatters` comment through the closing `}`
of `render_dashboard`).**

old_str anchor (top):
```
// --- Per-panel content formatters (pure, testable) ---

/// Session panel: phase / session / model / state / turn / stage / freshness.
```

Delete from here through the closing `}` of `render_dashboard`.

**Deletion B — remove `run_loop` (keep `run_dashboard`).**

old_str anchor:
```
fn run_loop(
    terminal: &mut ratatui::DefaultTerminal,
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
```

Delete from here through the closing `}` of `run_loop`.

**Step 4c — Replace `run_dashboard` body to delegate to `event_loop::run_loop`.**

old_str:
```rust
pub fn run_dashboard(
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, repo, session, rates);
    ratatui::restore();
    result
}
```

new_str:
```rust
pub fn run_dashboard(
    repo: &Path,
    session: Option<&str>,
    rates: BudgetRates,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop::run_loop(&mut terminal, repo, session, rates);
    ratatui::restore();
    result
}
```

**Step 4d — Confirm `mod.rs` now contains only:**
- The module doc comment + imports (~15 lines)
- Six `mod` declarations
- `pub use panels::BudgetRates`
- `DashboardData` struct + `load_data` fn (~25 lines)
- `run_dashboard` fn (~7 lines)
- `#[cfg(test)] mod tests` with `load_data_*` tests (~60 lines)

Total: ~110 lines.

### Task 5 — Fix cross-module references

Common issues to check and fix:

1. `render_dashboard` (now in `render.rs`) references `ActivityFilter` for the
   filter panel rendering. `render.rs` imports it via `super::filter::ActivityFilter`. ✓

2. `run_loop` (now in `event_loop.rs`) uses `FILTER_ITEM_COUNT` for cursor
   wrapping. `event_loop.rs` imports it via `super::filter::FILTER_ITEM_COUNT`. ✓

3. `budget_lines` (now in `panels.rs`) calls `tokens_per_sec` which is in the
   same file — no import needed. ✓

4. `dollars_saved_line` uses `dollars_saved` (same file) and `BudgetRates`
   (same file). ✓

5. `render_dashboard` builds the `transcript` via `transcript_lines` — imported
   in `render.rs` via `super::transcript::transcript_lines`. ✓

6. `session_lines` calls `status::humanize_age` — `panels.rs` must import it:
   `use crate::status;`. ✓ (already in the skeleton above)

### Task 6 — Build and test

```bash
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo test
```

Run each command separately (STANDARDS.md §4). All must pass.

## Acceptance criteria

- [ ] `mcp/src/dashboard/panels.rs`, `render.rs`, `event_loop.rs` each exist.
- [ ] `mcp/src/dashboard/mod.rs` is ≤120 lines (public API + load_data + tests).
- [ ] `dashboard::BudgetRates` is accessible from `main.rs` (re-exported via `pub use panels::BudgetRates`).
- [ ] `dashboard::run_dashboard` is accessible from `main.rs`.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes with the same count as the phase-05a baseline.
- [ ] No panel formatter, scroll helper, renderer, or event-loop function definitions remain in `mod.rs`.

## Test plan

No new tests. Existing tests relocate:

- `session_lines_*`, `files_lines_*`, `budget_lines_*`, `tokens_per_sec_*`,
  `compactions_lines_*`, `dollars_saved_*` → `mcp/src/dashboard/panels.rs` `mod tests`
- `visible_offset_*`, `clamp_scroll_*` → `mcp/src/dashboard/render.rs` `mod tests`
- `load_data_*` stays in `mod.rs` `mod tests`

## End-to-end verification

Not applicable — pure structural refactor; the binary entry points and public API
are unchanged. Confirm `rexymcp dashboard --help` still shows the expected usage
(the CLI arg parsing in `main.rs` is unchanged).

## Authorizations

None. No new dependencies. No `docs/architecture.md` changes.

## Out of scope

- Do not add any new functionality to any of the new modules.
- Do not move `load_data` tests out of `mod.rs`.
- Do not change `main.rs`.
- Do not modify existing test assertions.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
