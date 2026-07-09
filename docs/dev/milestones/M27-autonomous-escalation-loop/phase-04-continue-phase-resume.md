# Phase 04: `continue_phase` — briefing-seeded resume

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** todo
**Depends on:** phase-03b (server-authored finalize is live; a failed run leaves the doc at `in-progress`)
**Estimated diff:** ~430 lines
**Tags:** language=rust, kind=feature, size=l

## Goal

Add the third escalation lever: a `continue_phase` MCP tool that resumes a
`hard_fail`/`budget_exceeded` phase **briefing-seeded** — a fresh executor
context built from the phase doc + architect guidance + the current on-disk diff,
with task states restored from the prior session log so the resumed run does not
redo the tasks already done. Un-stub the escalate skill's resume lever with
criteria for choosing it over re-dispatch, and amend the executor contract so the
executor knows how to behave when it is resuming.

This gets "don't redo the 90% that's done" **without** replaying the transcript
rot that the re-dispatch lever exists to escape (see `architecture.md` §
"Escalation = Claude Code itself"; codebase-review § 3.1).

## Architecture references

Read before starting:

- `docs/architecture.md` § "Escalation = Claude Code itself" — resume is a
  committed lever, briefing-seeded (fresh context, not transcript rehydration).
- `docs/dev/codebase-review-2026-07-07.md` § 3.1 — the design rationale.
- The M27 milestone [README](README.md) § Design and § Exit criteria (the
  `continue_phase` bullet).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

- **The `execute_phase` MCP tool** is special-cased in `mcp/src/server.rs`
  `call_tool` (`server.rs:477–535`) — it is *not* routed through the
  `#[rmcp::tool]` macro router, because it needs roots corroboration + the
  progress token. It is also listed manually in `list_tools` (`server.rs:549`)
  and `get_tool` (`server.rs:564`). Its inner logic is
  `execute_phase_inner_with_client` (`server.rs:85–126`): load config, read
  `STANDARDS.md`, resolve `project_id`, call `runner::run_phase(&RunPhaseConfig{…})`,
  cap, serialize.
- **`runner::run_phase`** (`mcp/src/runner.rs:329`) builds real seams and delegates
  to the hermetic `run_phase_with` (`runner.rs:224`), which reads the phase doc,
  calls `parse_phase_doc`, seeds the registry, builds `PhaseInput`
  (`runner.rs:255`), runs `agent::execute_phase`, then `finalize_complete`.
- **`agent::execute_phase`** (`executor/src/agent/mod.rs:140`) seeds tasks from
  the phase doc at `mod.rs:141` (`seed_from_spec` — always `Pending`), builds the
  `task_states` shadow map at `mod.rs:150`, and renders `prompt::task_section(&seeded)`
  into the system prompt at `mod.rs:152–157`. It emits one seed `TaskUpdate` per
  task at `mod.rs:232–245`.
- **Session logs** are JSONL under `<repo>/.rexymcp/sessions/`; each `TaskUpdate`
  record carries `{id, title, state}` (`store/sessions/event.rs:124`), last-write-wins
  per id. Read them with `store::sessions::jsonl::read_session_log(path) ->
  io::Result<Vec<SessionRecord>>` (`jsonl.rs:54`). `PhaseResult.log_path` is the
  path a prior run returned.
- **The executor contract** is `executor/templates/executor_contract.md`, embedded
  via `contract.rs:5`. Its "Phase lifecycle" section (§ starting at line 59) has no
  resume note today.
- **The escalate skill** (`plugin/skills/escalate/SKILL.md`) documents resume as
  "not yet implemented" (§ 2 "Resume — not yet implemented", § 3 "Resume", and the
  decision-summary table).

## Spec

Build bottom-up (executor lib → runner threading → resume module → server tool →
contract → skill), running `cargo build` after each of tasks 1, 2, and 3 — each
adds a field to a struct with many literal sites and the compiler will point you
at every one.

### 1. Additive `PhaseInput` field + seed-site restore

In `executor/src/agent/mod.rs`, add one field to `PhaseInput` (after `tier`,
`mod.rs:83`):

```rust
    /// Task states restored from a prior run's session log, for a resumed phase
    /// (`continue_phase`). `None` on a normal run → seed all `Pending`. When
    /// `Some`, each seeded task whose id is present takes the restored state, so
    /// the task-coverage gate does not re-demand work already done.
    pub resumed_task_states: Option<std::collections::HashMap<String, crate::store::sessions::event::TaskState>>,
```

