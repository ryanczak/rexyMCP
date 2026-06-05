# Phase 05a: split `dashboard/mod.rs` — extract `filter`, `highlight`, `transcript`

**Milestone:** M9 — Executor runtime hardening
**Status:** in-progress
**Depends on:** phase-04
**Estimated diff:** ~650 lines (new files created) + ~600 lines deleted from mod.rs
**Tags:** language=rust, kind=refactor, size=l

## Goal

`mcp/src/dashboard.rs` (2 098 lines) is too large to navigate safely. This phase
converts it to a `dashboard/` module directory and extracts three focused
submodules — `filter`, `highlight`, and `transcript` — leaving ~1 000 lines in
`mod.rs` for phase-05b to handle. No logic or behavior changes; all existing
tests move to their home modules and continue to pass.

## Architecture references

Read before starting:

- `docs/architecture.md` — the dashboard is the live TUI; this is a pure
  structural refactor, not a behavior change.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm `git status` is clean; stash or commit any ambient changes first.
4. Confirm `cargo test` passes before you start. Capture the test count.

## Current state

`mcp/src/dashboard.rs` is a single 2 098-line flat file. The logical sections
this phase extracts:

| Lines (approx.) | Content → target module |
|-----------------|------------------------|
| 24–33 | syntect statics + accessors → `highlight.rs` |
| 58–68 | four constants → split across new modules |
| 70–181 | `ActivityFilter`, `FilterState` → `filter.rs` |
| 183–354 | `detect_syntax`, `is_diff_content`, `diff_body_lines`, `highlighted_body_lines`, `plain_body_lines`, `body_lines` → `highlight.rs` |
| 356–529 | `transcript_lines`, `record_lines`, `preview` → `transcript.rs` |
| 1557–1801 | transcript + record_lines tests → `transcript.rs` |
| 1803–1931 | highlight tests → `highlight.rs` |
| 2008–2097 | filter tests → `filter.rs` |

Everything else (scroll helpers, panel formatters, renderer, event loop, and the
remaining tests) stays in `mod.rs` for phase-05b.

`main.rs` line 7 uses `mod dashboard;` — this declaration is unchanged. Rust
resolves both `src/dashboard.rs` and `src/dashboard/mod.rs` via the same
declaration, so no `main.rs` edits are needed.

Public API (`BudgetRates`, `run_dashboard`) stays in `mod.rs`; `main.rs` call
sites are unaffected.

## Spec

Tasks run in this exact order. Do **not** create `panels.rs`, `render.rs`, or
`event_loop.rs`; those are phase-05b.

**Read strategy:** `mcp/src/dashboard.rs` is capped at 500 lines per read.
Read it in targeted ranges using `start_line`/`end_line`. The file shells below
tell you which ranges to read; you should need at most 6–8 targeted reads total.

### Task 1 — Rename `dashboard.rs` → `dashboard/mod.rs`

```bash
mkdir -p mcp/src/dashboard
git mv mcp/src/dashboard.rs mcp/src/dashboard/mod.rs
```

Verify the file exists at `mcp/src/dashboard/mod.rs` before proceeding.
`main.rs` does not change.

### Task 2 — Create `mcp/src/dashboard/filter.rs`

The complete file content is pre-injected below. Write it exactly; do not read
`mod.rs` for this task.

