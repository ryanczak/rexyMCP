# Phase 02: read-only search tools — find_files + search

**Milestone:** M2 — Executor tools & security
**Status:** todo
**Depends on:** phase-01 (done)
**Estimated diff:** ~400 lines (two tool lifts + scope adaptation + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the two read-only discovery tools the executor needs to navigate a repo:
`find_files` (glob-based, gitignore-aware) and `search` (regex grep, gitignore-
aware). Both are near-direct lifts from Rexy with one adaptation — their search
root resolves through the phase-01 `Scope`, so they cannot read or enumerate
outside the target-repo root. After this phase the registry can dispatch
`find_files` and `search`, both confined.

`symbols` (tree-sitter) is **not** in this phase — see Out of scope.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (tools row).
- `docs/architecture.md` — "The executor turn cycle" step 5 (all filesystem
  access scoped to the target-repo root).

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the architecture references and the M2 README Notes (tools re-root to a
   configured root, not the process CWD).
3. Read this entire phase doc.
4. Confirm phase-01 is `done`; `executor::security::scope::Scope` and
   `executor::tools::registry` exist and the workspace builds clean.
5. **Read the Rexy source** (reference, not a dependency):
   - `rexy/src/tools/find_files.rs` — `FindFiles` (globset + `ignore::Walk`).
   - `rexy/src/tools/search.rs` — `Search` (`ignore::Walk` + `regex`). Both lift
     cleanly; neither references `context::*`. See Spec for the one adaptation.

## Current state

- After phase-01, `executor/src/tools/` has `registry.rs`, `read_file.rs`,
  `mod.rs`; `executor/src/security/scope.rs` has `Scope` (`resolve` confines to
  the target-repo root, defaulting relative paths under the root).
- `executor/Cargo.toml` has no `globset`, `ignore`, or `regex` yet (authorized
  below).
- `read_file` (phase-01) is the pattern to follow: a tool holds a `Scope`, its
  constructor takes a `Scope`, and `execute` resolves model-supplied paths
  through `self.scope.resolve(...)`, returning advisory errors on `ScopeError`.

## Spec

### 1. Dependencies

Add to `[workspace.dependencies]` and `executor/Cargo.toml` (authorized below),
matching the versions Rexy uses:

- `globset = "0.4"` (find_files glob matcher)
- `ignore = "0.4"` (gitignore-aware directory walk, the engine ripgrep uses)
- `regex = "1"` (search pattern)

### 2. find_files — `executor/src/tools/find_files.rs`

Lift `FindFiles` from `rexy/src/tools/find_files.rs`. Args: `pattern` (glob,
required), `path` (optional search root), `max_results` (optional, default 100).
Keep: empty-pattern guard, `max_results` cap, glob compile via
`globset::Glob`, gitignore-aware `ignore::Walk`, file:path output.

**Adaptation — scope the search root:**

- `FindFiles` holds a `Scope`; constructor `find_files(scope: Scope) -> Arc<dyn
  Tool>`.
- The search root is `self.scope.resolve(parsed.path.as_deref().unwrap_or("."))`
  — i.e. default to the scope root, and confine any supplied `path`. On
  `Err(ScopeError)` return an advisory `ToolResult { error: Some(<display>), .. }`.
  Do **not** read the process CWD or use `"."` against the ambient directory.
- Keep the not-a-directory / does-not-exist advisory guards (check the resolved
  path).
- **Symlink confinement:** do not enable `follow_links` on the `Walk` (the
  `ignore` default is off). Leaving it off means the walk descends only within
  the confined root and cannot enumerate through an in-root symlink that points
  outside. State this in the Update Log.

Update the `path` schema description from "Defaults to cwd." to "Defaults to the
project root. Confined to the project root."

### 3. search — `executor/src/tools/search.rs`

Lift `Search` from `rexy/src/tools/search.rs`. Args: `pattern` (regex, required),
`path` (optional, dir or file), `max_results` (optional, default 100),
`case_insensitive` (optional, default false). Keep: empty-pattern guard,
`max_results` cap, `regex::RegexBuilder` with the case flag, gitignore-aware
`ignore::Walk`, and the `file:line:col: content` match formatting.

**Adaptation — scope the search root** (identical pattern to §2):

- `Search` holds a `Scope`; constructor `search(scope: Scope) -> Arc<dyn Tool>`.
- Resolve `path` (default `"."`) through `self.scope.resolve(...)`; advisory
  error on `ScopeError`. `path` may be a file or a directory (preserve that).
- Same `follow_links`-off symlink confinement note.
- Update the `path` schema description to note confinement to the project root.

### 4. Wiring — `executor/src/tools/mod.rs`

Add `pub mod find_files;` and `pub mod search;` (and re-export the constructors
if `mod.rs` re-exports the others).

## Acceptance criteria

- [ ] `executor/src/tools/find_files.rs` and `tools/search.rs` exist and are
      declared in `tools/mod.rs`.
- [ ] `find_files(scope)` and `search(scope)` construct `Arc<dyn Tool>` holding
      the scope.
- [ ] `find_files` returns paths matching a glob under the scope root, respects
      `.gitignore`, and caps at `max_results`.
- [ ] `search` returns `file:line:col` matches for a regex under the scope root,
      honors `case_insensitive`, respects `.gitignore`, and caps at `max_results`.
- [ ] Both tools return an advisory error (no panic) when the `path` arg resolves
      outside the scope root (`ScopeError`), when the pattern is empty/invalid,
      and when `path` is missing / not the right type.
- [ ] Neither tool references `context::`, `path_resolve`, `std::env::current_dir`,
      and neither enables `follow_links` on the walk.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, `tempfile::TempDir` as the scope root; construct tools with
`Scope::new(dir.path())`. Pin behavior, not names/count. Lift Rexy's find_files
and search tests, adapting them to construct with a `Scope` and to assert against
the confined root.

find_files:
- finds files matching a glob (e.g. write `a.rs`, `b.txt`; `**/*.rs` returns only
  `a.rs`);
- respects `.gitignore` (a gitignored file is not returned);
- caps results at `max_results`;
- empty / invalid glob → advisory error;
- `path` resolving outside the root (e.g. `"../"`) → advisory error.

search:
- finds a regex match and reports `file:line:col`;
- `case_insensitive: true` matches mixed case;
- caps at `max_results`;
- empty / invalid regex → advisory error;
- `path` outside the root → advisory error.

## End-to-end verification

> Not applicable — phase ships two library tools exercised directly by their unit
> tests (the registry/loop that drives them lands in M4; the MCP `execute_phase`
> in M5). State this in the completion entry.

## Authorizations

- [x] **May add dependencies:** `globset = "0.4"`, `ignore = "0.4"`, `regex =
      "1"` (runtime). No others.
- [x] **May create** `executor/src/tools/find_files.rs` and `tools/search.rs`;
      **may modify** `tools/mod.rs` and the two `Cargo.toml` files for the deps.
- [ ] May **NOT** add `symbols` / tree-sitter, write tools, `bash`, or the
      router.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Out of scope

- **`symbols`** — Rexy's `src/tools/symbols.rs` is a 3-line stub (tree-sitter was
  never implemented), so there is nothing to lift; it is a net-new design surface
  + a heavy dependency. It gets **M2 phase-03** of its own.
- Write tools (`write_file`, `patch`) — M2 phase-04.
- `bash` + `bash_classify` — M2 phase-05.
- The 2-stage router — M2 phase-06.
- `follow_links` / symlink *following* — intentionally off; not a feature here.

## Update Log

<!-- entries appended below this line -->
