# Phase 03: session-log query tools (executor_log_search / executor_log_tail / get_turn)

**Milestone:** M5 — MCP server
**Status:** review
**Depends on:** M5 phase-02 (done) — extends the same `#[rmcp::tool_router]` on `RexyMcpServer`. M4 phase-03 (the JSONL log + `read_session_log` + `SessionRecord` schema with `Serialize+Deserialize` already in place).
**Estimated diff:** ~450 lines (log_query module + cap extension + three tool handlers + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the three **read-back-the-log** MCP tools so Claude can drill into a phase's
inner transcript on demand, **without re-flooding context** — the pull-not-push
half of the MCP boundary:

- **`executor_log_search(log_path, event_type?, tool_name?, query_text?, limit?)`** —
  grep/filter the JSONL log by event kind, tool name, or substring; return
  matching records, capped per-record and limited in count.
- **`executor_log_tail(log_path, n?)`** — the last `n` records, each capped
  per-field.
- **`get_turn(log_path, turn)`** — all records for one turn number, **uncapped**
  — architecture-mandated: "the one place the raw detail is allowed through,
  scoped to a single turn."

These read the log Claude already knows about from `PhaseResult.log_path`
(returned by `execute_phase` in phase-02). They're the deep-dive complement to
the briefing — the briefing handles the common case, the log is there when the
compression lost the detail Claude needs.

## Architecture references

- `docs/architecture.md` — "Session log & troubleshooting tools" (the three
  query tools, per-tool output capping, `get_turn` as the one uncapped escape
  hatch); Layer 2 ("`execute_phase` result reports the log path so Claude can
  reference it"); Status §M5.
- M5 README Notes — "Output capping is the boundary's whole point".
- M4: `store::sessions::jsonl::read_session_log`, `store::sessions::event::
  {SessionRecord, SessionEvent, FileNumstat}` (all `Serialize+Deserialize`).
- M5 phase-02: the `RexyMcpServer` tool router, `cap::cap_string` /
  `MAX_FIELD_BYTES`, the `pub(crate)` inner-fn factoring pattern.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README Notes.
2. Read this entire phase doc.
3. Confirm M5 phase-02 is `done`; `RexyMcpServer` + `#[rmcp::tool_router(...)]`
   are in `mcp/src/server.rs`; `cap::cap_string` + `MAX_FIELD_BYTES` are public
   in `mcp/src/cap.rs`.
4. Confirm `read_session_log(path) -> std::io::Result<Vec<SessionRecord>>`
   exists and is forgiving (missing file → `Ok(empty)`; malformed lines silently
   skipped — both confirmed in `executor/src/store/sessions/jsonl.rs`).
5. Confirm `SessionRecord` / `SessionEvent` / `FileNumstat` derive both
   `Serialize` and `Deserialize`. They do (M4 phase-03 + the architect's
   `Deserialize` resolution). **No `executor/` edit is needed in this phase.**

## Spec

### 1. New module — `mcp/src/log_query.rs`

A pure module (no rmcp, no I/O beyond the one `read_session_log` call) holding
the filter + capping logic. Declared `mod log_query;` in `mcp/src/main.rs`.

```rust
// Per-tool result-count caps. Each tool advertises its own default + max so a
// debugging query can't re-flood Claude's context.
pub const SEARCH_DEFAULT_LIMIT: usize = 20;
pub const SEARCH_MAX_LIMIT: usize = 50;
pub const TAIL_DEFAULT_N: usize = 10;
pub const TAIL_MAX_N: usize = 50;

pub struct SearchFilter<'a> {
    pub event_type: Option<&'a str>,   // e.g. "tool_result" — matches the
                                       // serialized `event_type` discriminant
    pub tool_name: Option<&'a str>,    // substring match (case-sensitive)
    pub query_text: Option<&'a str>,   // substring match across the record's
                                       // serialized JSON
}

pub fn search(records: &[SessionRecord], filter: &SearchFilter, limit: usize)
    -> Vec<SessionRecord>;

pub fn tail(records: &[SessionRecord], n: usize) -> Vec<SessionRecord>;

pub fn get_turn(records: &[SessionRecord], turn: usize) -> Vec<SessionRecord>;
```

**Filter semantics:**

- `event_type`: an exact match against the serialized discriminant string
  (snake_case — matches what `#[serde(tag = "event_type", rename_all =
  "snake_case")]` writes: `"session_start"`, `"prompt"`, `"completion"`,
  `"parsed"`, `"parse_failed"`, `"tool_result"`, `"verify"`, `"hard_fail"`,
  `"progress"`, `"session_end"`). Compare against the SessionEvent variant via
  a small helper `event_type_str(&SessionEvent) -> &'static str` (a 10-arm
  match — straightforward).
- `tool_name`: substring match against the tool name in `Parsed{tool_call.name}`
  and `ToolResult{name}` events. Non-matching events fail the filter
  unconditionally when `tool_name` is `Some` (i.e. the filter requires the
  event *can* carry a tool name).
- `query_text`: substring match against the record's serialized JSON
  (`serde_json::to_string(record).contains(query_text)`). Simple and uniform
  across all event kinds. Case-sensitive (Claude can lowercase if it wants
  case-insensitive).

All three filters AND together — every `Some` filter must match. All `None` →
no filtering (returns first `limit` records).

`limit` is clamped to `SEARCH_MAX_LIMIT`; `n` to `TAIL_MAX_N`. A caller-supplied
0 is treated as "use default".

`get_turn` returns **every** record where `record.turn == turn` (a turn produces
multiple events: Prompt, Completion, Parsed, ToolResult, …). Order preserved
from input.

### 2. Capping extension — `mcp/src/cap.rs`

Add a new public fn:

```rust
pub fn cap_session_record(record: SessionRecord) -> SessionRecord;
```

That truncates the long-string fields *inside* the `SessionEvent` variants to
`MAX_FIELD_BYTES` via the existing `cap_string`. Long fields per variant:

- `Prompt { rendered }` — the assembled system prompt; routinely large.
- `Completion { raw }` — the model's full raw output; can be huge.
- `ToolResult { output_preview }` — already a "preview" by name, but unbounded
  upstream; cap defensively.
- `HardFail { reason }` — usually small but uncapped upstream; cap defensively.
- `Progress { message }` — the heartbeat numstat blob; cap defensively.

Other variants (`SessionStart`, `Parsed`, `ParseFailed`, `Verify`, `SessionEnd`)
have only bounded fields and are pass-through. Keep `cap_string` private
(file-local) — `cap_session_record` is the new pub entrypoint alongside
`cap_phase_result`. Or, if cleaner, **promote `cap_string` to `pub(crate)`** so
both `cap_phase_result` and `cap_session_record` share it without duplication
(authorized — this is a same-module visibility tweak).

`cap_session_record` is applied to records returned by `executor_log_search` and
`executor_log_tail`, and **not** by `get_turn` (the architecture-mandated
uncapped escape hatch).

### 3. Three tool handlers — extend `mcp/src/server.rs`

Follow the phase-02 pattern verbatim: a `pub(crate)` inner fn per tool plus a
thin `#[rmcp::tool]` method on `RexyMcpServer` wrapping it (the inner fns are
what tests call).

```rust
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutorLogSearchParams {
    pub log_path: String,
    pub event_type: Option<String>,
    pub tool_name: Option<String>,
    pub query_text: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExecutorLogTailParams {
    pub log_path: String,
    pub n: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetTurnParams {
    pub log_path: String,
    pub turn: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LogQueryOutput {
    /// The matching records as a JSON array. Each record is a serialized
    /// SessionRecord. Wrapped in serde_json::Value so SessionRecord doesn't
    /// need JsonSchema (mirrors ExecutePhaseOutput's approach — see phase-02).
    pub records: serde_json::Value,
    /// True when the result was clipped by a per-tool count cap, so Claude
    /// knows to refine its query if it cares.
    pub truncated: bool,
}

pub(crate) fn executor_log_search_inner(params: &ExecutorLogSearchParams)
    -> Result<LogQueryOutput, String>;

pub(crate) fn executor_log_tail_inner(params: &ExecutorLogTailParams)
    -> Result<LogQueryOutput, String>;

pub(crate) fn get_turn_inner(params: &GetTurnParams)
    -> Result<LogQueryOutput, String>;
```

Handler logic for each:

1. **Resolve the path** from `params.log_path` (parse as `PathBuf`; no
   normalization, no scope confinement — see Adaptation 4).
2. **Call `read_session_log(&path)`** — propagate IO errors as `Err(String)`.
   A missing file returns `Ok(empty)` from `read_session_log` (handled by
   upstream); pass that through as an empty result (not an error).
3. **Filter / select / get** via the `log_query` fns.
4. **Cap** the per-record contents (search + tail only; `get_turn` skips this).
5. **Serialize** the `Vec<SessionRecord>` to `serde_json::Value` via
   `serde_json::to_value`, wrap in `LogQueryOutput { records, truncated }`,
   return.
6. **`truncated`** is `true` iff the count cap fired (matched records >
   `limit`/`n` — or, for `get_turn`, always `false` since no count cap).

Register the three tools in the existing `#[rmcp::tool_router(server_handler)]`
impl alongside `execute_phase` / `executor_health`. Tool names: literal
`"executor_log_search"`, `"executor_log_tail"`, `"get_turn"` (verify rmcp's
`#[tool]` macro infers from method name — it does in phase-02).

### 4. `mcp/src/main.rs` — no new subcommand

The log-query tools are pull-not-push debugging surfaces; there's no manual CLI
caller value here (unlike phase-01's `run-phase` which is a real manual entry
point). Declare `mod log_query;` if not already (it will be), nothing else.

## Adaptations / decisions

1. **`LogQueryOutput` wraps `Value`** — same trade-off as phase-02's
   `ExecutePhaseOutput`. Avoids cascading `JsonSchema` derives across
   `SessionRecord` / `SessionEvent` / `FileNumstat` / `ToolCall` / `ParseFailure`
   / `Diagnostic` / `FileNumstat`. The cost: Claude sees `{ "records": [...] }`
   instead of `[...]`. Acceptable; the `truncated` flag pays for the wrapper.
2. **No JsonSchema on executor types** — direct consequence of (1). **Zero
   `executor/` edits this phase.** (Cross-boundary trait bounds were the
   recurring pattern noted in phase-02's verdict; here we sidestep them via the
   wrapper, which is the cleaner option when the schema tree is large.)
3. **Substring filter, not regex.** Regex would pull in another dep and the
   query language is supposed to be simple — Claude can iterate filters if a
   first pass is too broad. Document this in the tool description.
4. **No path confinement on `log_path`.** The architect (Claude) is the trusted
   caller — not the local model. The log lives under
   `<repo>/.rexymcp/sessions/`, but the tool accepts any path Claude passes (it
   already gets the path from `PhaseResult.log_path`). This is consistent with
   the executor's `Scope` confining the *model*, not the architect. Document in
   the tool description.
5. **`get_turn` is uncapped per-field** but bounded by single-turn-record set.
   The architecture is explicit on this; do not double-guess.
6. **Promote `cap_string` to `pub(crate)`** so both `cap_phase_result` and
   `cap_session_record` share it (no duplication). Authorized.

## Acceptance criteria

- [ ] `mcp/src/log_query.rs` exists; `mod log_query;` is wired in
      `mcp/src/main.rs`; `search` / `tail` / `get_turn` + `SearchFilter` +
      `event_type_str` + the four `*_LIMIT` / `*_MAX` constants are reachable.
- [ ] `event_type_str(&SessionEvent)` returns the exact snake_case discriminant
      for all 10 variants (`session_start`, `prompt`, `completion`, `parsed`,
      `parse_failed`, `tool_result`, `verify`, `hard_fail`, `progress`,
      `session_end`).
- [ ] `search` filters: `event_type` exact match, `tool_name` substring on
      `Parsed{tool_call.name}` and `ToolResult{name}` (other variants fail
      `tool_name` filter), `query_text` substring on `serde_json::to_string`.
      All `Some` filters AND together. All `None` → first `limit` records.
- [ ] `search` clamps `limit` to `SEARCH_MAX_LIMIT` (50); a 0 input uses
      `SEARCH_DEFAULT_LIMIT` (20). `tail` clamps `n` to `TAIL_MAX_N`; a 0 uses
      `TAIL_DEFAULT_N`.
- [ ] `tail` returns the last `n` records in original order.
- [ ] `get_turn` returns **all** records where `record.turn == turn`, in
      original order; no field capping applied.
- [ ] `cap_session_record` truncates `Prompt{rendered}`, `Completion{raw}`,
      `ToolResult{output_preview}`, `HardFail{reason}`, `Progress{message}` via
      `cap_string`; other variants pass through unchanged. UTF-8 char-boundary
      safety inherited from `cap_string`.
- [ ] `mcp/src/server.rs` registers three new tools named exactly
      `"executor_log_search"`, `"executor_log_tail"`, `"get_turn"`, each with a
      `pub(crate)` `*_inner` fn factored out per the phase-02 pattern.
- [ ] Each `*_inner` fn: missing file → `Ok(empty)` pass-through (not `Err`);
      IO read error → `Err(String)`; success → `LogQueryOutput { records,
      truncated }` where `truncated` reflects the count-cap firing
      (`get_turn` always `false`).
- [ ] **Handler success-path tests** (the phase-02 calibration note): each of
      the three handlers has a test that writes a small fixture JSONL log to a
      `TempDir`, calls the `*_inner` fn, and asserts the returned `records`
      contains the expected serialized records. *Not just error paths.* (Error
      paths still tested too: malformed path / unreadable file.)
- [ ] `executor_log_search` handler test exercises each of the three filters
      (event_type, tool_name, query_text) at least once.
- [ ] `executor_log_tail` handler test exercises both the default-`n` and the
      clamped-to-max-`n` cases.
- [ ] `get_turn` handler test asserts a turn with multiple events returns all
      of them (not just one) and that field content is **uncapped** (a huge
      string passes through).
- [ ] No `#[allow]`; no `unwrap()` / `expect()` / `panic!()` in production
      paths (test code exempt); no Rexy phase references in new files.
- [ ] **No new dependency.** **No `executor/` edits.** (If you find a real
      need for either, **stop and file a blocker** — do not add silently.)
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic. In `mcp/src/log_query.rs` `#[cfg(test)] mod tests`:

- **`event_type_str`** — round-trip every variant against the snake_case names.
- **`search`** — fixture vector of records spanning every event kind:
  - no filters → first N records
  - event_type only → only matching kind
  - tool_name only → matches Parsed + ToolResult by substring; rejects others
  - query_text only → substring grep
  - combined filters (AND)
  - `limit` clamping (50, 0 → default, 10)
- **`tail`** — last-N (default, max-clamped, zero-default, more-than-available)
- **`get_turn`** — single-turn-many-events; turn with no records → empty Vec;
  turn 0 valid

In `mcp/src/cap.rs` `#[cfg(test)] mod tests` (extend):
- `cap_session_record` truncates `Prompt`/`Completion`/`ToolResult.output_preview`/
  `HardFail.reason`/`Progress.message`; pass-through for `SessionStart`/`Parsed`/
  `Verify`/`SessionEnd`/`ParseFailed`; UTF-8 boundary inherited.

In `mcp/src/server.rs` `#[cfg(test)] mod tests` (extend):
- **Success-path tests for all three handlers** (phase-02 lesson) — write a
  tiny fixture log to a `TempDir`, invoke each `*_inner`, assert the JSON.
- Error-path tests: nonexistent file path → handlers vary (`read_session_log`
  returns `Ok(empty)` for missing — so a missing file is *not* an error from
  the tool's perspective; document this in the description; the test asserts
  empty records). A genuinely unreadable file (e.g. a directory passed where a
  file is expected) → `Err(String)`.
- **`get_turn` uncapped assertion:** write a fixture record with a `Prompt {
  rendered }` of 100k bytes; `get_turn` returns it intact; `executor_log_tail`
  on the same record returns it capped.

## End-to-end verification

> Not applicable yet — same as phase-02. The handler logic is exercised by
> unit tests over `TempDir` fixtures; the rmcp transport is M6 dogfood.

## Authorizations

- [x] **May create** `mcp/src/log_query.rs`; **may modify** `mcp/src/server.rs`
      (three new param structs, one shared `LogQueryOutput`, three new tool
      methods + `pub(crate)` inner fns), `mcp/src/cap.rs` (add
      `cap_session_record`, promote `cap_string` to `pub(crate)`),
      `mcp/src/main.rs` (declare `mod log_query;`).
- [ ] **No new dependencies.** The `mcp` crate already has `serde`,
      `serde_json`, `schemars`, `rmcp` (phase-02).
- [ ] **No `executor/` edits.** `SessionRecord` / `SessionEvent` / `FileNumstat`
      already derive `Serialize+Deserialize`; the `Value` wrapper sidesteps
      `JsonSchema`. If a genuine need arises, **stop and file a blocker.**
- [ ] May **NOT** add `model_scorecard` (phase-04), progress notifications
      (phase-05), or roots corroboration (phase-06).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `AGENTS.md`, `mcp/src/runner.rs`, or any other phase doc.
- [ ] **Calibration carry-forward from phase-02 (mandatory):** *declare every
      scope deviation in "Notes for review"*, even defensible ones (and
      especially when adding a dep or touching `executor/`). The phase-02 verdict
      praised this explicitly — keep the muscle.

## Out of scope

- **`model_scorecard`** — phase-04.
- **Progress notifications** (the live MCP heartbeats + progress-callback seam
  in the loop) — phase-05. Note: `Progress` *log records* already flow through
  `executor_log_search` etc. via the standard event-type filter; reading them
  back is in scope here, *emitting* them is not.
- **Roots corroboration** — phase-06.
- **Regex queries** — substring only (Adaptation 3).
- **Pagination / offset / `before_turn`-style cursors** — not needed at this
  scale; `limit` + filters cover the use cases. If M6 dogfood proves
  pagination is wanted, add it then.
- **Auto-deriving the log path from `(repo, phase, session_id)`** — Claude has
  `PhaseResult.log_path` already; no need for a second resolution path here.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-30 12:00 (started)

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8)

Starting phase-03: session-log query tools. Will implement `log_query` module, extend `cap.rs` with `cap_session_record`, add three tool handlers to `server.rs`, and wire `mod log_query;` in `main.rs`.

### Update — 2026-05-30 12:30 (complete)

**Summary:** Built three session-log query MCP tools (`executor_log_search`, `executor_log_tail`, `get_turn`) plus supporting `log_query` module and `cap_session_record`. All spec tasks implemented in order: (1) `mcp/src/log_query.rs` with `SearchFilter`, `search`, `tail`, `get_turn`, `event_type_str`, and limit constants; (2) `cap_session_record` in `cap.rs` with `cap_string` promoted to `pub(crate)`; (3) three tool handlers in `server.rs` with param structs, `LogQueryOutput`, and `pub(crate)` inner fns; (4) `mod log_query;` wired in `main.rs`. No scope deviations.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.92s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.24s

cargo test 2>&1 | grep "test result:"
test result: ok. 71 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 495 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The handler logic is exercised by unit tests over `TempDir` fixtures; the rmcp transport is M6 dogfood.

**Files changed:**
- `mcp/src/log_query.rs` — new module: `SearchFilter`, `event_type_str`, `search`, `tail`, `get_turn`, limit constants, + 18 unit tests
- `mcp/src/cap.rs` — promoted `cap_string` to `pub(crate)`, added `cap_session_record` + 12 tests
- `mcp/src/server.rs` — added `ExecutorLogSearchParams`, `ExecutorLogTailParams`, `GetTurnParams`, `LogQueryOutput`, three `*_inner` fns, three `#[rmcp::tool]` methods, + 13 handler tests
- `mcp/src/main.rs` — declared `mod log_query;`

**New tests:**
- `event_type_str_round_trips_all_variants`, `search_no_filters_returns_first_limit_records`, `search_event_type_filter`, `search_tool_name_filter_matches_parsed_and_tool_result`, `search_tool_name_filter_rejects_non_tool_events`, `search_query_text_substring`, `search_combined_filters_and`, `search_limit_clamped_to_max`, `search_limit_zero_uses_default`, `search_limit_small`, `tail_returns_last_n_in_order`, `tail_default_n_when_zero`, `tail_clamped_to_max`, `tail_more_than_available_returns_all`, `get_turn_returns_all_events_for_turn`, `get_turn_empty_when_no_records`, `get_turn_turn_zero` (in `log_query.rs`)
- `cap_session_record_truncates_prompt_rendered`, `cap_session_record_truncates_completion_raw`, `cap_session_record_truncates_tool_result_output_preview`, `cap_session_record_truncates_hard_fail_reason`, `cap_session_record_truncates_progress_message`, `cap_session_record_passes_through_session_start`, `cap_session_record_passes_through_parsed`, `cap_session_record_passes_through_parse_failed`, `cap_session_record_passes_through_verify`, `cap_session_record_passes_through_session_end`, `cap_session_record_short_fields_untouched` (in `cap.rs`)
- `executor_log_search_returns_matching_records`, `executor_log_search_filter_by_tool_name`, `executor_log_search_filter_by_query_text`, `executor_log_search_returns_empty_for_missing_file`, `executor_log_tail_returns_last_n_records`, `executor_log_tail_default_n`, `executor_log_tail_clamped_to_max`, `executor_log_tail_returns_empty_for_missing_file`, `get_turn_returns_all_events_for_turn`, `get_turn_empty_when_no_records`, `get_turn_returns_empty_for_missing_file`, `get_turn_uncapped_vs_tail_capped`, `executor_log_search_directory_path_returns_error` (in `server.rs`)

**Commits:**
- `fdeb2e4` — feat: add session-log query MCP tools (executor_log_search / executor_log_tail / get_turn)

**Notes for review:** No scope deviations. `cap_string` promoted to `pub(crate)` as authorized. Zero `executor/` edits. No new dependencies.

verification: fmt OK · clippy OK · tests 566 passed · build OK