```rust
use rexymcp_executor::store::sessions::event::SessionEvent;

pub(crate) const FILTER_ITEM_COUNT: usize = 11;

/// Per-event-type visibility toggles for the Activity pane.
/// All enabled by default except `progress` (too noisy).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ActivityFilter {
    pub(crate) session: bool,
    pub(crate) prompt: bool,
    pub(crate) completion: bool,
    pub(crate) tool_call: bool,
    pub(crate) parse_failed: bool,
    pub(crate) tool_result: bool,
    pub(crate) verify: bool,
    pub(crate) hard_fail: bool,
    pub(crate) progress: bool,
    pub(crate) metrics: bool,
    pub(crate) compaction: bool,
}

impl Default for ActivityFilter {
    fn default() -> Self {
        Self {
            session: true,
            prompt: true,
            completion: true,
            tool_call: true,
            parse_failed: true,
            tool_result: true,
            verify: true,
            hard_fail: true,
            progress: false,
            metrics: true,
            compaction: true,
        }
    }
}

impl ActivityFilter {
    pub(crate) fn allows(&self, event: &SessionEvent) -> bool {
        match event {
            SessionEvent::SessionStart { .. } | SessionEvent::SessionEnd { .. } => self.session,
            SessionEvent::Prompt { .. } => self.prompt,
            SessionEvent::Completion { .. } => self.completion,
            SessionEvent::Parsed { .. } => self.tool_call,
            SessionEvent::ParseFailed { .. } => self.parse_failed,
            SessionEvent::ToolResult { .. } => self.tool_result,
            SessionEvent::Verify { .. } => self.verify,
            SessionEvent::HardFail { .. } => self.hard_fail,
            SessionEvent::Progress { .. } => self.progress,
            SessionEvent::Metrics { .. } => self.metrics,
            SessionEvent::Compaction { .. } => self.compaction,
        }
    }

    pub(crate) fn toggle(&mut self, index: usize) {
        match index {
            0 => self.session = !self.session,
            1 => self.prompt = !self.prompt,
            2 => self.completion = !self.completion,
            3 => self.tool_call = !self.tool_call,
            4 => self.parse_failed = !self.parse_failed,
            5 => self.tool_result = !self.tool_result,
            6 => self.verify = !self.verify,
            7 => self.hard_fail = !self.hard_fail,
            8 => self.progress = !self.progress,
            9 => self.metrics = !self.metrics,
            10 => self.compaction = !self.compaction,
            _ => {}
        }
    }

    pub(crate) fn is_enabled(&self, index: usize) -> bool {
        match index {
            0 => self.session,
            1 => self.prompt,
            2 => self.completion,
            3 => self.tool_call,
            4 => self.parse_failed,
            5 => self.tool_result,
            6 => self.verify,
            7 => self.hard_fail,
            8 => self.progress,
            9 => self.metrics,
            10 => self.compaction,
            _ => false,
        }
    }

    pub(crate) fn item_label(index: usize) -> &'static str {
        match index {
            0 => "session start/end",
            1 => "prompt",
            2 => "completion",
            3 => "tool call",
            4 => "parse fail",
            5 => "tool result",
            6 => "verify",
            7 => "hard fail",
            8 => "progress",
            9 => "metrics",
            10 => "compaction",
            _ => "?",
        }
    }
}

/// Filter panel UI state — open/closed, cursor position, current settings.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilterState {
    pub(crate) open: bool,
    pub(crate) cursor: usize,
    pub(crate) filter: ActivityFilter,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

    fn rec(ts: u64, turn: usize, event: SessionEvent) -> SessionRecord {
        SessionRecord { ts, turn, event }
    }

    fn progress_event(turn: usize, stage: &str) -> SessionEvent {
        SessionEvent::Progress {
            turn,
            stage: stage.into(),
            files_changed: vec![],
            message: format!("turn={turn} stage={stage} +0/-0 files=0"),
        }
    }

    #[test]
    fn filter_default_disables_progress() {
        let f = ActivityFilter::default();
        assert!(!f.progress, "progress should be disabled by default");
        assert!(f.session);
        assert!(f.prompt);
        assert!(f.completion);
        assert!(f.tool_call);
        assert!(f.parse_failed);
        assert!(f.tool_result);
        assert!(f.verify);
        assert!(f.hard_fail);
        assert!(f.metrics);
        assert!(f.compaction);
    }

    #[test]
    fn filter_allows_progress_when_enabled() {
        let f = ActivityFilter {
            progress: true,
            ..Default::default()
        };
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_blocks_progress_by_default() {
        let f = ActivityFilter::default();
        let progress_rec = rec(100, 4, progress_event(4, "verify"));
        assert!(!f.allows(&progress_rec.event));
    }

    #[test]
    fn filter_toggle_flips_field() {
        let mut f = ActivityFilter::default();
        assert!(!f.progress);
        f.toggle(8);
        assert!(f.progress);
        f.toggle(8);
        assert!(!f.progress);
    }

    #[test]
    fn filter_cursor_wraps_forward() {
        let mut fs = FilterState::default();
        fs.cursor = FILTER_ITEM_COUNT - 1;
        fs.cursor = (fs.cursor + 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, 0);
    }

    #[test]
    fn filter_cursor_wraps_backward() {
        let mut fs = FilterState::default();
        fs.cursor = 0;
        fs.cursor = (fs.cursor + FILTER_ITEM_COUNT - 1) % FILTER_ITEM_COUNT;
        assert_eq!(fs.cursor, FILTER_ITEM_COUNT - 1);
    }
}
```

**Visibility notes:** The original `ActivityFilter` fields had no `pub` — they
were private to `mod tests` within the same file. After extraction, the tests
live inside `filter.rs`'s own `mod tests` block, where they retain private-field
access. The `pub(crate)` annotations on the fields are required because
`render_dashboard` (in `mod.rs`) accesses `filter_state.filter` (a `FilterState`
field) and `run_loop` mutates `filter_state.cursor` — all cross-module.

