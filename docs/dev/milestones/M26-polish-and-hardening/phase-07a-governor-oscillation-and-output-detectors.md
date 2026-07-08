# Phase 07a: Governor oscillation & windowed-output detectors

**Milestone:** M26 — Polish & Hardening
**Status:** todo
**Depends on:** none
**Estimated diff:** ~300 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

The governor's three stall detectors (`IdenticalToolCallRepetition`,
`VerifierFailurePersistent`, `RunawayOutput`) cover only the extremes and miss the
shapes in between (codebase review §2 "Governor blind spots"):

- An **A,B,A,B** read↔patch oscillation never trips `IdenticalToolCallRepetition`,
  which requires N *consecutive byte-identical* calls.
- `RunawayOutput` measures only the **last** call's fed-back byte length, so a
  multi-call flood of sub-threshold outputs (a filtered `cargo` dump repeated, a
  loop of 60 KB reads) passes unbounded.

This phase adds two additive, pure detectors that plug into the same Step-7
hard-fail seam: a **sliding-window distinct-call-set** detector (oscillation) and a
**windowed cumulative-output** detector. Both default to values that leave every
existing session's behavior unchanged and both are disable-able (`window = 0`).

**Split note:** the third review item — a **wall-clock ceiling** (`[budget]
wall_clock_secs`) — is a different mechanism (a clock-based *budget* terminal, not
a governor tool-call detector) and is deferred to **phase-07b** per the milestone
README § Notes ("Phase-07 … may re-split (07a detectors / 07b wall-clock
ceiling)"). Do **not** add any wall-clock/time budget in this phase.

## Architecture references

Read before starting:

- `docs/dev/STANDARDS.md` — the Definition of Done. §2.2 "No fallbacks for if X is
  missing" — the detectors have defaults, not silent no-ops; `window = 0` is an
  explicit documented off-switch, not a fallback.
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — both detectors are new
  standalone pure functions + new `HardFailSignal` variants + new `#[serde(default)]`
  config fields; **no existing signature or field changes meaning**. This is the same
  additive shape M22 phase-01/02 used for `check_empty_completion_stall` /
  `check_repeated_gate_feedback`.
- `docs/architecture.md` § the turn cycle / governor — the governor returns a
  `HardFailSignal` that terminates the loop with a briefing to the architect.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The hard-fail module (`executor/src/governor/hard_fail.rs`)

Two patterns already coexist and are the exact templates to follow:

1. **Detectors folded into `evaluate`** — `check_identical_repetition`,
   `check_verifier_persistence`, `check_runaway_output`. `evaluate` chains them and
   returns the first `Some`.
2. **Standalone pure detectors called directly from the loop** —
   `check_empty_completion_stall` and `check_repeated_gate_feedback` (hard_fail.rs:151,
   167). These are **not** in `evaluate`; the loop calls them at its own seams. This
   is the lower-blast-radius pattern (it does not change `evaluate`'s signature or its
   ~10 test call sites), and it is the one this phase uses.

The `HardFailSignal` enum (hard_fail.rs:14-36) and its exhaustive `describe` match
(hard_fail.rs:38-68) are the **only** place new variants must be handled — nothing
else matches `HardFailSignal` exhaustively (`briefing.rs` only constructs one variant
in a test; `mod.rs` constructs `BackendError`). Adding a variant requires a new
`describe` arm and nothing more.

`ToolCallSnapshot` (hard_fail.rs:7-12) carries `tool: String`, `arguments:
serde_json::Value`, `succeeded: bool`. `serde_json::Value` implements `PartialEq`
but **not** `Hash`/`Ord`, so distinct-set membership is a linear `Vec` scan, not a
`HashSet` (worked example below).

### The Step-7 hard-fail seam (`executor/src/agent/mod.rs:1236-1241`)

The loop evaluates the governor once per turn, after tool dispatch, before the turn
cap. Current code:

```rust
        // Step 7 — hard-fail detection (repetition / persistent verifier failure /
        // runaway output). Checked before the turn cap so the specific cause wins.
        if let Some(signal) = evaluate(
            &recent_tool_calls,
            &recent_verifier_error_counts,
            Some((&tool_call.name, content.len())),
            &deps.governor,
        ) {
```

`recent_tool_calls` (declared mod.rs:161, `VecDeque<ToolCallSnapshot>`) is
**unbounded** — the full call history is retained (briefing reads it), so a windowed
detector just inspects the last N via `.iter().rev().take(N)`. It is pushed once per
tool call at **mod.rs:1055**:

```rust
        recent_tool_calls.push_back(ToolCallSnapshot {
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
            succeeded,
        });
```

`content` (the tool output string) is in scope at both mod.rs:1055 and the Step-7
site — `content.len()` is already the byte count fed to `check_runaway_output`.

The standalone helpers are called fully-qualified, e.g. mod.rs:540:
`crate::governor::hard_fail::check_empty_completion_stall(...)`. Mirror that style
for the two new detectors (no `use` change needed).

### `GovernorConfig` (`executor/src/config.rs:162-192`)

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct GovernorConfig {
    pub identical_call_threshold: usize,
    pub verifier_persistence_threshold: usize,
    pub runaway_output_bytes: usize,
    pub empty_completion_threshold: usize,
    pub gate_feedback_repeat_threshold: usize,
}

impl Default for GovernorConfig {
    fn default() -> Self {
        Self {
            identical_call_threshold: 6,
            verifier_persistence_threshold: 6,
            runaway_output_bytes: 100 * 1024,
            empty_completion_threshold: 3,
            gate_feedback_repeat_threshold: 5,
        }
    }
}
```

The struct is `#[serde(default)]`, so **adding fields keeps every existing config
TOML and its parse tests valid** — omitted keys fall to the new defaults.

`ModelOverride` (config.rs:200-211) mirrors the governor knobs as `Option<_>`;
`resolve_for_model` (config.rs:479-494) applies each `Some` over the global default:

```rust
        if let Some(v) = over.runaway_output_bytes {
            self.governor.runaway_output_bytes = v;
        }
        if let Some(v) = over.empty_completion_threshold {
            self.governor.empty_completion_threshold = v;
        }
        if let Some(v) = over.gate_feedback_repeat_threshold {
            self.governor.gate_feedback_repeat_threshold = v;
        }
```

### The `rexymcp init` template (`mcp/src/init.rs:34-49`)

```toml
[governor]
identical_call_threshold = 6      # consecutive identical tool calls → hard-fail
verifier_persistence_threshold = 6 # consecutive turns with verifier errors → hard-fail
runaway_output_bytes = 102400     # single tool output bytes → hard-fail (100 KB)

# [models."<model-id>"]              # per-model knob overrides; key is the exact
# ...
# identical_call_threshold = 8       # override [governor] identical_call_threshold
# verifier_persistence_threshold = 8 # override [governor] verifier_persistence_threshold
# runaway_output_bytes = 204800      # override [governor] runaway_output_bytes
```

## Spec

Numbered tasks in execution order.

1. **Add four `GovernorConfig` fields + defaults** — in `executor/src/config.rs`, in
   the `GovernorConfig` struct (config.rs:164-180) add, after
   `gate_feedback_repeat_threshold`:

   ```rust
       /// Sliding window of recent tool calls examined for oscillation. When the
       /// distinct `(tool, arguments)` count in the last `oscillation_window` calls
       /// is in `2..=oscillation_distinct_max`, the loop hard-fails with
       /// `Oscillation`. `0` disables the detector. Default 8.
       pub oscillation_window: usize,
       /// Max distinct calls in the oscillation window still treated as a stuck
       /// cycle. Default 2 (an A,B,A,B alternation).
       pub oscillation_distinct_max: usize,
       /// Sliding window of recent tool outputs summed for the cumulative-output
       /// flood check. `0` disables the detector. Default 6.
       pub output_window: usize,
       /// Total bytes across the last `output_window` tool outputs before
       /// `CumulativeOutputFlood`. Catches multi-call floods each below
       /// `runaway_output_bytes`. Default 262144 (256 KB).
       pub output_window_bytes: usize,
   ```

   and in the `Default` impl (config.rs:182-192) add:

   ```rust
           oscillation_window: 8,
           oscillation_distinct_max: 2,
           output_window: 6,
           output_window_bytes: 256 * 1024,
   ```

2. **Add matching `ModelOverride` fields** — in `executor/src/config.rs`, in
   `ModelOverride` (config.rs:200-211) add after `gate_feedback_repeat_threshold`:

   ```rust
       pub oscillation_window: Option<usize>,
       pub oscillation_distinct_max: Option<usize>,
       pub output_window: Option<usize>,
       pub output_window_bytes: Option<usize>,
   ```

