# Phase 02: Structured output for `execute_phase` / `continue_phase`

**Milestone:** M31 — rmcp v2 Upgrade
**Status:** todo
**Depends on:** phase-01 (the rmcp 2.2 constructors this phase calls are 2.x-only; phase-01 is `done`)
**Estimated diff:** ~200 lines (mostly mechanical derives + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

The eight `#[rmcp::tool]` router tools already return spec-native structured
output through rmcp's `Json<T>` wrapper, but the two hand-rolled tools —
`execute_phase` and `continue_phase` — return their payloads as a JSON
*string* stuffed in a text content block, with no declared output schema.
This phase has them return `structured_content` (with the spec-recommended
back-compat text block, which rmcp emits automatically) and declares typed
output schemas on their `Tool` entries, so the architect gets typed,
spec-visible results instead of parse-this-text.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #31 — names this milestone; phase-02 is the
  structured-output half.
- This milestone's [README](README.md) § "Phase-02 — structured tool output"
  — the design record.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The two call sites (in `RexyMcpServer::call_tool`, `mcp/src/server.rs`)

`execute_phase` tail, `mcp/src/server.rs:714-718`:

```rust
let payload = serde_json::json!({ "run_id": run_id });
let json_str = serde_json::to_string(&payload).map_err(|e| {
    rmcp::ErrorData::internal_error(format!("serialization failed: {}", e), None)
})?;
Ok(CallToolResult::success(vec![ContentBlock::text(json_str)]))
```

`continue_phase` tail, `mcp/src/server.rs:767-771`:

```rust
let json_str = serde_json::to_string(&output.result).map_err(|e| {
    rmcp::ErrorData::internal_error(format!("serialization failed: {}", e), None)
})?;

Ok(CallToolResult::success(vec![ContentBlock::text(json_str)]))
```

### The two hand-built `Tool` entries

Built in **four** places with duplicated name/description/schema arguments:
`list_tools` (`mcp/src/server.rs:784,789`) and `get_tool`
(`mcp/src/server.rs:805,811`), each calling
`rmcp::model::Tool::new(name, description, schema_for_type::<Parameters<..>>())`
with no output schema.

### The types

`PhaseResult` (`executor/src/phase/result.rs:77`) derives
`Serialize`/`Deserialize` but **not** `schemars::JsonSchema`. The executor
crate already depends on `schemars = "1"` (`executor/Cargo.toml:27`) and has a
derive precedent in `executor/src/health.rs`:

```rust
use schemars::JsonSchema;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Health { .. }
```

`mcp/src/server.rs` already imports `schemars::JsonSchema` (line 10) for its
param structs.

## Reference excerpts (verified against the vendored rmcp 2.2.0 source, 2026-07-10)

**`CallToolResult::structured` emits BOTH the structured field and the
back-compat text block** — you do not build the text block yourself
(from `~/.cargo/registry/src/*/rmcp-2.2.0/src/model.rs:3069`):

```rust
pub fn structured(value: Value) -> Self {
    CallToolResult {
        content: vec![ContentBlock::text(value.to_string())],
        structured_content: Some(value),
        is_error: Some(false),
        meta: None,
    }
}
```

**Fallible output-schema generation** (from
`rmcp-2.2.0/src/handler/server/common.rs:110`; available under our existing
`server` feature — the same module as the `schema_for_type` we already call;
result is cached per type):

```rust
pub fn schema_for_output<T: JsonSchema + std::any::Any>() -> Result<Arc<JsonObject>, String>
```

**Attaching a raw output schema** (from `rmcp-2.2.0/src/model/tool.rs:271`):

```rust
pub fn with_raw_output_schema(mut self, output_schema: Arc<JsonObject>) -> Self
```

Do **not** use `Tool::with_output_schema::<T>()` — its body `panic!`s on a
schema-generation error (verified in `tool.rs:323-328`); use the fallible
`schema_for_output` + `with_raw_output_schema` pair as specced in Task 4.

## Spec

### Task 1 — Derive `JsonSchema` on `PhaseResult` and its nested types

Add `schemars::JsonSchema` to the existing `#[derive(..)]` list of exactly
these **13 types** (append the derive; change nothing else — no field, no
serde attribute, no ordering changes; schemars 1.x honors the existing serde
attributes like `rename_all = "snake_case"` automatically). Add
`use schemars::JsonSchema;` to each file's imports (the `health.rs` precedent
above) and write the bare `JsonSchema` name in the derive lists:

- `executor/src/phase/result.rs`: `PhaseStatus` (line 11), `CancelReason`
  (22), `Cancellation` (29), `FileChange` (42), `CommandOutputs` (50),
  `PhaseResult` (77).
- `executor/src/phase/briefing.rs`: `AttemptSummary` (19), `WorkingFile`
  (25), `Blocker` (32), `Briefing` (41).
- `executor/src/governor/hard_fail.rs`: `HardFailSignal` (15).
- `executor/src/governor/verifier.rs`: `Severity` (17), `Diagnostic` (28).

**Edit order is load-bearing — leaf types first.** Deriving `JsonSchema` on a
container type before its field types have it leaves the crate
**non-compiling** (`E0277: the trait bound X: JsonSchema is not satisfied`)
until the whole graph is done, and every non-compiling turn burns a verifier
strike. Apply the derives **bottom-up**, one file per patch, in exactly this
order — the crate compiles after every step:

1. `executor/src/governor/verifier.rs` — `Severity`, `Diagnostic` (leaves).
2. `executor/src/governor/hard_fail.rs` — `HardFailSignal` (leaf).
3. `executor/src/phase/briefing.rs` — `AttemptSummary`, `WorkingFile`,
   `Blocker`, `Briefing` (needs 1 and 2 in place).
4. `executor/src/phase/result.rs` — the six result.rs types (`PhaseResult`
   needs 3 in place).

Then run `cargo build` and confirm it is green **before** starting Task 2.

Do **not** add the derive to `Artifacts` (result.rs:59) or
`DiagnosticSignature` (verifier.rs:48) — neither is part of the serialized
`PhaseResult`. If the compiler reports a further nested type missing a
`JsonSchema` impl, add the same derive to that type and record it in "Notes
for review" — the 13 above were enumerated from the actual field graph at
draft time, so extras should be rare.

### Task 2 — The `SpawnedRun` payload struct

In `mcp/src/server.rs`, near the other param/output structs (around line 51),
add:

```rust
/// The immediate `execute_phase` response — the spawned run's handle,
/// polled to completion via `get_run_status`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct SpawnedRun {
    pub(crate) run_id: String,
}
```

### Task 3 — One result-building helper; rewire the two call sites

Add to `mcp/src/server.rs` (free function, near `execute_phase_inner`):

```rust
/// Build a hand-rolled tool's success result: `structured_content` plus the
/// spec-recommended back-compat text block (`CallToolResult::structured`
/// emits both from one `Value`).
fn structured_result<T: serde::Serialize>(
    value: &T,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let json = serde_json::to_value(value).map_err(|e| {
        rmcp::ErrorData::internal_error(format!("serialization failed: {}", e), None)
    })?;
    Ok(CallToolResult::structured(json))
}
```

