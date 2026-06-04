# Bug 1 on phase-13: Lint gate fails — two clippy errors (derivable_impls + field_reassign_with_default)

**Severity:** blocker
**Status:** open
**Filed:** 2026-06-04

## What's wrong

`cargo clippy --all-targets --all-features -- -D warnings` fails with two
errors, so the phase cannot merge:

1. **`clippy::derivable_impls`** — `mcp/src/dashboard.rs:182`:

   ```rust
   impl Default for FilterState {
       fn default() -> Self {
           Self {
               open: false,
               cursor: 0,
               filter: ActivityFilter::default(),
           }
       }
   }
   ```

   Every field falls back to its type's own `Default` (`bool` → `false`,
   `usize` → `0`, `ActivityFilter` → its `Default` impl), so the manual impl
   is exactly what `#[derive(Default)]` would generate. clippy rejects it.

   (Note: this manual impl was pre-injected verbatim in the phase doc §1
   sketch. `ActivityFilter`'s manual `Default` is **not** derivable — it sets
   `progress: false` against `bool`'s `true`-less default differing from the
   other fields — so leave that one alone. Only `FilterState` is the problem.)

2. **`clippy::field_reassign_with_default`** — `mcp/src/dashboard.rs:2057-2058`,
   test `filter_allows_progress_when_enabled`:

   ```rust
   let mut f = ActivityFilter::default();
   f.progress = true;
   ```

   clippy wants the value built in one initializer rather than default-then-
   reassign.

The executor's `PhaseResult` reported `status: "complete"` despite its own
captured `lint` command output showing both errors — the lint gate must be
green before the phase is reviewable.

## What should happen

`cargo clippy --all-targets --all-features -- -D warnings` exits 0 with no
errors and no new warnings (STANDARDS.md §1 Definition of Done: lint passes).

## How to fix

In `mcp/src/dashboard.rs`:

1. Delete the manual `impl Default for FilterState { … }` block (lines
   ~182–190) and add `Default` to the existing derive on the struct:

   ```rust
   #[derive(Clone, Debug, Default)]
   struct FilterState {
       open: bool,
       cursor: usize,
       filter: ActivityFilter,
   }
   ```

   (`ActivityFilter` already implements `Default`, so the derive composes.)

2. In test `filter_allows_progress_when_enabled`, replace the default-then-
   reassign with struct-update syntax:

   ```rust
   let f = ActivityFilter {
       progress: true,
       ..ActivityFilter::default()
   };
   ```

Do not touch `ActivityFilter`'s manual `Default` impl — it is intentionally
non-derivable (`progress: false`).

## Verification

- [ ] `cargo clippy --all-targets --all-features -- -D warnings` exits 0
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo build` succeeds
- [ ] `cargo test filter` passes (all 8 filter tests)
