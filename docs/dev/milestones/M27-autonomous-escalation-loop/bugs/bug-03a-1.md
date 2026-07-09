# Bug 1 on phase-03a: `flip_readme_row` duplicates the status cell instead of replacing it

**Severity:** minor
**Status:** open
**Filed:** 2026-07-09

## What's wrong

`flip_readme_row` in `mcp/src/finalize.rs:176-207` is supposed to replace a
phase-table row's `in-progress` status cell with `review`. Instead it *inserts*
a new `| review |` cell after the stale one, leaving both in the row.

The bug is in the replacement at `mcp/src/finalize.rs:190`:

```rust
format!("{}| review |{}", &line[..last_pipe], &line[last_pipe + 1..])
```

`&line[..last_pipe]` is the whole line up to (not including) the final `|` —
which still contains the old `" in-progress "` cell text. The format string
glues a brand-new `| review |` pair onto the end of that unchanged prefix,
rather than replacing the interior between `second_last_pipe` and `last_pipe`
(the value the code already computed as `last_cell` at line 186 but never
used for the splice).

**Observed twice in production:**
- M27 phase-05a's README row became `| done | review |` (noted in `docs/dev/NEXT.md`'s
  05a entry, manually fixed at the next `/rexymcp:architect next`).
- M27 phase-06a's README row became `| in-progress | review |` (this review pass).

**Why the existing test didn't catch it:** `flip_readme_row_flips_matching_row_only`
(`mcp/src/finalize.rs:346-360`) asserts `lines[0].contains("| review |")` — a
substring check that passes on the malformed `| in-progress | review |` output
just as readily as on the correct `| review |`-only row. The test never asserts
the stale cell is *gone*, nor that the row has the expected cell count.

## What should happen

The status cell should be **replaced**, not appended to. The row should read
`| 06a | ... | review |` with exactly one status cell — matching the milestone
README template's phase-table shape (`docs/dev/WORKFLOW.md` § "Milestone README
template").

## How to fix

In `mcp/src/finalize.rs`, splice out the old cell interior instead of gluing a
new cell onto the untouched prefix. Using the already-computed
`second_last_pipe` (line 185):

```rust
format!(
    "{}| review |{}",
    &line[..second_last_pipe + 1],
    &line[last_pipe + 1..]
)
```

(`&line[..second_last_pipe + 1]` keeps everything up to and including the
delimiter *before* the status cell, dropping the stale `" in-progress "` text
that `&line[..last_pipe]` was wrongly retaining.)

Strengthen `flip_readme_row_flips_matching_row_only` to assert the row does
**not** also contain `"in-progress"` after the flip (the negative the current
substring-only assertion misses), and add a case asserting the row has exactly
one status cell (e.g. by splitting on `|` and checking the trimmed cell count,
or an exact-equality assertion on the full expected row string instead of
`contains`).

## Verification

- [ ] `flip_readme_row_flips_matching_row_only` passes and additionally asserts
      `!lines[0].contains("in-progress")` after the flip.
- [ ] A live `finalize_complete` run against a `todo`/`in-progress` README row
      produces a row with exactly one status cell (`review`), not two.
