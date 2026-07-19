# Phase 04: Novelty-detector observability

**Milestone:** M34 — Governor Stall Hardening
**Status:** done
**Depends on:** phase-03 (the `check_low_novelty_stall` detector this makes observable)
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

The `LowNoveltyStall` detector shipped with **untuned, invisible** defaults
(`novelty_window = 24`, `novelty_distinct_floor = 6`) chosen by judgment, not
data. There is no way to see how many distinct targets real runs actually probe
per window, so the floor/window can only be guessed. Emit a lightweight
`NoveltySample` session-log event on every full-window novelty measurement so the
distribution is **observable via the existing log-query tools** and the knobs can
be calibrated from real runs.

## Architecture references

Read before starting:

- `docs/architecture.md` § Layer 1 (the executor turn cycle — the JSONL session
  log; every turn event is written) and § Status #34.
- `docs/dev/STANDARDS.md` § "Derive intentionally / wired-in state" — do not
  record into something with no consumer. The consumer here is the **existing**
  generic log-query surface (see § Consumer below); confirm it before coding.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

The novelty detector lives in `executor/src/governor/hard_fail.rs`. It computes a
measurement but only **returns** it when the stall fires — the passing
measurements (the calibration signal) are discarded:

```rust
// executor/src/governor/hard_fail.rs:331 (current)
pub fn check_low_novelty_stall(
    recent: &VecDeque<ToolCallSnapshot>,
    window: usize,
    distinct_floor: usize,
) -> Option<HardFailSignal> {
    if window == 0 {
        return None;
    }
    // Examine only the trailing read-only run; a mutating call is file progress.
    let mut run: Vec<&ToolCallSnapshot> = Vec::new();
    for call in recent.iter().rev() {
        if crate::tools::mutates_files(&call.tool) {
            break;
        }
        run.push(call);
        if run.len() == window {
            break;
        }
    }
    if run.len() < window {
        return None;
    }
    let mut distinct: Vec<String> = Vec::new();
    for call in &run {
        let key = normalize_target(call);
        if !distinct.contains(&key) {
            distinct.push(key);
        }
    }
    if distinct.len() <= distinct_floor {
        Some(HardFailSignal::LowNoveltyStall {
            window,
            distinct_targets: distinct.len(),
        })
    } else {
        None
    }
}
```

`normalize_target` (line 302, unchanged by this phase) is the target-key logic.

The hard-fail chain calls this at `executor/src/agent/mod.rs:1307`:

```rust
.or_else(|| {
    crate::governor::hard_fail::check_low_novelty_stall(
        &recent_tool_calls,
        deps.governor.novelty_window,
        deps.governor.novelty_distinct_floor,
    )
})
```

`SessionEvent` is defined in `executor/src/store/sessions/event.rs` (a
`#[serde(tag = "event_type", rename_all = "snake_case")]` enum). Existing additive
siblings to mirror: `OutputFiltered`, `ReadEvicted`, `ReadDeduped`, `TaskUpdate`.

### Consumer (why this is not a dead write)

A new `SessionEvent` variant is **immediately queryable** through the existing
generic log-query surface — no new consumer is needed:

- `mcp/src/log_query.rs` `event_type_str` maps each variant to its snake_case
  tag; `executor_log_search`/`executor_log_tail` filter by that tag, so
  `--event-type novelty_sample` works the moment the arm is added.
- The dashboard Activity transcript renders every variant.

This satisfies STANDARDS "wired-in state must have a consumer": the log-query
tools are that consumer. **Out of scope:** any `PhaseRun`/scorecard aggregation
of these samples (a possible future phase, deliberately not this one).

## Spec

The change is additive **plus** a behavior-preserving refactor of one function,
then a new `SessionEvent` variant whose exhaustive-match consumers must each gain
an arm. Because a new variant breaks three exhaustive `match`es the moment it is
added, **follow the leaf-first edit order in Task 6 and build after each
checkpoint** (STANDARDS § "topological edit order").

### 1. Extract `measure_novelty` from `check_low_novelty_stall`

In `executor/src/governor/hard_fail.rs`, add a small measurement type and a pure
function that returns the measurement whenever the trailing read-only run fills
the window (**regardless** of the floor), then make `check_low_novelty_stall` a
thin wrapper so the detector and the sample can never diverge:

```rust
/// The novelty detector's raw reading over a full trailing read-only window:
/// how many distinct normalized targets (`normalize_target`) the last `window`
/// read-only calls probed. Present only when the run actually reached `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoveltyMeasurement {
    pub window: usize,
    pub distinct_targets: usize,
}

