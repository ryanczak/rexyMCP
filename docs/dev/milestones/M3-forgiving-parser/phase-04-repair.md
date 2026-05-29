# Phase 04: the repair transforms

**Milestone:** M3 — Forgiving parser
**Status:** review
**Depends on:** phase-03 (done)
**Estimated diff:** ~1450 lines (orchestrator + six transforms + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Lift the repair pipeline — the stage that takes a malformed/near-miss `Candidate`
and tries to fix it before validation. An orchestrator (`repair::apply`) runs six
deterministic, ordered transforms under a total cap of **4** applications per
candidate, each appending a `RepairOp` to `candidate.repairs_attempted`.

The six transforms, in contract order: **name** fuzzy-match → **alias** (param
rename) → **coerce** (string→typed) → **default_fill** → **json** (syntax repair)
→ **strings** (newline-escape).

## Architecture references

- `docs/architecture.md` — lift/drop map (parser row, "Lift"); turn cycle step 4.
- Rexy source: `rexy/src/agent/parser/repair/` — `mod.rs` + `name.rs`, `alias.rs`,
  `coerce.rs`, `default_fill.rs`, `json.rs`, `strings.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M3 README Notes.
2. Read this entire phase doc.
3. Confirm phase-03 is `done`; `executor::parser::{Candidate, Format, RepairOp}`
   and `executor::tools::{ToolRegistry, Tool, ToolResult}` exist.
4. **Read** each `rexy/src/agent/parser/repair/*.rs`. Lift faithfully (vendored).

## Spec

Create `executor/src/parser/repair/` with `mod.rs` + the six transform files; wire
`pub mod repair;` into `parser/mod.rs`. Each transform is `pub fn apply(candidate:
&mut Candidate, registry: &ToolRegistry, budget: usize) -> usize` (returns the
number applied, `0` if `budget == 0`). The orchestrator:

```rust
const CAP: usize = 4;
pub fn apply(candidate: &mut Candidate, registry: &ToolRegistry) {
    let mut applied = 0;
    applied += name::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += alias::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += coerce::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += default_fill::apply(candidate, registry, CAP.saturating_sub(applied));
    applied += json::apply(candidate, registry, CAP.saturating_sub(applied));
    let _ = strings::apply(candidate, registry, CAP.saturating_sub(applied));
}
```

Per-file notes (lift verbatim except where stated):

- **`name.rs`** — fuzzy-match the tool name (Levenshtein ≤ 2, lexicographic
  tiebreak) against the registry; skip if exact match exists. Keeps its own
  `levenshtein` (per-file, vendored — same as `score.rs`).
- **`alias.rs`** — `ALIAS_TABLE = [(cmd,command),(file_path,path),(filename,path),
  (query,pattern)]`; renames an arg key to its canonical when the tool's schema
  has the canonical property, the alias is present, and the canonical isn't. (The
  table maps onto rexyMCP's tools: `command`→bash, `path`→read_file, `pattern`→
  search/find_files.)
- **`coerce.rs`** — string→declared-type (`integer`/`number`/`boolean`/`object`/
  `array`) per the schema.
- **`default_fill.rs`** — fill missing **non-required** params that declare a
  `default`. (Its tests use an inline mock tool; keep them.)
- **`json.rs`** — repair raw_body syntax (trailing commas, unquoted keys, single
  quotes, unclosed braces), re-parse, repopulate. Exposes
  `pub(crate) fn populate_from_parsed(&mut Candidate, &Value)` used by `strings`.
  No-op when `arguments` already `Some` or `format == PlainText`.
- **`strings.rs`** — escape literal `\n`/`\r`/`\t` inside string literals in
  raw_body, re-parse, repopulate via `json::populate_from_parsed`. `use
  super::json::populate_from_parsed;`.

**Adaptations:**

1. **Strip Rexy stage/plan references** ("Stage 5", "transform 5/6", "phase 05's
   three transforms", "phase 06's transforms run as no-ops"). Rewrite the cap doc
   to: "Once the cap is reached, later transforms run as no-ops." No phase refs.
2. **Test registry — no `build_default`.** In `mod.rs`, `name.rs`, `alias.rs`,
   `coerce.rs`, replace Rexy's `build_default(&Config::default())` with the
   real-tools-over-`TempDir`-`Scope` `test_registry()` (same helper used in
   phase-03). `default_fill.rs` (inline mock tool) and `json.rs`/`strings.rs`
   (`ToolRegistry::new()`) need no registry change.
3. `populate_from_parsed` stays `pub(crate)`; `type_matches`/`levenshtein`
   duplication across files stays (vendored — do not extract shared helpers).

## Acceptance criteria

- [ ] `executor/src/parser/repair/{mod,name,alias,coerce,default_fill,json,strings}.rs`
      exist; `pub mod repair;` declared in `parser/mod.rs`.
- [ ] `repair::apply` composes the six transforms in order under a cap of 4
      (a candidate needing name+alias+coerce×2 stops at 4, not more).
- [ ] Each transform: respects `budget` (incl. `budget == 0` → `0`); appends the
      correct `RepairOp`; is a no-op when nothing applies.
- [ ] name fuzzy-matches `read_fil`→`read_file`; alias renames `file_path`→`path`;
      coerce turns `"42"`→`42` for an integer field; default_fill fills an absent
      optional-with-default; json repairs a trailing comma; strings escapes a
      literal newline inside a string value.
- [ ] **Negatives:** name leaves an exact match untouched and skips a too-far
      name; alias skips when the canonical is already present; coerce skips an
      already-correct type; default_fill skips required params and present params;
      json/strings are no-ops when `arguments` is already `Some` or format is
      `PlainText`.
- [ ] No Rexy stage/plan references survive (grep `Stage [0-9]`, `phase 0`,
      `transform [0-9]` → 0).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic; lift each file's Rexy tests, swapping `build_default` for
`test_registry()` where used. Pin behavior incl. the negatives and the cap.

## End-to-end verification

> Not applicable — pure library functions exercised by unit tests; `parse()`
> (phase-05) composes them. Restate in the completion entry.

## Authorizations

- [x] **May create** `executor/src/parser/repair/` (mod + six files); **may
      modify** `executor/src/parser/mod.rs` (`pub mod repair;`).
- [ ] **No new dependencies.**
- [ ] May **NOT** lift `feedback` or `parse()`; may **NOT** add
      `tools::build_default`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **feedback / `parse()`** — M3 phase-05.
- **Sharing `levenshtein` / `type_matches` / the alias table via config** — kept
  per-file / hardcoded (vendored).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (in-progress, executor handoff)

**Executor:** Claude Code (Opus 4.7) — executing directly (parser content trips
opencode's serializer, per AGENTS.md).

### Update — 2026-05-28 (complete)

**Executor:** Claude Code (Opus 4.7).

**Summary:** Lifted the repair pipeline into `executor/src/parser/repair/`:
`mod.rs` (orchestrator, ordered transforms, `CAP = 4`) + the six transforms
(`name`, `alias`, `coerce`, `default_fill`, `json`, `strings`). Wired `pub mod
repair;` into `parser/mod.rs`. `populate_from_parsed` kept `pub(crate)` (shared by
`json` + `strings`); `levenshtein`/`type_matches` duplication kept per-file
(vendored). Rexy stage/plan doc references stripped.

**Adaptations:** test registries in `mod`/`name`/`alias`/`coerce` built from the
real tool constructors over a `TempDir` `Scope` (no `build_default`);
`default_fill` keeps its inline mock tool; `json`/`strings` use
`ToolRegistry::new()`. The `ALIAS_TABLE` (cmd→command, file_path/filename→path,
query→pattern) maps onto rexyMCP's tools as-is. `strings` tests use `\n`/`\r`/`\t`
escapes (equivalent to Rexy's multi-line source literals).

**Acceptance criteria:** all met, incl. the cap (name+alias+coerce×2 = 4) and the
negatives (exact-name skip, alias-target-present skip, already-correct-type skip,
required/present default-fill skips, json/strings no-op when args present or
PlainText).

**Commands:**

```
cargo fmt --all --check        # clean (after rustfmt on the repair files)
cargo build                    # clean, 0 warnings
cargo clippy --all-targets --all-features -- -D warnings   # clean
cargo test                     # 285 passed; 0 failed
```

**End-to-end verification:** N/A — pure library functions exercised by unit tests;
`parse()` (phase-05) composes them.

**Files changed:**
- `executor/src/parser/repair/{mod,name,alias,coerce,default_fill,json,strings}.rs` — new
- `executor/src/parser/mod.rs` — `pub mod repair;`

**Grep proof:** `grep -rniE 'stage [0-9]|phase 0|transform [0-9]'
executor/src/parser/repair/` → 0 hits.

**Notes for review:** executed by Claude Code, not opencode. Not self-approved —
flipped to `review`.

verification: fmt OK · clippy OK · tests 285 passed · build OK
