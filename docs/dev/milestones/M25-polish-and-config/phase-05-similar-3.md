# Phase 05: `similar` 2→3

**Milestone:** M25 — Polish & Config Pass
**Status:** review
**Depends on:** none
**Estimated diff:** ~2 lines
**Tags:** language=rust, kind=refactor, size=s

## Goal

Bump the `similar` diff crate from the `2` major to the `3` major (latest
`3.1.1`). This is the first of the five M25 dependency major-version bumps,
ordered smallest-to-largest blast radius. `similar` is a leaf diff utility with a
small, stable surface; the 3.0 breaking changes do **not** touch any API this
codebase calls, so the expected diff is the single version constraint plus the
`Cargo.lock` update — no source change.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #25 — names M25 as the polish & config pass; the
  dep bumps are committed scope.
- This milestone's [README](README.md) § "Dependency update phases (05–09)" — the
  per-phase ordering and the uniform recipe (bump constraint → `cargo update` →
  fix compiler-flagged breaks → verify four gates).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Confirm the toolchain meets `similar` 3.x's MSRV.** `similar` 3.x requires
   Rust **1.85** (Rust 2024 edition). This workspace is already `edition = "2024"`
   (see the root `Cargo.toml`), which itself requires ≥ 1.85, so the installed
   toolchain already satisfies this. Confirm with `rustc --version` (expect
   ≥ 1.85). If it is somehow below 1.85, **stop and file a blocker** — do not
   attempt a toolchain upgrade.

## Current state

`similar` is a **workspace dependency**. The version is declared once, at the
workspace root, and the executor crate inherits it:

`Cargo.toml` (workspace root), line 29:

```toml
similar = "2"
```

`executor/Cargo.toml`, line 25:

```toml
similar.workspace = true
```

`Cargo.lock` currently pins `similar` at `2.7.0`.

**The crate is used in exactly three executor source files**, all via a small,
stable API surface:

- `executor/src/tools/patch.rs:12` — `use similar::TextDiff;`
  - line 162: `TextDiff::from_lines(&content, &after)`
  - line 268: `TextDiff::from_chars(old_str, &window).ratio()`
- `executor/src/agent/outcome.rs:4` — `use similar::{ChangeTag, TextDiff};`
  - line 105: `TextDiff::from_lines(&before, &after)`
  - lines 109–113: `for change in text_diff.iter_all_changes() { match change.tag() { ChangeTag::Insert => …, ChangeTag::Delete => …, ChangeTag::Equal => {} … } }`
- `executor/src/agent/progress.rs:4` — `use similar::{ChangeTag, TextDiff};`
  - line 62: `TextDiff::from_lines(&before, &after)`
  - lines 67–71: the same `iter_all_changes()` / `change.tag()` / `ChangeTag` match as `outcome.rs`.

## What changed in `similar` 3.0 (and why none of it affects us)

The 3.0 breaking changes, from the upstream CHANGELOG, with the impact on **this**
codebase noted inline:

- **MSRV raised to Rust 1.85, crate moved to Rust 2024 edition.** No impact — this
  workspace is already edition 2024 (toolchain already ≥ 1.85; see Pre-flight 5).
- **Removed `TextDiff::old_slices` / `new_slices`.** No impact — neither is called
  anywhere (the codebase uses `from_lines` / `from_chars` / `iter_all_changes`).
- **Renamed `get_diff_ratio` → `diff_ratio`.** No impact — this is a free function
  in `similar::utils`; the codebase calls the **`TextDiff::ratio()`** *method*
  (`patch.rs:268`), which is **unchanged**.
- **`utils::diff_lines_inline` signature change.** No impact — not called.
- **`TextDiff::iter_changes` now panics on out-of-bounds `DiffOp` ranges instead
  of silently truncating.** No impact — the codebase calls `iter_all_changes()`
  (the all-changes convenience iterator), not `iter_changes(op)` with a
  hand-constructed op.

`TextDiff::from_lines`, `TextDiff::from_chars`, `TextDiff::ratio()`,
`iter_all_changes()`, `Change::tag()`, and `ChangeTag::{Insert,Delete,Equal}` are
all unchanged from 2.x to 3.x. The expected outcome is therefore a **clean bump
with no source edits**.

## Spec

1. **Bump the version constraint** — in the workspace-root `Cargo.toml`, line 29,
   change `similar = "2"` to `similar = "3"`. This is the only `Cargo.toml` edit;
   leave `executor/Cargo.toml`'s `similar.workspace = true` untouched (it inherits
   the workspace constraint).

2. **Update the lockfile** — run `cargo update -p similar`. Confirm `Cargo.lock`
   now pins `similar` at a `3.x` version (expected `3.1.1`). Commit the
   `Cargo.lock` change together with the `Cargo.toml` change.

