# Phase 06: `tree-sitter` 0.25→0.26 + `tree-sitter-python` 0.23→0.25

**Milestone:** M25 — Polish & Config Pass
**Status:** done
**Depends on:** none (phase-05 already landed the `similar` bump; this is independent)
**Estimated diff:** ~3 lines (two `Cargo.toml` constraints + the `Cargo.lock` churn)
**Tags:** language=rust, kind=refactor, size=s

## Goal

Bump the tree-sitter core crate from the `0.25` minor to `0.26` (latest `0.26.10`)
**and** the `tree-sitter-python` grammar from `0.23` to `0.25` (latest `0.25.0`).
These two move together: the `symbols` tool parses both Rust and Python, and the
Python grammar's published line caught up to a newer minor while the core moved
to 0.26. The grammar and core crates communicate through the stable
`tree-sitter-language` ABI crate (`0.1.x`), and the 0.26 core's public Rust API
that `symbols.rs` actually calls is **unchanged** from 0.25 — so the expected
diff is the two version constraints plus the `Cargo.lock` update, with **no
source edit**.

`tree-sitter-rust` stays at `0.24` (its line has no newer published minor, and
0.24 remains ABI-compatible with the 0.26 core — see the analysis below). This is
the second of the five M25 dependency major/minor bumps.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #25 — names M25 as the polish & config pass; the
  dep bumps are committed scope.
- This milestone's [README](README.md) § "Dependency update phases (05–09)" — the
  per-phase ordering and the uniform recipe (bump constraint → `cargo update` →
  fix compiler-flagged breaks → verify four gates). Phase-05 (`similar` 2→3,
  `done`) is the immediately preceding worked example of this same recipe.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Confirm the toolchain meets the 0.26 core's MSRV.** tree-sitter `0.26.x`
   builds on stable Rust well below this workspace's edition-2024 floor (the
   workspace is already `edition = "2024"`, which itself requires ≥ 1.85; the
   installed toolchain at draft time was `rustc 1.95.0`). Confirm with
   `rustc --version` (expect ≥ 1.85). If it is somehow below 1.85, **stop and file
   a blocker** — do not attempt a toolchain upgrade.

## Current state

All three tree-sitter crates are **workspace dependencies**, declared once at the
workspace root and inherited by the executor crate:

`Cargo.toml` (workspace root), lines 26–28:

```toml
tree-sitter = "0.25"
tree-sitter-rust = "0.24"
tree-sitter-python = "0.23"
```

`executor/Cargo.toml`, lines 22–24:

```toml
tree-sitter.workspace = true
tree-sitter-rust.workspace = true
tree-sitter-python.workspace = true
```

`Cargo.lock` currently pins (verified at draft time):

| Crate | Current pin | After this phase |
|---|---|---|
| `tree-sitter` | `0.25.10` | `0.26.x` (expected `0.26.10`) |
| `tree-sitter-python` | `0.23.6` | `0.25.0` |
| `tree-sitter-rust` | `0.24.2` | `0.24.2` (**unchanged**) |
| `tree-sitter-language` | `0.1.7` | `0.1.x` (unchanged major; the shared ABI crate) |

**The crates are used in exactly one executor source file**,
`executor/src/tools/symbols.rs`, via a small, stable API surface. The complete
set of API items the file touches (quoted so you can confirm none changed):

- `symbols.rs:14` — `use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};`
- `symbols.rs:446-447` and `496-497` — `tree_sitter_rust::LANGUAGE.into()` /
  `tree_sitter_python::LANGUAGE.into()`, each coerced to a `tree_sitter::Language`:
  ```rust
  let (language, query_str) = match lang {
      Language::Rust => (tree_sitter_rust::LANGUAGE.into(), RUST_QUERY),
      Language::Python => (tree_sitter_python::LANGUAGE.into(), PYTHON_QUERY),
  };
  let language: tree_sitter::Language = language;
  ```
