# Phase 05: Session/Budget — session `duration:` + move `last update:` to Budget

**Milestone:** M13 — Dashboard Polish
**Status:** done
**Depends on:** none (independent of phases 01–04; touches the Session/Budget
panels and `StatusSummary`, which those phases did not change)
**Estimated diff:** ~120 lines (2 new pure helpers ~30, session_lines edit ~6,
`StatusSummary` field + summarize ~5, render wiring ~5, tests ~75)
**Tags:** language=rust, kind=feature, size=m

## Goal

Two timing improvements to the header panels (user items #4, #5):

1. **Session panel gains a `duration:` line** — the wall-clock age of the session,
   live-growing while running and frozen once it ends. This needs a new
   `started_at` capture in the status summary.
2. **The `last update:` freshness line moves from the Session panel to the Budget
   panel.** It is a metrics-freshness signal and reads better next to the token
   counts; the Session panel's identity lines (phase/session/state/duration) stay
   clean.

Pure presentation — no feed, config, or executor change. The data already exists:
`duration` is derived from record timestamps already in the log; `last update:` is
the same line, rendered in a different panel.

## Architecture references

Read before starting:

- `docs/dev/milestones/M13-dashboard-polish/README.md` — the milestone's
  **display-only** constraint and the phase table. This phase touches **only**
  `mcp/src/status.rs` (one `StatusSummary` field + one `summarize` assignment),
  `mcp/src/dashboard/panels.rs` (the two panel builders + two new pure helpers),
  and `mcp/src/dashboard/render.rs` (one Budget-composition edit). It adds **no**
  `SessionEvent` variant, no config, and no signature change to any panel builder.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the milestone README above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### `last update:` lives in the Session panel today

In `mcp/src/dashboard/panels.rs`, `session_lines` (lines 27–83) renders the
`last update:` block (lines 64–75). `session_lines` already receives `now_ms`:

```rust
pub(crate) fn session_lines(
    summary: &StatusSummary,
    now_ms: u64,
    spinner: Option<usize>,
) -> Vec<Line<'static>> {
    // ... phase / session / model / state / turn-stage lines ...

    if let Some(ts) = summary.last_ts {
        let age_ms = now_ms.saturating_sub(ts);
        let age_str = status::humanize_age(age_ms);
        let line = match summary.update_interval_avg_ms {
            Some(avg) => format!(
                "last update: {age_str} ago (avg: {})",
                status::humanize_age(avg),
            ),
            None => format!("last update: {age_str} ago"),
        };
        lines.push(Line::from(line));
    }

    if let Some(frame) = spinner {
        // ... spinner line ...
    }

    lines
}
```

### Budget is composed in `render.rs`, with an optional line appended

`budget_lines` (panels.rs:216) takes only `&StatusSummary` (no clock). In
`render.rs` (lines 147–151) the optional `dollars_saved_line` is **pushed onto the
returned vec** rather than computed inside `budget_lines`:

```rust
let mut budget = budget_lines(&data.summary);
if let Some(line) = dollars_saved_line(&data.summary, rates) {
    budget.push(line);
}
frame.render_widget(panel(" Budget ", budget), budget_area);
```

`dollars_saved_line` (panels.rs:289–300) is the established **optional-line helper**
pattern you will copy for the new `last_update_line`:

```rust
pub(crate) fn dollars_saved_line(
    summary: &StatusSummary,
    rates: BudgetRates,
) -> Option<Line<'static>> {
    let in_tok = summary.last_input_tokens?;
    // ...
    Some(Line::from(format!("$ saved: ${saved:.2}")))
}
```

### `StatusSummary` is `Default`-built; `summarize` sets `last_ts` as the max ts

In `mcp/src/status.rs`, `summarize` (lines 98–248) builds the summary mutably from
a `Default`. It already tracks the **latest** record timestamp (lines 113–116):

```rust
summary.last_ts = Some(match summary.last_ts {
    Some(prev) => prev.max(rec.ts),
    None => rec.ts,
});
```

`started_at` is the symmetric **earliest** timestamp. There is no `started_at` /
`duration` handling anywhere in `mcp/src/` today — confirm with
`grep -rn "started_at\|duration:" mcp/src/` returning nothing (greenfield).

`status::humanize_age(ms) -> String` (status.rs:337, `pub(crate)`) renders a
millisecond span as `"5s"` / `"3m12s"` / `"1h04m"` — reuse it for both lines.

## The chosen change shape (read this — it avoids the known stall)

`budget_lines` has **one production call site and nine test call sites**
(panels.rs:620/637/655/673/682/697/712/735/754). **Do NOT add a `now_ms`
parameter to `budget_lines`** — that signature change would break all ten call
sites and is exactly the multi-site mechanical-churn pattern that has stalled this
executor before (see the calibration history). Instead, follow the
`dollars_saved_line` precedent: write a **new** pure `last_update_line(summary,
now_ms) -> Option<Line>` helper and **push it onto the Budget vec in `render.rs`**,
which already holds `now_ms`. `budget_lines`' signature and all nine of its test
calls stay **untouched**.

Likewise, `session_lines` already takes `now_ms`, so the new `duration:` line needs
**no** signature change there either. The net effect: no panel-builder signature
changes anywhere, no call-site cascade.

## Spec

All changes are in `status.rs`, `panels.rs`, and `render.rs`. No other files.

### 1. Capture `started_at` — `status.rs`

Add one field to `StatusSummary` (near `last_ts`, line 27), with a doc-comment:

```rust
/// Timestamp (unix millis) of the *earliest* record — when the session began.
/// Symmetric with `last_ts`; drives the Session panel's `duration:` line.
pub started_at: Option<u64>,
```

In `summarize`, alongside the existing `last_ts` assignment (lines 113–116), add
the symmetric earliest-timestamp fold:

```rust
summary.started_at = Some(match summary.started_at {
    Some(prev) => prev.min(rec.ts),
    None => rec.ts,
});
```

(`StatusSummary` derives `Serialize`; this adds a harmless additive `started_at`
field to the `rexymcp status --json` output. No existing test pins the full
serialized shape — leave the JSON path otherwise alone.)

### 2. Add the duration helper — `panels.rs`

Add this pure helper near `session_lines`:

```rust
/// Wall-clock session duration in ms: **live** (`now_ms − started_at`) while the
/// session is running, **frozen** (`last_ts − started_at`) once it has ended.
/// `None` for an empty log (no `started_at`). `saturating_sub` guards a clock that
/// reads behind the first record.
pub(crate) fn session_duration_ms(summary: &StatusSummary, now_ms: u64) -> Option<u64> {
    let start = summary.started_at?;
    let end = if summary.ended.is_some() {
        summary.last_ts.unwrap_or(start)
    } else {
        now_ms
    };
    Some(end.saturating_sub(start))
}
```

### 3. Render `duration:` in the Session panel — `panels.rs`

In `session_lines`, **add** a duration line and **remove** the `last update:` block.

Add the duration line immediately after the `state:` line is pushed (the bold
`state: …` push around lines 47–56), so the order reads
phase / session / model / state / **duration** / turn-stage:

```rust
if let Some(dur) = session_duration_ms(summary, now_ms) {
    lines.push(Line::from(format!("duration: {}", status::humanize_age(dur))));
}
```

Then **delete** the entire `if let Some(ts) = summary.last_ts { … }` block (the
`last update:` lines 64–75) from `session_lines` — it moves to step 4. The
`spinner` block and everything else stay.

### 4. Add the `last update:` helper — `panels.rs`

Add this pure helper near `dollars_saved_line`, lifting the exact age + interval
logic you just removed from `session_lines`:

```rust
/// "last update: …" freshness line for the Budget panel — the age of the most
/// recent record, with the average update interval when enough records exist.
/// `Some` whenever the session has at least one record (`last_ts`); `None` for an
/// empty log. Mirrors the optional-line shape of `dollars_saved_line`.
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

### 5. Compose `last update:` into the Budget panel — `render.rs`

Change the Budget composition (render.rs:147–151) to **prepend** the
`last update:` line (so the freshness line sits at the top of Budget, visible even
in the pre-metrics `(no metrics yet)` state), then keep the existing
`budget_lines` + `dollars_saved_line`:

```rust
let mut budget = Vec::new();
if let Some(line) = last_update_line(&data.summary, now_ms) {
    budget.push(line);
}
budget.extend(budget_lines(&data.summary));
if let Some(line) = dollars_saved_line(&data.summary, rates) {
    budget.push(line);
}
frame.render_widget(panel(" Budget ", budget), budget_area);
```

Add `last_update_line` to the `use super::panels::{…}` import block (render.rs:10–13).

## Acceptance criteria

Verifiable by `cargo test` and reading the diff.

- [ ] `summarize` sets `started_at` to the earliest record timestamp (and `last_ts`
      to the latest, unchanged); for an empty log both are `None`.
- [ ] `session_duration_ms` returns `now_ms − started_at` while running
      (`ended.is_none()`) and `last_ts − started_at` once ended — the ended value
      does **not** depend on `now_ms`.
- [ ] The Session panel renders a `duration:` line and renders **no** `last update:`
      line: `session_lines(&summary, now, None)` output contains a line starting
      `duration:` and **no** line containing `last update:`.
- [ ] The Budget panel renders the `last update:` line: with `last_ts` set,
      `last_update_line(&summary, now)` is `Some` and its text contains
      `last update:`; for a default (empty) summary it is `None`.
- [ ] The `last update:` line still carries `(avg: …)` when
      `update_interval_avg_ms` is `Some`, and omits it when `None` — the behavior is
      identical to today's Session-panel rendering, just relocated.
- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, and `cargo test`
      all pass.
- [ ] `git diff --name-only` lists only `mcp/src/status.rs`,
      `mcp/src/dashboard/panels.rs`, and `mcp/src/dashboard/render.rs` (plus this
      phase doc and the README row). No `Cargo.toml`, no `filter.rs`/`transcript.rs`/
      `event_loop.rs`, no `SessionEvent`/config edit, no `budget_lines` /
      `session_lines` **signature** change.

## Test plan

Add unit tests in the existing `#[cfg(test)] mod tests` blocks (`use super::*` is in
scope). Names describe behavior; exact count and placement are yours. The
**load-bearing** tests are `session_duration_ms_ended_uses_last_ts` (pins the
running-vs-frozen distinction — a mutation that uses `now_ms` when ended fails it)
and `last_update_line_none_for_empty_log` together with the Session-panel
must-NOT-contain assertion (pins that the line genuinely *moved*, not duplicated).

