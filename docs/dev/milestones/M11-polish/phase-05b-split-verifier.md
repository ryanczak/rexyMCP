# Phase 05b: Split `verifier.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** todo
**Depends on:** phase-05a (ordering only)
**Estimated diff:** ~0 net lines (move only)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`executor/src/governor/verifier.rs` is 1 163 lines: approximately 330 lines of
production code (`FileVerifier` trait + `RustVerifier` implementation) followed by
~833 lines of `#[cfg(test)] mod tests { … }`. Extract the test block into
`executor/src/governor/verifier_tests.rs`.

**Zero logic changes.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor` and confirm all tests pass on HEAD.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`executor/src/governor/verifier.rs` (1 163 lines):
- Lines 1–330 (approx): production code — `FileVerifier` trait, `VerifierResult`,
  `Baseline`, `Diagnostic`, `Severity`, `RustVerifier`, and helpers.
- Lines 331–1163 (approx): `#[cfg(test)] mod tests { … }`.

## Spec

### Step 1: determine exact boundary

```bash
grep -n "^#\[cfg(test)\]" executor/src/governor/verifier.rs
wc -l executor/src/governor/verifier.rs
```

### Step 2: create `executor/src/governor/verifier_tests.rs`

Read `verifier.rs` in two range-reads (~600 lines each). Write `verifier_tests.rs`
containing the inner body of the `mod tests { … }` block (without the outer
wrapper). The file begins with the `use` imports and test helpers from the original
block, copied exactly.

### Step 3: trim `verifier.rs` and add declaration

Remove the `#[cfg(test)]` block and append:

```rust
#[cfg(test)]
#[path = "verifier_tests.rs"]
mod tests;
```

### Step 4: verify and format

```bash
cargo build -p rexymcp-executor
cargo test -p rexymcp-executor
rustfmt executor/src/governor/verifier.rs executor/src/governor/verifier_tests.rs
cargo fmt --all --check
```

## Acceptance criteria

- [ ] `executor/src/governor/verifier.rs` is ≤ 340 lines (production code only).
- [ ] `executor/src/governor/verifier_tests.rs` exists and contains all tests.
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes — same test count as before.
- [ ] `cargo fmt --all --check` passes.
- [ ] No logic change of any kind.

## Notes for executor

- `verifier.rs` is 1 163 lines — readable in two range-reads of ~600 lines each.
- The verifier tests spawn a real `rustc` subprocess (they are `#[ignore]`-gated
  live tests) as well as hermetic unit tests. Do NOT add or remove any
  `#[ignore]` annotations during the move.
- After the split, `use super::*;` in `verifier_tests.rs` refers to items in
  `verifier.rs` (the parent module). This is the same visibility as before.
- Check if the test block uses `use super::` or `use crate::governor::verifier::*`
  — copy the exact import style.

## Update Log

<!-- entries appended below this line -->
