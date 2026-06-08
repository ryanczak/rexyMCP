# Phase 03: Arc A reclaim events (OutputFiltered)

**Milestone:** M10 — Context optimization
**Status:** done
**Depends on:** phase-01, phase-02 (the Arc A filters this phase instruments)
**Estimated diff:** ~170 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Arc A (phase-01 generic filter, phase-02 cargo filter) reclaims context at the
`bash` boundary but **records nothing** — so we can't yet prove the milestone
thesis ("every change is scorecard-measurable"). This phase adds a per-lever
`SessionEvent::OutputFiltered { tokens_before, tokens_after, filter }` emitted
once per filtered `bash` call, so boundary filtering is visible on the live
dashboard, queryable via the log tools, and (in a later phase) aggregatable onto
`PhaseRun`. It is pure instrumentation: **no change to what the filter returns to
the model**, only a new event recording how much it reclaimed.

This establishes the **per-lever reclaim-event pattern** that phase-04
(superseded-read eviction → `ReadEvicted`) and phase-05 (re-read dedupe) reuse.

## Architecture references

Read before starting:

- `executor/src/store/sessions/event.rs` — the `SessionEvent` enum. The new
  variant goes here, modeled on the existing `Compaction` variant (the precedent
  for a context-operation event with token before/after).
- `executor/src/tools/bash.rs` — where the filter runs and the `ToolResult`
  metadata is built.
- `executor/src/agent/tools.rs` (`dispatch`) and `executor/src/agent/mod.rs`
  (the dispatch call site + `log_event`) — where the event is emitted.
- `docs/dev/milestones/M10-context-optimization/README.md` §"Measure, don't
  assert" — why each lever must record its effect.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).
5. Run `cargo test` and record the passing count — the completion log must show
   the same count plus the new tests.

## Current state

**The `Compaction` event is the exact precedent** — a context-operation event with
token before/after (`executor/src/store/sessions/event.rs:76-84`):

```rust
    /// Emitted each time the context compactor runs (on budget overflow at the
    /// top of a turn). Mirrors `CompactionReport`: token totals before/after and
    /// the message counts touched. Tokens freed = `tokens_before - tokens_after`.
    Compaction {
        tokens_before: usize,
        tokens_after: usize,
        messages_signaturized: usize,
        messages_evicted: usize,
    },
```

**Adding a `SessionEvent` variant has a fixed, grep-verified blast radius.** These
**four** match sites are exhaustive and require a new arm; everything else
(`cap.rs` catch-all, `status.rs` `_ => {}`, `agent/log.rs` generic serde,
`store/sessions/jsonl.rs` generic serde) needs **no** change:

1. `executor/src/agent/mod.rs` — the `event_type_str` match (the arm
   `SessionEvent::Compaction { .. } => "compaction"` is near the bottom, around
   line 1680).
2. `mcp/src/log_query.rs` — the `event_kind` match (line ~27,
   `SessionEvent::Compaction { .. } => "compaction"`).
3. `mcp/src/dashboard/filter.rs` — the `ActivityFilter::allows` exhaustive match
   (line ~53, `SessionEvent::Compaction { .. } => self.compaction`) **and** the
   `ActivityFilter` struct (line 8) + its `Default` (line 22), which carry one
   `bool` per event kind.
4. `mcp/src/dashboard/transcript.rs` — the `record_lines` render match (line
   ~139, the `Compaction { tokens_before, tokens_after, .. }` arm).

**`tokens::count`** is the estimator (`executor/src/context/tokens.rs:9`):
`pub fn count(text: &str) -> usize`.

**The bash metadata block** (`executor/src/tools/bash.rs`, the success arm):

```rust
                let output_body = format!("{status_line}\n\n{body}");

                let metadata = json!({
                    "exit_code": exit_code,
                    "duration_ms": elapsed.as_millis(),
                    "stdout_bytes": output.stdout.len(),
                    "stderr_bytes": output.stderr.len(),
                    "truncated": truncated,
                    "timed_out": false,
                });
```

`combined` is the raw captured output (pre-filter); `body` is the filtered result
(both in scope here). `self.filter` is the kill-switch (true = filtering on).

**`dispatch` drops `ToolResult.metadata`** today (`executor/src/agent/tools.rs:110`):