At the seed site (`mod.rs:141`), after `seed_from_spec` produces the `Vec<Task>`,
apply the restored states **before** the `task_states` map and the seed
`TaskUpdate` events are built — so the prompt's task section, the shadow map, and
the seed log events all reflect the restored state. Make `seeded` mutable and,
when `input.resumed_task_states` is `Some`, overwrite each matching task's
`.state`. Tasks absent from the map keep `Pending`; ids in the map with no seeded
task are ignored.

This is the **only** executor-crate behavior change. Every `PhaseInput { … }`
literal must gain `resumed_task_states: None` — the complete list (grep-verified),
add `None` to each:

- `mcp/src/runner.rs:255` (production — task 2 sets this to the real value, not `None`)
- `executor/src/agent/tests.rs`: lines 29, 282, 3974, 4040, 4074, 4105, 4186, 4231, 4268, 4312

### 2. Thread a resume context through the runner

Define a carrier (in `mcp/src/resume.rs`, task 3 — but the type is used here, so
create the module first or forward-declare):

```rust
pub struct ResumeContext {
    /// Markdown appended to the phase doc in the system prompt.
    pub preamble: String,
    /// Restored task states (id → state) for `PhaseInput.resumed_task_states`.
    pub task_states: std::collections::HashMap<String, rexymcp_executor::store::sessions::event::TaskState>,
}
```

Add `pub resume: Option<crate::resume::ResumeContext>` to **both**
`RunPhaseConfig` (`runner.rs:314`) and `AssemblyInput` (`runner.rs:102`). In
`run_phase` (`runner.rs:329`) pass `resume: inp.resume` … but `resume` is not
`Copy`; take it by move — `RunPhaseConfig` is consumed by value through `&`, so
change `run_phase` to move the field out, or make `AssemblyInput.resume` a
`&Option<ResumeContext>` borrow of `inp.resume`. Pick whichever keeps the borrow
checker happy without cloning the diff string twice; a borrow is cleanest.

In `run_phase_with` (`runner.rs:224`):
- Parse `fields` from the **original** phase doc (before appending the preamble)
  — the preamble must not perturb `## Goal` / `## Acceptance criteria` extraction.
- When `resume` is `Some`, append `"\n\n"` + `preamble` to the `phase_doc` string
  that goes into `PhaseInput` (so it reaches the system prompt and `seed_from_spec`).
- Set `PhaseInput.resumed_task_states` from `resume.task_states` (clone), else `None`.

Every `RunPhaseConfig { … }` and `AssemblyInput { … }` literal gains `resume: None`
except the resume path. Grep-verified lists:

- `RunPhaseConfig`: `mcp/src/server.rs:106` (execute_phase → `None`),
  `mcp/src/main.rs:372` (run-phase CLI → `None`). Task 4's `continue_phase` site
  sets the real value.
- `AssemblyInput`: `mcp/src/runner.rs` lines 377 (production), 581, 636, 691, 754,
  807, 878, 1004 (tests → `None`).

### 3. The resume-assembly module `mcp/src/resume.rs`

New module (add `mod resume;` to `mcp/src/main.rs`). Three pure-ish helpers plus a
top assembler. Mirror the `git`-via-`CommandRunner` pattern from
`mcp/src/finalize.rs:186` (`git_head`).

- `restore_task_states(prior_log_path: Option<&Path>) -> HashMap<String, TaskState>`
  — `None` or unreadable → empty map. Else `read_session_log`, fold every
  `SessionEvent::TaskUpdate { id, state, .. }` into the map (later records win —
  last-write-wins per id). Ignore all other event kinds.
- `current_diff(runner, repo_root) -> String` — run `git --no-pager diff HEAD` via
  the runner; on success return stdout, char-safe-truncated to a
  `const MAX_RESUME_DIFF_CHARS: usize = 50_000;` (mirror the executor's
  `MAX_DIFF_CHARS`); on failure return an empty string. This captures the
  **uncommitted** working-tree work of the failed run (the common case — a failed
  run rarely reaches its commit step). Committed partial work is visible to the
  resumed executor in the repo itself; it need not be in the diff.
