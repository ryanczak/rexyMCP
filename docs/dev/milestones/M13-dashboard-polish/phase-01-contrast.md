# Phase 01: Legibility — raise dark-grey dashboard text to `Rgb(200,200,200)`

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none
**Estimated diff:** ~15 lines prod + ~4 small tests
**Tags:** language=rust, kind=feature, size=s

## Goal

Secondary text in the dashboard is rendered with `Color::DarkGray`, which is too
low-contrast to read on a dark terminal. Raise every `Color::DarkGray` site in
`mcp/src/dashboard/` to `Color::Rgb(200, 200, 200)` — the exact soft-white the
Completion-body and plain-text-fallback paths already use — so all text reads at a
consistent, legible contrast. Mechanical, single-concern; the milestone's lowest-
risk phase, drafted first.

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  "display only" constraint and phase table. This phase touches **only** color
  styling; no logic, no data, no new events.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the milestone README above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

There are **exactly seven** `Color::DarkGray` uses in `mcp/src/dashboard/`
(grep-verified — `grep -rn "DarkGray" mcp/src/` returns these and only these):

1. `mcp/src/dashboard/render.rs:130` — the filter-panel help footer:
   ```rust
   filter_lines.push(Line::from(Span::styled(
       " ↑↓/jk move · space toggle · f/Esc close",
       Style::new().fg(Color::DarkGray),
   )));
   ```
2. `mcp/src/dashboard/transcript.rs:59` — the `Prompt` event header color (3rd
   element of the match tuple):
   ```rust
   SessionEvent::Prompt { rendered } => (
       format!("prompt ({} chars)", rendered.chars().count()),
       Color::DarkGray,
       false,
       None,
   ),
   ```
3. `mcp/src/dashboard/transcript.rs:113` — the `Progress` event header color:
   ```rust
   SessionEvent::Progress { stage, .. } => {
       (format!("progress: {stage}"), Color::DarkGray, false, None)
   }
   ```
4. `mcp/src/dashboard/transcript.rs:127` — the `Metrics` event header color (3rd
   element of the match tuple):
   ```rust
   format!("metrics: {input_tokens} in / {output_tokens} out"),
   Color::DarkGray,
   ```
5. `mcp/src/dashboard/highlight.rs:119` — the diff **context** line (the `else`
   arm in `diff_body_lines`):
   ```rust
   } else {
       Line::from(Span::styled(
           format!("    {line}"),
           Style::new().fg(Color::DarkGray),
       ))
   };
   ```
6. `mcp/src/dashboard/highlight.rs:127` — the diff **overflow** marker:
   ```rust
   if overflow > 0 {
       result.push(Line::from(Span::styled(
           format!("    … ({overflow} more lines)"),
           Style::new().fg(Color::DarkGray),
       )));
   }
   ```
7. `mcp/src/dashboard/highlight.rs:180` — the syntax-highlight **overflow** marker
   (same shape as #6, in `highlighted_body_lines`):
   ```rust
   result.push(Line::from(Span::styled(
       format!("    … ({overflow} more lines)"),
       Style::new().fg(Color::DarkGray),
   )));
   ```

**The target color already exists in the tree** — use it verbatim. Two worked
examples, both `Color::Rgb(200, 200, 200)`:

- `transcript.rs:68` — `Some(plain_body_lines(raw, Color::Rgb(200, 200, 200)))`
  (Completion body).
- `highlight.rs:145` — the plain-text fallback:
  ```rust
  .map(|l| Line::from(Span::styled(l, Style::new().fg(Color::Rgb(200, 200, 200)))))
  ```

No test currently asserts on any of these colors (`grep -rn "DarkGray" mcp/`
shows zero matches outside the seven production sites), so no existing test
breaks; this phase **adds** color assertions to lock the change in.

## Spec

1. **Replace all seven `Color::DarkGray` with `Color::Rgb(200, 200, 200)`** at the
   exact sites listed in Current state — `render.rs:130`; `transcript.rs:59`,
   `:113`, `:127`; `highlight.rs:119`, `:127`, `:180`. Pure find-and-replace of the
   color value; change nothing else on those lines. After the edit,
   `grep -rn "DarkGray" mcp/src/` must return **zero** matches.

## Acceptance criteria

- [ ] `grep -rn "DarkGray" mcp/src/` returns no matches.
- [ ] `record_lines` for a `Prompt` event produces a header span whose
      `style.fg == Some(Color::Rgb(200, 200, 200))` (was `DarkGray`).
- [ ] `record_lines` for a `Progress` event and a `Metrics` event likewise
      produce header spans with `fg == Some(Color::Rgb(200, 200, 200))`.
- [ ] `diff_body_lines` renders a **context** line (one with no `+`/`-`/`@@`
      prefix) with `fg == Some(Color::Rgb(200, 200, 200))`.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy …`,
      `cargo fmt --all --check`, and `cargo test` all pass.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` blocks of the files
touched. Assert on the rendered span's `fg`, not on text (the text is unchanged).
Inspect a `Line`'s span style like this (the spans/style accessors ratatui
exposes):

```rust
let lines = record_lines(&rec(0, 0, SessionEvent::Prompt { rendered: "hi".into() }));
assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(200, 200, 200)));
```

