# Phase 10a: Activity transcript — raw-record reader + scroll mechanics

**Milestone:** M8 — Live session dashboard
**Status:** review
**Depends on:** phase-09 (done — the current panel layout this builds on).
**Estimated diff:** ~230 lines (`mcp/src/status.rs` reader + `mcp/src/dashboard.rs`
transcript builder, scroll state, render wiring + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

First half of turning the **Activity panel** into a scrollable transcript (the
wireframe's centerpiece). Today the Activity panel shows a 4-line *signal summary*
distilled from `StatusSummary` (last tool / verify / parse / hard-fail). This phase
replaces it with a **chronological, scrollable transcript** that renders **one
plain-text line per session record, for every event type** (full replay), and adds
**keyboard scrolling** (Up/Down/PageUp/PageDown/Home/End).

This is **10a of a two-phase split** (decision recorded in `NEXT.md`):

- **10a (this phase):** raw-record reader plumbing + scroll state/keys + **one
  plain-text summary line per record**. No color, no multi-line expansion, no full
  tool-output rendering.
- **10b (next):** expand each transcript item into formatted, **colored**, multi-line
  detail (parsed JSON, full tool output, etc.) on top of 10a's structure.

Keeping 10a to "one plain line per record + scrolling" makes it a bounded, testable
unit; 10b does the rich rendering.

## Architecture references

Read before starting:

- M8 README § "Design decisions" — "Read-only, no side effects" and "Hermetic data
  layer". The transcript line-builder is a **pure** function tested without a terminal;
  the scroll **clamp** is a pure function; only key handling in `run_loop` (TTY) is
  review-by-inspection.
- `executor/src/store/sessions/event.rs` — the 12 `SessionEvent` variants the
  transcript renders (full list quoted below).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/status.rs` end to end (small) — you add `load_records` and refactor
   `load_status` to use it.
3. Read `mcp/src/dashboard.rs` end to end (small) — you add a transcript builder, a
   `records` field on `DashboardData`, scroll state + keys in `run_loop`, and a scroll
   param on `render_dashboard`.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### `load_status` reads records then throws them away (`mcp/src/status.rs:239`)

```rust
pub fn load_status(repo: &Path, session: Option<&str>) -> Result<StatusSummary, String> {
    let log_path = resolve_session_log(repo, session)?;

    let records = read_session_log(&log_path)
        .map_err(|e| format!("failed to read {}: {}", log_path.display(), e))?;

    Ok(summarize(&records))
}
```

`summarize` (also in `status.rs`, already `pub`) and `read_session_log`
(`use rexymcp_executor::store::sessions::jsonl::read_session_log;`, already imported)
are both available. `SessionRecord` is imported at the top of `status.rs`.

### `DashboardData` / `load_data` carry only the summary (`mcp/src/dashboard.rs:19`)

```rust
pub struct DashboardData {
    pub summary: StatusSummary,
    pub error: Option<String>,
}

pub fn load_data(repo: &Path, session: Option<&str>) -> DashboardData {
    match status::load_status(repo, session) {
        Ok(summary) => DashboardData { summary, error: None },
        Err(e) => DashboardData { summary: StatusSummary::default(), error: Some(e) },
    }
}
```

### The Activity panel today is a signal summary (`mcp/src/dashboard.rs`)

`activity_lines(summary)` builds the last-tool / verify / parse / hard-fail lines.
**This phase stops using `activity_lines` in the renderer** (replaced by the
transcript). Leave the `activity_lines` function and its tests in place for now —
do not delete them (10b/cleanup decides their fate); they just become unused by
`render_dashboard`. **However**, an unused private function trips
`-D warnings`. To avoid that without deleting it, the renderer change below keeps
`activity_lines` referenced is *not* an option — so instead: **move `activity_lines`
and its three tests out is also not in scope.** Resolve this cleanly by **deleting
`activity_lines` and its tests** (`activity_lines_shows_tool_and_verify`,
`activity_lines_empty_placeholder`) — the transcript supersedes it. (This is the one
deletion this phase authorizes; see Spec task 5.)

### `run_loop` handles only quit today (`mcp/src/dashboard.rs:323`)

```rust
        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
```

`render_dashboard(frame, area, &data, now_ms)` is called only from `run_loop`.

### The 12 `SessionEvent` variants to render (`executor/src/store/sessions/event.rs`)

```rust
SessionStart { session_id: String, model: String, phase: String }
Prompt { rendered: String }
Completion { raw: String }
Parsed { tool_call: crate::parser::ToolCall }   // tool_call.name: String
ParseFailed { failure: crate::parser::ParseFailure }   // failure.feedback: String
ToolResult { name: String, succeeded: bool, output_preview: String }
Verify { diagnostics: Vec<…::Diagnostic> }   // use .len()
HardFail { reason: String }
Progress { turn: usize, stage: String, files_changed: Vec<FileNumstat>, message: String }
SessionEnd { status: String, turns: usize }
Metrics { input_tokens: u32, output_tokens: u32, context_pct: f64 }
Compaction { tokens_before: usize, tokens_after: usize, messages_signaturized: usize, messages_evicted: usize }
```

`SessionRecord { ts: u64, turn: usize, event: SessionEvent }`.

## Spec

Numbered tasks in execution order. **mcp-crate only**, no executor change, no new deps.

1. **Add `load_records` and refactor `load_status`** — in `mcp/src/status.rs`:
   ```rust
   /// Resolve the log to report on and return its raw records (chronological).
   /// Shares resolution + read with `load_status`; the dashboard transcript needs
   /// the raw records, not the distilled summary.
   pub fn load_records(repo: &Path, session: Option<&str>) -> Result<Vec<SessionRecord>, String> {
       let log_path = resolve_session_log(repo, session)?;
       read_session_log(&log_path).map_err(|e| format!("failed to read {}: {}", log_path.display(), e))
   }
   ```
   Then `load_status` becomes `Ok(summarize(&load_records(repo, session)?))`. This is
   **behavior-preserving** for `load_status` (same resolution, same error strings, same
   summary). The `status` CLI command (`main.rs`) calls `load_status` and is unaffected.

2. **Carry raw records on `DashboardData`** — in `mcp/src/dashboard.rs`:
   - Add `use rexymcp_executor::store::sessions::event::SessionRecord;` to the
     production imports (top of file). (It is currently only imported in `#[cfg(test)]`.)
   - Add a field: `pub records: Vec<SessionRecord>,`.
   - Rewrite `load_data` to read records once and derive the summary from them:
     ```rust
     pub fn load_data(repo: &Path, session: Option<&str>) -> DashboardData {
         match status::load_records(repo, session) {
             Ok(records) => DashboardData {
                 summary: status::summarize(&records),
                 records,
                 error: None,
             },
             Err(e) => DashboardData {
                 summary: StatusSummary::default(),
                 records: Vec::new(),
                 error: Some(e),
             },
         }
     }
     ```

3. **Add a pure transcript line-builder** — in `mcp/src/dashboard.rs`, **one
   `Line<'static>` per record**, in input (chronological) order:
   ```rust
   /// Max chars of free-text content shown per transcript line in 10a (10b expands
   /// to full multi-line). Keeps one record = one line.
   const TRANSCRIPT_PREVIEW_MAX: usize = 100;

   fn transcript_lines(records: &[SessionRecord]) -> Vec<Line<'static>> {
       if records.is_empty() {
           return vec![Line::from("(no activity yet)")];
       }
       records.iter().map(transcript_line).collect()
   }
   ```
   `transcript_line(rec) -> Line<'static>` formats `"[t{turn}] {summary}"` where the
   summary is per-variant (match on `&rec.event`). Suggested summaries (pin the
   *content*, the executor may adjust punctuation):
   - `SessionStart { model, phase, .. }` → `session start — phase {phase}, model {model}`
   - `Prompt { rendered }` → `prompt ({} chars)` with `rendered.chars().count()`
     (do **not** dump the whole system prompt — it is huge; one-line summary only)
   - `Completion { raw }` → `completion: {preview}`
   - `Parsed { tool_call }` → `→ call {}` with `tool_call.name`
   - `ParseFailed { failure }` → `parse failed: {preview of failure.feedback}`
   - `ToolResult { name, succeeded, output_preview }` →
     `tool {name} [{}]` (`ok`/`FAIL`) ` {preview of output_preview}`
   - `Verify { diagnostics }` → `verify: {} diagnostic(s)` with `diagnostics.len()`
   - `HardFail { reason }` → `HARD FAIL: {reason}`
   - `Progress { stage, .. }` → `progress: {stage}`
   - `SessionEnd { status, turns }` → `session end — {status} ({turns} turns)`
   - `Metrics { input_tokens, output_tokens, .. }` → `metrics: {in} in / {out} out`
   - `Compaction { tokens_before, tokens_after, .. }` →
     `compaction: {tokens_before} → {tokens_after} tokens`

   Use a `preview(s: &str) -> String` helper that replaces newlines/tabs with spaces
   and truncates to `TRANSCRIPT_PREVIEW_MAX` chars with a trailing `…` when longer
   (char-based, not byte `len()`, to avoid multibyte panics — mirror the
   `trim_path_left` char-iteration pattern already in this file). No color in 10a.

4. **Add a pure scroll-clamp helper + wire scroll through the renderer**:
   - ```rust
     /// Clamp a scroll offset so it can't run past the last line.
     fn clamp_scroll(scroll: u16, total_lines: usize) -> u16 {
         let max = total_lines.saturating_sub(1) as u16;
         scroll.min(max)
     }
     ```
   - Change `render_dashboard` signature to take `scroll: u16` (after `now_ms`). In the
     body, build `let transcript = transcript_lines(&data.records);` and render the
     **Activity** panel as a scrolled `Paragraph` instead of via the `panel` helper:
     ```rust
     let activity = Paragraph::new(transcript)
         .scroll((scroll, 0))
         .block(Block::default().borders(Borders::ALL).title(" Activity "));
     frame.render_widget(activity, activity_area);
     ```
     (The other four panels — Session, Budget, Compactions, Files — stay exactly as
     they are, built via `panel(...)`.)

5. **Delete the now-superseded `activity_lines`** — remove the `activity_lines`
   function and its two unit tests (`activity_lines_shows_tool_and_verify`,
   `activity_lines_empty_placeholder`). The transcript replaces it; leaving it unused
   fails `-D warnings`. This is the only deletion this phase authorizes.

6. **Add scroll state + key handling in `run_loop`** — before the loop:
   `let mut scroll: u16 = 0;`. Pass `scroll` into `render_dashboard`. After computing
   `data` each iteration, clamp: `scroll = clamp_scroll(scroll, transcript_lines(&data.records).len());`
   (rebuild is cheap). In the key match, **keep the `q`/`Esc` quit** and add:
   ```rust
   KeyCode::Up        => scroll = scroll.saturating_sub(1),
   KeyCode::Down      => scroll = scroll.saturating_add(1),
   KeyCode::PageUp    => scroll = scroll.saturating_sub(10),
   KeyCode::PageDown  => scroll = scroll.saturating_add(10),
   KeyCode::Home      => scroll = 0,
   KeyCode::End       => scroll = u16::MAX, // clamped to bottom next frame
   ```
   The clamp at the top of the loop bounds `End`/`PageDown`/`Down` to the last line.

**Scroll semantics for 10a (pin in the doc comment):** transcript is **oldest-first**;
offset 0 shows the top (oldest). The user scrolls down / presses `End` to reach the
newest. **Auto-follow-tail (start pinned to newest, stick to bottom until the user
scrolls up) is deferred to 10b** — do not implement it here.

## Acceptance criteria

- [ ] `load_records` exists and `load_status` is refactored to call it
      (behavior-preserving — existing `status.rs` tests still pass unchanged).
- [ ] `DashboardData` has a `records: Vec<SessionRecord>` field, populated by
      `load_data` (non-empty when a log exists, empty on error).
- [ ] `transcript_lines` returns one line per record (chronological) with a
      placeholder when empty; `transcript_line` renders all 12 event variants.
- [ ] `clamp_scroll` bounds the offset to `total_lines - 1`.
- [ ] `render_dashboard` takes a `scroll` param and renders the Activity panel as a
      scrolled `Paragraph`; the other four panels are unchanged.
- [ ] `run_loop` adjusts `scroll` on Up/Down/PageUp/PageDown/Home/End and still quits
      on `q`/`Esc`.
- [ ] `activity_lines` and its two tests are removed; no dead-code/unused warnings.
- [ ] `cargo build` clean; clippy `-D warnings` clean; `cargo fmt --all --check` clean
      (use `rustfmt` only on touched files); `cargo test -p rexymcp` passes.

## Test plan

Add to `#[cfg(test)] mod tests` in `dashboard.rs` (and `status.rs` for the reader).
Follow the existing `*_lines` test style and the `rec(ts, turn, event)` /
`start_event()` helpers already in `dashboard.rs` tests.

- `load_records_returns_raw_records_in_order` (status.rs) — write a log with a
  `SessionStart` then a `Progress`; assert `load_records` returns 2 records in order
  (mirror `load_data_returns_summary_when_log_exists`'s tempdir setup).
- `load_records_errs_when_no_logs` (status.rs) — empty dir → `Err`.
- `load_data_carries_raw_records` (dashboard.rs) — log with ≥1 record →
  `data.records` is non-empty and `data.error` is `None`.
- `load_data_empty_records_on_error` (dashboard.rs) — no sessions dir →
  `data.records` is empty and `data.error` is `Some`.
- `transcript_lines_empty_placeholder` — `&[]` → contains "no activity".
- `transcript_lines_one_line_per_record` — 3 records → exactly 3 lines, in order
  (assert line 0 corresponds to the first record, etc.).
- `transcript_line_renders_each_variant` — one assertion block per variant (or a few
  representative: `Completion` preview, `ToolResult` ok vs FAIL, `SessionStart`,
  `Compaction`, `HardFail`) checking the summary text and the `[t{turn}]` prefix.
- `transcript_line_truncates_long_content` — a `Completion` with >100-char raw → the
  line contains `…` and is bounded; a short one is **not** truncated (negative case).
- `clamp_scroll_bounds_to_last_line` — `clamp_scroll(5, 3) == 2`,
  `clamp_scroll(0, 0) == 0`, `clamp_scroll(10, 100) == 10` (under-max unchanged).

Key handling in `run_loop` is not unit-testable (TTY) — verified by inspection, like
the existing `q`/`Esc`. The `clamp_scroll` helper is its testable core.

## End-to-end verification

The transcript content and scroll-clamp are proven by the unit tests above (they parse
real records and exercise the pure builders). The live scrolling itself needs a TTY and
is not runnable headlessly. Verify:

1. Paste `cargo test -p rexymcp` output covering the new `transcript_*`,
   `clamp_scroll_*`, and `load_records_*` tests.
2. Build and launch against this repo's existing session logs to confirm the Activity
   panel renders a transcript without panicking:
   `cargo run -p rexymcp -- dashboard --repo .` — observe transcript lines in the
   Activity panel, press Down/PageDown/End/Up, then `q`. Quote one line of what you
   observed (transcript visible, scroll responds, no panic). If no session log exists,
   the error pane is expected — note that instead.

## Authorizations

None. No new dependency (`KeyCode::{Up,Down,PageUp,PageDown,Home,End}` are existing
`crossterm` variants; `Paragraph::scroll` is existing `ratatui`). No
`docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edit. The single authorized
deletion is `activity_lines` + its two tests (Spec task 5).

## Out of scope

- **Color, multi-line expansion, JSON parsing/pretty-printing, full tool-output
  rendering.** All of that is **phase-10b**. 10a is one plain-text line per record.
- **Auto-follow-tail / start-at-bottom** scroll behavior — deferred to 10b.
- **Horizontal scrolling**, search, filtering, or per-event-type toggles.
- **Any executor-crate change**, any `StatusSummary`/`summarize` change beyond the
  `load_records` extraction, and any change to the Session/Budget/Compactions/Files
  panels.
- **Removing `status`-CLI behavior** — `load_status` stays and stays behavior-identical.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-18 (started)

**Executor:** rexyMCP executor

**Plan:** Implement all 6 spec tasks: (1) `load_records` + refactor `load_status`, (2) `DashboardData.records` field + `load_data` rewrite, (3) transcript line builder (`transcript_lines`, `transcript_line`, `preview`), (4) `clamp_scroll` + scroll param on `render_dashboard`, (5) delete `activity_lines` + tests, (6) scroll state + key handling in `run_loop`. Add all tests from the test plan.

### Update — 2025-07-18 (complete)

**Executor:** rexyMCP executor

**Files changed:**
- `mcp/src/status.rs` — added `load_records`, refactored `load_status` to call it; added 2 tests (`load_records_returns_raw_records_in_order`, `load_records_errs_when_no_logs`)
- `mcp/src/dashboard.rs` — added `SessionRecord`/`SessionEvent` imports; added `records` field on `DashboardData`; rewrote `load_data` to use `load_records`; added `transcript_lines`, `transcript_line`, `preview`, `clamp_scroll`; updated `render_dashboard` to take `scroll: u16` and render Activity as scrolled `Paragraph`; added scroll state + Up/Down/PageUp/PageDown/Home/End key handling in `run_loop`; deleted `activity_lines` and its 2 tests; added 7 new tests (`load_data_carries_raw_records`, `load_data_empty_records_on_error`, `transcript_lines_empty_placeholder`, `transcript_lines_one_line_per_record`, `transcript_line_renders_each_variant`, `transcript_line_truncates_long_content`, `clamp_scroll_bounds_to_last_line`)
- `docs/dev/milestones/M8-dashboard/README.md` — phase table row 10a → `review`
- `docs/dev/milestones/M8-dashboard/phase-10a-activity-transcript-scroll.md` — status → `review`, Update Log entries

**Verification commands:**

```
cargo fmt --all --check → clean
cargo build → clean
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test -p rexymcp → 194 passed; 0 failed; 0 ignored
```

**Verification summary:** fmt clean, build clean, clippy -D warnings clean, all 194 tests pass (7 new + 2 new in status.rs, 2 deleted activity_lines tests net +7).

**End-to-end verification:** N/A — live scrolling requires a TTY and cannot be run headlessly. The transcript content and scroll-clamp are fully proven by the unit tests above (they exercise all 12 event variants, truncation, empty placeholder, clamp boundaries).

**Notes for review:**
- `ToolCall` struct has an `origin: Origin` field not mentioned in the phase spec's event listing; the test for `Parsed` variant needed `origin: Origin::Native` to compile. This is an external-API divergence from the spec's sketch.
- One conventional commit per phase.
