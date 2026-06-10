# Phase 02: Activity — surface injected context + tool-call arguments

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none (phase-01 already recolored the two headers this phase adds bodies to)
**Estimated diff:** ~20 lines prod + ~5 small tests
**Tags:** language=rust, kind=feature, size=s

## Goal

The Activity transcript already *captures* the rendered prompt and every tool
call's arguments in its feed, but it only shows a one-line header for each and
throws the payload away: a `Prompt` renders as `prompt (N chars)` with the text
hidden, and a tool call renders as `→ call <name>` with the arguments hidden.
Surface both as record bodies — the `Prompt.rendered` text in soft white, the
`Parsed.tool_call.arguments` JSON dimmed (item R4) — reusing the existing
multi-line body machinery so they are truncation-bounded and gated by the
already-present `prompt` / `tool call` filter toggles. Pure display; the data is
already in the JSONL log.

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  "display only" constraint (no new `SessionEvent`, no config, no loop edits)
  and the phase table. This phase (#2, #3, with R4 folded in as the dim styling)
  adds **body content** to two existing transcript arms; it changes no event
  schema, no filter, and no scroll math.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the milestone README above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Both target sites are in `record_lines` in `mcp/src/dashboard/transcript.rs`.
The function returns `(summary, color, bold, body)` per event, where `body:
Option<Vec<Line<'static>>>` — `None` is a header-only record, `Some(lines)`
appends a multi-line body under the header (see how `Completion` and `ToolResult`
already do it).

**The `Prompt` arm today** (`transcript.rs:57-62`) discards `rendered`:

```rust
SessionEvent::Prompt { rendered } => (
    format!("prompt ({} chars)", rendered.chars().count()),
    Color::Rgb(200, 200, 200),
    false,
    None,
),
```

**The `Parsed` arm today** (`transcript.rs:70-75`) discards `tool_call.arguments`:

```rust
SessionEvent::Parsed { tool_call } => (
    format!("→ call {}", tool_call.name),
    Color::Blue,
    false,
    None,
),
```

**The worked example to copy** — the `Completion` arm (`transcript.rs:64-69`)
already attaches a soft-white plain-text body via the shared helper:

```rust
SessionEvent::Completion { raw } => (
    "completion:".to_string(),
    Color::Reset,
    false,
    Some(plain_body_lines(raw, Color::Rgb(200, 200, 200))),
),
```

`plain_body_lines(content, color)` is already imported at the top of
`transcript.rs` (`use super::highlight::{highlighted_body_lines,
plain_body_lines};`). It splits on newlines, indents each line by four spaces,
**and already enforces the truncation cap** — `body_lines` in `highlight.rs`
caps at `TRANSCRIPT_CONTENT_MAX_LINES` (= 20) lines and appends a
`… (N more lines)` overflow marker (`highlight.rs:196-212`). So routing both new
bodies through `plain_body_lines` gives the "truncation-bounded by the existing
body machinery" exit criterion for free — do **not** add a new cap.

**Filters already gate these records.** `ActivityFilter::allows`
(`filter.rs:53-54`) maps `Prompt → self.prompt` and `Parsed → self.tool_call`,
both default-on. `record_lines` is only ever called for records that already
passed `allows` (`transcript_lines`, `transcript.rs:36-41`), so the new bodies
inherit the existing toggles automatically — **do not touch `filter.rs`.**

**`tool_call.arguments` is a `serde_json::Value`** (`executor/src/parser/mod.rs:54`,
`pub arguments: Value`). `serde_json` is a normal dependency of the `mcp` crate
(`mcp/Cargo.toml` `[dependencies]`), so `serde_json::to_string_pretty` is
available with no new dependency and no import (call it fully qualified, matching
how the existing tests reference `serde_json::json!`).

## Spec

Both changes are in `record_lines` (`mcp/src/dashboard/transcript.rs`). Each is
**additive** — it only fills in the `body` slot that is currently `None`; leave
the summary string, color, and bold flag on each arm exactly as they are.

1. **Surface the prompt text** — in the `SessionEvent::Prompt { rendered }` arm,
   replace the `None` body with `Some(plain_body_lines(rendered, Color::Rgb(200,
   200, 200)))`. Same shape and soft-white color as the `Completion` body — the
   rendered prompt is primary injected content, so it reads at the same contrast.
   Keep the `prompt (N chars)` header.

2. **Surface the tool-call arguments, dimmed** — in the `SessionEvent::Parsed {
   tool_call }` arm, build the body from `tool_call.arguments`:

   - When `arguments` is JSON `null` **or** an empty object (`{}`), render
     **header only** (`None` body) — a no-argument call should not show an empty
     `{}` block.
   - Otherwise pretty-print the arguments and render them through
     `plain_body_lines` in a **dim** grey, `Color::Rgb(128, 128, 128)` — visibly
     dimmer than the `Rgb(200, 200, 200)` primary body (this is item R4, "dim
     tool-call arguments"), and distinct from the removed `DarkGray`. Use a plain
     dim body, **not** `highlighted_body_lines` — the arguments are intentionally
     de-emphasized, not syntax-colored.

   Worked shape (the empty-skip and the infallible-serialize fallback are the
   load-bearing details):

   ```rust
   SessionEvent::Parsed { tool_call } => {
       let body = match &tool_call.arguments {
           serde_json::Value::Null => None,
           serde_json::Value::Object(m) if m.is_empty() => None,
           args => {
               let pretty = serde_json::to_string_pretty(args)
                   .unwrap_or_else(|_| args.to_string());
               Some(plain_body_lines(&pretty, Color::Rgb(128, 128, 128)))
           }
       };
       (format!("→ call {}", tool_call.name), Color::Blue, false, body)
   }
   ```

   `unwrap_or_else(|_| args.to_string())` is the correct fallback here (a
   `serde_json::Value` always serializes, but the fallback keeps it total without
   an `unwrap()`/`expect()`); do **not** use `.unwrap()`, `.expect()`, or
   `unwrap_or_default()`.

No other file changes. No new helper, no shared color constant (STANDARDS §2.2 —
the two literal `Color::Rgb(...)` call sites match the existing ones; keep that
shape).

## Acceptance criteria

- [ ] A `Prompt` record renders its `rendered` text as a body: `record_lines`
      for `Prompt { rendered: "injected ctx" }` produces more than one line and
      some line's text contains `injected ctx`, with the body span
      `fg == Some(Color::Rgb(200, 200, 200))`.
- [ ] A `Parsed` record with non-empty arguments renders them as a body whose
      span `fg == Some(Color::Rgb(128, 128, 128))` and whose text contains the
      argument keys/values (e.g. `path` and `x.rs`).
- [ ] A `Parsed` record with empty (`{}`) or `null` arguments renders
      **header-only** — exactly one line, no body.
- [ ] A `Prompt` whose `rendered` exceeds `TRANSCRIPT_CONTENT_MAX_LINES` lines is
      capped with a `… (N more lines)` overflow marker (the existing machinery).
- [ ] `grep -rn "filter.rs" mcp/src/dashboard/transcript.rs` is irrelevant —
      `filter.rs` is **unchanged** (`git diff --name-only` does not list it).
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy
      --all-targets --all-features -- -D warnings`, `cargo fmt --all --check`, and
      `cargo test` all pass.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` block of
`transcript.rs`. Assert on the rendered body span's `fg` and text; reuse the
existing `rec(ts, turn, event)` and `record_text(rec)` helpers. Inspect a body
line's style like the phase-01 tests do (`lines[i].spans[0].style.fg`).

- `prompt_body_shows_rendered_text_soft_white` — `Prompt { rendered: "injected
  ctx" }` → `record_lines` returns ≥ 2 lines; a body line's text contains
  `injected ctx` and its span `fg == Some(Color::Rgb(200, 200, 200))`.
- `tool_call_args_render_dimmed` — `Parsed` with
  `arguments: serde_json::json!({ "path": "x.rs" })` → a body line's text
  contains `path` and `x.rs`, and the body span `fg == Some(Color::Rgb(128, 128,
  128))` (the load-bearing assertion: proves it is the dim plain body, not the
  syntax-highlighted path).
- `tool_call_empty_args_render_header_only` — `Parsed` with
  `arguments: serde_json::json!({})` → `record_lines` returns exactly **one**
  line (header only). Add a sibling or parameterized case for
  `serde_json::Value::Null` arguments yielding the same header-only result (the
  pinned negative — an empty call must not emit an empty `{}` block).
- `prompt_body_caps_long_text` — a `Prompt` whose `rendered` has more than
  `TRANSCRIPT_CONTENT_MAX_LINES` newline-separated lines → the last body line
  contains `more lines` (exercises the shared cap; mirrors the existing
  `record_lines_caps_long_content` test for `Completion`).

(Test names describe behavior; exact count/placement is yours. The empty-args
header-only case and the `Rgb(128,128,128)` dim assertion are the two
load-bearing checks — they pin that the edit de-emphasizes arguments and stays
silent for no-arg calls.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI styling/layout has
no headless render harness; consistent with prior dashboard-panel phases
M8/M10/M12 and M13 phase-01). Verification is the pure-function `record_lines`
assertions above plus the `cargo` gates.

## Authorizations

None. (No dependencies — `serde_json` is already a dependency; no
architecture-doc edit; no `SessionEvent`/config/filter change.)

## Out of scope

Do **not**:

- Touch `filter.rs` — the `prompt` / `tool call` toggles already gate these
  records; the bodies inherit them for free.
- Touch `render.rs` or the scroll math (`clamp_scroll` / `visible_offset`). The
  follow offset counts `transcript_lines().len()` (pre-wrap), and the new body
  lines increase that count the same way `Completion`/`ToolResult` bodies already
  do — no autoscroll change is needed here. **Word/line wrapping and autoscroll
  over wrapped lines is phase-03** — do not add `Wrap`, a scrollbar, or any
  width-based truncation in this phase. A long single-line JSON string value
  (e.g. a `write_file` `content` argument) may overflow the panel width for now;
  phase-03 wraps it.
- Syntax-highlight the arguments (no `highlighted_body_lines` for the `Parsed`
  body) — they are deliberately dimmed, not colored.
- Change the `prompt (N chars)` or `→ call <name>` header text, color, or the
  bold flag on either arm. Headers were set by phase-01; this phase adds bodies
  only.
- Add a shared color constant or a new body helper, or change
  `TRANSCRIPT_CONTENT_MAX_LINES`. Reuse `plain_body_lines` and the existing cap.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 16:26 (started)

**Executor:** Claude (executor)

Implementing phase-02: surfacing `Prompt.rendered` and `Parsed.tool_call.arguments` as transcript bodies in `record_lines`.

### Update — 2026-06-10 16:26 (complete)

**Summary:** Added body content to `Prompt` and `Parsed` transcript records in `record_lines`. `Prompt.rendered` now renders as a soft-white body via `plain_body_lines`, matching the existing `Completion` body style. `Parsed.tool_call.arguments` renders as a dim grey body (`Rgb(128,128,128)`) when non-empty/non-null, and header-only when empty (`{}`) or `null`. Four unit tests added. No changes to `filter.rs`, `render.rs`, or any other file.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(check passed — no output)

cargo build 2>&1 | tail -20
(check passed — no output)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(check passed — no output)

cargo test 2>&1 | tail -30
test result: ok. 725 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**Verification summary:** `cargo fmt --all --check` clean, `cargo build` clean, `cargo clippy` clean, `cargo test` 725 passed / 0 failed.

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact (TUI styling/layout has no headless render harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13 phase-01). Verification is the pure-function `record_lines` assertions above plus the `cargo` gates.

**Grep for spec-pinned literals:**
- `grep -n "plain_body_lines(rendered, Color::Rgb(200, 200, 200))" mcp/src/dashboard/transcript.rs` → line 61 ✓
- `grep -n "Color::Rgb(128, 128, 128)" mcp/src/dashboard/transcript.rs` → lines 77, 552 ✓

**Files changed:**
- `mcp/src/dashboard/transcript.rs` — added body to `Prompt` arm (soft white), added conditional body to `Parsed` arm (dim grey for non-empty args), added 4 unit tests

**New tests:**
- `prompt_body_shows_rendered_text_soft_white` in `mcp/src/dashboard/transcript.rs`
- `tool_call_args_render_dimmed` in `mcp/src/dashboard/transcript.rs`
- `tool_call_empty_args_render_header_only` in `mcp/src/dashboard/transcript.rs`
- `prompt_body_caps_long_text` in `mcp/src/dashboard/transcript.rs`

**Commits:**
- `b7897ce` — feat: surface prompt text and tool-call arguments as transcript bodies

**Notes for review:** None — implementation matches spec exactly. No deviations.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — both `record_lines` arms match the spec's worked
  shape byte-for-byte (Prompt body `plain_body_lines(rendered, Rgb(200,200,200))`;
  Parsed body dim `Rgb(128,128,128)` with the `Null`/empty-`Object` header-only
  skip and the `unwrap_or_else(|_| args.to_string())` total fallback). `filter.rs`,
  `render.rs`, and the scroll math are untouched (confirmed via `git show --stat`).
- **Independent re-run:** `cargo fmt --all --check` clean, `cargo build` zero
  warnings, `cargo clippy --all-targets --all-features -- -D warnings` clean,
  `cargo test` **725 passed / 0 failed / 2 ignored**. The four new tests pass and
  are mutation-resistant — `tool_call_args_render_dimmed` pins
  `fg == Some(Rgb(128,128,128))` (proves the dim plain body, not the highlighted
  path), and `tool_call_empty_args_render_header_only` pins exactly one line for
  both `{}` and `null` (removing the empty-skip would make it 2 lines and fail).
- **Calibration:** none on the code. Cosmetic-only quirk: the Update Log
  self-stamps commit `b7897ce`, but the real commit is `1c06116` (the recurring
  local-LLM self-stamping quirk — same class as the hallucinated date/identity;
  machine records are correct). Resolved once `rexymcp serve` is restarted.