- `symbols.rs:451` / `501` — `Query::new(&language, query_str)?`
- `symbols.rs:453-454` / `503-504` — `Parser::new()` + `parser.set_language(&language)?`
- `symbols.rs:456` / `506` — `parser.parse(source.as_bytes(), None)`
- `symbols.rs:460` — `query.capture_names()`
- `symbols.rs:461-462` / `510-511` — `QueryCursor::new()` +
  `cursor.matches(&query, tree.root_node(), source.as_bytes())`
- `symbols.rs:465` / `514` — `it.next()` (the `StreamingIterator` method)
- `symbols.rs:467` — `cap.index` (a `u32` capture index)
- `symbols.rs:469` / `516` — `cap.node.byte_range()`
- `symbols.rs:480` / `522` — `cap.node.start_position()` → `.row` / `.column`

## What changed in `tree-sitter` 0.26 / `tree-sitter-python` 0.25 (and why none of it affects us)

Verified against the upstream releases and the published crate sources at draft
time:

- **The shared `LanguageFn` ABI crate is unchanged across all three crates.**
  `tree-sitter` `0.26.x`, `tree-sitter-rust` `0.24`, and `tree-sitter-python`
  `0.25.0` all depend on `tree-sitter-language` `^0.1`. Because the `LanguageFn`
  type comes from that one shared crate, the three resolve to a **single**
  `LanguageFn` type, so `tree_sitter_rust::LANGUAGE.into()` and
  `tree_sitter_python::LANGUAGE.into()` continue to coerce into
  `tree_sitter::Language` exactly as today. (This is the one thing a tree-sitter
  bump most often breaks — a core/grammar `tree-sitter-language` major skew that
  splits `LanguageFn` into two incompatible types and makes `.into()` stop
  resolving. It does **not** happen here: the ABI crate stays at `0.1.x`.)

- **`tree-sitter-python` 0.25 keeps the `LANGUAGE` constant.** Its
  `bindings/rust/lib.rs` still declares
  `pub const LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_python) };`
  — same name, same type as 0.23. No call-site rename needed.

- **`QueryCursor::matches` signature is unchanged.** In 0.26 it is still
  `matches<T: TextProvider<I>, I: AsRef<[u8]>>(&mut self, &Query, Node, T)`
  returning a `QueryMatches` that requires the `StreamingIterator` trait in scope
  to iterate. `source.as_bytes()` is a `&[u8]`, which satisfies `TextProvider` /
  `AsRef<[u8]>` just as in 0.25. The `use … StreamingIterator` import at
  `symbols.rs:14` is still required and still correct.

- **`Query::new`, `Parser::new`/`set_language`/`parse`, `capture_names`,
  `Node::byte_range`/`start_position`, `QueryCursor::new` are unchanged.** The
  0.26 release line (PRs rolled up under the `v0.26.1` notes) consists of wasm,
  CLI, build, and internal-fix changes plus an MSRV *decrease* for the
  `tree-sitter-language` crate — **no breaking change to the Rust crate's
  `Parser`/`Query`/`QueryCursor`/`Node` public API** that this file calls.

- **Grammar ABI compatibility.** `set_language` returns a `LanguageError` (which
  `symbols.rs` propagates with `?`) if a grammar's generated ABI is too old for
  the core. `tree-sitter-rust` `0.24.2` and `tree-sitter-python` `0.25.0` are both
  recent, modern-ABI grammars that the `0.26` core loads — so this stays a
  compile-and-pass, not a runtime `LanguageError`. The existing `symbols` tests
  (which parse real Rust and Python) are the regression guard that proves the
  grammars actually load.

The expected outcome is therefore a **clean bump with no source edits**.

## Spec

1. **Bump the two version constraints** — in the workspace-root `Cargo.toml`,
   change line 26 `tree-sitter = "0.25"` to `tree-sitter = "0.26"`, and line 28
   `tree-sitter-python = "0.23"` to `tree-sitter-python = "0.25"`. **Leave line 27
   `tree-sitter-rust = "0.24"` untouched** (no newer published minor; it stays
   ABI-compatible with the 0.26 core). Leave `executor/Cargo.toml`'s three
   `*.workspace = true` lines untouched (they inherit the workspace constraints).

