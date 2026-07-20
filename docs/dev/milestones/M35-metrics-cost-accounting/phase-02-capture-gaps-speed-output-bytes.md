# Phase 02: Capture gaps — generation speed + output bytes

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-01
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Record the two measurements later M35 phases need but nothing captures today:
per-run **generation time** (`PhaseRun.gen_time_s`, the ingredient for tok/s)
and per-tool-call **full output size** (`SessionEvent::ToolResult.output_bytes`,
the M34-deferred field that makes the output-flood detector calibratable).
Capture only — no display, no derivation, no detector changes.

**Committed consumers** (so this is not dead state): phase-03's shared metrics
core derives tok/s from `tokens.output_tokens / gen_time_s` and phase-04 shows
it in `rexymcp runs`; phase-07 / a future `calibrate-governor` output-flood
signal reads `output_bytes`. `output_bytes` is additionally visible immediately
via `executor_log_search` (tool_result records carry it).

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" —
  what `PhaseRun` records and why (pull-not-push: the loop records, surfaces
  derive later).
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — the milestone
  plan; this phase is exit-criterion bullet 2.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule (this exact mistake caused two hard-fails on the
   previous phase):** every new `#[test]` fn goes **INSIDE the existing
   `#[cfg(test)] mod tests { ... }` block** of its file — appended just before
   that block's closing `}`. Never at file scope, never in a new sibling
   module. If you cannot find the block, `grep -n "mod tests" <file>`.

## Current state

**Generation timing does not exist.** The loop (`executor/src/agent/mod.rs`)
drives each model call as a pinned future inside a `select!`:

```rust
// mod.rs:412 — the model call begins here
let chat_fut = deps.client.chat(&system, messages.clone(), tx, tools_opt);
tokio::pin!(chat_fut);
...
loop {
    tokio::select! {
        _ = cancel.cancelled() => { ... return ...; }
        result = &mut chat_fut => {
            match result { Ok(()) => {}  ...error paths return early... }
            break;                       // ← success lands here
        }
        _ = heartbeat.tick() => { ... }
    }
}

let mut completion = String::new();      // ← mod.rs ~507, after the loop
```

`RunMetrics` (`executor/src/agent/metrics.rs:8-19`) accumulates per-call token
counts (`add_tokens`, fed by `AiEvent::Done`) and `emit_phase_run`
(metrics.rs:55-137) builds the `PhaseRun` literal, including
`wall_clock_s = now - metrics.start_ms` off the injected `deps.clock`
(a `&dyn Fn() -> u64` returning epoch millis). No per-call duration is measured.

**`PhaseRun`** (`executor/src/store/telemetry.rs:121`) derives
`Debug, Clone, Serialize, Deserialize` — **no `Default`**. Full struct
literals (naming every field) exist at exactly these places; everything else
constructs via functional-update (`..sample()`), which absorbs new fields
automatically:

- `executor/src/agent/metrics.rs` ~104 — the one **production** literal.
- `executor/src/store/telemetry.rs:796` — the `sample()` test helper (8 other
  test literals spread `..sample()` and need no edit).
- mcp test helpers/literals: `mcp/src/profile_cli.rs:114`,
  `mcp/src/profile.rs:255` and `:298`, `mcp/src/runs.rs:188`, `:231`, `:640`,
  `mcp/src/scorecard_cli.rs:104`, `mcp/src/scorecard_tests.rs:13`, `:403`,
  and `:~620` (`make_run_with_eff`).

Re-verify this inventory live with `grep -rn "PhaseRun {" executor/src mcp/src`
— the rule: a literal ending in `..something()` needs **no** edit; a literal
naming every field is on the list.

**Tool output size is lost.** The one production `ToolResult` event emit
(`executor/src/agent/mod.rs:1092-1101`):

```rust
log_event(
    &log_handle, &redactor, deps.clock, turns,
    SessionEvent::ToolResult {
        name: tool_call.name.clone(),
        succeeded,
        output_preview: output_preview(&content),
    },
);
```

