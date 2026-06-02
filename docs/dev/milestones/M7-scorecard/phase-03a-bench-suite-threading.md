# Phase 03a: thread `bench_suite` through the loop + stamp a single benchmarked run

**Milestone:** M7 — Model scorecard & routing
**Status:** todo
**Depends on:** phase-02 (done). `PhaseRun.bench_suite` and the scorecard
`SourceFilter` exist; this phase makes the executor loop actually *write* a
non-`None` value when asked.
**Estimated diff:** ~220 lines (one `LoopDeps` field + the emit read + the
runner threading chain + a CLI flag + tests, plus mechanical
`bench_suite: None` additions to existing `LoopDeps` constructions).
**Tags:** language=rust, kind=feature, size=m

## Goal

Complete the data-path deferral from phase-02: thread a `bench_suite` value
through `LoopDeps` and `run_phase` so that a single phase run can be **stamped**
as a benchmark run, and `emit_phase_run` writes `bench_suite: Some(name)` into
the `PhaseRun` record instead of the hardcoded `None`. Expose it as a
`--bench-suite <name>` flag on the existing `run-phase` CLI subcommand.

This is the run **primitive** the phase-03b benchmark sweep is built on: 03b's
multi-model runner will call this same stamped path once per (model, benchmark
phase). This phase does **not** build the sweep, the fixture repo, or the
`bench` command — see Out of scope.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Model effectiveness metrics & routing" → "Benchmark
  vs. telemetry" — *"Both emit the same `PhaseRun` schema; small models are
  high-variance, so a routing decision needs a minimum sample size."* The
  `bench_suite` stamp is what lets one store hold both, written by the runner.
- The phase-02 doc (`phase-02-benchmark-provenance.md`) § "Out of scope" pinned
  this exact deferral: *"Threading `bench_suite` through `LoopDeps` /
  `emit_phase_run` so a live run can be stamped with a suite — phase-03, where
  the runner that needs it lives."* This phase is that thread.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching code.
4. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### `LoopDeps` (`executor/src/agent/mod.rs:80`)

The loop's dependency bundle. Note the existing borrowed-scalar idiom
(`model: &'a str`, `telemetry_dir: Option<&'a Path>`) — the new field follows
it:

```rust
pub struct LoopDeps<'a> {
    pub client: &'a dyn AiClient,
    // …
    pub model: &'a str,
    // …
    pub generation_params: GenerationParams,
    /// Cross-project telemetry dir for the `PhaseRun` record; `None` disables it.
    pub telemetry_dir: Option<&'a Path>,
    pub progress: Option<&'a dyn ProgressCallback>,
}
```

### The emit site (`executor/src/agent/mod.rs:1189`, inside `emit_phase_run`)

`emit_phase_run` (`mod.rs:1153`) builds the `PhaseRun`. phase-02 hardcoded the
last field:

```rust
    let run = PhaseRun {
        ts: now,
        model: deps.model.to_string(),
        generation_params: deps.generation_params.clone(),
        // …
        architect_verdict: None,
        bench_suite: None,            // ← phase-02 hardcoded; this phase reads deps
    };
    let _ = telemetry::append(dir, &run);
```

### The runner threading chain (`mcp/src/runner.rs`)

`run_phase` → `run_phase_with` assembles `LoopDeps` from two grouped structs.
`RunPhaseConfig` (`runner.rs:200`) is the public entry; `AssemblyInput`
(`runner.rs:103`) is the inner non-seam bundle; the `LoopDeps` literal is at
`runner.rs:177`. The chain mirrors how `telemetry_dir` already flows:

```rust
// RunPhaseConfig (public)         runner.rs:200
pub telemetry_dir: Option<&'a Path>,
// AssemblyInput (inner)           runner.rs:103
telemetry_dir: Option<&'a Path>,
// LoopDeps literal                runner.rs:191
telemetry_dir: inp.telemetry_dir,
// run_phase wires config→assembly runner.rs:252
telemetry_dir: inp.telemetry_dir,
```

### The CLI handler (`mcp/src/main.rs:109`)

