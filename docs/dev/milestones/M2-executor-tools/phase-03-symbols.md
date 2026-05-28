# Phase 03: symbols — tree-sitter symbol-definition search

**Milestone:** M2 — Executor tools & security
**Status:** done
**Depends on:** phase-02 (done)
**Estimated diff:** ~450 lines (net-new tool + two language grammars + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add `symbols`: a language-aware tool that finds **symbol definitions** by name
across the scoped repo using tree-sitter. Where `search` (phase-02) is a blind
regex grep, `symbols` parses each file and returns only *definitions* (functions,
structs, classes, …) whose identifier matches, tagged with their kind. This is
the executor's "go to definition" primitive.

This phase is **net-new, not a lift.** Rexy's `src/tools/symbols.rs` is a 3-line
`// TODO: implement` stub — there is nothing to copy. The design below is the
spec; implement it, do not improvise an alternative.

Scope is deliberately **two languages — Rust and Python** — behind an
extension→language dispatch so a third language is a few lines later. See Out of
scope.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (tools
  row; `symbols` is listed but, per the M2 README Notes, is the one net-new tool).
- `docs/architecture.md` — "The executor turn cycle" step 5 (all filesystem
  access scoped to the target-repo root).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M2 README Notes.
3. Read this entire phase doc — including the **Pre-injection** section, which
   pins the exact tree-sitter queries and API call sequence. Do not write your
   own queries; use the ones given.
4. Confirm phase-02 is `done`; `executor::security::scope::Scope`,
   `executor::tools::registry::{Tool, ToolResult}`, and the `find_files` /
   `search` tools exist and the workspace builds clean.
5. Study `executor/src/tools/search.rs` (phase-02) — it is the structural pattern
   to follow: a tool holds a `Scope`, the constructor takes a `Scope`, `execute`
   resolves the model-supplied `path` through `self.scope.resolve(...)` (advisory
   `ToolResult { error: Some(...) }` on `ScopeError`), walks gitignore-aware via
   `ignore::Walk` with `follow_links` **off**, supports single-file `path`, and
   caps at `max_results` with a truncation note.

## Current state

- `executor/src/tools/` has `registry.rs`, `read_file.rs`, `find_files.rs`,
  `search.rs`, `mod.rs`. The `Tool` trait (`registry.rs`):

  ```rust
  #[async_trait]
  pub trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn description(&self) -> &str;
      fn schema(&self) -> Value;            // OpenAI function-calling shape
      async fn execute(&self, args: Value) -> Result<ToolResult>;  // anyhow::Result
  }
  pub struct ToolResult { pub output: String, pub error: Option<String>, pub metadata: Option<Value> }
  ```

- `executor/Cargo.toml` has `globset`, `ignore`, `regex` (phase-02) but **no
  tree-sitter crates** — authorized below.
- `Scope::resolve(&str) -> Result<PathBuf, ScopeError>` confines to the root;
  `Scope::new(&Path)` canonicalizes. Construct tools in tests with
  `Scope::new(dir.path()).unwrap()`.

## Spec

### 1. Dependencies

Add to `[workspace.dependencies]` and `executor/Cargo.toml` (authorized below):

- `tree-sitter = "0.25"` — the core parser/query engine.
- `tree-sitter-rust = "0.24"` — Rust grammar. Exposes `LANGUAGE: LanguageFn`.
- `tree-sitter-python = "0.23"` — Python grammar. Exposes `LANGUAGE: LanguageFn`.

These version *numbers* differ between core and grammars — that is normal;
grammar crates version independently of the core. **What matters is ABI
compatibility:** the grammar crate must target the core's language ABI. Pin the
latest published `tree-sitter-rust` / `tree-sitter-python` that build against
`tree-sitter = "0.25"` and expose the `LANGUAGE: LanguageFn` constant (the modern
API). **If `cargo build` reports a language-ABI / version mismatch between the
core and a grammar, that is a blocker** — report the exact versions cargo
resolved and stop. Do not downgrade the core below 0.24 to dodge it without
authorization.

The streaming-iterator trait needed by `QueryCursor::matches` (see Pre-injection)
is re-exported as `tree_sitter::StreamingIterator` in the 0.25 line — prefer that
import. **Only if** the pinned version does not re-export it, the
`streaming-iterator = "0.1"` crate is additionally authorized for that one
import; note in the Update Log which path you took.

### 2. symbols — `executor/src/tools/symbols.rs` (new file)

A `Symbols` struct holding a `Scope`; constructor `symbols(scope: Scope) ->
Arc<dyn Tool>`. Name: `"symbols"`.

**Args** (deserialize with serde, advisory error on bad shape — mirror search):

- `name` (string, **required**) — the symbol identifier to find. Matched
  **exactly** against the captured identifier text (no substring, no regex).
- `path` (string, optional) — file or directory, scope-confined, **defaults to
  the scope root**. Same resolution + existence handling as `search`.
- `kind` (string, optional) — restrict to one kind. Valid values are exactly the
  capture names in the Pre-injection queries:
  `function`, `struct`, `enum`, `trait`, `module`, `const`, `static`, `type`,
  `macro`, `class`. If `kind` is present and not one of these, return an
  **advisory** error listing the valid kinds (do not silently ignore it).
- `max_results` (usize, optional, default 100). `0` → advisory error
  (`max_results must be >= 1`), same as search.

**Schema** — OpenAI function-calling shape (`json!`), `required: ["name"]`. The
`path` description must state: "File or directory to search. Defaults to the
project root. Confined to the project root." The `kind` description must list the
valid kinds.

**Behavior:**

1. Validate args (empty `name` → advisory `"name must not be empty"`; bad `kind`
   → advisory; `max_results == 0` → advisory).
2. Resolve `path` (default `"."`) through `self.scope.resolve(...)`; advisory on
   `ScopeError`. Existence check (advisory `"search root does not exist: <path>"`).
   Canonicalize for the walk root (`canonicalize().unwrap_or_else(|_| root.clone())`,
   same as search).
3. If the resolved path **is a file**: parse just that file. If its extension is
   unsupported, return an advisory error `"unsupported file type: <path>"`.
4. If it is a **directory**: walk gitignore-aware via `ignore::Walk` with
   `follow_links` **off**. For each regular file, dispatch on extension:
   `.rs` → Rust, `.py` → Python. **Files with any other extension are skipped
   silently** (not an error — a polyglot repo is normal).
5. For each supported file: read it (`read_to_string`; on `Err`, skip the file —
   binary/unreadable is not a tool error), parse with tree-sitter, run that
   language's pinned query, and for every capture whose **node text == `name`**
   (and whose capture-name == `kind` if a `kind` filter is set), record a hit:
   relative path (strip the walk root, same as search), 1-based line and column
   (`node.start_position()` is 0-based — add 1 to each), the kind (the capture
   name), and the symbol name.
6. Cap at `max_results` (stop walking once reached; mark truncated). Empty →
   advisory `"no symbols named \`<name>\` in <root>"`. Otherwise format output.

**Output format** (mirror search's grouped style):

```
✓ <N> definitions of `<name>`

<rel_path>:
  <line>:<col>  <kind> <name>
  ...
```

…with a truncation footer when capped (`[… truncated at <N> …]`) and `metadata`
carrying `{ path, name, definitions: N, files: M, truncated: bool }`. Match the
shape of `search`'s `format_output` — you may diverge on wording, not on having
`output` + `metadata` + `error: None` on success.

**Error handling.** Bad args / scope escape / no matches / unsupported single
file → advisory `ToolResult { error: Some(_), output: "" }` (model-visible
outcomes, never `Result::Err`). A failure compiling the **static** query or
setting the **static** grammar language is *our* programmer error, not a
model-visible one — propagate it with `?` (it cannot happen at runtime with a
correct query, and a green build proves the query compiles). A `parser.parse(...)`
returning `None` → skip that file (treat as unparseable, not a crash). No
`.unwrap()` / `.expect()` / `panic!()` in the production path; no raw slice
indexing that can panic (use `source.get(node.byte_range())` / a checked lookup
for capture names — see Pre-injection).

### 3. Wiring — `executor/src/tools/mod.rs`

Add `mod symbols;` and `pub use symbols::{Symbols, symbols};` (mirror the
existing `find_files` / `search` re-exports).

## Pre-injection — use these verbatim

> The architect supplies these because authoring tree-sitter queries and the 0.25
> streaming-iterator call sequence from scratch is the failure-prone part. Treat
> the query strings as **spec-pinned literals** — copy them exactly. (See
> Acceptance criteria for the required grep proof.)

**Rust query** (`const RUST_QUERY: &str`):

```
(function_item name: (identifier) @function)
(struct_item name: (type_identifier) @struct)
(enum_item name: (type_identifier) @enum)
(trait_item name: (type_identifier) @trait)
(mod_item name: (identifier) @module)
(const_item name: (identifier) @const)
(static_item name: (identifier) @static)
(type_item name: (type_identifier) @type)
(macro_definition name: (identifier) @macro)
```

**Python query** (`const PYTHON_QUERY: &str`):

```
(function_definition name: (identifier) @function)
(class_definition name: (identifier) @class)
```

The **capture name is the kind** — `@function` → kind `"function"`, etc. Methods
nested in a Rust `impl` or a Python `class` are themselves `function_item` /
`function_definition`, so they are captured as `function`; v1 does not give them
a distinct `method` kind.

**API call sequence** (tree-sitter 0.25; adapt the language/query per file):

```rust
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
let query = Query::new(&language, RUST_QUERY)?;     // static query → propagate with ?
let mut parser = Parser::new();
parser.set_language(&language)?;                    // static grammar → propagate with ?

let Some(tree) = parser.parse(source.as_bytes(), None) else {
    // unparseable file: skip it, do not error
    continue;
};

let capture_names = query.capture_names();          // &[&str], indexed by capture.index
let mut cursor = QueryCursor::new();
let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());
while let Some(m) = it.next() {                      // StreamingIterator: while-let + .next()
    for cap in m.captures {
        let kind = capture_names[cap.index as usize];        // index is always valid for this query
        let Some(text) = source.get(cap.node.byte_range()) else { continue };
        if text != requested_name { continue; }
        if let Some(want) = kind_filter && want != kind { continue; }
        let p = cap.node.start_position();           // 0-based row/column
        // record hit: line = p.row + 1, col = p.column + 1, kind, text
    }
}
```

`cursor.matches(...)` returns a **streaming iterator**, not a normal `Iterator` —
you must bring the `StreamingIterator` trait into scope and drive it with
`while let Some(_) = it.next()`. Calling `.into_iter()` / `for m in ...` will not
compile. (Prefer `use tree_sitter::StreamingIterator;`; fall back to the
`streaming-iterator` crate only if that re-export is absent — see §1.)

Indexing `capture_names[cap.index as usize]` is sound: `cap.index` is always a
valid capture index for the query the match came from (tree-sitter invariant). If
clippy or review prefers it, `capture_names.get(idx).copied().unwrap_or("")` is an
acceptable panic-free equivalent — but never an `.unwrap()`.

## Acceptance criteria

- [ ] `executor/src/tools/symbols.rs` exists and is declared + re-exported in
      `tools/mod.rs`; `symbols(scope)` constructs an `Arc<dyn Tool>` holding the
      scope; tool name is `"symbols"`.
- [ ] In a Rust source file, `symbols { name: "foo" }` finds a `fn foo` and a
      `struct Foo`-style definition by its exact identifier, reporting
      `file:line:col`, the kind, and the name; a non-matching identifier is not
      returned.
- [ ] In a Python source file, it finds a `def foo` (kind `function`) and a
      `class Foo` (kind `class`).
- [ ] The `kind` filter restricts results to that kind; an invalid `kind` returns
      an advisory error listing the valid kinds.
- [ ] Exact-match semantics: `name: "foo"` does **not** match an identifier
      `foobar`.
- [ ] gitignore is respected; results cap at `max_results` with a truncation
      note; `follow_links` is **not** enabled on the walk.
- [ ] Files of unsupported extension in a directory walk are skipped silently
      (no error, no spurious match); a single-file `path` of unsupported type
      returns an advisory error.
- [ ] Advisory error (no panic) on: empty `name`, `max_results: 0`, `path`
      resolving outside the scope root (`ScopeError`), `path` that does not exist.
- [ ] The pinned query literals appear verbatim in `symbols.rs` (grep proof in
      the completion entry — see AGENTS.md § "Grep for spec-pinned literals").
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, `tempfile::TempDir` as the scope root; `Scope::new(dir.path())`. Pin
behavior, not exact counts/names of tests. Write small fixture sources into the
TempDir.

Rust:
- finds a function definition by exact name, reports kind `function` + a
  plausible `line:col`;
- finds a `struct` / `trait` / `enum` definition with the right kind;
- `kind: "struct"` returns the struct but not a same-named function;
- exact match: `name: "foo"` does not match `foobar`;
- gitignored file is not searched (use the `git init` + `.gitignore` pattern from
  `search.rs`'s gitignore test);
- caps at `max_results`.

Python:
- finds a `def` (kind `function`) and a `class` (kind `class`) by name.

Cross-cutting:
- a `.txt` / unsupported file in the walk is skipped (no match, no error);
- single-file `path` pointing at an unsupported extension → advisory error;
- empty `name` → advisory; invalid `kind` → advisory listing valid kinds;
- `path: "../outside"` → advisory `ScopeError` (assert the error mentions
  escaping the root, as in search's `rejects_path_outside_root`);
- non-existent in-scope `path` → advisory "does not exist".

## End-to-end verification

> Not applicable — this phase ships one library tool exercised directly by its
> unit tests. The registry/loop that drives it lands in M4 and the MCP
> `execute_phase` in M5; there is no runtime-loadable artifact to exercise yet.
> Restate this in the completion entry.

## Authorizations

- [x] **May add dependencies:** `tree-sitter = "0.25"`, `tree-sitter-rust`,
      `tree-sitter-python` (versions per §1), and **conditionally**
      `streaming-iterator = "0.1"` *only* if `tree_sitter::StreamingIterator` is
      not re-exported. No others.
- [x] **May create** `executor/src/tools/symbols.rs`; **may modify**
      `tools/mod.rs` and the two `Cargo.toml` files for the deps.
- [ ] May **NOT** add languages beyond Rust + Python, an LSP bridge, write/run
      tools, or the router.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      or any other phase doc.

## Out of scope

- **More languages** (Go, TypeScript, JavaScript, …). The extension→language
  dispatch makes each a small follow-up (one grammar dep + one pinned query +
  tests); not in this phase. Do not add them speculatively.
- **References / call-sites / usages.** This tool returns *definitions* only.
- **A distinct `method` kind**, signatures (`function_signature_item`), or
  impl/trait association — methods report as `function`.
- **LSP bridge** (hover/refs) — a later, separate effort if ever.
- **The 2-stage router** (M2 phase-06) and write tools (phase-04).
- **`follow_links` / symlink following** — intentionally off; not a feature here.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (progress)

Implemented `symbols` tool: created `executor/src/tools/symbols.rs` with `Symbols` struct holding a `Scope`, constructor `symbols(scope) -> Arc<dyn Tool>`, name `"symbols"`. Added tree-sitter dependencies (`tree-sitter = "0.25"`, `tree-sitter-rust = "0.24"`, `tree-sitter-python = "0.23"`) to workspace and executor `Cargo.toml`. Used verbatim pinned queries from spec. `tree_sitter::StreamingIterator` is re-exported in 0.25 — no separate crate needed. Wired in `tools/mod.rs`. Wrote 18 hermetic tests covering Rust/Python definitions, kind filter, exact match, gitignore, max_results truncation, unsupported extension handling, and all advisory error paths. All 102 tests pass (84 existing + 18 new).

### Update — 2026-05-28 (complete)

**Summary:** Implemented `symbols` tool — tree-sitter symbol-definition search across Rust and Python files. Added `tree-sitter`, `tree-sitter-rust`, `tree-sitter-python` dependencies. Created `executor/src/tools/symbols.rs` with full implementation: scope-confined path resolution, gitignore-aware directory walk (`follow_links` off), exact identifier matching via pinned tree-sitter queries, kind filtering, `max_results` capping with truncation note, advisory errors for all failure modes. Used `tree_sitter::StreamingIterator` re-export (no separate crate needed). Wired in `tools/mod.rs`. 18 hermetic tests added. No deviations from spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.45s

cargo clippy --all-targets --all-features -- -D warnings
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.41s

cargo test
running 102 tests
test result: ok. 102 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**
Not applicable — this phase ships one library tool exercised directly by its unit tests. The registry/loop that drives it lands in M4 and the MCP `execute_phase` in M5; there is no runtime-loadable artifact to exercise yet.

**Files changed:**
- `Cargo.toml` — added `tree-sitter`, `tree-sitter-rust`, `tree-sitter-python` workspace deps
- `executor/Cargo.toml` — added tree-sitter workspace deps
- `executor/src/tools/symbols.rs` — new file: `Symbols` tool implementation + 18 tests
- `executor/src/tools/mod.rs` — added `mod symbols;` and re-exports

**New tests:**
- `finds_rust_function_by_name` in `executor/src/tools/symbols.rs`
- `finds_rust_struct_and_trait` in `executor/src/tools/symbols.rs`
- `kind_filter_returns_only_matching_kind` in `executor/src/tools/symbols.rs`
- `exact_match_no_substring` in `executor/src/tools/symbols.rs`
- `respects_gitignore` in `executor/src/tools/symbols.rs`
- `caps_at_max_results` in `executor/src/tools/symbols.rs`
- `finds_python_function_and_class` in `executor/src/tools/symbols.rs`
- `unsupported_extension_skipped_in_dir_walk` in `executor/src/tools/symbols.rs`
- `single_file_unsupported_extension_advisory_error` in `executor/src/tools/symbols.rs`
- `rejects_empty_name` in `executor/src/tools/symbols.rs`
- `rejects_invalid_kind` in `executor/src/tools/symbols.rs`
- `rejects_max_results_zero` in `executor/src/tools/symbols.rs`
- `rejects_path_outside_root` in `executor/src/tools/symbols.rs`
- `rejects_nonexistent_path` in `executor/src/tools/symbols.rs`
- `reports_line_and_column` in `executor/src/tools/symbols.rs`
- `defaults_to_scope_root_when_no_path_given` in `executor/src/tools/symbols.rs`
- `no_symbols_returns_advisory_error` in `executor/src/tools/symbols.rs`
- `metadata_carries_definitions_and_files_count` in `executor/src/tools/symbols.rs`

**Spec-pinned literal grep proof:**
```
grep "function_item name: (identifier) @function" symbols.rs → 1 match
grep "function_definition name: (identifier) @function" symbols.rs → 1 match
```

**verification:** fmt OK · clippy OK · tests 102 passed · build OK

**Notes for review:** None.
