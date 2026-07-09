# Phase 04b: Finalize tolerates a bounced status line

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** in-progress
**Depends on:** phase-03a (server-authored finalize), phase-04 (surfaced this defect)
**Estimated diff:** ~80 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

Fix the 03a server-authored finalize so it works on a **bounced** phase. Today
`finalize_complete` matches the phase-doc status line and README row **exactly**
against `in-progress`, but the review skill's bounce convention appends a note
(`**Status:** in-progress (bounced — see bugs/bug-04-1.md)`), so finalize
silently no-ops on any bounced-then-completed phase — the server writes no status
flip and no completion entry. This must land **before phase-06**: the autonomous
loop bounces and re-dispatches as normal operation, so without this fix the
marquee server-authored-finalize feature disengages exactly where it is needed.

## Architecture references

- M27 [README](README.md) § Exit criteria (server writes the flip + baseline
  entry on a `complete` run) and the phase-04 Review verdict (this defect's
  first occurrence).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Three match sites in `mcp/src/finalize.rs`, all exact and all defeated by a
bounce note:

- `status_is_in_progress` (`finalize.rs:52`): `line.trim() == "**Status:** in-progress"`.
- `flip_status_to_review` (`finalize.rs:60`): guards on the same `==` and then does
  `line.replace("**Status:** in-progress", "**Status:** review")` — note that even
  if the guard were relaxed, this substring replace on a bounced line would leave a
  **stale** `**Status:** review (bounced — …)`, which is wrong.
- `flip_readme_row` (`finalize.rs:164`): matches a row that
  `line.trim().ends_with("| in-progress |")`.

The phase-04 review reproduced the failure: `finalize_complete` returned
`Ok(false)` for a doc whose status line was
`**Status:** in-progress (bounced — see bugs/bug-04-1.md)`, so the architect had
to hand-author the flip + completion entry.

Existing `finalize.rs` tests pin the exact-match *positives* and the
`review`/`todo`/`done`/prose *negatives* — none asserts a bounced line is
*rejected*, so relaxing the match to a prefix does not break them. Re-verify by
running them.

## Spec

### 1. Prefix-tolerant in-progress predicate

In `mcp/src/finalize.rs`, add a private helper both status functions share (so
the match rule can't drift between them):

```rust
/// True iff `trimmed` is an in-progress status line, with or without a trailing
/// note (the review skill appends `(bounced — …)` on a bounce). The space before
/// the note is the delimiter, so `**Status:** in-progressish` does NOT match.
fn is_in_progress_status(trimmed: &str) -> bool {
    trimmed == "**Status:** in-progress" || trimmed.starts_with("**Status:** in-progress ")
}
```

Use it in `status_is_in_progress` (replace the `==` comparison). The space in the
`starts_with` arm is load-bearing — it is what keeps `in-progressish` /
`in-progress-foo` out.

### 2. `flip_status_to_review` drops the bounce note

Change the per-line guard to `is_in_progress_status(line.trim())`. On a match,
emit the line's **leading whitespace + `**Status:** review`** (the canonical
line), **not** a substring replace — so a bounced line becomes exactly
`**Status:** review` with the `(bounced — …)` note **removed** (it is stale once
the phase reaches review). A clean (unnoted) line still becomes `**Status:** review`,
byte-identical to today. Preserve the first-match-only behavior and the trailing
newline handling already in the function.

### 3. `flip_readme_row` tolerates a noted status cell

Change the row match so it fires when the row contains `phase_doc_filename` **and**
the row's **last table cell** (the text between the final two `|`, trimmed)
**starts with** `in-progress`. On a match, replace that last cell with ` review `
(dropping any note, mirroring Task 2), leaving the rest of the row and all other
rows byte-identical. A row whose last cell is `review` / `done` must **not** match.

## Acceptance criteria

- [ ] `cargo build` zero new warnings; `cargo clippy` and `cargo fmt --all --check`
      pass; `cargo test` passes (existing + new).
- [ ] `status_is_in_progress("**Status:** in-progress (bounced — see bugs/bug-04-1.md)")`
      is `true`; `"**Status:** review"`, `"**Status:** done"`, `"**Status:** todo"`,
      and `"**Status:** in-progressish"` are all `false`.
- [ ] `flip_status_to_review` turns a bounced status line into exactly
      `**Status:** review` (no residual `(bounced …)`), and a clean line's result is
      byte-identical to today.
- [ ] `flip_readme_row` flips a row whose last cell is `in-progress (bounced, bug-04-1)`
      to `review`, and returns `None` for a `review`/`done` row.
- [ ] `finalize_complete` on a `Complete` result whose doc status line carries a
      bounce note flips the doc to `**Status:** review` and appends the
      `(complete, server-authored)` entry (the end-to-end proof).

## Test plan

Add to the `finalize.rs` test module (mirror the existing test shapes):

- `status_is_in_progress_matches_bounced_line` — the noted line is `true`.
- `status_is_in_progress_rejects_in_progressish` — the space-delimiter negative.
- `flip_status_to_review_drops_bounce_note` — bounced line → exactly `**Status:** review`.
- `flip_readme_row_flips_bounced_row` — noted row cell → `review`; a sibling
  `review` row is untouched.
- `finalize_flips_bounced_status_and_appends_entry` — the integration proof:
  a `Complete` result + a `TempDir` phase doc whose status line is
  `**Status:** in-progress (bounced — see bugs/bug-04-1.md)` finalizes to
  `review` with the server-authored entry present (mirror
  `finalize_flips_status_and_appends_entry`).

## End-to-end verification

Quote the `finalize_flips_bounced_status_and_appends_entry` run showing the
`TempDir` doc's status line before (`in-progress (bounced …)`) and after
(`review`) plus the appended `(complete, server-authored)` entry. That exercises
the real `finalize_complete` path end-to-end (the same fake `RecordingRunner`
the sibling finalize integration tests use).

## Authorizations

None. Pure `mcp/src/finalize.rs` change (production + tests). No new dependency,
no `Cargo.toml` / `architecture.md` / `STANDARDS.md` / `WORKFLOW.md` / contract /
skill edit.

## Out of scope

- **No change to the review skill's bounce convention.** This phase fixes the
  server side (option A from the phase-04 verdict); the human-readable
  `(bounced — …)` note on the status line stays.
- **No change to the completion-entry format, the git commit logic, or the
  `Complete`-status gate** in `finalize_complete` — only the three match/flip
  helpers.
- Do not touch any phase doc other than this one (the phase-04 doc's status line
  is already resolved).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-09 03:23 (started)

**Executor:** Claude (Sonnet 4.5)
**Action:** Started phase 04b implementation.
