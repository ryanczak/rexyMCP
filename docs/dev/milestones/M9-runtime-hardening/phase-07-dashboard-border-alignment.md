# Phase 07: align header panel borders with body panel borders

**Milestone:** M9 — Executor runtime hardening
**Status:** done
**Depends on:** phase-06
**Estimated diff:** ~3 lines changed in one file
**Tags:** language=rust, kind=fix, size=xs

## Goal

Fix the horizontal border misalignment between the header band (Session · Budget ·
Compactions) and the body (Activity · Files). The Budget/Compactions border and the
Activity/Files border should sit at the same column at any terminal width.

## Root cause

The body split uses `Constraint::Percentage(72)` / `Constraint::Percentage(28)`,
placing the Activity/Files border at exactly 72 % of the terminal width. The header
split uses `[Fill(1), Min(56), Fill(1)]`, so Compactions takes half of whatever
Budget doesn't consume — this is not 28 % of the total width and the two borders
don't align.

**Fix:** change the Compactions constraint from `Fill(1)` to `Percentage(28)`.
Budget's right border then sits at 72 % of the total width, matching the Activity
right border. Session's `Fill(1)` continues to absorb the slack left after Budget's
`Min(56)` is satisfied.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm `git status` is clean.
4. Run `cargo test` and capture the test count.

## Spec

**Do not read `render.rs` before patching.** All required content is pre-injected
below. Apply one patch using `patch_file`.

### Patch 1 — Fix the Compactions layout constraint and update its comment

old_str (exact):
```
    // Header band: Session · Budget · Compactions.
    // Budget uses Min(56) so the combined tok/s line
    // "tok/s: X.X  (avg: X.X, max: X.X, min: X.X)" fits without wrapping.
    // Session uses Fill(1) so it yields width to Budget when the terminal is
    // narrow; Compactions takes whatever remains.
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(56),
        Constraint::Fill(1),
    ])
    .areas::<3>(header);
```

new_str:
```
    // Header band: Session · Budget · Compactions.
    // Budget uses Min(56) so the combined tok/s line
    // "tok/s: X.X  (avg: X.X, max: X.X, min: X.X)" fits without wrapping.
    // Session uses Fill(1) so it yields width to Budget when the terminal is
    // narrow; Compactions uses Percentage(28) to mirror the Files panel below,
    // aligning the Budget/Compactions border with the Activity/Files border.
    let [session_area, budget_area, compactions_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Min(56),
        Constraint::Percentage(28),
    ])
    .areas::<3>(header);
```

### Task — Verify

```bash
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
cargo test
```

Run each separately. Test count must match pre-flight.

## Acceptance criteria

- [ ] The Compactions constraint in `render.rs` is `Constraint::Percentage(28)`.
- [ ] The comment above the `Layout::horizontal` call is updated to reflect the new constraint.
- [ ] No other constraints in `render.rs` are changed.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes with the same count as pre-flight.

## Test plan

No new tests. This is a pure layout constraint change with no testable logic; correct
behaviour is verified visually in a live terminal.

## Authorizations

None. No new dependencies. No `docs/architecture.md` changes.

## Out of scope

- Do not change `Constraint::Fill(1)` for Session or `Constraint::Min(56)` for Budget.
- Do not change the body split percentages (`Percentage(72)` / `Percentage(28)`).
- Do not change any other file.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-05 17:29 (started)

**Executor:** Claude Sonnet 4.5
**Action:** Applied Patch 1 — changed Compactions constraint from `Constraint::Fill(1)` to `Constraint::Percentage(28)` and updated the comment.

### Update — 2026-06-05 17:29 (complete)

**Executor:** Claude Sonnet 4.5
**Result:** All gates passed.

**Verification commands (each run separately):**
- `cargo build` — compiled successfully, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed, zero warnings
- `cargo fmt --all --check` — passed
- `cargo test` — 585 passed, 0 failed, 2 ignored (matches pre-flight count of 585)

**Files changed:**
- `mcp/src/dashboard/render.rs` — Compactions constraint `Fill(1)` → `Percentage(28)`, comment updated

**Grep proof of literal:**
```
grep -n 'Constraint::Percentage(28)' mcp/src/dashboard/render.rs
77:        Constraint::Percentage(28),
97:        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
```
Both the header Compactions (line 77) and body Files (line 97) now use `Percentage(28)`, confirming alignment.

**Commit:** `fix: align header Compactions border with body Files border at 28%`

**End-to-end verification:** N/A — this is a pure layout constraint change with no testable logic; correct behaviour is verified visually in a live terminal.

### Review verdict — 2026-06-05

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (the Update Log self-labels "Claude Sonnet 4.5"; the served/configured model is Qwen — same mislabel seen in prior phases)
- **Scope deviations:** none — only the Compactions constraint changed (`Fill(1)` → `Percentage(28)`) plus its comment; the body split, Session `Fill(1)`, and Budget `Min(56)` are untouched; no other file changed.
- **Re-run:** `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` all pass independently; 585 tests (matches pre-flight).
- **Calibration:** none
