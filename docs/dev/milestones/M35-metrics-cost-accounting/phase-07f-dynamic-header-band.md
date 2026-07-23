# Phase 07f: Dynamic header-band height — no trailing blank on the tallest header panel

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-07e
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=fix, size=m

## Goal

The dashboard's three header panels — **Session / Budget / Context** — sit side-by-side
in one **fixed-height** band (`Layout::vertical([Length(11), …])` = 2 border + 9 content
rows). That 9 was sized long ago to the tallest panel (Budget). Since then 07d removed the
Budget `Assists:` row and 07e combined the two token lines, so Budget shrank by ~2 rows
and the fixed band is now **over-provisioned** — every panel shows extra trailing blank
rows.

**Fix (user-approved, 2026-07-22): size the band dynamically to the tallest of the three
panels' actual content** — `max(session, budget, context) + 2 borders` — instead of the
stale fixed `11`. This removes the over-provisioning (the tallest panel is blank-free; the
idle/short state no longer shows a big gap) and self-adjusts to future content changes.

**Honest limit (shared-band physics):** the three panels share one *horizontal* band, so
they are forced to the same height. A panel genuinely shorter than the tallest (e.g.
Budget while an active run makes Session taller) still shows a trailing blank equal to its
shortfall. Truly zeroing all three would require stacking the panels vertically — a bigger
redesign the user chose **not** to do. This phase only removes the *over-provisioning*.

## ⚠️ Read this first — editing discipline on the dashboard

`mcp/src/dashboard/render.rs` is layout-critical rendering code. If a build/syntax error
appears after an edit, run `cargo build -p rexymcp` **once** and read the compiler's error
location, then open **only that region** with `read_file` and fix it. **Never** hunt for a
problem by re-reading the whole file in a loop with `python3 -c`/`bash`/`sed`/`cat` —
repeating the same read-only inspection command trips the governor's oscillation
terminator (it has hard-failed **three** runs on `panels.rs`/dashboard this milestone).
Read once, fix once, verify with `cargo build`.

## Architecture references

Read before starting:

- `mcp/src/dashboard/render.rs` — `render_dashboard` (164–end); the header-band section to
  reorder is **184–239** (the fixed vertical split at 191–192, the horizontal split at
  200–205, and the Session/Budget/Context builds+renders at 207–239). Quoted in full below.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above **and the editing-discipline note above**.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. `cargo build -p rexymcp` after each change.

## Current state

**The header-band section of `render_dashboard`** (`render.rs:184–239`) — note the
ordering: the **fixed** vertical split happens **first** (so the height is decided before
the panel content is built):

```rust
    // Outer split: fixed-height header band + filling body.
    // Height 11 = 2 border rows + 9 content rows. ... [stale comment]
    let total_wrapped;
    let [header, body] =
        Layout::vertical([Constraint::Length(11), Constraint::Min(0)]).areas::<2>(area);

    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(52),
        Constraint::Percentage(28),
    ])
    .areas::<3>(header);

    let session_inner_width = session_area.width.saturating_sub(2) as usize;
    let mut session = Vec::new();
    if let Some(name) = &data.milestone {
        session.push(milestone_line(name, session_inner_width));
    }
    session.extend(session_lines(&data.summary, now_ms));
    if let Some(line) = spinner_line(state.spinner, session_inner_width) {
        session.push(line);
    }
    frame.render_widget(panel(" Session ", session), session_area);
    let mut budget = Vec::new();
    budget.extend(budget_lines(&data.summary));
    budget.extend(savings_lines(
        &data.summary,
        rates,
        data.milestone_costs,
        data.project_costs,
        data.project_escalation_count,
        state.budget_display,
    ));
    if let Some(ref ts) = data.top_skill
        && ts.cost > 0.0
    {
        budget.push(Line::from(format!(
            "  Top skill: {} ${:.2}",
            ts.skill, ts.cost
        )));
    }
    frame.render_widget(panel(" Budget [b=toggle view] ", budget), budget_area);
    frame.render_widget(
        panel(" Context ", reclaim_lines(&data.summary)),
        compactions_area,
    );
```

**Key fact that makes the reorder safe:** the three column **widths** come from
`Layout::horizontal([Fill(1), Min(52), Percentage(28)])`, which distributes **width** only
— it is **independent of the band's height**. So we can split for widths *before* deciding
the band height, then re-split for the final areas once the height is known. The widths
match because both splits use the same constraints on the same width.

## Spec

### Task 1 — add a pure `header_band_height` helper (render.rs)

Add a small private fn (near `render_dashboard`) — the pure, testable core of the fix:

```rust
/// Rows for the header band: the tallest of the three header panels' content plus
/// 2 border rows. The panels share one horizontal band, so it fits the tallest;
/// a shorter panel shows a trailing blank equal to its shortfall.
fn header_band_height(session_len: usize, budget_len: usize, context_len: usize) -> u16 {
    let max = session_len.max(budget_len).max(context_len);
    (max as u16).saturating_add(2)
}
```

### Task 2 — reorder `render_dashboard` to build panels first, then size the band (render.rs)

Replace the `render.rs:184–239` block above with the reordered version below: probe the
column widths, build all three panel `Vec<Line>`s, size the band via `header_band_height`,
then do the vertical + final horizontal splits and render the (already-built) panels.

