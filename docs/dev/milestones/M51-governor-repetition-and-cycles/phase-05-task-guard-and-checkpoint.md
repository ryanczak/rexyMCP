# Phase 05: Per-task no-edit guard & optional soft checkpoint

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** review
**Depends on:** phase-04
**Estimated diff:** ~340 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Bound edit-free turns while a tracked task stays `active` (`task_no_edit_threshold`
→ `HardFailSignal::TaskNoEditStall`), and add an opt-in resumable soft turn
checkpoint (`[budget] checkpoint_turns`, default `0`) that returns
`PhaseStatus::BudgetExceeded` with a briefing recommending `continue_phase`
while preserving partial work on disk. Implements Asks 2 and 3 of
[issue #18](https://github.com/ryanczak/rexyMCP/issues/18).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Exit criteria and § Notes #3.
- `docs/architecture.md` § Status, M51 entry.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/src/config.rs` defines `BudgetConfig` (with `wall_clock_secs: u64`
defaulting to `0`), `GovernorConfig`, and `ModelOverride` (resolved in
`Config::resolve_for_model`). `mcp/src/init.rs` and
`mcp/tests/readme_config_reference.rs` guard-test every field of
`GovernorConfig` and `ModelOverride`; `mcp/src/runner.rs` wires `BudgetConfig`
fields into `LoopDeps` and constructs `ModelOverride` test literals.
`executor/src/governor/hard_fail.rs` defines `HardFailSignal` and
`check_read_only_stall`. `executor/src/agent/mod.rs` maintains `task_states`
from `tasks::seed_from_spec` + `resumed_task_states` + `update_task` metadata,
evaluates Step 7 hard-fail signals, and handles Step 9 `max_turns` via
`outcome::budget_exceeded_result` + `outcome::turns_line`.

## Spec

1. **Config knobs (`task_no_edit_threshold`, `checkpoint_turns`)** — in
   `executor/src/config.rs`:
   - Add `pub task_no_edit_threshold: usize` to `GovernorConfig` (default `40`;
     `0` disables and is allowed at load, unlike `read_only_stall_threshold`).
   - Add `pub task_no_edit_threshold: Option<usize>` to `ModelOverride`, wire
     it in `Config::resolve_for_model`, and update the explicit `ModelOverride`
     test literals in `mcp/src/runner.rs`.
   - Add `#[serde(default)] pub checkpoint_turns: u32` to `BudgetConfig`
     (default `0` = disabled).
   - Document `checkpoint_turns = 0` under `[budget]` and
     `task_no_edit_threshold = 40` under `[governor]` and `[models]` in
     `mcp/src/init.rs` (`generate_config`) and `README.md`.
2. **Per-task no-edit signal in `executor/src/governor/hard_fail.rs`**:
   - Add `HardFailSignal::TaskNoEditStall { task_id: String, consecutive_turns: u32 }`
     with `describe()` formatting
     `"task {task_id} ran {consecutive_turns} consecutive turns with no file edit (task no-edit stall)"`.
   - Export `pub fn check_task_no_edit_stall(active_task_id: Option<&str>, no_edit_turns: usize, threshold: usize) -> Option<HardFailSignal>`:
     returns `None` when `threshold == 0`, `active_task_id` is `None`, or
     `no_edit_turns < threshold`; otherwise returns
     `Some(HardFailSignal::TaskNoEditStall { task_id: id.to_string(), consecutive_turns: no_edit_turns as u32 })`.
3. **Per-task tracking in `executor/src/agent/mod.rs`**:
   - Track `active_task_id: Option<String>` initialized to the first `Active`
     task in `seeded` (so resumed runs with an active task are bounded too),
     and `task_no_edit_turns: usize = 0`.
   - When the `task_update` metadata block runs on a turn, track
     `task_changed: bool`:
     - On `TaskState::Active`: if `active_task_id.as_deref() != Some(id)`, set
       `active_task_id = Some(id.to_string())`, `task_no_edit_turns = 0`, and
       `task_changed = true`. A redundant `update_task(id, "active")` on the
       **already-active** task must **not** reset `task_no_edit_turns`.
     - On `TaskState::Done` or `TaskState::Pending`: if
       `active_task_id.as_deref() == Some(id)`, replace `active_task_id` with
       any other task still `Active` in `seeded` (consulting `task_states`) or
       `None`, reset `task_no_edit_turns = 0`, and set `task_changed = true`.
   - After the `task_update` block (using the turn's `made_file_progress`):
     - If `made_file_progress` (`mutates_files` or `changed_tree`): reset
       `task_no_edit_turns = 0`.
     - Else if `active_task_id.is_some()` and `!task_changed`: increment
       `task_no_edit_turns += 1`.
     - Else if `active_task_id.is_none()`: reset `task_no_edit_turns = 0`.
   - Wire `check_task_no_edit_stall(active_task_id.as_deref(), task_no_edit_turns, deps.governor.task_no_edit_threshold)`
     into Step 7's `hard_fail_signal` chain.
   - **Gotcha:** Note the relative ordering in `execute_phase`: `made_file_progress`
     is computed before the `task_update` metadata block, so updating
     `task_no_edit_turns` **after** the `task_update` block ensures a freshly
     activated task starts at `0` on its activation turn rather than `1`.
4. **Opt-in soft checkpoint (`checkpoint_turns`) in `executor/src/agent/mod.rs`, `executor/src/agent/outcome.rs`, and `mcp/src/runner.rs`**:
   - Add `pub checkpoint_turns: usize` to `LoopDeps` (wired from
     `inp.cfg.budget.checkpoint_turns as usize` in `mcp/src/runner.rs`; `0` in
     existing `LoopDeps` test literals in `executor/src/agent/tests.rs`).
   - Add `pub(super) fn checkpoint_line(turns: usize, checkpoint: usize, max_turns: usize) -> String`
     in `executor/src/agent/outcome.rs` returning
     `format!("soft checkpoint reached at turn {turns} of {max_turns} (checkpoint_turns = {checkpoint}); review partial work and resume with continue_phase")`.
   - In Step 9 of `executor/src/agent/mod.rs`: keep `if turns >= deps.max_turns`
     first (hard cap wins when `checkpoint_turns >= max_turns`); add an
     `else if deps.checkpoint_turns > 0 && turns >= deps.checkpoint_turns`
     branch that logs `SessionEnd("budget_exceeded", turns)`, emits `PhaseRun`
     with `"budget_exceeded"`, builds artifacts (preserving partial edits on
     disk in `files_changed` and `diff`), and returns
     `budget_exceeded_result(..., checkpoint_line(turns, deps.checkpoint_turns, deps.max_turns), artifacts)`.
   - **Constraints:**
     - Do not change `PhaseStatus` or `Blocker` enum variants.
     - Clean completion on or before turn `checkpoint_turns` must return
       `PhaseStatus::Complete` (it returns before Step 9).
     - No `.unwrap()` / `.expect()` on production paths (`STANDARDS.md` §2.1).
5. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `task_no_edit_threshold` (default `40`, `0` disables) exists on
      `GovernorConfig`, `ModelOverride`, `Config::resolve_for_model`,
      `mcp/src/init.rs`, and `README.md`; `[budget] checkpoint_turns`
      (default `0`, disables) exists on `BudgetConfig`, `LoopDeps`,
      `mcp/src/runner.rs`, `mcp/src/init.rs`, and `README.md`.
- [ ] `HardFailSignal::TaskNoEditStall` and `check_task_no_edit_stall` exist in
      `executor/src/governor/hard_fail.rs`, and the agent loop fires
      `TaskNoEditStall` when an active task reaches `task_no_edit_threshold`
      consecutive edit-free turns — resetting on file edits or task switches,
      and **not** resetting on redundant `update_task(same_id, "active")`.
- [ ] When `checkpoint_turns > 0` and `< max_turns`, reaching turn
      `checkpoint_turns` without completing returns `PhaseStatus::BudgetExceeded`
      with `Blocker::BudgetExceeded`, partial edits preserved in `files_changed`
      / `diff`, and `briefing.budget_remaining` naming `"soft checkpoint"` and
      `"continue_phase"`.
- [ ] Every test named in § Test plan passes.
- [ ] Preservation: `checkpoint_turns = 0` leaves runs unchanged; `PhaseStatus`
      and `Blocker` variants are unchanged; all existing tests pass (including
      `mcp/tests/readme_config_reference.rs` and `mcp/src/init.rs`).
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/config.rs`:
  - `task_no_edit_and_checkpoint_knobs_default_allow_zero_and_override` —
    `GovernorConfig::default().task_no_edit_threshold == 40`,
    `BudgetConfig::default().checkpoint_turns == 0`, TOML `task_no_edit_threshold = 0`
    and `checkpoint_turns = 25` load cleanly, and per-model `[models."..."]`
    `task_no_edit_threshold` resolves via `resolve_for_model`.
- In `executor/src/governor/hard_fail.rs`:
  - `task_no_edit_stall_fires_at_threshold_and_disabled_at_zero_or_no_active_task`
    — fires `HardFailSignal::TaskNoEditStall` with expected `describe()` at
    `no_edit_turns >= threshold`; returns `None` below threshold, when
    `threshold == 0`, or when `active_task_id` is `None`.
- In `executor/src/agent/tests.rs`:
  - `task_no_edit_stall_fires_on_stuck_active_task_and_ignores_redundant_active`
    — with `task_no_edit_threshold: 3`, marking task `"1"` active, running two
    read-only calls, redundantly re-marking task `"1"` active (must **not**
    reset the counter), triggers `HardFailSignal::TaskNoEditStall { task_id: "1", consecutive_turns: 3 }`.
  - `task_no_edit_stall_resets_on_edit_and_task_switch` — negative case: with
    `task_no_edit_threshold: 3`, two edit-free turns then a `write_file` resets
    the counter (two more edit-free turns stay below threshold), and completing
    task `"1"` / activating task `"2"` resets the counter so the run completes.
  - `task_no_edit_stall_disabled_when_threshold_zero` — negative case:
    `task_no_edit_threshold = 0` lets an active task run 4 edit-free turns and
    complete cleanly.
  - `checkpoint_turns_returns_resumable_budget_exceeded_preserving_partial_edits`
    — with `checkpoint_turns: 2, max_turns: 10`, turn 1 edits a file via
    `write_file` and turn 2 runs a read-only call; the loop returns
    `PhaseStatus::BudgetExceeded`, `Blocker::BudgetExceeded`,
    `briefing.budget_remaining` contains `"soft checkpoint"` and
    `"continue_phase"`, and `result.files_changed` includes the edited file
    with its content preserved on disk.
  - `checkpoint_turns_zero_disabled_and_completion_before_checkpoint_succeeds`
    — negative cases: `checkpoint_turns = 0` runs past turn 2 and completes;
    and with `checkpoint_turns = 2`, finishing cleanly on turn 2 returns
    `PhaseStatus::Complete` (not `BudgetExceeded`).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-05.txt; : > "$A"
for t in task_no_edit_and_checkpoint_knobs_default_allow_zero_and_override task_no_edit_stall_fires_at_threshold_and_disabled_at_zero_or_no_active_task task_no_edit_stall_fires_on_stuck_active_task_and_ignores_redundant_active task_no_edit_stall_resets_on_edit_and_task_switch task_no_edit_stall_disabled_when_threshold_zero checkpoint_turns_returns_resumable_budget_exceeded_preserving_partial_edits checkpoint_turns_zero_disabled_and_completion_before_checkpoint_succeeds; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
for t in readme_documents_every_governor_knob readme_documents_every_per_model_override template_spells_out_every_governor_knob template_offers_every_per_model_override; do
  cargo test -p rexymcp "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test prints `ok` with `exit=0` and a non-zero passed count.

## Authorizations

- `executor/src/config.rs`
- `executor/src/governor/hard_fail.rs`
- `executor/src/agent/outcome.rs`
- `executor/src/agent/mod.rs`
- `executor/src/agent/tests.rs`
- `mcp/src/runner.rs`
- `mcp/src/init.rs`
- `README.md`

No new dependencies.

## Out of scope

- Changing `PhaseStatus` or `Blocker` enum variants.
- Async `continue_phase` (Phase 06).
- Unix process-group cleanup for `bash` and gate commands (Phase 07).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-25 03:45 (started)

**By:** executor (M51 phase-05)

Flipped the phase status to `in-progress` (doc + milestone README). Beginning
Spec task 1: config knobs (`task_no_edit_threshold`, `checkpoint_turns`).

### Update — 2026-11-25 04:10 (end-to-end verification)

Ran the phase doc's § End-to-end verification block verbatim; artifact pasted
from `target/e2e/m51-phase-05.txt` (redirected output, not retyped):

```
test config::tests::task_no_edit_and_checkpoint_knobs_default_allow_zero_and_override ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.00s
task_no_edit_and_checkpoint_knobs_default_allow_zero_and_override exit=0
test governor::hard_fail::tests::task_no_edit_stall_fires_at_threshold_and_disabled_at_zero_or_no_active_task ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.00s
task_no_edit_stall_fires_at_threshold_and_disabled_at_zero_or_no_active_task exit=0
test agent::tests::task_no_edit_stall_fires_on_stuck_active_task_and_ignores_redundant_active ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.01s
task_no_edit_stall_fires_on_stuck_active_task_and_ignores_redundant_active exit=0
test agent::tests::task_no_edit_stall_resets_on_edit_and_task_switch ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.02s
task_no_edit_stall_resets_on_edit_and_task_switch exit=0
test agent::tests::task_no_edit_stall_disabled_when_threshold_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.02s
task_no_edit_stall_disabled_when_threshold_zero exit=0
test agent::tests::checkpoint_turns_returns_resumable_budget_exceeded_preserving_partial_edits ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.01s
checkpoint_turns_returns_resumable_budget_exceeded_preserving_partial_edits exit=0
test agent::tests::checkpoint_turns_zero_disabled_and_completion_before_checkpoint_succeeds ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1143 filtered out; finished in 0.02s
checkpoint_turns_zero_disabled_and_completion_before_checkpoint_succeeds exit=0
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 710 filtered out; finished in 0.00s
test readme_documents_every_governor_knob ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
readme_documents_every_governor_knob exit=0
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 710 filtered out; finished in 0.00s
test readme_documents_every_per_model_override ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
readme_documents_every_per_model_override exit=0
test init::tests::template_spells_out_every_governor_knob ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 709 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
template_spells_out_every_governor_knob exit=0
test init::tests::template_offers_every_per_model_override ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 709 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
template_offers_every_per_model_override exit=0
```

Positive control: each of the 11 targeted tests prints `ok` with a non-zero
passed count and `exit=0` — a count that would read `0 passed` / non-zero exit
if the new code paths did not exist. Literal grep: `task_no_edit_threshold`
and `checkpoint_turns` appear 29 times across `mcp/src/init.rs`, `README.md`,
and `executor/src/config.rs` (29 matches, not zero).

Gates: `cargo fmt --all` clean; `cargo build` zero warnings;
`cargo clippy --all-targets --all-features -- -D warnings` clean;
`cargo test` — executor lib 1142 passed, rexymcp bin 710 passed,
readme_config_reference 2 passed, 0 failures anywhere.

### Update — 2026-11-25 04:12 (complete, server-authored)

**Summary:** </think>Committed, tree clean. Final summary.</think>
**Summary**

M51 phase-05 is implemented and committed (`feat:` commit, working tree clean; phase doc left at `in-progress` for the server's completion bookkeeping). All five Spec tasks are done:

1. **Config knobs** — `GovernorConfig::task_no_edit_threshold` (default 40; `0` allowed at load, unlike `read_only_stall_threshold`), `ModelOverride::task_no_edit_threshold` wired through `Config::resolve_for_model`, and `BudgetConfig::checkpoint_turns` (serde default `0`). Documented in `mcp/src/init.rs`'s template (`[budget]`, `[governor]`, `[models]` override comment) and README (sample config, per-model block, section-reference table).
2. **Signal** — `HardFailSignal::TaskNoEditStall { task_id, consecutive_turns }` with the pinned `describe()` text, plus `check_task_no_edit_stall` (returns `None` on threshold 0, no active task, or below threshold) in `executor/src/governor/hard_fail.rs`.
3. **Tracking** — `active_task_id` (seeded from post-resume states so resumed runs are bounded) and `task_no_edit_turns` in `execute_phase`; the counter increments only on edit-free, task-stable turns, resets on `made_file_progress` or a real task switch, and a redundant `update_task(same_id, "active")` does **not** reset it. Wired into Step 7's `hard_fail_signal` chain.
4. **Soft checkpoint** — `LoopDeps.checkpoint_turns` (wired from `inp.cfg.budget.checkpoint_turns` in `mcp/src/runner.rs`; `0` in all existing test literals), `checkpoint_line()` in `outcome.rs`, and a Step 9 `else if` branch (hard cap checked first) that logs `budget_exceeded`, emits `PhaseRun`, builds artifacts preserving partial edits, and returns `budget_exceeded_result(..., checkpoint_line(...))`. No `PhaseStatus`/`Blocker` variants changed; no `unwrap`/`expect` on production paths.
5. **E2E evidence** — pasted verbatim from `target/e2e/m51-phase-05.txt` into the phase doc's end-to-end Update Log entry: all 11 targeted tests `ok` with `exit=0` and non-zero passed counts (positive control), plus a literal grep confirming `task_no_edit_threshold`/`checkpoint_turns` landed in the template/README/config (29 matches).

**Notes for review**

- Two test-authoring adaptations worth knowing: the task-guard tests set `read_only_stall_threshold: 0` and `empty_completion_threshold: 50` in their governor fixture so the run-level read-only and empty-completion backstops don't fire before the per-task guard under test; and the "resets on task switch" test completes task 2 via `update_task` before the final turn because the task-coverage gate otherwise forces a `budget_exceeded` retry that masks the counter behavior.
- The checkpoint test's turn-1 call is a `read_file` (not a bare `write_file`) because the read-before-edit gate requires a prior read; the checkpoint still fires on turn 2 as specified.
- All four gates green: `cargo fmt --all`, `cargo build` (zero warnings), `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` (1142 executor + 710 rexymcp + 2 readme tests, 0 failures).

**Executor:** GLM-5.3-Flash

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::symbols::tests::references_finds_call_sites ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::symbols::tests::references_exclude_substring ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::references_exclude_strings_and_comments ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test store::telemetry::tests::append_is_atomic_under_concurrent_appenders ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1142 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.16s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `README.md` — +7 -4
- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` — +1 -1
- `docs/dev/milestones/M51-governor-repetition-and-cycles/phase-05-task-guard-and-checkpoint.md` — +64 -1
- `executor/src/agent/mod.rs` — +99 -3
- `executor/src/agent/outcome.rs` — +6 -0
- `executor/src/agent/tests.rs` — +378 -0
- `executor/src/config.rs` — +84 -0
- `executor/src/governor/hard_fail.rs` — +63 -0
- `mcp/src/init.rs` — +3 -0
- `mcp/src/runner.rs` — +3 -0

**Commit:** 40e8548ac43d3160cc03ea84da14cb7dffc3332e

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).