3. **Apply the overrides in `resolve_for_model`** — in `executor/src/config.rs`,
   after the `gate_feedback_repeat_threshold` block (config.rs:492-494), add the four
   parallel blocks:

   ```rust
           if let Some(v) = over.oscillation_window {
               self.governor.oscillation_window = v;
           }
           if let Some(v) = over.oscillation_distinct_max {
               self.governor.oscillation_distinct_max = v;
           }
           if let Some(v) = over.output_window {
               self.governor.output_window = v;
           }
           if let Some(v) = over.output_window_bytes {
               self.governor.output_window_bytes = v;
           }
   ```

4. **Add two `HardFailSignal` variants + `describe` arms** — in
   `executor/src/governor/hard_fail.rs`, in the enum (hard_fail.rs:14-36) add:

   ```rust
       Oscillation {
           distinct_calls: usize,
           window: usize,
       },
       CumulativeOutputFlood {
           window: usize,
           bytes: usize,
       },
   ```

   and in `describe` (hard_fail.rs:38-68) add the matching arms:

   ```rust
           Self::Oscillation {
               distinct_calls,
               window,
           } => {
               format!("only {distinct_calls} distinct tool calls across the last {window} turns (oscillation)")
           }
           Self::CumulativeOutputFlood { window, bytes } => {
               format!("tool output flooded {bytes} bytes across the last {window} calls (over threshold)")
           }
   ```

5. **Add `check_oscillation`** — a standalone pure fn in
   `executor/src/governor/hard_fail.rs` (next to `check_repeated_gate_feedback`), in
   the shape of the existing standalone detectors. Note the **linear distinct-set
   scan** — `serde_json::Value` is not `Hash`/`Ord`:

   ```rust
   /// Oscillation stall: the last `window` tool calls collapse to only a small set
   /// of distinct `(tool, arguments)` pairs (e.g. an A,B,A,B read↔patch cycle) that
   /// `IdenticalToolCallRepetition` misses because the calls are not *consecutively*
   /// identical. Fires when the distinct count is in `2..=distinct_max`. A distinct
   /// count of 1 is left to `check_identical_repetition`; `window == 0` disables.
   pub fn check_oscillation(
       recent: &VecDeque<ToolCallSnapshot>,
       window: usize,
       distinct_max: usize,
   ) -> Option<HardFailSignal> {
       if window == 0 || recent.len() < window {
           return None;
       }
       let mut distinct: Vec<(&str, &serde_json::Value)> = Vec::new();
       for call in recent.iter().rev().take(window) {
           let key = (call.tool.as_str(), &call.arguments);
           if !distinct.iter().any(|(t, a)| *t == key.0 && *a == key.1) {
               distinct.push(key);
           }
       }
       let n = distinct.len();
       if n >= 2 && n <= distinct_max {
           Some(HardFailSignal::Oscillation {
               distinct_calls: n,
               window,
           })
       } else {
           None
       }
   }
   ```

6. **Add `check_windowed_output`** — a standalone pure fn in
   `executor/src/governor/hard_fail.rs`:

   ```rust
   /// Cumulative-output flood: the sum of the last `window` tool outputs exceeds
   /// `limit` bytes, catching a multi-call flood of sub-`runaway_output_bytes`
   /// outputs that `check_runaway_output` (single-call only) misses. Requires a full
   /// window; `window == 0` disables.
   pub fn check_windowed_output(
       recent_output_bytes: &VecDeque<usize>,
       window: usize,
       limit: usize,
   ) -> Option<HardFailSignal> {
       if window == 0 || recent_output_bytes.len() < window {
           return None;
       }
       let bytes: usize = recent_output_bytes.iter().rev().take(window).sum();
       if bytes > limit {
           Some(HardFailSignal::CumulativeOutputFlood { window, bytes })
       } else {
           None
       }
   }
   ```

