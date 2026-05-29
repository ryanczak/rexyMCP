# Phase 01: parser core types + strip_think_blocks + detect

**Milestone:** M3 — Forgiving parser
**Status:** review
**Depends on:** M2 (done)
**Estimated diff:** ~300 lines (type definitions + two lifted functions + tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

Lay the parser module's foundation: the **core types** every later M3 phase
imports, plus the two self-contained pipeline pieces that have no other
dependencies — `strip_think_blocks` (drop `<think>…</think>` reasoning) and
`detect` (sniff which text formats a response might contain). After this phase the
`executor::parser` module exists with its full type vocabulary and the detection
stage; extraction/scoring/repair/validation/feedback and the `parse()`
orchestration come in later phases.

These are **load-bearing, prescriptive types** — every subsequent phase and the M4
session log depend on their exact shape — so this spec pins them precisely (the
WORKFLOW.md exception for "load-bearing types every later phase imports").

## Architecture references

- `docs/architecture.md` — "The executor turn cycle" step 4 (parser produces a
  `ToolCall` or a `ParseFailure`) and § "Session log" (these `Serialize` types are
  the M4 log's event schema).
- Rexy source: `rexy/src/agent/parser/mod.rs` (types + `strip_think_blocks`) and
  `rexy/src/agent/parser/detect.rs` (the `detect` function).

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read the M3 README — especially the Notes on re-rooting and stripping Rexy's
   stale plan references.
3. Read this entire phase doc.
4. Confirm M2 is `done`; `executor::tools::{ToolRegistry, Tool, ToolResult}` exist
   and the workspace builds clean. `regex` and `serde` are already workspace deps.
5. **Read** `rexy/src/agent/parser/mod.rs` (lines 1-149 for the types +
   `strip_think_blocks`) and `rexy/src/agent/parser/detect.rs`. Lift these; do not
   lift `parse()` (the orchestration — it needs later-phase stages) or the other
   submodules.

## Current state

- No `executor/src/parser/` module yet. `executor/src/lib.rs` declares `ai`,
  `config`, `error`, `health`, `security`, `tools`.
- `regex` (workspace) and `serde` (workspace, `derive`) are available.

## Spec

### 1. Module — `executor/src/parser/mod.rs` (new)

Lift the core types from `rexy/src/agent/parser/mod.rs` **verbatim in shape**
(re-rooted, Rexy plan-comments stripped):

- `pub fn strip_think_blocks(s: &str) -> String` — strip `<think>…</think>` spans;
  an **unterminated** `<think>` discards everything from the open tag onward; a
  single `\n` immediately after `</think>` is consumed. (Keep the *why* doc
  comment — it explains a non-obvious behavior.)
- `pub struct ToolCall { pub name: String, pub arguments: Value, pub origin: Origin }`
- `pub enum Origin { Native, Extracted { format: Format }, Repaired { format: Format, repairs: Vec<RepairOp> } }`
- `pub enum Format { Hermes, FencedJson, LooseJson, Yaml, XmlVariant, PlainText }`
  — derive `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize` with
  `#[serde(rename_all = "snake_case")]`.
- `pub enum RepairOp { NameFuzzyMatch { from, to }, ParamAlias { from, to }, TypeCoerce { field, from_type, to_type }, DefaultFill { field }, JsonRepair, NewlineEscape }`
  — derive `Debug, Clone, PartialEq, Serialize` with
  `#[serde(rename_all = "snake_case", tag = "kind")]`.
- `pub struct ParseFailure { pub raw: String, pub detected_format: Option<Format>, pub candidates: Vec<Candidate>, pub feedback: String }`
- `pub struct Candidate { pub format: Format, pub name: Option<String>, pub arguments: Option<Value>, pub score: i32, pub repairs_attempted: Vec<RepairOp>, pub raw_body: Option<String> }`
- `pub enum ParseResult { NoToolCall, Found(ToolCall), Failed(ParseFailure) }`

Keep the `Serialize` derives on `ToolCall`, `Origin`, `Format`, `RepairOp`,
`ParseFailure`, `Candidate` — they are the M4 session-log event schema (M3 README
Notes). `ParseResult` does not need `Serialize` (keep its `Debug`).

Declare the detection submodule: `pub mod detect;`. **Do not** declare `extract`,
`score`, `repair`, `validate`, `feedback`, `native`, `stream`, or define `parse()`
— those are later phases.

**Strip Rexy's plan references** while lifting the doc comments: e.g.
"the enforcement lives in M2 phase 06", "sent back … in M2 phase 07", "Stage 2 of
the parser pipeline". Rewrite to rexyMCP's reality or drop. No comment may
reference a phase/section that doesn't exist here.

### 2. Detection — `executor/src/parser/detect.rs` (new)

Lift `detect` from `rexy/src/agent/parser/detect.rs`:

- `pub fn detect(response: &str) -> Vec<Format>` — lexically sniff for each
  format's marker and return the formats worth attempting, **in this fixed
  priority order**: `Hermes` (`<tool_call>`), `XmlVariant` (`<function=`),
  `FencedJson` (` ```json `), `Yaml` (` ```yaml ` or the `name:`/`arguments:`
  block regex), `LooseJson` (balanced, non-zero `{`…`}` counts). `PlainText` (the
  `\w+\(\w+\s*=` call regex) fires **only when no other format did**.
- Keep the private helpers (`has_balanced_braces`, the two `OnceLock<Regex>`
  patterns). `regex` is available.

### 3. Wiring — `executor/src/lib.rs`

Add `pub mod parser;`.

## Acceptance criteria

- [ ] `executor/src/parser/mod.rs` defines `ToolCall`, `Origin`, `Format` (six
      variants), `RepairOp` (six variants), `ParseFailure`, `Candidate`,
      `ParseResult` with the derives/serde attributes above; `strip_think_blocks`
      is present; `pub mod detect;` is declared; `lib.rs` has `pub mod parser;`.
- [ ] `executor/src/parser/detect.rs` defines `pub fn detect(&str) -> Vec<Format>`.
- [ ] `strip_think_blocks`: removes a `<think>…</think>` block and a single
      following newline; passes plain text through unchanged; an **unterminated**
      `<think>` drops everything from the tag onward; preserves a `<tool_call>`
      that follows a closed think block.
- [ ] `detect` returns the documented format for each marker and the fixed
      priority order when several markers are present.
- [ ] **Negative cases** (pin these, not just the positives): `detect` returns an
      **empty** `Vec` for plain prose with no markers; `PlainText` does **not**
      fire when any structured marker is present; `LooseJson` does **not** fire on
      unbalanced braces (e.g. `"a { b"`).
- [ ] No comment references a Rexy phase/section that doesn't exist in rexyMCP
      (grep the two files for `phase 0`, `Stage 2` — zero hits).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, pure-function unit tests (no registry needed — `detect` and
`strip_think_blocks` take only `&str`). Pin behavior, not test count.

`strip_think_blocks`:
- single block + trailing newline removed; multiple blocks; plain passthrough;
  unterminated block drops the tail; a `<tool_call>` after a closed block survives.

`detect` (positives):
- each marker → its `Format` (`<tool_call>`→Hermes, `<function=`→XmlVariant,
  ` ```json `→FencedJson, the yaml block→Yaml, balanced braces in prose→LooseJson,
  `call foo(path=x)`→PlainText);
- fixed priority order when several fire (e.g. Hermes before FencedJson before
  LooseJson).

`detect` (**negatives** — required):
- plain prose with no markers → empty `Vec`;
- a structured marker present ⇒ result does **not** contain `PlainText`;
- unbalanced braces (`"a { b c"`) ⇒ result does **not** contain `LooseJson`.

## End-to-end verification

> Not applicable — this phase ships pure library types + two pure functions,
> exercised directly by unit tests. The `parse()` orchestration that composes them
> lands in M3 phase-05; the loop that calls `parse()` is M4. Restate this in the
> completion entry.

## Authorizations

- [x] **May create** `executor/src/parser/mod.rs` and
      `executor/src/parser/detect.rs`; **may modify** `executor/src/lib.rs` to
      declare the module.
- [ ] **No new dependencies** (`regex`, `serde` already present).
- [ ] May **NOT** lift `parse()`, the extractors, score, repair, validate,
      feedback, `native`, or `stream` — those are later M3 phases / M4.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      any other phase doc.

## Out of scope

- **`parse()` orchestration** — M3 phase-05 (needs all stages).
- **Extractors / score / repair / validate / feedback** — M3 phases 02-05.
- **`native` (backend tool_calls) and `stream` (streaming accumulation)** — M4
  (they couple to the AI client / loop). `Origin::Native` is defined here so the
  schema is complete, but nothing constructs it yet.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 12:00 (progress)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Lifting core types and `strip_think_blocks` from `rexy/src/agent/parser/mod.rs` (lines 1-149) and `detect` from `rexy/src/agent/parser/detect.rs` into `executor/src/parser/`. Stripping Rexy-specific plan references from doc comments. Re-rooting `crate::` paths. Adding tests per the phase spec.

### Update — 2026-05-28 (progress, executor handoff)

**Executor:** Claude Code (Opus 4.7) — taken over from opencode.

opencode's `write` left `mod.rs` corrupted: the `'\n'` char literal in
`strip_think_blocks` was serialized as a real newline inside the source (a syntax
error), and the type definitions were dropped mid-file — the documented opencode
tool-call serializer fragility (AGENTS.md § "Writing files when the opencode tool
harness fails"), triggered here by the parser's close-tag / fence / escaped-quote
content. Re-implemented the phase with Claude Code's tooling.

### Update — 2026-05-28 (complete)

**Executor:** Claude Code (Opus 4.7).

**Summary:** Created `executor/src/parser/mod.rs` (core types — `ToolCall`,
`Origin`, `Format`, `RepairOp`, `ParseFailure`, `Candidate`, `ParseResult` — with
the pinned derives + serde attributes, and `strip_think_blocks`) and
`executor/src/parser/detect.rs` (`detect`), and declared `pub mod parser;` in
`lib.rs`. Faithful lift from Rexy, re-rooted; no `crate::tools` dependency yet
(`parse()` deferred). Rexy's plan references ("Stage 2", "M2 phase 06/07",
`session.messages`) stripped/rewritten. `Serialize` kept on the session-log types.

**Acceptance criteria:** all met. Negative cases pinned per the spec (detect →
empty on prose; PlainText suppressed when a structured marker is present;
LooseJson suppressed on unbalanced braces).

**Commands:**

```
cargo fmt --all --check        # clean (after rustfmt on the two new files)
cargo build                    # clean, 0 warnings
cargo clippy --all-targets --all-features -- -D warnings   # clean
cargo test                     # 195 passed; 0 failed
```

**End-to-end verification:** N/A — pure library types + two pure functions,
exercised by unit tests; `parse()` orchestration is M3 phase-05, the loop is M4.

**Files changed:**
- `executor/src/parser/mod.rs` — new: core types + `strip_think_blocks` + 5 tests
- `executor/src/parser/detect.rs` — new: `detect` + 10 tests (incl. 2 negatives)
- `executor/src/lib.rs` — `pub mod parser;`

**Grep proof (spec-pinned):** `grep -niE 'phase 0|stage [0-9]|session\.messages'
executor/src/parser/` → 0 hits (no Rexy plan references survived the lift).

**Notes for review:** executed by Claude Code, not opencode, because the parser
content trips opencode's serializer (see the handoff entry). I have **not**
self-approved — flipped to `review` for architect sign-off.

verification: fmt OK · clippy OK · tests 195 passed · build OK
