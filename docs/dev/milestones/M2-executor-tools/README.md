# M2 — Executor tools & security

**Goal:** Give the executor a working tool set — the built-in file/search/run
tools, the registry, and the 2-stage router — with every filesystem and shell
operation confined to the configured **target-repo root** by a path-scope
security layer.

**Status:** in-progress

**Depends on:** M1 (done)

**Exit criteria:**
- A `ToolRegistry` dispatches the built-in tools by name; unknown names are
  advisory `ToolResult` failures, not Rust errors.
- Every file/shell tool resolves paths through a **scope** that confines them to
  the target-repo root; `..` traversal, absolute paths outside the root, and
  symlink escapes are refused (advisory), never executed.
- `bash` is gated by the block/confirm classifier; the full tool set
  (`read_file`, `write_file`, `patch`, `bash`, `search`, `find_files`,
  `symbols`) is registered and tested hermetically.
- The 2-stage router exposes tools by category so a weak model sees a small,
  relevant schema set.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (tools,
  registry/router, security rows).
- `docs/architecture.md` — "The executor turn cycle" steps 5–6 (dispatch through
  the governor → registry; scope confinement).

## Phases

Expanded on demand (WORKFLOW.md § Milestones), not all at once.

| #  | Phase                                                              | Status |
|----|-------------------------------------------------------------------|--------|
| 01 | tool trait + registry + **scope confinement** + `read_file` ([phase-01-registry-scope-read.md](phase-01-registry-scope-read.md)) | done |

Tentative remaining phases (draft when the prior one lands):

- **02** — read-only tools: `find_files`, `search` (the `ignore`/`globset`
  crates), `symbols` (tree-sitter), all scope-confined.
- **03** — write tools: `write_file`, `patch` (the primary edit primitive;
  search-replace with fuzzy fallback), scope-confined including non-existent
  leaf paths.
- **04** — `bash` tool + `bash_classify` (block/confirm lists) + the
  capabilities/audit layer.
- **05** — the 2-stage tool router (categories: Read | Write | Search | Run |
  Plan) + governor-aware schema ordering hook.

## Notes

**Scope is net-new, not a lift.** Rexy's `src/security/scope.rs` is a stub
(`// TODO: implement`), and `src/tools/path_resolve.rs` only does project-root
*discovery* — it does **not** enforce confinement (absolute paths pass through
unchanged; no `..`/symlink rejection). rexyMCP must implement the confinement
primitive from scratch; it is the security backbone of this milestone and is
specified prescriptively in phase-01.

**Tools re-root to a configured root, not the process CWD.** Rexy's tools resolve
paths against `std::env::current_dir()`. rexyMCP tools are constructed with the
target-repo root (the `repo_path` arg of `execute_phase`) and resolve through the
scope. Do not read the process CWD.

**Strip `context::*` integration when lifting tools.** Rexy's `read_file` writes
to `context::file_cache` and counts tokens via `context::tokens`. The context
module is M4; drop that integration when lifting (a `// TODO(M4)` is *not*
allowed — just omit it; re-add when M4 exists).
