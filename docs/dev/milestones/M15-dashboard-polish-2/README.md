# M15 — Dashboard Polish (Round 2)

**Goal:** Four targeted dashboard fixes: use full panel width for task titles;
move the `last update:` line from the Budget panel to the Session panel; update
`$ saved` with current model pricing (Opus 4.8 / Fable 5); and change the
Activity transcript time-delta color to dull yellow.

**Status:** complete

**Depends on:** M14 (complete)

**Exit criteria:**
- [x] Task titles in the Tasks panel use all available panel width instead of
      the hardcoded 24-char cap.
- [x] `last update:` appears in the Session panel (under the `duration:` line),
      not in the Budget panel.
- [x] `$ saved` auto-fills rates for recognized Claude model names
      (`claude-opus-4-8` → $5/$25/MTok; `claude-fable-5` → $10/$50/MTok; etc.)
      via a new optional `saved_model` config field; numeric override still works.
- [x] Activity transcript time-delta spans (`[+0s]`, `[+5s]`, …) are rendered
      in dull yellow (`Rgb(180, 150, 50)`) rather than dim grey.
- [x] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
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
| 01 | Move `last update:` to Session panel + dull yellow timestamps ([phase-01-layout-and-color.md](phase-01-layout-and-color.md)) | done | feat | xs |
| 02 | Width-aware task title truncation ([phase-02-task-width.md](phase-02-task-width.md)) | done | feat | xs |
| 03 | Model-aware `$ saved` pricing ([phase-03-pricing.md](phase-03-pricing.md)) | done | feat | s |

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

### Retrospective — 2026-06-11

**M15 — Dashboard Polish (Round 2) is complete: 3/3 phases approved_first_try,
zero bounces, zero escalations.**

- **Phase 01** (xs) — moved `last update:` to the Session panel + dull-yellow
  `[+Xs]` timestamps; clean 43-turn first-try, commits `77c3c27`/`ef14d74`.
- **Phase 02** (xs) — width-aware task-title truncation (dropped the hardcoded
  `TASK_TITLE_MAX = 24`); clean 40-turn first-try, commit `1eced62`.
- **Phase 03** (s) — model-aware `$ saved` pricing via an optional `saved_model`
  config field; clean 50-turn first-try, commit `38cc819`.

**What worked.** The display-only constraint that carried M13 forward held again:
phases 01–02 touched only `mcp/src/dashboard/` (pure presentation, no config/event
churn), and phase 03's single cross-crate change — `saved_model: Option<String>`
on `DashboardConfig` — was deliberately shaped to dodge both documented stall
classes. It added **no new variant** (no match-arm wall) and touched **one**
non-`Default`-built struct literal region; the `Copy`-drop gotcha (pinned in the
phase doc because `Option<String>` isn't `Copy`) was traversed first-try, and the
`main.rs` wiring stayed a trivial `Option`-fallback expression. Three consecutive
xs/xs/s phases landing first-try confirms the additive-shape lever continues to
neutralize the mechanical-churn stall on small dashboard work.

**Process.** The cosmetic local-LLM identity self-stamp persists (Update Logs read
"Claude (Sonnet)"; executor is Qwen/Qwen3.6-27B-FP8) — but across M15 the
**dates are correct** (`2026-06-11`), confirming the M11 phase-06 datetime
injection is live post-`rexymcp serve` restart. Identity-label drift remains
cosmetic; no fold.

**Carried forward.** The open contract-doc item (documenting the `### Task N —`
heading variant in `WORKFLOW.md`, landed by M16 phase-01) is a human-gated
talk-through, not executor scope. With M15 closed, the next milestone is a fresh
human-gated kickoff.
