# Phase 01: Tool trait + registry + scope confinement + read_file

**Milestone:** M2 — Executor tools & security
**Status:** done
**Depends on:** M1 (done)
**Estimated diff:** ~450 lines (registry lift + new scope layer + read_file + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Establish the executor's tool foundation: the `Tool` trait + `ToolResult` +
`ToolRegistry` (lifted from Rexy), a **path-scope confinement layer** that
confines every filesystem operation to the configured target-repo root (built
fresh — Rexy stubbed this), and the first tool, `read_file`, adapted to resolve
through the scope. After this phase a registry dispatches `read_file`, and a
read outside the target-repo root is refused as an advisory `ToolResult` error,
never executed.

This phase is the security backbone of M2. The remaining tools (M2 phases 02–05)
all depend on the `Tool` trait and the scope built here.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map: the tools
  and "Security: scope …" rows. Note the map says **Lift** for security, but see
  Current state — scope itself is net-new.
- `docs/architecture.md` — "The executor turn cycle" steps 5–6: dispatch through
  the registry; all filesystem/bash access scoped to the target-repo root.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the architecture references and the M2 README (esp. the Notes — scope is
   net-new, tools re-root to a configured root, strip `context::*`).
3. Read this entire phase doc.
4. Confirm M1 is `done` and the workspace builds clean.
5. **Read the Rexy source** (reference, not a dependency):
   - `rexy/src/tools/registry.rs` — the `Tool` trait, `ToolResult`,
     `ToolRegistry`. Lift near-verbatim.
   - `rexy/src/tools/read_file.rs` — the `read_file` tool. Lift the logic, but
     see Spec §4 for the two required adaptations (scope, no `context::*`).
   - `rexy/src/security/scope.rs` — **a stub** (`// TODO: implement`). There is
     nothing to lift; Spec §3 specifies the confinement layer to build.
   - `rexy/src/tools/path_resolve.rs` — project-root *discovery* (convenience),
     **not** confinement. Do not rely on it for security; it lets absolute paths
     pass through unchanged.

## Current state

- After M1, `executor/src/lib.rs` declares `pub mod ai;`, `pub mod config;`,
  `pub mod error;`, `pub mod health;`. There is no `tools` or `security` module.
- `executor/Cargo.toml` already has `async-trait`, `serde`, `serde_json`,
  `anyhow`, `tokio`, and `tempfile` (dev). No new dependencies are needed.

## Spec

### 1. The registry — `executor/src/tools/registry.rs`

Lift from `rexy/src/tools/registry.rs`:

```rust
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;   // OpenAI function-calling shape
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult>;
}
```

Plus `ToolRegistry` (`HashMap<String, Arc<dyn Tool>>`) with `new`, `register`,
`get`, `all`, and `dispatch(name, args)` — where an unknown name returns an
advisory `ToolResult { error: Some("unknown tool: …"), .. }`, **not** a Rust
error. Lift verbatim.

### 2. Module wiring

- `executor/src/tools/mod.rs` — declares `pub mod registry;` and `pub mod
  read_file;` (and re-exports `Tool`, `ToolResult`, `ToolRegistry`).
- `executor/src/security/mod.rs` — declares `pub mod scope;`.
- `executor/src/lib.rs` — add `pub mod tools;` and `pub mod security;`.

### 3. The scope confinement layer (NEW — prescriptive) — `executor/src/security/scope.rs`

This does not exist in Rexy; build it. It is the single chokepoint every
file/shell tool uses to turn a model-supplied path into a safe absolute path.

```rust
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Scope {
    root: PathBuf,   // canonicalized target-repo root
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScopeError {
    Escapes { requested: String },   // resolves outside the root
    BadRoot { reason: String },      // root missing / not canonicalizable
}
// impl std::fmt::Display + std::error::Error for ScopeError.
// Display(Escapes) → e.g. "path escapes the project root: <requested>"
```

Behavior contract:

- `Scope::new(root: &Path) -> Result<Scope, ScopeError>` — canonicalize `root`
  (must exist and be a directory); store it. Missing/non-dir → `BadRoot`.
- `resolve(&self, requested: &str) -> Result<PathBuf, ScopeError>`:
  1. Form a candidate: if `requested` is absolute, use it as-is; else join it
     onto `root`.
  2. Confine by canonical prefix. The candidate may not exist yet (write tools,
     later phases), so canonicalize the **nearest existing ancestor** and append
     the remaining (non-existent) components, then require the result's canonical
     form to be `root` or a descendant of `root`. Reject (`Escapes`) anything
     that resolves outside — this catches `..` traversal, absolute paths outside
     the root, **and symlinks whose canonical target leaves the root** (because
     canonicalization resolves the symlink before the prefix check).
  3. Return the confined absolute `PathBuf`.
- `root(&self) -> &Path` accessor.

Confinement must be by **canonicalized prefix**, not string prefix (a string
check is defeated by `..` and symlinks). Resolve symlinks via
`std::fs::canonicalize` on the existing portion.

### 4. read_file — `executor/src/tools/read_file.rs`

Lift `ReadFile` from `rexy/src/tools/read_file.rs` with **two required
adaptations**:

1. **Scope, not CWD.** `ReadFile` holds a `Scope` (e.g. `pub struct ReadFile {
   scope: Scope }`, constructed by `read_file(scope: Scope) -> Arc<dyn Tool>`).
   In `execute`, resolve the path with `self.scope.resolve(&parsed.path)`; on
   `Err(ScopeError)`, return an advisory `ToolResult { error: Some(<display>), ..
   }`. Do **not** call `std::env::current_dir()` or `path_resolve`.
2. **No `context::*`.** Drop the `context::file_cache` population and the
   `context::tokens` count entirely (that module arrives in M4). Keep the
   metadata object but compute `bytes`/`lines`/`lines_read` from the content
   directly; omit token counts.

Preserve the rest of the lifted behavior and its advisory failures: not-found,
path-is-directory, non-UTF-8, malformed args, line-range validation (start≥1,
start≤end, clamp end past EOF), and the `{path, bytes, lines, lines_read}`
metadata.

## Acceptance criteria

- [ ] `executor/src/tools/registry.rs`, `tools/read_file.rs`,
      `tools/mod.rs`, `security/scope.rs`, `security/mod.rs` exist; `pub mod
      tools;` and `pub mod security;` are in `lib.rs`.
- [ ] `ToolRegistry::dispatch` runs a registered tool and returns an advisory
      `ToolResult` (not `Err`) for an unknown name.
- [ ] `Scope::resolve` returns `Ok` for an in-root relative path and for an
      in-root absolute path.
- [ ] `Scope::resolve` returns `Err(ScopeError::Escapes)` for a `..` traversal
      that leaves the root, for an absolute path outside the root, and for a
      symlink whose target is outside the root.
- [ ] `read_file` reads an in-root file (whole + line range) and returns the
      `{path, bytes, lines, lines_read}` metadata, with **no** token count.
- [ ] `read_file` returns an advisory error (no panic) for: a path outside the
      root, a missing file, a directory, non-UTF-8 content, and malformed args.
- [ ] No reference to `context::`, `path_resolve`, or `std::env::current_dir` in
      the new tool code.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, `tempfile::TempDir` for the root. Pin behavior, not names/count.

Scope (`security/scope.rs`):
- in-root relative path resolves under the root;
- in-root absolute path resolves;
- `../escape` (and an absolute path outside) → `Err(Escapes)`;
- a symlink inside the root pointing outside → `Err(Escapes)` (create with
  `std::os::unix::fs::symlink`; gate the test `#[cfg(unix)]`);
- a not-yet-existing leaf under the root resolves `Ok` (confine via existing
  ancestor — needed by write tools later);
- `Scope::new` on a missing dir → `Err(BadRoot)`.

Registry (`tools/registry.rs`):
- `dispatch` of a registered tool returns its output;
- `dispatch` of an unknown name returns advisory `error`, not `Err`.

read_file (`tools/read_file.rs`): lift Rexy's read_file tests (whole file, line
range, clamp past EOF, not-found, directory, non-UTF-8, malformed args, start>end)
**minus** the two `context::file_cache` tests; **add** a confinement test: a path
resolving outside the root returns an advisory error. Construct `ReadFile` with a
`Scope` rooted at the `TempDir`.

