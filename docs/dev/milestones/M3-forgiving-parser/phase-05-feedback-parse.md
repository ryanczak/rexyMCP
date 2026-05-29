# Phase 05: feedback formatter + parse() orchestration

**Milestone:** M3 — Forgiving parser
**Status:** review
**Depends on:** phase-04 (done)
**Estimated diff:** ~700 lines (feedback + parse orchestration + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Close the parser pipeline: lift the **feedback** formatter (turn a
`ValidationError` into a model-readable "name the wrong value, suggest the fix"
message) and add the **`parse()` orchestration** that composes every stage —
detect → extract → score → repair → validate → feedback — into a single
`parse(response, &registry) -> ParseResult`.

This is the **last phase in M3**. Its approval is a milestone boundary (human
gate: retrospective + any doc-folding).

## Architecture references

- `docs/architecture.md` — lift/drop map (parser row); turn cycle step 4 (parser →
  `ToolCall` or `ParseFailure` fed back as repair guidance).
- Rexy source: `rexy/src/agent/parser/feedback.rs` and the `parse` fn +
  `strip_think_blocks` in `rexy/src/agent/parser/mod.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M3 README Notes.
2. Read this entire phase doc.
3. Confirm phase-04 is `done`; `detect`, `extract`, `score`, `validate`, `repair`,
   and the core types all exist in `executor::parser`.
4. **Read** `rexy/src/agent/parser/feedback.rs` and the `parse` fn in `mod.rs`.

## Spec

- **`executor/src/parser/feedback.rs`** (new) — lift `format_failure(_best,
  &ValidationError, &registry) -> String` and `format_no_match(&str) -> String`,
  with the private `format_unknown_tool` / `format_missing_required` /
  `format_type_mismatch` / `format_unknown_params` / `closest_tool` /
  `quote_value` / `suggest_fix` / `levenshtein`. Priority order: unknown-tool >
  missing-required > type-mismatch > unknown-params. Wire `pub mod feedback;`.
- **`parse()` in `executor/src/parser/mod.rs`** — `pub fn parse(response: &str,
  registry: &crate::tools::ToolRegistry) -> ParseResult`. detect the formats; run
  each through its extractor; if no candidates → `NoToolCall`; score + sort
  descending; for each candidate clone → `repair::apply` → `validate`, returning
  `Found` on the first success; otherwise build the `Failed(ParseFailure)` with
  `format_failure` on the best (highest-scoring) repaired candidate.

**Adaptations:**

1. **`format_failure`'s `best` param is unused** (as in Rexy) — name it `_best`
   (`-D warnings` flags an unused param). Keep it in the signature for API
   stability / richer feedback later.
2. **`parse()`'s `.expect(...)`** is permitted: candidates are non-empty (checked)
   and every candidate was validated without returning, so an error is recorded —
   give the `expect` a message stating that invariant (STANDARDS §2.1).
3. **Strip Rexy stage/plan references** ("Stage 6 follow-up", etc.).
4. **Test registry — no `build_default`.** feedback + parse tests build the
   registry from the real tool constructors over a `TempDir` `Scope`. The parse
   tests + the existing `strip_think_blocks` tests share `mod.rs`'s one tests
   module.

## Acceptance criteria

- [ ] `executor/src/parser/feedback.rs` exists (wired `pub mod feedback;`);
      `parse()` exists in `parser/mod.rs`.
- [ ] `parse` returns `NoToolCall` for plain prose; `Found` for a valid hermes /
      fenced / loose-json call; `Found` after a repair (`read_fil`→`read_file`);
      `Failed` (with feedback) for an unknown tool, a missing-required, and an
      empty object (`lacked a name field`); picks the highest-scoring candidate
      when several are present; sets `detected_format` on failure.
- [ ] End-to-end: a `<think>…</think>` + `<tool_call>{bash}` response, after
      `strip_think_blocks`, parses to `Found(bash)` with the right arguments.
- [ ] `format_failure` names the offending value and suggests a fix, with the
      documented priority order.
- [ ] No Rexy stage/plan references survive (grep `Stage [0-9]`, `phase 0` → 0).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic; lift Rexy's feedback tests + the `parse` tests from `mod.rs`, swapping
`build_default` for the `TempDir`-`Scope` `test_registry()`. Pin the end-to-end
round-trips and the `Failed`-path feedback contents.

## End-to-end verification

> N/A as a runtime artifact — `parse()` is a library entry point exercised by unit
> tests (incl. full-pipeline round-trips). The agent loop that calls it is M4.

## Authorizations

- [x] **May create** `executor/src/parser/feedback.rs`; **may modify**
      `executor/src/parser/mod.rs` (`pub mod feedback;` + `parse()` + parse tests).
- [ ] **No new dependencies.**
- [ ] May **NOT** add `native`/`stream` (M4) or `tools::build_default`.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **`native` (backend tool_calls) + `stream`** — M4 (couple to the AI client).
- **The agent loop** that calls `parse()` and feeds `feedback` back — M4.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-28 (in-progress, executor handoff)

**Executor:** Claude Code (Opus 4.7) — executing directly (parser content trips
opencode's serializer, per AGENTS.md).

### Update — 2026-05-28 (complete)

**Executor:** Claude Code (Opus 4.7).

**Summary:** Lifted `feedback.rs` (`format_failure` + `format_no_match` + the
private formatters and `levenshtein`) into `executor/src/parser/`, and added the
`parse()` orchestration to `parser/mod.rs` (composes detect → extract → score →
sort → repair+validate loop → feedback), wired `pub mod feedback;`. This closes
the pipeline: `parse(response, &registry) -> ParseResult`.

**Adaptations:** `format_failure`'s unused `best` param marked `_best` (unused in
Rexy too; `-D warnings` would flag it). `levenshtein` kept per-file (vendored).
`parse()`'s `.expect()` is justified (candidates non-empty ⇒ an error was
recorded; message states the invariant). Rexy stage/plan doc references stripped.
Test registries built from the real tools over a `TempDir` `Scope`; the parse
tests + strip_think tests share the one tests module in `mod.rs`.

**Acceptance criteria:** all met. End-to-end pipeline tests pass, including the
vLLM/Qwen3 `<think>`+`<tool_call>` round-trip → `Found(bash)`, highest-scoring
candidate selection, repair-then-validate (`read_fil`→`read_file`), and the
`Failed` paths (unknown tool, missing required, empty object → feedback).

**Commands:**

```
cargo fmt --all --check        # clean (after rustfmt on the two files)
cargo build                    # clean, 0 warnings
cargo clippy --all-targets --all-features -- -D warnings   # clean
cargo test                     # 303 passed; 0 failed
```

**End-to-end verification:** N/A as a runtime artifact — `parse()` is exercised by
unit tests (incl. full-pipeline round-trips). The agent loop that calls it is M4.

**Files changed:**
- `executor/src/parser/feedback.rs` — new (formatter + tests)
- `executor/src/parser/mod.rs` — `pub mod feedback;`, `parse()`, parse tests

**Grep proof:** `grep -rniE 'stage [0-9]|phase 0' feedback.rs mod.rs` → 0 hits.

**Notes for review:** executed by Claude Code, not opencode. Not self-approved —
flipped to `review`. **This is the last M3 phase**: its approval is a milestone
boundary (human gate — retrospective + any doc-folding).

verification: fmt OK · clippy OK · tests 303 passed · build OK