- `render_preamble(guidance: &str, diff: &str, task_states: &HashMap<…>) -> String`
  — the `# Resume context` markdown block. **Seed-safety is load-bearing:** this
  text is fed to `seed_from_spec`, so it must NOT contain any pattern the seeder
  parses as a task — no `N. **bold**` list items and no `### N.` / `### Task N`
  headings. Use `##` sub-headings and render task progress as a plain bullet list
  (`- <id> (<state>): <title>` is fine — no leading number). Suggested shape:

  ```markdown
  # Resume context

  You are RESUMING this phase. A prior executor run did not finish; you are
  continuing its work, not starting over. Build on what is already on disk. Do
  not redo tasks already marked done.

  ## Architect guidance

  <guidance>

  ## Work already on disk (uncommitted diff vs HEAD)

  ```
  <diff, or "(no uncommitted changes)">
  ```

  ## Prior task progress

  <bullet list of restored task states, or "(no prior task state recorded)">
  ```

- `build_resume_context(guidance, prior_log_path, repo_root, runner) -> ResumeContext`
  — glue: restore states, compute diff, render preamble, return the struct.

### 4. The `continue_phase` MCP tool

In `mcp/src/server.rs`:
- Add `ContinuePhaseParams { phase_doc_path, repo_path, guidance: String,
  prior_log_path: Option<String>, model: Option<String> }` (derive the same traits
  as `ExecutePhaseParams`, `server.rs:27`).
- Add `continue_phase_inner_with_client` mirroring `execute_phase_inner_with_client`
  (`server.rs:85`): load config, read `STANDARDS.md`, resolve `project_id`, then
  `resume::build_resume_context(&params.guidance, params.prior_log_path.as_deref().map(Path::new),
  &repo_path, &RealCommandRunner)`, and call `run_phase` with `resume: Some(ctx)`.
  Cap + serialize identically.
- Wire it as a **second special-case branch** in `call_tool` alongside
  `execute_phase` (`server.rs:477`) — including the same roots corroboration and
  progress-token handling. Add it to `list_tools` (`server.rs:549`, insert like
  `execute_phase`) and `get_tool` (`server.rs:564`). Give it a tool description
  that states: resumes a non-complete phase from a fresh briefing-seeded context;
  `guidance` is the architect's distilled direction; `prior_log_path` (optional) is
  the failed run's `PhaseResult.log_path`, used to restore task states.

The resumed run flows through `run_phase` → `execute_phase` → `emit_phase_run`
normally, so it telemeters as an ordinary `PhaseRun`, and `finalize_complete`
flips the still-`in-progress` doc to `review` if it completes. No special-casing
needed for either.

### 5. Amend the executor contract (authorized — see Authorizations)

Add a short **"Resuming a phase"** note to `executor/templates/executor_contract.md`,
at the end of the "Phase lifecycle" section (after line 106, before the
"Completion checklist"). This is the load-bearing contract change; use this text
**verbatim**:

```markdown
### Resuming a phase

If the phase prompt contains a **`# Resume context`** block, you are **resuming**
a prior run that did not finish — not starting fresh. The block carries the
architect's guidance, the work already on disk (a diff), and the prior task
progress. Build on that work: read the current state of the files the diff
touched before editing them, keep tasks already marked `done`, and pick up where
the prior run stopped. The Update Log's prior entries stay as they are — append a
new started entry naming yourself, then continue. Everything else in this
lifecycle is unchanged: you still own the code and the start-of-phase status, and
the server still authors the completion tail.
```

### 6. Un-stub the escalate skill's resume lever

In `plugin/skills/escalate/SKILL.md`, replace the "not yet implemented" resume
content with a real lever. Behavioral pins (write the prose yourself):
- **§ 1 description line** (frontmatter `description`, top of file, and the "Three
  levers" intro at § 2): resume is now available, no longer "(future)".
- **§ 2 "Resume" subsection:** describe when to choose it — the failure reads as
  "we were most of the way done and hit one specific wall" (a late `budget_exceeded`,
  or a single diagnostic the executor couldn't clear) where the completed work is
  worth preserving. Contrast with re-dispatch (choose re-dispatch when the *spec*
  was the problem — the resumed context would carry the same gap forward) and
  takeover (choose takeover when the executor genuinely can't reach this work).
- **Decision-summary table:** add a "most of the phase done, hit one wall → Resume"
  row.
- **§ 3 "Resume" execution steps:** call `continue_phase` with `phase_doc_path`,
  `repo_path`, a `guidance` string distilled from the briefing (what to fix, what
  is already done, what to avoid re-doing), and `prior_log_path` = the failed
  `PhaseResult.log_path`. Then treat the returned `PhaseResult` like any dispatch
  result (review on complete, escalate again on failure).

Keep it consistent with the existing skill's voice; do not restructure sections
that don't mention resume.

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings; `cargo clippy` and
      `cargo fmt --all --check` pass; `cargo test` passes (existing + new).
- [ ] A normal `execute_phase` run is byte-identical in behavior:
      `PhaseInput.resumed_task_states` is `None`, `resume` is `None`, no preamble
      appended, all pre-existing tests pass unmodified except the mechanical
      `resumed_task_states: None` / `resume: None` literal additions.
- [ ] `restore_task_states` folds a session log's `TaskUpdate` records
      last-write-wins and returns an empty map for `None`/missing/unreadable.
- [ ] The rendered `# Resume context` preamble seeds **zero** tasks via
      `seed_from_spec` (the seed-safety negative).
