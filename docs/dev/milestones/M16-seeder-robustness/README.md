# M16 — Seeder Format Robustness

**Goal:** Broaden the task seeder so it recognizes the `### Task N — Title`
heading style the architect naturally writes, in addition to the two formats it
already parses (`N. Title` list items and `### N. Title` subheadings).

**Status:** complete (1/1 approved_first_try, 2026-06-10)

**Depends on:** M14 (complete — added the `### N. Title` heading parser)

## Why M16 exists

M14 phase-01 fixed the silent empty-seed failure by teaching `seed_from_spec`
to parse `### N. Title` subheadings (the de-facto M13 spec format). But the
architect's *natural* heading style is `### Task N — Title` (used throughout
M15's phase docs), which matches **neither** recognized pattern. Result: M15
phase-02 seeded **zero tasks** — the turn-0 warning fired correctly, and the
executor improvised `update_task(id="02")` (the phase number), which the tool
correctly rejected with `no task with id "02"`.

**This is not an `update_task` bug** — the tool returned the designed
model-visible advisory for an unknown id. The gap is the seeder/spec-format
mismatch, now recurring across M13 → M14 → M15 with a third heading variant.

Per the 2026-06-10 decision (with the user), the fix is **both**:
1. **Code** (this milestone): extend `parse_heading_task_line` to accept
   `### Task N — Title` / `### Task N: Title` / `### Task N. Title`.
2. **Convention** (architect, ongoing): prefer a recognized format in new phase
   docs. M15 phase-03's Spec was reformatted to `### N. Title` at the same time.

## Exit criteria

- [ ] `seed_from_spec` seeds tasks from `### Task N — Title`, `### Task N: Title`,
      and `### Task N. Title` headings, in addition to the existing `N. Title`
      and `### N. Title` formats.
- [ ] The existing `### N. Title` and `N. Title` paths are unchanged (all prior
      `tasks.rs` tests pass without modification).
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Architecture references

- `executor/src/agent/tasks.rs` — `seed_from_spec`, `parse_heading_task_line`
- M14 README — the prior seeder fix this extends.

## Phases

| # | Phase | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Recognize `### Task N —` heading task format ([phase-01-heading-task-formats.md](phase-01-heading-task-formats.md)) | done | fix | xs |

Phase 01 is the only in-scope M16 phase; it closes the milestone once approved.

## Notes

### Operational follow-up (architect, not executor)

`WORKFLOW.md`'s "accepted Spec formats" documentation (added in M14 phase-01)
should be updated to list the `### Task N —` format now that this phase has
landed. That is a **contract-doc change** — the architect makes it with the
user, not the executor (the executor cannot touch `WORKFLOW.md` per STANDARDS
§5). Not in this phase's scope; **still open** for the architect to action with
the user (see the [talk-through-contract-doc-changes] convention).

### Retrospective — 2026-06-10

**Outcome:** complete in a single phase, **approved_first_try**, zero bounces,
zero escalations. The narrowest milestone to date (one xs single-file `fix:`).

**What worked:** the phase doc pre-supplied the exact replacement function body
and the full test block, so the executor's job was a faithful transcription
against a clear additive shape. The deliberate additive framing — new `Task `
prefix branch *above* a byte-identical dot-branch — meant every prior `tasks.rs`
test passed unmodified, and the only new risk surface was the new branch. Clean
35-turn first-try.

**Recurring-quirk watch:** the local-LLM Update-Log clock/identity self-stamp
quirk (tracked across M11–M15) did **not** recur — the executor stamped
`2026-06-11 04:48`, plausibly real, suggesting M11 phase-06's datetime injection
is now live after the `rexymcp serve` restart. One data point; keep watching.

**Convention half of the "Both" decision:** the code fix landed; the matching
M15 phase-03 Spec reformat to `### N.` was already committed at M16 kickoff
(`d3410c6`). The seeder now accepts all three heading variants
(`N.` list-item, `### N.`, `### Task N — / : / .`), closing the M13→M14→M15
format-drift gap. The remaining open item is the `WORKFLOW.md` accepted-formats
doc update (architect + user, above).

**Calibration:** none — no new stall classes, no fold.