7. **Track recent output bytes in the loop** — in `executor/src/agent/mod.rs`,
   declare a companion `VecDeque` next to `recent_tool_calls` (near mod.rs:161):

   ```rust
       let mut recent_output_bytes: VecDeque<usize> = VecDeque::new();
   ```

   and push to it immediately after the `recent_tool_calls.push_back(...)` block
   (mod.rs:1055-1059), so the two stay in lockstep (one entry per tool call,
   regardless of success — a failed call's output still consumes context):

   ```rust
       recent_output_bytes.push_back(content.len());
   ```

8. **Chain the two detectors into the Step-7 seam** — in `executor/src/agent/mod.rs`,
   replace the `if let Some(signal) = evaluate(...) {` head at mod.rs:1236-1241 with a
   bound expression that falls through to the two new detectors:

   ```rust
       // Step 7 — hard-fail detection (repetition / persistent verifier failure /
       // runaway output / oscillation / cumulative-output flood). Checked before the
       // turn cap so the specific cause wins.
       let hard_fail_signal = evaluate(
           &recent_tool_calls,
           &recent_verifier_error_counts,
           Some((&tool_call.name, content.len())),
           &deps.governor,
       )
       .or_else(|| {
           crate::governor::hard_fail::check_oscillation(
               &recent_tool_calls,
               deps.governor.oscillation_window,
               deps.governor.oscillation_distinct_max,
           )
       })
       .or_else(|| {
           crate::governor::hard_fail::check_windowed_output(
               &recent_output_bytes,
               deps.governor.output_window,
               deps.governor.output_window_bytes,
           )
       });
       if let Some(signal) = hard_fail_signal {
   ```

   Leave the body of the `if` (the `log_event` / `log_session_end` /
   `emit_phase_run` / `build_artifacts` / `return Ok(hard_fail_result(...))`) exactly
   as it is — it already consumes `signal` generically.

9. **Document the four knobs in the `rexymcp init` template** — in
   `mcp/src/init.rs`, in the `[governor]` block (init.rs:34-37) add after
   `runaway_output_bytes`:

   ```toml
   oscillation_window = 8            # sliding window scanned for A,B,A,B oscillation (0 disables)
   oscillation_distinct_max = 2      # ≤ this many distinct calls in the window → hard-fail
   output_window = 6                 # sliding window of tool outputs summed for flood check (0 disables)
   output_window_bytes = 262144      # total bytes across the output window → hard-fail (256 KB)
   ```

   and add matching override comment lines in the `[models]` comment block after the
   `runaway_output_bytes` override line (init.rs:49):

   ```toml
   # oscillation_window = 10            # override [governor] oscillation_window
   # oscillation_distinct_max = 2       # override [governor] oscillation_distinct_max
   # output_window = 8                  # override [governor] output_window
   # output_window_bytes = 524288       # override [governor] output_window_bytes
   ```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `GovernorConfig` has the four new fields with defaults 8 / 2 / 6 / 262144;
      `ModelOverride` has the four `Option` counterparts; `resolve_for_model` applies
      them. Every pre-existing `config.rs`/`init.rs` parse test passes **unmodified**
      (additive `#[serde(default)]` fields).
- [ ] `HardFailSignal::Oscillation` and `HardFailSignal::CumulativeOutputFlood` exist
      with `describe` arms.
- [ ] A model alternating between exactly two distinct tool calls until the
      oscillation window fills terminates as `hard_fail` with `Oscillation` — **not**
      at the turn cap and **not** as `IdenticalToolCallRepetition`.
- [ ] A model emitting a windowful of sub-`runaway_output_bytes` outputs whose sum
      exceeds `output_window_bytes` terminates as `hard_fail` with
      `CumulativeOutputFlood`.
- [ ] With `oscillation_window = 0` (resp. `output_window = 0`) the corresponding
      detector never fires (unit test).
- [ ] `rexymcp init --config <tmp>/rexymcp.toml` writes a template whose `[governor]`
      block contains `oscillation_window` and `output_window_bytes`, and the written
      file loads without error.

## Test plan

**Unit tests in `executor/src/governor/hard_fail.rs`** (`#[cfg(test)] mod tests`),
mirroring the existing `empty_completion_stall_*` / `repeated_gate_feedback_*` shape.
Build `VecDeque`s with the `snap`/`push_back` idiom already used by
`detects_identical_repetition`.

- `oscillation_fires_on_two_call_cycle` — 4-call window `A,B,A,B` (two distinct),
  `window=4, distinct_max=2` → `Some(Oscillation { distinct_calls: 2, window: 4 })`.
- `oscillation_silent_when_window_not_full` — 3 calls, `window=4` → `None`.
- `oscillation_silent_when_all_identical` — 4 byte-identical calls (distinct 1),
  `window=4, distinct_max=2` → `None` (left to `check_identical_repetition`). **This
  is the load-bearing negative pin** — it must fail if the `n >= 2` guard is dropped.
- `oscillation_silent_when_too_many_distinct` — 4 distinct calls, `distinct_max=2` →
  `None`.
- `oscillation_disabled_when_window_zero` — non-empty history, `window=0` → `None`.
- `windowed_output_fires_when_sum_exceeds_limit` — outputs `[400,400,400]`,
  `window=3, limit=1000` → `Some(CumulativeOutputFlood { window: 3, bytes: 1200 })`.
- `windowed_output_silent_at_or_below_limit` — outputs summing to exactly `limit` →
  `None` (strict `>` boundary pin).
- `windowed_output_silent_when_window_not_full` — 2 outputs, `window=3` → `None`.
- `windowed_output_disabled_when_window_zero` — non-empty history, `window=0` →
  `None`.
- `describe_oscillation` / `describe_cumulative_output_flood` — assert the rendered
  string contains `oscillation` (resp. `flooded`) and the byte/count values.

**Integration tests in `executor/src/agent/tests.rs`**, modelled on
`identical_tool_call_repetition_trips_hard_fail` / `runaway_output_trips_hard_fail`.
Build `LoopDeps` with a **tuned** `GovernorConfig` (small windows) so the tests are
fast, hermetic, and immune to context compaction:

```rust
governor: GovernorConfig {
    oscillation_window: 4,
    oscillation_distinct_max: 2,
    output_window: 3,
    output_window_bytes: 1000,
    ..GovernorConfig::default()
},
```

- `oscillation_across_alternating_reads_trips_hard_fail` — write two tiny files
  `a.txt`/`b.txt`; script the model to alternate `read_file a`, `read_file b` for
  ≥ 4 turns (distinct 2). Assert `PhaseStatus::HardFail` and
  `Blocker::HardFail(HardFailSignal::Oscillation { .. })`. (Alternating distinct
  reads avoid `IdenticalToolCallRepetition`, which needs consecutive identical.)
- `cumulative_output_flood_trips_hard_fail` — write three **distinct** ~400-byte
  files (distinct args, so oscillation with `distinct_max=2` does *not* fire and
  identical-repetition does not fire); script three `read_file` turns. With
  `output_window=3, output_window_bytes=1000` the 3-output sum (~1200) trips
  `CumulativeOutputFlood`. Assert `PhaseStatus::HardFail` and the
  `CumulativeOutputFlood` variant.

**Mock-exhaustion gotcha (from phase-06's takeover):** `MockAiClientScript::new`
must be scripted with **at least as many turns as the loop consumes before the
detector fires** — when the script runs dry, `chat()` emits nothing and the empty
completion routes to the *unrelated* empty-completion recovery branch
(`mod.rs:531`), drifting the loop to the turn cap instead of your detector. For the
oscillation test script the 4 alternating tool-call turns (+ one trailing `read_file`
turn as slack); for the flood test script the 3 read turns. Each turn here is a
**tool call**, not a completion, so there is no empty-completion interference —
just ensure enough turns exist.

## End-to-end verification

The behavioral outcome (a `hard_fail` termination) is loop-internal and exercised
hermetically by the integration tests above; a live model that deterministically
oscillates or floods is not hermetically reproducible. The **runtime-loadable real
artifact** this phase ships is the config surface: the four new `[governor]` knobs
that the running binary parses and the `rexymcp init` template documents. Verify
that artifact:

```
$ cargo run -p rexymcp -- init --config /tmp/p07a.toml
$ grep -E 'oscillation_window|oscillation_distinct_max|output_window|output_window_bytes' /tmp/p07a.toml
$ cargo run -p rexymcp -- health --config /tmp/p07a.toml   # loads the generated toml without a parse error
```

Quote the actual grep output (the four documented lines) and confirm `health` does
not error on the new keys, in the completion Update Log.

## Authorizations

None. No new dependency; no `Cargo.toml`/`architecture.md`/`STANDARDS.md`/
`WORKFLOW.md` edit. `mcp/src/init.rs` template edits and `executor/src/config.rs`
field additions are within scope (they are the config surface this phase ships).

## Out of scope

- **Wall-clock / cost ceiling (`[budget] wall_clock_secs`)** — deferred to
  phase-07b. Do not add any time-based budget or clock reads.
- **The `Scorer` feedback-into-control-flow idea** (review §2, last bullet) — the
  Laplace-smoothed per-tool scorer stays telemetry-only; not this phase.
- **The verifier-persistence 2,1,2,1 bounce** (review §2) — leave
  `check_verifier_persistence` unchanged; this phase is tool-call oscillation +
  output volume only.
- **Changing `evaluate`'s signature or its three existing detectors** — the two new
  detectors are standalone fns chained via `.or_else` at the call site, exactly like
  `check_empty_completion_stall`. Do not fold them into `evaluate`.
- **Bounding/trimming `recent_tool_calls` or `recent_output_bytes`** — they are
  intentionally unbounded (briefing reads the full history); the detectors window
  with `.rev().take(N)`.
- **A new `PhaseResult` status** — both detectors map to the existing `hard_fail`
  outcome via `hard_fail_result`; do not add an enum variant.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
