# Phase 05: Governor — tool scorer + hard-fail detector

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** todo
**Depends on:** phase-01 (verifier `Diagnostic`/`Severity`, done), phase-03
(session-event types that supply the structured turn data, done).
**Estimated diff:** ~340 lines (scorer lift + hard-fail adapt + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

The two **governor** pieces the loop consults each turn: a per-tool **success
scorer** (Laplace-smoothed, reorders the advertised tool list so reliable tools
float up) and a **hard-fail detector** (watches for the model getting stuck —
identical-call repetition, persistent verifier failure, runaway tool output —
and emits a structured `HardFailSignal` the briefing will consume in phase-06).
Neither runs the loop; they are the signals the loop (phase-07) reads.

Two pieces:
- **Scorer** — a near-direct lift of `rexy/src/governor/scorer.rs`. Clean: no
  clock, no network, no `tracing`. Lift close to verbatim.
- **Hard-fail detector** — Rexy's `hard_fail.rs` thresholds and `HardFailSignal`
  semantics, but **re-fed from structured inputs** instead of Rexy's
  message-string scraping (see Adaptations). This is a deliberate redesign, not a
  verbatim lift.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" steps 5–7 (lines ~118–123):
  the governor wraps tool dispatch; the hard-fail detector watches repetition,
  repeated verifier failures, and budget overflow. And § Scorecard (lines
  ~277–282): the per-tool scorer is the **within-session seed** of the
  cross-session `model × tag` matrix.
- Rexy source: `rexy/src/governor/scorer.rs` (lift) and
  `rexy/src/governor/hard_fail.rs` (adapt — thresholds + signal enum + `describe`,
  not the `Message`-scraping input path).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. **Read** `rexy/src/governor/scorer.rs` and `rexy/src/governor/hard_fail.rs`.
5. Read `executor/src/governor/verifier.rs`'s public types (`Diagnostic`,
   `Severity`) — the verifier-persistence signal counts `Severity::Error`
   diagnostics.
6. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`executor/src/governor/` contains `mod.rs` (`pub mod verifier;`) and
`verifier.rs` (phase-01: `Diagnostic { path, line, column, severity, message,
code }`, `Severity::{Error, Warning, …}`, both `Serialize`+`Deserialize`). There
is **no** `scorer.rs` or `hard_fail.rs`. The phase-03 session log already records
structured per-turn events (`Parsed { tool_call }`, `ToolResult { name,
succeeded, output_preview }`, `Verify { diagnostics }`) — so the loop has
structured turn data; the detector does **not** need to parse strings.

Rexy's `hard_fail.rs` takes `&[Message]` and scrapes verifier error counts out of
message content with a `"[verifier] N error(s) after edit:"` string marker and a
`<tool_result>` envelope. **rexyMCP must not do this** — string-scraping a
log-format marker is exactly the brittleness the structured event schema (phase-03)
exists to avoid. Feed the detector structured values instead.

## Spec

Create `executor/src/governor/scorer.rs` and `executor/src/governor/hard_fail.rs`;
wire both into `executor/src/governor/mod.rs` and re-export the public types.

### Scorer — lift `scorer.rs` near-verbatim

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCounts { pub successes: u32, pub failures: u32 }

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scorer { pub counts: HashMap<String, ToolCounts> }

impl Scorer {
    pub fn new() -> Self;                          // empty
    pub fn record(&mut self, tool: &str, succeeded: bool);  // saturating_add
    pub fn score(&self, tool: &str) -> f64;        // Laplace: (s+1)/(s+f+2), 0.5 unobserved
}
```

Keyed on **tool name only** — within one `execute_phase` session the task type is
fixed (it comes from the phase doc's `Tags:`), so the architecture's
"per-(task-type, tool)" scorer collapses to per-tool *within* a session; the
task-type axis is added when these counts are persisted into the `PhaseRun`
record (phase-08) and aggregated by the M7 scorecard. Lift Rexy's code and tests
as-is (re-root nothing — it has no `crate::` deps).

### Hard-fail detector — adapt `hard_fail.rs` to structured inputs

Keep Rexy's thresholds and signal enum verbatim:

```rust
pub const IDENTICAL_CALL_THRESHOLD: usize = 3;
pub const VERIFIER_PERSISTENCE_THRESHOLD: usize = 3;
pub const RUNAWAY_OUTPUT_BYTES: usize = 100 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallSnapshot { pub tool: String, pub arguments: serde_json::Value, pub succeeded: bool }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HardFailSignal {
    IdenticalToolCallRepetition { tool: String, consecutive_count: u32 },
    VerifierFailurePersistent { consecutive_failures: u32 },
    RunawayOutput { tool: String, bytes: usize },
}

