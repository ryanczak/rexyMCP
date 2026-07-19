# Phase 06: Threshold calibration & governor metrics overhaul

**Milestone:** M34 — Governor Stall Hardening
**Status:** planning (requirements only — deferred; full spec drafted later, with the user)
**Depends on:** phase-04 (`NoveltySample` telemetry), phase-05 (advisory-demotion —
the detectors must be non-fatal so runs produce full-length data to calibrate against)
**Estimated diff:** TBD (likely `l` — a new analysis/reporting surface)
**Tags:** language=rust, kind=feature, size=l

> **Requirements-capture stub, not a dispatch-ready spec.** The user asked to
> document the back-test requirements and take the time to flesh out a *complete
> metrics overhaul* as its own phase. This file fixes the requirements and open
> questions so the eventual spec is grounded; it is deliberately **not** drafted
> for execution yet.

## Goal (intent)

Make every governor stall threshold **data-backed**. Replay the accumulated
session-log corpus, measure what the detectors *would* have done on real runs
(especially runs that **succeeded**), and turn that distribution into a threshold
recommendation — or the evidence to delete a detector. Advisory-by-default
(phase-05) is what makes this honest: with detectors non-fatal, runs go to their
natural length, so the corpus contains the full-length churn/exploration behavior
we need to distinguish "productive" from "stuck."

## Core requirement — the calibration back-test

Replay `.rexymcp/sessions/*.jsonl` through the detector functions and report, per
detector (and ideally per model), the distribution of the signal against the
run's **outcome label**:

- **Outcome label** comes from the `SessionEnd { status }` event: `complete`
  (success), `hard_fail`, `budget_exceeded`, `cancelled`.
- **Novelty:** for each full window, the `distinct_targets` count. On *successful*
  runs, how low does `distinct_targets` legitimately go? If successful runs
  routinely reach `<= 6`, the floor is provably wrong.
- **Raw no-progress:** the max consecutive read-only run length per run. On
  successful runs, how long do legitimate read-only stretches get vs. the 60
  backstop?
- Report percentiles (p50/p90/p99) of each signal split by outcome, with per-cell
  sample sizes (small models are high-variance — never hide N).

The recommendation is **human-facing** (a report), mirroring the scorecard ethos:
"there is no automated model-routing / no self-tuning governor." A human reads the
distribution and sets the threshold (or removes the detector).

## Open questions to resolve with the user before speccing

1. **Corpus reach.** `NoveltySample` only exists in logs written after phase-04.
   Older logs still carry the raw `ToolResult`/`Parsed` tool-call stream, so the
   back-test can **re-derive** `measure_novelty` / `check_read_only_stall` from raw
   events for the whole corpus — confirm we re-derive rather than depend on
   `NoveltySample` being present. (Re-deriving also keeps the back-test honest if
   `normalize_target` changes.)
2. **Ship shape.** A new `rexymcp calibrate-governor` (or `governor-report`) CLI
   subcommand vs. a one-off analysis script. Calibration is arguably a recurring
   need (new model → re-calibrate), which argues for a subcommand. Decide.
3. **Scope of "metrics overhaul."** Just the two stall signals, or a governor-wide
   pass (also surface identical-repetition / oscillation / verifier-persistence
   distributions, since those thresholds are equally un-calibrated)? The user
   flagged wanting a *complete* overhaul — enumerate which detectors are in.
4. **Per-model vs global.** Do thresholds become per-model (`ModelOverride`
   already supports it) informed by per-model distributions, or global?
5. **Live advisory marker.** phase-05 deferred a live "novelty flagged (advisory)"
   signal for the dashboard/log. Fold a clean queryable advisory `SessionEvent`
   (its own leaf-first cascade, per phase-04) into this phase if live visibility
   is wanted, or leave calibration to the `NoveltySample`/re-derived stream.
6. **Actionability loop.** Does the report just print, or also emit a suggested
   `[governor]` TOML block (like `rexymcp calibrate` does for other knobs)? If the
   latter, coordinate with the existing `calibrate` subcommand.

## Non-goals (fixed now)

- No **self-tuning / adaptive** governor — thresholds stay human-set (scorecard
  ethos: telemetry informs a human decision).
- Not a benchmark apparatus — reuse passive production session logs, as the
  scorecard does.

## Update Log

<!-- entries appended below this line -->