### Task 3 — Create `mcp/src/dashboard/highlight.rs`

Write the file with this skeleton. For each function body, read the indicated
range from `mod.rs` and paste verbatim.

```rust
use std::sync::OnceLock;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

pub(crate) fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Max content lines shown per record before collapsing the rest.
pub(crate) const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

// ── body from mod.rs:185–230 (detect_syntax) ──────────────────────────────
/// Detect a syntax definition from content alone (no filename available).
/// Returns `None` when no language can be confidently identified.
fn detect_syntax<'a>(content: &str, ss: &'a SyntaxSet) -> Option<&'a SyntaxReference> {
    // paste body verbatim from mod.rs lines 186–230
}

// ── body from mod.rs:232–245 (is_diff_content) ────────────────────────────
/// True when `content` looks like unified diff output.
fn is_diff_content(content: &str) -> bool {
    // paste body verbatim from mod.rs lines 233–245
}

// ── body from mod.rs:247–291 (diff_body_lines) ────────────────────────────
/// Render unified diff content with line-level background colors.
fn diff_body_lines(content: &str) -> Vec<Line<'static>> {
    // paste body verbatim from mod.rs lines 248–291
}

// ── body from mod.rs:293–346 (highlighted_body_lines) ─────────────────────
/// Render `content` as indented, syntax-highlighted lines.
pub(crate) fn highlighted_body_lines(content: &str) -> Vec<Line<'static>> {
    // paste body verbatim from mod.rs lines 294–346
}

// ── body from mod.rs:348–354 (plain_body_lines) ───────────────────────────
/// Render `content` as indented lines, all in the same `color`.
pub(crate) fn plain_body_lines(content: &str, color: Color) -> Vec<Line<'static>> {
    // paste body verbatim from mod.rs lines 349–354
}

// ── body from mod.rs:379–397 (body_lines) ─────────────────────────────────
/// Split `body` on newlines into indented display lines.
pub(crate) fn body_lines(body: &str) -> Vec<String> {
    // paste body verbatim from mod.rs lines 380–397
}

#[cfg(test)]
mod tests {
    use super::*;

    // paste tests verbatim from mod.rs lines 1803–1931
    // (detect_syntax, highlighted_body_lines, diff highlighting tests)
}
```

**Visibility:** `highlighted_body_lines`, `plain_body_lines`, `body_lines`,
`syntax_set`, `TRANSCRIPT_CONTENT_MAX_LINES` → `pub(crate)`.
`detect_syntax`, `is_diff_content`, `diff_body_lines`, `theme_set` → private.

Note: `highlighted_body_lines` calls `body_lines` and `is_diff_content` —
all in the same file, no import needed.

### Task 4 — Create `mcp/src/dashboard/transcript.rs`

```rust
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

use super::filter::ActivityFilter;
use super::highlight::{highlighted_body_lines, plain_body_lines};

/// Max chars of free-text content shown per transcript line.
pub(crate) const TRANSCRIPT_PREVIEW_MAX: usize = 100;

pub(crate) const SPINNER_FRAMES: &[&str] =
    &["🐾", "🐾🐾", "🐾🐾🐾", "🐾🐾🐾🐾", "🐾🐾🐾", "🐾🐾", "🐾"];

// ── body from mod.rs:359–377 (transcript_lines) ───────────────────────────
/// Build all transcript lines for the given records, in chronological order.
pub(crate) fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
    spinner: Option<usize>,
) -> Vec<Line<'static>> {
    // paste body verbatim from mod.rs lines 360–377
}

// ── body from mod.rs:399–509 (record_lines) ───────────────────────────────
/// Render one record as one or more transcript lines.
pub(crate) fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>> {
    // paste body verbatim from mod.rs lines 400–509
}

// ── body from mod.rs:511–529 (preview) ────────────────────────────────────
/// Replace newlines/tabs with spaces and truncate to `TRANSCRIPT_PREVIEW_MAX`.
fn preview(s: &str) -> String {
    // paste body verbatim from mod.rs lines 512–529
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::filter::ActivityFilter;
    use crate::status::sessions_dir;
    use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};
    use tempfile::TempDir;

    fn rec(ts: u64, turn: usize, event: SessionEvent) -> SessionRecord {
        SessionRecord { ts, turn, event }
    }

    fn start_event() -> SessionEvent {
        SessionEvent::SessionStart {
            session_id: "test-session".into(),
            model: "test-model".into(),
            phase: "phase-01".into(),
        }
    }

    fn progress_event(turn: usize, stage: &str) -> SessionEvent {
        SessionEvent::Progress {
            turn,
            stage: stage.into(),
            files_changed: vec![],
            message: format!("turn={turn} stage={stage} +0/-0 files=0"),
        }
    }

    fn record_text(r: &SessionRecord) -> String {
        record_lines(r)
            .iter()
            .map(|l| format!("{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // paste tests verbatim from mod.rs lines 1557–1801
    // (transcript_lines_*, record_lines_*, spinner_*)
}
```

