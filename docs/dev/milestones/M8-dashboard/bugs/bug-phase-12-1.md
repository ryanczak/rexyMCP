# Bug 1 on phase-12: `#[allow(clippy::too_many_arguments)]` must be removed

**Severity:** blocker
**Status:** open
**Filed:** 2026-06-04

## What's wrong

`mcp/src/dashboard.rs:683` has:

```rust
#[allow(clippy::too_many_arguments)] // spec-mandated spinner param pushes to 8
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    offset: u16,
    follow: bool,
    rates: BudgetRates,
    spinner: Option<usize>,
) {
```

The `#[allow]` silences the `clippy::too_many_arguments` diagnostic that would
otherwise fail the `-D warnings` gate. STANDARDS.md and the DoD both prohibit
`#[allow]` attributes to mask diagnostics.

## What should happen

Introduce a `ViewState` struct that groups the mutable render-time parameters,
reducing `render_dashboard` to ≤ 7 arguments and eliminating the need for the
`#[allow]`. The struct must be designed to accommodate phase-13's `filter_state`
addition (9 params without a struct; 6 with one):

```rust
/// Run-loop state threaded into each render call.
struct ViewState {
    offset: u16,
    follow: bool,
    spinner: Option<usize>,
}
```

New `render_dashboard` signature (6 params — under the clippy limit):

```rust
fn render_dashboard(
    frame: &mut Frame,
    area: Rect,
    data: &DashboardData,
    now_ms: u64,
    rates: BudgetRates,
    state: &ViewState,
) {
```

Inside `render_dashboard`, replace `offset`, `follow`, `spinner` references
with `state.offset`, `state.follow`, `state.spinner`.

In `run_loop`, build the struct before `terminal.draw`:

```rust
    let view = ViewState { offset, follow, spinner };
    terminal.draw(|frame| {
        render_dashboard(frame, frame.area(), &data, now_ms, rates, &view)
    })?;
```

`offset` and `follow` remain as `mut` locals in `run_loop`; the struct is
constructed fresh each tick from the current locals. **Do not move `offset`
or `follow` into a persistent `ViewState` — that would complicate the
mutation in the key-handler.**

## How to fix

1. Remove `#[allow(clippy::too_many_arguments)]` from `render_dashboard`
   (`mcp/src/dashboard.rs:683`).
2. Add `struct ViewState { offset: u16, follow: bool, spinner: Option<usize> }`
   (near the other local structs, after `BudgetRates` or the constants).
3. Change `render_dashboard` signature: remove `offset`, `follow`, `spinner`
   params; add `state: &ViewState`.
4. Update body: `state.offset` / `state.follow` / `state.spinner`.
5. Update the single `terminal.draw` call site in `run_loop` to construct
   `ViewState { offset, follow, spinner }` and pass `&view`.
6. Run `cargo clippy --all-targets --all-features -- -D warnings` — must pass
   clean with no `#[allow]`.

No other changes. `offset` and `follow` remain as `let mut` locals in
`run_loop`.

## Verification

- [ ] `grep -n "#\[allow" mcp/src/dashboard.rs` returns no matches.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test` — 237 mcp + 585 executor tests pass, 0 failed.