- [ ] `continue_phase` appears in `tools/list`, corroborates `repo_path` like
      `execute_phase`, and a resumed run restores the prior task states into the
      seeded list.
- [ ] `executor_contract.md` contains the verbatim "Resuming a phase" block and
      `assemble_executor_contract` still substitutes command placeholders cleanly.
- [ ] The escalate skill no longer says resume is "not yet implemented" and
      documents the `continue_phase` call.

## Test plan

- `restore_task_states_folds_last_write_wins` in `resume.rs` — two `TaskUpdate`s
  for the same id; the later state wins.
- `restore_task_states_empty_for_missing_log` — `None` and a nonexistent path both
  yield an empty map.
- `resume_preamble_seeds_no_tasks` — `seed_from_spec(render_preamble(…with a
  guidance string containing "1. do a thing" and a bullet task list…))` returns an
  empty `Vec` (proves the preamble can't inject phantom tasks). Put this where
  `seed_from_spec` is reachable (a `resume.rs` test importing the executor seeder,
  or an `executor` integration test — your call).
- `restored_states_override_seeded_pending` — an `execute_phase` (or `run_phase_with`)
  integration test with `resumed_task_states` marking task "1" `Done`: assert the
  seeded task 1 renders/records as `done`, task 2 stays `pending`. Use
  `MockAiClient` + the `NoopVerifier`/`NoopRunner` already in `runner.rs` tests.
- `continue_phase` server test mirroring the `execute_phase` server tests
  (`server_tests.rs`): a resumed run over a `TempDir` repo with a prior session log
  returns `Complete` and the doc ends at `review`.
- Contract test: `assemble_executor_contract` output contains "Resuming a phase".

## End-to-end verification

Run the real `continue_phase` path against a `TempDir` repo (or the live MCP tool
if convenient): a phase doc left at `in-progress` with a prior session log
containing a `done` `TaskUpdate` for task 1, resumed with a one-line guidance,
completes and finalizes to `review` without the task-coverage gate re-demanding
task 1. Quote the resumed `PhaseResult` status + the doc's `**Status:**` line in
the completion Update Log.

## Authorizations

- [x] May edit `executor/templates/executor_contract.md` — the "Resuming a phase"
      block in Task 5 (verbatim text supplied).
- [x] May add a new module `mcp/src/resume.rs` and register it in `mcp/src/main.rs`.
- [x] May edit `plugin/skills/escalate/SKILL.md` — Task 6.

No new crate dependencies. No `Cargo.toml` / `architecture.md` / `STANDARDS.md` /
`WORKFLOW.md` edit.

## Out of scope

- **No journaling of the resume as an `ArchitectActivity`.** The loop journal
  (phase-02) is produced by the skill layer / `rexymcp journal`; the loop skill
  (phase-06) records the `assist`. `continue_phase` the tool does not self-journal.
- **No transcript rehydration.** Do not replay the prior session's messages into
  the new context — briefing-seeded means fresh context + guidance + diff + task
  states only. That is the whole point (avoids the rot re-dispatch escapes).
- **No auto-discovery of the prior log.** `prior_log_path` is passed explicitly by
  the architect; do not scan `.rexymcp/sessions/` for "the latest" log.
- **No change to `finalize_complete` or the completion path.** A resumed run that
  completes finalizes through the existing dormant-safe path unchanged.
- Do not touch `execute_phase`'s behavior beyond adding `resume: None` to its
  `RunPhaseConfig` literal.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
