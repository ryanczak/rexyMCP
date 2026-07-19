# Phase 06b: Extend calibration to the remaining governor detectors

**Milestone:** M34 — Governor Stall Hardening
**Status:** todo (planned — drafted after 06a lands)
**Depends on:** phase-06a (the replay/aggregate/report framework + `Signal` seam)
**Estimated diff:** ~250 lines (estimate — firmed at drafting)
**Tags:** language=rust, kind=feature, size=m

> **Planned stub.** Completes the governor-**wide** scope the user chose: extend
> 06a's `calibrate-governor` framework to every remaining detector so *all*
> thresholds are backed by corpus data. Drafted once 06a's `Signal` seam exists to
> extend against.

## Goal

Add the remaining detectors' signals to the `Signal` enum + `SIGNALS` list from
06a, each re-derived from the replayed event stream and reported per-model with
the same outcome-labeled p50/p90/p99 + N. No framework changes — additive
variants only.

## Signals to add (re-derive via the existing detector primitives)

Each must go through the live `hard_fail` primitive where one exists (no
re-implementation — same anti-drift pin as 06a's novelty signal):

- **identical-repetition** — longest run of consecutive byte-identical `(tool,
  arguments)` calls (vs `identical_call_threshold`, default 6).
- **oscillation** — min distinct `(tool, arguments)` over the sliding
  `oscillation_window` (vs `oscillation_distinct_max`, default 2). Needs the
  `Verify`/tool stream; reuse `check_oscillation`'s window logic.
- **verifier-persistence** — longest run of consecutive non-decreasing
  author-attributed verifier-error turns (from `Verify { diagnostics }` events,
  vs `verifier_persistence_threshold`, default 6). **Requires** pairing the
  `Verify` event stream into per-turn error counts — 06a only reconstructed
  `Parsed` tool calls, so this signal adds `Verify` extraction to `replay`.
- **empty-completion** — longest run of consecutive empty/think-only completions
  (needs `Completion { raw }` inspection or a dedicated marker — confirm what the
  log carries; the loop counts these internally).
- **output-flood** — distribution of single-call output bytes (`runaway_output_bytes`,
  100 KB) and windowed-sum bytes (`output_window_bytes`, 256 KB). Output byte
  counts are not directly in the log's `ToolResult { output_preview }` (preview is
  truncated) — **open question:** does the corpus carry enough to re-derive this,
  or is it out of reach without a new logged field? Resolve at drafting; if
  unreachable, document the gap rather than approximate.

## Open questions to resolve at drafting

1. **Verify/Completion extraction.** 06a's `replay` only collected `Parsed`. This
   phase extends it to `Verify` (per-turn error counts) and possibly `Completion`.
   Confirm the events carry what each signal needs before speccing.
2. **Output-flood reachability.** `output_preview` is truncated in the log; the
   real byte counts may not be recoverable. Decide: re-derive if present, else
   document as needing a new logged field (a separate, later change).
3. **Live advisory marker (carried from 06a).** Whether to add a queryable
   "advisory fired" `SessionEvent` for live dashboard visibility — its own
   leaf-first cascade (per phase-04). Fold in here or leave to a separate phase.

## Out of scope

- Framework changes (06a owns the replay/aggregate/report; this is additive
  signal variants).
- Suggested-threshold output / config mutation (report-only, fixed decision).

## Update Log

<!-- entries appended below this line -->
