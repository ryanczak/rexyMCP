# Phase 08e: Fold reclaim events into `StatusSummary` + the live dashboard

**Milestone:** M10 — Context optimization
**Status:** review
**Depends on:** phase-03 (`OutputFiltered` event — done), phase-04 (`ReadEvicted`
— done), phase-06 (`ReadDeduped` — done), phase-08a–08d (context-efficiency
surfacing — done). This is the **live-view** sibling of 08b/08c/08d: those folded
the reclaim signal into the *post-hoc* `runs` / `scorecard` tables; this folds it
into the *in-flight* `rexymcp status` / dashboard view.
**Estimated diff:** ~160 lines (incl. tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Make M10's three Arc-A/Arc-B reclaim levers visible **while a phase is running**.
Today `summarize` (the session-log → live-status fold) silently drops
`OutputFiltered`, `ReadEvicted`, and `ReadDeduped` into its `_ => {}` catch-all,
so the dashboard's reclaim panel only ever shows compaction. This phase folds
those three variants into `StatusSummary` counters, surfaces them in the live
dashboard panel (repurposing the existing **Compactions** panel into an aggregate
**Reclaim** panel), and adds a `reclaimed:` line to the `rexymcp status` text
output. Why now: it's the last in-scope M10 phase — it closes the loop so every
lever that lands is visible both post-hoc (08b–08d) **and** live.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Context optimization (M10)" — the per-lever
  `SessionEvent` reclaim instrumentation and the live-status path.
- `docs/dev/milestones/M10-context-optimization/README.md` § "Phases" (row 08e)
  and § "Measurement is per-lever".

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The three reclaim variants already exist — DO NOT add or modify them

`OutputFiltered`, `ReadEvicted`, and `ReadDeduped` are existing
`SessionEvent` variants (added in phases 03/04/06). Their shapes, verbatim from
`executor/src/store/sessions/event.rs:89-110`:

```rust
OutputFiltered {
    tokens_before: usize,
    tokens_after: usize,
    filter: String,
},
ReadEvicted {
    path: String,
    reads_evicted: usize,
    tokens_reclaimed: usize,
},
ReadDeduped {
    path: String,
    tokens_saved: usize,
    prior_turn: usize,
},
```

**This is NOT a new-variant phase.** These variants are *already* matched
everywhere a new variant would need wiring — `mcp/src/log_query.rs:28-30`
(`event_type_str`), `mcp/src/dashboard/transcript.rs:149-175` (per-event activity
lines), `mcp/src/dashboard/filter.rs:60-62` (`ActivityFilter`). **You touch none
of those.** The only place these three variants are *unhandled* is `summarize`'s
`_ => {}` catch-all. The entire blast radius of this phase is that one catch-all
plus the panel/text renderers that read the new counters. There is no exhaustive
`match SessionEvent` to extend, so there is no match-arm wall.

### `summarize` and its catch-all (`mcp/src/status.rs:79-166`)

`summarize(records) -> StatusSummary` folds a session log into the live-status
struct. It builds a `StatusSummary::default()` and mutates it per record. The
**compaction arm is your worked example** — copy its shape for the three levers
(`mcp/src/status.rs:155-163`):

```rust
SessionEvent::Compaction {
    tokens_before,
    tokens_after,
    ..
} => {
    summary.compaction_count += 1;
    summary.compaction_tokens_before += *tokens_before;
    summary.compaction_tokens_after += *tokens_after;
}
_ => {} // Prompt, Completion, Parsed remain intentionally unread
```

The `_ => {}` arm **must stay** — `Prompt` / `Completion` / `Parsed` are still
intentionally unread. You add three arms *before* it, not replace it.

### The compaction counter fields (`StatusSummary`, `mcp/src/status.rs:59-64`)

```rust
/// Number of `Compaction` records seen so far.
pub compaction_count: usize,
/// Sum of `tokens_before` across all `Compaction` records.
pub compaction_tokens_before: usize,
/// Sum of `tokens_after` across all `Compaction` records.
pub compaction_tokens_after: usize,
```

`StatusSummary` derives `Default` (`mcp/src/status.rs:16`) and **every** literal
in the codebase builds it via `..StatusSummary::default()` (grep-verified: the 21
test literals in `mcp/src/dashboard/panels.rs` all spread; `summarize` uses
`StatusSummary::default()` then mutates). **Adding fields is therefore purely
additive — there is no struct literal to update.** This is the deliberately
chosen low-churn shape (contrast 08d's 3 cross-file literals).

### The dashboard panel you will repurpose (`mcp/src/dashboard/panels.rs:74-90`)

```rust
/// Compactions panel: count, freed tokens, compression ratio.
pub(crate) fn compactions_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    if summary.compaction_count == 0 {
        return vec![Line::from("(no compactions)")];
    }
    let before = summary.compaction_tokens_before;
    let after = summary.compaction_tokens_after;
    let mut lines = vec![
        Line::from(format!("events: {}", summary.compaction_count)),
        Line::from(format!("freed: {} tokens", before.saturating_sub(after))),
    ];
    if after != 0 {
        let ratio = before as f64 / after as f64;
        lines.push(Line::from(format!("ratio: {ratio:.1}x")));
    }
    lines
}
```

It is rendered in the dashboard header at `mcp/src/dashboard/render.rs:90-93`:

```rust
frame.render_widget(
    panel(" Compactions ", compactions_lines(&data.summary)),
    compactions_area,
);
```

and imported at `mcp/src/dashboard/render.rs:11`. The header is a fixed 3-panel
horizontal split (Session · Budget · Compactions). **You do not change the header
geometry** — you repurpose the existing Compactions slot into an aggregate
Reclaim slot by renaming + extending the content function and retitling the panel.

### The text-status renderer (`mcp/src/status.rs:226-256`)

`format_status(summary, now_ms) -> String` builds the `rexymcp status` human
output line-by-line (phase / model / state / turn / message / last-update). You
append one reclaim line. The compact-token convention elsewhere uses raw integer
token counts (the compaction panel prints `freed: 400 tokens`); match that — raw
integers, no `k` suffix.

### The `rexymcp status` CLI and its `--json` path (`mcp/src/main.rs:232-259`)

`rexymcp status --repo <path> [--session <needle>] [--json]` resolves the latest
session log, runs `summarize`, and prints either `format_status` (text) or
`serde_json::to_string_pretty(&summary)` (`--json`). **`StatusSummary` derives
`Serialize`, so the new counter fields serialize through `--json` automatically —
no `main.rs` change.** This is your headless end-to-end artifact.

## Spec

Numbered tasks in execution order.

### The reclaim-tokens convention (pin this — identical across all three surfaces)

"Tokens reclaimed" per lever, matching the per-event field each variant already
carries (the same sources 08a/08b/08c/08d sum):

- **filter** (`OutputFiltered`): `tokens_before - tokens_after` (saturating).
- **evict** (`ReadEvicted`): `tokens_reclaimed`.
- **dedupe** (`ReadDeduped`): `tokens_saved`.

Each lever also carries an **event count** (number of records of that variant).

### 1. Add six fields to `StatusSummary` (`mcp/src/status.rs:64`)

After the `compaction_tokens_after` field, add (additive — no literal to update):

```rust
/// Number of `OutputFiltered` records (Arc-A boundary filter) seen so far.
pub output_filtered_count: usize,
/// Sum of tokens reclaimed by the boundary filter (`tokens_before - tokens_after`).
pub output_filtered_tokens: usize,
/// Number of `ReadEvicted` records (Arc-B superseded-read eviction) seen so far.
pub read_evicted_count: usize,
/// Sum of `tokens_reclaimed` across all `ReadEvicted` records.
pub read_evicted_tokens: usize,
/// Number of `ReadDeduped` records (Arc-B redundant-read dedupe) seen so far.
pub read_deduped_count: usize,
/// Sum of `tokens_saved` across all `ReadDeduped` records.
pub read_deduped_tokens: usize,
```

### 2. Add three arms to `summarize`, before the `_ => {}` catch-all (`mcp/src/status.rs:163`)

Insert immediately after the `Compaction` arm and before `_ => {}`:

```rust
SessionEvent::OutputFiltered {
    tokens_before,
    tokens_after,
    ..
} => {
    summary.output_filtered_count += 1;
    summary.output_filtered_tokens += tokens_before.saturating_sub(*tokens_after);
}
SessionEvent::ReadEvicted {
    tokens_reclaimed, ..
} => {
    summary.read_evicted_count += 1;
    summary.read_evicted_tokens += *tokens_reclaimed;
}
SessionEvent::ReadDeduped { tokens_saved, .. } => {
    summary.read_deduped_count += 1;
    summary.read_deduped_tokens += *tokens_saved;
}
```

Leave the `_ => {}` arm in place.

### 3. Rename + extend the panel function (`mcp/src/dashboard/panels.rs:74-90`)

Rename `compactions_lines` → `reclaim_lines` and extend it to render the three
levers below the compaction summary. **Keep the existing compaction lines
verbatim** (`events:` / `freed:` / `ratio:`) so the compaction behavior is
unchanged; append a per-lever line for each lever whose count is `> 0`; change the
all-empty placeholder to `(no reclaim yet)`. The complete new function:

```rust
/// Reclaim panel: compaction plus the three M10 per-lever reclaim sources.
pub(crate) fn reclaim_lines(summary: &StatusSummary) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if summary.compaction_count > 0 {
        let before = summary.compaction_tokens_before;
        let after = summary.compaction_tokens_after;
        lines.push(Line::from(format!("events: {}", summary.compaction_count)));
        lines.push(Line::from(format!(
            "freed: {} tokens",
            before.saturating_sub(after)
        )));
        if after != 0 {
            let ratio = before as f64 / after as f64;
            lines.push(Line::from(format!("ratio: {ratio:.1}x")));
        }
    }
    if summary.output_filtered_count > 0 {
        lines.push(Line::from(format!(
            "filter: {} calls, {} freed",
            summary.output_filtered_count, summary.output_filtered_tokens
        )));
    }
    if summary.read_evicted_count > 0 {
        lines.push(Line::from(format!(
            "evict: {} reads, {} freed",
            summary.read_evicted_count, summary.read_evicted_tokens
        )));
    }
    if summary.read_deduped_count > 0 {
        lines.push(Line::from(format!(
            "dedupe: {} reads, {} saved",
            summary.read_deduped_count, summary.read_deduped_tokens
        )));
    }

    if lines.is_empty() {
        return vec![Line::from("(no reclaim yet)")];
    }
    lines
}
```

### 4. Repoint the dashboard render (`mcp/src/dashboard/render.rs`) — two sites

This is the **complete** list of `compactions_lines` references (grep-verified —
`grep -rn 'compactions_lines' mcp/`). Both are in `render.rs`:

```
mcp/src/dashboard/render.rs:11   — the `use crate::dashboard::panels::{...}` import list
mcp/src/dashboard/render.rs:91   — the panel() call
```

- **Import** (line 11): change `compactions_lines` → `reclaim_lines` in the import list.
- **Call** (line 91): change `panel(" Compactions ", compactions_lines(&data.summary))`
  → `panel(" Reclaim ", reclaim_lines(&data.summary))`. (Retitle *and* rename.)

Do not change the header `Layout::horizontal` constraints or `compactions_area` —
the slot stays put, only its title and content function change.

### 5. Add a `reclaimed:` line to `format_status` (`mcp/src/status.rs`)

In `format_status`, after the `last update:` block (`mcp/src/status.rs:250-253`)
and before `lines.join("\n")`, append a reclaim summary line **only when total
reclaim is nonzero**:

```rust
let reclaimed = summary.output_filtered_tokens
    + summary.read_evicted_tokens
    + summary.read_deduped_tokens
    + summary
        .compaction_tokens_before
        .saturating_sub(summary.compaction_tokens_after);
if reclaimed > 0 {
    lines.push(format!(
        "reclaimed: {reclaimed} tokens (filter {}, evict {}, dedupe {}, compaction {})",
        summary.output_filtered_tokens,
        summary.read_evicted_tokens,
        summary.read_deduped_tokens,
        summary
            .compaction_tokens_before
            .saturating_sub(summary.compaction_tokens_after),
    ));
}
```

When no reclaim has happened (all sources zero), **no** `reclaimed:` line is
emitted — a clean run's status output is unchanged.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes (only files this phase touched).
- [ ] `cargo test` passes (existing + new).
- [ ] `StatusSummary` has the six new fields (`output_filtered_count`,
      `output_filtered_tokens`, `read_evicted_count`, `read_evicted_tokens`,
      `read_deduped_count`, `read_deduped_tokens`), all `usize`.
- [ ] `summarize` over a log containing one `OutputFiltered { tokens_before: 1000,
      tokens_after: 200, .. }` yields `output_filtered_count == 1` and
      `output_filtered_tokens == 800`.
- [ ] `summarize` over a log with one `ReadEvicted { tokens_reclaimed: 500, .. }`
      yields `read_evicted_count == 1`, `read_evicted_tokens == 500`; one
      `ReadDeduped { tokens_saved: 300, .. }` yields `read_deduped_count == 1`,
      `read_deduped_tokens == 300`.
- [ ] `summarize` still folds `Compaction` exactly as before (existing compaction
      tests pass unchanged) and still ignores `Prompt`/`Completion`/`Parsed`.
- [ ] `reclaim_lines` over a summary with all four sources zero returns a single
      `(no reclaim yet)` line.
- [ ] `reclaim_lines` over a summary with `output_filtered_count: 3,
      output_filtered_tokens: 2048` renders a line containing `filter: 3 calls`
      and `2048 freed`; with compaction present, the `events:`/`freed:` compaction
      lines still render.
- [ ] `format_status` emits a `reclaimed:` line when any reclaim occurred, and
      **no** `reclaimed:` line when all reclaim sources are zero.
- [ ] `rexymcp status --repo <fixture> --json` output JSON carries the six new
      fields with the folded values (verified end-to-end, quoted in the Update Log).

## Test plan

All hermetic, no IO except the `TempDir`-scoped session-log fixture in the e2e.
Behavior pinned; names below are the floor, not a cap.

**`summarize` fold (`mcp/src/status.rs` `mod tests`, using the existing `rec(...)`
helper + new event constructors mirroring the existing `compaction(...)` helper):**

- `summarize_folds_output_filtered_count_and_tokens` — one `OutputFiltered
  { tokens_before: 1000, tokens_after: 200, filter: "cargo" }` → `count == 1`,
  `tokens == 800`. **Mutation-resistant:** uses `tokens_before - tokens_after`,
  not either operand alone.
- `summarize_folds_read_evicted` — one `ReadEvicted { tokens_reclaimed: 500, .. }`
  → `read_evicted_count == 1`, `read_evicted_tokens == 500`.
- `summarize_folds_read_deduped` — one `ReadDeduped { tokens_saved: 300, .. }` →
  `read_deduped_count == 1`, `read_deduped_tokens == 300`.
- `summarize_reclaim_levers_default_zero_when_absent` — a log with only
  `SessionStart` + `Progress` → all six new fields are `0` (the must-stay-zero
  negative case; guards against an arm firing on the wrong variant).
- (Optional) extend an existing compaction test to assert the lever fields stay
  `0` when only `Compaction` records are present — pins that compaction and the
  levers don't cross-contaminate.

**`reclaim_lines` panel (`mcp/src/dashboard/panels.rs` `mod tests` — rename the
existing `compactions_lines_*` tests to `reclaim_lines_*` and add lever tests):**

- `reclaim_lines_empty_placeholder` — `StatusSummary::default()` → a line
  containing `no reclaim yet` (renamed from `compactions_lines_empty_placeholder`;
  the placeholder string changed from `no compactions`).
- `reclaim_lines_shows_compaction_events_and_ratio` — the existing
  `compactions_lines_shows_events_and_ratio` body, renamed; still asserts
  `events: 2` / `freed: 400` / `1.7x` (compaction behavior unchanged).
- `reclaim_lines_omits_ratio_when_after_zero` — renamed, unchanged assertions.
- `reclaim_lines_shows_filter_lever` — `output_filtered_count: 3,
  output_filtered_tokens: 2048` → a line containing `filter: 3 calls` and `2048`.
- `reclaim_lines_shows_evict_and_dedupe_levers` — `read_evicted_count: 2,
  read_evicted_tokens: 900, read_deduped_count: 1, read_deduped_tokens: 120` → a
  line containing `evict: 2 reads` and a line containing `dedupe: 1 reads`.
- `reclaim_lines_lever_absent_renders_no_lever_line` — a summary with only
  compaction set → no `filter:` / `evict:` / `dedupe:` line appears (per-lever
  `count > 0` gating; the must-NOT-render negative case).

**`format_status` text (`mcp/src/status.rs` `mod tests`):**

- `format_status_shows_reclaimed_line_when_reclaim_occurred` — a summary with
  `output_filtered_tokens: 800` → output contains `reclaimed:` and `800`.
- `format_status_omits_reclaimed_line_when_no_reclaim` — a clean summary (all
  reclaim sources zero) → output does **not** contain `reclaimed:` (the
  must-NOT-render negative case; would fail if the line rendered `reclaimed: 0`).

## End-to-end verification

This phase ships two runtime-loadable artifacts: the live dashboard panel (a
ratatui TUI — not headlessly quotable, verified by the `reclaim_lines` unit tests,
the same testability contract every other panel function uses) and the `rexymcp
status` CLI (**headlessly quotable** — verify this one against the real binary).

1. Build: `cargo build -p rexymcp`.
2. Create a session-log fixture under a temp repo:
   `<tmp>/.rexymcp/sessions/sess-08e.jsonl`, one `SessionRecord` JSON line per
   event — at minimum a `SessionStart`, then one each of `OutputFiltered`,
   `ReadEvicted`, `ReadDeduped` (hand-write the JSON, or emit via
   `serde_json::to_string` in a scratch test). Use nonzero reclaim values.
3. Run both:
   - `cargo run -p rexymcp -- status --repo <tmp> --json` — confirm the JSON
     carries the six new fields with the folded values.
   - `cargo run -p rexymcp -- status --repo <tmp>` — confirm a `reclaimed:` line
     appears with the expected total.
4. Quote both actual outputs in the completion Update Log under "End-to-end
   verification."

## Authorizations

None. (No new dependencies; no architecture-doc edit; no new `SessionEvent`
variant; no `main.rs` change; no struct changes outside `StatusSummary`'s additive
fields.)

## Out of scope

What this phase must **not** do, even if tempted:

- **Do not touch `mcp/src/log_query.rs`, `mcp/src/dashboard/transcript.rs`, or
  `mcp/src/dashboard/filter.rs`.** The three reclaim variants are already handled
  there (event-type strings, per-event activity lines, the `ActivityFilter`
  toggles). They are correct as-is; this phase only adds the *aggregate counter*
  fold, not per-event rendering.
- **Do not add a new `SessionEvent` variant or change the existing three.** They
  exist and ship. The field is read-only here.
- **Do not change the dashboard header geometry** (`render.rs`'s
  `Layout::horizontal` constraints / `compactions_area`). Repurpose the existing
  slot in place — retitle + swap the content function only.
- **Do not change `format_status`'s existing lines** (phase/model/state/turn/
  message/last-update). Append the one new `reclaimed:` line; touch nothing above
  it.
- **Do not touch the `runs` / `scorecard` tables** (`mcp/src/runs.rs`,
  `mcp/src/scorecard.rs`, `mcp/src/scorecard_cli.rs`). Those are the post-hoc
  surfaces (08b–08d, done). This phase is the live-view surface only.
- **Do not edit `main.rs`** — the `--json` path serializes the new fields
  automatically via the existing `Serialize` derive.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-09 (started)

**Executor:** rexyMCP executor

### Update — 2025-07-09 (complete)

**Tasks completed:**
1. Added six fields to `StatusSummary` (`output_filtered_count`, `output_filtered_tokens`, `read_evicted_count`, `read_evicted_tokens`, `read_deduped_count`, `read_deduped_tokens`) — purely additive via `..StatusSummary::default()`.
2. Added three match arms to `summarize` for `OutputFiltered`, `ReadEvicted`, `ReadDeduped` before the `_ => {}` catch-all.
3. Renamed `compactions_lines` → `reclaim_lines` in `panels.rs`, extended to render all four reclaim sources (compaction + filter + evict + dedupe).
4. Repointed `render.rs`: import `reclaim_lines`, retitled panel from "Compactions" to "Reclaim".
5. Added `reclaimed:` line to `format_status`, gated on nonzero total reclaim.
6. Renamed existing compaction panel tests to `reclaim_lines_*`, added lever-specific tests.
7. Added `summarize` fold tests for all three new levers plus a negative case.
8. Added `format_status` reclaim line tests (positive and negative).

**Verification commands:**
- `cargo fmt --all --check` — passed (after formatting)
- `cargo build -p rexymcp` — passed, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed
- `cargo test -p rexymcp` — 266 passed, 0 failed

**End-to-end verification:**
- `rexymcp status --repo <tmp> --json` confirmed six new fields in JSON output:
  `"output_filtered_count": 1, "output_filtered_tokens": 800, "read_evicted_count": 1, "read_evicted_tokens": 500, "read_deduped_count": 1, "read_deduped_tokens": 300`
- `rexymcp status --repo <tmp>` (text) confirmed:
  `reclaimed: 1600 tokens (filter 800, evict 500, dedupe 300, compaction 0)`

**Files changed:**
- `mcp/src/status.rs` — six new fields, three summarize arms, format_status reclaim line, test helpers + 6 new tests
- `mcp/src/dashboard/panels.rs` — renamed `compactions_lines` → `reclaim_lines`, extended body, renamed + added 4 panel tests
- `mcp/src/dashboard/render.rs` — import rename, panel title + function call rename
- `docs/dev/milestones/M10-context-optimization/phase-08e-reclaim-events-statussummary-dashboard.md` — status → review, Update Log
- `docs/dev/milestones/M10-context-optimization/README.md` — phase table row → review

**Notes for review:** None — implementation matches spec exactly.

**Commit:** `feat: fold reclaim events into StatusSummary and live dashboard`
