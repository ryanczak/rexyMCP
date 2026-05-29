# Phase 03: JSONL session log — writer/reader + event schema

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** done
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

**`Deserialize` round-trip (blocker resolution, 2026-05-28).** `read_session_log`
deserializes records, so `SessionEvent` (and its embedded types) must derive
`Deserialize`, not just `Serialize`. `Diagnostic` already has both. The M3 parser
types (`ToolCall`, `Origin`, `Format`, `RepairOp`, `ParseFailure`, `Candidate` in
`executor/src/parser/mod.rs`) derive only `Serialize` — **add `Deserialize` to all
six** (a one-token addition per type, zero behavior change). This is correct, not a
hack: the session log round-trips these types, so they are genuinely
`Deserialize`-able by design. Authorized below. (Architect note: M3's "derive
intentionally" pinned `Serialize` for the write-side log and didn't anticipate the
read-side — this completes their serde story.)

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
- [x] **May modify `executor/src/parser/mod.rs`** solely to add `Deserialize` to
      the derive list of `ToolCall`, `Origin`, `Format`, `RepairOp`, `ParseFailure`,
      and `Candidate` (blocker resolution above — derive additions only, no other
      change to those types).
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

### Update — 2026-05-28 (started)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Reading Rexy source and preparing to lift `jsonl.rs` and build the net-new event schema.

### Update — 2026-05-28 (blocker)

**Blocked on:** `SessionEvent` requires `Deserialize` on M3 parser types that only derive `Serialize`.

**Detail:** The spec prescribes `#[derive(Debug, Clone, Serialize, Deserialize)]` on `SessionEvent`. The variants `Parsed { tool_call: crate::parser::ToolCall }`, `ParseFailed { failure: crate::parser::ParseFailure }`, and `Verify { diagnostics: Vec<crate::governor::verifier::Diagnostic> }` embed types from M3 phase-02. `Diagnostic` already derives both `Serialize` and `Deserialize`. But `ToolCall`, `ParseFailure`, and their dependencies (`Origin`, `Candidate`, `Format`, `RepairOp`) in `executor/src/parser/mod.rs` only derive `Serialize` — not `Deserialize`.

For `SessionEvent` to compile with `Deserialize`, all six parser types must also derive `Deserialize`. This is a one-word addition per type with zero behavioral change, but it touches settled M3 code outside this phase's authorization.

**What I need:** Authorization to add `Deserialize` to `ToolCall`, `ParseFailure`, `Origin`, `Candidate`, `Format`, and `RepairOp` in `executor/src/parser/mod.rs`, or an alternative approach (e.g., a wrapper type in `store::sessions` that is `Deserialize` and converts to/from the M3 types).

### Update — 2026-05-28 (blocker resolved — architect)

**Resolution:** Granted — **add `Deserialize`** to the six parser types
(`ToolCall`, `Origin`, `Format`, `RepairOp`, `ParseFailure`, `Candidate`). Not the
wrapper-type alternative: a wrapper would duplicate the schema and risk drift,
whereas the session log is *meant* to round-trip these types, so making them
`Deserialize` is the correct completion of their serde story (not a hack). See the
Spec § "`Deserialize` round-trip" and the new Authorization. Derive additions
only — no other change to those types; the M3 parser tests must still pass
unchanged.

Good catch refusing to edit settled M3 code without authorization — that's exactly
the contract. The miss was the spec's (it prescribed `Deserialize` on `SessionEvent`
without ensuring the embedded types supported it). Resume phase-03 with the
authorization above; status stays `in-progress`.

### Update — 2026-05-28 (progress)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Resuming after blocker resolution. Implementing the full phase: adding `Deserialize`
to six parser types, creating `store/sessions/` with `event.rs` (schema) and
`jsonl.rs` (writer/reader + helpers), wiring into `lib.rs`, and writing tests.

### Update — 2026-05-28 (complete)

