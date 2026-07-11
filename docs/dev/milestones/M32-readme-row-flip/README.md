# M32 — README Row-Flip Fix

**Goal:** `flip_readme_row` produces a well-formed phase-table row — exactly
one status cell, exactly one trailing pipe — and its tests would fail on any
malformed shape.

**Status:** in-progress

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
| 01 | Fix the doubled trailing pipe in `flip_readme_row` ([phase-01-fix-row-flip-trailing-pipe.md](phase-01-fix-row-flip-trailing-pipe.md)) | todo |

## Notes

Single-phase cleanup milestone (the M29 shape). The defect is a one-character
slice fix; the substance of the phase is hardening the tests from substring
checks to exact-equality + pinned negatives, so this class of malformation
can't pass silently again. The architect flips bug-03a-1 to resolved at
milestone close.

<!-- retrospective appended at milestone close -->
