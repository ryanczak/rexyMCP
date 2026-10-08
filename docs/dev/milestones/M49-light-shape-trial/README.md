# M49 — Light-shape phase trial

**Goal:** Land the two test follow-ups held at M48 close, using a phase doc
that specifies behaviour and test names only — no line-quoted current state,
no dictated test bodies, no pinned counts — and measure whether a large-tier
executor completes it without a bounce.

**Status:** done *(opened and closed 2026-10-08, one phase, `approved_first_try`)*

**Depends on:** M48.

**Why:** M48 cost ~356k architect output tokens to draft and review three
phases whose executor output was ~61k, and every M48 spec defect lived in
material the architect pre-derived and the executor re-derived anyway. This
milestone is the control experiment: same executor, a quarter of the spec.

## Exit criteria

- `piped_cargo_style_output_is_not_filtered` exercises a command that begins
  with `cargo`, so it would have failed against the retired cargo filter.
- `repeated_identical_bash_trips_hard_fail` carries no setup that the run
  does not use.
- All four gates green.
- Recorded in the retrospective: architect drafting tokens for this phase,
  executor turns, and the verdict — compared against M48's per-phase numbers.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | M48 test follow-ups ([phase-01-m48-test-followups.md](phase-01-m48-test-followups.md)) | done |

## Notes

- What the light shape deliberately omits, so the comparison is honest:
  `## Current state` with line numbers, test code in the Spec, `grep -c`
  criteria, and a paste self-check ritual. The E2E block stays because it is
  what makes the evidence real.

## M49 retrospective — closed 2026-10-08

**Outcome: `approved_first_try`, 136 executor turns, no spec defects.**

| | M48 per phase (avg) | M49 phase-01 |
|---|---|---|
| Phase doc | ~480 lines | ~75 lines |
| Architect review output | ~42.6k tokens | ~30.5k tokens |
| Executor turns | 203 | 136 |
| Architect spec defects | ≥1 every phase | none |

Drafting tokens are not separable: the doc was written in free conversation,
so the ledger books it under `other`. It took one tool call and no
pre-dispatch runs.

**What the light shape shifted to the executor, unprompted:** it verified
the spec's premise against git history before editing, ran the regression
check against the pre-M48 code itself, and reported its own model correctly.
The reviewer accepted that evidence instead of reproducing it.

**Caveat:** one small, test-only phase. It shows the light shape works for
a change of this size; it does not yet show it for a feature phase.

**Open for the human:** whether the light shape becomes the default. That
touches the architect skill's pre-injection section and the WORKFLOW.md
phase-doc template, both contract docs.
