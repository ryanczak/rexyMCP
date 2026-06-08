# Phase 08a: Context-efficiency aggregation onto `PhaseRun`

**Milestone:** M10 — Context optimization
**Status:** done
**Depends on:** phase-03 (`OutputFiltered`), phase-04 (`ReadEvicted`), phase-06 (`ReadDeduped`), phase-07 (`Compaction` shape), phase-05 (`Budget::estimate` fix so `context_pct` is real)
**Estimated diff:** ~230 lines (incl. tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Capture M10's context-efficiency signal onto the cross-project `PhaseRun`
telemetry record so it becomes comparable **across runs**. At phase end, read
the durable session JSONL the loop just wrote, aggregate the per-lever reclaim
events (`OutputFiltered`, `ReadEvicted`, `ReadDeduped`) + `Compaction` + the
per-turn `Metrics` peak into a new `ContextEfficiency` value, and fold it onto
the `PhaseRun`. This is the **data-capture half** of the original roadmap
phase-08; surfacing it in `rexymcp runs` / the scorecard (08b) and in the live
`StatusSummary`/dashboard (08c) are separate follow-on phases — see Out of scope.

Why now: every M10 lever (phases 01–07) now emits a durable per-lever
`SessionEvent`, but none of it lands on `PhaseRun`, so M10's effect is not yet
measurable across runs (an M10 exit criterion). This phase closes that gap on
the storage side.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` record is the cross-run substrate; this phase adds objective fields
  the executor fills at phase end.
- `docs/architecture.md` § "Session log & troubleshooting tools" — the JSONL is
  the durable single source; we read it back rather than thread new counters.
- `docs/dev/milestones/M10-context-optimization/README.md` § "Phases" (row 08)
  and § "What is novel to rexyMCP (Arc B)" item 4 ("Scorecard-measured
  optimization").

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The reclaim events already exist in the JSONL (do not add or change them)

`executor/src/store/sessions/event.rs` defines `SessionRecord` and the
`SessionEvent` variants this phase reads. **They are already implemented and
serialized — this phase only reads them back.** The relevant variants, verbatim:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub ts: u64,
    pub turn: usize,
    pub event: SessionEvent,
}

// (inside enum SessionEvent, #[serde(tag = "event_type", rename_all = "snake_case")])
    Metrics {
        input_tokens: u32,
        output_tokens: u32,
        context_pct: f64,
        context_used: u32,
        context_window: u32,
    },
    Compaction {
        tokens_before: usize,
        tokens_after: usize,
        messages_signaturized: usize,
        messages_evicted: usize,
    },
    OutputFiltered {
        tokens_before: usize,
        tokens_after: usize,
        filter: String,
    },
    ReadEvicted {
        path: String,
        reads_evicted: usize,
        tokens_reclaimed: usize,
    },
    ReadDeduped {
        path: String,
        tokens_saved: usize,
        prior_turn: usize,
    },
```

### Reading a session log back — the function to use (already exists)

`executor/src/store/sessions/jsonl.rs:54`:

```rust
pub fn read_session_log(path: &Path) -> std::io::Result<Vec<SessionRecord>> {
    // ... reads the JSONL, returns Ok(Vec::new()) for a missing file,
    //     silently skips blank/corrupt lines ...
}
```

### Where the loop opens the session log — this is how you reconstruct the path

In `executor/src/agent/mod.rs` (`execute_phase`, around line 146):

```rust
let log_dir = deps.project_root.join(".rexymcp").join("sessions");
let log_handle: Option<SessionLogHandle> =
    open_session_log(&log_dir, &format!("{}-{}", input.phase, deps.session_id)).ok();
```

And `SessionLogger::open` (`executor/src/store/sessions/jsonl.rs:32`) turns that
id into the final filename:

```rust
let path = log_dir.join(format!("session-{session_id}.jsonl"));
```

So the on-disk log path is **deterministically**:

```
<project_root>/.rexymcp/sessions/session-<phase>-<session_id>.jsonl
```

This phase reconstructs that exact path inside `emit_phase_run` — **no signature
change, no call-site churn** (see Spec task 3 and the Gotcha below).

### The `PhaseRun` record and where it's built

`executor/src/store/telemetry.rs:36` defines `PhaseRun` (a flat record with
`#[serde(default)]` on the recently-added optional provenance fields
`served_model` / `length_finish_rate` / `context_window` — the same additive
pattern this phase follows). It is constructed in exactly one production site:
`executor/src/agent/metrics.rs:90`, inside `emit_phase_run`:

```rust
pub(super) fn emit_phase_run(
    deps: &LoopDeps<'_>,
    input: &PhaseInput,
    status: &str,
    gates: Gates,
    metrics: &RunMetrics,
    scorer: &Scorer,
    turns: usize,
) {
    let Some(dir) = deps.telemetry_dir else {
        return;
    };
    // ... rate computations ...
    let run = PhaseRun {
        ts: now,
        model: deps.model.to_string(),
        // ... all existing fields ...
        context_window: deps.context_window,
    };
    let _ = telemetry::append(dir, &run);
}
```

`emit_phase_run` already has `deps.project_root`, `deps.session_id`, and
`input.phase` in scope — everything needed to reconstruct the log path. It is
called from **9 sites** in `mod.rs`, all on the terminal path **after**
`log_session_end` has flushed, so by the time it runs every reclaim/compaction/
metrics record is already on disk. **Its signature does not change in this phase**
— see the Gotcha.

## Spec

Numbered tasks in execution order.

### 1. Define `ContextEfficiency` and add the pure aggregator — `executor/src/store/telemetry.rs`

At the top of the file, extend the imports to bring in the session-log types
(no cycle — `store::sessions` does not import `store::telemetry`):

```rust
use crate::store::sessions::event::{SessionEvent, SessionRecord};
```

Add a new struct (place it just above `PhaseRun`):

```rust
/// Context-efficiency signal for one run, aggregated from the session JSONL at
/// phase end (M10). All token figures are chars/4 estimates, consistent with the
/// per-lever events that produce them. Nested in `PhaseRun` as a single
/// `#[serde(default)]` field so legacy records (and every struct literal) need
/// only `Default`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextEfficiency {
    /// Highest `context_pct` observed across the run's per-turn `Metrics`
    /// events; `0.0` if none were emitted.
    pub peak_context_pct: f64,
    /// Number of `Compaction` events the loop emitted.
    pub compaction_count: usize,
    /// Tokens freed by compaction: Σ(tokens_before − tokens_after) over
    /// `Compaction` events.
    pub compaction_tokens_reclaimed: usize,
    /// Tokens reclaimed by the Arc-A boundary output filter: Σ(tokens_before −
    /// tokens_after) over `OutputFiltered` events.
    pub output_filtered_tokens: usize,
    /// Tokens reclaimed by superseded-read eviction: Σ tokens_reclaimed over
    /// `ReadEvicted` events.
    pub read_evicted_tokens: usize,
    /// Tokens saved by redundant-read dedupe: Σ tokens_saved over
    /// `ReadDeduped` events.
    pub read_deduped_tokens: usize,
}
```

Add the pure aggregator (place it below `read`, above `#[cfg(test)]`):

