# Phase 01: Move `last update:` under `duration:` + capitalize panel labels

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** done
**Depends on:** none
**Estimated diff:** ~40 lines (mostly string-literal edits)
**Tags:** language=rust, kind=feature, size=xs

## Goal

Two cosmetic Session/Budget/Reclaim panel fixes: (1) render the `last update:`
freshness line directly under the `duration:` line inside the Session panel
instead of appending it from `render.rs`; (2) capitalize the first letter of
every panel label so they read as titles (`Phase:`, `Tokens in:`, `Events:`)
rather than lowercase fragments.

## Architecture references

Read before starting:

- `mcp/src/dashboard/panels.rs` — `session_lines`, `last_update_line`,
  `budget_lines`, `reclaim_lines`, `dollars_saved_line` (the label strings).
- `mcp/src/dashboard/render.rs:144–151` — where `session_lines` is built and
  `last_update_line` is currently appended.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### `session_lines` (`panels.rs:74–115`)

`session_lines(summary, now_ms)` pushes, in order: `phase:`, `session:`,
`model:` (optional), `state:` (styled), `duration:` (optional), and a
`turn {N}, stage {stage}` line. It does **not** push the `last update:` line.

### `render.rs:144–151` — Session panel assembly

```rust
let mut session = session_lines(&data.summary, now_ms);
if let Some(line) = last_update_line(&data.summary, now_ms) {
    session.push(line);
}
let session_inner_width = session_area.width.saturating_sub(2) as usize;
if let Some(line) = spinner_line(state.spinner, session_inner_width) {
    session.push(line);
}
```

So today the order rendered is: …, `duration:`, `turn/stage`, `last update:`,
spinner. The `last update:` line is appended **after** `turn/stage`, not under
`duration:`.

### `last_update_line` (`panels.rs:409–420`)

```rust
pub(crate) fn last_update_line(summary: &StatusSummary, now_ms: u64) -> Option<Line<'static>> {
    let ts = summary.last_ts?;
    let age_str = status::humanize_age(now_ms.saturating_sub(ts));
    let line = match summary.update_interval_avg_ms {
        Some(avg) => format!(
            "last update: {age_str} ago (avg: {})",
            status::humanize_age(avg),
        ),
        None => format!("last update: {age_str} ago"),
    };
    Some(Line::from(line))
}
```

It is `pub(crate)` and pure. Keep it `pub(crate)` — `session_lines` will call it.

### Current label strings (the ones to capitalize)

- `session_lines`: `"phase: …"`, `"session: …"`, `"model: …"`, `"state: …"`,
  `"duration: …"`, `"turn {N}, stage {stage}"`.
- `last_update_line`: `"last update: …"`.
- `budget_lines` (`panels.rs:319–376`): `"(no metrics yet)"`, `"tokens in:  …"`,
  `"tokens out: …"`, `"tok/s: …"`, `"context: — (unmeasured)"`,
  `"context: {pct}% …"`.
- `reclaim_lines` (`panels.rs:146–185`): `"events: …"`, `"freed: …"`,
  `"ratio: …"`, `"filter: …"`, `"evict: …"`, `"dedupe: …"`, `"(no reclaim yet)"`.
- `dollars_saved_line` (`panels.rs:392–403`): `"$ saved: —"`, `"$ saved: $…"`.

## Spec

### 1. Move `last update:` into `session_lines`, under `duration:`

In `panels.rs`, inside `session_lines`, immediately **after** the
`duration:` push block (the `if let Some(dur) = session_duration_ms(...)` block
ending around line 106) and **before** the `turn {N}, stage` push, add:

```rust
if let Some(line) = last_update_line(summary, now_ms) {
    lines.push(line);
}
```

`session_lines` already has both `summary` and `now_ms` in scope, so no
signature change.

Then in `render.rs`, **delete** the now-duplicate append block:

```rust
// DELETE these three lines from render.rs:
if let Some(line) = last_update_line(&data.summary, now_ms) {
    session.push(line);
}
```

