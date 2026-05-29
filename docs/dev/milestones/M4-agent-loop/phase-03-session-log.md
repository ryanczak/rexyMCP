# Phase 03: JSONL session log — writer/reader + event schema

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** todo
**Depends on:** phase-02 (done); reuses M3 parser types + M4 phase-01 `Diagnostic`.
**Estimated diff:** ~480 lines (writer/reader lift + net-new event schema + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

The turn-by-turn **JSONL session log**: an append-only writer + a reader, plus the
**event schema** that records what each turn did. The log is the queryable record
behind the lean `PhaseResult` (architecture § "Session log & troubleshooting
tools" — *pull, not push*: it costs nothing in Claude's context until queried by
the M5 tools).

Two pieces:
- **Writer/reader** — a near-direct lift of `rexy/src/store/sessions/jsonl.rs`
  (`SessionLogger`, `open`/`log`/`path`, `read_session_log`, `generate_session_id`),
  adapted per below.
- **Event schema** — **net-new** for rexyMCP. Rexy's `SessionEvent` is TUI-oriented
  (`ModeChange`, `StateTransition`, …); rexyMCP's is the **turn-cycle** schema,
  reusing the already-`Serialize` types from M3 (`parser::ToolCall`,
  `parser::ParseFailure`) and phase-01 (`verifier::Diagnostic`), and **reserving
  the `Progress` variant** for M5 heartbeats (README Notes § "Progress heartbeats").

## Architecture references

- `docs/architecture.md` — "Session log & troubleshooting tools" (format, location,
  pull-not-push); "The executor turn cycle" (every step's event appended).
- Rexy source: `rexy/src/store/sessions/jsonl.rs` (writer/reader) and
  `rexy/src/observability/session_event.rs` (Rexy's event enum — **reference only**,
  rexyMCP's schema is redesigned).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M4 README Notes (esp. "Redaction is its own
   phase" and "Timestamps without `chrono`").
2. Read this entire phase doc.
3. Confirm phase-02 is `done`; `executor::parser::{ToolCall, ParseFailure}` and
   `executor::governor::verifier::Diagnostic` exist and are `Serialize`.
4. **Read** `rexy/src/store/sessions/jsonl.rs` and (for reference)
   `rexy/src/observability/session_event.rs`.

## Spec

Create `executor/src/store/mod.rs` + `executor/src/store/sessions/mod.rs` +
`executor/src/store/sessions/{event.rs, jsonl.rs}` (or inline the schema in
`jsonl.rs` — your structural call). Wire `pub mod store;` into `lib.rs`.

### Event schema (net-new) — `SessionRecord` + `SessionEvent`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub ts: u64,        // unix epoch millis, set by the caller (NOT chrono / Utc::now)
    pub turn: usize,
    pub event: SessionEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum SessionEvent {
    SessionStart { session_id: String, model: String, phase: String },
    Prompt { rendered: String },                 // the assembled turn/system prompt
    Completion { raw: String },                  // raw model output for the turn
    Parsed { tool_call: crate::parser::ToolCall },           // reuse M3 type
    ParseFailed { failure: crate::parser::ParseFailure },    // reuse M3 type (carries RepairOp history + feedback)
    ToolResult { name: String, succeeded: bool, output_preview: String },
    Verify { diagnostics: Vec<crate::governor::verifier::Diagnostic> },  // reuse phase-01 type
    HardFail { reason: String },
    Progress { turn: usize, stage: String, files_changed: Vec<FileNumstat>, message: String },
    SessionEnd { status: String, turns: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNumstat { pub path: String, pub added: u32, pub removed: u32 }
```

This schema is **prescriptive** (load-bearing — M5's query tools and the M4
`PhaseRun` reference it). Use `#[serde(tag = "event_type", rename_all =
"snake_case")]` so each line carries a discriminant (Rexy's tests grep for
`"event_type"`). Reuse the M3/phase-01 types directly — do not redefine `ToolCall`
/ `ParseFailure` / `Diagnostic`.

### Writer/reader — lift `jsonl.rs`

- `SessionLogger::open(log_dir, session_id) -> io::Result<Self>` (creates dir,
  `session-<id>.jsonl`, append mode); `log(&SessionRecord) -> io::Result<()>`
  (serialize one line + flush); `path()`.
- `read_session_log(path) -> io::Result<Vec<SessionRecord>>` — tolerant: missing
  file → empty `Vec`; a malformed/partial last line is skipped (lift Rexy's
  line-by-line `from_str().ok()` behavior).
- `generate_session_id() -> String` (8-char hex; lift — `SystemTime` for the ID is
  fine, it's not a logged timestamp).
- `SessionLogHandle = Arc<Mutex<SessionLogger>>` + `open_session_log` + a
  best-effort `session_log(handle, ts, turn, event)` helper.

**Adaptations:**

1. **No `chrono`.** `SessionRecord.ts: u64` (epoch millis), supplied by the
   caller. The best-effort `session_log` helper takes `ts` as a parameter; it must
   **not** call a clock (determinism — the loop injects the time in phase-07).
2. **No `observability` module / no `tracing`.** Put the schema under
   `store::sessions` (not Rexy's `observability::session_event`). Rexy's
   best-effort helper does `tracing::warn!` on a write error; rexyMCP has no
   `tracing` — instead **intentionally discard** the error with a one-line comment
   explaining logging is best-effort (architecture: "logging is a side effect…
   never changes what the loop returns"). `log()` itself still returns
   `io::Result`; only the best-effort wrapper discards.
3. **Drop the pricing/cost machinery.** Rexy's `token_usage_event` looks up
   `ModelPricing` + `compute_cost`. rexyMCP has no pricing here — omit it. Token
   metrics live in the `PhaseRun` telemetry phase (08); this phase needs no
   `TokenUsage` variant.
4. **The writer is redaction-agnostic** — it writes whatever record it's given.
   Redaction is phase-04, applied upstream. Do **not** add redaction here.

## Acceptance criteria

- [ ] `executor/src/store/...` exists with `SessionRecord` + `SessionEvent`
      (variants above) + `FileNumstat`; `pub mod store;` in `lib.rs`.
- [ ] `SessionEvent` reuses `parser::ToolCall` / `parser::ParseFailure` /
      `verifier::Diagnostic` (not redefined); each serialized line carries
      `"event_type"`.
- [ ] `SessionLogger::open` creates `session-<id>.jsonl`; `log` appends one JSON
      line per record; `read_session_log` round-trips written records.
- [ ] `read_session_log` returns empty `Vec` for a missing file and **skips a
      malformed/partial last line** (negative case — pin it).
- [ ] `ts` is `u64` and caller-supplied; no `chrono`, no `Utc::now()`, no
      `tracing` dependency added.
- [ ] every `SessionEvent` variant round-trips through JSON (incl. `Parsed`,
      `ParseFailed`, `Verify`, `Progress`).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, `tempfile::TempDir` for the log dir; deterministic `ts` values (fixed
`u64`s, never a clock). Lift Rexy's writer/reader tests (file creation, append,
round-trip, partial-last-line, missing-file, 8-char-hex id), adapting `ts` to
`u64` and the event variants to rexyMCP's set. Add a round-trip test for the
M3-type-bearing variants (`Parsed`/`ParseFailed`/`Verify`) and `Progress`.

## End-to-end verification

> Not applicable — a library writer/reader + schema exercised by unit tests. The
> loop that emits events (redacted) is phase-07; the M5 query tools read the log
> back.

## Authorizations

- [x] **May create** `executor/src/store/**`; **may modify** `executor/src/lib.rs`
      (`pub mod store;`).
- [ ] **No new dependencies** (no `chrono`, no `tracing`).
- [ ] May **NOT** implement redaction (phase-04), the loop, `PhaseRun` telemetry,
      or pricing/cost.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **Redaction** — phase-04 (applied upstream; this writer is redaction-agnostic).
- **The loop emitting events / injecting `ts`** — phase-07.
- **Token cost / pricing** — `PhaseRun` telemetry, phase-08.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
