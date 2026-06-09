# Phase 04: Split `scorecard.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** todo
**Depends on:** phase-03 (ordering only — no code dependency; can run after phase-01)
**Estimated diff:** ~0 net lines (move only)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`mcp/src/scorecard.rs` is 1 153 lines: ~390 lines of production code
(`ScorecardRow`, `SettingsScorecardRow`, `ScorecardFilter`, `Accumulator`,
`SettingsAccumulator`, `gates_all_pass`, `aggregate`, `aggregate_by_settings`,
`MAX_ROWS`) followed by ~763 lines of `#[cfg(test)] mod tests { … }`. Extract the
test block into `mcp/src/scorecard_tests.rs`, leaving `scorecard.rs` as
production-code-only (≤ 400 lines).

**Zero logic changes.** Every test stays exactly as-is.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp` and confirm all tests pass on HEAD.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`mcp/src/scorecard.rs` (1 153 lines):
- Lines 1–390 (approx): production code — two row types, two accumulator types,
  filter, two `aggregate` functions, `MAX_ROWS`.
- Lines 392–1153 (approx): `#[cfg(test)] mod tests { … }`.

## Spec

### Step 1: determine exact boundary

```bash
grep -n "^#\[cfg(test)\]" mcp/src/scorecard.rs
wc -l mcp/src/scorecard.rs
```

### Step 2: create `mcp/src/scorecard_tests.rs`

`scorecard.rs` is 1 153 lines — within range-read range if split into two reads.
Read the production block (lines 1 to the `#[cfg(test)]` line) and the test block
(from `#[cfg(test)]` to end), separately.

`scorecard_tests.rs` contains the inner body of `mod tests { … }` (everything
between the outer braces, not including them). It begins with the `use` imports
from the original test block. The `use super::*;` becomes `use super::*;` (still
correct — `scorecard.rs` is the parent module).

### Step 3: trim `scorecard.rs` and add declaration

Remove the `#[cfg(test)]` block (from its opening attribute through the closing
`}`) and append:

```rust
#[cfg(test)]
mod tests;
```

Because there is no `mod scorecard_tests;` declaration — in Rust the file
`scorecard_tests.rs` in the same directory is NOT automatically a module. The
Rust module system requires an explicit declaration. The convention when the test
file is a sibling is:

```rust
// At the bottom of scorecard.rs:
#[cfg(test)]
#[path = "scorecard_tests.rs"]
mod tests;
```

Use the `#[path]` attribute because the file is named `scorecard_tests.rs` (not
`tests.rs`), which Rust would not find automatically under the name `tests`.

### Step 4: verify and format

```bash
cargo build -p rexymcp
cargo test -p rexymcp
rustfmt mcp/src/scorecard.rs mcp/src/scorecard_tests.rs
cargo fmt --all --check
```

## Acceptance criteria

- [ ] `mcp/src/scorecard.rs` is ≤ 400 lines (production code only).
- [ ] `mcp/src/scorecard_tests.rs` exists and contains all tests.
- [ ] `cargo build -p rexymcp` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp` passes — same test count as before.
- [ ] `cargo fmt --all --check` passes.
- [ ] No logic change of any kind.

## Notes for executor

- The `#[path = "scorecard_tests.rs"]` attribute is required because the file name
  does not match the module name `tests`. Without it, the compiler looks for
  `tests.rs` or `tests/mod.rs` and will not find `scorecard_tests.rs`.
- `scorecard.rs` is 1 153 lines — readable in two range-reads of ~600 lines each.
  Do NOT attempt a full single read.
- After moving, `use super::*;` in `scorecard_tests.rs` refers to the production
  items in `scorecard.rs` — same as before the split.

## Update Log

<!-- entries appended below this line -->