```rust
/// Aggregate the context-efficiency signal from a run's session-log records.
/// Pure over the slice; an empty slice yields `ContextEfficiency::default()`.
pub fn aggregate_context_efficiency(records: &[SessionRecord]) -> ContextEfficiency {
    let mut eff = ContextEfficiency::default();
    for rec in records {
        match &rec.event {
            SessionEvent::Metrics { context_pct, .. } => {
                eff.peak_context_pct = eff.peak_context_pct.max(*context_pct);
            }
            SessionEvent::Compaction {
                tokens_before,
                tokens_after,
                ..
            } => {
                eff.compaction_count += 1;
                eff.compaction_tokens_reclaimed += tokens_before.saturating_sub(*tokens_after);
            }
            SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                ..
            } => {
                eff.output_filtered_tokens += tokens_before.saturating_sub(*tokens_after);
            }
            SessionEvent::ReadEvicted {
                tokens_reclaimed, ..
            } => {
                eff.read_evicted_tokens += *tokens_reclaimed;
            }
            SessionEvent::ReadDeduped { tokens_saved, .. } => {
                eff.read_deduped_tokens += *tokens_saved;
            }
            _ => {}
        }
    }
    eff
}
```

Use `.max(...)` (not a manual `if >` compare) for `peak_context_pct` — clippy
prefers it.

### 2. Add the field to `PhaseRun` — `executor/src/store/telemetry.rs`

Append one field to the struct, after `context_window`:

```rust
    /// Context-efficiency signal aggregated from the session JSONL at phase end
    /// (M10/phase-08a). Default (all zeros) for legacy records and for runs that
    /// produced no reclaim/metrics events.
    #[serde(default)]
    pub context_efficiency: ContextEfficiency,
```

### 3. Aggregate and set it in `emit_phase_run` — `executor/src/agent/metrics.rs`

Add the import at the top of the file:

```rust
use crate::store::sessions::jsonl::read_session_log;
```

