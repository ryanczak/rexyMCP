# Phase 01: No-progress read-only stall detector (FR-2)

**Milestone:** M34 — Governor Stall Hardening
**Status:** done
**Depends on:** none
**Estimated diff:** ~150 lines
**Tags:** language=rust, kind=feature, size=m

> **Retrospective phase doc.** This work was executed **directly by the architect
> (Claude Code)** and committed as `2a405a7` (feat) + the docs portion of
> `dc1155d` *before* this doc was written. The doc records what shipped, not a
> forward-looking spec. Direct execution (not dispatch) because the change lives
> in the governor that supervises the executor loop.

## Goal

Give the governor a detector for the verify-loop signature the existing
detectors miss: a long run of *varied* read-only tool calls (grep/test/`git
status`) that make no code progress. Fire `NoProgressStall` before the loop burns
its whole turn budget and needs a human `rexymcp stop`.

## Why

DaemonEye M4 phases 05b and 06 verify-looped for **529** and **167** turns
respectively, evading the governor entirely. `check_identical_repetition` needs
*exactly* repeated calls; `check_oscillation` needs a *small distinct* set —
neither sees a loop whose calls vary while making zero file edits.

## What shipped

- `HardFailSignal::NoProgressStall { consecutive_read_only: u32 }` +
  `describe()` arm — `executor/src/governor/hard_fail.rs`.
- `check_read_only_stall(recent, threshold)` — counts the trailing run of
  non-mutating calls (reset on any file edit) and fires at `threshold`.
- `[governor] read_only_stall_threshold` config knob (default 20 at ship time;
  **later demoted to 60** in phase-03 when the novelty detector became the early
  catch) + per-model override; `0` disables. `executor/src/config.rs`.
- Wired into the loop's hard-fail `.or_else(...)` chain — `mcp/src/runner.rs`
  threads the knob; `executor/src/agent/mod.rs` calls it after the other checks.

## Acceptance criteria

- [x] A run of ≥ threshold varied read-only calls fires `NoProgressStall`.
- [x] Any file-mutating call resets the trailing run (read-heavy exploration
      between edits never trips it).
- [x] `read_only_stall_threshold = 0` disables the detector.
- [x] All four gates green.

## Test plan (shipped)

- `read_only_stall_fires_on_long_varied_run` — varied calls, no edits → fires.
- `read_only_stall_resets_on_mutating_call` — a `write_file` mid-run resets it.
- `read_only_stall_silent_below_threshold` — short run, no fire.
- `read_only_stall_disabled_when_threshold_zero`.
- `read_only_stall_counts_write_file_as_progress` — edit counts as progress.
- `describe_no_progress_stall` — the rendered signal string.

## End-to-end verification

Not applicable at the time — the detector's real-artifact exercise is the live
DaemonEye run that motivated it; the hermetic tests above pin the behavior. A
live-loop validation is deferred to a future milestone phase if churn recurs.

## Authorizations

- [x] Touched `docs/architecture.md` § Layer 1 (detector list) via `dc1155d`.

## Out of scope

- Novelty-aware detection (phase-03).
- The `git stash` self-revert guard (phase-02).

## Update Log

<!-- entries appended below this line -->

### Update — 2026-07-14 (complete)

**Summary:** Shipped `NoProgressStall` + `check_read_only_stall` +
`read_only_stall_threshold` knob, wired into the loop's hard-fail chain.
Direct-executed and committed `2a405a7`; docs folded via `dc1155d`.

### Review verdict — 2026-07-18 (retrospective)

- **Verdict:** approved_first_try (retrospective — reviewed at M34 formalization)
- **Bounces:** none
- **Executor:** Claude Code (direct)
- **Scope deviations:** none
- **Calibration:** none — superseded design (raw-volume threshold) refined by
  phase-03's novelty detector; the knob was demoted 20 → 60, not removed.
