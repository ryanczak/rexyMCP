# Phase 06: 2-stage tool router — category primitive + registry queries

**Milestone:** M2 — Executor tools & security
**Status:** todo
**Depends on:** phase-05 (done)
**Estimated diff:** ~190 lines (router lift + registry category methods + tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Add the **routing primitive** that lets the agent loop show a weak model a small,
relevant slice of the tool schemas instead of all of them at once: a `Category`
enum + a `categorize(tool_name)` function, plus `ToolRegistry` methods to list the
categories present and the tools within one.

This is the **last phase in M2.** After it is approved, the milestone closes —
which is a human gate (retrospective + doc-folding), not an automatic advance.

## Scope decisions (read this)

The M2 README tentatively described phase-06 as "the 2-stage tool router
(categories: Read | Write | Search | Run | Plan) + governor-aware schema ordering
hook." Two parts of that don't belong in M2; this phase narrows the description
(README updated to match):

1. **The actual 2-stage *presentation* is M4, not here.** "Model picks a category,
   then receives only that category's schemas" is an agent-loop behavior. M2 has
   no loop. Phase-06 ships the **primitive** (categorize + registry grouping) that
   the M4 loop will consume; it does not present categories to a model.
2. **No `Plan` category.** Rexy maps `plan` / `memory_*` tools to `Plan`, but
   rexyMCP has none of those executor tools (the local planner is dropped per
   architecture.md § Non-goals; no plan/memory executor tool is on the roadmap).
   Shipping an empty, unconstructed `Plan` variant is speculative (same reasoning
   that dropped the classifier's `Warn` tier in phase-05). The enum covers the
   four categories that have tools: **Read | Write | Search | Run**. A later phase
   that introduces a plan/memory tool adds the variant then.
3. **Governor-aware schema ordering is M4.** The governor (per-tool success
   scorer) lands in M4; ordering tools within a category by its scores is wired
   there, against the real scorer. No ordering hook here — phase-06 returns tools
   in a deterministic, stable order.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map ("Tool
  registry + 2-stage router | Lift").
- `docs/architecture.md` — M2 exit criteria: "The 2-stage router exposes tools by
  category so a weak model sees a small, relevant schema set."

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the architecture references and the M2 README.
3. Read this entire phase doc.
4. Confirm phase-05 is `done`; the registry (`executor/src/tools/registry.rs`,
   `ToolRegistry` with `register` / `get` / `all` / `dispatch`) and all seven
   built-in tools (`read_file`, `symbols`, `search`, `find_files`, `write_file`,
   `patch`, `bash`) build clean.
5. **Read `rexy/src/tools/router.rs`** (reference) — the `Category` enum +
   `categorize` are a near-direct lift, minus `Plan`, with the tool-name list
   matched to rexyMCP's actual tools.

## Current state

- `executor/src/tools/registry.rs` — `ToolRegistry { tools: HashMap<String, Arc<dyn
  Tool>> }` with `new` / `register` / `get(name)` / `all() -> impl Iterator<Item =
  &Arc<dyn Tool>>` / `dispatch(name, args)`. The `Tool` trait exposes `name()`,
  `description()`, `schema()`, `execute()`.
- `executor/src/tools/mod.rs` declares the seven tools + `registry`, and re-exports
  `Tool`, `ToolRegistry`, `ToolResult` and each tool's constructor.
- No `router` module yet.

## Spec

### 1. router — `executor/src/tools/router.rs` (new file)

Lift `Category` + `categorize` from `rexy/src/tools/router.rs`, adapted:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Read,
    Write,
    Search,
    Run,
}

/// Map a built-in tool name to its router category. `None` for an unknown name.
pub fn categorize(tool_name: &str) -> Option<Category> {
    Some(match tool_name {
        "read_file" | "symbols" => Category::Read,
        "write_file" | "patch" => Category::Write,
        "search" | "find_files" => Category::Search,
        "bash" => Category::Run,
        _ => return None,
    })
}
```

- No `Plan` variant; no `plan`/`memory_*` names (see Scope decisions).
- **Every built-in tool must categorize to `Some`** — an uncategorized tool is
  invisible to the 2-stage router. There is an acceptance test for this.

### 2. registry category queries — `executor/src/tools/registry.rs`

Add two methods to `ToolRegistry` (use `router::categorize`):

- `pub fn categories(&self) -> Vec<Category>` — the distinct categories present
  among the registered tools, in a **stable, deterministic order** (the enum's
  declaration order: Read, Write, Search, Run — not `HashMap` iteration order).
- `pub fn tools_in(&self, category: Category) -> Vec<Arc<dyn Tool>>` — the
  registered tools whose name categorizes to `category`, sorted by tool name for
  determinism. Returns an empty `Vec` for a category with no registered tools.

These are what the M4 loop calls to present one category's schemas at a time. Keep
them pure queries over the existing `tools` map; do not change `register` /
`dispatch` / the `Tool` trait.

### 3. Wiring — `executor/src/tools/mod.rs`

Add `mod router;` and `pub use router::{Category, categorize};`.

## Acceptance criteria

- [ ] `executor/src/tools/router.rs` exists with `Category { Read, Write, Search,
      Run }` + `categorize(&str) -> Option<Category>`, declared + re-exported in
      `tools/mod.rs`.
- [ ] `categorize` returns the documented category for each of the seven built-in
      tool names and `None` for an unknown name.
- [ ] **Every built-in tool name** (`read_file`, `symbols`, `search`,
      `find_files`, `write_file`, `patch`, `bash`) categorizes to `Some(_)` — a
      test iterates the names and asserts none is `None`.
- [ ] `ToolRegistry::categories()` returns the present categories in stable
      enum-declaration order (not HashMap order), with no duplicates.
- [ ] `ToolRegistry::tools_in(c)` returns exactly the registered tools in category
      `c`, name-sorted, and an empty `Vec` for a category with none registered.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic. For registry tests, register the real tools (construct with a
`Scope::new(TempDir)` and a bash timeout) or small fake `Tool`s named after the
built-ins — your call; pin behavior, not construction.

router (`categorize`):
- each built-in name → its expected `Category`;
- an unknown name (e.g. `"frobnicate"`) → `None`;
- **completeness:** iterate the seven built-in names, assert every one is
  `Some(_)` (guards against a new tool being added without a category).

registry:
- a registry with one tool per category → `categories()` is `[Read, Write,
  Search, Run]` (stable order), regardless of registration order;
- a registry with only Read tools → `categories()` is `[Read]`;
- `tools_in(Category::Read)` on a registry holding `read_file` + `symbols` returns
  both, name-sorted; `tools_in` of a category with no registered tools → empty;
- registration order does not change `categories()` / `tools_in()` output
  (determinism).

## End-to-end verification

> Not applicable — this phase ships a pure categorization primitive + registry
> query methods, exercised directly by unit tests. The 2-stage presentation that
> consumes them is an M4 agent-loop behavior; there is no runtime-loadable artifact
> here. Restate this in the completion entry.

## Authorizations

- [x] **May create** `executor/src/tools/router.rs`; **may modify**
      `executor/src/tools/registry.rs` (add the two query methods) and
      `tools/mod.rs` (declare + re-export).
- [ ] **No new dependencies.**
- [ ] May **NOT** add the 2-stage prompt presentation, a governor / schema
      ordering, a `Plan` category, or change the `Tool` trait / `register` /
      `dispatch`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      any other phase doc.

## Out of scope

- **The 2-stage presentation flow** (showing a model categories, then one
  category's schemas) — M4 agent loop.
- **Governor-aware ordering** of tools/schemas within a category — M4 (needs the
  governor's scorer).
- **A `Plan` category / plan / memory tools** — not in rexyMCP's roadmap; add the
  variant if/when such a tool lands.
- **`build_default`-style registry assembly from config** (Rexy's
  `tools::build_default`) — that is loop/config wiring, M4. Phase-06 only adds
  query methods to the existing registry.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
