# Phase 06: Async `continue_phase` (`run_id` + cancel + skill update)

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress
**Depends on:** phase-05
**Estimated diff:** ~160 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Make the `continue_phase` MCP tool spawn asynchronously through `JobRegistry`
and return `{ run_id }` immediately — polled with `get_run_status` and
stoppable with `stop_phase` / `.rexymcp/stop`, matching `execute_phase` — so
long resumed runs never hit the MCP client's 1800s synchronous tool-call
timeout. Implements related observation #1 of
[issue #18](https://github.com/ryanczak/rexyMCP/issues/18).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Exit criteria and § Notes #1.
- `docs/architecture.md` § Layer 2 (`execute_phase` async `run_id` contract) and § Status, M51 entry.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

In `mcp/src/server.rs`, `execute_phase` spawns its work through
`crate::jobs::spawn_run` with a live `CancelSignal` and
`crate::stop_watcher::watch_stop_sentinel`, returns
`structured_result(&SpawnedRun { run_id })`, and declares
`schema_for_output::<SpawnedRun>()` in `execute_phase_tool()`. By contrast,
`continue_phase` still awaits `continue_phase_inner` synchronously inside
`call_tool`, hardcodes `CancelSignal::never()`, and declares
`schema_for_output::<PhaseResult>()` in `continue_phase_tool()`.
`mcp/src/server_tests.rs` currently asserts the old `PhaseResult` schema in
`continue_phase_tool_declares_phase_result_output_schema`.
`plugin/skills/escalate/SKILL.md` § "Resume" still assumes `continue_phase`
returns a `PhaseResult` directly.

## Spec

1. **Cancellable & testable `continue_phase_inner` in `mcp/src/server.rs`**:
   - Add `cancel: CancelSignal` to `continue_phase_inner`, and factor out
     `pub(crate) async fn continue_phase_inner_with_client(..., test_client: Option<&dyn AiClient>, cancel: CancelSignal) -> Result<ExecutePhaseOutput, String>`
     (mirroring `execute_phase_inner` / `execute_phase_inner_with_client`).
   - Pass `test_client` and `cancel` into `runner::RunPhaseConfig` instead of
     `None` and `CancelSignal::never()`.
   - Update the comment on `RunPhaseConfig.cancel` in `mcp/src/runner.rs` so it
     no longer says `continue_phase` disables cancellation.
2. **Async spawn + stop-sentinel watcher in `mcp/src/server.rs`**:
   - In `ServerHandler::call_tool`'s `"continue_phase"` branch, after
     parameter parsing, `roots::corroborate`, and `progress_callback` setup,
     spawn the run identically to `"execute_phase"`:
     - Allocate `run_id = crate::jobs::new_run_id()` and
       `(cancel_handle, cancel_signal) = CancelSignal::new()`.
     - Build the `work` future calling `continue_phase_inner` with
       `cancel_signal` and mapping `Ok(o) => Ok(o.result)`.
     - Call `crate::jobs::spawn_run(runs.clone(), run_id.clone(), cancel_handle, work)`.
     - Spawn `crate::stop_watcher::watch_stop_sentinel(repo_path.clone(), runs.clone(), run_id.clone(), crate::stop_watcher::STOP_POLL_INTERVAL)`.
     - Return `structured_result(&SpawnedRun { run_id })`.
   - Update `continue_phase_tool()` to declare
     `schema_for_output::<SpawnedRun>()` and describe the async `{ run_id }` /
     `get_run_status` / `stop_phase` contract (mirroring `execute_phase_tool()`).
   - Update `stop_phase`'s `#[rmcp::tool(description = ...)]` to mention both
     `execute_phase` and `continue_phase` runs.
   - **Constraints:**
     - Do not remove `roots::corroborate` or `progress_callback` wiring on
       `continue_phase`.
     - No `.unwrap()` / `.expect()` on production paths (`STANDARDS.md` §2.1).
3. **Skill & README updates**:
   - In `plugin/skills/escalate/SKILL.md` § "Resume — resume from a fresh
     briefing-seeded context", update execution step 2 to match
     `dispatch/SKILL.md` §2's detect-and-adapt async contract:
     - When `{ run_id }` is present: confirm startup with one
       `get_run_status(run_id)` (expect `running`), stop polling and hand off to
       the user (`rexymcp status` / `rexymcp dashboard`), and reap the terminal
       `PhaseResult` when signalled (in `/rexymcp:auto`, reap minimally without
       narration or log-grepping); note it is stoppable via `stop_phase(run_id)`
       or `rexymcp stop` (`.rexymcp/stop`).
     - When `status` is present instead of `run_id` (older blocking binary):
       treat the returned `PhaseResult` directly.
   - In `README.md`, update the `continue_phase` row in the MCP tools table to
     document that it spawns asynchronously and returns `{ run_id }` (polled
     with `get_run_status`, stoppable with `stop_phase`).
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `continue_phase` spawns via `crate::jobs::spawn_run` and
      `crate::stop_watcher::watch_stop_sentinel`, forwards a live `CancelSignal`
      into `RunPhaseConfig`, and returns `{ run_id }`.
- [ ] `continue_phase_tool()` declares `SpawnedRun` output schema (`run_id`
      property present, `status` property absent).
- [ ] `plugin/skills/escalate/SKILL.md` and `README.md` document the async
      `{ run_id }` / `get_run_status` / `stop_phase` flow for `continue_phase`.
- [ ] Every test named in § Test plan passes.
- [ ] Preservation: `continue_phase_returns_error_for_missing_phase_doc`,
      `continue_phase_restores_task_states_from_prior_log`,
      `execute_phase_tool_declares_run_id_output_schema`, and all existing
      tests pass.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `mcp/src/server_tests.rs`:
  - `continue_phase_tool_declares_run_id_output_schema` (replaces
    `continue_phase_tool_declares_phase_result_output_schema`) — asserts
    `output_schema` has `properties.run_id` and does **not** have
    `properties.status`.
  - `continue_phase_spawned_run_completes_via_get_run_status` — spawns
    `continue_phase_inner_with_client` (with a `MockAiClient` returning a clean
    completion and a `prior_log_path` marking task `"1"` `done`) through
    `JobRegistry` + `spawn_run`, polls `get_run_status_inner`, and asserts
    `state == "done"` and `result["status"] == "complete"` (proving the
    resumed task state was restored so the task-coverage gate passes).
  - `continue_phase_spawned_run_honors_stop_phase_cancel` — spawns
    `continue_phase_inner_with_client` through `JobRegistry` + `spawn_run` with
    a `MockAiClient` scripted to execute a tool call, triggers
    `runs.request_stop(&run_id, CancelReason::ClaudeStop)`, polls
    `get_run_status_inner`, and asserts `state == "done"`,
    `result["status"] == "cancelled"`, and
    `result["cancellation"]["reason"] == "claude_stop"`.
  - Update existing `continue_phase_inner` test call sites to pass
    `CancelSignal::never()`.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-06.txt; : > "$A"
for t in continue_phase_tool_declares_run_id_output_schema continue_phase_spawned_run_completes_via_get_run_status continue_phase_spawned_run_honors_stop_phase_cancel continue_phase_returns_error_for_missing_phase_doc continue_phase_restores_task_states_from_prior_log execute_phase_tool_declares_run_id_output_schema; do
  cargo test -p rexymcp "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
grep -n 'get_run_status' plugin/skills/escalate/SKILL.md >> "$A"; echo "escalate_skill_async exit=$?" >> "$A"
```

Expected: every listed test prints `ok` with `exit=0` and a non-zero passed
count, and `escalate_skill_async exit=0`.

## Authorizations

- `mcp/src/server.rs`
- `mcp/src/server_tests.rs`
- `mcp/src/runner.rs`
- `plugin/skills/escalate/SKILL.md`
- `README.md`

No new dependencies.

## Out of scope

- Changing `execute_phase`, `JobRegistry`, or `stop_watcher` behavior.
- Unix process-group cleanup for `bash` and gate commands (Phase 07).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-25 04:20 (progress)

Started phase 06: making `continue_phase` spawn asynchronously through
`JobRegistry` (mirroring `execute_phase`), returning `{ run_id }` immediately.
Flipped phase status and milestone README row to `in-progress`. Next:
cancellable/testable `continue_phase_inner`, async spawn in `call_tool`, then
the skill/README updates.

### Update — 2026-11-25 04:26 (end-to-end verification)

Ran the phase doc's E2E block verbatim; artifact pasted below. Positive control:
each listed test prints `ok` with a non-zero passed count and `exit=0` — a
binary that never spawned the run would fail
`continue_phase_spawned_run_completes_via_get_run_status` (state would not be
`done`) and `continue_phase_spawned_run_honors_stop_phase_cancel` (reason would
not be `claude_stop`), and the skill grep returning `exit=0` proves the async
contract text landed in `escalate/SKILL.md`.

```
target/e2e/m51-phase-06.txt:
test server::tests::continue_phase_tool_declares_run_id_output_schema ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
continue_phase_tool_declares_run_id_output_schema exit=0
test server::tests::continue_phase_spawned_run_completes_via_get_run_status ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
continue_phase_spawned_run_completes_via_get_run_status exit=0
test server::tests::continue_phase_spawned_run_honors_stop_phase_cancel ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 0.01s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
continue_phase_spawned_run_honors_stop_phase_cancel exit=0
test server::tests::continue_phase_returns_error_for_missing_phase_doc ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 6.03s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
continue_phase_returns_error_for_missing_phase_doc exit=0
test server::tests::continue_phase_restores_task_states_from_prior_log ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 12.04s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
continue_phase_restores_task_states_from_prior_log exit=0
test server::tests::execute_phase_tool_declares_run_id_output_schema ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 711 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
execute_phase_tool_declares_run_id_output_schema exit=0
210:     startup with a **single** `get_run_status(run_id)` (expect `running`),
214:     your next turn) with one more `get_run_status(run_id)` — in an
escalate_skill_async exit=0
```

Gates: `cargo fmt --all` clean; `cargo build` zero warnings;
`cargo clippy --all-targets --all-features -- -D warnings` clean;
`cargo test` 1142 passed, 0 failed, 2 ignored (pre-existing live-LLM ignores).
