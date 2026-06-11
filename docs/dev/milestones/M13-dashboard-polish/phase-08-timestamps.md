# Phase 08: Activity — per-event relative timestamps

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none for code (independent of phases 01–07; touches only the
Activity transcript header builder). Phase-05 already established the
"relative-to-session-start" timing idiom (`session_duration_ms` + `humanize_age`)
this phase reuses, but there is no code dependency.
**Estimated diff:** ~60 lines (one `relative_ts` helper ~6, `transcript_lines`
timestamp-prefix rewrite ~12, tests ~40)
**Tags:** language=rust, kind=feature, size=s

## Goal

Give each Activity transcript line a **relative timestamp** so the user can see
*when* each event happened relative to the session's start (user enhancement R2,
the last M13 phase). Today every transcript header reads `[t4] progress: verify`
— the turn number but no time. This phase prepends a dim `[+3m12s]`-style gutter
so the header reads `[+3m12s] [t4] progress: verify`.

The offset is `record.ts − first_record.ts`, formatted by the **existing**
`crate::status::humanize_age` helper (the same `5s` / `3m12s` / `1h04m` buckets
the Session panel's `duration:` line uses). This is relative-to-session-**start**,
not relative-to-**now**: a record's timestamp is fixed the moment it is logged and
does **not** churn frame-to-frame as the wall clock advances — exactly the
property a scrolling transcript wants.

Pure presentation — no feed, config, or executor change. Every byte rendered is
already in the JSONL log: `SessionRecord.ts` has carried a millis timestamp on
every record since the log format was defined; it was simply never surfaced in the
Activity panel.

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  **display-only** constraint and the phase table (this is phase 08, item R2). This
  phase touches **only** `mcp/src/dashboard/transcript.rs`. It adds **no**
  `SessionEvent` variant, no config, no `status.rs`/`panels.rs`/`render.rs`/
  `filter.rs` change.
- `docs/architecture.md` § Status #13 (M13 "Panel polish" thread — "Each Activity
  line carries a relative timestamp"). This phase implements that sentence.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### `transcript_lines` flat-maps records to lines with no timestamp

In `mcp/src/dashboard/transcript.rs`, `transcript_lines` (lines 17–27) filters the
records and flat-maps each through `record_lines`:

```rust
pub(crate) fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
) -> Vec<Line<'static>> {
    let visible: Vec<_> = records.iter().filter(|r| filter.allows(&r.event)).collect();
    if visible.is_empty() {
        vec![Line::from("(no activity yet)")]
    } else {
        visible.iter().flat_map(|r| record_lines(r)).collect()
    }
}
```

`record_lines(rec)` (lines 32–188) builds, per record, a **header line** plus
optional **body lines**. The header is assembled at line 177:

```rust
let header_text = format!("[t{}] {}", rec.turn, summary);
// ...
let mut lines = vec![Line::from(Span::styled(header_text, style))];
if let Some(body) = body {
    lines.extend(body);
}
```

`render.rs` calls **`transcript_lines`** (render.rs:213, 219) — never `record_lines`
directly. So adding the timestamp inside `transcript_lines` reaches the entire live
render path while leaving `record_lines` untouched.

### `SessionRecord` carries a millis timestamp

In `executor/src/store/sessions/event.rs`:

```rust
pub struct SessionRecord {
    pub ts: u64,        // unix millis, monotonic non-decreasing across the log
    pub turn: usize,
    pub event: SessionEvent,
}
```

Records are appended chronologically, so **`records[0].ts` is the session's earliest
timestamp** — the baseline this phase measures from. (Phase-05 defined
`StatusSummary.started_at` as exactly this earliest ts; this phase does not need the
summary — it derives the baseline locally from the records slice it already has.)

### `humanize_age` already formats a millis span — reuse it

In `mcp/src/status.rs` (line 364), `humanize_age` is `pub(crate)` and produces the
compact buckets the Session panel's `duration:` line uses:

```rust
pub(crate) fn humanize_age(age_ms: u64) -> String {
    let secs = age_ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}
```

Its tested buckets (`status.rs:751-754`): `humanize_age(5_000) == "5s"`,
`humanize_age(192_000) == "3m12s"`, `humanize_age(3_840_000) == "1h04m"`. **Reuse
this — do not write a second time formatter.**

### Dim-grey secondary styling already exists — match it

The `Parsed` tool-call-arguments body (transcript.rs:61) and the `<think>` reasoning
lines (phase-04) render at `Color::Rgb(128, 128, 128)` — the codebase's established
"dim metadata, still legible" tier (distinct from the `Rgb(200, 200, 200)` soft-white
of primary secondary text, and far above the `Color::DarkGray` that M13 phase-01
retired). The timestamp gutter is metadata of exactly this kind — use the **same**
`Color::Rgb(128, 128, 128)`.

## Spec

All changes are in `mcp/src/dashboard/transcript.rs`. No other production file.

### 1. Add the `relative_ts` helper — `transcript.rs`

Add a small private pure helper near `preview` (e.g. just above it):

```rust
/// Relative timestamp for a transcript record: `+{humanized}` elapsed since the
/// session's first record (`base_ts`). `+0s` at the baseline; `saturating_sub`
/// guards a record that reads before the baseline (shouldn't happen — records are
/// chronological — but stays panic-free). Reuses the Session-panel duration
/// formatter so the buckets match (`5s` / `3m12s` / `1h04m`).
fn relative_ts(ts: u64, base_ts: u64) -> String {
    format!("+{}", crate::status::humanize_age(ts.saturating_sub(base_ts)))
}
```

(`crate::status::humanize_age` is `pub(crate)`; call it fully-qualified or add a
`use crate::status::humanize_age;` — your call. Do **not** make `humanize_age`
`pub` or move it; it is already reachable as `pub(crate)`.)

### 2. Prepend the timestamp gutter in `transcript_lines` — `transcript.rs`

Rewrite the non-empty branch of `transcript_lines` to (a) compute the baseline from
the **full** `records` slice and (b) prepend a dim timestamp span to the **first
line** (the header) of each record's rendered output. The body lines are left
unchanged.

```rust
pub(crate) fn transcript_lines(
    records: &[SessionRecord],
    filter: &ActivityFilter,
) -> Vec<Line<'static>> {
    let visible: Vec<_> = records.iter().filter(|r| filter.allows(&r.event)).collect();
    if visible.is_empty() {
        return vec![Line::from("(no activity yet)")];
    }
    let base_ts = records.first().map(|r| r.ts).unwrap_or(0);
    visible
        .iter()
        .flat_map(|r| {
            let mut lines = record_lines(r);
            if let Some(header) = lines.first_mut() {
                let mut spans = Vec::with_capacity(header.spans.len() + 1);
                spans.push(Span::styled(
                    format!("[{}] ", relative_ts(r.ts, base_ts)),
                    Style::new().fg(Color::Rgb(128, 128, 128)),
                ));
                spans.append(&mut header.spans);
                *header = Line::from(spans);
            }
            lines
        })
        .collect()
}
```

**Three load-bearing properties — get these exactly right:**

- **Baseline is the first record in the *full* `records` slice, NOT the first
  *visible* (post-filter) record.** Compute `base_ts` from `records.first()`, before
  filtering. If a filter hides the session's opening events, the surviving events
  must still show their true offset from session start (e.g. a record 4s after start
  shows `[+4s]` even when the start record is filtered out — not `[+0s]`). This is
  pinned by a negative test below.
- **The timestamp goes only on the header (`lines.first_mut()`), never on body
  lines.** Completion text, tool output, and tool-call argument bodies keep their
  existing rendering with no `+`-prefix.
- **`record_lines`'s signature stays `record_lines(rec)`.** Do not thread `base_ts`
  into `record_lines`. The timestamp is a `transcript_lines`-layer concern; keeping
  `record_lines` unchanged leaves its ~15 test call sites and the header-color tests
  (`prompt_header_uses_soft_white`, `tool_call_args_render_dimmed`, etc., which call
  `record_lines` **directly** and read `lines[0].spans[0]`) compiling and passing
  untouched.

`Color`, `Style`, `Span`, and `Line` are already imported at the top of
`transcript.rs` (lines 1–4) — no new import beyond the optional `humanize_age` one.

### 3. Tests — `transcript.rs`

See the Test plan. Add to the existing `#[cfg(test)] mod tests` block (`use super::*`
is in scope; the `rec(ts, turn, event)` helper at transcript.rs:216 already takes a
`ts`).

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `transcript_lines` prepends a `[+{humanized}]` timestamp span to the **header**
      line of every rendered record; the offset is `record.ts − records[0].ts`
      formatted by `crate::status::humanize_age`. The baseline is the **first record
      in the full slice**, not the first record that survives the filter.
- [ ] The timestamp span is styled `Color::Rgb(128, 128, 128)` and is the **first**
      span of the header line; the original header span (with its event color) follows
      it.
- [ ] Body lines (completion text, tool output, tool-call arguments) carry **no**
      timestamp prefix.
- [ ] `record_lines`'s signature is unchanged (`record_lines(rec)`); the empty case
      is unchanged (`vec![Line::from("(no activity yet)")]`, no timestamp).
- [ ] `relative_ts(ts, base_ts)` returns `"+0s"` when `ts == base_ts`, `"+5s"` for a
      5000ms offset, `"+3m12s"` for 192000ms, and `"+0s"` (not a panic) when
      `ts < base_ts`.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test` all
      pass.
- [ ] `git diff --name-only` lists only `mcp/src/dashboard/transcript.rs` (plus this
      phase doc and the README row). No `status.rs`, `panels.rs`, `render.rs`,
      `filter.rs`, `Cargo.toml`, `SessionEvent`, or config edit.

## Test plan

Names describe behavior; exact count and placement are yours. The **load-bearing**
tests are `relative_ts_formats_offset_from_base` (pins the format + `+` prefix +
saturating guard) and
`transcript_lines_timestamp_relative_to_first_record_not_first_visible` (pins the
baseline is the first *record*, not the first *visible* line — the boundary a naive
"use the first visible record" impl gets wrong).

- `relative_ts_formats_offset_from_base` — `relative_ts(1000, 1000) == "+0s"`;
  `relative_ts(6000, 1000) == "+5s"`; `relative_ts(193_000, 1000) == "+3m12s"`;
  `relative_ts(500, 1000) == "+0s"` (saturating — no panic, no underflow).
- `transcript_lines_prefixes_relative_timestamp` — two records:
  `rec(1000, 0, start_event())` and `rec(4000, 1, progress_event(1, "verify"))`.
  Render via `transcript_lines(&records, &ActivityFilter::default())`. Assert the
  first header line's text contains both `"[+0s]"` and `"[t0]"`, and some line
  contains both `"[+3s]"` and `"[t1] progress: verify"`. (4000−1000 = 3000ms → `3s`.)
- `transcript_lines_timestamp_relative_to_first_record_not_first_visible` — **the
  load-bearing negative.** Three records where the first is filtered out:
  `rec(1000, 0, SessionEvent::Prompt { rendered: "ctx".into() })` (hidden) and
  `rec(5000, 1, progress_event(1, "build"))` (visible). Build an `ActivityFilter`
  that disables the prompt event (see the existing `filter.rs` toggles / the
  `tool_call_empty_args_render_header_only` style of constructing events; if
  constructing a filtered `ActivityFilter` is awkward, instead assert via a
  non-default filter helper already in `filter.rs` tests). Assert the visible
  progress record's header contains `"[+4s]"` (5000−1000), **not** `"[+0s]"`. A
  baseline taken from the first *visible* record would wrongly show `"[+0s]"`.
  - If wiring a real `ActivityFilter` that hides a specific event is more than a
    couple of lines, an acceptable equivalent: pass two records where the first is a
    *visible* event at ts 1000 and assert the second (ts 5000) shows `"[+4s]"` — this
    still pins "baseline = records[0].ts, offsets are differences, not per-record
    absolute." But prefer the filtered version if `filter.rs` exposes a simple
    constructor, since it pins the *full-slice* baseline directly.
- `transcript_lines_timestamp_only_on_header_not_body` — one record:
  `rec(1000, 0, SessionEvent::Completion { raw: "alpha\nbeta".into() })`. Baseline =
  1000 → header shows `"[+0s]"`. Assert exactly **one** rendered line contains the
  `"[+0s]"` token (the header), and the body lines `"alpha"` / `"beta"` do **not**
  contain `"+0s"`. (Confirms the prefix is header-only.)
- `transcript_lines_timestamp_span_is_dim_grey` — render a single record; assert the
  header line's **first** span has `style.fg == Some(Color::Rgb(128, 128, 128))` and
  its content starts with `"["` and contains `"+"`, and the **second** span carries
  the original header text (e.g. `"[t0]"`).
- Confirm the existing `transcript_lines_flatmaps_records` (asserts line **count** 5)
  and `transcript_lines_empty_placeholder` still pass **unmodified** — prepending a
  span does not change the line count, and the empty path is untouched.

(Reading a line's text: `format!("{l}")`. Reading the timestamp span:
`line.spans[0]`. Records are built with the `rec(ts, turn, event)` helper at
transcript.rs:216, which already takes the `ts` this phase reads.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13
phases 01–07). Verification is the `relative_ts` / `transcript_lines` pure-function
assertions plus the `cargo` gates. The panel-composition path is exercised by the
live binary; the line-builder function that drives it is fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — only `transcript.rs` changes. **No `Cargo.toml`
      edit.**
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- Add a new `SessionEvent` variant, a config field, or change `SessionRecord` /
  `SessionEvent`. The `ts` field already exists; this phase only surfaces it. If you
  think you need a schema change, **stop and file a blocker** — you have left M13's
  display-only scope.
- Change `record_lines`'s signature or thread `base_ts` into it. The timestamp is
  added one layer up, in `transcript_lines`. Keeping `record_lines(rec)` unchanged is
  what keeps its existing call sites and header-color tests green — this is
  deliberate, not an oversight.
- Make the timestamp relative to **now** (`now_ms`). It is relative to session start,
  so it is stable per record and `transcript_lines` needs no clock parameter and
  `render.rs` needs no edit. (A relative-to-now timestamp would re-render every line's
  age each frame — explicitly not wanted.)
- Write a second time/duration formatter. Reuse `crate::status::humanize_age`.
- Touch `render.rs` (the `transcript_lines(&data.records, …)` calls are unchanged),
  `panels.rs`, `status.rs` (beyond *calling* `humanize_age`), `filter.rs`, or the
  `rexymcp status` CLI text path (`format_status` — the CLI is not the dashboard).
- Add a timestamp to body lines, or to the empty `(no activity yet)` placeholder.
- Reformat, recolor, or restyle any other part of the transcript. Only the header
  gutter prefix is added.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-11 01:22 (started)

**Executor:** Claude (Sonnet 4.5)

Implementing per-event relative timestamps in Activity transcript headers.

### Update — 2026-06-11 01:22 (complete)

**Executor:** Claude (Sonnet 4.5)

**Summary:** Added `relative_ts` helper and timestamp gutter to `transcript_lines`. Each Activity transcript header now shows `[+{humanized}]` elapsed since session start, styled in dim grey `Rgb(128,128,128)`. Baseline is derived from `records.first().ts` (full slice, pre-filter), ensuring filtered-out events don't shift the baseline. `record_lines` signature is unchanged.

**Files changed:**
- `mcp/src/dashboard/transcript.rs` — added `relative_ts` helper, rewrote `transcript_lines` non-empty branch, added 5 tests

**Verification commands:**
```
$ cargo fmt --all --check
(no output — clean)

$ cargo build 2>&1 | tail -20
(0 warnings)

$ cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(0 warnings)

$ cargo test 2>&1 | tail -30
test result: ok. 725 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**One-line verification summary:** fmt clean, build 0 warnings, clippy 0 warnings, 725 tests passed (5 new + 720 existing).

**Grep proof — `relative_ts` literal landed:**
```
$ grep -n 'relative_ts' mcp/src/dashboard/transcript.rs
195:fn relative_ts(ts: u64, base_ts: u64) -> String {
30:                    format!("[{}] ", relative_ts(r.ts, base_ts)),
664:    fn relative_ts_formats_offset_from_base() {
665:        assert_eq!(relative_ts(1000, 1000), "+0s");
666:        assert_eq!(relative_ts(6000, 1000), "+5s");
667:        assert_eq!(relative_ts(193_000, 1000), "+3m12s");
669:        assert_eq!(relative_ts(500, 1000), "+0s");
```

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact (consistent with prior M13 phases). Verification is the `relative_ts` / `transcript_lines` pure-function assertions plus cargo gates.

**Notes for review:** None — implementation matches spec exactly. Clippy `field_reassign_with_default` lint was fixed by using struct update syntax in the test.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — only `mcp/src/dashboard/transcript.rs` (plus this phase
  doc + the README row) touched; no `render.rs`/`status.rs`/`panels.rs`/`filter.rs`/
  `Cargo.toml`/`SessionEvent`/config edit. `record_lines`'s signature was correctly
  left unchanged (the timestamp is added one layer up in `transcript_lines`), so its
  call sites and the header-color tests stayed green.
- **Calibration:** none. Clean 33-turn first-try with full bookkeeping (status flip +
  Update Log + single `feat:` commit `14cc751`). All four gates re-run green
  independently (725 mcp+executor pass, 0 failed, 2 ignored). Production clean of new
  `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]` (the `records.first().map(..).unwrap_or(0)`
  is a safe Option default, unreachable as `None` given the empty-`visible` early
  return). Load-bearing tests confirmed mutation-resistant at review: mutating the
  baseline from `records.first()` → `visible.first()` made
  `transcript_lines_timestamp_relative_to_first_record_not_first_visible` fail
  (`[+0s]` instead of `[+4s]`), and `relative_ts_formats_offset_from_base` pins the
  `+0s`/`+5s`/`+3m12s` buckets + saturating guard. The dirty-tree-at-dispatch quirk
  (phase-06 calibration) did **not** recur — the draft was committed before dispatch,
  so the executor committed cleanly on its own. E2E correctly declared N/A (TUI, no
  headless harness). Cosmetic-only quirk: the Update Log self-stamps `2026-06-11 01:22`
  / "Claude (Sonnet 4.5)" — the recurring local-LLM clock/identity quirk (machine
  records correct; the executor is Qwen/Qwen3.6-27B-FP8; fixed once `rexymcp serve` is
  restarted to pick up M11 phase-06's datetime injection — still pending).
