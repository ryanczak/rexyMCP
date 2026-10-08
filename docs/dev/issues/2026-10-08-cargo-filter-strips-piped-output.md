# The cargo output filter strips lines a piped `grep` selected — and it reclaims 0.04 % of context

**Filed:** 2026-10-08 by the rexyMCP architect session, from
[GitHub issue #13](https://github.com/ryanczak/rexyMCP/issues/13)
**Area:** `executor/src/context/output_filter.rs`, `executor/src/tools/bash.rs`,
`executor/src/governor/hard_fail.rs`, `executor/src/config.rs`
**Related:** M10 Arc A (the filter), M34 (`NoProgressStall`), M37 (the
non-mutating exemption)
**Evidence:** the issue's DaemonEye session logs, and
`scripts/output-filter-savings.py` over the local telemetry store
(`~/.rexymcp/telemetry/phase_runs.jsonl`, 1154 phase runs).

## Summary

Issue #13 reports a 482-repeat loop: the phase's evidence command was
`cargo test … 2>&1 | grep -E '^test |^test result'`, the model expected the
ten `test … ok` lines the `grep` selects, and the cargo filter dropped every
one of them. The model saw output contradicting its spec, with no marker
saying why, and re-ran the identical command until a human stopped it at
turn 570.

Three findings, in order of what they change:

1. **The cargo filter is not worth its risk.** Measured over every local run
   that recorded both figures, the boundary filter reclaims **0.04 %** of
   input tokens. The lossy half of it (dropping passing-test and progress
   lines) is the only part that can lie to the model; the lossless half
   (ANSI strip, duplicate collapse, head/tail with a recovery file) is the
   real safety net and stays.
2. **The governor stayed silent because the backstop was switched off.** The
   downstream `rexymcp.toml` has `read_only_stall_threshold = 0`, which the
   config documents as "disables" — set by mistake on 2026-07-25 per that
   file's own comment, and already recorded once in
   `2026-09-10-governor-blind-to-edit-without-gate.md`. With the default of
   60 the run ends at turn 60. A value that silently removes the only
   terminator for non-mutating loops should not load.
3. **Byte-identical repetition is not diagnosis.** M37 exempted
   non-mutating windows from both tight detectors so `sed -n` inspection
   loops are not killed mid-diagnosis. That reasoning holds for the
   oscillation detector (A, B, A, B over different ranges) and does not hold
   for N *byte-identical* consecutive calls, which carry no information after
   the first repeat. Restoring identical-repetition on non-mutating windows
   is defence in depth for the case where the backstop is misconfigured.

## What the filter reclaims

`python3 docs/dev/issues/scripts/output-filter-savings.py` on 2026-10-08:

| Measure | Value |
|---|---|
| phase runs in the store | 1154 |
| runs recording both `output_filtered_tokens` and `input_tokens` | 479 |
| filtered tokens, total | 1 453 945 |
| input tokens, total | 3 583 234 148 |
| share of input reclaimed | **0.041 %** |
| per-run share, median | 0.051 % |
| per-run share, p90 | 1.81 % |
| runs where the filter reclaimed nothing | 56 |

The p90 run is the one shape where the filter earns anything: a long
`cargo test` with hundreds of passing tests. That output already exceeds
the 100-line cap, so the lossless path spills it to a recovery file the
model can `read_file` — the one case the lossy filter helps is the case the
lossless one already handles.

## Where the issue's three suggestions land

| Suggestion in #13 | Decision |
|---|---|
| don't apply the cargo filter to pipelines | superseded: retire the cargo filter entirely (M48 phase-01) |
| tell the model when filtering changed its output | moot once nothing lossy runs |
| identical-repetition fires regardless of mutation state | adopted (M48 phase-03), oscillation keeps its exemption |

Plus one the issue did not ask for: `read_only_stall_threshold = 0` is
refused at config load (M48 phase-02), because the downstream failure was
the backstop being off, not the backstop being wrong.
