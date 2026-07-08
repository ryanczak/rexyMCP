# Phase 06: Wire `gate_retries` into the gate-retry loop

**Milestone:** M26 — Polish & Hardening
**Status:** done
**Depends on:** none
**Estimated diff:** ~150 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

`rexymcp calibrate` writes `[budget] gate_retries` (and derives it from
`[executor] tier`), and `BudgetConfig::effective_gate_retries` resolves it — but
**nothing in the loop reads it**. The M19 gate-retry loop re-injects a red gate's
output and loops until the *turn cap*, ignoring the retry budget entirely
(codebase review §1.1: "gate retries are bounded only by `max_turns`"). This
phase wires `effective_gate_retries(tier)` into that loop so a weak model stops
burning turns re-running gates it cannot fix, and terminates with a briefing the
architect can act on. `LARGE`/no-tier resolve to `u32::MAX` (unlimited), so
current behavior is preserved exactly.

**Scope note (decided with the user 2026-07-07):** the sibling escalation knobs —
`[budget] escalation_slots` and `[escalation] max_assists`/`EscalationConfig` — are
**explicitly deferred to M27** (the Autonomous Escalation Loop milestone). Their
consumer is the *architect-side* `/loop` driver, which does not exist yet; wiring
them into the executor now would violate WORKFLOW § "Derive intentionally" ("don't
have a phase populate something whose consumer doesn't exist yet") and would
contradict the architecture non-goal *"rexyMCP never links a cloud provider."*
This phase does **not** touch them beyond correcting a stale doc comment (Task 4).

## Architecture references

Read before starting:

- `docs/dev/STANDARDS.md` — the Definition of Done (§2.2 "No fallbacks for if X is
  missing" and "no silent degradation" is exactly what this closes).
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — the new `LoopDeps`
  field is an additive multi-site change; the E0063 traversal recipe applies.
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the loop
  terminates as `budget_exceeded` with a briefing; that briefing is the architect's
  input. This phase adds one more path to that outcome (gate-retry budget exhausted).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**The resolution helper already exists** (`executor/src/config.rs:376-383`) and is
unit-tested (`budget_effective_gate_retries_*`, config.rs:1322-1346). Nothing calls
it outside `config.rs`/tests:

```rust
impl BudgetConfig {
    /// Resolved gate_retries: explicit field wins; falls back to tier default;
    /// falls back to `u32::MAX` (unlimited, bounded by `max_turns`).
    pub fn effective_gate_retries(&self, tier: Option<Tier>) -> u32 {
        self.gate_retries
            .or_else(|| tier.map(|t| t.default_gate_retries()))
            .unwrap_or(u32::MAX)
    }
}
```

`Tier::default_gate_retries` (config.rs:43-49) returns `LARGE => u32::MAX`,
`MEDIUM => 2`, `SMALL => 1`.

**The gate-retry loop** lives in the `ParseResult::NoToolCall` completion arm of
`execute_phase` (`executor/src/agent/mod.rs`). After a productive completion the
loop runs the final command set, then this block (mod.rs:732-781) re-injects a red
gate and loops, bounded only by `turns >= deps.max_turns`:

```rust
if let Some(feedback) = command::gate_failure_feedback(&gates, &command_outputs)
{
    log_event(
        &log_handle,
        &redactor,
        deps.clock,
        turns,
        SessionEvent::Progress {
            turn: turns,
            stage: "gate_retry".to_string(),
            files_changed: vec![],
            message: feedback.clone(),
        },
    );
    messages.push(user_text(&feedback, turns));
    if turns >= deps.max_turns {
        log_session_end(
            &log_handle,
            &redactor,
            deps.clock,
            "budget_exceeded",
            turns,
        );
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
            turns_line(deps.max_turns),
            artifacts,
        ));
    }
    continue;
}
```

**`LoopDeps`** (mod.rs:88-125) is the injected-dependency struct. `max_turns` is
already a *resolved* field (`pub max_turns: usize`) — the config is resolved at the
call site, not inside the loop. The new `gate_retries` field follows this precedent
exactly.

**The single production construction site** is `mcp/src/runner.rs:270`, where
`max_turns` is resolved:

```rust
    let deps = LoopDeps {
        // ...
        budget: &budget,
        max_turns: inp.cfg.budget.max_turns as usize,
        // ...
    };
```

`inp.cfg` is in scope here (`cfg.budget`, `cfg.executor.tier`), so
`effective_gate_retries` resolves cleanly at this site.

**`budget_exceeded_result`** (`executor/src/agent/outcome.rs:42-49`) takes a
`budget_remaining: String` reason as its 4th argument — today the gate-retry path
passes `turns_line(deps.max_turns)`. A gate-retry-exhaustion termination should pass
a *different* reason so the briefing tells the architect which budget ran out.

## Spec

Numbered tasks in execution order.

1. **Add a resolved `gate_retries` field to `LoopDeps`** — in
   `executor/src/agent/mod.rs`, in the `LoopDeps<'a>` struct (mod.rs:88-125), add a
   field after `max_turns` for cohesion:

   ```rust
   /// Resolved gate-retry budget: max gate-retry rounds at completion time
   /// before `budget_exceeded`. `u32::MAX` = unlimited (bounded by `max_turns`).
   /// Resolved from `[budget] gate_retries` / `[executor] tier` at the call site.
   pub gate_retries: u32,
   ```

2. **Declare a gate-retry counter** — in `execute_phase`, alongside the existing
   stall counters (near mod.rs:171, next to `consecutive_gate_repeats`):

   ```rust
   // M19 gate-retry rounds consumed (M26 phase-06: bounded by deps.gate_retries).
   let mut gate_retry_count: u32 = 0;
   ```

3. **Wire the counter + budget check into the gate-retry block** — in the
   `if let Some(feedback) = command::gate_failure_feedback(...)` block quoted under
   Current state (mod.rs:732-781), after `messages.push(user_text(&feedback, turns));`,
   increment the counter and terminate when *either* the gate-retry budget *or* the
   turn cap is exhausted, with a reason string that names the cause. Replace:

   ```rust
       messages.push(user_text(&feedback, turns));
       if turns >= deps.max_turns {
   ```

   with:

   ```rust
       messages.push(user_text(&feedback, turns));
       gate_retry_count += 1;
       let gate_budget_exhausted = gate_retry_count >= deps.gate_retries;
       if gate_budget_exhausted || turns >= deps.max_turns {
   ```

   and change the `budget_exceeded_result` reason argument in that same block from
   `turns_line(deps.max_turns)` to a computed reason:

   ```rust
       let reason = if gate_budget_exhausted {
           format!("gate-retry budget exhausted after {gate_retry_count} retries")
       } else {
           turns_line(deps.max_turns)
       };
       return Ok(budget_exceeded_result(
           input,
           &recent_tool_calls,
           deps.project_root,
           reason,
           artifacts,
       ));
   ```

   Leave the `log_session_end` / `emit_phase_run` / `build_artifacts` calls in that
   branch unchanged (still `"budget_exceeded"`). **Do not** add the counter to the
   task-coverage block (mod.rs:784+) or the A3 peek-guard (mod.rs:661-730) — those
   are separate M21/M22 concerns and `gate_retries` bounds gate retries only.

4. **Resolve `gate_retries` at the production call site** — in `mcp/src/runner.rs`,
   at the `LoopDeps { ... }` construction (runner.rs:270), add after the `max_turns`
   line:

   ```rust
       gate_retries: inp.cfg.budget.effective_gate_retries(inp.cfg.executor.tier),
   ```

5. **Fix the E0063 fallout in tests** — adding a `LoopDeps` field breaks every
   construction site with `error[E0063]: missing field gate_retries`. Run a
   compiler-guided traversal (`cargo build 2>&1 | grep E0063`) and add
   `gate_retries: u32::MAX` at **every** test construction site in
   `executor/src/agent/tests.rs`. `u32::MAX` = unlimited, so every existing test's
   behavior stays **byte-identical** (this is the backward-compat pin). Grep the
   sites first:

   ```
   grep -n "LoopDeps {" executor/src/agent/tests.rs
   ```

   As of drafting there are ~9 sites (inline `let d = LoopDeps {` blocks plus one
   builder `fn build(self) -> LoopDeps<'a>`). Trust the compiler over this count —
   add the field until `cargo build` is green.

6. **Correct the stale config doc comments** (doc-only, no behavior change) — in
   `executor/src/config.rs`, two comments claim escalation is "wired in M21", which
   is false (M21 shipped the task-coverage gate; review §1.1). Update them to reflect
   reality:
   - The `Tier` doc comment (config.rs:20-22) says tier controls "default `max_turns`,
     `gate_retries`, and whether mid-phase Architect escalation is enabled (SMALL
     only, wired in M21)." Change the parenthetical to note that `gate_retries` is
     wired as of M26, and escalation budgeting is deferred to M27 (architect-side
     loop). Keep it to one sentence.
   - The `EscalationConfig` doc comment (config.rs:52-54) says "wired in M21." Change
     "(wired in M21)" to reflect that `max_assists` is consumed by the architect-side
     `/loop` (M27), not the executor loop.

   Do not change any code, field, or default in `config.rs` — comments only.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] `LoopDeps` has a `gate_retries: u32` field; `mcp/src/runner.rs` resolves it via
      `inp.cfg.budget.effective_gate_retries(inp.cfg.executor.tier)`.
- [ ] With `gate_retries = 2` and a persistently red gate, the loop returns
      `budget_exceeded` after exactly 2 gate-retry rounds, **before** the turn cap.
- [ ] With `gate_retries = u32::MAX`, behavior is unchanged: the loop retries to the
      turn cap. The existing tests `gate_failure_loops_until_gates_pass` and
      `gate_failure_at_turn_cap_is_budget_exceeded` pass **unmodified** (aside from
      the additive `gate_retries: u32::MAX` field).
- [ ] No `[escalation]`/`escalation_slots`/`max_assists` code or field changed;
      only their doc comments corrected.

## Test plan

Two new integration tests in `executor/src/agent/tests.rs`, modelled on the
existing `gate_failure_at_turn_cap_is_budget_exceeded` (find it and mirror its
`ScriptedCommandRunner`/`MockAiClient` setup):

- `gate_retry_budget_exhaustion_returns_budget_exceeded_before_turn_cap` — script a
  model that completes each turn (no tool call → completion attempt) and a command
  runner whose gate stays **red** every time. Build `LoopDeps` with
  `gate_retries: 2` and a comfortably high `max_turns` (e.g. 50). Assert the result
  is `budget_exceeded` **and** that the model was called few enough times to prove
  termination happened at the retry budget, not the turn cap (assert
  `client.calls().len()` is small — around 3 — not ~50). This is the load-bearing
  positive pin: it must fail if the counter/`>=` check is removed.
- `unlimited_gate_retries_retries_to_turn_cap` — same red-gate setup but
  `gate_retries: u32::MAX` and a **low** `max_turns` (e.g. 3). Assert the result is
  `budget_exceeded` and `client.calls().len()` reflects reaching the turn cap
  (mutation resistance for the `||` condition — proves `u32::MAX` does not
  short-circuit early).

The config-layer resolution (`effective_gate_retries`) is already covered by the
existing `budget_effective_gate_retries_*` unit tests — do not duplicate them.

## End-to-end verification

> Not applicable — phase ships a loop-internal control-flow change with no new
> CLI-, MCP-, or config-surfaced artifact (the `gate_retries` field and its
> `calibrate` writer already exist and are unchanged). The behavior is exercised
> hermetically by the two new `MockAiClient` integration tests driving a
> persistently-red gate; a live demonstration would require a model that
> deterministically fails a real gate, which is not hermetically reproducible.

## Authorizations

None. (No new dependency; no `Cargo.toml`/`architecture.md`/`STANDARDS.md`/
`WORKFLOW.md` edit; `config.rs` change is comment-only.)

## Out of scope

- **`escalation_slots` and `max_assists`/`EscalationConfig`** — deferred to M27
  (architect-side autonomous `/loop`). Do not wire, retire, or otherwise change
  their fields/defaults. Only their doc comments are corrected (Task 6).
- **`calibrate`** (`mcp/src/calibrate.rs`) — it already writes `gate_retries`
  correctly; no change. Do not touch its `max_assists` write either.
- **A new `PhaseResult` status** — gate-retry exhaustion maps to the existing
  `budget_exceeded`; do not add an enum variant.
- **The task-coverage retry (M21) and A3 stuck-gate stall (M22)** — separate
  concerns; `gate_retries` bounds only the `gate_failure_feedback` block.
- **`effective_max_turns`** (config.rs:390) — the reserved tier hook; leave it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-08 (escalation)

**Chosen lever:** refined re-dispatch (no spec change — plain retry)
**Rationale:** hard_fail at turn 3 was a backend infrastructure blip, not a spec
gap: the model called `read_file` with `arguments: null` (a normal recoverable
tool-result error, handled gracefully), but the *next* request to the backend
(`http://brain:8000/v1`) 400'd with `"Can only get item pairs from a mapping"` —
a chat-template error rendering the prior null-argument tool call back into the
prompt, not a spec ambiguity the executor couldn't resolve. No files were
touched; `Status:` never left `todo`. Re-dispatch as-is via
`/rexymcp:dispatch phase-06`. If this backend-400-on-null-args pattern recurs,
it's worth a future investigation into how the executor serializes a
failed-tool-call turn for the next request — not a phase-doc fix.

### Update — 2026-07-08 (escalation, 2nd dispatch)

**Chosen lever:** session takeover
**Rationale:** the 2nd dispatch (session `6a4dba4a`) ran to `budget_exceeded`
at 400/400 turns, but landed the entire production Spec correctly and
byte-identical to the pre-injected fragments (`LoopDeps.gate_retries`,
the counter + `gate_budget_exhausted` check, the `runner.rs` resolve call,
and both `config.rs` doc-comment corrections — grep-confirmed "wired M26" /
"deferred to M27" landed). Tasks 1–5 were marked done via `update_task`; task
6 (doc comments) was never marked done but its edit is present in the diff.
The blocker was the two new tests: `MockAiClientScript::new(vec![vec![token("All
done.")]])` scripts only **one** model turn — once exhausted, `chat()` sends
zero events, and an empty completion routes to the *unrelated*
empty-completion recovery branch (`mod.rs:531`) instead of re-running
`gate_failure_feedback`, so the loop drifted to the turn cap instead of the
gate-retry budget. The executor spent turns 70–400 oscillating (re-reading
the same ~15-line window of `tests.rs` without re-running `cargo test` after
turn ~125) and never diagnosed the mock's turn-exhaustion behavior — a
genuine capability gap, not a spec gap a re-dispatch refinement would fix
quickly. Took over directly: scripted 4 (resp. 3) `"All done."` turns in the
two new tests; no production code changed. See completion entry below.

### Update — 2026-07-08 (complete, architect takeover)

**Executor:** Claude (direct)
**Verdict:** escalated (session takeover after 2nd dispatch budget_exceeded)

All six Spec tasks landed via the 2nd dispatch (session `6a4dba4a`) with
production code matching the phase doc's pre-injected fragments verbatim.
The only defect was in the two new tests' `MockAiClientScript` setup (see
escalation entry above) — fixed by scripting enough `"All done."` turns to
cover the full expected model-call count in each test, matching the
existing-test precedent's implicit assumption that the mock's turn queue is
never exhausted mid-test.

