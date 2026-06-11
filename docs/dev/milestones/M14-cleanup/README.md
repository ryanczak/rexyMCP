# M14 — Cleanup

**Goal:** Fix the silent task-seeder failure, fold the calibration lesson into
`WORKFLOW.md`, and gather the deferred M12/M13 cleanup items into a single sweep.

**Status:** in-progress

**Depends on:** M13 (complete)

**Exit criteria:**
- [ ] `seed_from_spec` parses both `N. **Title**` list items and `### N. Title`
      subheadings from `## Spec`; the stop condition breaks only at `## ` section
      boundaries, not at `### ` task-subheadings.
- [ ] A warning `Progress` event is emitted at turn 0 when `task_tracking` is on
      but `## Spec` seeds zero tasks — the empty state is observable in the
      Activity panel.
- [ ] `WORKFLOW.md` phase-doc template documents both accepted Spec formats.
- [ ] Deferred M12/M13 cleanup items resolved (prod `eprintln!` ×2, stale
      doc-comment, `symbols` copy bug).
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Architecture references

- `docs/architecture.md` — no change required; this milestone is maintenance-only.

## Phases

| # | Phase | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Fix task seeder: `### N.` headings + empty-spec warning ([phase-01-task-seeder.md](phase-01-task-seeder.md)) | done | bugfix | s |
| 02 | Deferred cleanup sweep: prod `eprintln!`, stale doc-comment, `symbols` copy bug | todo | chore | s |

Phase 02 is undrafted; expand on demand via `/rexymcp:architect next` after phase 01
is approved.

## Notes

### Why M14 exists

M13 closed cleanly (8/8 approved_first_try). Post-close investigation of the
phase-08 executor session (`6a2a0de6`) revealed that the task seeder had been
silently producing zero tasks for every M13 phase from 03 onward — six sessions
affected. The root cause (`seed_from_spec` stopping at `### ` headings, which
became the de-facto M13 spec format from phase-03 onward) was caught only when the
user noticed the Tasks panel was empty and the executor called
`update_task(id="08")` — improvising a task id from the phase number.

**Calibration status:** 6+ occurrences across M13 — well past the WORKFLOW "3 =
fold immediately" threshold. Phase 01 of this milestone carries the code fix and
the WORKFLOW fold simultaneously.

### Deferred items (phase 02 scope)

From M12 retrospective, not M13 scope:
1. Two prod `eprintln!` at `mcp/src/server.rs:426` / `:450`.
2. Stale `RUNAWAY_OUTPUT_BYTES` doc-comment in `executor/src/tools/read_file.rs:17`.
3. `format_references` truncation-note copy bug in `executor/src/tools/symbols.rs`
   (says "add a kind filter" in references mode where `kind` is rejected — noted at
   M12 phase-03 review).

### Operational follow-up (still open, not a code change)

**Restart `rexymcp serve`** to activate M11 phase-06's datetime injection and end
executor self-stamping in Update Logs. Open since M11; still the single
highest-value operational action before the next dispatch.
