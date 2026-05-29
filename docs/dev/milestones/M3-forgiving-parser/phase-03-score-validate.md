# Phase 03: candidate scoring + validation

**Milestone:** M3 — Forgiving parser
**Status:** done
**Depends on:** phase-02 (done)
**Estimated diff:** ~620 lines (score + validate + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Lift the two registry-aware stages that turn `Candidate`s into a decision: `score`
(rank a candidate against the registry — exact/fuzzy name + required/unknown/type
param signals) and `validate` (turn the best candidate into a `ToolCall`, or a
structured `ValidationError` the feedback formatter will explain). These are the
first parser stages that depend on the M2 `ToolRegistry`.

## Architecture references

- `docs/architecture.md` — lift/drop map (parser row, "Lift"); turn cycle step 4.
- Rexy source: `rexy/src/agent/parser/score.rs`, `rexy/src/agent/parser/validate.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M3 README Notes.
2. Read this entire phase doc.
3. Confirm phase-02 is `done`; `executor::parser::{Candidate, Format, Origin,
   RepairOp, ToolCall}` and `executor::tools::ToolRegistry` exist.
4. **Read** `rexy/src/agent/parser/score.rs` and `validate.rs`. Lift faithfully.

## Spec

Create `executor/src/parser/score.rs` and `executor/src/parser/validate.rs`; wire
`pub mod score; pub mod validate;` into `parser/mod.rs`. Both use
`use crate::tools::ToolRegistry;` and `use super::{...}` for the parser types
(these resolve in rexyMCP as-is).

- **`score.rs`** — `pub fn score(&Candidate, &ToolRegistry) -> i32`. Weight table
  (lift verbatim): exact name `+5`, fuzzy name (Levenshtein ≤ 2, tiebreak
  lexicographically smallest) `+3`; per required param present `+1` / missing
  `-2`; per arg unknown `-1`; per arg type-match `+2`. Keep the private
  `levenshtein` and `type_matches`.
- **`validate.rs`** — `pub fn validate(&Candidate, &ToolRegistry) -> Result<ToolCall,
  ValidationError>`; the `pub enum ValidationError { UnknownTool {…},
  SchemaFailures {…} }` and `pub struct TypeMismatch {…}`. On success build
  `Origin::Extracted` (no repairs) or `Origin::Repaired` (with the candidate's
  repairs). Keep the private `sorted_tool_names` and `type_matches`.

**Adaptations:**

1. **Strip Rexy stage/plan references** ("Stage 6", "stage-4 weight table", "M2
   phase 07's orchestrator", "the caller … assigns it"). Rewrite to rexyMCP terms.
2. **`type_matches` stays per-file** (it is in both score and validate in Rexy —
   vendored, do not extract a shared helper).
3. **The two `.expect(...)` in `validate`** (`"name checked Some above"`,
   `"tool existence checked above"`) are permitted — the invariant is proven a few
   lines up (STANDARDS §2.1 allows `expect` with a justifying message). Keep them;
   do not introduce any *new* unwrap/expect.
4. **Test registry — do not use Rexy's `build_default`.** rexyMCP has no
   `tools::build_default` (registry assembly is M4). Build the test registry from
   the real tool constructors over a `TempDir` scope (validate/score only read
   `name()`/`schema()`, so the dir need not outlive construction):

   ```rust
   fn test_registry() -> ToolRegistry {
       use crate::security::scope::Scope;
       use crate::tools::{bash, find_files, patch, read_file, search, symbols, write_file};
       let dir = tempfile::TempDir::new().unwrap();
       let scope = Scope::new(dir.path()).unwrap();
       let mut r = ToolRegistry::new();
       for t in [
           read_file(scope.clone()), write_file(scope.clone()), patch(scope.clone()),
           search(scope.clone()), find_files(scope.clone()), symbols(scope.clone()),
           bash(scope, 30),
       ] { r.register(t); }
       r
   }
   ```

   `read_file`'s schema (`required: ["path"]`, `path: string`, plus optional
   `start_line`/`end_line`) makes every lifted score/validate assertion hold as-is
   (the extra optional props don't affect the scores the tests exercise).

## Acceptance criteria

- [ ] `executor/src/parser/{score,validate}.rs` exist, wired via `pub mod` in
      `parser/mod.rs`; `score(&Candidate,&ToolRegistry)->i32` and
      `validate(&Candidate,&ToolRegistry)->Result<ToolCall,ValidationError>`.
- [ ] `score`: exact name `+5`, fuzzy (dist ≤ 2) `+3`, name too far `0`; required
      present `+1` / missing `-2`; unknown arg `-1`; type match `+2`; mismatch adds
      nothing.
- [ ] `validate`: ok when required present + typed (origin `Extracted`, or
      `Repaired` if the candidate carried repairs); `UnknownTool` for a missing or
      `None` name (with sorted `available_tools`); `SchemaFailures` reporting
      missing-required, unknown-params, and type-mismatches together.
- [ ] **Negatives:** a name beyond fuzzy distance scores `0` and yields no param
      signals; `validate` of an unknown tool does not fabricate a `ToolCall`.
- [ ] No Rexy stage/plan references survive (grep `Stage [0-9]`, `phase 0` → 0).
- [ ] No new `unwrap()`/`expect()` beyond the two justified `expect`s in validate.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic; lift Rexy's score + validate tests, swapping `build_default` for the
`test_registry()` above. Pin behavior incl. the negatives. The exact score
integers from Rexy's tests hold against rexyMCP's `read_file` schema.

## End-to-end verification

> Not applicable — pure library functions exercised by unit tests; `parse()`
> (phase-05) composes them, the loop (M4) drives it. Restate in the completion
> entry.

## Authorizations

- [x] **May create** `executor/src/parser/score.rs` + `validate.rs`; **may
      modify** `executor/src/parser/mod.rs` (`pub mod` declarations).
- [ ] **No new dependencies.**
- [ ] May **NOT** lift `repair`, `feedback`, or `parse()`; may **NOT** add
      `tools::build_default`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **repair / feedback / `parse()`** — later M3 phases.
- **Sharing `type_matches`** between score and validate — kept per-file (vendored).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (in-progress, executor handoff)

**Executor:** Claude Code (Opus 4.7) — executing directly (parser content trips
opencode's serializer, per AGENTS.md).

### Update — 2026-05-28 (complete)

**Executor:** Claude Code (Opus 4.7).

**Summary:** Lifted `score.rs` (the weight-table scorer + private `levenshtein` /
`type_matches`) and `validate.rs` (`validate` + `ValidationError` + `TypeMismatch`
+ private `sorted_tool_names` / `type_matches`) into `executor/src/parser/`, wired
via `pub mod score; pub mod validate;`. `type_matches` kept per-file (vendored).
Rexy stage/plan doc references stripped. Test registries built from the real tool
constructors over a `TempDir` `Scope` (no `build_default`); the two justified
`expect`s in `validate` retained, no new unwrap/expect.

**Acceptance criteria:** all met, incl. negatives (name beyond fuzzy distance →
`0`; `validate` of an unknown tool yields no `ToolCall`). Rexy's exact score
integers hold against rexyMCP's `read_file` schema.

**Commands:**

```
cargo fmt --all --check        # clean (after rustfmt on the two files)
cargo build                    # clean, 0 warnings
cargo clippy --all-targets --all-features -- -D warnings   # clean
cargo test                     # 248 passed; 0 failed
```

**End-to-end verification:** N/A — pure library functions exercised by unit tests;
`parse()` (phase-05) composes them, the loop (M4) drives it.

**Files changed:**
- `executor/src/parser/score.rs` — new (scorer + tests)
- `executor/src/parser/validate.rs` — new (validator + error types + tests)
- `executor/src/parser/mod.rs` — `pub mod score; pub mod validate;`

**Grep proof:** `grep -rniE 'stage [0-9]|phase 0' score.rs validate.rs` → 0 hits.

**Notes for review:** executed by Claude Code, not opencode. Not self-approved —
flipped to `review`.

verification: fmt OK · clippy OK · tests 248 passed · build OK