**Visibility:** `transcript_lines`, `record_lines`, `SPINNER_FRAMES`,
`TRANSCRIPT_PREVIEW_MAX` → `pub(crate)`. `preview` → private.

`transcript.rs` imports from sibling modules via `super::filter` and
`super::highlight`. The test block uses `crate::dashboard::filter::ActivityFilter`
(absolute path from inside the test module).

`record_lines` references `TRANSCRIPT_CONTENT_MAX_LINES` (now in `highlight.rs`)
and `SPINNER_FRAMES` (in the same file). Add `use super::highlight::TRANSCRIPT_CONTENT_MAX_LINES;`
inside `record_lines` — or import it at the top of `transcript.rs`:

```rust
use super::highlight::TRANSCRIPT_CONTENT_MAX_LINES;
```

### Task 5 — Patch `mod.rs`: add submodule declarations and update imports

The current `mod.rs` starts with (lines 1–23):

```rust
//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Continuously refreshes a `ratatui` terminal with a header band (Session ·
//! Budget · Compactions) above a body (Activity · Files).

use std::path::Path;
use std::sync::OnceLock;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};

use rexymcp_executor::store::sessions::event::{SessionEvent, SessionRecord};

use crate::status::{self, StatusSummary};
```

Replace that entire block with:

```rust
//! Live dashboard — polls session logs and renders a paned TUI summary.
//!
//! Continuously refreshes a `ratatui` terminal with a header band (Session ·
//! Budget · Compactions) above a body (Activity · Files).

use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use rexymcp_executor::store::sessions::event::SessionRecord;

use crate::status::{self, StatusSummary};

mod filter;
mod highlight;
mod transcript;

use filter::{ActivityFilter, FilterState, FILTER_ITEM_COUNT};
use transcript::{transcript_lines, SPINNER_FRAMES};
```

Removed: `std::sync::OnceLock`, syntect imports, `SessionEvent` (no longer
needed in `mod.rs`; `SessionRecord` still is for `DashboardData`).

Added: three `mod` declarations + the cross-module `use` statements that the
remaining inline code (`render_dashboard`, `run_loop`) needs.

### Task 6 — Delete extracted code from `mod.rs`

Apply these patch_file deletions **in order from bottom to top** to avoid
shifting line numbers.

**Deletion A — filter tests (mod.rs lines 2008–2097, inside `mod tests`)**

old_str (unique anchor):
```
    // --- Activity filter tests ---

    #[test]
    fn filter_default_disables_progress() {
```

new_str: _(empty — delete this block and everything through line 2097 which is
`    }\n}`)_

The closing `}` of the test module stays (it closes the remaining test block).
The new end of `mod tests` ends after `filter_cursor_wraps_backward`.

Use this anchor to find the start; delete from it through the final `}\n}` of
the file.

**Deletion B — highlight + diff tests (mod.rs lines 1803–1931, inside `mod tests`)**

old_str anchor:
```
    // --- detect_syntax / highlighted_body_lines tests ---

    #[test]
    fn detect_syntax_identifies_json() {
```

Delete from this line through the closing `}` of `highlighted_body_lines_routes_diff_to_diff_renderer`.

**Deletion C — transcript / record_lines tests (mod.rs lines 1557–1801, inside `mod tests`)**

old_str anchor:
```
    // --- transcript_lines tests ---

    #[test]
    fn transcript_lines_empty_placeholder() {
```

Delete from this line through the closing `}` of `transcript_lines_all_filtered_shows_placeholder`.
Spinner tests (lines 1765–1801) are part of this block — delete them too.

**Deletion D — `body_lines`, `plain_body_lines`, `transcript_lines`, `record_lines`, `preview`
(mod.rs lines 348–529)**

old_str anchor (top of the block):
```
/// Render `content` as indented lines, all in the same `color`.
fn plain_body_lines(content: &str, color: Color) -> Vec<Line<'static>> {
```

Delete from here through the closing `}` of `preview` (the function whose last
line is `    result.push('…');\n    result\n}`).

**Deletion E — `transcript_lines` declaration header comment (mod.rs ~lines 356–378)**

