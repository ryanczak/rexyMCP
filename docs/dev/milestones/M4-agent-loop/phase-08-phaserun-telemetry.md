# Phase 08: `PhaseRun` telemetry (M4 closer)

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** todo
**Depends on:** phase-07a–07e (the full loop), phase-03 (`store::sessions` for the
JSONL-append pattern + `generate_session_id`), `ai::types::TokenBreakdown`,
`governor::scorer::Scorer`. All done.
**Estimated diff:** ~450 lines (PhaseRun + store + metric accumulation + emit +
tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Emit one **`PhaseRun`** summary record per `execute_phase` into a **cross-project**
telemetry store — the durable substrate for the model scorecard (`model × tag`)
and the human project review (`milestone × phase`), both M7. This closes the M4
loop: the loop already produces the lean `PhaseResult` (Claude's view) and the
JSONL session log (the deep-dive); the `PhaseRun` is the third output — the
*metrics row*. It also resolves the long-carried **scorer consumer**:
`tool_success_rate` is computed from the loop's `Scorer`.

> **This is net-new, not a lift.** Rexy's `store/telemetry.rs` is a one-line
> `// TODO` stub (same situation as the M4 redaction primitive) — design it for
> rexyMCP; don't "lift" the stub.

> **Milestone closer.** 08 is the last in-scope M4 phase. Completing it (review →
> done) hits a **milestone boundary**, which is a **human gate**: the architect
> stops for the M4 retrospective + doc-fold review (WORKFLOW § "Calibration",
> "Milestone boundaries are always a human gate"). Do **not** chain into M5.

## Architecture references

Read before starting:

- `docs/architecture.md` — "Model effectiveness metrics & routing": the **exact
  `PhaseRun` field list** (lines ~240–255), "The executor fills the objective
  fields at phase end; the architect's review fills `bugs_filed`,
  `bounces_to_approval`, and `architect_verdict`", and "cross-project … accumulate
  across every repo … not per-repo". The consumers (`model_scorecard`,
  `project_review`) are **M7** — this phase only writes the record.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference and the M4 README (incl. the retrospective note
   at the bottom — you are about to enable it).
3. Read this entire phase doc before touching any code.
4. Read:
   - `executor/src/store/sessions/jsonl.rs` — the append-JSONL pattern + dir
     creation to mirror (do **not** reuse the session schema — `PhaseRun` is its
     own record).
   - `executor/src/governor/scorer.rs` — `Scorer.counts: HashMap<String,
     ToolCounts { successes, failures }>` is **public**; sum it for the overall
     success rate (no `scorer` edit needed).
   - `executor/src/ai/types.rs` — `TokenBreakdown` (the loop currently **ignores**
     `AiEvent::Done(breakdown)` — this phase accumulates it).
   - `executor/src/agent/mod.rs` — the turn loop (where metrics accrue) + the
     terminal returns (where the record is emitted) + `agent/command.rs` (the
     `CommandRunner` seam, extended here for gate exit status).
   - `executor/src/parser/mod.rs` — `Origin::Repaired { repairs }` (for
     `repairs_per_call`).

## Current state

The loop returns `PhaseResult` + writes the session log, but emits **no** metrics
record. `AiEvent::Done(_)` is discarded (no token accounting). The `Scorer` is
recorded into but never read (the carry from 07a). `CommandRunner::run` returns a
plain `String` (no exit status, so no gate pass/fail). `store/mod.rs` declares only
`pub mod sessions;`.

## Spec

### 1. The `PhaseRun` record (`executor/src/store/telemetry.rs`, new)

Define `PhaseRun` (`Serialize` + `Deserialize`) with the architecture's fields.
Suggested grouping (exact shape is yours; the **field set + the objective/
supervision split** are pinned):

```rust
pub struct GenerationParams { pub temperature: Option<f64>, pub seed: Option<u64> }
pub struct Gates { pub fmt: Option<bool>, pub build: Option<bool>,
                   pub lint: Option<bool>, pub test: Option<bool> }   // None = not run

pub struct PhaseRun {
    // identity
    pub model: String,
    pub generation_params: GenerationParams,
    pub phase_id: String,
    pub tags: Vec<String>,
    // outcome
    pub status: String,            // "complete" | "hard_fail" | "budget_exceeded"
    pub escalated: bool,           // status != complete (briefing returned to Claude)
    // quality (objective)
    pub gates: Gates,
    // reliability (objective)
    pub parse_failure_rate: f64,
    pub repairs_per_call: f64,
    pub verifier_retries: usize,
    pub tool_success_rate: f64,
    // efficiency (objective)
    pub turns: usize,
    pub wall_clock_s: f64,
    pub tokens: TokenBreakdown,
    // supervision (architect-filled later, M7) — default/None now
    pub warnings: Option<u32>,
    pub bugs_filed: Option<u32>,
    pub bounces_to_approval: Option<u32>,
    pub architect_verdict: Option<String>,
}
```

The executor fills every objective field; the supervision fields are
`None`/default here (the architect/M7 fills them at review). `ts` (emit time) via
the injected clock.

### 2. Telemetry store (append-JSONL, cross-project, best-effort)

In the same module: append a `PhaseRun` as one JSON line to
`<telemetry_dir>/phase_runs.jsonl` (create the dir if needed). This is **cross-
project** — `telemetry_dir` is a single global path the caller (M5) resolves, **not**
under the target repo. Provide `append(telemetry_dir: &Path, run: &PhaseRun) ->
io::Result<PathBuf>` (mirror `session log`'s append + the records reader for tests).

Emission is **best-effort** (same contract as the session log): the loop holds an
`Option<&Path>` telemetry dir; `None` or an append error must **not** change the
returned `PhaseResult` or error the phase. (`.ok()` / ignore, with a comment.)

### 3. Extend the command-runner seam for gate status

`CommandRunner::run` must report pass/fail so `Gates` can be populated. Change it
to return a small `CommandResult { output: String, success: bool }` (or
`(String, bool)`); `RealCommandRunner` uses `output.status.success()`. 07e's
`command_outputs` keeps using the **output** string (tail-capped); `Gates` uses the
**success** bool. Update the `MockCommandRunner` in tests accordingly. Only the
clean-completion path runs commands, so on `hard_fail`/`budget_exceeded` every gate
is `None`.

### 4. Accumulate metrics in the loop

Track during the turn cycle (a small `RunMetrics` carrier or inline counters):

- **`parse_failure_rate`** = parse failures / parse **attempts**. Increment
  attempts only on the **text-parser** branch (a native `ToolCallGeneric` turn is
  not a parse attempt); increment failures on `ParseResult::Failed`. `0.0` when no
  attempts.
- **`repairs_per_call`** = total repairs / dispatched calls. A dispatched
  `ToolCall` with `Origin::Repaired { repairs }` contributes `repairs.len()`;
  `Native`/`Extracted` contribute 0. `0.0` when no calls.
- **`verifier_retries`** = count of verifier runs that produced author diagnostics
  (the nonzero entries in the 07c `recent_verifier_error_counts`).
- **`tool_success_rate`** = Σ`successes` / Σ(`successes`+`failures`) over
  `scorer.counts` (**the scorer consumer**); `0.0` when no calls.
- **`tokens`** = accumulate each `AiEvent::Done(breakdown)` (sum the four buckets,
  saturating) — change the current `Done(_) => {}` to fold it in.
- **`turns`** = the existing counter.
- **`wall_clock_s`** = `(clock_end_ms − clock_start_ms) as f64 / 1000.0`; capture
  start at loop entry, end at the terminal return (deterministic under the injected
  clock — a constant clock yields `0.0`).

### 5. Emit at every terminal return

Build the `PhaseRun` and `append` it (best-effort) on **all three** terminal paths
(complete / hard_fail / budget_exceeded), after the session-log `SessionEnd`.
`status`/`escalated`/`gates` reflect the path (gates populated only on complete).
Inputs the record needs that the loop doesn't already hold:

- `model` — already on `LoopDeps`.
- `phase_id` — `PhaseInput.phase` (the slug).
- `tags` — **add `tags: Vec<String>` to `PhaseInput`** (the architect sets them
  from the phase-doc `Tags:` line; tests pass a sample).
- `generation_params` — **add `generation_params: GenerationParams` to
  `LoopDeps`** (M5 populates from the request; default/`None` in tests).
- `telemetry_dir: Option<&Path>` — **add to `LoopDeps`**.

### 6. Error model

- Telemetry append is best-effort: `io::Error` and a `None` dir are swallowed
  (commented as deliberate, citing the same principle as the session log). Never
  `?`-propagated to fail the phase.
- No `.unwrap()`/`.expect()`/`panic!()` in the loop or store.

### 7. Opportunistic fix

While editing the agent tests, correct the stale `NoopRunner` doc comment from
07e ("panics if called" → it returns empty; reword to match). (07e review nit.)

## Acceptance criteria

- [ ] `store::telemetry::{PhaseRun, append}` exist; `pub mod telemetry;` in
      `store/mod.rs`; `PhaseRun` round-trips through JSON.
- [ ] A run appends exactly one `PhaseRun` line to
      `<telemetry_dir>/phase_runs.jsonl`; a `None` dir or write error leaves the
      returned `PhaseResult` unchanged and does not error (**best-effort**).
- [ ] `status`/`escalated` correct (escalated iff not complete); `gates` populated
      (from command exit status) **only** on clean completion, `None` otherwise.
- [ ] `tool_success_rate` is computed from `scorer.counts` (the scorer consumer);
      `parse_failure_rate`, `repairs_per_call`, `verifier_retries`, `turns`,
      `tokens` reflect the run; `wall_clock_s` from the injected clock.
- [ ] `CommandRunner::run` reports exit status; 07e `command_outputs` still carry
      the tail string; tests' `MockCommandRunner` updated.
- [ ] No new dependency; no `tracing`; `governor`/`parser`/`config` unmodified
      (scorer read via its public `counts`); only `store/**` + `agent/**` changed.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic (`TempDir` telemetry dir; read back with a `read` helper), deterministic
(injected clock; `MockAiClientScript`; `MockFileVerifier`/`MockCommandRunner`).

**Record / store:**
- `phase_run_round_trips_through_json`.
- `run_appends_one_phase_run_line` — read `phase_runs.jsonl`, exactly one record.
- `telemetry_none_dir_is_noop_and_completes` (**best-effort** — `telemetry_dir:
  None` → run completes, no panic).
- `telemetry_write_failure_does_not_change_result` (dir path is a file).

**Field correctness:**
- `complete_run_is_not_escalated` / `hard_fail_run_is_escalated`.
- `gates_populated_on_complete_from_exit_status` (mock runner: build success, test
  failure → `gates.build == Some(true)`, `gates.test == Some(false)`).
- `gates_none_on_hard_fail` (**negative**).
- `tool_success_rate_reflects_scorer` — drive one success + one failure → rate
  matches `Σsucc/Σtotal`.
- `parse_failure_rate_counts_only_parse_attempts` (a native turn doesn't count).
- `repairs_per_call_counts_repaired_origin`.
- `verifier_retries_counts_author_failures`.
- `tokens_accumulate_across_done_events`.
- `wall_clock_zero_under_constant_clock`.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The `PhaseRun` is
> written by `execute_phase`, exercised via mocks + `TempDir` and read back. The
> M7 scorecard/`project_review` consumers and a live cross-project store are the
> first real end-to-end.

## Authorizations

- [x] **May create** `executor/src/store/telemetry.rs`; **may modify**
      `executor/src/store/mod.rs` (`pub mod telemetry;`) and `executor/src/agent/**`
      (loop + `command.rs` runner seam + tests).
- [ ] **No new dependencies**; no `tracing`.
- [ ] May **NOT** modify `governor/**` (scorer read via public `counts`),
      `parser/**`, `config.rs`, `phase/**`, `Cargo.toml`, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, or another phase doc.

## Out of scope

- **`model_scorecard` / `project_review` consumers** + the `model × tag` matrix —
  M7. This phase only *writes* the record.
- **Benchmark suite / routing policy** — later milestones.
- **Filling supervision fields** (`bugs_filed` / `bounces_to_approval` /
  `architect_verdict` / `warnings`) — architect/M7; left `None`/default here.
- **MCP wiring** (`execute_phase`, log-query tools, telemetry path resolution) — M5.
- **Telemetry rotation / GC** (Rexy's stub mentioned 10 MB rotate / 30-day GC) —
  not now; a plain append is enough for M4. Note it for a future phase if wanted.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
