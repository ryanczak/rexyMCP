# Phase 10b: Activity transcript — color, multi-line, tool output, tail-follow

**Milestone:** M8 — Live session dashboard
**Status:** done
**Depends on:** phase-10a (done — the `transcript_line` / `clamp_scroll` / scroll
wiring this phase enriches).
**Estimated diff:** ~230 lines (`mcp/src/dashboard.rs` transcript rebuild + scroll
state + tests).
**Tags:** language=rust, kind=feature, size=m

## Goal

Second half of the Activity transcript. 10a shipped one **plain** line per record
plus manual scrolling. This phase makes it usable on a live session:

1. **Color** — style each transcript line by event type (e.g. tool `FAIL` / hard-fail
   red, clean verify / tool `ok` green, agent thought dim).
2. **Multi-line expansion** — render **Completion** (agent thought) and **ToolResult**
   (tool output) across multiple lines instead of a one-line `preview()`, so the
   actual content is readable. Capped per record so one huge output can't flood.
3. **Auto-follow-tail** — the transcript starts pinned to the **newest** record and
   sticks to the bottom as new records arrive, until the user scrolls up; `End`
   re-pins. This fixes 10a's wart (started at the oldest record, `End` to reach now).

**mcp-crate only.** No executor change, no new dependency.

## Architecture references

Read before starting:

- M8 README § "Design decisions" — hermetic data layer. The line-builder and the
  tail-follow offset math are **pure** and unit-tested; color and the live scroll are
  review-by-inspection (terminal `Display` drops styling, so tests assert text + line
  counts, exactly as the existing `session_lines` styled-panel tests do).