In `status.rs`:

- `summarize_captures_started_at` — over records at ts 100 / 300, assert
  `started_at == Some(100)` and `last_ts == Some(300)`.
- `summarize_empty_log_has_no_started_at` — `summarize(&[]).started_at == None`
  (extend the existing `summarize_empty_log_is_all_none` or add a sibling).

In `panels.rs` (duration + the relocated line):

- `session_duration_ms_running_uses_now` — `started_at: Some(1000)`, `ended: None`,
  `now_ms = 4000` → `Some(3000)`.
- `session_duration_ms_ended_uses_last_ts` — `started_at: Some(1000)`,
  `ended: Some("complete")`, `last_ts: Some(5000)`, `now_ms = 9000` → `Some(4000)`
  (**not** `8000`; the frozen-on-end negative).
- `session_duration_ms_none_for_empty_log` —
  `session_duration_ms(&StatusSummary::default(), 5000) == None`.
- `session_lines_shows_duration_while_running` — `started_at: Some(1000)`,
  `ended: None`, `now_ms = 4000` → some line is `"duration: 3s"`.
- `session_lines_omits_last_update` — with `last_ts: Some(1000)` set,
  `session_lines(&summary, 4000, None)` produces **no** line containing
  `last update:` (the line moved out of the Session panel).