After Deletion D removes `plain_body_lines` through `preview`, the
`transcript_lines` function is already gone. Confirm `body_lines` is also gone
(it was between `preview` and `highlighted_body_lines` in source order:
`plain_body_lines` at 348, `transcript_lines` at 356, `body_lines` at 379 —
all covered by Deletion D which starts at `plain_body_lines`).

**Deletion F — `detect_syntax` through `plain_body_lines` (mod.rs lines 183–354)**

old_str anchor:
```
/// Detect a syntax definition from content alone (no filename available).
/// Returns `None` when no language can be confidently identified, which
/// causes the caller to fall back to unstyled DarkGray text.
fn detect_syntax<'a>(content: &str, ss: &'a SyntaxSet) -> Option<&'a SyntaxReference> {
```

Delete from here through the closing `}` of the `highlighted_body_lines`
function (its last line is `    result\n}`). This covers lines 183–346.

**Deletion G — `ActivityFilter` + `FilterState` (mod.rs lines 70–181)**

old_str anchor:
```
const FILTER_ITEM_COUNT: usize = 11;

/// Per-event-type visibility toggles for the Activity pane.
/// All enabled by default except `progress` (too noisy).
```

Delete from here through the closing `}` of the `FilterState` struct definition.

**Deletion H — four constants (mod.rs lines 58–68)**

old_str (exact):
```
/// Max chars of free-text content shown per transcript line in 10a (10b expands
/// to full multi-line). Keeps one record = one line.
const TRANSCRIPT_PREVIEW_MAX: usize = 100;

/// Max content lines shown per record before collapsing the rest into a
/// "… (N more lines)" marker. Keeps one large tool output from flooding the panel.
const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

const SPINNER_FRAMES: &[&str] = &["🐾", "🐾🐾", "🐾🐾🐾", "🐾🐾🐾🐾", "🐾🐾🐾", "🐾🐾", "🐾"];

const FILTER_ITEM_COUNT: usize = 11;
```

new_str: _(empty)_

**Deletion I — syntect statics (mod.rs lines 24–33, after the import patch
from Task 5 removes the syntect `use` lines)**

After Task 5 replaces the import block, the statics at the top of the file
are gone as part of the replacement. Confirm they were removed by Task 5.

### Task 7 — Verify the build is green

```bash
cargo build
cargo clippy --all-targets --all-features -- -D warnings
```

Fix any import errors or visibility issues before running tests. Common issues:
- `ActivityFilter` or `FilterState` fields not `pub(crate)` → add the annotation in `filter.rs`
- `SessionEvent` still imported in `mod.rs` but no longer used → remove from import
- `TRANSCRIPT_CONTENT_MAX_LINES` missing in `transcript.rs` → add `use super::highlight::TRANSCRIPT_CONTENT_MAX_LINES;`

### Task 8 — Run the full test suite

```bash
cargo test
```

All tests that existed before this phase must still pass. The count should be
unchanged (no new tests added, no tests deleted — only relocated). Confirm by
comparing the count to the pre-flight baseline.

## Acceptance criteria

- [ ] `mcp/src/dashboard.rs` no longer exists; `mcp/src/dashboard/mod.rs` exists.
- [ ] `mcp/src/dashboard/filter.rs`, `highlight.rs`, `transcript.rs` each exist.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes with the same test count as pre-flight.
- [ ] No `ActivityFilter`, `FilterState`, `detect_syntax`, `transcript_lines`,
  `record_lines` definitions remain in `mod.rs`.
- [ ] `main.rs` is unchanged.

## Test plan

No new tests. Existing tests relocate:

- `filter_*` and `filter_cursor_*` tests → `mcp/src/dashboard/filter.rs` `mod tests`
- `detect_syntax_*`, `highlighted_body_lines_*`, `is_diff_content_*`, `diff_body_lines_*` → `mcp/src/dashboard/highlight.rs` `mod tests`
- `transcript_lines_*`, `record_lines_*`, `spinner_*` → `mcp/src/dashboard/transcript.rs` `mod tests`
- `load_data_*` tests stay in `mod.rs` `mod tests`

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. This is a pure
module-boundary reorganization; behavior and the compiled binary are identical.
Confirm with `cargo build` producing the same binary entry points.

## Authorizations

None. No new dependencies. No `docs/architecture.md` changes.

## Out of scope

- Do not extract `panels.rs`, `render.rs`, or `event_loop.rs` — those are phase-05b.
- Do not move or modify `BudgetRates`, `DashboardData`, `load_data`,
  `run_dashboard`, panel formatters (`session_lines`, `budget_lines`, etc.),
  `render_dashboard`, or `run_loop`.
- Do not change any test assertions — only move test functions.
- Do not add `#[allow]` annotations to silence warnings; fix the root cause.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
