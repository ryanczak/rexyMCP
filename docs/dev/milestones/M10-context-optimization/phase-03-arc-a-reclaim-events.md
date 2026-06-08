# Phase 03: Arc A reclaim events (OutputFiltered)

**Milestone:** M10 — Context optimization
**Status:** todo
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

**(c)** `mcp/src/dashboard/filter.rs` — add a field to `ActivityFilter` (after
`compaction`), default it `true`, and add the `allows` arm:

```rust
    pub(crate) output_filtered: bool,    // struct field
    // in Default: output_filtered: true,
    SessionEvent::OutputFiltered { .. } => self.output_filtered,   // allows() arm
```

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