- `last_update_line_shows_age` — `last_ts: Some(1000)`, `now_ms = 4000` → `Some`,
  text contains `"last update: 3s ago"`.
- `last_update_line_none_for_empty_log` —
  `last_update_line(&StatusSummary::default(), 4000) == None`.
- `last_update_line_shows_interval_stats` — `last_ts: Some(5000)`,
  `update_interval_avg_ms: Some(2000)`, `now_ms = 5000` → text contains `"avg:"`
  (relocate the old `session_lines_shows_update_interval_stats` assertion here).
- `last_update_line_omits_interval_stats_without_enough_data` — same but
  `update_interval_avg_ms: None` → text does **not** contain `"avg:"`.

**Revise the existing Session-panel tests** whose assertions targeted the moved
line (they will otherwise fail or assert nothing meaningful):

- `session_lines_shows_turn_stage_and_age` (panels.rs:341) — drop the `"3s ago"`
  assertion (the last-update line is gone from Session); keep the turn/stage
  assertions. Rename to reflect the narrowed behavior if you like.
- `session_lines_omits_age_when_no_ts` (panels.rs:356) — repurpose to a
  duration-omission test (default summary → no `duration:` line), or remove it in
  favor of `session_duration_ms_none_for_empty_log`.
- `session_lines_shows_update_interval_stats` (panels.rs:367) and
  `session_lines_omits_interval_stats_without_enough_data` (panels.rs:382) — these
  asserted the avg suffix on the now-relocated line; **move** them to the
  `last_update_line_*` tests above (the two listed) rather than leaving them against
  `session_lines`.

(Reading a line's text: `format!("{l}")`. The existing `session_lines_*` and
`budget_lines_*` tests are templates for the assertion shape.)

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact (TUI rendering has no
headless harness; consistent with prior dashboard-panel phases M8/M10/M12 and M13
phase-01/02/03/04). Verification is the pure-function assertions
(`summarize` `started_at`, `session_duration_ms`, `last_update_line`) plus the
`session_lines` relocation assertions and the `cargo` gates. The actual panel
*rendering* is exercised by the live binary; the line-builder functions that drive
it are fully covered.