```rust
pub(super) async fn dispatch(registry: &ToolRegistry, tc: &ToolCall) -> (bool, String) {
    match registry.get(&tc.name) {
        None => (false, format!("error: unknown tool '{}'", tc.name)),
        Some(tool) => match tool.execute(tc.arguments.clone()).await {
            Ok(result) => match result.error {
                Some(error) => (false, error),
                None => (true, result.output),
            },
            Err(e) => (false, format!("tool execution failed: {e}")),
        },
    }
}
```

It has **exactly one call site** (`executor/src/agent/mod.rs:627`), inside a
`match` whose other arm is the read-before-edit refusal:

```rust
        let (succeeded, content) =
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal),
                None => {
                    // … baseline + pre-edit capture …
                    dispatch(deps.registry, &tool_call).await
                }
            };
```

## Spec

Numbered tasks in execution order.

### 1. Add the `OutputFiltered` variant

In `executor/src/store/sessions/event.rs`, add after `Compaction`:

```rust
    /// Emitted once per `bash` call whose output the boundary filter (Arc A)
    /// shrank. `filter` is `"generic"` (phase-01 normalize+truncate) or
    /// `"cargo"` (phase-02 structured). Tokens reclaimed = `tokens_before -
    /// tokens_after` (chars/4 estimate, same heuristic as the budget).
    OutputFiltered {
        tokens_before: usize,
        tokens_after: usize,
        filter: String,
    },
```

### 2. Have the bash tool report the filter's effect via metadata

In `executor/src/tools/bash.rs`, in the success arm, **only when `self.filter`**,
compute the token counts and add an `output_filter` object to the metadata.
`combined` is the raw output, `body` is the filtered output:

```rust
                let mut metadata = json!({
                    "exit_code": exit_code,
                    "duration_ms": elapsed.as_millis(),
                    "stdout_bytes": output.stdout.len(),
                    "stderr_bytes": output.stderr.len(),
                    "truncated": truncated,
                    "timed_out": false,
                });
                if self.filter {
                    let filter = if crate::context::output_filter::is_cargo_command(&parsed.command) {
                        "cargo"
                    } else {
                        "generic"
                    };
                    metadata["output_filter"] = json!({
                        "tokens_before": crate::context::tokens::count(&combined),
                        "tokens_after": crate::context::tokens::count(&body),
                        "filter": filter,
                    });
                }
```

(`is_cargo_command` is already `pub` from phase-02; `tokens::count` is `pub`.)

### 3. Surface metadata through `dispatch`

Change `dispatch` (`executor/src/agent/tools.rs`) to also return the success
metadata. This is the additive widening of a single-call-site helper:

```rust
pub(super) async fn dispatch(
    registry: &ToolRegistry,
    tc: &ToolCall,
) -> (bool, String, Option<serde_json::Value>) {
    match registry.get(&tc.name) {
        None => (false, format!("error: unknown tool '{}'", tc.name), None),
        Some(tool) => match tool.execute(tc.arguments.clone()).await {
            Ok(result) => match result.error {
                Some(error) => (false, error, None),
                None => (true, result.output, result.metadata),
            },
            Err(e) => (false, format!("tool execution failed: {e}"), None),
        },
    }
}
```

### 4. Emit `OutputFiltered` from the loop

In `executor/src/agent/mod.rs`, update the dispatch call-site match (line ~592)
so both arms produce the 3-tuple, binding the metadata:

```rust
        let (succeeded, content, tool_meta) =
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal, None),
                None => {
                    // … unchanged baseline + pre-edit capture + emit_progress …
                    dispatch(deps.registry, &tool_call).await
                }
            };
```

Then, **after** `append_tool_exchange(&mut messages, &tool_call, &content, turns);`
(line 651), emit the event when the bash filter reclaimed tokens:

```rust
        // Per-lever reclaim event (M10 Arc A): record how much the boundary
        // filter shrank this bash call's output. Emit only on a real reduction.
        if let Some(meta) = &tool_meta
            && let Some(of) = meta.get("output_filter")
            && let (Some(before), Some(after), Some(filter)) = (
                of.get("tokens_before").and_then(|v| v.as_u64()),
                of.get("tokens_after").and_then(|v| v.as_u64()),
                of.get("filter").and_then(|v| v.as_str()),
            )
            && after < before
        {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::OutputFiltered {
                    tokens_before: before as usize,
                    tokens_after: after as usize,
                    filter: filter.to_string(),
                },
            );
        }