## End-to-end verification

The phase ships library types + tools exercised directly by unit tests (the
registry + scope + read_file public APIs are the real artifacts; there is no
binary entrypoint or runtime caller yet — the agent loop that drives the registry
is M4, the MCP `execute_phase` is M5).

> Not applicable — phase ships no runtime-loadable artifact beyond the library
> APIs the unit tests exercise directly. State this in the completion entry.

## Authorizations

- [x] **May create** `executor/src/tools/` and `executor/src/security/` module
      trees.
- [x] **May modify** `executor/src/lib.rs` (add the two `pub mod` lines).
- [ ] May **NOT** add dependencies — `async-trait`/`serde_json`/`tempfile` cover
      it. (`#[cfg(unix)]` symlink test uses `std::os::unix`, no crate.)
- [ ] May **NOT** lift or reference `context::*`, `path_resolve`, or
      `std::env::current_dir`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`.

## Out of scope

- All other tools — `find_files`, `search`, `symbols` (M2 phase-02);
  `write_file`, `patch` (phase-03); `bash` + `bash_classify` (phase-04).
- The 2-stage router and tool categories — M2 phase-05.
- `capabilities`, `audit`, `injection`, `redact` — later (redact lands with the
  session log in M4).
- File cache / token counting (`context::*`) — M4.
- The agent loop that calls `registry.dispatch` in a turn — M4.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-05-28 (progress)

Lifted `Tool`, `ToolResult`, `ToolRegistry` from Rexy's `registry.rs` verbatim. Built `Scope` confinement layer from spec (canonicalized prefix check with nearest-existing-ancestor strategy for non-existent paths). Lifted `read_file` adapted to use `Scope` instead of CWD, dropped `context::*` integration entirely. Added tests for scope (7 tests), registry (2 tests), read_file (10 tests). All 62 tests pass.

### Update — 2026-05-28 (complete)

**Summary:** Lifted `Tool` trait + `ToolResult` + `ToolRegistry` from Rexy. Built `Scope` confinement layer (canonicalized prefix check, nearest-existing-ancestor for non-existent paths, symlink escape detection). Lifted `read_file` adapted to use `Scope` instead of CWD, dropped `context::*` integration. Added hermetic tests for all three components. No deviations from spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s

cargo clippy --all-targets --all-features -- -D warnings
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.27s

cargo test
running 62 tests
test result: ok. 62 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**
Not applicable — phase ships no runtime-loadable artifact beyond the library APIs the unit tests exercise directly.

**Files changed:**
- `executor/src/tools/registry.rs` — lifted Tool trait, ToolResult, ToolRegistry + 2 tests
- `executor/src/tools/read_file.rs` — lifted read_file adapted to Scope + 10 tests
- `executor/src/tools/mod.rs` — module declarations and re-exports
- `executor/src/security/scope.rs` — new scope confinement layer + 7 tests
- `executor/src/security/mod.rs` — module declarations and re-exports
- `executor/src/lib.rs` — added `pub mod tools;` and `pub mod security;`

**New tests:**
- `resolves_in_root_relative_path` in `executor/src/security/scope.rs`
- `resolves_in_root_absolute_path` in `executor/src/security/scope.rs`
- `rejects_dot_dot_escape` in `executor/src/security/scope.rs`
- `rejects_absolute_path_outside_root` in `executor/src/security/scope.rs`
- `rejects_symlink_escape` in `executor/src/security/scope.rs`
- `resolves_nonexistent_leaf_under_root` in `executor/src/security/scope.rs`
- `new_on_missing_dir_returns_bad_root` in `executor/src/security/scope.rs`
- `dispatch_of_registered_tool_returns_output` in `executor/src/tools/registry.rs`
- `dispatch_of_unknown_name_returns_advisory_error` in `executor/src/tools/registry.rs`
- `reads_whole_file` in `executor/src/tools/read_file.rs`
- `reports_file_not_found` in `executor/src/tools/read_file.rs`
- `reports_directory_as_advisory` in `executor/src/tools/read_file.rs`
- `reads_line_range` in `executor/src/tools/read_file.rs`
- `clamps_end_line_past_eof` in `executor/src/tools/read_file.rs`
- `rejects_start_greater_than_end` in `executor/src/tools/read_file.rs`
- `rejects_non_utf8` in `executor/src/tools/read_file.rs`
- `rejects_malformed_args` in `executor/src/tools/read_file.rs`
- `rejects_path_outside_root` in `executor/src/tools/read_file.rs`
- `no_token_count_in_metadata` in `executor/src/tools/read_file.rs`

**verification:** fmt OK · clippy OK · tests 62 passed · build OK

**Notes for review:** None.
