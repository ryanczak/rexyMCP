# Phase 04: write tools — write_file + patch

**Milestone:** M2 — Executor tools & security
**Status:** review
**Depends on:** phase-03 (done)
**Estimated diff:** ~480 lines (two tool lifts + scope adaptation + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the executor's two **write** tools: `write_file` (create-or-overwrite full
content) and `patch` (exact search-and-replace, the primary edit primitive, with
a fuzzy "closest window" hint when the search string isn't found). Both are
near-direct lifts from Rexy with the **same one adaptation as phase-02**: every
write path resolves through the phase-01 `Scope`, so a model can never create or
modify a file outside the target-repo root.

These are the first tools that **mutate** the filesystem, so scope confinement is
load-bearing, not advisory polish: a scope escape must return an advisory error
and **write nothing**.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (tools row;
  `write_file` + `patch` are **lifts**).
- `docs/architecture.md` — "The executor turn cycle" step 5 (all filesystem
  access scoped to the target-repo root) and step 6 (edit-class tools precede the
  verifier — relevant later, in M4; not this phase).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom — §2.1 (error model: model-visible
   outcomes go to the `ToolResult` surface, not `Result::Err`) is directly
   load-bearing here.
2. Read the architecture references and the M2 README Notes (tools re-root to a
   configured root, not the process CWD; strip `context::*`).
3. Read this entire phase doc.
4. Confirm phase-03 is `done`; `executor::security::scope::Scope`,
   `executor::tools::registry::{Tool, ToolResult}`, and the existing tools build
   clean.
5. Study `executor/src/tools/search.rs` and `find_files.rs` (phase-02) — they are
   the scope-adaptation pattern: hold a `Scope`, constructor takes a `Scope`,
   resolve the model-supplied `path` through `self.scope.resolve(...)`, advisory
   `ToolResult { error: Some(...) }` on `ScopeError`.
6. **Read the Rexy sources** (reference, not a dependency):
   - `rexy/src/tools/write_file.rs` — `WriteFile`.
   - `rexy/src/tools/patch.rs` — `Patch` + the `fuzzy_hint` helper (uses the
     `similar` crate). **Note the adaptations in the Spec — do not lift these two
     files verbatim.**

## Current state

- `executor/src/tools/` has `registry.rs`, `read_file.rs`, `find_files.rs`,
  `search.rs`, `symbols.rs`, `mod.rs`. No write tools yet.
- `executor/Cargo.toml` has `globset`, `ignore`, `regex`, `tree-sitter*` — but
  **no `similar`** (authorized below).
- `Scope::resolve(&str) -> Result<PathBuf, ScopeError>` confines to the root and
  **already handles non-existent leaf paths** (it canonicalizes the nearest
  existing ancestor, checks the prefix, then re-appends the remaining components —
  see `scope.rs::resolves_nonexistent_leaf_under_root`). So resolving a path for a
  *new* file under the root works and stays confined.
- rexyMCP has **no `path_resolve` module** (Rexy's CWD-discovery helper was
  intentionally not lifted — see M2 README Notes). Both Rexy tools `use super::
  path_resolve` and `std::env::current_dir()`; **both must be dropped** in favor
  of `self.scope.resolve(...)`.

## Spec

### 1. Dependencies

Add to `[workspace.dependencies]` and `executor/Cargo.toml` (authorized below),
matching the version Rexy uses:

- `similar = "2"` — text diff + fuzzy ratio for `patch` (unified diff on success,
  closest-window hint on a miss).

### 2. write_file — `executor/src/tools/write_file.rs` (new file)

Lift `WriteFile` from `rexy/src/tools/write_file.rs`. Args: `path` (required),
`content` (required). Keep: the parent-directory-must-exist guard, the
`created` / `overwritten` / `bytes_written` metadata, and the
`"wrote N bytes to <path>"` output.

**Adaptations:**

- `WriteFile` holds a `Scope`; constructor `write_file(scope: Scope) -> Arc<dyn
  Tool>` (Rexy's takes no args).
- Replace `std::env::current_dir()` + `path_resolve::resolve(...)` with
  `self.scope.resolve(&parsed.path)`. On `Err(ScopeError)` return an advisory
  `ToolResult { error: Some(<display>), .. }` and **write nothing**. Remove the
  `use super::path_resolve` import.
- Keep the parent-exists guard against the **resolved** path (a missing parent is
  an advisory error — `write_file` does *not* create parent directories).
- Update the `path` schema description from "Absolute or relative to cwd." to
  "Path to write, confined to the project root. Relative paths resolve under the
  project root."
- Wrap the `std::fs::write` failure as an advisory `ToolResult` error (Rexy
  already does this for `write_file`).

### 3. patch — `executor/src/tools/patch.rs` (new file)

Lift `Patch` + `fuzzy_hint` from `rexy/src/tools/patch.rs`. Args: `path`,
`old_str`, `new_str` (all required; `new_str` may be empty for deletion). Keep:
the empty-`old_str` guard, the `old_str == new_str` no-op guard, the
not-found / is-a-directory / not-valid-UTF-8 guards, the **0 / 1 / n match**
logic, the `fuzzy_hint` closest-window helper (`similar::TextDiff`), and the
unified-diff success output (`TextDiff::from_lines(...).unified_diff()`).

**Adaptations:**

- `Patch` holds a `Scope`; constructor `patch(scope: Scope) -> Arc<dyn Tool>`.
- Replace `std::env::current_dir()` + `path_resolve::resolve(...)` with
  `self.scope.resolve(&parsed.path)`; advisory on `ScopeError`, **modifying
  nothing**. Remove the `use super::path_resolve` import.
- **Drop the `TODO(read-before-edit)` comment block** entirely. It references a
  Rexy concept (`EditedFiles` registry, an "architecture.md §Read-before-edit
  invariant" section) that **does not exist in rexyMCP**. Do not port the TODO,
  do not invent the invariant — read-before-edit enforcement is out of scope (see
  Out of scope). A bare comment referencing a non-existent section is exactly the
  kind of rot STANDARDS §2.3 forbids.
- Update the `path` schema description ("Absolute or relative to cwd." →
  "Path to patch, confined to the project root.").
- Wrap the success-path `std::fs::write` failure as an advisory `ToolResult`
  error rather than bubbling it with `?` — keep model-facing file outcomes on the
  tool-result surface, consistent with `write_file` and STANDARDS §2.1. (Rexy's
  `patch` bubbles it with `?`; this is the one behavioral change from the lift.)
- Preserve the **write-nothing-unless-exactly-one-match** invariant: the file is
  written only in the `match_count == 1` branch. 0 matches → fuzzy hint, no
  write; n matches → ambiguous advisory, no write.

### 4. Wiring — `executor/src/tools/mod.rs`

Add `mod write_file;` and `mod patch;`, and re-export the constructors + structs
(`pub use write_file::{WriteFile, write_file};`, `pub use patch::{Patch,
patch};`), mirroring the existing tools.

## Acceptance criteria

- [ ] `executor/src/tools/write_file.rs` and `tools/patch.rs` exist and are
      declared + re-exported in `tools/mod.rs`.
- [ ] `write_file(scope)` and `patch(scope)` construct `Arc<dyn Tool>` holding the
      scope.
- [ ] `write_file` creates a new file under the scope root and overwrites an
      existing one, reporting `created` / `overwritten` / `bytes_written`; a
      missing parent directory is an advisory error and **no file is created**.
- [ ] `patch` replaces an exact single-match `old_str`, writes the file, and
      returns output containing a unified diff and `(1 hunk)`; 0 matches returns
      an advisory error containing a "closest window" hint and **does not modify
      the file**; >1 match returns an advisory "disambiguate" error and **does not
      modify the file**.
- [ ] `patch` advisory (no panic, no write) on: empty `old_str`, `old_str ==
      new_str` (no-op), file not found, path is a directory, file not valid UTF-8.
- [ ] **Scope confinement (both tools):** a `path` resolving outside the scope
      root (e.g. `"../outside"`) returns an advisory `ScopeError` and **writes /
      modifies nothing on disk**. There is a test asserting the on-disk state is
      unchanged.
- [ ] Neither tool references `path_resolve`, `std::env::current_dir`, or
      `context::`; neither leaves a `TODO` / `FIXME` or a debug `eprintln!` /
      `println!`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, `tempfile::TempDir` as the scope root; construct tools with
`Scope::new(dir.path())`. Lift Rexy's `write_file` and `patch` tests, adapting
them to construct with a `Scope` and to use paths under the confined root. Pin
behavior, not test count.

**When lifting the patch tests, drop the two `eprintln!("DEBUG diff output: …")`
lines** in Rexy's `success_output_contains_unified_diff` test — they are leftover
debug prints and violate STANDARDS / the hard rules.

write_file:
- creates a new file (content on disk matches; `created` true, `overwritten`
  false);
- overwrites an existing file (`overwritten` true);
- missing parent directory → advisory error, file not created;
- malformed args → advisory error.

patch:
- exact single match → patched, output has the unified diff + `(1 hunk)`, file
  content updated;
- 0 matches → advisory with "0 matches" + "Closest window at" hint, file
  unchanged;
- >1 match → advisory "disambiguate", file unchanged;
- empty `old_str`, no-op (`old_str == new_str`), missing file, directory path,
  non-UTF-8 file → advisory (each), file unchanged where one exists.

Scope confinement (both tools — **new**, not in the Rexy tests):
- `write_file` with `path: "../outside.txt"` → advisory `ScopeError`; assert no
  file appears outside the root;
- `patch` with `path: "../outside.txt"` → advisory `ScopeError`; if you stage a
  file outside the root first, assert it is **unchanged**.

## End-to-end verification

> Not applicable — this phase ships two library tools exercised directly by their
> unit tests. The registry/loop that drives them (and the verifier that runs
> after edit-class tools) lands in M4; the MCP `execute_phase` in M5. Restate this
> in the completion entry.

## Authorizations

- [x] **May add dependencies:** `similar = "2"` (runtime). No others.
- [x] **May create** `executor/src/tools/write_file.rs` and `tools/patch.rs`;
      **may modify** `tools/mod.rs` and the two `Cargo.toml` files for the dep.
- [ ] May **NOT** add `bash`, the router, or any read-before-edit / session-state
      machinery.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      or any other phase doc.

## Out of scope

- **Read-before-edit enforcement** (an `EditedFiles` / mtime registry that
  refuses a `patch` to a file the executor hasn't read, or that changed under it).
  rexyMCP has no such mechanism yet and the architecture doc does not specify one;
  it belongs to the M4 agent loop / session-state work if adopted at all. Drop
  Rexy's TODO; do not stub the registry here.
- **`write_file` creating parent directories** (`mkdir -p`). A missing parent is
  an advisory error, matching Rexy — keep it explicit.
- **`bash` + `bash_classify`** (M2 phase-05).
- **The 2-stage router** (M2 phase-06).
- **The verifier / post-edit typecheck** (M4) — edit-class tools just write here.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 12:00 (progress)

Lifted `WriteFile` and `Patch` from Rexy. Adapted both to hold a `Scope` and resolve
paths through `self.scope.resolve()` instead of `current_dir()` + `path_resolve`.
Dropped the `TODO(read-before-edit)` block from patch. Wrapped `fs::write` failure
in patch as advisory `ToolResult` error (Rexy bubbles with `?`). Added `similar = "2"`
dependency. All lifted tests adapted to construct with `Scope`; dropped `eprintln!`
debug lines from Rexy's patch test. Added scope-confinement tests for both tools.

### Update — 2026-05-28 12:30 (complete)

**Summary:** Both `write_file` and `patch` tools implemented, tested, and wired into
the registry. All Rexy tests lifted and adapted; scope-confinement tests added. No
deviations from spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling similar v2.7.0
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.84s

cargo clippy --all-targets --all-features -- -D warnings
    Checking similar v2.7.0
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.93s

cargo test
running 120 tests
test result: ok. 120 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — this phase ships two library tools exercised directly by their
unit tests. The registry/loop that drives them (and the verifier that runs
after edit-class tools) lands in M4; the MCP `execute_phase` in M5.

**Files changed:**
- `Cargo.toml` — added `similar = "2"` to workspace dependencies
- `executor/Cargo.toml` — added `similar.workspace = true`
- `executor/src/tools/write_file.rs` — new file; lifted WriteFile with Scope adaptation
- `executor/src/tools/patch.rs` — new file; lifted Patch + fuzzy_hint with Scope adaptation
- `executor/src/tools/mod.rs` — added mod declarations and re-exports for write_file and patch

**New tests:**
- `write_file::tests::creates_new_file` — asserts file created with correct metadata
- `write_file::tests::overwrites_existing_file` — asserts overwritten flag set
- `write_file::tests::reports_missing_parent_dir` — advisory error, no file created
- `write_file::tests::rejects_malformed_args` — advisory error on bad JSON
- `write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing` — scope confinement
- `patch::tests::patches_exact_single_match` — patched file + unified diff output
- `patch::tests::success_output_contains_unified_diff` — diff contains -/+ lines
- `patch::tests::preserves_surrounding_content` — surrounding lines unchanged
- `patch::tests::reports_zero_matches_with_fuzzy_hint` — 0 matches + Closest window hint
- `patch::tests::reports_ambiguous_multiple_matches` — n matches + disambiguate
- `patch::tests::rejects_empty_old_str` — advisory error
- `patch::tests::rejects_identical_old_and_new` — no-op advisory
- `patch::tests::rejects_missing_file` — file not found advisory
- `patch::tests::rejects_directory_as_path` — directory advisory
- `patch::tests::rejects_non_utf8_file` — UTF-8 advisory
- `patch::tests::rejects_malformed_args` — bad JSON advisory
- `patch::tests::does_not_modify_file_on_zero_match` — file unchanged on miss
- `patch::tests::scope_escape_returns_advisory_error_and_modifies_nothing` — scope confinement

**Commits:**
- (pending)

**Notes for review:** None.

**Grep verification — no forbidden references:**
```
grep -r "path_resolve" executor/src/tools/write_file.rs executor/src/tools/patch.rs
grep -r "current_dir" executor/src/tools/write_file.rs executor/src/tools/patch.rs
grep -r "context::" executor/src/tools/write_file.rs executor/src/tools/patch.rs
```
All three return zero matches — no forbidden references present.

### Update — 2026-05-28 13:00 (progress)

Fixed bug-04-1: rewrote both scope-escape tests to nest the scope root inside the
TempDir (`temp.path().join("root")`), so `"../outside.txt"` resolves to a sibling
directory still inside the sandbox. No production-code changes.

### Update — 2026-05-28 13:15 (complete)

**Summary:** Fixed bug-04-1 — both scope-escape tests now use a nested scope root
inside the TempDir, keeping all filesystem state hermetic. No production-code changes.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s

cargo clippy --all-targets --all-features -- -D warnings
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.41s

cargo test
running 120 tests
test result: ok. 120 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — this phase ships two library tools exercised directly by their
unit tests. The registry/loop that drives them (and the verifier that runs
after edit-class tools) lands in M4; the MCP `execute_phase` in M5.

**Files changed:**
- `executor/src/tools/write_file.rs` — fixed scope-escape test to use nested scope root
- `executor/src/tools/patch.rs` — fixed scope-escape test to use nested scope root
- `docs/dev/milestones/M2-executor-tools/bugs/bug-04-1.md` — status: open → fixed
- `docs/dev/milestones/M2-executor-tools/phase-04-write-tools.md` — status + Update Log

**New tests:** None (existing tests fixed).

**Commits:**
- (pending)

**Notes for review:** None.

**Grep verification — no `.parent()` in test code:**
```
grep -n "\.parent()" executor/src/tools/write_file.rs executor/src/tools/patch.rs
```
Only match is line 75 of write_file.rs (production parent-dir guard) — no test code references `.parent()`.