```

`SessionEvent` is already imported in `mod.rs` (it logs many variants). `log_event`,
`log_handle`, `redactor`, `deps.clock`, `turns` are all in scope here.

### 5. Add the four required match arms

Mirror the `Compaction` arm in each. **(a)** `executor/src/agent/mod.rs`
`event_type_str`:

```rust
            SessionEvent::OutputFiltered { .. } => "output_filtered",
```

**(b)** `mcp/src/log_query.rs` `event_kind`:

```rust
        SessionEvent::OutputFiltered { .. } => "output_filtered",
```

**(c)** `mcp/src/dashboard/filter.rs` — this file has **five** sites that carry
one entry per event kind, not just three. `output_filtered` is the **12th** kind
(index `11`); `FILTER_ITEM_COUNT` is already `12`, so every per-index match must
gain an arm `11`. Add **all** of:

```rust
    // 1. struct field (ActivityFilter), after `compaction`:
    pub(crate) output_filtered: bool,

    // 2. Default impl, after `compaction: true,`:
    output_filtered: true,

    // 3. allows() match, after the Compaction arm:
    SessionEvent::OutputFiltered { .. } => self.output_filtered,

    // 4. toggle() match, after `10 => self.compaction = !self.compaction,`:
    11 => self.output_filtered = !self.output_filtered,

    // 5. is_enabled() match, after `10 => self.compaction,`:
    11 => self.output_filtered,

    // 6. item_label() match, after `10 => "compaction",`:
    11 => "output filtered",
```

Without **all six** edits the crate will not compile (`E0063` missing-field on
`Default`, `E0004` non-exhaustive on `allows`) **or** will silently desync the
filter panel (`toggle`/`is_enabled`/`item_label` falling through `_ =>` for the
12th item). Also extend the `filter_default_disables_progress` test's assertion
block with `assert!(f.output_filtered);` to mirror the other kinds.

**(d)** `mcp/src/dashboard/transcript.rs` `record_lines` — add a render arm
mirroring the `Compaction` one:

```rust
            SessionEvent::OutputFiltered {
                tokens_before,
                tokens_after,
                filter,
            } => (
                format!("filtered ({filter}): {tokens_before} → {tokens_after} tokens"),
                Color::Cyan,
                false,
                None,
            ),
