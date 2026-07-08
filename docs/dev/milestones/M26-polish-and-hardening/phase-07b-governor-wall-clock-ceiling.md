# Phase 07b: Governor wall-clock ceiling (`[budget] wall_clock_secs`)

**Milestone:** M26 — Polish & Hardening
**Status:** todo
**Depends on:** none (07a landed; this touches the same `LoopDeps`/loop but no 07a code)
**Estimated diff:** ~180 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

The loop today terminates only on the **turn cap** (`max_turns`), the **context
cap** (compaction still overflows), or a **governor hard-fail signal**. None of
these bounds *wall-clock time*: a model that makes slow-but-legal progress (long
prefills, a genuinely large phase) can run for an unbounded number of minutes as
long as it stays under the turn cap and keeps producing parseable output. The
post-M25 codebase review (§2 "Governor blind spots") flagged an **optional
wall-clock ceiling** as the third blind spot alongside the two 07a detectors.

This phase adds `[budget] wall_clock_secs` — a clock-based **budget terminal**
(not a governor tool-call detector). When set, a run whose elapsed wall-clock
time reaches the ceiling terminates as `budget_exceeded` at the next turn
boundary. Default `0` **disables** it, so every existing config and session is
byte-identical to today.

**07a split note (context):** 07a explicitly deferred this — "the third review
item — a **wall-clock ceiling** (`[budget] wall_clock_secs`) — is a different
mechanism (a clock-based *budget* terminal, not a governor tool-call detector)
and is deferred to **phase-07b**." This is that phase. It does **not** touch the
07a detectors (`check_oscillation` / `check_windowed_output`) or `GovernorConfig`.

## Architecture references

Read before starting:

- `docs/dev/STANDARDS.md` — the Definition of Done. §2.2 "No fallbacks for if X
  is missing": `wall_clock_secs = 0` is an explicit documented off-switch (the
  default), not a silent fallback.
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — the config field is
  `#[serde(default)]` (omitted key → `0`); the loop change is a new terminal
  branch that is inert when the field is `0`. **No existing field changes
  meaning.** This mirrors how M26 phase-06 added `gate_retries` (a new `[budget]`
  field + a new `LoopDeps` field threaded to every construction site).
- `docs/architecture.md` § the turn cycle — the loop returns `budget_exceeded`
  with a briefing to the architect when a budget is exhausted; a wall-clock
  ceiling is one more budget the same terminal covers.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The clock is already injected (no real `Utc::now()`)

`LoopDeps.clock` (mod.rs:104-105) is an injected **epoch-millis** clock:

```rust
    /// Epoch-millis clock for session-log record timestamps.
    pub clock: &'a (dyn Fn() -> u64 + Send + Sync),
```

The production clock (runner.rs:351-356) reads `SystemTime::now()`; tests inject
a deterministic one (`clock_zero`, or a fixed/advancing closure). **Use this
clock for the ceiling** — do **not** call `SystemTime::now()`/`Instant::now()`
directly anywhere in `executor/` (STANDARDS §3.3: no real `Utc::now()`, inject a
clock). The loop already captures a start timestamp from it at mod.rs:189:

```rust
    let mut metrics = RunMetrics::started_at((deps.clock)());
```

### `BudgetConfig` (`executor/src/config.rs:371-398`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Model's context-window size in tokens.
    pub context_length: usize,
    /// % of the model's context window the loop may fill before compacting.
    pub max_context_pct: u8,
    /// Hard cap on executor turns in one phase before budget_exceeded.
    pub max_turns: u32,
    /// Escalation slots (briefings returned to the architect) per phase.
    pub escalation_slots: u32,
    /// Max gate-retry loops at completion time before escalation. `None` = derive
    /// from `executor.tier`; ...
    #[serde(default)]
    pub gate_retries: Option<u32>,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            context_length: 32768,
            max_context_pct: 70,
            max_turns: 200,
            escalation_slots: 1,
            gate_retries: None,
        }
    }
}
```

`gate_retries` (added by phase-06) is the exact precedent for this phase: a new
`#[serde(default)]` `[budget]` field. **Unlike `gate_retries`, `wall_clock_secs`
is *not* tier-derived** — it is a flat opt-in ceiling. No `effective_*` helper,
no `ModelOverride` field. A plain `u64` seconds value, default `0` = disabled.