`output_preview` (`executor/src/agent/tools.rs:15`) truncates to
`OUTPUT_PREVIEW_CHARS = 500` **chars**, so the log cannot recover the true
size — the M34 gap. `content` (the full tool output `String`) is in scope at
the emit site.

**`SessionEvent::ToolResult`** (`executor/src/store/sessions/event.rs:50-53`)
has fields `name, succeeded, output_preview`. Sites that name all its fields
(and therefore break when a field is added):

- construct (prod): `executor/src/agent/mod.rs:1097`.
- destructure+reconstruct (prod): `mcp/src/cap.rs:79-87`
  (`cap_session_record`'s ToolResult arm).
- destructure (prod): `mcp/src/dashboard/transcript.rs:180-184` (render; does
  not need the new field).
- tests: `executor/src/store/sessions/jsonl.rs:106` + the `:116` roundtrip
  destructure, `:170`, `:288`; `mcp/src/cap.rs:358`; `mcp/src/status.rs:471`
  (helper inside `#[cfg(test)]`); `mcp/src/dashboard/filter.rs:226`;
  `mcp/src/dashboard/transcript.rs:408`; `mcp/src/log_query.rs:172`.

Sites matching with `{ .. }` or `{ name, .. }` (log_query.rs:21/39,
filter.rs:54, status.rs:170, transcript.rs:88, agent/tests.rs:960) are
unaffected.

**Old-corpus constraint:** `.rexymcp/sessions/*.jsonl` logs written before this
phase have no `output_bytes`, and `calibrate-governor` replays that corpus.
The new field must be `#[serde(default)]` and a test must pin that an
old-format line still parses. Likewise phase-01-era telemetry lines (already
`schema_version: 1`) have no `gen_time_s` and must keep parsing.

## Spec

Execute Part A fully (Tasks 1–5), then Part B (Tasks 6–8). The two parts are
independent; do not interleave them.

### Part A — `PhaseRun.gen_time_s`

### Task 1 — Derive `Default` on `PhaseRun`

In `executor/src/store/telemetry.rs:120`, add `Default` to `PhaseRun`'s derive
list. Every field type already implements `Default` (verified:
`GenerationParams`, `Gates`, `TokenBreakdown`, `ContextEfficiency`,
`TierTelemetry` all derive it). This compiles immediately. The derive exists
for test-literal ergonomics (Task 2); production code keeps constructing every
field explicitly.

### Task 2 — Future-proof the full test literals with `..Default::default()`

Append `..Default::default()` as the final entry of each **test** literal in
the inventory above (all except `agent/metrics.rs` — that production literal
is handled in Task 4). Example, the `sample()` helper:

```rust
fn sample() -> PhaseRun {
    PhaseRun {
        ts: 1_717_000_000_000,
        ...existing fields unchanged...
        tier_telemetry: TierTelemetry::default(),
        ..Default::default()
    }
}
```

One file per turn. The build stays **green after every single edit** (a
functional update with no remaining fields is legal Rust). Two gotchas:

- **Do not run the clippy gate between Task 2 and Task 4** — clippy flags a
  no-op `..Default::default()` as `needless_update` until the new field lands
  in Task 4, at which point the update becomes meaningful and the lint goes
  quiet. `cargo build` / `cargo check` are unaffected; use those.
- **Do not touch the 8 `..sample()` literals** in telemetry.rs — they absorb
  new fields through `sample()` automatically.

### Task 3 — Measure per-call generation time in the loop

1. `executor/src/agent/metrics.rs`: add `pub(super) gen_ms: u64` to
   `RunMetrics` and `gen_ms: 0` to `started_at`.
2. `executor/src/agent/mod.rs`: immediately **before** the
   `let chat_fut = deps.client.chat(...)` line (~412), insert:

   ```rust
   let call_started_ms = (deps.clock)();
   ```

   Immediately **after** the `select!` loop ends (just before
   `let mut completion = String::new();`, ~507), insert:

   ```rust
   metrics.gen_ms = metrics
       .gen_ms
       .saturating_add((deps.clock)().saturating_sub(call_started_ms));
   ```

   That point is reached only on a successful model call (the cancel and
   error arms return early), so `gen_ms` sums the wall time of every
   successful generation call — prefill + decode, including in-backend
   retries. That is the honest denominator for tok/s.

### Task 4 — Add the `gen_time_s` field and emit it

1. `executor/src/store/telemetry.rs`: in `PhaseRun`, directly after
   `wall_clock_s`, add:

   ```rust
   /// Total wall time spent awaiting model generation across all calls,
   /// in seconds. tok/s derives as `tokens.output_tokens / gen_time_s`
   /// (guard zero). `0.0` for v1 records written before this field existed.
   #[serde(default)]
   pub gen_time_s: f64,
   ```

2. `executor/src/agent/metrics.rs`: in the `emit_phase_run` literal, after
   `wall_clock_s,` add:

   ```rust
   gen_time_s: metrics.gen_ms as f64 / 1000.0,
   ```

Edit both files in this order in back-to-back turns — telemetry.rs first,
metrics.rs second. metrics.rs is the **only** site that breaks in between
(Task 2 made every test literal absorb the field), so the red window is one
turn. After metrics.rs, run `cargo build` — green.

### Task 5 — Part A tests

Per the Test plan below (`gen_time_recorded_with_advancing_clock`,
`phase_run_line_without_gen_time_s_parses_default`). Remember Pre-flight
step 5: inside the existing `mod tests` blocks.

### Part B — `SessionEvent::ToolResult.output_bytes`

### Task 6 — Pre-adapt the render destructure (green)

`mcp/src/dashboard/transcript.rs:180-184`: the exhaustive destructure binds
`name, succeeded, output_preview` for rendering and will not need the new
field — add `..` to the pattern now:

```rust
SessionEvent::ToolResult {
    name,
    succeeded,
    output_preview,
    ..
} => {
```

This compiles both before and after the field addition.

### Task 7 — Add the field and fix sites leaf-first

Add to `executor/src/store/sessions/event.rs`'s `ToolResult` variant, after
`output_preview`:

```rust
/// Full byte length (`content.len()`) of the tool output **before**
/// `output_preview` truncation. `0` for records written before this field
/// existed. The output-flood calibration signal reads this.
#[serde(default)]
pub output_bytes: u64,
```

(Enum-variant fields take `#[serde(default)]` exactly like struct fields.)

Then fix the breaking sites **one file per turn, in exactly this order** —
each fix strictly shrinks the error count, executor crate first so it goes
green before mcp starts:

1. `executor/src/agent/mod.rs:1097` — in the emit, add
   `output_bytes: content.len() as u64,`. **Compute from `content`, not from
   the preview** — `output_preview` caps at 500 chars; the full byte size is
   the entire point of the field.
2. `executor/src/store/sessions/jsonl.rs` — the `:106` constructor (use a
   value like `999`), the `:116` roundtrip destructure (bind `output_bytes`
   and assert it round-trips), and the `:170` and `:288` constructors
   (any literal value). One file, one turn.
3. `mcp/src/cap.rs:79-87` — bind `output_bytes` in the pattern and pass it
   through **uncapped** in the reconstruction (it is a number; `cap_string`
   applies only to strings). Fix the `:358` test constructor in the same
   turn (same file).
4. `mcp/src/status.rs:471` — test helper: `output_bytes: 0,`.
5. `mcp/src/dashboard/filter.rs:226` — test: `output_bytes: 0,`.
6. `mcp/src/dashboard/transcript.rs:408` — test: `output_bytes: 0,`.
7. `mcp/src/log_query.rs:172` — test: `output_bytes: 0,`.

Trust this list — do not re-derive it with exploratory greps between fixes
(the previous phase's dispatches died oscillating on re-verification loops).
If the compiler names a site this list missed, fix it and note it in "Notes
for review".

### Task 8 — Part B tests

Per the Test plan below. Same placement rule.

## Acceptance criteria

- [ ] `cargo test gen_time` passes (both Part A tests).
- [ ] `cargo test output_bytes` passes (Part B tests).
- [ ] A `PhaseRun` line appended by the loop contains `"gen_time_s":`
      (pinned by `gen_time_recorded_with_advancing_clock`).
- [ ] A session-log `tool_result` line contains `"output_bytes":` equal to
      the full output length, while `output_preview` stays capped (pinned by
      `tool_result_records_full_output_bytes_not_preview_len`).
- [ ] A pre-phase-02 telemetry line (schema_version 1, no `gen_time_s`) and a
      pre-phase-02 session-log line (no `output_bytes`) both still parse
      (pinned by the two `*_parses_default` tests).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `executor/src/agent/tests.rs` (inside `mod tests`; model the harness on
`run_appends_one_phase_run_line` at ~2377 and the bespoke-`LoopDeps` shape of
`injected_clock_sets_record_ts` at ~912):

- `gen_time_recorded_with_advancing_clock` — build `LoopDeps` with an
  advancing injected clock (no sleeps — a fn item over an atomic:

  ```rust
  fn clock_advancing() -> u64 {
      use std::sync::atomic::{AtomicU64, Ordering};
      static T: AtomicU64 = AtomicU64::new(0);
      T.fetch_add(250, Ordering::SeqCst)
  }
  ```

  ), a one-turn `MockAiClientScript` and a telemetry dir; run the loop; read
  the emitted run via the existing `read_runs` helper and assert
  `runs[0].gen_time_s > 0.0` **and** `runs[0].gen_time_s <= runs[0].wall_clock_s`
  (generation time is a subset of wall time — the invariant that fails if the
  measurement brackets the whole loop or is never accumulated).
- `tool_result_records_full_output_bytes_not_preview_len` — run a mock script
  whose tool call `read_file`s a file seeded with 700+ ASCII chars; read the
  session log; the `ToolResult` event has `output_bytes >= 700` **and**
  `output_bytes > output_preview.chars().count() as u64` (must-NOT pin: a
  wrong implementation computing size from the truncated preview yields 500
  and fails both).

In `executor/src/store/telemetry.rs` tests:

- `phase_run_line_without_gen_time_s_parses_default` — hand-write a JSONL
  line that **includes `"schema_version":1`** but no `gen_time_s`; `read`
  returns it with `gen_time_s == 0.0`. **Gotcha: without the
  `schema_version` key the phase-01 version gate silently drops the line and
  the test would assert on an empty vec.**

In `executor/src/store/sessions/jsonl.rs` tests:

- `tool_result_line_without_output_bytes_parses_default` — a raw pre-phase-02
  `tool_result` JSON line (no `output_bytes`) parses with `output_bytes == 0`
  (protects the `calibrate-governor` session-log corpus).
- Extend `session_event_round_trips_through_json` to construct with a nonzero
  `output_bytes` and assert it survives the round-trip.

In `mcp/src/cap.rs` tests:

- `cap_preserves_output_bytes` — a `ToolResult` record with a long
  `output_preview` and `output_bytes: 60_000` keeps `output_bytes == 60_000`
  after `cap_session_record` (must-NOT: capping the preview must not zero or
  shrink the numeric size).

## End-to-end verification

Both fields ship inside real on-disk artifacts whose **old** instances must
keep working. Build the real binary and run its readers against the real
pre-phase-02 data (read-only):

```bash
cargo build
cargo run -p rexymcp -- runs --config rexymcp.toml
cargo run -p rexymcp -- status
```

Expected: `runs` lists the existing phase-01-era records with no parse errors
(records lacking `gen_time_s` still load); `status` summarizes existing
`.rexymcp/sessions/` logs with no parse errors (ToolResult lines lacking
`output_bytes` still load). Paste both outputs in the completion Update Log.

## Authorizations

None. (No new dependencies; no `docs/architecture.md` edit — the field
additions are within the record shapes it already describes.)

## Out of scope

- Deriving or displaying tok/s anywhere (`runs` columns, scorecard, dashboard)
  — phase-03/04 own derivation and display.
- The shared metrics/cost core, pricing — phase-03.
- The output-flood **detector** (`governor/hard_fail.rs::check_runaway_output`)
  and its threshold — it already sees live sizes; this phase only logs them.
- Extending `calibrate-governor` with an output-flood signal — phase-07
  territory (and the corpus has no data until new runs accrue).
- The `Metrics` session event, session-log schema versioning, `Compaction` —
  untouched.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