impl HardFailSignal { pub fn describe(&self) -> String; }
```

**`describe()`** returns a one-line human summary. Pin the *content* (verified by
substring, not exact-string equality on the byte-budget interpolation):
- repetition → contains `"identical "`, the tool name, and `"repeated "`.
- verifier → contains `"verifier flagged errors on "` and the count.
- runaway → contains `"produced "`, the tool name, and `" bytes"`.

**The evaluate entry point — structured inputs, no string scraping:**

```rust
pub fn evaluate(
    recent_tool_calls: &VecDeque<ToolCallSnapshot>,
    recent_verifier_error_counts: &[usize],  // chronological: oldest first, newest last;
                                             // one entry per verifier run, = count of Severity::Error diagnostics
    last_tool_output: Option<(&str, usize)>, // (tool name, byte length) of the most recent tool result
) -> Option<HardFailSignal>
```

Check order (first match wins, mirroring Rexy): repetition → verifier persistence
→ runaway. Semantics:

1. **Identical repetition** — fire if the last `IDENTICAL_CALL_THRESHOLD` entries
   of `recent_tool_calls` exist and all share the same `tool` **and** identical
   `arguments` (ignore `succeeded`). `consecutive_count = IDENTICAL_CALL_THRESHOLD`.
2. **Verifier persistence** — fire if `recent_verifier_error_counts` has at least
   `VERIFIER_PERSISTENCE_THRESHOLD` entries, the **last** `THRESHOLD` are all
   `> 0`, and across those entries the count is **non-decreasing** oldest→newest
   (the model is not reducing errors). Do **not** fire if any of the last
   `THRESHOLD` is `0` (a clean verify = progress) or if the counts strictly
   decrease at any step. `consecutive_failures = THRESHOLD`.
3. **Runaway output** — fire if `last_tool_output` is `Some((tool, bytes))` with
   `bytes > RUNAWAY_OUTPUT_BYTES`. Boundary: `bytes == RUNAWAY_OUTPUT_BYTES` does
   **not** fire.

Do **not** lift Rexy's `extract_verifier_count` / `extract_tool_name_from_result`
/ `Message` import — they exist only to scrape the string format we are replacing.

### Wire into `governor/mod.rs`

Add `pub mod hard_fail;` and `pub mod scorer;`. Re-export `Scorer`,
`HardFailSignal`, `ToolCallSnapshot`, and `evaluate` (your call on whether to
alias `evaluate` to a clearer name like `evaluate_hard_fail` at the re-export —
pin only that it is reachable from `governor`). Do **not** lift Rexy's
`TaskType` / `classify()` (a stub; task type comes from phase metadata, not a
classifier) — out of scope.

**Serde note (completing the serde story up front — phase-03 calibration).**
Derive **both** `Serialize` and `Deserialize` on `Scorer`, `ToolCounts`,
`HardFailSignal`, and `ToolCallSnapshot`. They are self-contained (`String`,
`u32`, `serde_json::Value`, `HashMap`), so the derives create no upstream
pressure, and the `PhaseRun` telemetry (phase-08) and briefing (phase-06) will
both read and write these — pinning both now avoids the read-side derive scramble
that bounced phase-03.

## Acceptance criteria

- [ ] `governor/scorer.rs` + `governor/hard_fail.rs` exist; `governor/mod.rs`
      exposes `Scorer`, `HardFailSignal`, `ToolCallSnapshot`, and the evaluate fn.
- [ ] `Scorer::score` returns `0.5` for an unobserved tool and matches
      `(s+1)/(s+f+2)` after observations.
- [ ] `evaluate` detects each of the three signals from **structured** inputs and
      returns `None` for a healthy session — with **no** `Message`/string scraping
      and no `[verifier]`/`<tool_result>` literal anywhere in `hard_fail.rs`.
- [ ] Check order is repetition → verifier → runaway (first match wins).
- [ ] No `TaskType`/`classify` lifted; no new dependency; no `tracing`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Pure-function unit tests at the bottom of each file; no filesystem, clock, or
network. Pin positives **and** the boundary negatives (WORKFLOW § "Pin negative
cases").

**Scorer** (lift Rexy's): `new_scorer_is_empty`, `record_increments_successes`,
`record_increments_failures`, `score_unobserved_returns_half`,
`score_matches_laplace_formula`, `score_pure_successes_approaches_one_but_doesnt_reach`.

**Hard-fail — positives:**
- `detects_identical_repetition` — 3 identical snapshots → `IdenticalToolCallRepetition`.
- `detects_verifier_persistence` — error counts `[2, 2, 2]` (or non-decreasing
  >0) → `VerifierFailurePersistent`.
- `detects_runaway_output` — `Some(("read_file", RUNAWAY_OUTPUT_BYTES + 1))` →
  `RunawayOutput`.
- `describe_*` — one per variant, asserting the pinned content substrings.

**Hard-fail — negatives (the boundary):**
- `healthy_session_returns_none` — distinct calls, counts `[1]`, small output → `None`.
- `no_repetition_when_arguments_differ` — 3 same-tool calls, differing `arguments` → `None`.
- `no_repetition_below_threshold` — 2 identical snapshots → `None`.
- `no_verifier_persistence_when_errors_decrease` — `[5, 3, 1]` → `None`.
- `no_verifier_persistence_when_a_count_is_zero` — `[2, 0, 2]` (a clean verify in
  the window) → `None`.
- `no_runaway_at_exact_threshold` — `Some((_, RUNAWAY_OUTPUT_BYTES))` → `None`.
- `check_order_repetition_precedes_verifier` — a session that satisfies *both*
  repetition and verifier persistence returns the repetition signal.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The scorer and
> detector are pure library functions exercised by unit tests; the loop that
> calls them each turn (and writes their signals to the session log) is phase-07.

## Authorizations

- [x] **May create** `executor/src/governor/scorer.rs` and
      `executor/src/governor/hard_fail.rs`; **may modify**
      `executor/src/governor/mod.rs` (module decls + re-exports).
- [ ] **No new dependencies** (`serde`, `serde_json` are already present; no
      `tracing`).
- [ ] May **NOT** lift `TaskType`/`classify`, implement the loop, the briefing,
      or `PhaseRun` telemetry.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `verifier.rs`, or another phase doc.

## Out of scope

- **The loop that maintains turn state and calls `evaluate`/`record`** — phase-07.
  This phase only ships the pure functions and their input contract.
- **Budget overflow as a hard-fail signal** — budget is the context engine's
  concern (phase-02); the loop maps a budget cap to the `budget_exceeded` status
  directly (phase-07), not through `HardFailSignal`.
- **The briefing** that renders a signal for Claude — phase-06.
- **`TaskType` / task classification** — task type comes from phase metadata.
- **Persisting the scorer cross-session** — `PhaseRun` telemetry, phase-08.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