The `RunPhase` subcommand (CLI: `run-phase`) is defined at `main.rs:34` and
handled at `main.rs:109`. It builds a `RunPhaseConfig` and prints the
`PhaseResult` JSON.

### `PhaseRun` construction / `LoopDeps` construction sites that will need the new field

Adding a non-`Default` field to `LoopDeps` makes **every existing struct-literal
construction** fail `E0063` until updated. They are all in test code; each just
gets `bench_suite: None,`:

- `executor/src/agent/mod.rs` — the `LoopDeps { … }` literals in tests (around
  lines 1308, 2024, 2136, 2601, 3226, 3268, 3301 — grep `LoopDeps {` and the
  builder structs that mirror its fields to find them all).
- `mcp/src/runner.rs` — the production `LoopDeps` literal at `runner.rs:177`
  (gets `bench_suite: inp.bench_suite,`) and any test that builds
  `RunPhaseConfig`/`AssemblyInput`.

Grep before and after: `grep -rn 'LoopDeps {' executor mcp` and
`grep -rn 'AssemblyInput\|RunPhaseConfig' mcp` to be exhaustive — a missed site
is an `E0063` build break (this is exactly the class of miss that bounced
phase-02; do the field add first, then fix every construction the compiler
flags before moving on).

## Spec

### Task 1 — Add `bench_suite` to `LoopDeps` and read it at emit

In `executor/src/agent/mod.rs`:

1. Add the field to `LoopDeps` (after `telemetry_dir`, before `progress`), using
   the **borrowed** form to match `model: &'a str`:

   ```rust
       /// Benchmark provenance to stamp on the emitted `PhaseRun`. `None` = a
       /// production run; `Some(name)` = a controlled benchmark run of suite
       /// `name`. The phase-03b sweep sets this; production callers pass `None`.
       pub bench_suite: Option<&'a str>,
   ```

2. In `emit_phase_run` (`mod.rs:1189`), replace the hardcoded
   `bench_suite: None,` with a read from `deps`, converting borrow → owned:

   ```rust
       bench_suite: deps.bench_suite.map(|s| s.to_string()),
   ```

3. Add `bench_suite: None,` to every existing `LoopDeps` literal the compiler
   now flags (all in tests — see Current state). Where a test uses a builder
   struct that mirrors `LoopDeps` fields, add the field there too.

### Task 2 — Thread it through the runner

In `mcp/src/runner.rs`, follow the existing `telemetry_dir` flow exactly:

1. `RunPhaseConfig` (`runner.rs:200`) — add `pub bench_suite: Option<&'a str>,`.
2. `AssemblyInput` (`runner.rs:103`) — add `bench_suite: Option<&'a str>,`.
3. The `LoopDeps` literal in `run_phase_with` (`runner.rs:177`) — add
   `bench_suite: inp.bench_suite,`.
4. `run_phase` (`runner.rs:213`) — when building `AssemblyInput`, add
   `bench_suite: inp.bench_suite,`.
5. Update every existing `RunPhaseConfig { … }` construction (in
   `mcp/src/main.rs:120` and any test) to add `bench_suite: None,`.

### Task 3 — Expose `--bench-suite` on the `run-phase` CLI

In `mcp/src/main.rs`:

1. Add an optional arg to the `RunPhase` variant (`main.rs:34`):

   ```rust
           /// Stamp emitted PhaseRun records as a benchmark run of this suite.
           /// Omit for a normal production run.
           #[arg(long)]
           bench_suite: Option<String>,
   ```

2. In the `RunPhase` handler (`main.rs:109`), destructure the new field and pass
   it through: `bench_suite: bench_suite.as_deref(),` in the `RunPhaseConfig`.

## Acceptance criteria

- [ ] `LoopDeps` has `pub bench_suite: Option<&'a str>`.
- [ ] `emit_phase_run` writes `bench_suite: deps.bench_suite.map(|s| s.to_string())`
      — a run with `deps.bench_suite == Some("smoke")` emits a `PhaseRun` whose
      `bench_suite == Some("smoke")`; a run with `None` emits `None` (the
      production case — the must-NOT-stamp boundary).
- [ ] `RunPhaseConfig` and `AssemblyInput` carry `bench_suite: Option<&'a str>`,
      threaded into the `LoopDeps` literal.