Replace the `execute_phase` tail (the exact 5 lines quoted in "Current
state", `server.rs:714-718`) with:

```rust
structured_result(&SpawnedRun { run_id })
```

Replace the `continue_phase` tail (the exact 5 lines quoted in "Current
state", `server.rs:767-771`) with:

```rust
structured_result(&output.result)
```

(`output.result` is the `rexymcp_executor::phase::PhaseResult`; Task 1 gives
it `JsonSchema`, and it already has `Serialize`.)

### Task 4 — Tool-builder helpers with output schemas

In `mcp/src/server.rs`, extract the two duplicated `Tool` constructions into
two free functions, and attach the output schema fallibly (schema generation
is deterministic; on `Err` the tool is returned without an output schema —
the Test-plan pins `Some`, so a regression cannot pass silently):

```rust
fn execute_phase_tool() -> rmcp::model::Tool {
    let tool = rmcp::model::Tool::new(
        "execute_phase",
        "<the existing execute_phase description string, verbatim from server.rs:786>",
        rmcp::handler::server::tool::schema_for_type::<Parameters<ExecutePhaseParams>>(),
    );
    match rmcp::handler::server::tool::schema_for_output::<SpawnedRun>() {
        Ok(schema) => tool.with_raw_output_schema(schema),
        Err(_) => tool,
    }
}

fn continue_phase_tool() -> rmcp::model::Tool {
    let tool = rmcp::model::Tool::new(
        "continue_phase",
        "<the existing continue_phase description string, verbatim from server.rs:791>",
        rmcp::handler::server::tool::schema_for_type::<Parameters<ContinuePhaseParams>>(),
    );
    match rmcp::handler::server::tool::schema_for_output::<rexymcp_executor::phase::PhaseResult>() {
        Ok(schema) => tool.with_raw_output_schema(schema),
        Err(_) => tool,
    }
}
```

Copy the two existing description strings **verbatim** — do not rewrite them.
Then:

- In `list_tools`, replace the two inline `rmcp::model::Tool::new(..)` inserts
  with `tools.insert(0, execute_phase_tool());` and
  `tools.insert(1, continue_phase_tool());`.
- In `get_tool`, replace the two inline constructions with
  `Some(execute_phase_tool())` / `Some(continue_phase_tool())`.

### Task 5 — Tests

Write the Test-plan tests below in the existing test files
(`mcp/src/server_tests.rs` for the server-side ones), run all four gates as
separate invocations, and fill the Update Log.

## Acceptance criteria

- [ ] `grep -n 'CallToolResult::success' mcp/src/server.rs` shows **no**
      remaining uses in the `execute_phase` / `continue_phase` branches of
      `call_tool` (the router fallback and other tools are untouched).
- [ ] The 13 types listed in Task 1 each derive `JsonSchema`; `Artifacts` and
      `DiagnosticSignature` do **not**.
- [ ] `execute_phase`'s `Tool` entry (via both `list_tools` and `get_tool`)
      carries an `output_schema` whose `properties` contain `run_id`.
- [ ] `continue_phase`'s `Tool` entry carries an `output_schema` whose
      `properties` contain `status`.
- [ ] `structured_result` returns a `CallToolResult` where
      `structured_content` is `Some` and `content[0]`'s text parses to the
      identical `serde_json::Value`.
- [ ] No `Cargo.toml` file is edited (schemars is already a dependency of
      both crates). No new dependency.
- [ ] No `#[allow(..)]` added; no `.unwrap()` / `.expect()` / `panic!` in
      production paths (test code exempt).
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes.

## Test plan

In `mcp/src/server_tests.rs` (mirror the shape of
`get_run_status_tool_is_registered`, `server_tests.rs:1005`):

- `execute_phase_tool_declares_run_id_output_schema` — builds a
  `RexyMcpServer` (`make_test_config` helper), asserts
  `server.get_tool("execute_phase")`'s `output_schema` is `Some`, and that
  `schema["properties"]["run_id"]` exists. **Negative pin:** assert
  `schema["properties"]["status"]` does **not** exist (the run-handle schema
  must not be the `PhaseResult` schema).
- `continue_phase_tool_declares_phase_result_output_schema` — same shape;
  `output_schema` is `Some` and `properties` contains `status`.
- `list_tools_carries_output_schemas_for_hand_rolled_tools` — calls the
  `list_tools` path (or `Self::tool_router` + the two helpers if the async
  trait method is awkward to drive; asserting on `execute_phase_tool()` /
  `continue_phase_tool()` directly is acceptable) and asserts both tools'
  `output_schema.is_some()`.
- `structured_result_carries_matching_text_block` — calls
  `structured_result(&SpawnedRun { run_id: "r-1".into() })`; asserts
  `structured_content == Some(json!({"run_id": "r-1"}))`, asserts parsing
  `content[0]`'s text as `serde_json::Value` equals the same value, and
  asserts `is_error == Some(false)`.

In `executor/src/phase/result.rs`'s existing `#[cfg(test)]` module:

- `phase_result_json_schema_generates` — asserts
  `schemars::schema_for!(PhaseResult)` serializes to JSON whose
  `properties` contain `status` and `briefing` (this pins the whole Task-1
  derive graph: schema generation fails to compile if any nested type lacks
  the derive).

## End-to-end verification

The shipped artifact is the rebuilt `rexymcp` binary's `tools/list` and
`tools/call` wire shapes. The executor's obligation:

1. `cargo test -p rexymcp 2>&1 | tail -5` — quote the summary line in the
   completion Update Log.
2. Quote the `execute_phase` `output_schema` JSON (or its `properties` keys)
   from the new test's construction — e.g. add a temporary assertion message
   or run
   `cargo test execute_phase_tool_declares_run_id_output_schema -- --nocapture`
   — to prove the schema actually generates (not just `is_some`).

(The live `rexymcp serve` restart + Claude Code `tools/list` inspection and a
real `execute_phase` structured-content round-trip are **review-time,
architect-side work** — the executor cannot restart the architect's MCP
client. Do not attempt it.)

## Authorizations

- [x] May edit `executor/src/phase/result.rs`, `executor/src/phase/briefing.rs`,
      `executor/src/governor/hard_fail.rs`, `executor/src/governor/verifier.rs`
      — **derive additions (+ the `use schemars::JsonSchema;` import) only**,
      per Task 1.
- [x] May edit `mcp/src/server.rs` and `mcp/src/server_tests.rs` per Tasks 2–5.

No `Cargo.toml` edit is authorized (none is needed — schemars is already a
dependency of both crates). No new dependency.

## Out of scope

- The eight `Json<T>` router tools (`executor_health`, `get_run_status`, …) —
  the rmcp macro wrapper already handles their structured output; do not
  touch them.
- Removing or altering the text content block — `CallToolResult::structured`
  emits it automatically for back-compat; do not add a second one or strip it.
- Adopting MCP tasks (SEP-1686), elicitation, meta/trace helpers, icons, or
  tool titles — surveyed and deferred at the milestone level.
- Changing `cap_phase_result`, the job registry, redaction, or any tool
  description string.
- `Tool::with_output_schema::<T>()` — it panics on schema error; use the
  fallible `schema_for_output` + `with_raw_output_schema` pair as specced.
- Editing serde attributes, field names, or field order on any Task-1 type —
  derives only. The wire format must be byte-identical to before.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-07-10 (READ THIS FIRST — re-dispatch after a mid-cascade hard_fail)

The first run hard-failed (`VerifierFailurePersistent`, 6 strikes) because the
Task-1 derives were applied **top-down**, leaving the crate non-compiling for
6+ consecutive turns. The working tree already carries **correct partial
work — do not redo or revert it**:

- `executor/src/phase/result.rs` — **DONE.** All 6 types derive `JsonSchema`;
  the import is in place. Do not touch this file again.
- `executor/src/phase/briefing.rs` — import + 3 derives in place
  (`AttemptSummary`, `WorkingFile`, `Blocker`). **Only `Briefing` (line ~41)
  is missing the derive** — one edit: append `, JsonSchema` to its
  `#[derive(..)]` list.
- `executor/src/governor/hard_fail.rs` — the `use schemars::JsonSchema;`
  import is in place. **`HardFailSignal` (line ~14) is missing the derive** —
  one edit: append `, JsonSchema`.
- `executor/src/governor/verifier.rs` — **untouched.** Add the
  `use schemars::JsonSchema;` import and the derive on `Severity` (~17) and
  `Diagnostic` (~28).

**Finish Task 1 in this exact order (4 small patches, then build):**
(1) verifier.rs import + `Severity` + `Diagnostic`; (2) hard_fail.rs
`HardFailSignal`; (3) briefing.rs `Briefing`; (4) run `cargo build` — it must
be green before you start Task 2. If it is not green, read the compiler error
and fix only what it names; do not churn.

Then proceed with Tasks 2–5 exactly as specced (all in `mcp/src/server.rs` /
`mcp/src/server_tests.rs` — the mcp half was never started).

### Update — 2026-07-10 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** spec gap, not an executor-capability gap — Task 1 didn't order
the derive cascade, so the crate couldn't compile until the whole graph was
done and the 6-strike verifier limit fired mid-cascade (the known
required-trait-cascade wall). The partial diff on disk is correct as far as it
goes; the refinement enumerates the exact remaining edits bottom-up so every
step compiles.