**Files changed:**
- `executor/src/agent/mod.rs` — `LoopDeps.gate_retries: u32` field;
  `gate_retry_count` counter; `gate_budget_exhausted` check in the
  gate-retry block; computed `reason` string for the briefing.
- `executor/src/agent/tests.rs` — `gate_retries: u32::MAX` at all 12
  `LoopDeps` construction sites (E0063 fallout); two new integration tests
  (`gate_retry_budget_exhaustion_returns_budget_exceeded_before_turn_cap`,
  `unlimited_gate_retries_retries_to_turn_cap`), fixed at takeover to script
  4/3 `MockAiClientScript` turns respectively instead of 1.
- `executor/src/config.rs` — doc-comment only: `Tier` and `EscalationConfig`
  comments corrected to say `gate_retries` is wired M26 and escalation
  budgeting is deferred to M27.
- `mcp/src/runner.rs` — resolves `gate_retries` via
  `inp.cfg.budget.effective_gate_retries(inp.cfg.executor.tier)` at the
  `LoopDeps` construction site.

**Grep proving the pinned literals landed:**
```
$ grep -n "gate_retries (wired M26)\|deferred to M27" executor/src/config.rs
21:/// `[executor].tier`. Controls default `max_turns`, `gate_retries` (wired M26),
22:/// and whether mid-phase Architect escalation is enabled (deferred to M27).
```