- `executor/src/store/sessions/event.rs` — the 12 `SessionEvent` variants (unchanged
  since 10a).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/dashboard.rs` end to end — you restructure the transcript builder,
   `render_dashboard`, and `run_loop` scroll state. Note the existing styled-panel
   idiom (`Span::styled(text, Style::new().fg(Color::X))`, e.g. in `session_lines` /
   `compactions_lines`) — reuse it for transcript color.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### 10a's single-line builder (`mcp/src/dashboard.rs`)

```rust
/// Format a single record as a one-line transcript entry.
fn transcript_line(rec: &SessionRecord) -> Line<'static> {
    let summary = match &rec.event {
        SessionEvent::SessionStart { model, phase, .. } => format!("session start — phase {phase}, model {model}"),
        SessionEvent::Prompt { rendered } => format!("prompt ({} chars)", rendered.chars().count()),
        SessionEvent::Completion { raw } => format!("completion: {}", preview(raw)),
        SessionEvent::Parsed { tool_call } => format!("→ call {}", tool_call.name),
        SessionEvent::ParseFailed { failure } => format!("parse failed: {}", preview(&failure.feedback)),
        SessionEvent::ToolResult { name, succeeded, output_preview } => {
            let status = if *succeeded { "ok" } else { "FAIL" };
            format!("tool {name} [{status}] {}", preview(output_preview))
        }
        SessionEvent::Verify { diagnostics } => format!("verify: {} diagnostic(s)", diagnostics.len()),
        SessionEvent::HardFail { reason } => format!("HARD FAIL: {reason}"),
        SessionEvent::Progress { stage, .. } => format!("progress: {stage}"),
        SessionEvent::SessionEnd { status, turns } => format!("session end — {status} ({turns} turns)"),
        SessionEvent::Metrics { input_tokens, output_tokens, .. } => format!("metrics: {input_tokens} in / {output_tokens} out"),
        SessionEvent::Compaction { tokens_before, tokens_after, .. } => format!("compaction: {tokens_before} → {tokens_after} tokens"),
    };
    Line::from(format!("[t{}] {}", rec.turn, summary))
}
```

`transcript_lines(records)` maps `transcript_line` over records (placeholder when
empty). `preview(s)` collapses newlines and truncates to `TRANSCRIPT_PREVIEW_MAX = 100`.

### The Activity render + scroll today (`mcp/src/dashboard.rs`)

```rust
fn render_dashboard(frame: &mut Frame, area: Rect, data: &DashboardData, now_ms: u64, scroll: u16) {
    …
    let transcript = transcript_lines(&data.records);
    let activity = Paragraph::new(transcript)
        .scroll((scroll, 0))
        .block(Block::default().borders(Borders::ALL).title(" Activity "));
    frame.render_widget(activity, activity_area);
    …
}
```

`run_loop` owns `let mut scroll: u16 = 0;`, clamps it with
`clamp_scroll(scroll, transcript_lines(&data.records).len())` each frame, and handles
Up/Down/PageUp/PageDown/Home (offset) / End (`u16::MAX`). `clamp_scroll(scroll, total)`
returns `scroll.min(total.saturating_sub(1) as u16)`.

## Spec

Numbered tasks in execution order. **mcp-crate only**, no executor change, no new deps.

1. **Restructure the builder to multi-line: `transcript_line` → `record_lines`** — in
   `mcp/src/dashboard.rs`, rename and change the return type:
   ```rust
   /// Render one record as one or more transcript lines (header + optional body),
   /// styled by event type. Completion and ToolResult expand their content across
   /// multiple lines; all other events are a single styled header line.
   fn record_lines(rec: &SessionRecord) -> Vec<Line<'static>>
   ```
   `transcript_lines` then **flat-maps**: `records.iter().flat_map(record_lines).collect()`
   (placeholder `vec![Line::from("(no activity yet)")]` when empty — unchanged).

   Each event produces a **header** line `[t{turn}] {summary}` as today, but built as a
   `Span::styled(..)` with a per-type color (see task 2). For **Completion** and
   **ToolResult**, append the content as additional indented lines (task 3).

2. **Color the header per event type** — use the existing idiom
   `Line::from(Span::styled(text, Style::new().fg(Color::X)))`. Pin the *behavior*
   (each header is styled; the error/failure cases are visually distinct); the exact
   hues are yours. Recommended palette (adjust freely):
   - `ToolResult` FAIL, `HardFail`, `ParseFailed`, `Verify` with diagnostics > 0 → red
     (`HardFail` also bold via `.add_modifier(Modifier::BOLD)`)
   - `ToolResult` ok, `Verify` 0 diagnostics → green
   - `Parsed` (→ call) → blue; `Compaction` → magenta
   - `Prompt`, `Metrics`, `Progress` → dark-gray (dim, low-signal)
   - `SessionStart` / `SessionEnd` → cyan
   - `Completion` → default foreground (the header; body dim, see task 3)

3. **Multi-line content for Completion + ToolResult** — add a helper that splits a
   body into capped, indented lines:
   ```rust
   /// Max content lines shown per record before collapsing the rest into a
   /// "… (N more lines)" marker. Keeps one large tool output from flooding the panel.
   const TRANSCRIPT_CONTENT_MAX_LINES: usize = 20;

   /// Split `body` on newlines into indented display lines, capped at
   /// TRANSCRIPT_CONTENT_MAX_LINES with a trailing overflow marker when longer.
   fn body_lines(body: &str) -> Vec<String> {
       let all: Vec<&str> = body.split('\n').collect();
       if all.len() <= TRANSCRIPT_CONTENT_MAX_LINES {
           all.iter().map(|l| format!("    {l}")).collect()
       } else {
           let mut out: Vec<String> = all
               .iter()
               .take(TRANSCRIPT_CONTENT_MAX_LINES)
               .map(|l| format!("    {l}"))
               .collect();
           out.push(format!("    … ({} more lines)", all.len() - TRANSCRIPT_CONTENT_MAX_LINES));
           out
       }
   }
   ```
   - **Completion**: header `[t{turn}] completion:`; then `body_lines(raw)` each as a
     dim `Line` (`Style::new().fg(Color::DarkGray)` or `.add_modifier(Modifier::DIM)`).
   - **ToolResult**: header `[t{turn}] tool {name} [{ok|FAIL}]` (green/red per task 2);
     then `body_lines(output_preview)` each as a plain/dim `Line`.
   - All other variants stay **single-line** (the header only). `ParseFailed` keeps
     using `preview(&failure.feedback)` on its one header line (its feedback is short);
     do **not** multi-line it in this phase.
   - Long *individual* content lines are clipped horizontally by the panel — do **not**
     enable `Paragraph` wrapping (wrapping breaks the line-based scroll math below).

4. **Add the tail-follow offset helper** — a pure function that resolves the displayed
   scroll offset, replacing `clamp_scroll`'s role inside the renderer:
   ```rust
   /// Resolve the scroll offset to display. `follow` pins to the bottom (newest):
   /// the offset that shows the last `viewport` lines. Otherwise the manual `offset`
   /// is clamped so it can't scroll past the bottom.
   fn visible_offset(follow: bool, offset: u16, total_lines: usize, viewport: u16) -> u16 {
       let total = total_lines.min(u16::MAX as usize) as u16;
       let max = total.saturating_sub(viewport);
       if follow { max } else { offset.min(max) }
   }
   ```
   Keep `clamp_scroll` for the coarse `run_loop` clamp (task 6) — or remove it and
   clamp via `visible_offset`'s `max` is not available there (no viewport); simplest is
   to **keep `clamp_scroll`** for the run_loop coarse bound. Your call, but if you
   remove `clamp_scroll` you must delete its test too.

5. **Wire follow + viewport into `render_dashboard`** — change the signature from
   `scroll: u16` to `offset: u16, follow: bool`. In the Activity render:
   ```rust
   let transcript = transcript_lines(&data.records);
   let viewport = activity_area.height.saturating_sub(2); // minus top+bottom border
   let display = visible_offset(follow, offset, transcript.len(), viewport);
   let activity = Paragraph::new(transcript)
       .scroll((display, 0))
       .block(Block::default().borders(Borders::ALL).title(" Activity "));
   frame.render_widget(activity, activity_area);
   ```
   The other four panels are unchanged.

6. **Track follow state in `run_loop`** — replace `let mut scroll: u16 = 0;` with
   `let mut offset: u16 = 0;` and `let mut follow = true;` (start pinned to newest).
   Keep the per-frame coarse clamp `offset = clamp_scroll(offset, transcript_lines(&data.records).len());`.
   Pass `offset, follow` into `render_dashboard`. Key handling — any manual scroll
   turns following off; `End` turns it back on:
   ```rust
   KeyCode::Char('q') | KeyCode::Esc => break,
   KeyCode::Up       => { follow = false; offset = offset.saturating_sub(1); }
   KeyCode::Down     => { follow = false; offset = offset.saturating_add(1); }
   KeyCode::PageUp   => { follow = false; offset = offset.saturating_sub(10); }
   KeyCode::PageDown => { follow = false; offset = offset.saturating_add(10); }
   KeyCode::Home     => { follow = false; offset = 0; }
   KeyCode::End      => { follow = true; }
   _ => {}
   ```

7. **Update the three 10a transcript tests that call `transcript_line`** — they now
   call `record_lines` (returns `Vec<Line>`). See Test plan for the new shape. This is
   authorized churn from the task-1 signature change.

## Acceptance criteria

- [ ] `record_lines` returns `Vec<Line>`; `transcript_lines` flat-maps it; empty →
      placeholder.
- [ ] Completion and ToolResult expand to header + indented body lines, capped at
      `TRANSCRIPT_CONTENT_MAX_LINES` with an overflow marker; all other variants are a
      single header line.
- [ ] Each header is styled by event type (verified by inspection); FAIL/hard-fail are
      red, ok/clean-verify green.
- [ ] `visible_offset` pins to the bottom screenful when `follow`, else clamps the
      manual offset; `render_dashboard` takes `(offset, follow)` and uses it.
- [ ] `run_loop` starts with `follow = true`, turns it off on any manual scroll key,
      and back on with `End`; still quits on `q`/`Esc`.
- [ ] `cargo build` clean; clippy `-D warnings` clean; `cargo fmt --all --check` clean
      (use `rustfmt` only on touched files); `cargo test -p rexymcp` passes.

## Test plan

Add to / update `#[cfg(test)] mod tests` in `dashboard.rs`. Color is **not** asserted
(terminal `Display` drops styling — assert text content and line counts, as the
existing styled-panel tests do).