**Summary:** Created the JSONL session log module with `SessionRecord` + `SessionEvent`
(10 variants, tagged with `event_type`) and `FileNumstat`. Lifted and adapted
`SessionLogger` writer/reader from Rexy: no `chrono`, no `tracing`, no pricing.
Best-effort `session_log` helper discards errors. Added `Deserialize` to six M3
parser types (`ToolCall`, `Origin`, `Format`, `RepairOp`, `ParseFailure`,
`Candidate`) to enable JSON round-trip. All 15 new tests pass, including
per-variant round-trip tests for `Parsed`, `ParseFailed`, `Verify`, and
`Progress`.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
  Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.30s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.49s

cargo test 2>&1 | tail -30
test store::sessions::jsonl::tests::parsed_variant_round_trips_tool_call ... ok
test store::sessions::jsonl::tests::parse_failed_variant_round_trips ... ok
test store::sessions::jsonl::tests::verify_variant_round_trips_diagnostics ... ok
test store::sessions::jsonl::tests::progress_variant_round_trips_numstat ... ok
test store::sessions::jsonl::tests::all_event_variants_serialize_with_event_type ... ok
...
test result: ok. 373 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — a library writer/reader + schema exercised by unit tests. The
loop that emits events (redacted) is phase-07; the M5 query tools read the log
back.

**Files changed:**
- `executor/src/parser/mod.rs` — added `Deserialize` to six type derives
- `executor/src/lib.rs` — added `pub mod store;`
- `executor/src/store/mod.rs` — new, `pub mod sessions;`
- `executor/src/store/sessions/mod.rs` — new, `pub mod event; pub mod jsonl;`
- `executor/src/store/sessions/event.rs` — new, `SessionRecord`, `SessionEvent`, `FileNumstat`
- `executor/src/store/sessions/jsonl.rs` — new, `SessionLogger`, `read_session_log`, `generate_session_id`, `session_log`, `SessionLogHandle`, `open_session_log` + 15 tests

**New tests:**
- `session_event_round_trips_through_json`
- `session_logger_creates_file`
- `session_logger_appends_lines`
- `read_session_log_reads_written_records`
- `read_session_log_handles_partial_last_line`
- `read_session_log_returns_empty_for_missing_file`
- `generate_session_id_is_8_chars_hex`
- `all_event_variants_serialize_with_event_type`
- `session_log_handle_open_and_log`
- `parsed_variant_round_trips_tool_call`
- `parse_failed_variant_round_trips`
- `verify_variant_round_trips_diagnostics`
- `progress_variant_round_trips_numstat`
- `session_log_discards_errors_on_locked_handle`

**Commits:** (pending — will commit below)

**Notes for review:** The `session_log` helper intentionally discards errors
(best-effort logging). No `chrono` or `tracing` added. The `Format` enum
gains `Deserialize` but its `#[serde(rename_all = "snake_case")]` attribute was
already present, so serialization format is unchanged.

verification: fmt OK · clippy OK · tests 373 passed · build OK

### Review verdict — 2026-05-29

- **Verdict:** approved_first_try
- **Bounces:** none (the mid-phase `Deserialize` blocker was an authorization
  request the executor correctly raised before editing settled M3 code, not a
  review rejection)
- **Executor:** opencode (Qwen/Qwen3.6-27B-FP8)
- **Scope deviations:** none — schema matches the prescriptive spec exactly;
  adaptations (no chrono/tracing/pricing, best-effort discard) all honored;
  parser changes are precisely the six authorized `Deserialize` additions
- **Calibration:** one occurrence noted, no fold yet — the spec pinned
  `Deserialize` on `SessionEvent` without ensuring its embedded M3 types
  supported it, surfacing as a blocker. Extends "Derive intentionally"
  (consistency across embedded types), but one data point; fold only if it
  recurs. Architect ran all four gates (fmt/clippy/build clean, 373 passed) and
  spot-checked tests: partial-last-line truncates a real serialized record and
  asserts 3/4 survive; poison test exercises the best-effort discard path; all
  10 variants round-trip carrying `event_type`.
