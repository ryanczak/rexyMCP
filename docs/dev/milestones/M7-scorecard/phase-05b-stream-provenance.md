# Phase 05b: chat-stream provenance — served model id + finish_reason

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** todo
**Depends on:** phase-05a (done — settings are now configurable/sent/recorded) and
phase-04 (done — `rexymcp runs` displays the record). This phase captures two values
the chat response already carries but the client currently discards.
**Estimated diff:** ~370 lines (AiEvent::Done extension + SSE parse + RunMetrics
aggregation + two `PhaseRun` fields + `rexymcp runs` display + tests).
**Tags:** language=rust, kind=feature, size=l

## Goal

Capture two pieces of **endpoint-reported provenance** that the OpenAI-compatible
chat stream already sends on every completion but the client throws away:

1. **Served model id** — the top-level `model` field in the chat response. This is
   what the endpoint *actually* ran, which can differ from the requested
   `[executor] model` (aliasing, fallback, a vLLM `--served-model-name`). Recorded
   as `PhaseRun.served_model`.
2. **`finish_reason`** — per completion (`"stop"` / `"length"` / `"tool_calls"`).
   The load-bearing signal is the **fraction of completions that ended in
   `"length"`** (the model hit `max_tokens` and was truncated mid-output) — a
   reliability signal that sits alongside `parse_failure_rate`. Recorded as
   `PhaseRun.length_finish_rate`.

Both are surfaced in `rexymcp runs`. This is the chat-stream half of the original
phase-05b. The **context window** (`max_model_len` from `/v1/models`) is a separate
source and is **phase-05c** — out of scope here (see Out of scope).

## Architecture references

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` record's reliability axis. `length_finish_rate` joins
  `parse_failure_rate` as an objective reliability signal; `served_model` records
  the identity axis accurately.
- M7 README § Phases, the 05b bullet: served model id (response `model`),
  `finish_reason` (esp. the `length`-truncation fraction). **Out of scope:**
  quantization / parameter count — not portably exposed by the OpenAI API.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc — **including Task 6, the full `AiEvent::Done`
   cascade** — before touching code. This phase changes one enum variant that has
   **seven** call sites; phase-05a's first attempt hard-failed by changing a
   signature without updating every caller. Task 6 lists all seven. Do them all,
   then `cargo build` before `cargo test`.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### `AiEvent::Done` carries only tokens (`executor/src/ai/types.rs:136`)

```rust
#[derive(Debug)]
pub enum AiEvent {
    Token(String),
    ToolCallGeneric { id: String, name: String, args: serde_json::Value, thought_signature: Option<String> },
    Done(TokenBreakdown),
    Error(String),
}
```

`Done` must grow to carry the finish_reason and served model alongside usage.

### The SSE parse loop discards `model` and `finish_reason` (`executor/src/ai/backends/openai.rs:190`)

Inside `OpenAiClient::chat`, each SSE chunk `v` is parsed. Today the loop reads
`v["choices"][0]["delta"]` (content/tool_calls) and `v["usage"]`, but never
`v["model"]` (top-level, present on every chunk) or
`v["choices"][0]["finish_reason"]` (null until the final chunk, then a string). The
relevant existing extraction, for reference:

```rust
if let Ok(v) = serde_json::from_str::<Value>(data) {
    if let Some(delta) = v["choices"].get(0).and_then(|c| c["delta"].as_object()) {
        // ... content / tool_calls ...
    }
    if let Some(u) = v.get("usage").and_then(|u| u.as_object()) {
        usage = parse_openai_usage(u);
    }
}
```

The final send is `let _ = tx.send(AiEvent::Done(usage));` (openai.rs:327, after the
stall loop).

### The agent loop folds `Done` into `RunMetrics` (`executor/src/agent/mod.rs:320`, `:1112`)

```rust
// consumption (agent/mod.rs:320):
AiEvent::Done(breakdown) => metrics.add_tokens(&breakdown),

// the aggregator (agent/mod.rs:1112):
struct RunMetrics {
    parse_attempts: usize,
    parse_failures: usize,
    total_repairs: usize,
    total_calls: usize,
    verifier_retries: usize,
    tokens: TokenBreakdown,
    start_ms: u64,
}
```

`emit_phase_run` (agent/mod.rs:1153) builds the `PhaseRun` from `metrics`; the
construction is at agent/mod.rs:1189.

### `PhaseRun` — the record to extend (`executor/src/store/telemetry.rs:37`)

`PhaseRun` derives `Serialize`/`Deserialize` with **no** struct-level
`#[serde(default)]`. The store already holds historical records, and
`mcp/src/server.rs` tests deserialize hand-written JSONL fixtures **without** any
new fields. Therefore **new fields MUST be `Option<_>` and carry `#[serde(default)]`
per field**, so old records (and those fixtures) still deserialize. (See the
existing `#[serde(default)]` idiom in `config.rs`.)

### `rexymcp runs` rendering (`mcp/src/runs.rs:59` `format_runs`)

