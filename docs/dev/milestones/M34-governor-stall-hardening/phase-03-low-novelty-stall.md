# Phase 03: Low-novelty churn stall detector (issue #3)

**Milestone:** M34 — Governor Stall Hardening
**Status:** done
**Depends on:** phase-01 (the read-only run scan it shares), M33 (`tools::mutates_files`)
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=l

> **Retrospective phase doc.** This work was executed **directly by the architect
> (Claude Code)** and committed as `1671754` (feat) *before* this doc was written.
> The doc records what shipped.

## Goal

Replace *volume* with *novelty* as the early stall signal. The phase-01
`NoProgressStall` measured a raw count of consecutive read-only calls, which
conflates "stuck spinning on a few files" with "legitimately reading a large
codebase." Add `LowNoveltyStall`: fire when a window of read-only calls collapses
to only a small set of *distinct normalized targets* — tight churn fails fast,
wide exploration passes however long it runs.

## Why

Raising the raw threshold can't separate the two cases — it lets genuine churn
burn more turns while still tripping a wide, productive investigation. Volume is
the wrong knob; the signal we want is "no *new* information + no edit."

## What shipped

- `HardFailSignal::LowNoveltyStall { window: usize, distinct_targets: usize }` +
  `describe()` arm — `executor/src/governor/hard_fail.rs`.
- `normalize_target(call)` — maps a read-only call to the target it probes with
  volatile detail stripped, keyed off the router category (reusing M33's
  `tools::mutates_files` / `tools::categorize` so the two classifiers stay in
  sync):
  - `read_file`/`symbols` → `path` (line range dropped; `symbols` falls back to
    `name`).
  - `search`/`find_files` → `path` scope (pattern dropped; whole-repo → `.`).
  - `bash` → `command` with ASCII digits removed (`sed -n '1,9p'` and `sed -n
    '9,20p'` collapse to one target).
  - anything else → raw `tool(arguments)` so novelty is never *under*counted.
- `check_low_novelty_stall(recent, window, distinct_floor)` — examines only the
  trailing read-only run (reset on any mutating call); fires when a full
  `window`'s worth of calls has `<= distinct_floor` distinct targets.
- `[governor] novelty_window` (default 24) + `novelty_distinct_floor` (default 6)
  config knobs; `window == 0` disables. `read_only_stall_threshold` **demoted
  20 → 60** — now a high pure-volume backstop beneath `max_turns`, with novelty
  as the early catch. `executor/src/config.rs`.
- Wired into the loop's hard-fail chain **before** the raw stall check —
  `executor/src/agent/mod.rs`; `mcp/src/runner.rs` threads both knobs.

## Acceptance criteria

- [x] Wide exploration over many distinct files passes however long it runs.
- [x] Tight re-probing of a few files fires `LowNoveltyStall` *below* the raw
      backstop.
- [x] Re-reading one file at shifting line ranges, and re-grepping one scope with
      tweaked patterns, both collapse to a single target.
- [x] Any file-mutating call resets the run.
- [x] All four gates green.

## Test plan (shipped)

- `novelty_passes_wide_exploration`
- `novelty_fires_on_churn_over_few_files`
- `novelty_ignores_line_ranges_when_rereading_one_file`
- `novelty_ignores_grep_patterns_over_one_scope`
- `novelty_buckets_whole_repo_greps_together`
- `novelty_strips_numeric_literals_from_bash`
- `novelty_resets_on_mutating_call`
- `novelty_silent_when_run_shorter_than_window`
- `novelty_disabled_when_window_zero`
- `novelty_fires_below_raw_stall_threshold` — the layering pin.
- `describe_low_novelty_stall`

## End-to-end verification

Not applicable — hermetic tests pin the behavior; a live-loop calibration of the
window/floor against real runs is the subject of **phase-04** (observability).

## Authorizations

- [x] Touched `docs/architecture.md` § Layer 1 (detector list — the churn-stall
      line) via `1671754`.

## Out of scope

- Making the novelty measurement observable / calibratable (phase-04).
- Naming the churned targets in the escalation briefing (phase-05).

## Update Log

<!-- entries appended below this line -->

### Update — 2026-07-16 (complete)

**Summary:** Shipped `LowNoveltyStall` + `normalize_target` +
`check_low_novelty_stall` + `novelty_window`/`novelty_distinct_floor` knobs;
demoted the raw `read_only_stall_threshold` to a 60-turn backstop. Committed
`1671754`.

### Review verdict — 2026-07-18 (retrospective)

- **Verdict:** approved_first_try (retrospective — reviewed at M34 formalization)
- **Bounces:** none
- **Executor:** Claude Code (direct)
- **Scope deviations:** none
- **Calibration:** the window/floor defaults (24/6) are **untuned** — chosen by
  judgment, not data. This is the motivation for phase-04 (make the measurement
  observable so the defaults can be calibrated from real runs).