Just before the `let run = PhaseRun { ... };` block, reconstruct the log path
(mirroring `execute_phase` / `SessionLogger::open`), read it back best-effort,
and aggregate:

```rust
    // Aggregate the context-efficiency signal from the durable session log the
    // loop just wrote. Best-effort: a missing/unreadable log yields the default
    // (all zeros) — telemetry never fails the phase. The path must mirror what
    // `execute_phase` passed to `open_session_log` (see that call + `SessionLogger::open`).
    let log_path = deps
        .project_root
        .join(".rexymcp")
        .join("sessions")
        .join(format!("session-{}-{}.jsonl", input.phase, deps.session_id));
    let context_efficiency = read_session_log(&log_path)
        .map(|recs| telemetry::aggregate_context_efficiency(&recs))
        .unwrap_or_default();
```

Then add the field to the `PhaseRun` literal (after `context_window`):

```rust
        context_window: deps.context_window,
        context_efficiency,
```

**Do not** change `emit_phase_run`'s signature and **do not** touch any of its 9
call sites — the path is reconstructed internally. (See the Gotcha.)

### 4. Fix the executor-side struct literal — `executor/src/store/telemetry.rs`

The `sample()` test helper (in the `#[cfg(test)] mod tests` block, ~line 111)
constructs `PhaseRun { ... }` literally and will stop compiling. Add one line
after `context_window: None,`:

```rust
            context_efficiency: Default::default(),
```

### 5. Fix the mcp-crate struct literals (test-only) — keep `cargo test` green

Adding a field to `PhaseRun` breaks every struct literal of it. `cargo build`
(non-test) stays green — the mcp crate constructs `PhaseRun` only in `#[cfg(test)]`
helpers — but `cargo test` will not compile until these are fixed. There are
**five** such literals; add `context_efficiency: Default::default(),` to each
(use `Default::default()` so no new import is needed):

- `mcp/src/runs.rs:163` — `make_run`
- `mcp/src/runs.rs:201` — `make_run_with_params`
- `mcp/src/scorecard.rs:343` — the test `make_run`
- `mcp/src/scorecard.rs:728` — the second test `make_run`
- `mcp/src/scorecard_cli.rs:88` — the test `make_run`

This list is grep-verified complete (`grep -rn "PhaseRun {" executor/src mcp/src`).
Run `cargo build` after task 3, then `cargo test --no-run` (or `cargo build
--tests`) after this task to confirm all literals compile before writing new
tests. Do **not** add any other behavior to these mcp helpers — they are
untouched except for the one defaulted field. Surfacing the new field in the
`runs`/scorecard output is **08b, out of scope here**.

## Acceptance criteria

- [x] `cargo build` succeeds with zero new warnings.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] `cargo fmt --all --check` passes (only files this phase touched).
- [x] `cargo test` passes (existing + new).
- [x] `PhaseRun` carries a `context_efficiency: ContextEfficiency` field with
      `#[serde(default)]`; a legacy JSONL line lacking the field deserializes to
      `ContextEfficiency::default()`.
- [x] `aggregate_context_efficiency` sums each reclaim source independently and
      takes the **max** (not last) `context_pct` for `peak_context_pct`.
- [x] A real `execute_phase`/loop run with telemetry enabled persists a
      `PhaseRun` whose `context_efficiency` equals an independent aggregation of
      that run's on-disk session log.

## Test plan

Unit tests in `executor/src/store/telemetry.rs` `mod tests` (pure aggregator —
build `SessionRecord`s by hand; hermetic, no IO):

- `aggregate_context_efficiency_empty_is_default` — empty slice →
  `ContextEfficiency::default()`.
- `aggregate_context_efficiency_peak_is_max_not_last` — three `Metrics` events
  with `context_pct` 0.4, 0.9, 0.2 → `peak_context_pct == 0.9`.
- `aggregate_context_efficiency_sums_compaction` — two `Compaction` events
  (e.g. 1000→600 and 500→500) → `compaction_count == 2`,
  `compaction_tokens_reclaimed == 400`.
- `aggregate_context_efficiency_sums_each_reclaim_source_independently` — one
  `OutputFiltered` (before/after), one `ReadEvicted` (`tokens_reclaimed`), one
  `ReadDeduped` (`tokens_saved`) → each of `output_filtered_tokens`,
  `read_evicted_tokens`, `read_deduped_tokens` equals its source's figure and
  the others are unaffected (the **must-not-cross-contaminate** case).
- `aggregate_context_efficiency_ignores_unrelated_events` — a slice of only
  `Prompt`/`Completion`/`ToolResult`/`Verify` records → all-zero default (the
  **must-NOT-count** case proving the `_ => {}` arm is correct).