- [ ] `run-phase --bench-suite <name>` is accepted by the CLI and flows to the
      emitted record; omitting it yields a production (`None`) record.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

The codebase already has a full-loop telemetry harness: `run_full`
(`executor/src/agent/mod.rs:2589`) drives `execute_phase` against a
`MockAiClient` with a `telemetry_dir`, and `read_runs` (`mod.rs:2844`) reads the
emitted `phase_runs.jsonl` back into `Vec<PhaseRun>`. Reuse it — thread a
`bench_suite` argument through `run_full` (or add a sibling helper) rather than
inventing a new harness.

In `executor/src/agent/mod.rs` `#[cfg(test)] mod tests`:

- `emit_stamps_bench_suite_when_set` — run a full phase with
  `bench_suite = Some("smoke")` and a `telemetry_dir`; `read_runs` returns one
  record with `bench_suite == Some("smoke".to_string())`. (Must-match.)
- `emit_leaves_bench_suite_none_for_production` — same harness with
  `bench_suite = None`; the emitted record has `bench_suite == None`. (Must-NOT
  — the production boundary; this is the case every existing telemetry test
  already exercises implicitly, so make it explicit here.)

In `mcp/src/runner.rs` `#[cfg(test)] mod tests`:

- `run_phase_threads_bench_suite_to_emit` — extend the existing
  `run_phase_with_assembles_and_returns_result` pattern (MockAiClient,
  `NoopVerifier`/`NoopRunner`) with a `telemetry_dir` set and
  `bench_suite: Some("smoke")` on the `AssemblyInput`; assert the emitted
  `PhaseRun` read back from the telemetry dir has `bench_suite == Some("smoke")`.
  If the existing integration test passes `telemetry_dir: None` (no emission),
  add the telemetry dir in the new test so emission actually happens.

Per `STANDARDS.md` §3: the new behavior (stamp when set) gets both a must-match
(`Some` → `Some`) and a must-NOT (`None` → `None`) test.

## End-to-end verification

This phase ships a **real CLI surface** (the `--bench-suite` flag), so verify it
against the built binary, not just the hermetic tests:

1. Build, then run `cargo run -p rexymcp -- run-phase --help` and confirm the
   `--bench-suite` flag appears in the output. Quote the relevant help line in
   the completion Update Log.

The stamped-emission *behavior* (flag value → `PhaseRun.bench_suite`) is covered
definitively by the hermetic `run_full` / runner tests above — a full real
`run-phase` invocation needs a live LLM endpoint and is therefore not part of
this phase's hermetic verification (the phase-03b sweep is where live benchmark
runs are exercised).

## Authorizations

- [x] May edit `executor/src/agent/mod.rs` — add the `LoopDeps` field, the emit
      read, and update test constructions + the `run_full` harness.
- [x] May edit `mcp/src/runner.rs` — thread the field through
      `RunPhaseConfig` / `AssemblyInput` / `LoopDeps`, plus tests.
- [x] May edit `mcp/src/main.rs` — add the `--bench-suite` CLI arg and pass it
      through.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.

## Out of scope

- **The `rexymcp bench` sweep command** — phase-03b. This phase only stamps a
  *single* run via the existing `run-phase` path; the multi-model iteration is
  03b.
- **The benchmark fixture repo + curated benchmark phase docs** — phase-03b.
  Decided strategy (for 03b, do not build now): a pristine fixture tree checked
  in under `benchmarks/`, copied into a fresh `tempfile::TempDir` per run for
  hermetic isolation.
- **Validating that `bench_suite` names a *real* suite** — there is no suite
  registry yet (03b introduces one). This phase accepts any string; do not add
  validation, an enum, or a lookup. (`STANDARDS.md` §2.2 — no error handling for
  cases that can't yet happen; no wired-in validation with no registry to check
  against.)
- **A `bench_suite` param on the `execute_phase` MCP tool** — the MCP tool is
  the architect's production dispatch path, not the benchmark path; the sweep
  (03b) drives the CLI/library directly. Do not touch `mcp/src/server.rs`.
- **Threading any *other* new field** — only `bench_suite`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