```rust
    // Header band sized to the tallest of the three panels (Session / Budget /
    // Context) + 2 borders. They share one horizontal band, so it fits the
    // tallest; a shorter panel shows a trailing blank equal to its shortfall.
    let total_wrapped;

    // Column widths depend only on the header's WIDTH (not its height), so split
    // `area` to get them before the band height is known.
    let [session_probe, _budget_probe, _compactions_probe] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(52),
        Constraint::Percentage(28),
    ])
    .areas::<3>(area);
    let session_inner_width = session_probe.width.saturating_sub(2) as usize;

    // Build the three panels' content up front so the band can fit the tallest.
    let mut session = Vec::new();
    if let Some(name) = &data.milestone {
        session.push(milestone_line(name, session_inner_width));
    }
    session.extend(session_lines(&data.summary, now_ms));
    if let Some(line) = spinner_line(state.spinner, session_inner_width) {
        session.push(line);
    }

    let mut budget = Vec::new();
    budget.extend(budget_lines(&data.summary));
    budget.extend(savings_lines(
        &data.summary,
        rates,
        data.milestone_costs,
        data.project_costs,
        data.project_escalation_count,
        state.budget_display,
    ));
    if let Some(ref ts) = data.top_skill
        && ts.cost > 0.0
    {
        budget.push(Line::from(format!(
            "  Top skill: {} ${:.2}",
            ts.skill, ts.cost
        )));
    }

    let context = reclaim_lines(&data.summary);

    let band = header_band_height(session.len(), budget.len(), context.len());
    let [header, body] =
        Layout::vertical([Constraint::Length(band), Constraint::Min(0)]).areas::<2>(area);
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(52),
        Constraint::Percentage(28),
    ])
    .areas::<3>(header);

    frame.render_widget(panel(" Session ", session), session_area);
    frame.render_widget(panel(" Budget [b=toggle view] ", budget), budget_area);
    frame.render_widget(panel(" Context ", context), compactions_area);
```

Notes:
- `let total_wrapped;` stays declared here (the body/Activity section below assigns it,
  unchanged).
- The `_budget_probe`/`_compactions_probe` from the probe split are unused (only the
  session width is needed) — the `_` prefix keeps `-D warnings` clean.
- **Do not touch** anything after line 239 (the body split, the Activity/Tasks/Files
  rendering, `total_wrapped`, the return). Only the header-band section changes.

## Acceptance criteria

- [ ] `header_band_height` returns `max(session, budget, context) + 2`:
      `(7,5,3)→9`, `(3,8,4)→10`, `(6,6,6)→8`, `(0,0,0)→2`.
- [ ] `render_dashboard` builds the three header panels **before** the vertical split and
      sizes the band with `header_band_height(session.len(), budget.len(), context.len())`
      — no hard-coded `Length(11)` remains for the header band.
- [ ] The Session/Budget/Context panels render the same content as before (only the band
      *height* changes); the body/Activity section is unchanged.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.

## Test plan

- **Add `header_band_height_fits_tallest_plus_borders`** in `render.rs`'s `#[cfg(test)]
  mod tests` (create the block at the file bottom if it has none) — the mutation-sensitive
  pin of the core logic:
  ```rust
  #[test]
  fn header_band_height_fits_tallest_plus_borders() {
      assert_eq!(header_band_height(7, 5, 3), 9);   // fits Session (7) + 2 borders
      assert_eq!(header_band_height(3, 8, 4), 10);  // fits Budget (8) + 2
      assert_eq!(header_band_height(6, 6, 6), 8);   // all equal → 6 + 2
      assert_eq!(header_band_height(0, 0, 0), 2);   // borders only
  }
  ```
  (Fails under a `min`-instead-of-`max` mutation or a missing `+2`.)
- `render_dashboard` itself is not unit-testable (it needs a live `Frame`), so it is
  covered by `cargo build` + the E2E eyeball below, not a unit test.

## End-to-end verification

The dashboard is a live TUI (not hermetically capturable). Evidence:
1. Quote the `header_band_height_fits_tallest_plus_borders` test result in the completion
   Update Log.
2. Run `cargo run -p rexymcp -- dashboard --repo .` and confirm it launches and the header
   band is no longer over-tall (the trailing blank on the tallest header panel is gone).
   If you cannot run an interactive TUI in the harness, say so and note that the band-height
   helper test + a clean `cargo build` are the pinned evidence, and that the next real
   dashboard launch shows the tightened band.

## Authorizations

- Editing `mcp/src/dashboard/render.rs` (`render_dashboard` header section + the new helper
  + its test) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Stacking the panels vertically / any restructure that changes the 3-column header
  layout.** The user chose the dynamic-band fix, not a redesign. Keep the side-by-side
  Session/Budget/Context columns and their `Fill(1)`/`Min(52)`/`Percentage(28)` widths.
- **Fully zeroing the trailing blank on a genuinely-shorter panel** — impossible with the
  shared horizontal band (see the Goal's "honest limit"); not attempted here.
- **The body/Activity/Tasks/Files layout and `total_wrapped`** — unchanged.
- **`panels.rs`** — the panel *content* builders are unchanged; this phase only changes how
  the band *height* is chosen in `render.rs`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