- `record_lines_single_line_for_short_events` — a `Progress` record → exactly 1 line,
  text contains `[t{turn}] progress:`.
- `record_lines_expands_completion_multiline` — `Completion { raw: "a\nb\nc" }` → 4
  lines (1 header + 3 body); header contains `completion:`; a body line contains `a`.
- `record_lines_expands_tool_output_multiline` — `ToolResult` with a 2-line
  `output_preview` and `succeeded: false` → header contains `tool {name} [FAIL]` and 2
  body lines follow.
- `record_lines_caps_long_content` — `Completion` whose `raw` has > 
  `TRANSCRIPT_CONTENT_MAX_LINES` newline-separated lines → total body lines ==
  `TRANSCRIPT_CONTENT_MAX_LINES + 1` and the last contains `more lines`.
- `transcript_lines_flatmaps_records` — two records where one is multi-line (e.g. a
  `SessionStart` + a 3-line `Completion`) → total lines == 1 + 4.
- `transcript_lines_empty_placeholder` — `&[]` → contains "no activity" (unchanged).
- `visible_offset_follows_tail` — `visible_offset(true, 0, 100, 20) == 80`;
  `visible_offset(true, 0, 10, 20) == 0` (total < viewport → 0).
- `visible_offset_manual_clamped` — `visible_offset(false, 999, 100, 20) == 80`;
  `visible_offset(false, 5, 100, 20) == 5`.
- **Update** the 10a `transcript_line_renders_each_variant` test → `record_lines`:
  assert the **first** (header) line's text per representative variant (SessionStart,
  ToolResult ok/FAIL, HardFail, Compaction, SessionEnd, etc.). **Replace** the 10a
  `transcript_line_truncates_long_content` test with `record_lines_caps_long_content`
  above (truncation→capping is the new behavior). Keep `clamp_scroll_bounds_to_last_line`
  if `clamp_scroll` is retained.

Key handling in `run_loop` is not unit-testable (TTY) — review-by-inspection, as with
the existing keys. `visible_offset` is its testable core.

## End-to-end verification

Content/cap/offset logic is proven by the unit tests. Color and live tail-follow need a
TTY. Verify:

1. Paste `cargo test -p rexymcp` output covering the new `record_lines_*`,
   `visible_offset_*`, and `transcript_lines_*` tests.
2. Build and launch against this repo's session logs:
   `cargo run -p rexymcp -- dashboard --repo .` — confirm the Activity panel shows a
   **colored** transcript, that it **starts at the newest** records (tail-pinned), that
   scrolling **up** stops following and `End` re-pins, and that a multi-line completion
   / tool output renders across lines. Quote one line of what you observed (colored,
   tail-pinned, multi-line, no panic). If no session log exists, note the error pane.

## Authorizations

None. No new dependency (`Color`/`Style`/`Modifier`/`Span` are already imported;
`Paragraph::scroll` is already used). No `docs/architecture.md` / `STANDARDS.md` /
`WORKFLOW.md` edit. The authorized churn is renaming `transcript_line` → `record_lines`
and updating its three 10a tests (Spec task 7).

## Out of scope

- **Multi-line expansion for variants other than Completion / ToolResult**
  (ParseFailed feedback, HardFail reason stay single-line — their content is short).
- **Horizontal scrolling / line wrapping** — long content lines clip; wrapping is
  deferred (it breaks the line-based scroll math).
- **Search, filtering, per-event-type collapse/toggle, timestamps column.**
- **Budget Tokens/Sec and "$ saved"** — that is phase-11 (still pending the pricing
  decision).
- **Any executor-crate change** or any change to the Session/Budget/Compactions/Files
  panels, `load_records`, or `summarize`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-09 (started)

**Executor:** rexyMCP assistant

**Tasks:** Implementing spec tasks 1–7: restructure `transcript_line` → `record_lines`
with multi-line expansion, per-event-type color, `body_lines` helper, `visible_offset`
tail-follow, wire into `render_dashboard` and `run_loop`, update tests.

### Update — 2026-06-03 (escalation)

**Chosen lever:** session takeover
**Rationale:** Two hard_fails. First was a pure infra drop (SSE stall, no code written).
Second: the executor completed **all production tasks (1–6) correctly and compiling**
on disk, then got stuck on **task 7 (updating the stale 10a tests)** — it emitted three
identical malformed `patch null` calls and the governor terminated it
(`IdenticalToolCallRepetition`). The spec was sound (production code came out right), so
refined re-dispatch wouldn't address the failure and would re-encounter the same
test-churn wall. Architect finished task 7 (rewrote the `transcript_line`→`record_lines`
tests + added the multi-line/cap/`visible_offset` tests) and fixed one latent
`clippy::useless_format` (`format!("completion:")` → `.to_string()`).

### Update — 2026-06-03 (complete — architect takeover)

**Summary:** Production tasks 1–6 (record_lines multi-line + color, body_lines,
visible_offset tail-follow, render/run_loop wiring) were implemented by the executor and
left compiling on disk; the architect completed task 7 (test updates) where the executor
stalled, and fixed a latent clippy `useless_format`.

**Commands:**

```
cargo fmt --all --check        → clean
cargo build                    → clean
cargo clippy --all-targets --all-features -- -D warnings → clean (after the useless_format fix)
cargo test                     → 199 mcp + 565 executor passed; 0 failed; 2 ignored
```

**End-to-end verification:** Live color/tail-follow needs a TTY (not headless-runnable);
content/cap/offset logic proven by unit tests (`record_lines_*`, `visible_offset_*`,
`transcript_lines_flatmaps_records`). On-screen render to be confirmed by the user on a
live session, as with phase-08/09/10a.

**Files changed:** `mcp/src/dashboard.rs` (executor: tasks 1–6; architect: task-7 tests
+ clippy fix).

### Review verdict — 2026-06-03

- **Verdict:** escalated
- **Bounces:** 2 hard_fails (infra SSE stall, then `IdenticalToolCallRepetition` on
  test-update patches) — no bug docs filed (not spec defects).
- **Executor:** Qwen/Qwen3.6-27B-FP8 (production tasks 1–6) + Claude (direct) (task-7
  tests + clippy fix).
- **Scope deviations:** none. All spec tasks landed; out-of-scope held (no wrapping, no
  multi-line for non-Completion/ToolResult variants, no executor change, other panels
  untouched).
- **Calibration:** none yet — **data point** for a possible future fold: the local
  executor reliably implements production code but stalls on **mechanical multi-edit
  test churn** (repeated identical failed patches). If this recurs, consider splitting
  "implementation" and "test-update" into separate phases, or pinning smaller test
  edits. One occurrence; hold for recurrence.
- **Latent issue caught in takeover:** `format!("completion:")` would have failed the
  clippy `-D warnings` gate (`useless_format`) — the executor never reached the lint
  step (stalled earlier), so it was unguarded. Fixed to `.to_string()`.