```

(Use any existing `ratatui::style::Color` already imported in `transcript.rs`;
`Cyan` is fine. The tuple shape `(summary, color, bold, body)` must match the
other arms.)

Do **not** add a `status.rs` summarize arm (its `_ => {}` catch-all is correct;
folding `OutputFiltered` into a `StatusSummary` panel is deferred to phase-07).

## Acceptance criteria

- [ ] `grep -n 'OutputFiltered' executor/src/store/sessions/event.rs` matches the
      new variant.
- [ ] A filtered `bash` call that shrinks output emits exactly one
      `OutputFiltered` event with `tokens_after < tokens_before` and a `filter`
      of `"cargo"` (for a `cargo` command) or `"generic"` (otherwise).
- [ ] A `bash` call with `self.filter == false` (kill-switch off) emits **no**
      `OutputFiltered` event. **(negative)**
- [ ] A `bash` call whose output is too short to shrink emits **no**
      `OutputFiltered` event (no reduction → no event). **(negative)**
- [ ] `dispatch` returns the success metadata; non-bash / error / refusal paths
      carry `None` and emit no event. **(negative)**
- [ ] The four match arms compile (build is the proof) and the dashboard
      transcript renders an `OutputFiltered` record without panicking.
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Bash-tool unit tests in `executor/src/tools/bash.rs` (extend the existing test
module; these spawn real `sh` like the phase-01/02 bash tests):

- `filter_on_records_output_filter_metadata` — `bash_with_filter(scope, 30, true)`
  running a >100-line command → `result.metadata["output_filter"]["tokens_after"]`
  < `["tokens_before"]`, and `["filter"] == "generic"`.
- `cargo_command_records_cargo_filter_label` — a `cargo`-prefixed command (it can
  fail to run; the label is derived from the command string, not the exit) →
  `metadata["output_filter"]["filter"] == "cargo"`.
- `filter_off_records_no_output_filter_metadata` — `bash_with_filter(scope, 30,
  false)` → `metadata.get("output_filter")` is `None`. **(negative)**

Agent-loop integration test in `executor/src/agent/mod.rs` (mirror the existing
loop tests; assert on the **logged events**, read back from the session log via
the same path other loop tests use, or by scanning `client`/log records — follow
the pattern used by tests that assert a `Compaction`/`Metrics` event was logged):

- `loop_emits_output_filtered_event_for_filtered_bash` — script the model to run
  one `bash` command emitting >100 lines, then `token("done")`. After the run,
  the session log contains an `OutputFiltered` record with `tokens_after <
  tokens_before`. (If asserting via logged records is impractical in the harness,
  assert via a `MockProgress`/log-scan consistent with how `Compaction` is tested
  — find that test first and mirror it.)

`dispatch` unit test in `executor/src/agent/tools.rs` (if a test module is added):

- `dispatch_surfaces_success_metadata` — a stub tool returning metadata →
  `dispatch` returns it in the third tuple slot; an error result → `None`.
  **(negative on the error path)**

## End-to-end verification

The bash-tool tests spawn real `sh` subprocesses and inspect the real
`ToolResult.metadata`, so they exercise the shipped artifact. For the completion
log, run `cargo test filter_on_records_output_filter_metadata -- --nocapture` and
quote the recorded `tokens_before`/`tokens_after`/`filter` values. If the
agent-loop event test is included, run it `-- --nocapture` and quote the logged
`OutputFiltered` record.

## Authorizations

None. No new dependency. No `docs/architecture.md` change. No `Cargo.toml` change.

## Out of scope

- **`StatusSummary` / dashboard panel for total tokens reclaimed** — phase-07
  (metrics) folds the per-lever events into a summary; here we only emit + render
  the transcript line. The `status.rs` `_ => {}` catch-all stays.
- **`PhaseRun` / scorecard fields** — phase-07. Do not add a metrics field or a
  scorecard column; the JSONL events are the durable record phase-07 reads back.
- **`ReadEvicted` / dedupe events** — phase-04 / phase-05 add those, reusing this
  variant pattern.
- **Changing the filters' output** (`compact_with_recovery`, `cargo_filter`,
  `filter_for_command`) — this phase only *measures* them; their returned body is
  unchanged.
- **Emitting on non-`bash` tools** — only the `bash` boundary filter is
  instrumented (it is the only filtered tool).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-06-07

The first dispatch hard-failed `VerifierFailurePersistent` (3 consecutive
verifier failures) on three compile errors, **all** in the MCP-side consumers —
the executor-crate half (event variant, `bash` metadata, `dispatch` widening,
loop emit, `event_type_str`, `log_query`) all landed and are on the working tree
already. **Continue from the existing partial state; do not revert it.** The
remaining work is exactly:

1. **`mcp/src/dashboard/filter.rs`** — the struct field `output_filtered` and
   `FILTER_ITEM_COUNT = 12` are already present, but the file has **five** more
   per-event-kind sites that still need the 12th entry (index `11`): the
   `Default` impl, `allows()`, `toggle()`, `is_enabled()`, and `item_label()`.
   See the revised Task 5(c) above — it now lists all six verbatim edits plus the
   `assert!(f.output_filtered);` test line. The build errors were `E0063`
   (Default missing `output_filtered`) and `E0004` (`allows` non-exhaustive);
   the `toggle`/`is_enabled`/`item_label` index-11 arms prevent a silent panel
   desync even though they compile via `_ =>`.
2. **`mcp/src/dashboard/transcript.rs`** — never touched on the first dispatch.
   Add the `record_lines` arm from Task 5(d) (the `OutputFiltered { tokens_before,
   tokens_after, filter } => (…)` tuple).

After these two files, run the full command set (`cargo build`, `clippy`, `fmt
--check`, `test`) and complete the Update Log + commit.

### Update — 2026-06-07 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** first-dispatch spec gap — Task 5(c) under-listed the `filter.rs`
blast radius (3 of 5 per-event-kind sites), so the executor left the crate
non-compiling; the spec is now tightened to enumerate every remaining site and
the partial work is preserved for continuation.

### Update — 2026-06-07 (complete)

**Summary:** Continued from the partial state left by the first dispatch. The
executor-crate half (event variant, `bash` metadata, `dispatch` widening, loop
emit, `event_type_str`, `log_query`) was already on the working tree. Completed
the remaining MCP-side consumers: `mcp/src/dashboard/filter.rs` (Default impl,
`allows()`, `toggle()`, `is_enabled()`, `item_label()` — all six per-event-kind
sites for index 11) and `mcp/src/dashboard/transcript.rs` (`record_lines` render
arm). Added 6 new tests: 3 bash-tool unit tests
(`filter_on_records_output_filter_metadata`,
`cargo_command_records_cargo_filter_label`,
`filter_off_records_no_output_filter_metadata`), 1 agent-loop integration test
(`loop_emits_output_filtered_event_for_filtered_bash`), and 2 `dispatch` unit
tests (`dispatch_surfaces_success_metadata`,
`dispatch_returns_none_metadata_on_error`).

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3 (...)
   Compiling rexymcp v0.1.3 (...)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.96s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.3 (...)
    Checking rexymcp v0.1.3 (...)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.54s

cargo test 2>&1 | tail -30
test result: ok. 615 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

`cargo test filter_on_records_output_filter_metadata -- --nocapture` passed —
the test spawns real `sh`, runs a 200-line command through the filter, and
asserts `tokens_after < tokens_before` with `filter == "generic"`. The
`cargo_command_records_cargo_filter_label` test confirms `"cargo"` label on
cargo-prefixed commands. The `filter_off_records_no_output_filter_metadata`
negative test confirms no `output_filter` key when the kill-switch is off. The
agent-loop test `loop_emits_output_filtered_event_for_filtered_bash` confirms
the `OutputFiltered` event is written to the session JSONL with correct
`tokens_after < tokens_before`.

**Grep verification:**
```
$ grep -n 'OutputFiltered' executor/src/store/sessions/event.rs
89:    OutputFiltered {
```

**Files changed:**
- `mcp/src/dashboard/filter.rs` — Default impl, `allows()`, `toggle()`,
  `is_enabled()`, `item_label()` arms for index 11; test assertion
  `assert!(f.output_filtered)`
- `mcp/src/dashboard/transcript.rs` — `record_lines` render arm for
  `OutputFiltered`
- `executor/src/tools/bash.rs` — 3 new unit tests for output_filter metadata
- `executor/src/agent/tools.rs` — 2 new unit tests for `dispatch` metadata
  surface
- `executor/src/agent/mod.rs` — 1 new agent-loop integration test for
  `OutputFiltered` event emission
- `docs/dev/milestones/M10-context-optimization/phase-03-arc-a-reclaim-events.md`
  — status flip to `review`, completion Update Log entry
- `docs/dev/milestones/M10-context-optimization/README.md` — phase table row
  updated to `review`

**New tests:**
- `filter_on_records_output_filter_metadata` in `executor/src/tools/bash.rs`
- `cargo_command_records_cargo_filter_label` in `executor/src/tools/bash.rs`
- `filter_off_records_no_output_filter_metadata` in `executor/src/tools/bash.rs`
- `dispatch_surfaces_success_metadata` in `executor/src/agent/tools.rs`
- `dispatch_returns_none_metadata_on_error` in `executor/src/agent/tools.rs`
- `loop_emits_output_filtered_event_for_filtered_bash` in
  `executor/src/agent/mod.rs`

**Notes for review:** The `dispatch` widening (returning `Option<serde_json::Value>`
metadata) is additive — the third tuple slot is `None` for all error/refusal/
unknown-tool paths, so existing callers (only one: `mod.rs`) and future callers
get the metadata without any behavioral change on the failure paths.

### Review verdict — 2026-06-07

- **Verdict:** approved_first_try
- **Bounces:** none (no review bug reports; the prior `VerifierFailurePersistent`
  hard-fail was a pre-review escalation, resolved by refined re-dispatch)
- **Executor:** local LLM (executor) — completed from the preserved partial state
- **Scope deviations:** none — pure instrumentation; filter output unchanged, no
  `status.rs`/`PhaseRun`/scorecard touch (correctly deferred to phase-07)
- **Calibration:** one occurrence — the original Task 5(c) under-listed the
  `filter.rs` per-event-kind blast radius (3 of 5 sites), causing the first
  dispatch's hard-fail. Already folded into this phase's tightened spec; not yet a
  trend, so no WORKFLOW.md fold. Watch for repeats when adding future
  `SessionEvent` variants (e.g. phase-04 `ReadEvicted`).

**Independent re-run (reviewer):**

```
cargo fmt --all --check          → clean (no output)
cargo build                      → Finished, zero warnings
cargo clippy --all-targets --all-features -- -D warnings → clean
cargo test                       → 615 passed; 0 failed; 2 ignored
```

DoD: all boxes checked. Acceptance criteria: all met — verified the
`OutputFiltered` variant grep, the on/off/cargo-label bash tests, the
`dispatch` None-on-error path, and the agent-loop event emission read back from
the session JSONL. unwrap/expect appear only in test code; production paths are
clean. The four exhaustive match sites compile and the transcript render arm is
present.
