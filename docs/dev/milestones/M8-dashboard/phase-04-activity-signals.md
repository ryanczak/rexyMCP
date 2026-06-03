# Phase 04: surface parse / verify / tool signals — the Activity panel

**Milestone:** M8 — Live session dashboard
**Status:** todo
**Depends on:** phase-02 (done) — extends `StatusSummary` / `summarize` and the
`dashboard.rs` paned renderer. Independent of phase-03.
**Estimated diff:** ~260 lines (`mcp/src/status.rs` summarize + `mcp/src/dashboard.rs`
panel + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

The dashboard currently shows Session / Heartbeat / Files, but **not** the
parse-failure, verifier, and tool signals that M8's Exit criteria require
("parse/verifier signal"). That data is **already in the session JSONL** — the
loop logs `ParseFailed`, `Verify`, `ToolResult`, and `HardFail` records — but
`status::summarize` silently drops them (its match has a `_ => {}` arm). This
phase folds those four event kinds into `StatusSummary` and renders them in a new
**Activity** panel, turning the dashboard into a real "what is the model
struggling with right now" view. **No executor change** — pure surfacing of data
that already exists.

This closes the *parse/verifier-signal* half of M8's Exit criteria. The *budget
consumed* half needs new per-turn data the executor doesn't yet flush to the JSONL
(token / context-window usage) — that is **phase-05**, explicitly out of scope here.

## Architecture references

- `docs/architecture.md` § Layer 2 "Liveness (pull, not push)" — the dashboard
  surfaces the live JSONL; this phase widens *which* of its records it reads.
- M8 README § Exit criteria — "continuously shows … parse/verifier signal …".
- `executor/src/store/sessions/event.rs` — the `SessionEvent` variants; this phase
  reads `ParseFailed`, `Verify`, `ToolResult`, `HardFail` (currently unread).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/status.rs` end to end (it is small) — this phase extends
   `StatusSummary` and `summarize`.
3. Read `mcp/src/dashboard.rs` end to end — this phase adds one panel.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### The `SessionEvent` variants this phase starts reading (`executor/src/store/sessions/event.rs`)

```rust
pub enum SessionEvent {
    // ... (SessionStart, Prompt, Completion, Parsed, Progress, SessionEnd handled/ignored already)
    ParseFailed { failure: crate::parser::ParseFailure },   // failure.feedback: String
    ToolResult  { name: String, succeeded: bool, output_preview: String },
    Verify      { diagnostics: Vec<crate::governor::verifier::Diagnostic> },
    HardFail    { reason: String },
}
```

You only need scalar fields off these: `failure.feedback`, `name` + `succeeded`,
`diagnostics.len()`, `reason`. You do **not** need to name or import
`ParseFailure` / `Diagnostic` — pattern-match the variant and read the field.

### `StatusSummary` + `summarize` today (`mcp/src/status.rs`)

```rust
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusSummary {
    pub session_id: Option<String>,
    pub phase: Option<String>,
    pub model: Option<String>,
    pub latest_turn: usize,
    pub latest_stage: Option<String>,
    pub latest_message: Option<String>,
    pub files_changed: Vec<FileNumstat>,
    pub last_ts: Option<u64>,
    pub ended: Option<String>,
}

pub fn summarize(records: &[SessionRecord]) -> StatusSummary {
    let mut summary = StatusSummary { /* explicit literal, all fields */ };
    for rec in records {
        summary.last_ts = Some(/* max */);
        match &rec.event {
            SessionEvent::SessionStart { .. } => { /* sets session_id/model/phase */ }
            SessionEvent::Progress { .. }     => { /* sets turn/stage/message/files */ }
            SessionEvent::SessionEnd { .. }   => { /* sets ended, maxes turn */ }
            _ => {}        // <-- ParseFailed / Verify / ToolResult / HardFail dropped here
        }
    }
    summary
}
```

### The dashboard renderer today (`mcp/src/dashboard.rs`, from phase-02)

Three panels via `ratatui::layout::Layout`: a top row split `Session | Heartbeat`,
and a `Files` panel filling below. Pure per-panel formatters
(`session_lines`, `heartbeat_lines`, `files_lines`) each return
`Vec<Line<'static>>`; a `panel(title, lines)` helper wraps them; `render_dashboard`
splits the area and renders them. This phase adds a fourth formatter + panel and
re-splits the bottom row.

## Spec

### Task 1 — Extend `StatusSummary` (`mcp/src/status.rs`)

Add these fields (all `Default`-friendly; the derive already exists):

```rust
    /// Count of `ParseFailed` records seen so far.
    pub parse_failures: usize,
    /// Feedback text from the most recent `ParseFailed`, if any.
    pub last_parse_feedback: Option<String>,
    /// Diagnostic count from the most recent `Verify`. `None` = no verify yet;
    /// `Some(0)` = the last verify was clean.
    pub last_verify_diagnostics: Option<usize>,
    /// Name of the most recent `ToolResult`, and whether it succeeded.
    pub last_tool: Option<String>,
    pub last_tool_ok: Option<bool>,
    /// Reason from a `HardFail` record, if one was logged.
    pub hard_fail_reason: Option<String>,
```

### Task 2 — Fold the four events in `summarize` (`mcp/src/status.rs`)

First, replace the explicit `StatusSummary { … }` initializer with
`StatusSummary::default()` (the derive exists) so the new fields don't need
listing. The existing per-field assignments in the match arms stay as-is.

Then replace the `_ => {}` arm with arms for the four events:

```rust
            SessionEvent::ParseFailed { failure } => {
                summary.parse_failures += 1;
                summary.last_parse_feedback = Some(failure.feedback.clone());
            }
            SessionEvent::Verify { diagnostics } => {
                summary.last_verify_diagnostics = Some(diagnostics.len());
            }
            SessionEvent::ToolResult { name, succeeded, .. } => {
                summary.last_tool = Some(name.clone());
                summary.last_tool_ok = Some(*succeeded);
            }
            SessionEvent::HardFail { reason } => {
                summary.hard_fail_reason = Some(reason.clone());
            }
            _ => {}     // Prompt, Completion, Parsed remain intentionally unread
        }
```

`format_status` (the `rexymcp status` human text) is **not** changed in this phase
— the new fields flow into `status --json` automatically (additive), and the
dashboard renders them. Leaving `format_status` alone keeps the existing status
tests untouched.

### Task 3 — Add the `activity_lines` formatter (`mcp/src/dashboard.rs`)

A new pure formatter mirroring the phase-02 ones:

```rust
fn activity_lines(summary: &StatusSummary) -> Vec<Line<'static>> { /* ... */ }
```

Content (each line built like the phase-02 formatters; reuse the styling idiom):

- **Tool line:** when `summary.last_tool` is `Some(name)`, a line `tool: <name>
  <ok|FAIL>` — style the `FAIL` token red, `ok` green (use `last_tool_ok`).
- **Verify line:** when `summary.last_verify_diagnostics` is `Some(n)`, a line
  `verify: clean` (green) when `n == 0`, else `verify: <n> diagnostic(s)` (red).
- **Parse line:** when `summary.parse_failures > 0`, a line `parse failures: <n>`
  (yellow), and a second line with the truncated `last_parse_feedback` (first 80
  chars).
- **Hard-fail line:** when `summary.hard_fail_reason` is `Some(reason)`, a line
  `HARD FAIL: <reason>` (red, bold).
- When none of the above apply, a single `(no activity yet)` line.

### Task 4 — Add the Activity panel to `render_dashboard` (`mcp/src/dashboard.rs`)

Re-split the layout so the bottom region is itself a horizontal split of
`Files | Activity` (turning the layout into a 2×2 grid):

- Outer vertical: `[Constraint::Length(8), Constraint::Min(0)]` → `[top, bottom]`
  (unchanged).
- `top` horizontal split `[50%, 50%]` → `[Session, Heartbeat]` (unchanged).
- **New:** `bottom` horizontal split `[Constraint::Percentage(50),
  Constraint::Percentage(50)]` → `[files_area, activity_area]`.
- Render `panel(" Files ", files_lines(&data.summary))` into `files_area` and
  `panel(" Activity ", activity_lines(&data.summary))` into `activity_area`.

The error path (single full-area pane when `data.error.is_some()`) is unchanged.

## Acceptance criteria

- [ ] `StatusSummary` carries `parse_failures`, `last_parse_feedback`,
      `last_verify_diagnostics`, `last_tool` + `last_tool_ok`, `hard_fail_reason`,
      populated by `summarize`.
- [ ] `summarize` folds `ParseFailed`, `Verify`, `ToolResult`, and `HardFail`
      records (no longer dropped by `_ => {}`).
- [ ] The dashboard shows a fourth **Activity** panel rendering tool/verify/parse/
      hard-fail signals, with `(no activity yet)` when none are present.
- [ ] `status --json` includes the new fields (additive); `rexymcp status` human
      text and its tests are unchanged.
- [ ] No new dependencies. No executor-crate changes.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

Hermetic unit tests, mirroring the existing `status::tests` and `dashboard::tests`.
Add helper constructors for the new events in the test modules (follow the
existing `start()` / `progress()` helper pattern).

In `mcp/src/status.rs` `#[cfg(test)] mod tests`:
- `summarize_counts_parse_failures` — two `ParseFailed` records → `parse_failures
  == 2` and `last_parse_feedback == Some(<second feedback>)`.
- `summarize_records_last_verify` — a `Verify` with 3 diagnostics then one with 0
  → `last_verify_diagnostics == Some(0)`.
- `summarize_records_last_tool` — a `ToolResult { name: "bash", succeeded: false }`
  → `last_tool == Some("bash")`, `last_tool_ok == Some(false)`.
- `summarize_records_hard_fail` — a `HardFail { reason: "boom" }` →
  `hard_fail_reason == Some("boom")`.
- `summarize_clean_run_has_no_activity` — a SessionStart + Progress only →
  all five new fields are default (`0` / `None`).

In `mcp/src/dashboard.rs` `#[cfg(test)] mod tests`:
- `activity_lines_shows_tool_and_verify` — summary with `last_tool=Some("bash")`,
  `last_tool_ok=Some(true)`, `last_verify_diagnostics=Some(2)`; assert a line
  contains `bash` and a line contains `2 diagnostic`.
- `activity_lines_empty_placeholder` — default summary → a line contains
  `no activity`.

Keep all existing `summarize_*`, `load_data_*`, and phase-02 formatter tests
passing unchanged.

## End-to-end verification

The Activity panel is terminal rendering — not unit-tested directly (consistent
with phases 01–02). Verify against the built binary and quote in the Update Log:

1. `cargo run -p rexymcp -- dashboard --help` still lists `--repo` and `--session`.
2. Write a session JSONL containing a `tool_result` (e.g. `bash`, `succeeded:false`),
   a `verify` with diagnostics, and a `parse_failed` record to a temp dir; run
   `cargo run -p rexymcp -- dashboard --repo <tmpdir>`. Quote: four panels render,
   the Activity panel shows the tool/verify/parse signals.
3. `cargo run -p rexymcp -- status --repo <tmpdir> --json` includes the new fields;
   `cargo run -p rexymcp -- status --repo <tmpdir>` (human) output is unchanged
   from before this phase.

## Authorizations

- [x] May modify `mcp/src/status.rs` (extend `StatusSummary` + `summarize`) and
      `mcp/src/dashboard.rs` (Activity formatter + panel).
- [ ] No `format_status` change. No `Cargo.toml`, no `docs/architecture.md`, no
      executor-crate changes. No changes to `runs.rs` / `scorecard.rs`.

## Out of scope

- **Budget / token / context-window panel.** That data is not in the session JSONL
  — the executor computes token usage in `RunMetrics` but only writes it to the
  end-of-run `PhaseRun`, never per-turn. Surfacing it live requires the executor to
  emit a new per-turn `SessionEvent` — **phase-05**, not this phase. Do **not** add
  token/context fields to `StatusSummary` here (they would always be empty).
- **Compaction events** — `compact()` emits no `SessionEvent` today; recording it is
  **phase-06**.
- **`format_status` human-text changes** — additive `--json` fields only.
- **Per-tool latency, tokens-per-second, model-latency** — future measurement work,
  not this phase.
- **Scrolling / history** — the Activity panel shows latest-state, not a scrollback.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