/// Measure target-novelty over the trailing read-only run. Returns `None` when
/// novelty is disabled (`window == 0`) or the run is shorter than `window`
/// (measurement not yet meaningful). The trailing run resets on any
/// file-mutating call, exactly as the stall detector does.
pub fn measure_novelty(
    recent: &VecDeque<ToolCallSnapshot>,
    window: usize,
) -> Option<NoveltyMeasurement> {
    if window == 0 {
        return None;
    }
    let mut run: Vec<&ToolCallSnapshot> = Vec::new();
    for call in recent.iter().rev() {
        if crate::tools::mutates_files(&call.tool) {
            break;
        }
        run.push(call);
        if run.len() == window {
            break;
        }
    }
    if run.len() < window {
        return None;
    }
    let mut distinct: Vec<String> = Vec::new();
    for call in &run {
        let key = normalize_target(call);
        if !distinct.contains(&key) {
            distinct.push(key);
        }
    }
    Some(NoveltyMeasurement {
        window,
        distinct_targets: distinct.len(),
    })
}

pub fn check_low_novelty_stall(
    recent: &VecDeque<ToolCallSnapshot>,
    window: usize,
    distinct_floor: usize,
) -> Option<HardFailSignal> {
    measure_novelty(recent, window)
        .filter(|m| m.distinct_targets <= distinct_floor)
        .map(|m| HardFailSignal::LowNoveltyStall {
            window: m.window,
            distinct_targets: m.distinct_targets,
        })
}
```

This is behavior-preserving: **every existing `check_low_novelty_stall` /
`novelty_*` test in this file must still pass unchanged** — that is the proof the
refactor is safe. Do not alter those tests.

### 2. Add the `NoveltySample` session-log event

In `executor/src/store/sessions/event.rs`, add a variant to `SessionEvent`
(mirror the doc-comment style of `OutputFiltered`/`ReadDeduped`):

```rust
    /// Emitted each turn the novelty detector takes a full-window measurement of
    /// the trailing read-only run (M34). `distinct_targets` is how many distinct
    /// normalized targets the last `window` read-only calls probed — the raw
    /// signal `novelty_distinct_floor` is compared against. Calibration data: a
    /// distribution of `distinct_targets` across real runs shows where to set the
    /// floor. Emitted whether or not the measurement also trips `LowNoveltyStall`.
    NoveltySample {
        window: usize,
        distinct_targets: usize,
    },
```

### 3. Emit the sample in the turn cycle (deduped)

In `executor/src/agent/mod.rs`, declare dedup state before the turn loop
(alongside the other trailing-run trackers), e.g.:

```rust
    let mut last_novelty_distinct: Option<usize> = None;
```

Then, in the turn cycle near the hard-fail evaluation (around line 1287, before
the `hard_fail_signal` chain), emit a sample when the measurement's
`distinct_targets` **differs from the last emitted value**, and re-arm when the
run drops below the window (e.g. after an edit):

```rust
    match crate::governor::hard_fail::measure_novelty(
        &recent_tool_calls,
        deps.governor.novelty_window,
    ) {
        Some(m) if Some(m.distinct_targets) != last_novelty_distinct => {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::NoveltySample {
                    window: m.window,
                    distinct_targets: m.distinct_targets,
                },
            );
            last_novelty_distinct = Some(m.distinct_targets);
        }
        None => last_novelty_distinct = None,
        _ => {}
    }
```

Dedup rule (pin exactly): **consecutive identical `distinct_targets` emit once**;
any change emits; a measurement gap (run shorter than `window`, i.e. `None`) rearms
so the next full window emits again even if its count matches the pre-gap one.
This keeps a long read streak from flooding the log with identical samples while
still recording the *trajectory* of the distinct count.

### 4. Add the `event_type_str` arm (production)

In `mcp/src/log_query.rs`, add to the `event_type_str` match:

```rust
        SessionEvent::NoveltySample { .. } => "novelty_sample",
