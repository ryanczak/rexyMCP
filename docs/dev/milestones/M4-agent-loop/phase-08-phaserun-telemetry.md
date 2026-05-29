# Phase 08: `PhaseRun` telemetry (M4 closer)

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** done
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

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md.

Adding `store/telemetry.rs` (`PhaseRun`/`GenerationParams`/`Gates` + `append`/`read`),
extending `agent/command.rs`'s `CommandRunner` to return `CommandResult { output,
success }` (gates need exit status), and threading metrics through the loop: a
`RunMetrics` carrier (parse attempts/failures, repairs, calls, verifier_retries,
accumulated `tokens`, `start_ms`), token accumulation on `AiEvent::Done`, and a
best-effort `emit_phase_run` at every terminal computing `tool_success_rate` from
`Scorer.counts`. New `LoopDeps` fields `generation_params` + `telemetry_dir`;
`PhaseInput.tags`. Fixing the 07e `NoopRunner` comment nit en route.

### Update — 2026-05-29 (complete)

**Summary:** Net-new `store/telemetry.rs` — `PhaseRun` / `GenerationParams` /
`Gates` + `append` (JSONL to `<telemetry_dir>/phase_runs.jsonl`, dir created) +
`read`. Extended `agent/command.rs`'s `CommandRunner::run` to return
`CommandResult { output, success }` (07e `command_outputs` keeps `output`; `Gates`
take `success`). The loop accumulates a `RunMetrics` carrier (parse
attempts/failures, repairs from `Origin::Repaired`, dispatched calls,
`verifier_retries`, `tokens` folded from `AiEvent::Done`, `start_ms`) and, at every
terminal return, `emit_phase_run` builds the `PhaseRun` and appends it best-effort
— computing `tool_success_rate` from `Scorer.counts` (the long-carried scorer
consumer, now load-bearing). `escalated = status != "complete"`; `gates` populated
only on the clean-completion path (from command exit status), `None` elsewhere.
Supervision fields (`bugs_filed`/`bounces_to_approval`/`architect_verdict`/
`warnings`) left `None` for the architect/M7. New `LoopDeps.{generation_params,
telemetry_dir}`, `PhaseInput.tags`. `PhaseRun` omits `PartialEq` (`TokenBreakdown`
lacks it; the round-trip test compares serialized JSON). Fixed the 07e `NoopRunner`
doc-comment nit. No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.71s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.28s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 492 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(492 = 477 prior + 12 loop telemetry tests + 3 store tests. `agent::` alone: 64.)

**Spec-pinned literal grep** (the scorer consumer):

```
grep -c 'scorer.counts' executor/src/agent/mod.rs → 1 (emit_phase_run reads it for tool_success_rate)
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The `PhaseRun` is written
by `execute_phase`, exercised via mocks + `TempDir` and read back with
`telemetry::read`. M7 consumers + a live cross-project store are the first real E2E.

**Files changed:**
- `executor/src/store/telemetry.rs` — new: `PhaseRun`/`GenerationParams`/`Gates` +
  `append`/`read` + 3 tests.
- `executor/src/store/mod.rs` — `pub mod telemetry;`.
- `executor/src/agent/command.rs` — `CommandRunner::run` → `CommandResult`.
- `executor/src/agent/mod.rs` — `RunMetrics`, token accumulation, parse/repair/
  verifier counters, `emit_phase_run`, `run_command_set` returns `Gates`, new
  `LoopDeps`/`PhaseInput` fields, terminal emits; updated test harness + 12 tests;
  `NoopRunner` comment fix.

**New tests:** (store) `phase_run_round_trips_through_json`,
`append_writes_one_line_per_run`, `read_missing_file_is_empty`; (loop)
`run_appends_one_phase_run_line`, `telemetry_none_dir_is_noop_and_completes`,
`telemetry_write_failure_does_not_change_result`, `hard_fail_run_is_escalated`,
`gates_populated_on_complete_from_exit_status`, `gates_none_on_hard_fail`,
`tool_success_rate_reflects_scorer`, `parse_failure_rate_counts_only_parse_attempts`,
`repairs_per_call_counts_repaired_origin`, `verifier_retries_counts_author_failures`,
`tokens_accumulate_across_done_events`, `wall_clock_zero_under_constant_clock`.

**Commits:** (pending — committed below)

**Notes for review:**
- **Scorer consumer resolved:** `tool_success_rate` = Σsucc/Σtotal over
  `scorer.counts`; the 07a calibration carry is closed (no `governor` edit needed —
  `counts` is public).
- Gates need exit status, so `CommandRunner` was extended (within `agent/**`); the
  07e `command_outputs` contract is unchanged (still the tail string).
- `PhaseRun` is not `PartialEq` (TokenBreakdown isn't, and `ai/types.rs` is out of
  scope); round-trip test compares JSON.
- Telemetry is best-effort (`None` dir or write error → run unaffected), mirroring
  the session log.
- **Milestone closer:** this is the last in-scope M4 phase. Approving it hits the
  M4 milestone boundary → human gate (retrospective + doc folds). I am stopping at
  `review`, not advancing.

verification: fmt OK · clippy OK · tests 492 passed · build OK

### Review verdict — 2026-05-29

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude Code (direct). **Process note:** dispatched in the same turn
  as its draft (the architect overran the draft/dispatch gate); flagged to the
  human, who chose to keep it. Spec fidelity unaffected — recorded for honesty.
- **Scope deviations:** none. Only `store/**` + `agent/**` touched; `governor`/
  `parser`/`config`/`phase` untouched (scorer read via public `counts`; the
  `CommandResult` extension is in `agent/command.rs`).
- **Calibration:** the long-carried **scorer consumer is resolved** —
  `tool_success_rate` reads `Scorer.counts`, so `scorer.record` is finally
  load-bearing (no `governor` edit needed). Re-ran all four gates independently
  (492 passed); spot-checked the negatives — best-effort `None`-dir/write-failure
  paths still complete, `gates_none_on_hard_fail` confirms the command set is
  clean-completion-only, `tool_success_rate_reflects_scorer` = 0.5 on 1/2.
- **Milestone:** this is the M4 closer. With it `done`, M4's exit criteria are all
  met → milestone human gate (retrospective + folds) follows in the README.
