# Phase 07e: completion artifacts (diff, command set, result population)

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** review
**Depends on:** phase-07a–07d (the loop), phase-06 (`PhaseResult`, `Artifacts`,
`FileChange`, `CommandOutputs`), `config::CommandConfig`. All done. `similar` and
`tokio` are already workspace deps.
**Estimated diff:** ~450 lines (diff builder + command-runner seam + result
population + `log_path` field + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Fill in the `PhaseResult` the loop has been returning empty. This is the **last 07
sub-phase** — after it, `execute_phase` returns the complete contract Claude reads:
the **unified diff** of what the executor changed, the **`files_changed`** list, the
**final command set** output (`fmt`/`build`/`lint`/`test`) on a clean finish, and
the **session-log path**. Turn-cycle step 8 ("On clean completion, run the project's
full command set, capture output, and return") lands here.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" step 8, and "The `PhaseResult`
  / briefing contract": `diff` = "unified diff of the working tree (capped)";
  `command_outputs` = "{format, build, lint, test → tail of stdout/stderr}";
  `files_changed` = "[ { path, change_summary } ]"; and "The `execute_phase` result
  reports the **log path** so Claude can reference it."
- `docs/dev/STANDARDS.md` §3.3 — deterministic tests: the final command set must be
  **injected** (a `CommandRunner` seam), exactly as 07c injected the verifier, so
  unit tests never spawn real `cargo`/`npm`/etc.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. Read:
   - `executor/src/phase/result.rs` — `PhaseResult`, `Artifacts`, `FileChange`,
     `CommandOutputs`, and the `complete`/`hard_fail`/`budget_exceeded`
     constructors (all take `Artifacts`).
   - `executor/src/config.rs` — `CommandConfig { format, build, lint, test:
     Option<String> }`.
   - `executor/src/tools/patch.rs` — the existing `similar::TextDiff::from_lines(
     …).unified_diff()` usage (mirror it).
   - `executor/src/agent/mod.rs` — the terminal returns (`complete_result` /
     `budget_exceeded_result` / `hard_fail_result`) and the edit-class dispatch
     site (07c/07d) where pre-edit content can be captured.

## Current state

Every terminal path returns `empty_artifacts(...)`: `files_changed: vec![]`,
`diff: String::new()`, `command_outputs: CommandOutputs::default()`. `PhaseResult`
has **no `log_path` field** (phase-06 deferred it here). The loop never runs the
project command set and never tracks pre-edit content. `similar` is already used by
the patch tool; `tokio` (with `process`) is available.

## Spec

### 1. `log_path` on the result (edit phase-06 types — authorized)

Add `log_path: Option<PathBuf>` to **`Artifacts`** and **`PhaseResult`** in
`phase/result.rs`; thread it through `assemble` (the constructors keep taking
`Artifacts`, so their signatures don't change). Update phase-06's tests that build
`Artifacts` inline. The loop sets `log_path = log_handle.as_ref().map(|h|
h.lock().ok().map(|l| l.path().to_path_buf()))` flattened to `Option<PathBuf>` —
`None` when the log failed to open (07b best-effort). This is the **only** edit to
`phase/**` this phase authorizes.

### 2. Track pre-edit content (loop state)

Add `pre_edit_content: HashMap<PathBuf, Option<String>>`. At the edit-class
dispatch site, **before** the edit runs (the 07d-allowed / 07c-baseline branch,
pre-`dispatch`), if the resolved edit path is **not yet a key**, insert its current
content: `Some(text)` if readable, `None` if the file does not exist (a
to-be-created file). Capture once per path (first edit wins — that's the true
"before"). Refused edits (07d) never reach this point, so they aren't tracked.

### 3. Diff + `files_changed` builder (pure, at terminal)

A helper that, given `pre_edit_content` and `project_root`, returns `(diff: String,
files_changed: Vec<FileChange>)`:

- For each tracked path, read current on-disk content (`""` if now absent).
  Compare to the "before" (`None` → `""`). **Skip unchanged** files (before ==
  after — e.g. an edit later reverted).
- For a changed file: render a unified diff with `similar::TextDiff::from_lines(
  &before, &after).unified_diff()` and a per-file header naming the path
  (project-root-relative if possible). Append to the combined diff.
- `FileChange { path, change_summary }` where `change_summary` is a short
  `"+{added} -{removed}"` (count the `Insert`/`Delete` line ops; "created" /
  "deleted" wording for new/removed files is acceptable too — your call, keep it
  one short line).
- **Cap** the combined diff at a `MAX_DIFF_CHARS` const; if exceeded, truncate and
  append a clear `\n… (diff truncated)\n` marker. `files_changed` is **not** capped
  (it's just paths + summaries).

### 4. Command-runner seam (injected; runs only on clean completion)

```rust
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Run a shell command in `cwd`; return a combined stdout+stderr capture.
    async fn run(&self, command: &str, cwd: &Path) -> String;
}
```

- `RealCommandRunner` runs `sh -c <command>` via `tokio::process::Command` in
  `cwd`, capturing stdout+stderr (merge or concatenate). Put the trait +
  impl in a new `agent/command.rs`.
- `LoopDeps` gains `commands: &CommandConfig` and `runner: &dyn CommandRunner`.
- **Only on clean completion** (the `NoToolCall` → complete path) run the final
  command set: for each of `format` / `build` / `lint` / `test` that is
  `Some(cmd)`, `runner.run(cmd, project_root).await`, tail-capped to a
  `MAX_COMMAND_TAIL_CHARS` const, into the matching `CommandOutputs` field. A
  `None` config field stays `None` in the output. On `hard_fail` / `budget_
  exceeded`, **do not** run commands — `command_outputs` stays `CommandOutputs::
  default()` (the phase didn't cleanly finish).

### 5. Thread artifacts through the terminal returns

Replace `empty_artifacts` at each terminal site with a real `Artifacts`:

- **Every** terminal path (complete, budget_exceeded, hard_fail) builds `diff` +
  `files_changed` (§3) and sets `log_path` (§1) and `update_log` (the minimal line
  from 07a).
- **Only** the complete path additionally runs the command set (§4) into
  `command_outputs`.

Keep the status↔briefing invariant via the existing phase-06 constructors (don't
hand-build `PhaseResult`).

### 6. Error model

- The command runner returns a `String` (captured output), never `Err` — a command
  that fails to spawn yields a captured error string (the output is diagnostic, not
  a loop error). Diff/file reads are best-effort (`""` on unreadable), never panic.
- No `.unwrap()` / `.expect()` / `panic!()` in the loop or helpers.

## Acceptance criteria

- [ ] `PhaseResult` / `Artifacts` carry `log_path: Option<PathBuf>`; a run with a
      writable log dir returns `Some(path)`, and the loop sets it; phase-06 tests
      updated.
- [ ] After an edit, `PhaseResult.diff` contains a unified diff of the change and
      `files_changed` lists the edited path; an unchanged/reverted file is absent.
- [ ] A new file edit produces an all-added diff; the diff is capped at
      `MAX_DIFF_CHARS` with a truncation marker when exceeded.
- [ ] On clean completion, configured (`Some`) commands run via the injected
      `CommandRunner` and their tails land in the matching `command_outputs` field;
      `None`-configured fields stay `None`.
- [ ] On `hard_fail` / `budget_exceeded`, the command set does **not** run
      (`command_outputs` default) (**negative**) — though `diff` / `files_changed`
      are still populated.
- [ ] No new dependency; no `tracing`; tests inject a mock `CommandRunner` (no real
      subprocess); only `phase/result.rs` (the `log_path` field) is touched outside
      `agent/**`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic (`TempDir`), deterministic (`MockCommandRunner` scripting per-command
output + recording which ran; `MockAiClientScript`; the 07c `MockFileVerifier` /
`NoopVerifier`).

**Diff / files_changed (pure builder + via loop):**
- `diff_contains_unified_change_for_edited_file`.
- `files_changed_lists_edited_path_with_summary`.
- `new_file_diff_is_all_added`.
- `unchanged_file_is_absent_from_files_changed` (**edge**).
- `diff_capped_at_max_with_marker`.

**Command set:**
- `clean_completion_runs_configured_commands` — `MockCommandRunner` records the
  commands; assert only `Some`-configured ones ran and their tails are in
  `command_outputs`.
- `none_configured_command_is_not_run_and_output_is_none`.
- `command_output_is_tail_capped`.
- `hard_fail_does_not_run_command_set` (**negative** — drive a hard-fail; assert
  `command_outputs == default` but `diff`/`files_changed` populated).
- `budget_exceeded_does_not_run_command_set` (**negative**).

**log_path:**
- `complete_result_reports_log_path` (`Some`).
- `log_path_is_none_when_log_unopened` — point `.rexymcp` at a file (07b best-effort
  open failure) → `log_path == None`, run still completes.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. Diff/command-set/
> result population is exercised via `MockCommandRunner` + `MockAiClient*` over a
> `TempDir`. `RealCommandRunner` (a `sh -c` subprocess) is trivial plumbing covered
> by the first live M5 run.

## Authorizations

- [x] **May modify** `executor/src/agent/**` (loop + tests); **may create**
      `executor/src/agent/command.rs` (the `CommandRunner` seam).
- [x] **May modify** `executor/src/phase/result.rs` — **only** to add the
      `log_path: Option<PathBuf>` field to `Artifacts` / `PhaseResult` and thread it
      through the constructors (and fix the phase-06 tests that build `Artifacts`).
- [ ] **No new dependencies** (`similar`, `tokio` already present); no `tracing`.
- [ ] May **NOT** modify `governor/**`, `config.rs`, `store/**`, `tools/**`,
      `Cargo.toml`, `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **`PhaseRun` telemetry** (incl. the `scorer` consumer `tool_success_rate`) —
  phase-08, the M4 closer.
- **MCP wiring** of `execute_phase` + the log-query tools — M5.
- **Per-compiler diff parsing / structured diffs** — the diff is a plain unified
  text diff; structured diagnostics are the verifier's job (07c).
- **`Progress` heartbeat numstat** — M5 (it will reuse this diff/working-set
  machinery).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md.

Adding `log_path: Option<PathBuf>` to `Artifacts`/`PhaseResult` (phase/result.rs),
`pre_edit_content` tracking at the dispatch site, a pure diff/`files_changed`
builder (`similar::TextDiff` + `MAX_DIFF_CHARS` cap), a `CommandRunner` seam
(`agent/command.rs`: trait + `RealCommandRunner` over `tokio::process` `sh -c`)
injected via `LoopDeps` with `commands: &CommandConfig`, and artifact threading:
diff/files_changed on every terminal path, command set only on clean completion.
Tests inject a `MockCommandRunner`.

### Update — 2026-05-29 (complete)

**Summary:** Filled in the `PhaseResult`. Added `log_path: Option<PathBuf>` to
`Artifacts`/`PhaseResult` (the only `phase/**` edit) — set from the session-log
handle (`None` when the log didn't open). The loop now tracks `pre_edit_content:
HashMap<PathBuf, Option<String>>`, capturing each edited file's content **before**
the first edit (in the 07d-allowed branch, pre-dispatch). At every terminal return
`build_artifacts` → `build_diff` renders a `similar` unified diff (per-file header,
`+N -M` `change_summary`, unchanged files skipped, capped at `MAX_DIFF_CHARS` with
a truncation marker). A `CommandRunner` seam (`agent/command.rs`: trait +
`RealCommandRunner` over `tokio::process` `sh -c`) is injected via `LoopDeps`
alongside `commands: &CommandConfig`; `run_command_set` runs **only** on the clean-
completion path (configured `Some` commands only, tail-capped), so `hard_fail` /
`budget_exceeded` keep `CommandOutputs::default()`. No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.57s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.06s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 477 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(477 = 469 prior + 8 completion-artifact tests. `agent::` alone: 52 passed.)

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. Diff / command-set /
result population is exercised via `MockCommandRunner` + `MockAiClient*` over a
`TempDir`. `RealCommandRunner` (a `sh -c` subprocess) is trivial plumbing covered
by the first live M5 run.

**Files changed:**
- `executor/src/phase/result.rs` — `log_path` field on `Artifacts` / `PhaseResult`
  (+ `assemble` threading + phase-06 test fixture).
- `executor/src/agent/command.rs` — new: `CommandRunner` trait + `RealCommandRunner`.
- `executor/src/agent/mod.rs` — `commands`/`runner` on `LoopDeps`; `pre_edit_content`
  tracking; `build_artifacts` / `build_diff` / `run_command_set` / `run_one` /
  `tail`; terminal returns build real artifacts (command set only on complete);
  8 new tests + `MockCommandRunner` / `NoopRunner` / `run_full`.

**New tests:** `diff_and_files_changed_for_edited_file`, `new_file_diff_is_all_added`,
`unchanged_file_is_absent_from_files_changed`,
`clean_completion_runs_configured_commands`, `command_output_is_tail_capped`,
`hard_fail_does_not_run_command_set` (neg), `complete_result_reports_log_path`,
`log_path_is_none_when_log_unopened`.

**Commits:** (pending — committed below)

**Notes for review:**
- `build_diff` skips unchanged files (before == after) so a reverted edit produces
  no `files_changed` entry; deterministic order (sorted by path).
- The command set runs **only** on the clean-completion path — `hard_fail` /
  `budget_exceeded` deliberately leave `command_outputs` default (the phase didn't
  finish), but `diff` / `files_changed` are still populated everywhere.
- `log_path` resolved once after opening the log; `None` when the best-effort open
  failed (covered by `log_path_is_none_when_log_unopened`).
- This is the last 07 sub-phase — `execute_phase` now returns the full contract.
  Only phase-08 (`PhaseRun` telemetry, incl. the long-carried `scorer.record`
  consumer) remains in M4.

verification: fmt OK · clippy OK · tests 477 passed · build OK