**Verification (all four gates green, run independently at takeover):**
```
$ cargo fmt --all --check
(no output, exit 0)

$ cargo build
   Compiling rexymcp-executor v0.9.1
   Compiling rexymcp v0.9.1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.89s

$ cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.51s

$ cargo test
test result: ok. 439 passed; 0 failed; 0 ignored (mcp)
test result: ok. 888 passed; 0 failed; 2 ignored (executor)
```

Both new tests pass individually and as part of the full suite; the existing
`gate_failure_loops_until_gates_pass` / `gate_failure_at_turn_cap_is_budget_exceeded`
tests pass unmodified (only the additive `gate_retries: u32::MAX` field, per
the backward-compat pin).

**End-to-end verification:** N/A per the phase doc — no new CLI/MCP/config
surface; behavior is exercised hermetically by the two `MockAiClient`
integration tests.

**Notes for review:** No scope deviation — `escalation_slots`/`max_assists`
untouched beyond their doc comments, as authorized. No new dependency, no
`Cargo.toml`/`architecture.md`/`STANDARDS.md`/`WORKFLOW.md` edit.
**Calibration note:** 2nd occurrence of an executor stalling on a
mock-harness subtlety (test-authoring capability gap) rather than a
production-code gap — the model can follow a precisely-specified production
change but struggles to reason about a test double's internal state
(queue exhaustion) when a failure doesn't match its mental model. Flagged
for the user; not yet a fold.