## Authorizations

None.

- [ ] May add dependencies: **no** — only `StatusSummary` (a field), `panels.rs`,
      and `render.rs` change. **No `Cargo.toml` edit.**
- [ ] May touch `docs/architecture.md`: **no**.

## Out of scope

Do **not**:

- Add a new `SessionEvent` variant, a config field, or any `StatusSummary` field
  beyond `started_at`. If you think you need one, **stop and file a blocker**:
  you have left M13's display-only scope.
- Add a `now_ms` parameter to `budget_lines` (or otherwise change `budget_lines` /
  `session_lines` **signatures**). Use the `last_update_line` helper + `render.rs`
  push instead — this is deliberate, to avoid the multi-site call-site churn.
- Touch `mcp/src/status.rs`'s `format_status` (the `rexymcp status` **CLI text**
  path). It is a separate renderer and keeps its own `last update:` line; this phase
  is the **dashboard** panels only. Do not add `duration:` to the CLI text output.
- Touch `filter.rs`, `transcript.rs`, `event_loop.rs`, or the wrap/scroll machinery.
  Your extra/moved panel lines render through the existing `panel(...)` path for free.
- Change `humanize_age`, the spinner block, or any other Budget/Session line beyond
  the `duration:` add and the `last update:` relocation.
- Reorder or restyle the existing Budget lines (tokens / tok/s / context / `$ saved`).
  Only **prepend** `last update:`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-10 18:10 (started)

**Executor:** Claude (Sonnet 4.5)
**Changes:** Implementing session `duration:` line and relocating `last update:` to Budget panel.

### Update — 2026-06-10 18:10 (complete)

**Summary:** Added `started_at` field to `StatusSummary`, wired `session_duration_ms` helper into Session panel, relocated `last update:` line to Budget panel via `last_update_line` helper. No panel-builder signature changes — `budget_lines` and `session_lines` signatures untouched.

**Files changed:**
- `mcp/src/status.rs` — added `started_at` field to `StatusSummary`, `summarize` captures earliest timestamp, added 2 tests (`summarize_captures_started_at`, extended `summarize_empty_log_is_all_none`)
- `mcp/src/dashboard/panels.rs` — added `session_duration_ms` helper, added `last_update_line` helper, `session_lines` renders `duration:` and no longer renders `last update:`, revised 4 existing session tests, added 7 new tests (3 for `session_duration_ms`, 4 for `last_update_line`)
- `mcp/src/dashboard/render.rs` — Budget composition now prepends `last_update_line`, added `last_update_line` to imports
- `docs/dev/milestones/M13-dashboard-polish/phase-05-timing.md` — status `review`, Update Log
- `docs/dev/milestones/M13-dashboard-polish/README.md` — phase table row updated to `review`

**Commit:** `feat: add session duration line and move last update to Budget panel`

**Verification commands (all clean):**
- `cargo fmt --all --check`: exit 0
- `cargo build`: exit 0, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0
- `cargo test`: 725 passed, 0 failed, 2 ignored

**End-to-end verification:** N/A — TUI rendering has no headless harness; consistent with prior M8/M10/M12/M13 phases. All line-builder functions covered by pure-function assertions.

**Grep proof for spec-pinned literals:**
```
$ grep -rn "started_at" mcp/src/ | wc -l  → 13 matches
$ grep -rn "duration:" mcp/src/dashboard/panels.rs | wc -l → 5 matches (doc + impl + tests)
$ grep -rn "last_update_line" mcp/src/dashboard/ | wc -l → 11 matches (def + import + render + tests)
```

**Notes for review:** None — implementation matches spec exactly.

### Review verdict — 2026-06-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude (Sonnet 4.5)
- **Scope deviations:** none
- **Calibration:** none

**Re-run results (independent):** `cargo fmt --all --check` clean; `cargo build`
zero warnings; `cargo clippy --all-targets --all-features -- -D warnings` exit 0;
`cargo test` 725 passed / 0 failed / 2 ignored. All 11 new/revised phase-05 tests
present and green. The `dollars_saved_line` precedent was followed — `budget_lines`
and `session_lines` signatures untouched, no call-site cascade. Production helpers
(`session_duration_ms`, `last_update_line`) use `?`/`unwrap_or` only; no banned
`unwrap`/`expect`/`panic`/`unsafe`/`TODO`/`#[allow]` in the touched files. The
load-bearing `session_duration_ms_ended_uses_last_ts` test pins the frozen-on-end
distinction (asserts `Some(4000)`, not `8000`) and is mutation-resistant.
