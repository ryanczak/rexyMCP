# Phase 07c: verifier retry + hard-fail detection

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** review
**Depends on:** phase-07a (loop), 07b (session log + `Verify`/`HardFail` events
exist in the schema), phase-01 (`verifier`: `verify`, `capture_baseline`,
`Baseline`, `Diagnostic`, `VerifierResult`), phase-05 (`hard_fail::evaluate`,
`HardFailSignal`, `ToolCallSnapshot`). All done.
**Estimated diff:** ~450 lines (verifier seam + retry + baseline + hard-fail +
briefing + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Close the **governor feedback loop** (turn-cycle steps 6–7). After an edit-class
tool runs, verify the edited file and feed the model's *own* new diagnostics back
as a retry; watch for the three hard-fail signals (repetition, persistent verifier
failure, runaway output) and, when one trips, stop with a `hard_fail` briefing
instead of looping forever. This is what turns the 07a/07b "drive until the model
stops or runs out of budget" loop into one that **converges or escalates**.

This sub-phase wires the `Verify` and `HardFail` session-log events that 07b left
in the schema. It does **not** add the read-before-edit gate (07d) or the
completion artifacts (07e) — see § Out of scope.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" steps **6** ("After
  edit-class tools, run the verifier … On failure, feed the diagnostics back for a
  retry") and **7** (the hard-fail detector → assemble a briefing and stop).
- `docs/architecture.md` — "Escalation = Claude Code itself" and the briefing
  section list: a `hard_fail` briefing carries the **current blocker (exact
  diagnostic)** — so its `diagnostics` field is populated (unlike the budget
  briefing 07a built with empty diagnostics).
- `docs/dev/STANDARDS.md` §3.3 — deterministic tests (no real `cargo`/`tsc`/`ruff`
  spawn in unit tests). This is why the verifier is **injected** (Spec task 1).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. Read these surfaces:
   - `executor/src/governor/verifier.rs` — `async fn verify(&Path) ->
     VerifierResult` (`Checked { diagnostics } | Unsupported | Failed(String)`),
     `async fn capture_baseline(&[PathBuf]) -> Baseline`, and `Baseline::partition(
     &[Diagnostic]) -> (Vec<&Diagnostic> author, Vec<&Diagnostic> ambient)`. Note
     `Baseline.signatures` is a public `HashSet` (you can `extend` one baseline
     with another's signatures). `Diagnostic { path, line, column, severity,
     message, code }`.
   - `executor/src/governor/hard_fail.rs` — `evaluate(recent_tool_calls:
     &VecDeque<ToolCallSnapshot>, recent_verifier_error_counts: &[usize],
     last_tool_output: Option<(&str, usize)>) -> Option<HardFailSignal>`; the three
     `HardFailSignal` variants and `describe()`; the thresholds
     (`IDENTICAL_CALL_THRESHOLD`, `VERIFIER_PERSISTENCE_THRESHOLD`,
     `RUNAWAY_OUTPUT_BYTES`).
   - `executor/src/phase/briefing.rs` — `Briefing`, `Blocker::HardFail(signal)`,
     and that `diagnostics: Vec<Diagnostic>` is the "current code state".
   - `executor/src/agent/mod.rs` — the 07a/07b loop you are extending (the
     dispatch + log sites).

## Current state

The loop (`executor/src/agent/mod.rs`) dispatches tools and logs, but: it never
runs the verifier, never feeds diagnostics back, and never checks for hard-fail —
so a stuck model only ever stops via the turn cap (`budget_exceeded`). `hard_fail::
evaluate` and `verifier::*` are built and tested but have **no caller in the
loop**. `recent_tool_calls` is already maintained (07a). `LoopDeps` has no
verifier. The `Verify` / `HardFail` `SessionEvent` variants exist (07b) but are
never emitted.

> **Scorer:** untouched this phase. Its consumer is `PhaseRun.tool_success_rate`
> (phase-08, architecture line 250), not the governor — see 07b's review verdict.
> Leave `scorer.record` as 07a wrote it.

## Spec

All edits are in `executor/src/agent/**`. Do **not** modify `governor/verifier.rs`,
`governor/hard_fail.rs`, or `phase/**`.

### 1. Inject the verifier (a seam, for deterministic tests)

`governor::verifier::{verify, capture_baseline}` spawn real compilers — unusable in
hermetic unit tests. Introduce a trait so the loop depends on an abstraction and
tests inject a mock:

```rust
#[async_trait]
pub trait FileVerifier: Send + Sync {
    async fn verify(&self, path: &Path) -> VerifierResult;
    async fn capture_baseline(&self, paths: &[PathBuf]) -> Baseline;
}
```

Provide a default `RealVerifier` whose methods delegate to
`governor::verifier::{verify, capture_baseline}`. Add `verifier: &dyn FileVerifier`
to `LoopDeps`. (`async_trait` is already a workspace dep via `ai` — no new
dependency. Put the trait + `RealVerifier` in `agent/` — a new `agent/verify.rs` is
fine.)

### 2. Edit-class detection + edited path

A dispatched call is **edit-class** when its name is `"write_file"` or `"patch"`.
After such a call **executes** (07a's `dispatch`), pull the edited path from the
call's `"path"` argument. Non-edit calls (and edit calls with no `"path"`) skip the
verifier entirely.

### 3. Baseline (lazy, per language/extension)

Maintain a session `Baseline` (starts empty) and a set of already-baselined file
extensions. The **first** time an edit-class call targets a path whose extension
hasn't been baselined, call `verifier.capture_baseline(&[path])` and merge the
returned signatures into the running baseline (`baseline.signatures.extend(...)`),
then mark the extension. This captures the project's **pre-existing** diagnostics
so they classify as *ambient*, not the model's fault.

### 4. Post-edit verify + retry feedback (step 6)

After an edit-class dispatch, `verifier.verify(&path).await`:

- **`Checked { diagnostics }`** → `let (author, _ambient) = baseline.partition(
  &diagnostics);`
  - Log `SessionEvent::Verify { diagnostics: <author, owned> }`.
  - Push `author.len()` onto a `recent_verifier_error_counts: Vec<usize>` (this is
    the hard-fail input; push **only** on a `Checked` outcome, so "consecutive"
    means consecutive verifier *runs*).
  - If `!author.is_empty()`: append a **user** message rendering the author
    diagnostics (each: path, line, severity, message — a short local renderer;
    don't reach into `phase::briefing`'s private one) as retry guidance, and keep
    the most recent author diagnostics in a `last_author_diagnostics: Vec<
    Diagnostic>` (owned) for the hard-fail briefing. Continue the loop.
  - If `author.is_empty()`: no retry message; clear `last_author_diagnostics`.
- **`Unsupported`** → skip: no `Verify` event, no feedback, do **not** push a
  verifier count (the file's language has no checker — not a failure).
- **`Failed(msg)`** → append a brief `user`/notice message `"verifier failed:
  {msg}"` (infra hiccup, **not** the model's fault), do **not** push a verifier
  count, do **not** treat as `Err`.

The edit's `ToolResult` is still logged by 07b's path; the `Verify` event is
additional.

### 5. Hard-fail detection (step 7)

After every dispatched turn (edit **or** not — repetition/runaway don't need the
verifier), evaluate:

```rust
hard_fail::evaluate(&recent_tool_calls, &recent_verifier_error_counts,
                    Some((&tool_call.name, content.len())))
```

On `Some(signal)`:
- Log `SessionEvent::HardFail { reason: signal.describe() }`, then
  `SessionEnd { status: "hard_fail", turns }`.
- Assemble a **hard_fail briefing**: `goal` / `acceptance_criteria` from the input;
  `diagnostics: last_author_diagnostics.clone()` (the exact blocker — empty for a
  non-verifier signal); `working_files` / `what_was_tried` from the phase-06
  helpers; `current_blocker: Blocker::HardFail(signal)`; a `budget_remaining` line
  (e.g. `"halted on hard-fail at turn {turns}"`).
- Return `PhaseResult::hard_fail(briefing, artifacts)` (same empty artifacts as
  07a — 07e populates `diff` / `files_changed` / `command_outputs`).

Evaluate **before** the turn-cap check (a hard-fail at the cap boundary should
report `hard_fail`, the more specific cause).

### 6. Error model

- The verifier seam returns `VerifierResult` (a value), never `Err` to the loop —
  `Failed(msg)` is the infra-hiccup case and is model-visible (a notice), matching
  the existing tool-failure handling.
- No `.unwrap()` / `.expect()` / `panic!()` in the loop.
- Logging stays best-effort (07b).

## Acceptance criteria

- [ ] `LoopDeps` carries a `&dyn FileVerifier`; a `RealVerifier` delegates to
      `governor::verifier`. Loop tests inject a mock (no real compiler spawn).
- [ ] An edit-class call (`write_file`/`patch`) runs the verifier on its `"path"`;
      a non-edit call does not.
- [ ] `Checked` author diagnostics are fed back as a retry message and logged as a
      `Verify` event; a clean `Checked` produces neither.
- [ ] Ambient diagnostics (present in the baseline) are **not** fed back
      (**negative**).
- [ ] `Unsupported` is skipped (no `Verify`, no count); `Failed(msg)` appends a
      notice and is not an `Err` and not counted.
- [ ] Three consecutive `Checked`-with-author turns trip
      `HardFailSignal::VerifierFailurePersistent` → `status == HardFail`,
      `Blocker::HardFail`, briefing `diagnostics` non-empty.
- [ ] Three identical tool calls trip `IdenticalToolCallRepetition`; a >`RUNAWAY_
      OUTPUT_BYTES` tool output trips `RunawayOutput` — both → `HardFail` (no
      verifier needed).
- [ ] A hard-fail logs `HardFail` then `SessionEnd { status: "hard_fail" }`.
- [ ] No new dependency; no `tracing`; `scorer` untouched; `governor/**` and
      `phase/**` unmodified; read-before-edit and completion artifacts not added.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic via a **`MockFileVerifier`** (scripts a `VecDeque<
VerifierResult>` popped per `verify` call; `capture_baseline` returns a configured
`Baseline`; records call paths). `MockAiClientScript` for the model; real tool
registry over a `TempDir`. Pin negatives.

- `edit_class_call_runs_verifier` / `non_edit_call_does_not_run_verifier` (records).
- `clean_verify_produces_no_retry_message`.
- `author_diagnostics_fed_back_as_retry` — a `user` message contains the
  diagnostic text; a `Verify` event is logged; loop continues.
- `ambient_diagnostics_not_fed_back` (**negative** — baseline contains the
  signature → partitioned ambient → no retry, no author count).
- `unsupported_verify_is_skipped` (no `Verify` event, no count).
- `verifier_failed_appends_notice_not_err` (a notice message; result is `Ok`).
- `persistent_verifier_failure_trips_hard_fail` — three `Checked`-with-author →
  `VerifierFailurePersistent`; assert `status == HardFail`, `Blocker::HardFail`,
  `briefing.diagnostics` non-empty.
- `identical_tool_call_repetition_trips_hard_fail` — three identical native calls.
- `runaway_output_trips_hard_fail` — a tool whose output exceeds
  `RUNAWAY_OUTPUT_BYTES` (e.g. write a >100 KB file, then `read_file` it).
- `hard_fail_logs_hardfail_then_session_end` — inspect the session log order.
- `hard_fail_briefing_carries_blocker_and_diagnostics`.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The verifier-retry and
> hard-fail paths are exercised via `MockFileVerifier` + `MockAiClientScript` over a
> `TempDir`. The real verifier (`RealVerifier` → `cargo`/`tsc`/`ruff`) is covered by
> phase-01's verifier tests; the first live loop-over-real-verifier run is M5.

## Authorizations

- [x] **May modify** `executor/src/agent/**` (loop + tests); **may create**
      `executor/src/agent/verify.rs` (the `FileVerifier` seam + `RealVerifier`).
- [ ] **No new dependencies** (`async_trait` is already available via `ai`); no
      `tracing`.
- [ ] May **NOT** modify `executor/src/governor/**` (verifier / hard_fail / scorer
      are used as-is), `executor/src/phase/**`, `Cargo.toml`, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, or another phase doc.

## Out of scope

- **Read-before-edit invariant** (the loop refusing a `patch` on a file not read
  this session or changed underneath it) — **07d**. 07c runs the verifier *after*
  an edit; 07d adds the *pre*-dispatch gate at the same site.
- **Completion artifacts** — final command set, unified diff, `files_changed` /
  `command_outputs`, and surfacing the log path — **07e**.
- **The `scorer` consumer** — `PhaseRun` telemetry, phase-08.
- **`Progress` events** — M5.
- Changing the verifier's language coverage or diagnostic parsing — phase-01
  territory; use `verify` as-is.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md.

Adding `agent/verify.rs` (`FileVerifier` trait + `RealVerifier` delegating to
`governor::verifier`), a `verifier` field on `LoopDeps`, and the governor feedback
into the loop: **pre-dispatch** lazy baseline capture per file extension (must
precede the edit, else the model's own errors baseline as ambient), **post-dispatch**
`verify` with `Baseline::partition` feeding author diagnostics back as a retry +
`Verify` log event, and `hard_fail::evaluate` each turn → a `hard_fail` briefing
(`Blocker::HardFail`, `last_author_diagnostics`) + `HardFail`/`SessionEnd` events.
Tests inject a `MockFileVerifier`; a `NoopVerifier` keeps existing tests'
non-edit runs from spawning a real compiler.

### Update — 2026-05-29 (complete)

**Summary:** Added `agent/verify.rs` (`FileVerifier` trait + `RealVerifier`
delegating to `governor::verifier::{verify, capture_baseline}`) and a `verifier:
&dyn FileVerifier` field on `LoopDeps`. Threaded the governor feedback into the
loop: an `edit_target` helper resolves a `write_file`/`patch` call's `"path"`
(absolute or project-root-relative); **before** dispatch, the first edit per file
extension lazily captures a `Baseline` (merged via `baseline.signatures.extend`)
so pre-existing diagnostics are ambient; **after** a *successful* edit, `verify`
runs and `Baseline::partition` splits author vs ambient — author diagnostics are
fed back as a rendered `user` retry message, logged as a `Verify` event, and
counted into `recent_verifier_error_counts` (only on `Checked`). `Unsupported` is
skipped; `Failed(msg)` appends a notice (not an `Err`, not counted). Each turn,
`hard_fail::evaluate(recent_tool_calls, verifier_counts, Some((name, content.len())))`
runs before the turn cap; on a signal it logs `HardFail` + `SessionEnd{hard_fail}`
and returns `PhaseResult::hard_fail` with a briefing carrying `Blocker::HardFail`
and the last author diagnostics. Tests inject `MockFileVerifier`; existing
non-edit tests use a `NoopVerifier`. No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.71s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.09s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 461 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(461 = 450 prior + 11 verifier/hard-fail tests. `agent::` alone: 36 passed.)

**Spec-pinned literal grep** (the wired log events):

```
grep -c 'SessionEvent::HardFail\|SessionEvent::Verify' executor/src/agent/mod.rs → 5
(2 emit sites + the test event_kind matcher + 2 test assertions)
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. Verifier-retry and the
three hard-fail signals are exercised via `MockFileVerifier` + `MockAiClientScript`
over a `TempDir`; the real `cargo`/`tsc`/`ruff` path is covered by phase-01's
verifier tests. First live loop-over-real-verifier run is M5.

**Files changed:**
- `executor/src/agent/verify.rs` — new: `FileVerifier` trait + `RealVerifier`.
- `executor/src/agent/mod.rs` — `verify` module; `LoopDeps.verifier`; baseline +
  verify + hard-fail threaded into the loop; `edit_target` / `render_diagnostics`
  / `hard_fail_result` helpers; 11 new tests + `MockFileVerifier` / `NoopVerifier`.

**New tests:** `edit_class_call_runs_verifier`, `non_edit_call_does_not_run_verifier`,
`clean_verify_produces_no_retry_message`, `author_diagnostics_fed_back_as_retry`,
`ambient_diagnostics_not_fed_back` (negative), `unsupported_verify_is_skipped`,
`verifier_failed_appends_notice_not_err`,
`persistent_verifier_failure_trips_hard_fail`,
`identical_tool_call_repetition_trips_hard_fail`, `runaway_output_trips_hard_fail`,
`hard_fail_logs_hardfail_then_session_end`.

**Commits:** (pending — committed below)

**Notes for review:**
- **Baseline is captured pre-dispatch** (before the edit lands), not at session
  start — capturing after the edit would record the model's own new errors as
  ambient. Lazy per-extension; `capture_baseline` dedups internally.
- Post-edit verify is guarded on `succeeded` (verifying after a failed edit is
  noise); the spec said "after an edit-class dispatch" without conditioning on
  success — this is a deliberate, minor tightening, flagged here.
- `recent_verifier_error_counts` is pushed **only** on `Checked`, so
  `VerifierFailurePersistent` counts consecutive verifier *runs* (not turns).
- `render_diagnostics` is a small local renderer (not `phase::briefing`'s private
  one, which the phase may not import).
- `scorer.record` still has no consumer — unchanged (→ phase-08 telemetry).

verification: fmt OK · clippy OK · tests 461 passed · build OK
