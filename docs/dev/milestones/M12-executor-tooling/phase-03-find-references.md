# Phase 03: Find-references in `symbols` (tree-sitter call-site search)

**Milestone:** M12 — Executor Tooling
**Status:** review
**Depends on:** none (additive extension of the existing `symbols` tool)
**Estimated diff:** ~320 lines (≈140 production + ≈180 tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the executor a way to enumerate **every call site / usage of a named
symbol** before it makes a breaking edit. Today `symbols` finds only
*definitions*; this phase adds a `mode` parameter so the same tool can also
return *references* — every place the name is used — using the tree-sitter
parsers already compiled in. This directly attacks the multi-site-change failure
class folded into `WORKFLOW.md` § "Prefer additive change shapes": an executor
that must change a function signature can first list every caller, instead of
changing the definition and running out of verifier runway mid-cascade.

The win over the existing `search` (blind regex grep) is **syntactic
precision**: a reference is an identifier *token* in the parse tree, so `foo`
never matches `foobar`, and an occurrence inside a string literal or a comment is
**not** a reference. That precision is the whole point — keep it.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M12 Arc B, first bullet: "**Find-references in
  `symbols`** via tree-sitter (syntactic call-site search, deps already present).
  Attacks the multi-site-change failure class folded into WORKFLOW (§ 'Prefer
  additive change shapes') — enumerate every call site *before* a breaking edit."
- `docs/architecture.md#status` — M12 Arc B preamble: "incremental code
  intelligence (keep-it-simple; full LSP deferred)". This is **syntactic**, not
  semantic — no scope resolution. Two same-named symbols in unrelated scopes both
  match; that is accepted (see Out of scope).
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — the failure class
  this feature exists to mitigate, and the shape this phase itself follows (a new
  `mode` value, no change to definitions behavior).
- `docs/dev/STANDARDS.md` §2.2 — "No fallbacks for 'if X is missing.'" Inapplicable
  argument combinations (e.g. `kind` in references mode) fail loud, not silently.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Everything in this phase lives in **one file**: `executor/src/tools/symbols.rs`
(867 lines). Nothing else references the `symbols` schema or description — the
router categorizes it by name (`router.rs:15`), and the registry tests register a
mock named `"symbols"`, so **changing the schema/description breaks nothing
outside this file**. This is a single-file, additive change.

The tool today finds **definitions**. The load-bearing existing pieces you will
mirror:

### The definition query + parse loop (the worked example for references)

`symbols.rs:20-30` — the Rust *definition* query captures the **name node of each
definition construct**:

```rust
const RUST_QUERY: &str = r#"
(function_item name: (identifier) @function)
(struct_item name: (type_identifier) @struct)
(enum_item name: (type_identifier) @enum)
(trait_item name: (type_identifier) @trait)
(mod_item name: (identifier) @module)
(const_item name: (identifier) @const)
(static_item name: (identifier) @static)
(type_item name: (type_identifier) @type)
(macro_definition name: (identifier) @macro)
"#;
```

`symbols.rs:302-355` — `parse_file` runs that query, and for each capture whose
**text equals the requested name**, records a `SymbolHit`:

```rust
fn parse_file(
    source: &str,
    lang: Language,
    requested_name: &str,
    kind_filter: Option<&str>,
) -> Result<Vec<SymbolHit>> {
    let (language, query_str) = match lang {
        Language::Rust => (tree_sitter_rust::LANGUAGE.into(), RUST_QUERY),
        Language::Python => (tree_sitter_python::LANGUAGE.into(), PYTHON_QUERY),
    };
    let language: tree_sitter::Language = language;
    let query = Query::new(&language, query_str)?;
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return Ok(Vec::new());
    };
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut it = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut hits = Vec::new();
    while let Some(m) = it.next() {
        for cap in m.captures {
            let idx = cap.index as usize;
            let kind = capture_names.get(idx).copied().unwrap_or("");
            let Some(text) = source.get(cap.node.byte_range()) else {
                continue;
            };
            if text != requested_name {        // <-- exact-token match, not substring
                continue;
            }
            if let Some(want) = kind_filter
                && want != kind
            {
                continue;
            }
            let p = cap.node.start_position();
            hits.push(SymbolHit {
                rel_path: String::new(),
                line: p.row + 1,               // tree-sitter rows are 0-based
                col: p.column + 1,
                kind: kind.to_string(),
                name: text.to_string(),
            });
        }
    }
    Ok(hits)
}
```

`symbols.rs:357-399` — `format_output` renders grouped-by-file with a `✓ N
definitions of …` header and a `{path, name, definitions, files, truncated}`
metadata object. The references renderer mirrors this shape.

The directory walk (`symbols.rs:172-217`) and single-file branch
(`symbols.rs:168-170`, `execute_single_file`) call `parse_file`. References mode
needs the **same** file selection (gitignore-respecting `Walk`, `.rs`/`.py` only,
`max_results` cap, single-file vs. directory) — you may share that traversal or
add a parallel one; that is your structural call. Do **not** alter the
definitions path: existing tests must stay green unchanged.

### Verified node kinds (do not guess these)

The reference query below uses three tree-sitter-rust node kinds. All three are
confirmed present in the vendored grammar (`tree-sitter-rust 0.24.2`
`node-types.json`): `identifier`, `type_identifier`, `field_identifier`. An
invalid node kind makes `Query::new` return `Err` — these are valid. Python uses
only `identifier`.

## Spec

Numbered tasks in execution order. All changes are in
`executor/src/tools/symbols.rs`.

1. **Add the `mode` argument.** In `SymbolsArgs` (line 41), add
   `mode: Option<String>`. Add a `const VALID_MODES: &[&str] = &["definitions",
   "references"];` near `VALID_KINDS`. In `execute`, after parsing args and before
   resolving the search root, validate: if `mode` is `Some(m)` and
   `!VALID_MODES.contains(&m.as_str())`, return an advisory `ToolResult` (error =
   `format!("invalid mode: {m}. Valid modes: {}", VALID_MODES.join(", "))`,
   `output` empty, `metadata` None) — mirror the existing invalid-`kind` block at
   lines 120-131. A missing/`"definitions"` mode means the current behavior,
   unchanged.

2. **Reject `kind` in references mode.** `kind` is a definition-kind filter and
   has no meaning for references. If `mode == Some("references")` **and** `kind`
   is `Some`, return an advisory error
   `"kind filter is not supported in references mode"`. This is a loud failure for
   an inapplicable combination (STANDARDS §2.2), not a silent ignore.

3. **Add the reference queries.** Near the definition queries:

   ```rust
   const RUST_REF_QUERY: &str = r#"
   (identifier) @ref
   (type_identifier) @ref
   (field_identifier) @ref
   "#;

   const PYTHON_REF_QUERY: &str = r#"
   (identifier) @ref
   "#;
   ```

   These capture **every** identifier-class token. The exact-text filter (task 4)
   keeps only the ones equal to the requested name. Because they match parse-tree
   *nodes*, tokens inside string literals and comments are structurally excluded —
   that exclusion is a pinned requirement (Test plan), not an accident.

4. **Add `parse_references`.** A function parallel to `parse_file` that runs the
   reference query for the language and, for each capture whose **text equals
   `requested_name`**, records a reference hit carrying `line` (`row + 1`), `col`
   (`column + 1`), and a **snippet** = the source line at that row, trimmed:
   `source.lines().nth(p.row).unwrap_or("").trim().to_string()`. There is no
   `kind`/`kind_filter` for references. Use a dedicated hit struct (e.g.
   `struct RefHit { rel_path: String, line: usize, col: usize, snippet: String }`)
   — do not overload `SymbolHit`'s `kind`/`name` fields to smuggle the snippet.

   **References include the definition site.** A `fn foo` defined once and called
   twice yields **3** references (the definition's name token is an `identifier`
   too). This is intended: the use case is "every site I must touch for a breaking
   edit," and the definition is one of them. Do **not** filter the definition out.

5. **Add `format_references`.** Mirror `format_output` but for reference hits:
   - Header: `format!("✓ {} references to `{name}`\n\n", hits.len())`.
   - Grouped by file (same `current_file` grouping as `format_output`), each line
     `  {line}:{col}  {snippet}\n`.
   - Truncation note when capped (same `[… truncated at N …]` shape).
   - Metadata: `{"path": …, "name": …, "references": <count>, "files":
     <distinct file count>, "truncated": <bool>}`.

6. **Wire references mode into `execute` and the single-file path.** When
   `mode == Some("references")`, run the reference traversal instead of the
   definition one, honoring the same `max_results` cap, gitignore/`.rs`/`.py`
   filtering, and single-file-vs-directory dispatch. Empty result → advisory error
   `format!("no references to `{name}` in {}", <root>.display())` (parallel to the
   existing "no symbols named" advisory). You may extract a shared walk or
   duplicate the ~30-line traversal — structural call, but the **behavior** (file
   selection, capping, single-file handling, empty-advisory) must match
   definitions mode.

7. **Update `schema()` and `description()`.** Add a `mode` property to the schema
   (`type: "string"`, description naming the two values and that `definitions` is
   the default). Note in the description that `kind` applies only to definitions.
   Update `description()` to say the tool finds symbol **definitions or
   references**. Pin user-visible *content* only — exact wording is yours.

## Acceptance criteria

- [ ] `symbols` with `mode: "references"` for a name with a definition and N
      call sites returns N+1 references (definition included), grouped by file,
      each line showing `line:col  <source snippet>`.
- [ ] A reference search for `foo` does **not** match `foobar` (exact token, not
      substring).
- [ ] A reference search for `foo` does **not** count an occurrence of `foo`
      inside a string literal or a comment (tree-sitter token, not text grep).
- [ ] References span multiple files; metadata `files` reflects the distinct
      file count and `references` the total.
- [ ] Python references work (`mode: "references"` over a `.py` file).
- [ ] `mode: "references"` with a `kind` argument returns an advisory error
      naming the unsupported combination.
- [ ] An invalid `mode` value returns an advisory error listing the valid modes.
- [ ] With `mode` omitted (or `"definitions"`), behavior is byte-identical to
      today — all existing `symbols` tests pass unchanged.
- [ ] `max_results` caps references and sets `truncated: true` in metadata.
- [ ] No references found → advisory error `no references to `<name>` in <path>`.
- [ ] `cargo build` (zero new warnings), `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, `cargo test` all
      pass.

## Test plan

New `#[tokio::test]`s in the existing `mod tests` block at the bottom of
`symbols.rs`, mirroring the existing test style (real `TempDir`, real tree-sitter
parse, no mocks). Names are pinned; placement/count details are yours.

- `references_finds_call_sites` — `fn foo() { foo(); foo(); }` (def + 2 calls) →
  output contains `✓ 3 references to `foo``; metadata `references` == 3.
- `references_exclude_substring` — file with `fn foobar()` and `fn foo()` plus a
  call `foo();` → references to `foo` do **not** include any `foobar` occurrence.
- `references_exclude_strings_and_comments` — file:
  `fn foo() { foo(); }\n// foo again\nfn other() { let s = "foo"; }` → references
  to `foo` == **2** (the def + the call); the comment `foo` and the string `"foo"`
  are excluded. (This is the grep-differentiator — make the assertion explicit on
  the count.)
- `references_across_multiple_files` — `foo` referenced in `a.rs` and `b.rs`;
  metadata `files` == 2 and `references` == the combined count.
- `references_python_identifier` — `def foo():\n    pass\nfoo()` → references to
  `foo` found in the `.py` file (count includes the def + the call).
- `references_respects_max_results` — many references, `max_results: 1` →
  metadata `truncated` is `true`.
- `references_kind_filter_rejected` — `mode: "references"`, `kind: "function"` →
  `result.error` contains `not supported in references mode`.
- `references_invalid_mode_rejected` — `mode: "usages"` → `result.error` contains
  `invalid mode`.
- `references_single_file_path` — `path` pointing at one `.rs` file → references
  found within just that file.
- `references_no_matches_advisory` — `mode: "references"`, `name:
  "nonexistent"` → `result.error` contains `no references to`.
- `references_snippet_shows_source_line` — a call on a known line → output
  contains the trimmed source text of that line (e.g. `foo();`).

## End-to-end verification

> Not applicable — phase ships no runtime-loadable CLI/binary artifact. The
> `symbols` tool is reachable only through a full `execute_phase` local-LLM loop,
> which cannot be exercised hermetically. **However**, note for the reviewer: the
> unit tests above are *de facto* end-to-end for this artifact — they invoke the
> real `Symbols` tool (`tool.execute(...)`) with the real compiled-in
> tree-sitter-rust / tree-sitter-python grammars against real files written to a
> `TempDir`. There is no fake or mock anywhere in the path; the test *is* the real
> parser running on real source. Quote the `references_exclude_strings_and_comments`
> and `references_finds_call_sites` outputs in the completion Update Log.

## Authorizations

None. No new dependencies (`tree-sitter`, `tree-sitter-rust`,
`tree-sitter-python` are already in `executor/Cargo.toml`). No
`docs/architecture.md` edit. No files outside `executor/src/tools/symbols.rs`.

## Out of scope

- **No semantic scope resolution.** This is syntactic token matching. Two
  same-named symbols in unrelated scopes (a local `foo` variable and a function
  `foo`) both match — accepted, and documented in the schema/description. Do not
  build scope analysis.
- **No new languages.** Rust + Python only, exactly as definitions mode today.
- **No method-vs-field disambiguation.** `field_identifier` capture means a field
  access and a method call of the same name both surface; that is fine for a
  blast-radius enumerator.
- **No change to definitions-mode behavior or output.** The default path stays
  byte-identical.
- **Not the other Arc B phases.** No verifier suggested-fix spans (phase-04), no
  structured test-failure parsing (phase-05), no task tracking (phase-06/07).
- **No new dependency, no `unsafe`, no `#[allow]`.** If you think you need any,
  stop and file a blocker.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-09 (started)

**Executor:** rexyMCP executor (Claude Sonnet 4.6)

**Progress:** Phase 03 implementation started. Adding `mode` parameter to `symbols` tool with `references` support via tree-sitter call-site search. All changes in `executor/src/tools/symbols.rs`.

### Update — 2026-06-09 (complete)

**Summary:** Added `mode` parameter to the `symbols` tool enabling `references` mode — a syntactic call-site search via tree-sitter. All changes are in `executor/src/tools/symbols.rs`: new `VALID_MODES` const, `RUST_REF_QUERY`/`PYTHON_REF_QUERY` constants, `RefHit` struct, `parse_references` and `format_references` functions, wiring into `execute` and `execute_single_file`, schema/description updates, and 11 new tests. No deviations from the spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean — no output)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.12s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.55s

cargo test 2>&1 | tail -30
test result: ok. 685 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
```

**Verification summary:** `cargo fmt --all --check` clean, `cargo build` zero new warnings, `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo test` 685 passed (0 failed).

**End-to-end verification:** N/A — phase ships no runtime-loadable CLI/binary artifact. Unit tests invoke the real `Symbols` tool with real compiled-in tree-sitter grammars against real files in a `TempDir`. Key outputs:
- `references_finds_call_sites`: output contains `✓ 3 references to \`foo\``; metadata `references` == 3 (def + 2 calls).
- `references_exclude_strings_and_comments`: metadata `references` == 2 (def + call only; comment `foo` and string `"foo"` excluded by tree-sitter parse tree).

**Files changed:**
- `executor/src/tools/symbols.rs` — Added `mode` parameter, reference queries, `RefHit` struct, `parse_references`, `format_references`, wired into `execute`/`execute_single_file`, updated schema/description, 11 new tests.
- `docs/dev/milestones/M12-executor-tooling/phase-03-find-references.md` — Status `todo` → `review`, Update Log entries.
- `docs/dev/milestones/M12-executor-tooling/README.md` — Phase table row updated to `review`.

**New tests:**
- `references_finds_call_sites` in `executor/src/tools/symbols.rs`
- `references_exclude_substring` in `executor/src/tools/symbols.rs`
- `references_exclude_strings_and_comments` in `executor/src/tools/symbols.rs`
- `references_across_multiple_files` in `executor/src/tools/symbols.rs`
- `references_python_identifier` in `executor/src/tools/symbols.rs`
- `references_respects_max_results` in `executor/src/tools/symbols.rs`
- `references_kind_filter_rejected` in `executor/src/tools/symbols.rs`
- `references_invalid_mode_rejected` in `executor/src/tools/symbols.rs`
- `references_single_file_path` in `executor/src/tools/symbols.rs`
- `references_no_matches_advisory` in `executor/src/tools/symbols.rs`
- `references_snippet_shows_source_line` in `executor/src/tools/symbols.rs`

**Commits:**
- (pending) — `feat: add references mode to symbols tool`

**Notes for review:** None — implementation matches spec exactly. Grep confirms pinned literals landed: `grep -c 'RUST_REF_QUERY\|PYTHON_REF_QUERY\|VALID_MODES\|parse_references\|format_references' executor/src/tools/symbols.rs` → 20 matches across the expected definitions and call sites.