2. **Update the lockfile** — run the **package-scoped** update:
   `cargo update -p tree-sitter -p tree-sitter-python`. Confirm `Cargo.lock` now
   pins `tree-sitter` at a `0.26.x` version (expected `0.26.10`) and
   `tree-sitter-python` at `0.25.0`. A bare `cargo update` (no `-p` filter) would
   churn unrelated crates and is a scope violation — use the package-scoped form.
   Commit the `Cargo.lock` change together with the `Cargo.toml` change.

3. **Build and react only to what the compiler flags.** Run `cargo build`. The
   expectation (per the analysis above) is that it succeeds with no source change.
   **If — and only if — the compiler flags an error in `symbols.rs`**, fix that
   specific call site to the 0.26 API and record the change in "Notes for review".
   Do **not** preemptively edit `symbols.rs`; the bump itself is the change. If a
   break appears that you cannot resolve from this phase doc's analysis above —
   in particular an `.into()` trait-resolution error (the `LanguageFn` skew
   symptom) — **file a blocker** with the exact `cargo build` error rather than
   guessing or hand-rolling a `Language` conversion.

## Acceptance criteria

- [ ] `Cargo.toml` (workspace root) line 26 reads `tree-sitter = "0.26"` and line
      28 reads `tree-sitter-python = "0.25"`; line 27 still reads
      `tree-sitter-rust = "0.24"` (unchanged).
- [ ] `Cargo.lock` pins `tree-sitter` at a `0.26.x` version and
      `tree-sitter-python` at `0.25.0` (run `cargo tree -i tree-sitter` and
      `cargo tree -i tree-sitter-python`, or
      `grep -A2 'name = "tree-sitter"' Cargo.lock`, to confirm).
- [ ] `Cargo.lock` still pins `tree-sitter-rust` at `0.24.x` (it was not bumped).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing suite — no new tests).
- [ ] `executor/src/tools/symbols.rs` was **not** edited unless the compiler
      required it; any such edit is recorded in "Notes for review".

## Test plan

No new tests. The tree-sitter crates are vendored parser/grammar utilities
(STANDARDS §3.2 — vendored wire/utility code is covered upstream); this phase adds
no behavior. The existing `symbols` test module at the bottom of
`executor/src/tools/symbols.rs` is the regression guard — it parses real Rust and
Python (`finds_rust_function_by_name`, `finds_python_function_and_class`,
`references_python_identifier`, etc.), exercising the bumped crates' `LANGUAGE`
constants, `Query::new`, `set_language`, `parse`, and `matches` against the new
versions.

## End-to-end verification

The shipped artifact is the rebuilt binary linking `tree-sitter` 0.26 and
`tree-sitter-python` 0.25. Verify against it directly:

1. After the bump, run `cargo test symbols 2>&1 | tail -30` and confirm the
   `symbols` Rust- and Python-parsing tests pass — quote the pass/fail line in the
   completion Update Log.
2. Run `cargo tree -i tree-sitter` and `cargo tree -i tree-sitter-python` (or
   `grep -A2 'name = "tree-sitter"' Cargo.lock`) and quote the resolved
   `0.26.x` / `0.25.0` versions in the completion Update Log, confirming the new
   versions are actually linked (not a stale lock entry), and that
   `tree-sitter-rust` is still `0.24.x`.

This is **not** an N/A case: the bump changes the linked dependencies, and the
green Python+Rust symbol-parsing suite against the new versions is the
real-artifact evidence that the grammars load and the API still binds.

## Authorizations

- [x] May edit `Cargo.toml` (workspace root): bump `tree-sitter` `"0.25"` →
      `"0.26"` and `tree-sitter-python` `"0.23"` → `"0.25"`.
- [x] May change those two dependency versions and update `Cargo.lock` via
      `cargo update -p tree-sitter -p tree-sitter-python`.