3. **Build and react only to what the compiler flags.** Run `cargo build`. The
   expectation (per the analysis above) is that it succeeds with no source change.
   **If — and only if — the compiler flags an error in one of the three usage
   files**, fix that specific call site to the 3.x API and record the change in
   "Notes for review". Do **not** preemptively edit the source files; the bump
   itself is the change. If a break appears that you cannot resolve from this
   phase doc's changelog summary, **file a blocker** with the exact `cargo build`
   error rather than guessing.

## Acceptance criteria

- [ ] `Cargo.toml` (workspace root) line 29 reads `similar = "3"`.
- [ ] `Cargo.lock` pins `similar` at a `3.x` version (run
      `cargo tree -i similar` or `grep -A2 'name = "similar"' Cargo.lock` to
      confirm).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing suite — no new tests).
- [ ] No source file under `executor/src/` was edited **unless** the compiler
      required it; any such edit is recorded in "Notes for review".

## Test plan

No new tests. `similar` is a vendored leaf utility (STANDARDS §3.2 — vendored
wire/utility code is covered upstream); this phase adds no behavior. The existing
suite is the regression guard — in particular the diff-exercising tests already in
`executor/src/tools/patch.rs`, `executor/src/agent/outcome.rs`, and
`executor/src/agent/progress.rs`, which call the bumped crate's API directly.

## End-to-end verification

The shipped artifact is the rebuilt binary linking `similar` 3.x. Verify against
it directly:

1. After the bump, run `cargo test 2>&1 | tail -30` and confirm the diff-related
   tests pass — quote the pass/fail line in the completion Update Log.
2. Run `cargo tree -i similar` (or `grep -A2 'name = "similar"' Cargo.lock`) and
   quote the resolved `3.x` version in the completion Update Log, confirming the
   new major is actually linked (not a stale lock entry).

This is **not** an N/A case: the bump changes the linked dependency, and the
green diff-exercising suite against the new major is the real-artifact evidence.

## Authorizations

- [x] May edit `Cargo.toml` (workspace root): bump `similar` from `"2"` to `"3"`.
- [x] May change a dependency version (`similar` 2 → 3) and update `Cargo.lock`
      via `cargo update -p similar`.

This is **only** authorization to bump `similar`. No other dependency may be
added, removed, or bumped (a stray `cargo update` with no `-p` filter would churn
unrelated crates — use the package-scoped form).

## Out of scope

- Bumping any other dependency. Phases 06–09 bump `tree-sitter`/
  `tree-sitter-python`, `toml_edit`, `toml`, and `reqwest` respectively, each in
  its own review-gated phase. A bare `cargo update` (no `-p similar`) is a scope
  violation here.
- Editing `executor/Cargo.toml` (it inherits via `similar.workspace = true`).
- Refactoring or "improving" the diff call sites in `patch.rs` / `outcome.rs` /
  `progress.rs`. Touch them **only** if the compiler flags a 3.x break, and then
  only minimally.
- Adopting any new 3.x API (e.g. the new owned-input `from_*` overloads). The goal
  is the version bump with the smallest possible diff, not a feature adoption.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-30 13:47 (started)

Executor: local LLM. Starting phase 05: bump `similar` 2→3. Toolchain confirmed at rustc 1.95.0 (≥ 1.85 MSRV requirement met).

### Update — 2026-06-30 13:48 (complete)

**Summary:** Bumped `similar` from `"2"` to `"3"` in workspace `Cargo.toml` and updated `Cargo.lock` via `cargo update -p similar`. The bump was clean — no source edits required. All four gates passed.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(passed, no output)

cargo build 2>&1 | tail -20
(passed, no warnings)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(passed, no warnings)

cargo test 2>&1 | tail -30
test result: ok. 871 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

1. `cargo test 2>&1 | tail -30`: `test result: ok. 871 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out` — all diff-related tests pass against `similar` 3.x.
2. `cargo tree -i similar`: `similar v3.1.1` — confirmed 3.x is linked, not a stale lock entry.
3. Grep for pinned literal: `grep 'similar = "3"' Cargo.toml` confirms the version constraint landed correctly.

**Files changed:**
- `Cargo.toml` — bumped `similar` from `"2"` to `"3"` (line 29)
- `Cargo.lock` — updated `similar` pin from `2.7.0` to `3.1.1`
- `docs/dev/milestones/M25-polish-and-config/phase-05-similar-3.md` — status `todo` → `review`, Update Log entries added
- `docs/dev/milestones/M25-polish-and-config/README.md` — phase table row 05 status updated

**New tests:** none (existing suite is the regression guard per test plan).

**Commits:**
- pending — `chore: bump similar 2→3`

**Notes for review:** No source edits were required — the `similar` 3.0 breaking changes do not touch any API surface used by this codebase. Clean dependency bump.