Leave the `let mut session = session_lines(&data.summary, now_ms);` line and the
`session_inner_width` / `spinner_line` block intact. If removing the
`last_update_line` call leaves an unused import in `render.rs`'s `use
super::panels::{…}` list, drop `last_update_line` from that import list (the
compiler/clippy will flag it).

The resulting Session-panel order becomes: `Phase:`, `Session:`, `Model:`,
`State:`, `Duration:`, `Last update:`, `Turn …`, spinner.

### 2. Capitalize every panel label

Capitalize the first letter of each label string in `session_lines`,
`last_update_line`, `budget_lines`, `reclaim_lines`. Exact replacements:

| Function | Before | After |
|---|---|---|
| `session_lines` | `phase: {phase}` | `Phase: {phase}` |
| `session_lines` | `session: {session}` | `Session: {session}` |
| `session_lines` | `model: {model}` | `Model: {model}` |
| `session_lines` | `state: {state}` | `State: {state}` |
| `session_lines` | `duration: {…}` | `Duration: {…}` |
| `session_lines` | `turn {N}, stage {stage}` | `Turn {N}, stage {stage}` |
| `last_update_line` | `last update: {…} ago` | `Last update: {…} ago` |
| `last_update_line` | `last update: {…} ago (avg: {…})` | `Last update: {…} ago (avg: {…})` |
| `budget_lines` | `(no metrics yet)` | `(No metrics yet)` |
| `budget_lines` | `tokens in:  {…}` | `Tokens in:  {…}` |
| `budget_lines` | `tokens out: {…}` | `Tokens out: {…}` |
| `budget_lines` | `tok/s: {…}` | `Tok/s: {…}` |
| `budget_lines` | `context: — (unmeasured)` | `Context: — (unmeasured)` |
| `budget_lines` | `context: {pct}% …` (both the `{used}/{window}` and bare forms) | `Context: {pct}% …` |
| `reclaim_lines` | `events: {…}` | `Events: {…}` |
| `reclaim_lines` | `freed: {…} tokens` | `Freed: {…} tokens` |
| `reclaim_lines` | `ratio: {…}x` | `Ratio: {…}x` |
| `reclaim_lines` | `filter: {…}` | `Filter: {…}` |
| `reclaim_lines` | `evict: {…}` | `Evict: {…}` |
| `reclaim_lines` | `dedupe: {…}` | `Dedupe: {…}` |
| `reclaim_lines` | `(no reclaim yet)` | `(No reclaim yet)` |

**Do NOT change** `dollars_saved_line`'s `"$ saved: …"` — `$` is a symbol, not a
word; it stays lowercase `saved`. Leave it exactly as-is.

Keep the column alignment in `budget_lines` intact: `tokens in:` had two spaces
after the colon (`tokens in:  {in_toks}`) to align with `tokens out:`. Preserve
that — `Tokens in:  {in_toks}` keeps the same two spaces so the numbers stay
column-aligned.

### 3. Update the affected tests

The existing panel tests assert on the old lowercase strings. Update each
assertion to the capitalized form (and the new `last update:` position). Search
for the literal asserted strings and bump them. Notably:

- Any `session_lines` test asserting `"phase: …"`, `"session: …"`,
  `"ended (…)"` substrings, `"duration: …"`, `"turn …"`.
- The test pinning that `session_lines` does **NOT** contain `last update:`
  (it now **does** — invert that assertion to a must-contain, since the line
  moved into `session_lines`). Find it via
  `grep -n "last update" mcp/src/dashboard/panels.rs`.
- `budget_lines` / `reclaim_lines` tests asserting the lowercase labels.

Pin the new position with one assertion: in a `session_lines` test that has a
`last_ts` set, assert the rendered lines contain `Last update:` **after**
`Duration:` and **before** `Turn`. (Index-compare the two line positions, or
assert the `Last update:` line index is greater than the `Duration:` line index.)

## Acceptance criteria

- [ ] `session_lines` pushes the `Last update:` line between `Duration:` and
      `Turn`; `render.rs` no longer appends `last_update_line`.
- [ ] Every Session/Budget/Reclaim label is capitalized per the table above;
      `$ saved:` is unchanged.
- [ ] `Tokens in:` and `Tokens out:` remain column-aligned (two spaces after
      `Tokens in:`).
- [ ] A `session_lines` test asserts `Last update:` appears after `Duration:`
      and before `Turn`.
- [ ] `cargo build` zero warnings; `cargo clippy` clean (no unused-import
      warning from the dropped `last_update_line` use in `render.rs`).
- [ ] All four gates pass on an independent re-run.

## Test plan

- Update `session_lines` tests in `panels.rs` to the capitalized labels.
- `session_lines_places_last_update_under_duration` in `panels.rs` — build a
  summary with `last_ts` and `started_at` set; assert the `Last update:` line
  index falls between the `Duration:` line index and the `Turn` line index.
  Mutation-resistant: an implementation that appends `last update:` at the end
  (old behavior) fails the "before `Turn`" half.
- Update `budget_lines` / `reclaim_lines` label assertions to capitalized forms.
- If a `session_lines_omits_last_update` test exists, rename/invert it to
  `session_lines_includes_last_update` and assert presence.

## End-to-end verification

Run `cargo run -p rexymcp -- dashboard --repo . --config rexymcp.toml` against a
session log (or describe the rendered Session panel from a unit-rendered set of
lines if no live session is handy) and confirm the Session panel shows
`Last update:` directly under `Duration:` with capitalized labels. If no live
session is available at review time, this is a TUI render — restate that the
unit tests pin the line order and the label strings, consistent with prior
dashboard-panel phases (M13/M15) that declared the live TUI render E2E-N/A.

## Authorizations

None. No new dependencies. No `docs/architecture.md` change.

## Out of scope

- The spinner (phase 02), the `Milestone:` line (phase 03), task scrolling
  (phase 04), highlighting (phase 05).
- `dollars_saved_line`'s `$ saved:` label — explicitly left unchanged.
- The CLI `format_status` renderer in `status.rs` — it is a separate
  non-dashboard renderer; do not touch it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-11 16:42 (progress)

Executor started phase 01. Implementing `last update:` relocation into `session_lines` and capitalizing all Session/Budget/Reclaim panel labels.

### Update — 2026-06-11 16:43 (complete)

**Summary:** Moved `last_update_line` call from `render.rs` into `session_lines` in `panels.rs` (positioned between `Duration:` and `Turn` lines). Capitalized the first letter of every label in Session, Budget, and Reclaim panels per the spec table. `$ saved:` in `dollars_saved_line` left unchanged. Removed the now-unused `last_update_line` import from `render.rs`. Updated all affected test assertions to the new capitalized strings. Added `session_lines_places_last_update_under_duration` test to pin the new line order with index assertions. Inverted `session_lines_omits_last_update` to `session_lines_includes_last_update_when_ts_present`.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(checks clean, exit 0)

cargo build 2>&1 | tail -20
(compile clean, exit 0)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(lint clean, exit 0)

cargo test 2>&1 | tail -30
test result: ok. 734 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

E2E N/A — this is a TUI render; unit tests pin the line order (`session_lines_places_last_update_under_duration` asserts index ordering) and the label strings, consistent with prior dashboard-panel phases (M13/M15) that declared the live TUI render E2E-N/A.

Grep for pinned literal `Last update:` in panels.rs: 8 matches across source and tests, confirming the literal landed correctly.

**Files changed:**
- `mcp/src/dashboard/panels.rs` — moved `last_update_line` into `session_lines`, capitalized all Session/Budget/Reclaim labels, updated all test assertions, added position-pinning test
- `mcp/src/dashboard/render.rs` — removed `last_update_line` call and its import
- `docs/dev/milestones/M17-dashboard-polish-3/phase-01-labels.md` — status flip to review + Update Log
- `docs/dev/milestones/M17-dashboard-polish-3/README.md` — phase table row updated to review

**New tests:**
- `session_lines_places_last_update_under_duration` in `mcp/src/dashboard/panels.rs`

**Commits:**
- (pending) — feat: move last update under duration and capitalize panel labels

**Notes for review:** None. Straightforward cosmetic change, all tests pass.

### Review verdict — 2026-06-11

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none
- **Calibration:** none

Independent re-run of all four gates green: `cargo fmt --all --check` clean,
`cargo build` zero warnings, `cargo clippy --all-targets --all-features -- -D
warnings` clean, `cargo test` 734 executor + 349 mcp pass / 0 failed / 2
ignored (mcp +1 for the new position test). Production `session_lines`
(`panels.rs:74–117`) matches the spec byte-for-byte: `last_update_line` pushed
between `Duration:` and `Turn`; every Session/Budget/Reclaim label capitalized;
`Tokens in:` keeps its two-space alignment. `render.rs` dropped the duplicate
`last_update_line` append and the now-unused import (no clippy unused-import
warning). `$ saved:` left unchanged per spec (confirmed by the unchanged
`dollars_saved` tests). No `unwrap`/`expect`/`panic!` in production paths — all
grep hits fall inside the `#[cfg(test)]` block. The new
`session_lines_places_last_update_under_duration` is mutation-resistant
(asserts `Duration` idx < `Last update` idx < `Turn` idx; the old append-at-end
behavior fails the before-`Turn` half), and `session_lines_omits_last_update`
was correctly inverted to `session_lines_includes_last_update_when_ts_present`.
E2E is the live TUI render (E2E-N/A per prior M13/M15 dashboard-panel
precedent); unit tests pin both the line order and the label strings. Clean
85-turn first-try; commit `2e75185` (feat). No `SessionEvent`/config/`Cargo.toml`
change. The recurring local-LLM Update-Log clock/identity self-stamp did **not**
recur (stamp `2026-06-11 16:42`, plausibly real — M11 phase-06 datetime
injection is live post-restart). The stale `(pending)` commit line in the
executor's Update Log is cosmetic — the `feat:` commit `2e75185` did land.
