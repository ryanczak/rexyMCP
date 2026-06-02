# Phase 05b: chat-stream provenance — served model id + finish_reason

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** done
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
3. Read this entire phase doc before touching code. **The enum change here is
   ADDITIVE: you add a new `AiEvent::Completion` variant and you do NOT modify the
   existing `Done` variant or any of its call sites.** This keeps the crate
   compiling at every step. (A prior attempt instead *mutated* `Done` into a struct
   variant, which broke the whole crate at once and got the verifier stuck before
   the cascade finished — do not do that.) The only edits forced by the new variant
   are **two** `match` arms (Task 6); everything else is additive.
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

**`Done` stays exactly as it is.** A new sibling variant `Completion { finish_reason,
model }` carries the provenance, emitted just before `Done`. This is additive — no
existing `AiEvent::Done(...)` construction or match changes — so the crate keeps
compiling while you work. The only code that *must* change for the new variant is
the two exhaustive `match` statements over `AiEvent` (the agent loop and one test),
which gain a `Completion` arm.

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

### Task 1 — ADD an `AiEvent::Completion` variant (`executor/src/ai/types.rs`) — leave `Done` alone

Add a new variant to the `AiEvent` enum. **Do NOT touch the `Done(TokenBreakdown)`
variant** — it stays exactly as it is:

```rust
    Done(TokenBreakdown),   // ← UNCHANGED — do not edit this line
    /// Per-completion provenance, emitted once just before `Done`. Either field may be `None`.
    Completion {
        finish_reason: Option<String>,
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

**Leave the existing `let _ = tx.send(AiEvent::Done(usage));` line unchanged.**
Immediately **before** it, add one send for the new variant:

```rust
let _ = tx.send(AiEvent::Completion { finish_reason, model: served_model });
let _ = tx.send(AiEvent::Done(usage));   // ← UNCHANGED
```

### Task 3 — Aggregate in `RunMetrics` (`executor/src/agent/mod.rs`)

Add fields to `RunMetrics` and initialize them in `started_at`:

```rust
    served_model: Option<String>,
    length_finishes: usize,
    total_finishes: usize,