`format_runs` builds the human table; `select`/`load_runs` are unaffected. `--json`
serializes the whole `PhaseRun`, so the new fields appear there automatically once
added.

## Spec

### Task 1 — Extend `AiEvent::Done` to a struct variant (`executor/src/ai/types.rs`)

```rust
    Done {
        usage: TokenBreakdown,
        /// Endpoint `finish_reason` for the completion (`stop`/`length`/`tool_calls`/…). `None` if the backend omitted it.
        finish_reason: Option<String>,
        /// Served model id from the chat response `model` field. `None` if absent.
        model: Option<String>,
    },
```

### Task 2 — Parse `model` + `finish_reason` from the SSE stream (`executor/src/ai/backends/openai.rs`)

In `OpenAiClient::chat`, add two locals next to `usage` (e.g.
`let mut served_model: Option<String> = None;` and
`let mut finish_reason: Option<String> = None;`). Inside the chunk-parse block,
capture them (last-wins is fine):

```rust
if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
    served_model = Some(m.to_string());
}
if let Some(fr) = v["choices"].get(0).and_then(|c| c.get("finish_reason")).and_then(|f| f.as_str()) {
    finish_reason = Some(fr.to_string());
}
```

Change the terminal send to the struct form:

```rust
let _ = tx.send(AiEvent::Done { usage, finish_reason, model: served_model });
```

### Task 3 — Aggregate in `RunMetrics` (`executor/src/agent/mod.rs`)

Add fields to `RunMetrics` and initialize them in `started_at`:

```rust
    served_model: Option<String>,
    length_finishes: usize,
    total_finishes: usize,
```

Update the `Done` consumption arm (agent/mod.rs:320) to fold in the new data
**and** keep the token accounting:

```rust
AiEvent::Done { usage, finish_reason, model } => {
    metrics.add_tokens(&usage);
    if let Some(m) = model {
        metrics.served_model = Some(m);
    }
    if let Some(fr) = finish_reason {
        metrics.total_finishes += 1;
        if fr == "length" {
            metrics.length_finishes += 1;
        }
    }
}
```

### Task 4 — Record the two fields in `PhaseRun` (`executor/src/store/telemetry.rs` + the emit site)

Add to `PhaseRun` (each with `#[serde(default)]` — see Current state for why this is
mandatory):

```rust
    #[serde(default)]
    pub served_model: Option<String>,
    #[serde(default)]
    pub length_finish_rate: Option<f64>,
```

In `emit_phase_run` (agent/mod.rs:1189 construction), populate them from `metrics`:

```rust
    served_model: metrics.served_model.clone(),
    length_finish_rate: (metrics.total_finishes > 0)
        .then(|| metrics.length_finishes as f64 / metrics.total_finishes as f64),
```

### Task 5 — Surface them in `rexymcp runs` (`mcp/src/runs.rs`)

In `format_runs`, add to each run's line (and the header) the **served model**
(render the `served_model` string, or `—` when `None`) and the **length-finish
rate** (render as a percentage, e.g. `12%`, or `—` when `None`). Pin **behavior,
not spacing**: both values must appear per row; column layout is the executor's
call. The empty/`(no runs)` path is unchanged.

### Task 6 — Complete the `AiEvent::Done` cascade (worked examples) — do ALL of these

Changing the `Done` variant breaks every construction and the two consumption
sites. Update **all seven**, then `cargo build` to confirm zero `E0599`/`E0026`/
pattern errors. Find them with
`grep -rn "AiEvent::Done\|Done(" executor mcp` and cross-check against this list:

- **Construction — production:** `openai.rs` terminal send → already covered in
  Task 2 (the struct form).
- **Construction — `executor/src/ai/testing.rs:259`** (`MockAiClientEvents` sample):
  `AiEvent::Done { usage: TokenBreakdown::default(), finish_reason: None, model: None }`.
- **Construction — `executor/src/agent/mod.rs:1636`, `:1752`, `:3126`** (loop tests):
  same struct form. For `:3126` (currently `AiEvent::Done(TokenBreakdown { … })`),
  keep the populated `TokenBreakdown` as the `usage:` field and add
  `finish_reason: None, model: None` (or a real `finish_reason` if a test asserts on
  it — see Test plan).
- **Consumption — `executor/src/agent/mod.rs:320`** → the Task 3 struct-match arm.
- **Consumption — `executor/src/ai/testing.rs:275`** (`AiEvent::Done(_) => {}`) →
  `AiEvent::Done { .. } => {}`.

Then update every **`PhaseRun { … }` struct literal** to include the two new fields
(Rust requires all fields in a literal even when `Option`/defaulted). Find them with
`grep -rn "PhaseRun {" executor mcp` — the sites are `telemetry.rs` `sample()`,
`agent/mod.rs` `emit_phase_run` (Task 4, the real one), `mcp/src/scorecard.rs` test
helper, and `mcp/src/runs.rs` `make_run` / `make_run_with_params`. Add
`served_model: None, length_finish_rate: None` to the test/helper literals (the real
emit site uses the Task 4 values). **The hand-written JSONL string fixtures in
`mcp/src/server.rs` do NOT need changes** — `#[serde(default)]` lets them
deserialize without the new fields; that is the whole reason the fields are
defaulted.

