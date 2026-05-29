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
| 02 | read-only search tools: `find_files` + `search` ([phase-02-search-tools.md](phase-02-search-tools.md)) | done |
| 03 | `symbols` — tree-sitter symbol-definition search (Rust + Python) ([phase-03-symbols.md](phase-03-symbols.md)) | done |
| 04 | write tools: `write_file` + `patch` (scope-confined) ([phase-04-write-tools.md](phase-04-write-tools.md)) | done |
| 05 | `bash` tool + `bash_classify` (Block/Allow, cwd-pin, env-strip) ([phase-05-bash.md](phase-05-bash.md)) | done |
| 06 | 2-stage tool router primitive: `Category` + `categorize` + registry queries ([phase-06-router.md](phase-06-router.md)) | review |

Phase-06 is the **last phase in M2** — its approval closes the milestone (human
gate: retrospective + doc-folding).

**Deferred out of phase-06** (the README originally bundled these into "06"): the
actual 2-stage *presentation* (model picks a category → sees only its schemas) is
an M4 agent-loop behavior; **governor-aware schema ordering** is M4 (needs the
governor's scorer); the **`Plan` category** is omitted (rexyMCP has no
plan/memory executor tool — categories are Read | Write | Search | Run).

**Deferred out of phase-05** (the README originally bundled these into "05"): the
`bash_classify` confirm tier collapsed to two tiers (Block/Allow) since the
executor is headless; **capabilities** is a Rexy plugin-grant concept, not
applicable to rexyMCP and dropped; **audit / redact / injection** ride on the
session-log + telemetry store and land in **M4**, not here.

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
