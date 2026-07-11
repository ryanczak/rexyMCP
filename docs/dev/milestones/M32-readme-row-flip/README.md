# M32 — README Row-Flip Fix

**Goal:** `flip_readme_row` produces a well-formed phase-table row — exactly
one status cell, exactly one trailing pipe — and its tests would fail on any
malformed shape.

**Status:** done

**Depends on:** none

## Why now

The server-authored finalize's README row flip has misfired in production
**four times** (M27 phase-05a, M27 phase-06a, M31 phase-01, M31 phase-02),
each time hand-repaired by the reviewing architect. Past the three-occurrence
fold threshold.

History: [bug-03a-1](../M27-autonomous-escalation-loop/bugs/bug-03a-1.md)
diagnosed the original defect (the flip *appended* a `| review |` cell after
the stale one). A partial fix landed in commit `2d535be` (the stale cell text
is now correctly spliced out), but the replacement's suffix slice is off by
one — `&line[last_pipe..]` **includes** the final `|`, so every flip emits a
doubled trailing pipe: `| review ||`. The tests never caught either shape
because they assert `contains("| review |")`, which is a substring of the
malformed `| review ||` too.

## Exit criteria

- A flipped row is byte-exact well-formed (`… | review |`, single trailing
  pipe, one status cell).
- The tests assert **exact row equality** (not substrings) and pin the
  negatives (`||` must not appear; the stale status must be gone).
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #27 (server-authored bookkeeping — the
  finalize path this fixes) and § Status #32 (this milestone).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Fix the doubled trailing pipe in `flip_readme_row` ([phase-01-fix-row-flip-trailing-pipe.md](phase-01-fix-row-flip-trailing-pipe.md)) | done |

## Notes

Single-phase cleanup milestone (the M29 shape). The defect is a one-character
slice fix; the substance of the phase is hardening the tests from substring
checks to exact-equality + pinned negatives, so this class of malformation
can't pass silently again. The architect flips bug-03a-1 to resolved at
milestone close.

<!-- retrospective appended at milestone close -->

## Retrospective — 2026-07-10

**Shipped:** the one-character suffix-slice fix in `flip_readme_row`
(`&line[last_pipe..]` → `&line[last_pipe + 1..]`, commit `c930d02`) plus the
test hardening that makes the malformation class unrepresentable in a green
suite: exact full-row equality assertions, pinned `!contains("||")` /
stale-status negatives, and the `flip_readme_row_emits_single_trailing_pipe`
regression test.

**Verdict:** phase-01 approved_first_try (executor AEON-7/Qwen3.6-27B-AEON,
46 turns; diff landed byte-identical to the spec's pre-injected fragments).
**Mutation-verified in review:** reverting the fix fails 4 of the 6 flip
tests — before this phase, the reverted state passed all of them.

**Meta-evidence:** this phase's own server-authored finalize — running the
pre-fix serve binary — wrote `| review ||` into this README's phase row (the
5th and final production occurrence), hand-repaired at approve time. The next
finalize after a serve restart runs the fixed code; observing one well-formed
row flip on the next milestone's first phase completes the loop.

**Calibration:** the deeper lesson is about *test shape*, not the slice: all
five prior tests asserted `contains("| review |")`, which the malformed
`| review ||` output also satisfies — a weak-substring-assertion blind spot
that let both the original bug (duplicated cell) and the partial fix's
regression (doubled pipe) ship green. Same family as the M30 phase-01/02/04
weak-test bounces: **substring/disjunction assertions on exact-format output
are calibration data for "pin exact equality + negatives."** Recurring
pattern; fold candidate alongside the cascade-ordering one from M31.

**History closed:** bug-03a-1 (M27) flipped to resolved — its "How to fix"
splice was half-applied in `2d535be` (prefix corrected, suffix off-by-one
introduced); this milestone finishes it.