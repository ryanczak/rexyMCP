# Phase 03: Recoverable repeat nudge + `NudgeInjected` event & telemetry

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** todo
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
  - `repeat_nudge_silent_when_outputs_differ` — negative case: three identical
    calls whose tool outputs differ (not merely timing) inject zero nudges.
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
```

Expected: every listed test prints `ok` with `exit=0`, and each `test result`
line shows a non-zero passed count.

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