```

**Leave the existing `AiEvent::Done(breakdown) => metrics.add_tokens(&breakdown),`
arm unchanged.** Add a new arm next to it (this is one of the two forced match-arm
edits):

```rust
AiEvent::Done(breakdown) => metrics.add_tokens(&breakdown),   // ← UNCHANGED
AiEvent::Completion { finish_reason, model } => {
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

### Task 6 — The two forced `match` arms + the `PhaseRun` literal additions

Because `AiEvent::Completion` is a **new** variant, the only code it *breaks* is the
two **exhaustive `match` statements** over `AiEvent`. Every existing
`AiEvent::Done(...)` construction (in `testing.rs:259`, `agent/mod.rs:1636`/`:1752`/
`:3126`, and the `openai.rs` send) stays **exactly as it is** — do not touch them.

The two `match` arms to add (confirm with `grep -rn "AiEvent::Token" executor` —
each exhaustive match has a `Token` arm):

1. **`executor/src/agent/mod.rs` main loop** (the `match` with `AiEvent::Token(s)`,
   `AiEvent::Done(breakdown)`, `AiEvent::Error(e)` arms) → add the `AiEvent::Completion`
   arm from Task 3. Leave the `Done` and `Error` arms unchanged.
2. **`executor/src/ai/testing.rs:275`** (the test `match` whose `Done` arm is
   `AiEvent::Done(_) => {}`) → add a sibling arm `AiEvent::Completion { .. } => {}`.
   Leave the `Done(_)` arm as-is.

Then add the two new fields to every **`PhaseRun { … }` struct literal** (Rust
requires all fields in a literal even when `Option`/defaulted). Find them with
`grep -rn "PhaseRun {" executor mcp` — the sites are `telemetry.rs` `sample()`,
`agent/mod.rs` `emit_phase_run` (Task 4, the real one), `mcp/src/scorecard.rs` test
helper, and `mcp/src/runs.rs` `make_run` / `make_run_with_params`. Add
`served_model: None, length_finish_rate: None` to the test/helper literals (the real
emit site uses the Task 4 values). **The hand-written JSONL string fixtures in
`mcp/src/server.rs` do NOT need changes** — `#[serde(default)]` lets them
deserialize without the new fields; that is the whole reason the fields are
defaulted.

**Completion check:** `cargo build` clean (do this before `cargo test`). Because the
change is additive, the crate should compile after the two `match` arms exist and
the `PhaseRun` literals are updated — there is no window where `Done`'s own call
sites are broken.

## Acceptance criteria

- [ ] A new `AiEvent::Completion { finish_reason: Option<String>, model:
      Option<String> }` variant exists; the existing `Done(TokenBreakdown)` variant
      and all its call sites are unchanged.
- [ ] The SSE parser captures the response `model` and `finish_reason` and sends
      them in a `Completion` event just before `Done`.
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

In `executor/src/agent/mod.rs` tests, using `MockAiClientEvents` to script
`Completion` events (script a `Completion` then a `Done` per turn, mirroring what
`openai.rs` now emits):

- `length_finish_rate_is_fraction_of_length_finishes` — two completions, one
  `AiEvent::Completion { finish_reason: Some("length".into()), model: None }` and one
  `Some("stop")`; assert the emitted `PhaseRun.length_finish_rate == Some(0.5)`.
- `length_finish_rate_none_when_no_finish_reasons` — no `Completion` events with a
  `finish_reason` (or none at all); assert `length_finish_rate == None` (must-NOT be
  `Some(0.0)`; the divide-by-zero boundary).
- `served_model_recorded_from_completion` — a `Completion { model:
  Some("served-x".into()), finish_reason: None }` → emitted
  `PhaseRun.served_model == Some("served-x")`.

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

- [x] May add the `AiEvent::Completion` variant (leaving `Done` untouched), parse
      the two values in `openai.rs`, aggregate in `RunMetrics`, add the two
      `#[serde(default)]` fields to `PhaseRun`, populate them at the emit site, and
      render them in `mcp/src/runs.rs` — adding the two `Completion` match arms and
      updating all `PhaseRun` struct literals as required.
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

### Notes for executor — 2026-06-02 (refined after hard_fail)

The first attempt `hard_fail`ed (`VerifierFailurePersistent`): it **mutated** the
existing `Done` variant into a struct variant, which broke the whole crate at once
(every `AiEvent::Done(...)` call site stopped compiling), and the verifier hit its
3-strike limit before the cascade could be finished. The working tree has been reset
to a clean, green state.

**The spec is now restructured to be additive** and the partial work is gone — start
fresh from the current tree. The key change: **add a new `AiEvent::Completion`
variant; do NOT touch `Done`.** Tasks 1, 2, 3, and 6 now spell this out. The crate
stays compiling throughout because every existing `Done` construction/match is left
exactly as-is; the only forced edits for the new variant are **two** `match` arms
(Task 6). Build (`cargo build`) after adding those two arms + the `PhaseRun` fields
and literals — there is no all-or-nothing window this time.

### Update — 2026-06-02 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the failure was a verifier-trap from a breaking enum change, not a
spec-clarity gap — so the refinement restructures the design to be additive (new
`Completion` variant, `Done` untouched), a different and untried lever that keeps the
crate compiling and plays to the executor's additive-edit strength, rather than a
takeover that would forfeit the model telemetry point.

### Update — 2025-01-09 14:00 (progress)

**Executor:** rexyMCP executor
**Working on:** Implementing phase-05b spec tasks 1-6.

### Update — 2025-01-09 14:05 (complete)

**Summary:** Implemented chat-stream provenance capture. Added a new `AiEvent::Completion` variant (additive, `Done` untouched), parsed `model` and `finish_reason` from SSE chunks in `openai.rs`, aggregated served model + length-finish counts in `RunMetrics`, added `served_model` and `length_finish_rate` fields to `PhaseRun` with `#[serde(default)]` for back-compat, populated them at the `emit_phase_run` site, and rendered both in `rexymcp runs` table output. All `PhaseRun` struct literals updated across executor and mcp crates.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean — no output)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3
   Compiling rexymcp v0.1.3
    Finished `dev` profile [unoptimized + debuginfo]

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.3
    Checking rexymcp v0.1.3
    Finished `dev` profile

cargo test 2>&1 | tail -30
test result: ok. 552 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

`rexymcp runs` table with one record having provenance and one legacy record:
```
AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC
734d    qwen2.5-coder rust,feature   default      ✓✓✓✓  7      complete  —           qwen-served   25%
745d    gemma-7b rust           default      ✓✓✓✓  3      complete  —           —             —
```
First row shows `qwen-served` + `25%`; legacy row shows `—` for both (back-compat via `#[serde(default)]`).