- `phase_run_without_context_efficiency_deserializes` — a legacy JSONL line
  (reuse the existing `phase_run_without_context_window_deserializes` fixture
  string, which omits `context_efficiency`) deserializes with
  `run.context_efficiency == ContextEfficiency::default()`.

Integration / end-to-end test in `executor/src/agent/mod.rs` `mod tests` — see
the next section for the required real-artifact verification.

## End-to-end verification

The pure aggregator's unit tests can pass while `emit_phase_run` reconstructs the
**wrong** path (and silently aggregates an empty log → all-zeros). The E2E test
must exercise the real read-back path:

Add `phase_run_context_efficiency_matches_session_log` to
`executor/src/agent/mod.rs` `mod tests`. Model it on the existing
`context_window_recorded_from_loop_deps` (`mod.rs:3753`) + `read_runs(&telem)`
pattern and the existing `records(...)` / `log_path(...)` session-log test
helpers (`mod.rs:987`, `:1490`):

1. Drive a real loop with telemetry enabled (the `run_full_with_context_window`
   / `read_runs` harness the provenance tests use), producing both a session log
   and one persisted `PhaseRun`.
2. Independently read that run's session log off disk (`read_session_log` via the
   existing `records(...)`/`log_path(...)` helper) and compute the expected value
   with `telemetry::aggregate_context_efficiency`.
3. Assert the persisted `runs[0].context_efficiency` **equals** the independently
   computed `ContextEfficiency` (compare directly — the struct derives
   `PartialEq`).

This is strong because a wrong-path reconstruction makes `emit_phase_run` read an
empty/absent log (all-zeros) while the independent read of the correct path finds
the loop's per-turn `Metrics` events (non-zero `peak_context_pct`) — the equality
assertion then fails. Because `context_pct` is deterministic from the injected
budget, the test is deterministic.

> If the existing `run_full*` harness's budget is too large to make the
> assertion discriminating (e.g. `peak_context_pct` rounds to `0.0`), prefer
> additionally forcing **one** `Compaction` by driving `execute_phase` directly
> with a small `Budget::new(<small>)` (the `DepsBuilder` path at `mod.rs:3200`)
> and asserting `runs[0].context_efficiency.compaction_count >= 1`. The
> equality test above is the required form; this is a stronger optional addition,
> not a substitute. Quote the actual `runs[0].context_efficiency` in the
> completion Update Log.

## Authorizations

None. (No new dependencies; no architecture-doc edit; `PhaseRun` and
`SessionEvent` already live in this crate.)

## Out of scope

What this phase must **not** do, even if tempted:

- **Do not surface `context_efficiency` in `rexymcp runs`, `model_scorecard`, or
  `rexymcp scorecard`.** That is **phase-08b** (mcp-only: display columns + the
  two scorecard accumulators/row structs/renderers). Touch the five mcp struct
  literals **only** to add the one defaulted field (task 5) — no display, no
  aggregation, no new columns.
- **Do not fold the reclaim variants into `StatusSummary` / `summarize` or the
  dashboard.** That is **phase-08c** (mcp-only live view). The three reclaim
  variants stay in `summarize`'s `_ => {}` catch-all for now.
- **Do not change `emit_phase_run`'s signature or any of its 9 call sites.**
  Reconstruct the path internally.
- **Do not add, rename, or change any `SessionEvent` variant, the `Metrics`
  fields, or the emit sites.** This phase only reads them.
- **Do not change the compactor, the output filter, the eviction/dedupe logic,
  or `Budget::estimate`.** Those are phases 01–07, done.
- **Do not thread new counters through the loop / `RunMetrics`.** The design is
  read-the-JSONL-back, per the milestone README.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-08 (complete)

**Summary:** The executor (Qwen/Qwen3.6-27B-FP8) completed tasks 1–4 (ContextEfficiency struct + aggregator in `telemetry.rs`, path reconstruction + field in `emit_phase_run`, and the `telemetry.rs` sample fixture) plus the 2 of 5 mcp literals in `runs.rs` before hard-failing on IdenticalToolCallRepetition while reading `scorecard.rs`. Architect closed out: added `context_efficiency: Default::default()` to the 3 remaining mcp struct literals (`scorecard.rs` ×2, `scorecard_cli.rs` ×1), wrote all 7 tests (6 unit in `telemetry.rs` + 1 E2E in `mod.rs`), and applied the one `rustfmt` fix. No spec deviations.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3
   Compiling rexymcp v0.1.3
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.83s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -15
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.27s