**Completion check:** `cargo build` clean, then `grep -rn "AiEvent::Done" executor`
shows only struct-form uses, and `cargo test` compiles (all `PhaseRun` literals
have the fields).

## Acceptance criteria

- [ ] `AiEvent::Done` is a struct variant carrying `usage`, `finish_reason:
      Option<String>`, `model: Option<String>`; all seven call sites updated.
- [ ] The SSE parser captures the response `model` and `finish_reason` and sends
      them in `Done`.
- [ ] `RunMetrics` aggregates a served model and a `length`-finish count/total; a
      run that saw one `finish_reason: "length"` out of two completions records
      `length_finish_rate == Some(0.5)`; a run that saw no finish_reasons records
      `None` (must-NOT divide by zero).
- [ ] `PhaseRun` gains `served_model: Option<String>` and `length_finish_rate:
      Option<f64>`, both `#[serde(default)]`; a legacy JSONL line **without** these
      fields still deserializes (must-NOT break existing records).
- [ ] `rexymcp runs` surfaces both values (served model + length-finish rate) per
      row, rendering `—` when absent.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

Hermetic unit tests; mirror existing patterns.

In `executor/src/ai/backends/openai.rs` tests — if SSE-chunk parsing is only
reachable through `chat()` (which needs a live stream), instead assert the capture
logic at whatever seam is unit-testable; if not cleanly reachable, cover it via the
agent-loop test below and note it in "Notes for review". (Do **not** add a live
test.)

In `executor/src/agent/mod.rs` tests, using `MockAiClientEvents` to script `Done`:

- `length_finish_rate_is_fraction_of_length_finishes` — script two completions, one
  `Done { finish_reason: Some("length"), .. }` and one `Some("stop")`; assert the
  emitted `PhaseRun.length_finish_rate == Some(0.5)`. (Assert via the emitted record
  or the `RunMetrics` fold, whichever the loop exposes.)
- `length_finish_rate_none_when_no_finish_reasons` — all `Done { finish_reason:
  None, .. }`; assert `length_finish_rate == None` (must-NOT be `Some(0.0)`; the
  divide-by-zero boundary).
- `served_model_recorded_from_done` — a `Done { model: Some("served-x"), .. }` →
  emitted `PhaseRun.served_model == Some("served-x")`.

In `executor/src/store/telemetry.rs` tests:

- `phase_run_without_provenance_fields_deserializes` — a JSONL line lacking
  `served_model`/`length_finish_rate` deserializes with both `None` (pins the
  `#[serde(default)]` back-compat — the must-NOT-break-old-records boundary).

In `mcp/src/runs.rs` tests:

- `format_runs_shows_served_model_and_truncation` — a run with
  `served_model: Some("qwen-served"), length_finish_rate: Some(0.25)` renders both
  (the model string and a `25%`-style rate) in the output; a run with both `None`
  renders the chosen `—` sentinel for each. Assert **presence**, not spacing.

## End-to-end verification

1. Build a 2-line `phase_runs.jsonl`: one record with
   `served_model`/`length_finish_rate` set, one **legacy** line omitting both
   fields entirely. Run `cargo run -p rexymcp -- runs --config <tmp>` and quote the
   table — the first row shows the served model + truncation rate, the legacy row
   shows `—` for both (proving both the display and the `#[serde(default)]`
   back-compat against the real binary + real read path).
2. Quote a `--json` run of the same store showing the two new fields present on the
   first record and defaulted (`null`) on the legacy one.

## Authorizations

- [x] May extend `AiEvent::Done`, parse the two values in `openai.rs`, aggregate in
      `RunMetrics`, add the two `#[serde(default)]` fields to `PhaseRun`, populate
      them at the emit site, and render them in `mcp/src/runs.rs` — updating all
      seven `Done` call sites and all `PhaseRun` struct literals as required.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.
- [ ] Do **not** change the `model_scorecard` aggregation, the `AiClient` trait
      signature, or `executor/src/health.rs`.

## Out of scope

- **Context window (`max_model_len` from `/v1/models`)** — phase-05c. Do **not**
  touch `executor/src/health.rs` or the models path.
- **A scorecard slice over the new fields** — phase-06. This phase records +
  displays per-run; it does **not** aggregate `model × settings`/provenance.
- **Reasoning/other finish_reason analytics** beyond the `length` fraction (e.g.
  per-reason histograms). Keep to `served_model` + `length_finish_rate`.
- **Changing the requested-`model` field** (`PhaseRun.model`) — that stays the
  configured/requested id; `served_model` is the *separate* endpoint-reported one.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