### The `LoopDeps` struct and its construction sites

`LoopDeps` (mod.rs:88-129) carries `gate_retries: u32` (mod.rs:94-97). This phase
adds a sibling `wall_clock_secs: u64` field. Every `LoopDeps { ... }` literal
must set it. **There are 15 construction sites** — grep-verified complete list
(each *already* has a `gate_retries:` line, so the mechanical rule is "add a
`wall_clock_secs:` line immediately after the `gate_retries:` line"):

| # | Site | `gate_retries` value | Add `wall_clock_secs:` |
|---|------|----------------------|------------------------|
| 1 | `mcp/src/runner.rs:276` | `inp.cfg.budget.effective_gate_retries(...)` | `inp.cfg.budget.wall_clock_secs` |
| 2 | `executor/src/agent/tests.rs:156` (`make_deps` / `deps` helper) | `u32::MAX` | `0` |
| 3 | `executor/src/agent/tests.rs:905` | `u32::MAX` | `0` |
| 4 | `executor/src/agent/tests.rs:1027` | `u32::MAX` | `0` |
| 5 | `executor/src/agent/tests.rs:1332` | `u32::MAX` | `0` |
| 6 | `executor/src/agent/tests.rs:1386` | `u32::MAX` | `0` |
| 7 | `executor/src/agent/tests.rs:1902` | `u32::MAX` | `0` |
| 8 | `executor/src/agent/tests.rs:2665` | `u32::MAX` | `0` |
| 9 | `executor/src/agent/tests.rs:2740` | `u32::MAX` | `0` |
| 10 | `executor/src/agent/tests.rs:2928` | `u32::MAX` | `0` |
| 11 | `executor/src/agent/tests.rs:3734` | `u32::MAX` | `0` |
| 12 | `executor/src/agent/tests.rs:3931` | `u32::MAX` | `0` |
| 13 | `executor/src/agent/tests.rs:3996` | `u32::MAX` | `0` |
| 14 | `executor/src/agent/tests.rs:4141` | `u32::MAX` | `0` |
| 15 | `executor/src/agent/tests.rs:4644` | `u32::MAX` | `0` |

Line numbers are approximate (they shift as you edit above them). **Do not trust
the numbers — grep.** Before finishing, run
`grep -rn 'gate_retries:' executor/src/agent/tests.rs mcp/src/runner.rs` and
confirm **every** match has an adjacent `wall_clock_secs:` line. An omitted site
is a compile error (`LoopDeps` has no default for the missing field), so
`cargo build` will catch it — but the two **new** tests you add (§Test plan) are
extra sites the grep won't list; they set `wall_clock_secs` explicitly.

`0` at every existing test site keeps their behavior byte-identical (the ceiling
branch is guarded by `> 0`).

### The turn loop's budget-terminal shape (`executor/src/agent/mod.rs`)

The loop starts at mod.rs:259. Its first step is the **context-budget terminal**
(Step 2, mod.rs:261-302) — the exact shape your wall-clock terminal mirrors:

```rust
    loop {
        // Step 2 — budget: compact on overflow, give up if still over.
        if deps.budget.would_overflow(&system, &messages) {
            let report = compact(&mut messages, deps.budget, &system);
            log_event(/* Compaction */);
            if deps.budget.would_overflow(&system, &messages) {
                log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
                emit_phase_run(&deps, input, "budget_exceeded", Gates::default(), &metrics, &scorer, turns);
                let artifacts = build_artifacts(
                    &pre_edit_content, deps.project_root, log_path.clone(),
                    "budget_exceeded", turns, CommandOutputs::default(),
                );
                return Ok(budget_exceeded_result(
                    input, &recent_tool_calls, deps.project_root,
                    "context budget exhausted".to_string(), artifacts,
                ));
            }
        }
        // Step 3 — call the model ...
```

Every symbol your terminal needs (`log_session_end`, `emit_phase_run`,
`Gates::default()`, `metrics`, `scorer`, `build_artifacts`,
`CommandOutputs::default()`, `budget_exceeded_result`, `turns`,
`recent_tool_calls`, `pre_edit_content`, `log_path`) is already in scope here —
this is the block to copy.

`budget_exceeded_result` (outcome.rs:42-59) builds a `Briefing` whose
`current_blocker` is `Blocker::BudgetExceeded` and whose `budget_remaining` is
the `String` you pass — that string is the only wall-clock-specific text.

### The `rexymcp init` `[budget]` template (`mcp/src/init.rs:28-32`)

```toml
[budget]
context_length = 32768            # model context window in tokens
max_context_pct = 70              # trigger compaction above this % (0–100)
max_turns = 200                   # hard cap on executor turns per phase
escalation_slots = 1              # turns reserved for the final command set retry
```

(`gate_retries` is intentionally *not* written to the template — it is
tier-derived; `wall_clock_secs` **is** written because it is a flat opt-in the
user sets directly.)

## Spec

Numbered tasks in execution order.

1. **Add the `wall_clock_secs` field to `BudgetConfig`** — in
   `executor/src/config.rs`, in the `BudgetConfig` struct (config.rs:372-386) add,
   after the `gate_retries` field:

   ```rust
       /// Optional wall-clock ceiling in seconds. When > 0, a run whose elapsed
       /// wall-clock time reaches this value terminates as `budget_exceeded` at
       /// the next turn boundary. `0` (the default) disables the ceiling.
       #[serde(default)]
       pub wall_clock_secs: u64,
   ```

   and in the `Default` impl (config.rs:388-398) add `wall_clock_secs: 0,` after
   `gate_retries: None,`.

2. **Add the `wall_clock_secs` field to `LoopDeps`** — in
   `executor/src/agent/mod.rs`, in the `LoopDeps` struct (mod.rs:88-129) add,
   after the `gate_retries` field (mod.rs:94-97):

   ```rust
       /// Wall-clock ceiling in seconds. `0` disables it; when > 0 a run whose
       /// elapsed time (measured off `clock`) reaches the ceiling terminates as
       /// `budget_exceeded`. Resolved from `[budget] wall_clock_secs` at the call
       /// site.
       pub wall_clock_secs: u64,
   ```

3. **Capture the loop start timestamp** — in `executor/src/agent/mod.rs`,
   immediately after the `RunMetrics::started_at` line (mod.rs:189) add:

   ```rust
       let loop_started_ms = (deps.clock)();
   ```

   This is the wall-clock baseline. (Do not reuse `metrics`' internal start — a
   dedicated local is clearer and has no getter dependency.)

4. **Add the wall-clock terminal at the top of the loop** — in
   `executor/src/agent/mod.rs`, as the **first** statement inside `loop {`
   (mod.rs:259), *before* the Step 2 compaction block (mod.rs:261), add:

   ```rust
        // Step 2a — wall-clock ceiling. A [budget] wall_clock_secs of 0 disables
        // it; otherwise a run past the ceiling terminates as budget_exceeded — a
        // clock-based budget terminal, distinct from the turn/context caps.
        if deps.wall_clock_secs > 0 {
            let elapsed_ms = (deps.clock)().saturating_sub(loop_started_ms);
            if elapsed_ms >= deps.wall_clock_secs.saturating_mul(1000) {
                log_session_end(&log_handle, &redactor, deps.clock, "budget_exceeded", turns);
                emit_phase_run(
                    &deps,
                    input,
                    "budget_exceeded",
                    Gates::default(),
                    &metrics,
                    &scorer,
                    turns,
                );
                let artifacts = build_artifacts(
                    &pre_edit_content,
                    deps.project_root,
                    log_path.clone(),
                    "budget_exceeded",
                    turns,
                    CommandOutputs::default(),
                );
                return Ok(budget_exceeded_result(
                    input,
                    &recent_tool_calls,
                    deps.project_root,
                    format!("wall-clock ceiling of {}s exceeded", deps.wall_clock_secs),
                    artifacts,
                ));
            }
        }
   ```

   Use `saturating_sub` / `saturating_mul` (a non-monotonic clock or a large
   `wall_clock_secs` must not panic on overflow). Placing this **before** Step 2
   makes the ceiling win at the turn boundary and keeps the disabled path
   (`wall_clock_secs == 0`) byte-identical to today.

5. **Wire the field at the production call site** — in `mcp/src/runner.rs`, in the
   `LoopDeps { ... }` literal (runner.rs:270-293) add, after the `gate_retries:`
   line (runner.rs:276):

   ```rust
           wall_clock_secs: inp.cfg.budget.wall_clock_secs,
   ```

6. **Set `wall_clock_secs: 0` at every existing test `LoopDeps` site** — in
   `executor/src/agent/tests.rs`, add `wall_clock_secs: 0,` immediately after each
   `gate_retries: u32::MAX,` line (the 14 sites in the § Current state table).
   Verify with the grep in § Current state that no site is missed.

7. **Document the knob in the `rexymcp init` template** — in `mcp/src/init.rs`, in
   the `[budget]` block (init.rs:28-32) add after the `escalation_slots` line:

   ```toml
   wall_clock_secs = 0               # optional wall-clock ceiling in seconds (0 disables)
   ```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `BudgetConfig` has a `wall_clock_secs: u64` field, `#[serde(default)]`,
      default `0`. Every pre-existing `config.rs`/`init.rs` parse test passes
      **unmodified** (additive `#[serde(default)]` field).
- [ ] `LoopDeps` has a `wall_clock_secs: u64` field; `mcp/src/runner.rs` sets it
      from `inp.cfg.budget.wall_clock_secs`; every existing test `LoopDeps` site
      sets it to `0`.
- [ ] A run configured with a wall-clock ceiling that the injected clock exceeds
      terminates as `budget_exceeded` (`Blocker::BudgetExceeded`) with a
      `budget_remaining` string containing `wall-clock` — **not** at the turn cap,
      and **before** the model completes.
- [ ] A run with `wall_clock_secs = 0` and the same advancing clock completes
      normally (the ceiling never fires) — proving `0` disables.
- [ ] `rexymcp init --config <tmp>/rexymcp.toml` (or `--dir <tmp> --force`) writes
      a `[budget]` block containing `wall_clock_secs`, and the written file loads
      without a parse error.

## Test plan

**Unit tests in `executor/src/config.rs`** (`#[cfg(test)] mod tests`), mirroring
the existing `budget_effective_gate_retries_*` tests:

- `budget_default_wall_clock_secs_is_zero` — `BudgetConfig::default().wall_clock_secs
  == 0`.
- `budget_parses_wall_clock_secs_from_toml` — parse a `[budget]` TOML string
  setting `wall_clock_secs = 30` (via the same `toml::from_str::<Config>` /
  `Config` load path the existing budget parse tests use) → `budget.wall_clock_secs
  == 30`; and a TOML **omitting** the key → `wall_clock_secs == 0` (serde default).

**Integration tests in `executor/src/agent/tests.rs`**, modelled on
`oscillation_across_alternating_reads_trips_hard_fail` (tests.rs:1290-1341) for
the inline `LoopDeps` shape. Add a deterministic **advancing** clock helper near
`clock_zero` (tests.rs:24) — the clock is `Fn` (not `FnMut`), so hold the counter
in an `AtomicU64`:

```rust
/// A deterministic clock that advances 10 seconds per call, so any nonzero
/// `wall_clock_secs` ceiling is crossed after the first loop iteration.
fn advancing_clock() -> impl Fn() -> u64 + Send + Sync {
    let calls = std::sync::atomic::AtomicU64::new(0);
    move || calls.fetch_add(10_000, std::sync::atomic::Ordering::Relaxed)
}
```

- `wall_clock_ceiling_trips_budget_exceeded` — build the clock with
  `let clock = advancing_clock();`, script a **single completion turn**
  (`MockAiClientScript::new(vec![vec![token("done")]])` — a plain token, no tool
  call), and construct `LoopDeps` inline (copy the tests.rs:1309 shape) with
  `clock: &clock`, `wall_clock_secs: 1`, `governor: GovernorConfig::default()`,
  `commands: &EMPTY_COMMANDS`, `runner: &NoopRunner`. Assert
  `result.status == PhaseStatus::BudgetExceeded`, that
  `result.briefing.unwrap().current_blocker` matches `Blocker::BudgetExceeded`,
  and that its `budget_remaining` contains `"wall-clock"`. Because the ceiling
  fires at the **top** of the loop (before Step 3), the completion turn is never
  consumed — this is what proves the ceiling preempts an otherwise-completing run.
- `wall_clock_disabled_when_zero_completes` — identical setup and script, but
  `wall_clock_secs: 0`. Assert `result.status == PhaseStatus::Complete` and
  `result.briefing.is_none()`. This proves the advancing clock alone causes no
  termination — only a nonzero ceiling does.

(No mock-exhaustion risk here: in the enabled test the model is never called, and
in the disabled test the single completion turn suffices. Unlike 07a's detector
tests, the ceiling fires before Step 3.)

## End-to-end verification

The behavioral outcome (a `budget_exceeded` termination on the ceiling) is
loop-internal and exercised hermetically by the integration tests above; a live
model that deterministically runs past a wall-clock ceiling is not hermetically
reproducible. The **runtime-loadable real artifact** this phase ships is the
config surface: the new `[budget] wall_clock_secs` knob the running binary parses
and the `rexymcp init` template documents. Verify that artifact:

```
$ cargo run -p rexymcp -- init --dir /tmp/p07b --force
$ grep -n 'wall_clock_secs' /tmp/p07b/rexymcp.toml
$ cargo run -p rexymcp -- health --config /tmp/p07b/rexymcp.toml   # loads the generated toml without a parse error
```

Quote the actual grep output (the documented `[budget]` line) and confirm
`health` does not error on the new key (a nonzero exit from an unreachable model
endpoint is fine; a **config parse error** is not), in the completion Update Log.

## Authorizations

None. No new dependency; no `Cargo.toml`/`architecture.md`/`STANDARDS.md`/
`WORKFLOW.md` edit. `mcp/src/init.rs` template edits and `executor/src/config.rs`
field additions are within scope (they are the config surface this phase ships).

## Out of scope

- **Any governor detector change** — `check_oscillation` / `check_windowed_output`
  (07a) and the three original detectors are untouched. This phase is a budget
  terminal, not a `HardFailSignal`. Do **not** add a `HardFailSignal` variant for
  the ceiling — it maps to the existing `budget_exceeded` outcome via
  `budget_exceeded_result`, exactly like the turn/context caps.
- **A new `PhaseResult` status** — reuse `budget_exceeded`; do not add an enum
  variant.
- **Tier derivation / `ModelOverride`** — unlike `gate_retries`, `wall_clock_secs`
  is a flat opt-in. No `effective_wall_clock_secs` helper, no `[models]` override
  field, no `resolve_for_model` block.
- **A per-turn or per-tool time budget** — this is a single whole-run ceiling
  checked at the turn boundary. Do not add mid-turn interruption, a tokio timeout
  on `client.chat`, or a `[budget]` sub-field for it.
- **Calling `SystemTime::now()` / `Instant::now()` inside `executor/`** — use the
  injected `deps.clock` only (STANDARDS §3.3). The production `SystemTime`-backed
  clock already lives at `mcp/src/runner.rs:351`.
- **Writing `gate_retries` to the `init` template** — it stays tier-derived and
  undocumented in the template; only `wall_clock_secs` is added there.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