cargo test 2>&1 | tail -5
test result: ok. 664 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.05s
```

**End-to-end verification:** `phase_run_context_efficiency_matches_session_log` drives a real loop with `Budget::new(1_000_000)` and `context_window = Some(262_144)`, which produces a `Metrics` event with non-zero `context_pct` (loop tokens / 1_000_000). It reads the session log independently and asserts `runs[0].context_efficiency == aggregate_context_efficiency(&session_records)`. This assertion would fail if `emit_phase_run` reconstructed the wrong path (all-zeros vs. non-zero `peak_context_pct`). Test passes.

**Files changed:**
- `executor/src/store/telemetry.rs` — `ContextEfficiency` struct + `aggregate_context_efficiency` fn + `context_efficiency` field on `PhaseRun` + 6 unit tests
- `executor/src/agent/metrics.rs` — log-path reconstruction + `context_efficiency` field in `emit_phase_run`
- `mcp/src/runs.rs` — `context_efficiency: Default::default()` in 2 test helpers
- `mcp/src/scorecard.rs` — `context_efficiency: Default::default()` in 2 test helpers
- `mcp/src/scorecard_cli.rs` — `context_efficiency: Default::default()` in 1 test helper

**New tests:**
- `aggregate_context_efficiency_empty_is_default` in `executor/src/store/telemetry.rs`
- `aggregate_context_efficiency_peak_is_max_not_last` in `executor/src/store/telemetry.rs`
- `aggregate_context_efficiency_sums_compaction` in `executor/src/store/telemetry.rs`
- `aggregate_context_efficiency_sums_each_reclaim_source_independently` in `executor/src/store/telemetry.rs`
- `aggregate_context_efficiency_ignores_unrelated_events` in `executor/src/store/telemetry.rs`
- `phase_run_without_context_efficiency_deserializes` in `executor/src/store/telemetry.rs`
- `phase_run_context_efficiency_matches_session_log` in `executor/src/agent/mod.rs`

**Commits:**
- `14b4668` — feat: context-efficiency aggregation onto PhaseRun (M10/phase-08a)
- `1ccba39` — docs: phase-08a doc split (08a/08b/08c) and NEXT.md pointer update

**Notes for review:** Executor stalled on `IdenticalToolCallRepetition` (3× `read_file scorecard.rs`) — same pattern as phase-06 mechanical literal churn. Architect closed out the 3 remaining literals and all 7 tests. The `context_pct` in the E2E test is `used_tokens / 1_000_000` (small but non-zero), making the path-reconstruction assertion discriminating.

### Review verdict — 2026-06-08

- **Verdict:** escalated (architect closeout after executor `hard_fail`)
- **Bounces:** one executor dispatch (`hard_fail` — `IdenticalToolCallRepetition`, 3× `read_file scorecard.rs`); architect completed tasks 5 (3 remaining mcp literals) + all 7 tests, no re-dispatch
- **Executor:** Qwen/Qwen3.6-27B-FP8 (tasks 1–4 + 2/5 mcp literals); Claude Code (direct) closeout
- **Scope deviations:** none — implementation matches the spec verbatim (struct, aggregator, `#[serde(default)]` field, internal path reconstruction with no call-site churn)
- **Review checks:** all four gates re-run clean (`fmt --check`, `build`, `clippy -D warnings`, `test` = 243 mcp + 664 executor, deterministic across repeated full-suite runs). The E2E test `phase_run_context_efficiency_matches_session_log` was **mutation-verified**: injecting a wrong log-path prefix into `emit_phase_run` makes the persisted `context_efficiency` all-zeros while the independent on-disk read finds `peak_context_pct: 0.00336`, failing the equality assertion — confirming the test exercises the real read-back path, not a fake. The `unwrap_or_default()` in `emit_phase_run` is the phase-authorized best-effort telemetry read (documented; telemetry must never fail the phase) — not a STANDARDS §2.1 violation.
- **Calibration:** 4th occurrence of the mechanical-multi-site-churn stall class (after phase-03/04/06 match-arm walls + phase-10b test-churn). This one was **struct-literal** churn (5 mcp `PhaseRun {…}` literals), not match-arm — the phase's no-call-site-churn `emit_phase_run` design successfully avoided the match-arm wall, but the 5 defaulted-field literal adds were still enough repetitive churn to trigger the read-repetition stall. Reinforces the held-fold discussion: for any field add touching N>2 struct literals across crates, consider pre-applying the mechanical literal adds in the phase doc as pre-completed work, or splitting the cross-crate literal fixups into a separate micro-step.
