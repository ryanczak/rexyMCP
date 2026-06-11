# M15 — Dashboard Polish (Round 2)

**Goal:** Four targeted dashboard fixes: use full panel width for task titles;
move the `last update:` line from the Budget panel to the Session panel; update
`$ saved` with current model pricing (Opus 4.8 / Fable 5); and change the
Activity transcript time-delta color to dull yellow.

**Status:** in-progress

**Depends on:** M14 (complete)

**Exit criteria:**
- [ ] Task titles in the Tasks panel use all available panel width instead of
      the hardcoded 24-char cap.
- [ ] `last update:` appears in the Session panel (under the `duration:` line),
      not in the Budget panel.
- [ ] `$ saved` auto-fills rates for recognized Claude model names
      (`claude-opus-4-8` → $5/$25/MTok; `claude-fable-5` → $10/$50/MTok; etc.)
      via a new optional `saved_model` config field; numeric override still works.
- [ ] Activity transcript time-delta spans (`[+0s]`, `[+5s]`, …) are rendered
      in dull yellow (`Rgb(180, 150, 50)`) rather than dim grey.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Architecture references

- `mcp/src/dashboard/panels.rs` — `tasks_lines`, `last_update_line`,
  `BudgetRates`, `dollars_saved_line`, `truncate_title`
- `mcp/src/dashboard/render.rs` — session/budget vec assembly, `tasks_area`
- `mcp/src/dashboard/transcript.rs` — `transcript_lines`, timestamp span style
- `executor/src/config.rs` — `DashboardConfig` (phase-03 only)
- `mcp/src/main.rs` — `BudgetRates` construction from config (phase-03 only)
- `mcp/src/dashboard/mod.rs` — re-exports (phase-03 only)
- `mcp/src/init.rs` — toml template (phase-03 only)

## Phases

| # | Phase | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Move `last update:` to Session panel + dull yellow timestamps ([phase-01-layout-and-color.md](phase-01-layout-and-color.md)) | review | feat | xs |
| 02 | Width-aware task title truncation ([phase-02-task-width.md](phase-02-task-width.md)) | todo | feat | xs |
| 03 | Model-aware `$ saved` pricing ([phase-03-pricing.md](phase-03-pricing.md)) | todo | feat | s |

Phase 03 is the last in-scope M15 phase; it closes the milestone once approved.

## Notes

### Why M15 exists

M13 (Dashboard Polish, 8 phases) closed clean but left four small cosmetic
gaps visible in daily use:

1. **Task titles cut too early.** The Tasks panel clips titles at 24 chars
   regardless of actual panel width, wasting the space between the truncation
   point and the right border.
2. **`last update:` in the wrong panel.** It was moved to the Budget panel in
   M13 phase-05, but contextually it belongs in the Session panel alongside
   the `duration:` line.
3. **`$ saved` rates are stale by default.** The field defaults to 0.0/0.0
   (showing `—`). Users who want savings estimates must look up and manually
   enter current Claude pricing. Known pricing for Opus 4.8 and Fable 5 should
   be auto-fillable.
4. **Time deltas are too muted.** The dim grey `[+Xs]` timestamps in the
   Activity panel blend into body text; dull yellow adds contrast without
   being garish.

### Display-only constraint

**Phases 01 and 02** are pure presentation layer — zero config changes, zero
new `SessionEvent` variants, zero executor-crate edits. **Phase 03** adds one
optional `saved_model` field to `DashboardConfig` in the executor crate; this
is explicitly authorized in that phase doc. No new dependencies.

### Current pricing (as of 2026-06-04)

| Model | Input $/MTok | Output $/MTok |
|---|---|---|
| `claude-fable-5` / `claude-mythos-5` | $10.00 | $50.00 |
| `claude-opus-4-8` / `claude-opus-4-7` / `claude-opus-4-6` | $5.00 | $25.00 |
| `claude-sonnet-4-6` | $3.00 | $15.00 |
| `claude-haiku-4-5` | $1.00 | $5.00 |