This is **only** authorization to bump `tree-sitter` and `tree-sitter-python`. No
other dependency may be added, removed, or bumped — including `tree-sitter-rust`
(it stays at `0.24`) and `tree-sitter-language` (it stays at its current `0.1.x`,
moved only transitively if at all). A stray `cargo update` with no `-p` filter
would churn unrelated crates — use the package-scoped form.

## Out of scope

- Bumping any other dependency. Phases 07–09 bump `toml_edit`, `toml`, and
  `reqwest` respectively, each in its own review-gated phase. A bare `cargo update`
  (no `-p` filter) is a scope violation here.
- Bumping `tree-sitter-rust` (stays `0.24`) or jumping `tree-sitter` to `0.27`.
  The committed M25 scope (decided with the user) is `tree-sitter` → `0.26`; a
  newer `0.27` exists upstream but is **out of scope** for this phase. Do not
  reach past `0.26`.
- Editing `executor/Cargo.toml` (it inherits via the three `*.workspace = true`
  lines).
- Refactoring or "improving" the parser call sites in `symbols.rs`. Touch them
  **only** if the compiler flags a 0.26 break, and then only minimally.
- Adopting any new 0.26 API (e.g. the new `QueryCursor` containing-range methods).
  The goal is the version bump with the smallest possible diff, not a feature
  adoption.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-30 14:05 (started)

**Executor:** rexyMCP executor
**Action:** Bumping `tree-sitter` 0.25→0.26 and `tree-sitter-python` 0.23→0.25 per M25 spec.

### Update — 2026-06-30 14:05 (complete)

**Executor:** rexyMCP executor
**Action:** Completed tree-sitter 0.25→0.26 + tree-sitter-python 0.23→0.25 bump.

**Commands run:**
- `cargo build` — succeeded, zero new warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed
- `cargo fmt --all --check` — passed
- `cargo test` — 871 passed, 0 failed, 2 ignored
- `cargo test symbols` — 31 passed, 0 failed

**End-to-end verification:**
- `cargo tree -i tree-sitter` → `tree-sitter v0.26.10` (linked via rexymcp-executor)
- `cargo tree -i tree-sitter-python` → `tree-sitter-python v0.25.0` (linked via rexymcp-executor)
- `grep -A2 'name = "tree-sitter-rust"' Cargo.lock` → `version = "0.24.2"` (unchanged)

**Files changed:**
- `Cargo.toml` — bumped `tree-sitter = "0.26"` and `tree-sitter-python = "0.25"` (2 lines)
- `Cargo.lock` — updated via `cargo update -p tree-sitter -p tree-sitter-python`

**Notes for review:** No source edits were required. `executor/src/tools/symbols.rs` was not touched. The 0.26 core API is fully backward-compatible with the call sites in `symbols.rs`.

**Grep verification:**
```
$ grep 'tree-sitter = "0.26"' Cargo.toml
tree-sitter = "0.26"
$ grep 'tree-sitter-python = "0.25"' Cargo.toml
tree-sitter-python = "0.25"
$ grep 'tree-sitter-rust = "0.24"' Cargo.toml
tree-sitter-rust = "0.24"
```

### Review verdict — 2026-06-30

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-PrismaAURA
- **Scope deviations:** none
- **Calibration:** none

Independent re-run confirmed all four gates green: `cargo fmt --all --check`
clean, `cargo build` zero warnings, `cargo clippy --all-targets --all-features
-- -D warnings` clean, `cargo test` 871 executor + 429 mcp passed (2 ignored).
`cargo test symbols` → 31 passed (real Rust+Python parse against the bumped
crates — the regression guard). `Cargo.lock` pins `tree-sitter v0.26.10`,
`tree-sitter-python v0.25.0`, `tree-sitter-rust v0.24.2` (unchanged),
`tree-sitter-language v0.1.7` (unchanged). No `.rs` source edited — the commit
touches only `Cargo.toml`, `Cargo.lock`, and docs. Smallest-possible-diff bump
as specified.