- `prompt_header_uses_soft_white` in `transcript.rs` — a `Prompt` record's header
  span `fg` is `Rgb(200,200,200)`.
- `progress_header_uses_soft_white` in `transcript.rs` — a `Progress` record's
  header span `fg` is `Rgb(200,200,200)`.
- `metrics_header_uses_soft_white` in `transcript.rs` — a `Metrics` record's
  header span `fg` is `Rgb(200,200,200)`.
- `diff_context_line_uses_soft_white` in `highlight.rs` — feed
  `diff_body_lines` a small diff containing a context line (e.g. a ` unchanged`
  line with a leading space, alongside a `+added`/`-removed` line) and assert the
  context line's span `fg` is `Rgb(200,200,200)` while the `+`/`-` lines keep their
  green/red fg (a pinned **negative**: this phase must NOT recolor the diff add/
  remove styling — see Out of scope).

(Test names describe behavior; exact count/placement is yours. The diff test's
negative assertion on the `+`/`-` colors is the load-bearing one — it proves the
edit was surgical.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI styling has no
headless render harness; consistent with prior dashboard-panel phases M8/M10/M12
phase-07). Verification is the four pure-function color assertions above plus the
`grep` acceptance check.

## Authorizations

None. (No dependencies, no architecture-doc edit, no `SessionEvent`/config change.)

## Out of scope

Do **not** change any color other than the seven `DarkGray` sites. In particular,
leave these exactly as they are:

- The syntect theme colors in `highlight.rs:166-172` (`style.foreground.r/g/b`).
- The diff **add**/**remove** foreground+background colors (`highlight.rs:101-109`,
  `Rgb(180,242,180)`/`Rgb(0,48,0)` and `Rgb(242,180,180)`/`Rgb(64,0,0)`) and the
  diff hunk `Color::Cyan` (`highlight.rs:114`).
- All header colors that are **not** dark grey: `Color::Cyan`/`Green`/`Red`/
  `Blue`/`Magenta`/`Yellow`/`Reset` in `transcript.rs`; the `Color::Reset`
  Completion header (`transcript.rs:66`); the `Color::Yellow`/`Green`/`Red` panel
  styling in `panels.rs`; the filter-cursor `Color::Yellow` in `render.rs:121`.
- Anything in other crates. This is `mcp/src/dashboard/` only.

Do not refactor, extract a shared color constant, or "improve" surrounding code —
STANDARDS §2.2 (no premature abstraction). A literal `Color::Rgb(200, 200, 200)`
at each site matches the two existing call sites; keep that shape.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** rexyMCP executor
- **Scope deviations:** none
- **Calibration:** none

Re-ran all four gates independently: `cargo fmt --all --check`, `cargo build`
(zero warnings), `cargo clippy --all-targets --all-features -- -D warnings`,
and `cargo test` (725 passed, 0 failed, 2 ignored) — all green.
`grep -rn "DarkGray" mcp/src/` returns zero matches. The diff is surgical:
exactly the seven listed sites in `render.rs`/`transcript.rs`/`highlight.rs`,
each `Color::DarkGray` → `Color::Rgb(200, 200, 200)`, no adjacent code touched.
The four added tests are real (assert on span `fg`, fail if reverted to
`DarkGray`); `diff_context_line_uses_soft_white` carries the load-bearing
negative — it pins that the `+`/`-` lines keep their green/red
(`Rgb(180,242,180)`/`Rgb(242,180,180)`) and the hunk header stays `Cyan`,
proving the recolor did not bleed into the diff add/remove styling.

### Update — 2026-06-10 00:00 (started)

**Executor:** rexyMCP executor

Replaced all seven `Color::DarkGray` sites in `mcp/src/dashboard/` with `Color::Rgb(200, 200, 200)`. Added four unit tests to lock the color change in.

### Update — 2026-06-10 00:01 (complete)

**Executor:** rexyMCP executor

**Summary:** Replaced all 7 `Color::DarkGray` uses with `Color::Rgb(200, 200, 200)` across `render.rs`, `transcript.rs`, and `highlight.rs`. Added 4 unit tests (`prompt_header_uses_soft_white`, `progress_header_uses_soft_white`, `metrics_header_uses_soft_white`, `diff_context_line_uses_soft_white`).

**Files changed:**
- `mcp/src/dashboard/render.rs` — filter-panel help footer color
- `mcp/src/dashboard/transcript.rs` — Prompt/Progress/Metrics header colors + 3 tests
- `mcp/src/dashboard/highlight.rs` — diff context line + 2 overflow markers + 1 test

**Verification commands:**
- `cargo fmt --all --check` — passed
- `cargo build` — passed, zero new warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed
- `cargo test` — 722 passed, 0 failed, 2 ignored
- `grep -rn "DarkGray" mcp/src/` — zero matches (exit code 1)

**Grep proof:** `grep -rn "DarkGray" mcp/src/` returned zero matches, confirming all 7 sites were replaced.

**Notes for review:** None — mechanical find-and-replace, surgical, no adjacent code touched.

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact (TUI styling has no headless render harness; consistent with prior dashboard-panel phases M8/M10/M12). Verification is the four pure-function color assertions plus the grep acceptance check.
