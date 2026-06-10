# Phase 04: Activity — distinct `<think>`/`</think>` block formatting in Completion bodies

**Milestone:** M13 — Dashboard Polish
**Status:** review
**Depends on:** phase-02 (done) — phase-02 made the Completion body soft-white via
`plain_body_lines`; this phase replaces that single call with a think-aware
renderer. No code dependency beyond the same `record_lines` Completion arm.
**Estimated diff:** ~140 lines (2 prod helpers ~60, transcript wiring ~2, tests ~80)
**Tags:** language=rust, kind=feature, size=m

## Goal

A local-LLM completion often interleaves chain-of-thought reasoning wrapped in
`<think>…</think>` markers with the actual answer. The dashboard currently renders
the whole `raw` completion body uniformly in soft white, so the reasoning and the
answer are visually indistinguishable. Render the reasoning **dim + italic** and the
answer in the existing soft white, so the eye can separate "the model thinking" from
"the model answering." Pure presentation — no feed, config, or executor change
(item #6).

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  "display only" constraint and the phase table. This phase touches **only**
  `mcp/src/dashboard/highlight.rs` and `mcp/src/dashboard/transcript.rs`; it adds
  **no** `SessionEvent`, no config, no `StatusSummary` field. The README's
  pre-injection note for this phase: "`<think>` formatting (04) is greenfield —
  there is no existing `think` handling anywhere in `mcp/src/`. Pin the parsing
  behavior … with explicit negative cases (a body with no think tags renders
  byte-identically; a `</think>` with no opening `<think>` still separates)."

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the milestone README above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The Completion body is rendered in `record_lines` in
`mcp/src/dashboard/transcript.rs`. Today the arm (lines 63–69) sends the entire
`raw` through `plain_body_lines` at a single soft-white color — the reasoning gets
no distinct treatment:

```rust
// LLM completions: soft white so the model's words read easily.
SessionEvent::Completion { raw } => (
    "completion:".to_string(),
    Color::Reset,
    false,
    Some(plain_body_lines(raw, Color::Rgb(200, 200, 200))),
),
```

`plain_body_lines` (in `mcp/src/dashboard/highlight.rs:188`) is the existing
per-line indent+style+cap machinery you will mirror; it delegates the
line-splitting and the cap to `body_lines`:

```rust
/// Render `content` as indented lines, all in the same `color`.
pub(crate) fn plain_body_lines(content: &str, color: Color) -> Vec<Line<'static>> {
    body_lines(content)
        .into_iter()
        .map(|l| Line::from(Span::styled(l, Style::new().fg(color))))
        .collect()
}

/// Split `body` on newlines into indented display lines.
pub(crate) fn body_lines(body: &str) -> Vec<String> {
    let all: Vec<&str> = body.split('\n').collect();
    if all.len() <= TRANSCRIPT_CONTENT_MAX_LINES {
        all.iter().map(|l| format!("    {l}")).collect()
    } else {
        let mut out: Vec<String> = all
            .iter()
            .take(TRANSCRIPT_CONTENT_MAX_LINES)
            .map(|l| format!("    {l}"))
            .collect();
        out.push(format!(
            "    … ({} more lines)",
            all.len() - TRANSCRIPT_CONTENT_MAX_LINES
        ));
        out
    }
}
```

`TRANSCRIPT_CONTENT_MAX_LINES = 20` (`highlight.rs:23`). Your new renderer must
honor the **same** per-record cap and the **same** overflow marker text
(`    … (N more lines)`), measured across the **whole** body (think + answer
combined), so the existing cap test (`record_lines_caps_long_content`, which feeds
a Completion `TRANSCRIPT_CONTENT_MAX_LINES + 5` lines long) still passes unchanged.

`highlight.rs` currently imports `style::{Color, Style}` (line 4). You will add
`Modifier` for the italic reasoning style; `Modifier::BOLD` is already used the
same way over in `transcript.rs:196`, so the import shape is established.

There is **no** `<think>` handling anywhere in `mcp/src/` today — confirm with
`grep -ri think mcp/src/` returning nothing before you start (this is greenfield).

## The parsing contract (pin this exactly)

The markers are the **literal** ASCII substrings `<think>` and `</think>`. Match
them exactly — do **not** trim whitespace inside the angle brackets, do **not**
match case-insensitively, do **not** treat `<thinking>` as a marker (it does not
contain the substring `<think>` — after `<think` comes `i`, not `>`). This
literal-match simplification is the accepted scope, the same way phase-03 accepted
char-count over unicode-display-width wrapping; note it in the helper doc-comment.

A completion body is a sequence of **answer** regions and **think** (reasoning)
regions. The mode toggles at each marker. The one subtlety is the **initial mode**,
because real local-LLM output sometimes emits a closing `</think>` with the opening
`<think>` stripped (the harness consumed it):

**Initial mode = think iff a `</think>` occurs before any `<think>` (or there is a
`</think>` and no `<think>` at all). Otherwise initial mode = answer.** Then:

- `<think>` switches the following text to **think** mode.
- `</think>` switches the following text to **answer** mode.
- The markers themselves are **removed** from the rendered output.
- An **unterminated** `<think>` (no following `</think>`) leaves everything after
  it in think mode.

Worked truth table — verify your implementation produces exactly these
(segment, is_think) lists (empty segments dropped):

| Input `raw` | Segments `(text, is_think)` |
|---|---|
| `plain answer` (no markers) | `[("plain answer", false)]` |
| `<think>reasoning</think>answer` | `[("reasoning", true), ("answer", false)]` |
| `reasoning</think> answer` (no opening) | `[("reasoning", true), (" answer", false)]` |
| `<think>reasoning` (unterminated) | `[("reasoning", true)]` |
| `a<think>b</think>c` | `[("a", false), ("b", true), ("c", false)]` |

The **no-markers** row is the load-bearing negative: a body with no think tags must
render **byte-identically** to today's `plain_body_lines(raw, Rgb(200,200,200))`
output (same `    `-indent, same lines, same soft-white style, same cap + overflow
marker). The **no-opening** row (`reasoning</think> answer`) is the second
load-bearing negative — it must still separate, with the leading text as reasoning.

## Spec

All changes are in `highlight.rs` and `transcript.rs`. No other files.

### 1. Add the segment splitter — `highlight.rs`

Add this pure helper near `body_lines` (it is the load-bearing parser; copy this
shape exactly — it is the truth table above, implemented):

```rust
/// Split a completion `raw` body into ordered segments tagged with whether the
/// text is reasoning (inside a `<think>…</think>` block). The literal `<think>` /
/// `</think>` markers are matched exactly (no whitespace/case tolerance — a
/// `<thinking>` is not a marker) and removed from the output. Empty segments are
/// dropped. The initial mode is `think` when a `</think>` precedes any `<think>`
/// (or there is a closing tag and no opening one), which covers models that emit
/// a closing tag with the opening tag stripped; an unterminated `<think>` leaves
/// the remainder in think mode.
pub(crate) fn split_think_segments(raw: &str) -> Vec<(String, bool)> {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";

    let first_open = raw.find(OPEN);
    let first_close = raw.find(CLOSE);
    let mut in_think = match (first_open, first_close) {
        (Some(o), Some(c)) => c < o,
        (None, Some(_)) => true,
        _ => false,
    };

    let mut segments: Vec<(String, bool)> = Vec::new();
    let mut rest = raw;
    loop {
        let next_open = rest.find(OPEN);
        let next_close = rest.find(CLOSE);
        let (idx, marker_len, next_mode) = match (next_open, next_close) {
            (Some(o), Some(c)) if o < c => (o, OPEN.len(), true),
            (Some(_), Some(c)) => (c, CLOSE.len(), false),
            (Some(o), None) => (o, OPEN.len(), true),
            (None, Some(c)) => (c, CLOSE.len(), false),
            (None, None) => {
                if !rest.is_empty() {
                    segments.push((rest.to_string(), in_think));
                }
                break;
            }
        };
        let (before, after) = rest.split_at(idx);
        if !before.is_empty() {
            segments.push((before.to_string(), in_think));
        }
        in_think = next_mode;
        rest = &after[marker_len..];
    }
    segments
}
```

`raw.find` returns a byte index at the start of an ASCII marker (a valid char
boundary), and `marker_len` is the marker's ASCII byte length, so `split_at(idx)`
and `&after[marker_len..]` are boundary-safe on UTF-8 content. No `unwrap`,
`expect`, or `panic!` — STANDARDS §2.1.

### 2. Add the think-aware body renderer — `highlight.rs`

Add this pure helper just below `split_think_segments`. It flattens the segments
into per-line `(text, is_think)` entries, then applies the **same** indent, cap,
and overflow marker as `body_lines`, styling each line by its mode:

```rust
/// Render a completion `raw` body, styling `<think>…</think>` reasoning distinctly
/// (dim + italic) from the answer text (soft white). The per-record cap
/// (`TRANSCRIPT_CONTENT_MAX_LINES`) and overflow marker apply across the whole
/// body. With no think markers this is byte-identical to
/// `plain_body_lines(raw, Color::Rgb(200, 200, 200))`.
pub(crate) fn completion_body_lines(raw: &str) -> Vec<Line<'static>> {
    let answer = Style::new().fg(Color::Rgb(200, 200, 200));
    let think = Style::new()
        .fg(Color::Rgb(128, 128, 128))
        .add_modifier(Modifier::ITALIC);

    let mut tagged: Vec<(String, bool)> = Vec::new();
    for (text, is_think) in split_think_segments(raw) {
        for line in text.split('\n') {
            tagged.push((line.to_string(), is_think));
        }
    }

    let total = tagged.len();
    let mut result: Vec<Line<'static>> = tagged
        .into_iter()
        .take(TRANSCRIPT_CONTENT_MAX_LINES)
        .map(|(text, is_think)| {
            let style = if is_think { think } else { answer };
            Line::from(Span::styled(format!("    {text}"), style))
        })
        .collect();
    if total > TRANSCRIPT_CONTENT_MAX_LINES {
        result.push(Line::from(Span::styled(
            format!("    … ({} more lines)", total - TRANSCRIPT_CONTENT_MAX_LINES),
            answer,
        )));
    }
    result
}
```

Note the indent (`format!("    {text}")`), the cap (`take(TRANSCRIPT_CONTENT_MAX_LINES)`),
and the overflow marker (`    … (N more lines)` in the soft-white `answer` style) are
copied verbatim from `body_lines` so the no-markers path renders identically and the
existing cap test stays green.

### 3. Add the `Modifier` import — `highlight.rs`

Extend the existing `use ratatui::{ style::{Color, Style}, text::{Line, Span} };`
block: change `style::{Color, Style}` to `style::{Color, Modifier, Style}`. (`Line`,
`Span` are already imported.)

### 4. Wire the renderer into the Completion arm — `transcript.rs`

In `record_lines`, replace the Completion arm's body call. Change:

```rust
SessionEvent::Completion { raw } => (
    "completion:".to_string(),
    Color::Reset,
    false,
    Some(plain_body_lines(raw, Color::Rgb(200, 200, 200))),
),
```

to:

```rust
SessionEvent::Completion { raw } => (
    "completion:".to_string(),
    Color::Reset,
    false,
    Some(completion_body_lines(raw)),
),
```

Update the `use super::highlight::{…}` import (line 8) to bring in
`completion_body_lines`. `plain_body_lines` is **still used** by the `Prompt` and
`Parsed` arms, so keep it in the import — do **not** remove it (removing a
still-used import, or leaving an unused one, both fail the zero-warnings gate;
build to confirm). The import becomes
`use super::highlight::{completion_body_lines, highlighted_body_lines, plain_body_lines};`.

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `split_think_segments` reproduces every row of the truth table above exactly
      (including empty-segment dropping and the no-opening initial-think case).
- [ ] `completion_body_lines("plain answer")` is **byte-identical** to
      `plain_body_lines("plain answer", Color::Rgb(200, 200, 200))` — same line
      count, same text, same span style (the no-markers negative).
- [ ] In a body with a think block, the reasoning line's span style has
      `fg == Some(Color::Rgb(128, 128, 128))` **and** the `ITALIC` modifier set,
      while the answer line's span style has `fg == Some(Color::Rgb(200, 200, 200))`
      and **no** italic — the two regions are visually distinct.
- [ ] The literal markers `<think>` / `</think>` do **not** appear in any rendered
      line's text; `<thinking>` is treated as ordinary answer text (not a marker).
- [ ] A Completion whose body exceeds `TRANSCRIPT_CONTENT_MAX_LINES` lines still
      renders `1 header + TRANSCRIPT_CONTENT_MAX_LINES body + 1 overflow marker`
      lines, the marker containing `more lines` (existing
      `record_lines_caps_long_content` test, unchanged, still passes).
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test`
      all pass.
- [ ] `git diff --name-only` lists only `mcp/src/dashboard/highlight.rs` and
      `mcp/src/dashboard/transcript.rs` (plus this phase doc and the README row). No
      `Cargo.toml`, no `filter.rs`, no `render.rs`, no `event_loop.rs`, no
      `SessionEvent`/config edit.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` block of **`highlight.rs`**
for the two new pure helpers (`use super::*` is already in scope there), and one
integration assertion in **`transcript.rs`**'s test block for the wired Completion
arm. Names describe behavior; exact count and placement are yours. The
**load-bearing** tests are `split_think_segments_handles_no_opening_tag` and
`completion_body_no_markers_matches_plain` — they pin the two README negatives.

In `highlight.rs`:

- `split_think_segments_no_markers_is_single_answer` — `split_think_segments("plain
  answer") == vec![("plain answer".to_string(), false)]`.
- `split_think_segments_splits_open_and_close` —
  `split_think_segments("<think>reasoning</think>answer") ==
  vec![("reasoning".into(), true), ("answer".into(), false)]`.
- `split_think_segments_handles_no_opening_tag` —
  `split_think_segments("reasoning</think> answer") ==
  vec![("reasoning".into(), true), (" answer".into(), false)]` (the closing-only
  case starts in think mode).
- `split_think_segments_handles_unterminated_open` —
  `split_think_segments("<think>reasoning") == vec![("reasoning".into(), true)]`.
- `split_think_segments_ignores_thinking_lookalike` —
  `split_think_segments("<thinking>") == vec![("<thinking>".into(), false)]` (not a
  marker).
- `completion_body_no_markers_matches_plain` — assert
  `completion_body_lines("a\nb\nc")` equals
  `plain_body_lines("a\nb\nc", Color::Rgb(200, 200, 200))` by comparing rendered
  text (`format!("{l}")` per line) **and** the first span's `style` of each line
  (same `fg`, no `ITALIC`) — the byte-identical negative.
- `completion_body_styles_think_distinct_from_answer` — over
  `"<think>why</think>final"`, find the line whose text contains `why` and assert
  its span `fg == Some(Color::Rgb(128, 128, 128))` and
  `style.add_modifier`/`contains(Modifier::ITALIC)` is set; find the line whose text
  contains `final` and assert `fg == Some(Color::Rgb(200, 200, 200))` and italic
  **not** set. Also assert no rendered line contains `<think>` or `</think>`.
- `completion_body_caps_with_overflow_marker` — a body of
  `TRANSCRIPT_CONTENT_MAX_LINES + 3` newline-separated lines (no markers) →
  `completion_body_lines(...).len() == TRANSCRIPT_CONTENT_MAX_LINES + 1`, last line
  contains `more lines`.

In `transcript.rs`:

- `completion_record_styles_think_block` — build
  `SessionEvent::Completion { raw: "<think>reasoning here</think>the answer".into() }`,
  call `record_lines`, assert the header line is `[t…] completion:`, that some body
  line contains `reasoning here` with `fg == Some(Color::Rgb(128, 128, 128))`, that
  some body line contains `the answer` with `fg == Some(Color::Rgb(200, 200, 200))`,
  and that no body line's text contains the literal `<think>` / `</think>` markers.

(Reading a line's first span style: `line.spans[0].style.fg` and
`line.spans[0].style.add_modifier` — compare against `Modifier::ITALIC` via
`.contains(...)`. The existing `prompt_body_shows_rendered_text_soft_white` test in
`transcript.rs` is a template for the record-level assertion shape.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13
phase-01/02/03). Verification is the pure-function `split_think_segments` /
`completion_body_lines` assertions plus the `record_lines` integration test and the
`cargo` gates. The actual italic/dim *rendering* is exercised by the live binary;
the segment split and the per-line styling that drive it are fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — `Modifier` is already in `ratatui = "0.30"`
      (stable; `Modifier::BOLD` is already used in `transcript.rs`). **No
      `Cargo.toml` edit.**
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- Add a new `SessionEvent` variant, a config field, or a `StatusSummary` field —
  the data is already in `Completion.raw`. If you think you need one, **stop and
  file a blocker**: you have left M13's display-only scope.
- Touch `filter.rs`, `render.rs`, `event_loop.rs`, or `panels.rs`. The wrap/scroll
  machinery from phase-03 operates on whatever `record_lines` returns and is
  orthogonal — your extra think lines wrap and scroll for free.
- Change `plain_body_lines`, `body_lines`, `highlighted_body_lines`, or the
  `Prompt` / `Parsed` / `ToolResult` arms of `record_lines`. Only the Completion
  arm changes. `plain_body_lines` stays (still used by Prompt/Parsed).
- Change `TRANSCRIPT_CONTENT_MAX_LINES` or invent a new cap. Reuse the existing one
  exactly so the cap test stays green.
- Parse think markers with whitespace/case tolerance, regex, or a new dependency.
  Literal `<think>` / `</think>` substring matching is the accepted simplification
  (note it in the helper doc-comment, as phase-03 noted char-count wrapping).
- Strip, collapse, or reflow the reasoning text — render it line-for-line, only
  styled distinctly. (Hiding/folding think blocks behind a toggle is a possible
  future enhancement, not this phase.)

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 17:43 (started)

**Executor:** Claude (Sonnet 4.5)
**Changes:** Implementing `split_think_segments` and `completion_body_lines` in `highlight.rs`, wiring into Completion arm in `transcript.rs`, adding 9 unit tests.

### Update — 2026-06-10 17:43 (complete)

**Summary:** Added think-aware completion body rendering with `<think>`/`</think>` segment splitting and distinct dim+italic styling for reasoning blocks.

**Files changed:**
- `mcp/src/dashboard/highlight.rs` — Added `Modifier` import, `split_think_segments()` helper, `completion_body_lines()` renderer, 8 unit tests.
- `mcp/src/dashboard/transcript.rs` — Wired `completion_body_lines` into Completion arm, added `completion_record_styles_think_block` integration test.

**Verification commands:**
```
cargo fmt --all --check: PASS (formatted)
cargo build: PASS (zero warnings)
cargo clippy --all-targets --all-features -- -D warnings: PASS
cargo test: PASS (725 passed, 0 failed, 2 ignored)
```

**Grep for pinned literals:**
```
$ grep -r '<think>' mcp/src/dashboard/highlight.rs
  const OPEN: &str = "<think>";
  ... (8 matches across tests)
$ grep -r '</think>' mcp/src/dashboard/highlight.rs
  const CLOSE: &str = "</think>";
  ... (6 matches across tests)
```

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact (TUI rendering has no headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13 phase-01/02/03). Verification is the pure-function `split_think_segments` / `completion_body_lines` assertions plus the `record_lines` integration test and the `cargo` gates.

**Notes for review:** None — implementation follows the spec exactly.

**Commit:** `feat: render <think>/</think> reasoning blocks dim+italic in transcript`