```

### 5. Add the exhaustive-match arms in the remaining consumers

- `mcp/src/dashboard/transcript.rs` — the render match (ends at `TaskUpdate`, no
  wildcard). Add a one-line summary arm, mirroring the `ReadDeduped` arm's
  `(String, Color, bool, Option<..>)` tuple shape:

  ```rust
          SessionEvent::NoveltySample {
              window,
              distinct_targets,
          } => (
              format!("novelty: {distinct_targets} distinct target(s) over {window} read-only calls"),
              Color::Cyan,
              false,
              None,
          ),
  ```

- `mcp/src/dashboard/filter.rs` — the `allows()` match (exhaustive). Gate the new
  variant on the **existing** `self.metrics` toggle (a per-turn resource-style
  sample); do **not** add a new filter field or keyboard toggle index (keeping the
  dashboard legend unchanged is deliberate — this phase is the emit+query path,
  not a new UI control):

  ```rust
              SessionEvent::NoveltySample { .. } => self.metrics,
  ```

- `executor/src/agent/tests.rs` — the mirrored `event_type_str` test helper
  (around line 954, exhaustive). Add the same arm as Task 4:

  ```rust
          SessionEvent::NoveltySample { .. } => "novelty_sample",
  ```

### 6. Edit order (leaf-first; build at each checkpoint)

Adding the variant (Task 2) breaks three exhaustive matches until their arms
exist. Do the edits in this order and run `cargo build` at each **checkpoint**:

1. Task 2 (add the variant) **immediately followed by** Tasks 4 + 5 (all four
   arms: `log_query.rs`, `dashboard/transcript.rs`, `dashboard/filter.rs`,
   `agent/tests.rs`). — **checkpoint: `cargo build` green.**
2. Task 1 (`measure_novelty` + refactor `check_low_novelty_stall`). Additive; no
   consumer breaks. — **checkpoint: `cargo build` + the existing `novelty_*`
   tests green.**
3. Task 3 (emit site + dedup state in `agent/mod.rs`). — **checkpoint: `cargo
   build` green.**
4. New tests (Test plan below). — **checkpoint: all four gates green.**

## Acceptance criteria

- [ ] `measure_novelty` returns `Some` iff the trailing read-only run fills
      `window` (and `window != 0`); `None` otherwise.
- [ ] Every existing `novelty_*` / `check_low_novelty_stall` test in
      `hard_fail.rs` passes **unchanged** (refactor is behavior-preserving).
- [ ] `event_type_str(&SessionEvent::NoveltySample { .. })` == `"novelty_sample"`
      and `event_type_str_round_trips_all_variants` covers it — so
      `executor_log_search --event-type novelty_sample` filters these events.
- [ ] A read-only churn run emits `NoveltySample` events; consecutive identical
      `distinct_targets` values emit **once**; a mutating call re-arms emission.
- [ ] All four gates green.

## Test plan

- `measure_novelty_none_below_window` in `hard_fail.rs` — run shorter than
  `window` → `None`.
- `measure_novelty_none_when_window_zero` — disabled → `None`.
- `measure_novelty_counts_distinct_targets` — a full window over churn returns
  `Some { distinct_targets: <n> }` matching what the stall detector would see.
- (existing `novelty_fires_on_churn_over_few_files`,
  `novelty_passes_wide_exploration`, etc. — must still pass unchanged.)
- `novelty_sample_event_type_str_is_novelty_sample` in `log_query.rs`, and extend
  `event_type_str_round_trips_all_variants` to include the new variant.
- A `MockAiClient` loop test in `agent/tests.rs` (or `agent/mod.rs` tests):
  drive a scripted run that churns a few read-only targets past `novelty_window`,
  assert the session log contains ≥1 `NoveltySample` record; assert **dedup**
  (two turns at the same distinct count → one sample); assert a mutating call
  between two churn streaks re-arms (a second sample at the same count appears).

## End-to-end verification

The shipped real artifact is the queryable event. Verify against the real
log-query path (not only the unit fake): run a phase (or the hermetic loop
harness that writes a real JSONL session log), then confirm the samples are
filterable, e.g.

```
rexymcp executor-log-search --event-type novelty_sample --repo <path>   # or the MCP tool
```

and quote a matched `novelty_sample` record in the completion Update Log. If run
via the hermetic harness rather than a live model, state that and quote the
on-disk JSONL line the harness produced.

## Authorizations

- [ ] May touch `docs/architecture.md` (§ Layer 1 — one line noting the novelty
      sample is logged for calibration). Only if the executor judges the detector
      list wording needs it; otherwise leave architecture.md alone.

## Out of scope

- Any `PhaseRun`/scorecard aggregation or a dashboard panel for novelty samples
  (future phase if calibration demand appears).
- A dedicated dashboard filter toggle / legend entry (reuse `self.metrics`).
- Changing the detector's firing behavior, `normalize_target`, or the default
  `novelty_window`/`novelty_distinct_floor` values (this phase only *observes* so
  a later human decision can retune them).
- The escalation-briefing enrichment — that is phase-05.

## Notes

**Routing.** Like phases 01–03 this touches `governor/hard_fail.rs`; the
governor-supervises-the-executor consideration (see the M34 README Notes) plus
the multi-site `SessionEvent` cascade (the M10 08a/08d struct-literal/match-churn
stall class) both argue for **direct execution**. If dispatched instead, the
leaf-first order in Task 6 is the anti-stall countermeasure — the executor must
build at each checkpoint. The phase doc records the actual executor at review.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-07-18 (complete)

**Summary:** Shipped novelty-detector observability. `measure_novelty` +
`NoveltyMeasurement` extracted from `check_low_novelty_stall` (behavior-preserving
— the latter is now a thin `.filter(|m| distinct <= floor)` wrapper);
`SessionEvent::NoveltySample { window, distinct_targets }` added and emitted each
full-window measurement in the turn cycle, deduped on distinct-count change and
re-armed on a measurement gap. The full leaf-first cascade landed: `event_type_str`
+ round-trip test (`log_query.rs`), render arm (`dashboard/transcript.rs`), filter
arm reusing the `metrics` toggle (`dashboard/filter.rs`), and the mirrored
`agent/tests.rs` helper. No new config; no `PhaseRun`/scorecard rollup (out of
scope). `architecture.md` left untouched (the detector-list wording already covers
it).

**Execution:** **dispatched** to AEON-7/Qwen3.6-27B-AEON (run
`fc515cd4`), which correctly completed the entire cascade + emit site + the three
`measure_novelty` unit tests, then **hard_failed `LowNoveltyStall {window:24,
distinct_targets:6}` at turn 104/600** — churning read-only calls on
`agent/tests.rs` while trying to write the required integration test. The detector
under construction fired on its own author. **Session takeover:** the architect
(Claude Code, direct) finished the one missing piece — the
`MockAiClient` integration test — on top of the executor's correct work.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
FMT CLEAN  (after `rustfmt executor/src/agent/tests.rs mcp/src/dashboard/transcript.rs`)

cargo build 2>&1 | tail
Finished `dev` profile — zero warnings

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail
Finished — zero warnings

cargo test 2>&1 | grep "test result:"
test result: ok. 517 passed; 0 failed  (mcp)
test result: ok. 991 passed; 0 failed; 2 ignored  (executor)
```