`--json` output: first record has `"served_model": "qwen-served", "length_finish_rate": 0.25`; legacy record has `"served_model": null, "length_finish_rate": null`.

**Grep proving `AiEvent::Completion` landed:**
```
grep -rn "AiEvent::Completion" executor/src/ai/types.rs executor/src/ai/backends/openai.rs executor/src/agent/mod.rs executor/src/ai/testing.rs
executor/src/ai/backends/openai.rs:339:                    let _ = tx.send(AiEvent::Completion {
executor/src/agent/mod.rs:321:                AiEvent::Completion {
executor/src/agent/mod.rs:3715:            AiEvent::Completion {
executor/src/agent/mod.rs:3719:            AiEvent::Completion {
executor/src/agent/mod.rs:3782:            AiEvent::Completion {
executor/src/ai/testing.rs:276:            AiEvent::Completion { .. } => {}
```

**Files changed:**
- `executor/src/ai/types.rs` — added `AiEvent::Completion` variant
- `executor/src/ai/backends/openai.rs` — parse `model`/`finish_reason` from SSE, emit `Completion` event before `Done`
- `executor/src/agent/mod.rs` — `RunMetrics` fields + `Completion` match arm + `emit_phase_run` population + 3 new tests
- `executor/src/store/telemetry.rs` — `PhaseRun` fields with `#[serde(default)]` + `sample()` literal + legacy deserialization test
- `executor/src/ai/testing.rs` — `Completion` match arm in existing test
- `mcp/src/runs.rs` — `format_runs` table columns + `make_run`/`make_run_with_params` literals + new test
- `mcp/src/scorecard.rs` — `make_run` literal updated

**New tests:**
- `length_finish_rate_is_fraction_of_length_finishes` in `executor/src/agent/mod.rs`
- `length_finish_rate_none_when_no_finish_reasons` in `executor/src/agent/mod.rs`
- `served_model_recorded_from_completion` in `executor/src/agent/mod.rs`
- `phase_run_without_provenance_fields_deserializes` in `executor/src/store/telemetry.rs`
- `format_runs_shows_served_model_and_truncation` in `mcp/src/runs.rs`

**Notes for review:** The `length_finish_rate_is_fraction_of_length_finishes` test scripts two `Completion` events in a single turn (via `MockAiClientScript`) rather than across two turns, because `MockAiClientScript` emits one event batch per `chat` call and the agent loop accumulates `RunMetrics` across all events in a single run. This correctly exercises the aggregation logic — both completions contribute to the same `total_finishes`/`length_finishes` counters.

### Review verdict — 2026-06-02

- **Verdict:** approved_after_1
- **Bounces:** one prior failed cycle — a `hard_fail` (`VerifierFailurePersistent`)
  on the first dispatch, which mutated the existing `AiEvent::Done` tuple into a
  struct variant and broke the whole crate before the 3-strike verifier limit.
  Resolved by escalation → refined re-dispatch with an **additive** restructure (new
  sibling `AiEvent::Completion` variant, `Done` untouched). No review bounce; the
  re-dispatch landed clean.
- **Executor:** rexyMCP executor — `Qwen/Qwen3.6-27B-FP8`
- **Scope deviations:** one, benign and declared in Notes for review — the
  `length_finish_rate_is_fraction_of_length_finishes` test scripts the two
  `Completion` events in a single turn rather than across two turns. Acceptable:
  `RunMetrics` accumulates per-run, so both completions hit the same
  `total_finishes`/`length_finishes` counters and the 0.5 fraction is exercised
  correctly.
- **Calibration:** reviewer re-ran all four gates independently (fmt/build/clippy/
  test — 552 executor + 143 mcp pass) and verified end-to-end against the real
  binary: a provenance record renders `served_model` + a `25%` truncation cell while
  a legacy JSONL line (omitting both fields) renders `—`/`—` and `--json`-defaults to
  `null` (the `#[serde(default)]` back-compat boundary). **Two calibration wins:**
  (1) the additive-change-shape discipline folded into WORKFLOW.md after the
  hard_fail worked on its first application — the executor completed the additive
  variant in 75 turns with no cascade failure; (2) the bug-05a-1 end-of-phase
  drop-off (skipped commit + completion log) did **not** recur — the executor
  committed one clean `feat:` and wrote a full completion entry. The completion-ritual
  concern stays at 1 occurrence (data, not a trend); no contract fold needed yet.
