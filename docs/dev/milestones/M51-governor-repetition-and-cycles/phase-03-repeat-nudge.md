# Phase 03: Recoverable repeat nudge + `NudgeInjected` event & telemetry

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress *(bounced 2026-11-24 — see [bugs/bug-03-1.md](bugs/bug-03-1.md))*
**Depends on:** phase-02
**Estimated diff:** ~320 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add a recoverable repeat-nudge stage ahead of `IdenticalToolCallRepetition` so
repeating the same command (including re-plumbed bash variants) with the same
timing-normalized output injects one steering message, logs a `NudgeInjected`
session event, and records nudge counts in `PhaseRun` telemetry — keeping
`IdenticalToolCallRepetition` at `identical_call_threshold` as the hard-fail
backstop. Implements Proposal 2 of [issue #19](https://github.com/ryanczak/rexyMCP/issues/19).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Exit criteria.
- `docs/dev/issues/2026-11-24-executor-never-batches-repeated-commands.md` § Proposal 2.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/src/governor/normalize.rs` provides `normalize_tool_args_for_repeat`
and `normalize_output_for_repeat`. `executor/src/config.rs` defines
`GovernorConfig` and `ModelOverride` (resolved by `Config::resolve_for_model`),
and both `mcp/src/init.rs` and `README.md` are guard-tested against every field
of both structs (`template_spells_out_every_governor_knob`,
`readme_documents_every_governor_knob`, and their per-model twins).
`executor/src/store/sessions/event.rs` defines `SessionEvent` (exhaustively
matched in `executor/src/agent/tests.rs::event_kind`, `mcp/src/log_query.rs`,
`mcp/src/dashboard/filter.rs`, and `mcp/src/dashboard/transcript.rs`).
`executor/src/agent/mod.rs` injects gate/task feedback via
`messages.push(user_text(&feedback, turns))` and evaluates hard-fail signals
in Step 7. `executor/src/store/telemetry.rs` defines `PhaseRun`, populated by
`executor/src/agent/metrics.rs::emit_phase_run`.

## Spec

1. **Config knob (`repeat_nudge_threshold`)** — in `executor/src/config.rs`:
   - Add `pub repeat_nudge_threshold: usize` to `GovernorConfig` (default `3`;
     `0` disables the repeat nudge — unlike `read_only_stall_threshold`, `0` is
     allowed at load).
   - Add `pub repeat_nudge_threshold: Option<usize>` to `ModelOverride` and
     wire it in `Config::resolve_for_model`.
   - Add `repeat_nudge_threshold` to the `[governor]` and `[models]` sample
     blocks in `mcp/src/init.rs` (`generate_config`) and `README.md` so the
     template and README guard tests stay green.
2. **`SessionEvent::NudgeInjected` and exhaustive consumers**:
   - In `executor/src/store/sessions/event.rs`, add
     `SessionEvent::NudgeInjected { kind: String, tool: String, consecutive_count: usize }`
     (serializes with `event_type = "nudge_injected"`).
   - Add the matching arms in `executor/src/agent/tests.rs` (`event_kind` ->
     `"nudge_injected"`), `mcp/src/log_query.rs` (`event_type_str` ->
     `"nudge_injected"`), `mcp/src/dashboard/filter.rs`
     (`ActivityFilter::allows` -> `self.hard_fail`, keeping `FILTER_ITEM_COUNT`
     unchanged), and `mcp/src/dashboard/transcript.rs` (`record_lines_with_lang`
     -> styled one-line summary naming the kind, tool, and count).
3. **`PhaseRun` telemetry counters**:
   - In `executor/src/store/telemetry.rs`, add `#[serde(default)] pub nudges_injected: usize`
     and `#[serde(default)] pub nudges_heeded: usize` to `PhaseRun` (keeps
     `TELEMETRY_SCHEMA_VERSION = 1` and backward compatibility with legacy
     JSONL records).
   - In `executor/src/agent/metrics.rs`, track `nudges_injected` and
     `nudges_heeded` on `RunMetrics` and populate them on `PhaseRun` in
     `emit_phase_run`.
4. **Repeat-nudge detection and one-shot injection in `executor/src/agent/mod.rs`**:
   - Track the trailing repeat streak across tool calls using
     `key = (tool_call.name.clone(), normalize_tool_args_for_repeat(&tool_call.name, &tool_call.arguments))`
     and `norm_out = normalize_output_for_repeat(&content)`.
   - If a prior nudge is awaiting outcome (`pending_nudge_key: Option<(String, serde_json::Value)>`),
     consume it on this tool call: if `key != nudged_key`, increment
     `metrics.nudges_heeded += 1`.
   - If `(key, norm_out)` equals the previous tool call's `(key, norm_out)`,
     increment the repeat streak; otherwise reset the streak to `1` and clear
     `nudged_this_streak = false`.
   - After Step 7 hard-fail checks (so a hard-fail at `identical_call_threshold`
     still wins when reached, and a hard-failed run does not inject a pointless
     nudge), if `deps.governor.repeat_nudge_threshold > 0`,
     `repeat_streak >= deps.governor.repeat_nudge_threshold`, and
     `!nudged_this_streak`:
     - Inject one user message via `messages.push(user_text(&nudge, turns))`:
       `"You have run this command {repeat_streak} times with the same result. Running it again will give the same answer. If you need repetitions (a stress loop, a sweep), write one loop or script that does all of them and run it once. Otherwise, move on to the next task."`
     - Emit `SessionEvent::NudgeInjected { kind: "repeat".to_string(), tool: tool_call.name.clone(), consecutive_count: repeat_streak }`.
     - Increment `metrics.nudges_injected += 1`, set `nudged_this_streak = true`,
       and set `pending_nudge_key = Some(key)`.
   - **Constraints:**
     - Never inject the nudge more than once per streak (turns 4, 5, … of the
       same streak must not inject again).
     - Do not change `check_identical_repetition` — it still fires at
       `identical_call_threshold` on exact whitespace-normalized args only.
     - No `.unwrap()` / `.expect()` on runtime paths (`STANDARDS.md` §2.1).
     - Gotcha: if an existing `agent/tests.rs` test deliberately runs 3+
       identical calls and asserts an exact event/message count that did not
       anticipate nudges, check why before touching it; do not weaken existing
       assertions.
5. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `repeat_nudge_threshold` exists on `GovernorConfig` (default `3`, `0`
      disables), on `ModelOverride`, in `Config::resolve_for_model`, in
      `mcp/src/init.rs`, and in `README.md`.
- [ ] `SessionEvent::NudgeInjected` exists and round-trips with
      `"event_type":"nudge_injected"`, and `PhaseRun` records `nudges_injected`
      and `nudges_heeded` with `#[serde(default)]` backward compatibility.
- [ ] Every test named in § Test plan passes.
- [ ] `repeat_nudge_silent_when_outputs_differ` genuinely exercises the output-equality
      check in `executor/src/agent/mod.rs`: it issues three **identical** tool calls
      (same `(name, arguments)` across all three `SessionEvent::Parsed` records,
      asserted in the test; `! grep -n 'echo run-' executor/src/agent/tests.rs`
      exits 0) whose `SessionEvent::ToolResult` `output_preview`s are all distinct
      (asserted in the test), stays hermetic within `TempDir`, and injects zero
      nudges (see [bugs/bug-03-1.md](bugs/bug-03-1.md)).
- [ ] Preservation: `check_identical_repetition` still fires at
      `identical_call_threshold` on exact repeats and does **not** fire on
      re-plumbed bash variants; all existing tests pass (including
      `mcp/tests/readme_config_reference.rs` and `mcp/src/init.rs`).
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/config.rs`:
  - `repeat_nudge_threshold_defaults_to_three_and_allows_zero` — default is `3`;
    TOML `repeat_nudge_threshold = 0` loads cleanly (not refused); per-model
    `[models."..."]` override resolves via `resolve_for_model`.
- In `executor/src/store/telemetry.rs`:
  - `phase_run_without_nudge_fields_deserializes_to_zero` — legacy JSON line
    omitting `nudges_injected` / `nudges_heeded` deserializes with both `0`.
- In `executor/src/agent/tests.rs` (use `MockAiClient` + `bash_with_filter` /
  `run_with_bash` analogue, inspecting the messages sent to the mock client,
  the session log, and the emitted `PhaseRun` telemetry):
  - `repeat_nudge_injects_once_and_resets_on_different_call` — three identical
    bash calls with identical outputs inject **exactly one** nudge message to
    the model and emit one `SessionEvent::NudgeInjected`; a fourth call that
    differs resets the streak and counts as heeded (`nudges_heeded == 1`); two
    further identical calls of a new command stay below threshold (still only
    1 nudge total) and the run completes with `PhaseRun` reporting
    `nudges_injected == 1` and `nudges_heeded == 1`.
  - `repeat_nudge_collapses_replumbed_bash_without_hard_fail` — calling
    `cmd | tail -4`, `cmd | tail -4 > /tmp/x && cat /tmp/x`, and
    `cmd 2>&1 | tail -6` with identical output triggers one repeat nudge and
    continues (does **not** trip `IdenticalToolCallRepetition`).
  - `repeat_nudge_silent_when_outputs_differ` — negative case: three **identical**
    calls (same tool name and equal `arguments` on all three `SessionEvent::Parsed`
    records — asserted in the test, with no host state outside `TempDir`) whose
    `SessionEvent::ToolResult` outputs genuinely differ (not merely timing —
    asserted distinct in the test) inject zero nudges.
  - `repeat_nudge_disabled_when_threshold_zero` — negative case:
    `repeat_nudge_threshold = 0` injects zero nudges on three identical calls.
  - `identical_repetition_hard_fail_still_fires_after_nudge` — six byte-identical
    calls with `repeat_nudge_threshold = 3` and `identical_call_threshold = 6`
    inject exactly one nudge at call 3 (calls 4 and 5 do **not** re-nudge) and
    still hard-fail with `IdenticalToolCallRepetition` at call 6
    (`nudges_injected == 1`, `nudges_heeded == 0`).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-03.txt; : > "$A"
for t in repeat_nudge_threshold_defaults_to_three_and_allows_zero phase_run_without_nudge_fields_deserializes_to_zero repeat_nudge_injects_once_and_resets_on_different_call repeat_nudge_collapses_replumbed_bash_without_hard_fail repeat_nudge_silent_when_outputs_differ repeat_nudge_disabled_when_threshold_zero identical_repetition_hard_fail_still_fires_after_nudge; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
cargo test -p rexymcp readme_documents_every_governor_knob 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "readme_governor exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp template_spells_out_every_governor_knob 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "init_governor exit=${PIPESTATUS[0]}" >> "$A"
! grep -n 'echo run-' executor/src/agent/tests.rs >> "$A"; echo "no_varied_cmd exit=$?" >> "$A"
```

Expected: every listed test prints `ok` with `exit=0`, each `test result` line
shows a non-zero passed count, and `no_varied_cmd exit=0`.

## Authorizations

- `executor/src/config.rs`
- `executor/src/store/sessions/event.rs`
- `executor/src/store/telemetry.rs`
- `executor/src/agent/metrics.rs`
- `executor/src/agent/mod.rs`
- `executor/src/agent/tests.rs`
- `mcp/src/init.rs`
- `mcp/src/log_query.rs`
- `mcp/src/dashboard/filter.rs`
- `mcp/src/dashboard/transcript.rs`
- `README.md`

No new dependencies.

## Out of scope

- Cyclic read-only detection (`ReadOnlyCycle`) and `calibrate-governor` replay
  (Phase 04).
- Per-task no-edit cap and `checkpoint_turns` (Phase 05).
- Rendering nudge counts in `rexymcp runs` / `scorecard` CLI tables (follow-up
  once telemetry accumulates).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-25 02:35 (progress)

Started phase-03. Flipped status to in-progress. Beginning with the config knob, then the event, telemetry, loop integration, and tests.

### Update — 2026-11-25 02:55 (progress)

All four spec tasks implemented: config knob (with template/README guard tests green), `SessionEvent::NudgeInjected` + exhaustive consumer arms, `PhaseRun` nudge counters with `#[serde(default)]`, and one-shot repeat-nudge injection after the Step-7 hard-fail checks. All 6 new agent-loop tests plus the config and telemetry tests pass; existing governor tests untouched and green.

### Update — 2026-11-25 02:58 (end-to-end verification)

Ran the phase doc's E2E block verbatim; artifact file `target/e2e/m51-phase-03.txt`:

```
test config::tests::repeat_nudge_threshold_defaults_to_three_and_allows_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.00s
repeat_nudge_threshold_defaults_to_three_and_allows_zero exit=0
test store::telemetry::tests::phase_run_without_nudge_fields_deserializes_to_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.00s
phase_run_without_nudge_fields_deserializes_to_zero exit=0
test agent::tests::repeat_nudge_injects_once_and_resets_on_different_call ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.04s
repeat_nudge_injects_once_and_resets_on_different_call exit=0
test agent::tests::repeat_nudge_collapses_replumbed_bash_without_hard_fail ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.03s
repeat_nudge_collapses_replumbed_bash_without_hard_fail exit=0
test agent::tests::repeat_nudge_silent_when_outputs_differ ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.03s
repeat_nudge_silent_when_outputs_differ exit=0
test agent::tests::repeat_nudge_disabled_when_threshold_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.03s
repeat_nudge_disabled_when_threshold_zero exit=0
test agent::tests::identical_repetition_hard_fail_still_fires_after_nudge ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1128 filtered out; finished in 0.03s
identical_repetition_hard_fail_still_fires_after_nudge exit=0
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 708 filtered out; finished in 0.00s
test readme_documents_every_governor_knob ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
readme_governor exit=0
test init::tests::template_spells_out_every_governor_knob ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 707 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
init_governor exit=0
```

Every listed test `ok` with `exit=0` and a non-zero passed count. Positive control: the E2E run includes the two negative agent-loop cases (`repeat_nudge_silent_when_outputs_differ`, `repeat_nudge_disabled_when_threshold_zero`) and the positive ones in the same session — the suite discriminates nudge/no-nudge, so a green result is not silence. Literal grep: `"event_type":"nudge_injected"` round-trip asserted in `executor/src/store/sessions/jsonl.rs::nudge_injected_variant_round_trips_with_event_type_tag` (ok); `nudge_injected` arms present in `executor/src/agent/tests.rs:1038` and `mcp/src/log_query.rs:33`.

### Update — 2026-11-25 02:51 (complete, server-authored)

**Summary:** </think>Done. Final summary.</think>
**Summary:** Implemented M51 phase-03 end to end. `[governor] repeat_nudge_threshold` (default 3, `0` allowed at load and disables the nudge) now exists on `GovernorConfig`, `ModelOverride`, `Config::resolve_for_model`, the `rexymcp init` template, and both README sample blocks — all guard tests green. `SessionEvent::NudgeInjected { kind, tool, consecutive_count }` serializes with `"event_type":"nudge_injected"` (round-trip test in `jsonl.rs`) and has arms in `event_kind`, `log_query.rs`, `dashboard/filter.rs` (mapped to `hard_fail`, `FILTER_ITEM_COUNT` unchanged), and `transcript.rs`. `PhaseRun` gained `#[serde(default)] nudges_injected`/`nudges_heeded` (schema version untouched), populated from `RunMetrics` in `emit_phase_run`. The agent loop tracks a trailing streak over `(tool, normalize_tool_args_for_repeat)` + `normalize_output_for_repeat`, consumes a pending nudge as heeded when the next call differs, and — after Step 7's hard-fail checks — injects exactly one steering message per streak, emitting the event and bumping `nudges_injected`. `check_identical_repetition` is untouched.

**Notes for review:** (1) `nudges_heeded` increments only when the call *immediately after* a nudge differs from the nudged key; if the model repeats once more and then changes, that repeat still counts as unheeded-then-different (spec's "if `key != nudged_key`" reading). (2) The test helper `nudge_messages` counts per-call *deltas* of user nudge messages, since the conversation history is monotonic and a one-shot nudge reappears in every later call's message list — without that, one injection reads as N. (3) `mcp/src/profile.rs` and `mcp/src/runner.rs` test initializers needed the two new `PhaseRun`/`ModelOverride` fields (compiler-required, not spec-listed files). (4) E2E block ran verbatim with all named tests `ok`/`exit=0` (artifact quoted in the phase doc's end-to-end verification entry); all four gates green; tree clean, committed as one `feat:` commit.

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

test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::symbols::tests::references_finds_call_sites ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::references_exclude_substring ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_exclude_strings_and_comments ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test agent::tests::bash_edit_resets_read_only_stall ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
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

test result: ok. 1127 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.13s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `README.md` — +2 -0
- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` — +1 -1
- `docs/dev/milestones/M51-governor-repetition-and-cycles/phase-03-repeat-nudge.md` — +47 -1
- `executor/src/agent/metrics.rs` — +6 -0
- `executor/src/agent/mod.rs` — +66 -0
- `executor/src/agent/tests.rs` — +321 -0
- `executor/src/config.rs` — +84 -0
- `executor/src/store/sessions/event.rs` — +10 -0
- `executor/src/store/sessions/jsonl.rs` — +35 -0
- `executor/src/store/telemetry.rs` — +31 -0
- `mcp/src/dashboard/filter.rs` — +1 -0
- `mcp/src/dashboard/transcript.rs` — +12 -0
- `mcp/src/init.rs` — +2 -0
- `mcp/src/log_query.rs` — +1 -0
- `mcp/src/profile.rs` — +4 -0
- `mcp/src/runner.rs` — +2 -0

**Commit:** 3ef326c77d1e2c042e57b8ed6bb649d5b08cf91f

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).
