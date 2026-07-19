# Bug 1 on phase-06b: the four new signal extractors ship with zero tests

**Severity:** major
**Status:** verified
**Filed:** 2026-07-19

## What's wrong

The phase added four new `Signal` extractors to `mcp/src/calibrate_governor.rs`
(`IdenticalRun`, `OscillationMinDistinct`, `VerifierPersistenceRun`,
`EmptyCompletionRun`) plus two new `RunReplay` fields, but added **no tests for
any of them**. The 9 tests in the file are all the pre-existing 06a tests; the
diff only updated their `samples()` call sites for the new signature.

This violates the DoD (`STANDARDS.md` §1 "New code is covered by tests per §3"
and §3.1 "Every new pure function … unit test") and the phase doc's own **Test
plan**, which named seven tests — **none were written**:

- `identical_run_counts_longest_consecutive_identical`
- `oscillation_min_distinct_no_sample_below_window`
- `oscillation_min_distinct_finds_tightest_window`
- `verifier_persistence_run_matches_detector_semantics`
- `verifier_persistence_no_sample_without_verify_events`
- `empty_completion_run_counts_consecutive_blanks`
- `remaining_signals_appear_in_report`

The extractors are non-trivial — the `VerifierPersistenceRun` non-decreasing /
reset logic and the `OscillationMinDistinct` sliding-window distinct count are
easy to get subtly wrong, and there is nothing to catch drift from the detector
semantics they mirror. The gates are green only because untested code still
compiles; correctness of the new logic is entirely unverified.

## What should happen

Every new extractor has a unit test pinning its behavior against the detector
semantics it mirrors, including the negative/boundary cases the spec named. Per
`STANDARDS.md` "pin negative cases," each must include the must-NOT case (reset on
a differing call; no sample below the window; no sample without `Verify` events).

## How to fix

Add the seven tests from the phase doc's Test plan to the `#[cfg(test)] mod tests`
block in `mcp/src/calibrate_governor.rs`. Extend the existing `make_session_file`
helper (or add a sibling) so fixtures can carry `Verify` and `Completion` events —
`make_session_file` currently emits only `SessionStart` / `Parsed` / `SessionEnd`,
so `verifier_persistence_run` and `empty_completion_run` need a helper that also
emits `SessionEvent::Verify { diagnostics }` and `SessionEvent::Completion { raw }`
(do not break the existing tests — add parameters or a new helper). Pin at least:

- **identical**: `read a, read a, read b, read b, read b` → 3 (the `b` run), **not**
  5 (the total) — the reset on a differing call.
- **oscillation**: a run shorter than `OSCILLATION_WINDOW` → no sample; and an
  A,B,A,B stretch inside a longer run → min distinct 2.
- **verifier-persistence**: counts `[1,2,2,0,3]` → longest non-decreasing positive
  streak 3 (`1,2,2`), reset by the `0`; a decrease `[2,1]` → streak 1, not 2; a run
  with no `Verify` events → no sample.
- **empty-completion**: completions `["hi","","","x"]` → 2; a `<think>…</think>`-only
  completion counts as empty.
- **report**: a `run()` over a fixture dir surfaces all four new signal labels.

## Verification

- [ ] `cargo test -p rexymcp calibrate_governor` includes and passes the new tests
      (≥ 6 new test fns).
- [ ] Each new extractor has a test with a pinned negative/boundary case.
- [ ] All four gates green.

## Notes (not part of this bounce — for the deferred metrics/reporting pass)

The E2E run surfaced a reporting observation worth carrying into the planned
metrics deep-dive, **not** a 06b defect: `oscillation_min_distinct` is a
"lower-is-worse" signal, but the report shows p50/p90/p99 (the high tail), so the
oscillatory runs (low min-distinct) don't stand out — every outcome shows p50 ≈ 7–8
(≈ the window size). For min-type signals the informative tail is the *low*
percentiles. The raw signal is captured correctly per spec; only the percentile
presentation is the (deferred) design question.
