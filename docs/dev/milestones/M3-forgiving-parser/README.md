# M3 — Forgiving parser

**Goal:** Lift the forgiving tool-call parser — the pipeline that turns a weak
local model's messy output into a validated `ToolCall`, or, when it can't, into
actionable feedback the model can recover from.

**Status:** done — all five phases complete; signed off 2026-05-28 (retrospective below).

**Depends on:** M2 (done — the parser validates candidates against the M2
`ToolRegistry` and reads tool schemas for scoring/repair).

**Exit criteria:**
- `parse(response, &registry) -> ParseResult` composes the full pipeline:
  detect → extract (6 text formats) → score → repair → validate, plus a
  model-feedback formatter for failures.
- `ParseResult` is `NoToolCall | Found(ToolCall) | Failed(ParseFailure)`; a
  malformed-but-recoverable call is repaired and `Found`, an unrecoverable one is
  `Failed` with feedback naming what was wrong.
- Recognizes the six text formats (Hermes `<tool_call>`, fenced JSON, loose JSON,
  YAML, XML `<function=>`, plain-text call syntax) and the bounded repair set
  (name fuzzy-match, param alias, type coerce, default fill, JSON syntax repair,
  newline escape).
- Everything hermetic: pure functions over `&str` + `&ToolRegistry`, no network,
  deterministic.

## Architecture references

- `docs/architecture.md` — "Layer 1 — `executor` crate" lift/drop map (parser
  row: "Forgiving tool-call parser (6-stage pipeline) | Lift").
- `docs/architecture.md` — "The executor turn cycle" step 4 (model output → parser
  → `ToolCall` or a `ParseFailure` fed back as repair guidance) and § "Session
  log" (the parser's `Serialize` types are the M4 log's event schema).
- Rexy source: `rexy/src/agent/parser/` — `mod.rs` (types + `parse` orchestration
  + `strip_think_blocks`), `detect.rs`, `extract/`, `score.rs`, `repair/`,
  `validate.rs`, `feedback.rs`.

## Phases

Expanded on demand (WORKFLOW.md § Milestones), not all at once.

| #  | Phase                                                              | Status |
|----|-------------------------------------------------------------------|--------|
| 01 | parser core types + `strip_think_blocks` + `detect` ([phase-01-types-and-detect.md](phase-01-types-and-detect.md)) | done |
| 02 | the six format extractors ([phase-02-extractors.md](phase-02-extractors.md)) | done |
| 03 | candidate scoring + validation ([phase-03-score-validate.md](phase-03-score-validate.md)) | done |
| 04 | the repair transforms ([phase-04-repair.md](phase-04-repair.md)) | done |
| 05 | feedback formatter + `parse()` orchestration ([phase-05-feedback-parse.md](phase-05-feedback-parse.md)) | done |

## Notes

**This is a lift, re-rooted.** The parser is real Rexy code (~3.3k lines across
`mod.rs` + `detect.rs` + `extract/` + `score.rs` + `repair/` + `validate.rs` +
`feedback.rs`). Lift and adapt:

- **Module location:** `executor/src/parser/` (Rexy's is `src/agent/parser/`;
  rexyMCP has no `agent` parent — root it directly under the `executor` crate).
- **Re-root `crate::` paths:** the registry is `crate::tools::{ToolRegistry,
  Tool, ToolResult}`; errors adapt to `executor::error::Error` where a `Result`
  surfaces (most of the parser returns values, not `Result`).
- **Strip Rexy's plan references.** Rexy's parser doc comments carry stale,
  Rexy-specific pointers ("Stage 2", "the enforcement lives in M2 phase 06", "the
  pipeline (M2 phase 07)", "M2 phase 07"). Those are Rexy's milestone plan, not
  rexyMCP's — drop or rewrite them to rexyMCP's reality. Do not port a comment
  that references a phase/section that doesn't exist here (the same rule that
  dropped the `read-before-edit` TODO in M2 phase-04).

**Types are load-bearing and `Serialize` on purpose.** `ToolCall`, `Origin`,
`Format`, `RepairOp`, `ParseFailure`, `Candidate` are the **event schema for the
M4 JSONL session log** (architecture.md § "Session log"). Keep their `Serialize`
derives (and the `serde(rename_all = "snake_case")` / `tag = "kind"` attributes)
— this is not speculative; the architecture mandates these types as the log's
record shape.

**`native` + `stream` were dropped, not deferred (resolved 2026-05-29 in M4).**
At M3 close these were carried as "deferred to the loop in M4." When M4 reached
the question (the would-be "06b"), inspection showed neither Rexy file fits
rexyMCP's streaming, headless design and both would have zero callers: the OpenAI
backend already extracts native `tool_calls` from the SSE deltas and emits
`AiEvent::ToolCallGeneric` (no `native::extract(&Value)` path; rexyMCP never calls
Anthropic), and `stream.rs`'s `StreamRenderer` is TUI-only (rexyMCP is headless).
The only real gap — constructing a `parser::ToolCall { origin: Origin::Native }`
from a native event for uniform dispatch — folds into the phase-07 loop that
consumes the event. `Origin::Native` is defined in the M3 type set so the schema
is complete. (Rationale recorded in `M4-agent-loop/README.md` § Notes.)

## M3 retrospective (milestone close, 2026-05-28)

Five phases, all `done`; the full forgiving parser (~3.3k lines lifted from Rexy)
landed: `parse(response, &registry) -> ParseResult` composing detect → extract (6
formats) → score → repair (6 transforms, cap 4) → validate → feedback. 303 tests.

**Zero bounces.** Every phase passed review first-try. Lift discipline held: Rexy
stage/plan doc references stripped throughout (grep-verified per phase), the
`Serialize` types kept for the M4 session-log schema, `find_balanced_end` /
`levenshtein` / `type_matches` kept per-file (vendored, not de-duplicated).

**Lift-time defects fixed** (not ported): a production `.unwrap()` in `yaml`
extraction and in `validate`'s name handling (rebound from the `if let`); `yaml`'s
redundant double `detect_region` call and bare `use serde_yaml;`; leftover DEBUG
`eprintln!`s. Adapted every test off Rexy's `build_default` (absent here) to a
real-tools-over-`TempDir`-`Scope` registry.

**Executor note — folded.** All five M3 phases were executed by **Claude Code
directly, not opencode**: the parser's content (close-tags, ```fences, escaped
quotes inside JSON-in-Rust string literals) reliably corrupts opencode's
tool-call serializer — it left phase-01's `mod.rs` syntactically broken before
handoff. This is now a recurring, predictable pattern (5/5 phases), so the
architect-side routing lesson is folded into `WORKFLOW.md` § "Phase progression":
pre-route opencode-hostile content to direct execution rather than
dispatch-then-recover.

**New dependency:** `serde_yaml = "0.9"` (YAML extractor; unmaintained upstream,
acceptable for the lift, swappable later).