**End-to-end verification:** the shipped artifact is the queryable event.
Exercised the real write→persist→read path via the hermetic loop harness
(`novelty_samples_are_emitted_deduped_and_rearm_after_edit` drives the production
loop → real JSONL writer → `read_session_log` reader), and confirmed the
query-tool tag on a real serialized record:

```
{"event_type":"novelty_sample","window":24,"distinct_targets":6}
```

so `executor_log_search --event-type novelty_sample` filters these. (Verified via
the hermetic harness rather than a live serve dispatch — the connected serve
binary predates this change.)

**Files changed:**
- `executor/src/store/sessions/event.rs` — `NoveltySample` variant.
- `executor/src/governor/hard_fail.rs` — `NoveltyMeasurement` + `measure_novelty`;
  `check_low_novelty_stall` refactored to a wrapper; 3 unit tests.
- `executor/src/agent/mod.rs` — deduped emit site + `last_novelty_distinct` state.
- `executor/src/agent/tests.rs` — `event_kind` helper arm + the integration test.
- `mcp/src/log_query.rs` — `event_type_str` arm + round-trip test record.
- `mcp/src/dashboard/transcript.rs` — render arm.
- `mcp/src/dashboard/filter.rs` — `allows()` arm (reuses `metrics` toggle).

**New tests:**
- `measure_novelty_none_below_window`, `measure_novelty_none_when_window_zero`,
  `measure_novelty_counts_distinct_targets` in `hard_fail.rs` (executor-authored).
- `novelty_samples_are_emitted_deduped_and_rearm_after_edit` in `agent/tests.rs`
  (architect-authored; the single `== vec![(3,1),(3,1)]` assertion is
  mutation-resistant — no-dedup → ~6 samples, no-rearm → 1, no-emit → 0).

**Notes for review:** existing `novelty_*` tests all pass unchanged, proving the
`measure_novelty` refactor is behavior-preserving.

### Review verdict — 2026-07-18

- **Verdict:** escalated (dispatch hard_failed on `LowNoveltyStall`; architect
  session-takeover finished the missing integration test)
- **Bounces:** 1 (hard_fail — `LowNoveltyStall`, not a correctness defect)
- **Executor:** AEON-7/Qwen3.6-27B-AEON (cascade + unit tests) + Claude Code
  (direct — integration test)
- **Scope deviations:** none
- **Calibration:** the detector firing on its author at turn 104 of a 600-turn
  budget (82% unused) is live evidence for the **advisory-until-calibrated**
  redesign now folding into M34 — data-free early-kill thresholds pre-empt
  productive runs deep inside budget. This run is itself a calibration data point.
