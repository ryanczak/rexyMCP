# Phase 04: Plumbing-normalized `ReadOnlyCycle` detector & calibration replay

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress
**Depends on:** phase-03
**Estimated diff:** ~360 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add a dedicated plumbing-normalized `ReadOnlyCycle` detector that catches
cyclic read-only / bash loops (such as alternating `grep … | wc -l` and
`grep … | head -2` across 2–4 commands), nudges once recoverably before
terminating if the model stays in the cycle, and replays the signal in
`rexymcp calibrate-governor` — while keeping `LowNoveltyStall` advisory by
default and never false-firing on multi-chunk `read_file` or distinct `search`
exploration. Implements Ask 1 of [issue #18](https://github.com/ryanczak/rexyMCP/issues/18).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Exit criteria and § Notes #2.
- `docs/architecture.md` § Status, M51 and M34 / M37 entries.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/src/governor/normalize.rs` provides `normalize_tool_args_for_repeat`
(strips trailing bash plumbing and simple quotes for `bash`, and uses
whitespace-only normalization for all other tools so `read_file` line ranges
and `search` patterns stay distinct). `executor/src/governor/hard_fail.rs`
defines `HardFailSignal`, `measure_novelty`, `check_oscillation`, and
`check_read_only_stall`. `executor/src/config.rs` defines `GovernorConfig` and
`ModelOverride`, guard-tested by `mcp/src/init.rs` and
`mcp/tests/readme_config_reference.rs` (and explicitly constructed in test
literals in `mcp/src/runner.rs`). `executor/src/agent/mod.rs` evaluates Step 7
hard-fail signals and emits `SessionEvent::NudgeInjected` + `RunMetrics` nudge
counters. `mcp/src/calibrate_governor.rs` replays session logs over `SIGNALS`
and formats the percentile table.

## Spec

1. **Config knobs (`read_only_cycle_window`, `read_only_cycle_distinct_max`)** —
   in `executor/src/config.rs`:
   - Add `pub read_only_cycle_window: usize` (default `16`; `0` disables) and
     `pub read_only_cycle_distinct_max: usize` (default `4`; `0` disables) to
     `GovernorConfig`. Both `0` values are allowed at load (unlike
     `read_only_stall_threshold`).
   - Add matching `Option<usize>` fields to `ModelOverride`, wire them in
     `Config::resolve_for_model`, and update the explicit `ModelOverride` test
     literals in `mcp/src/runner.rs`.
   - Document both knobs in `[governor]` and `[models]` in `mcp/src/init.rs`
     (`generate_config`) and `README.md` so the template and README guard tests
     stay green.
2. **Detector & signal in `executor/src/governor/hard_fail.rs`**:
   - Add `HardFailSignal::ReadOnlyCycle { window: usize, distinct_calls: usize }`
     with `describe()` formatting
     `"only {distinct_calls} distinct normalized read-only calls across the last {window} calls (read-only cycle)"`.
   - Export `pub fn read_only_cycle_keys(recent: &VecDeque<ToolCallSnapshot>, window: usize) -> Option<Vec<(String, serde_json::Value)>>`:
     scans the trailing read-only run (`recent.iter().rev()`, stopping at any
     call where `call.made_file_progress()` is true). Returns `None` if
     `window == 0` or fewer than `window` read-only calls precede the last
     file-progress call; otherwise returns the deduplicated list of
     `(call.tool.clone(), normalize_tool_args_for_repeat(&call.tool, &call.arguments))`
     keys over those last `window` calls.
   - Export `pub fn measure_read_only_cycle(recent: &VecDeque<ToolCallSnapshot>, window: usize) -> Option<usize>`
     returning `read_only_cycle_keys(recent, window).map(|keys| keys.len())`.
   - Export `pub fn check_read_only_cycle(recent: &VecDeque<ToolCallSnapshot>, window: usize, distinct_max: usize) -> Option<HardFailSignal>`:
     returns `None` when `window == 0` or `distinct_max == 0`; otherwise calls
     `measure_read_only_cycle(recent, window)?` and returns
     `Some(HardFailSignal::ReadOnlyCycle { window, distinct_calls })` when
     `distinct_calls <= distinct_max` (including `distinct_calls == 1`, which
     catches re-plumbed 1-command loops that evade `IdenticalToolCallRepetition`).
3. **Staged nudge-first, then terminate in `executor/src/agent/mod.rs`**:
   - Maintain `cycle_calls_since_reset: usize` (starts at `0`) and
     `cycle_nudge_keys: Option<Vec<(String, serde_json::Value)>>` (starts at
     `None`), independent of the repeat-nudge state (`pending_nudge_key`).
   - On each tool call (after `recent_tool_calls.push_back` and computing the
     normalized `key`):
     - Compute `made_file_progress = crate::tools::mutates_files(&tool_call.name) || changed_tree`.
     - If `let Some(keys) = cycle_nudge_keys.take()`:
       - If `!made_file_progress && keys.contains(&key)`: the model stayed
         inside the nudged cycle — set
         `cycle_hard_fail = Some(HardFailSignal::ReadOnlyCycle { window: deps.governor.read_only_cycle_window, distinct_calls: keys.len() })`.
       - Otherwise (file progress **or** a call outside `keys`): the model
         **heeded** the nudge — increment `metrics.nudges_heeded += 1` and
         reset `cycle_calls_since_reset = if made_file_progress { 0 } else { 1 }`
         so stale pre-nudge calls do not re-trigger the detector immediately.
     - Else: if `made_file_progress` reset `cycle_calls_since_reset = 0`; else
       increment `cycle_calls_since_reset += 1`.
   - Wire `.or(cycle_hard_fail)` into Step 7's `hard_fail_signal` chain so
     continuing the nudged cycle terminates with `HardFailSignal::ReadOnlyCycle`
     (while `IdenticalToolCallRepetition` and earlier Step 7 signals still take
     precedence if reached).
   - After Step 7's `hard_fail_signal` return check: if
     `deps.governor.read_only_cycle_window > 0`,
     `deps.governor.read_only_cycle_distinct_max > 0`,
     `cycle_nudge_keys.is_none()`, and
     `cycle_calls_since_reset >= deps.governor.read_only_cycle_window`, call
     `read_only_cycle_keys(&recent_tool_calls, deps.governor.read_only_cycle_window)`:
     when `keys.len() <= deps.governor.read_only_cycle_distinct_max`,
     - Inject one user message via `messages.push(user_text(&nudge, turns))`:
       `"You are cycling through the same {distinct} read-only commands across the last {window} calls without making edits. Stop re-running these commands and either apply an edit, run a single batched command, or move to the next task. Continuing this cycle will terminate the run."`
     - Emit `SessionEvent::NudgeInjected { kind: "read_only_cycle".to_string(), tool: tool_call.name.clone(), consecutive_count: deps.governor.read_only_cycle_window }`.
     - Increment `metrics.nudges_injected += 1` and set `cycle_nudge_keys = Some(keys)`.
   - **Constraints:**
     - Leave `GovernorConfig::default().novelty_action == NoveltyAction::Advisory`
       and `check_oscillation` untouched.
     - No `.unwrap()` / `.expect()` on production paths (`STANDARDS.md` §2.1).
     - Gotcha: when a model heeds the cycle nudge by calling a brand-new read
       command `C` (not in `keys`) while `distinct_max >= 3`, the raw trailing
       window still contains pre-nudge calls; resetting `cycle_calls_since_reset`
       is required so that `C` does **not** trigger an immediate hard-fail or
       second nudge.
4. **Calibration replay in `mcp/src/calibrate_governor.rs`**:
   - Add `Signal::ReadOnlyCycleMinDistinct` (`label`:
     `"read_only_cycle_min_distinct"`, `direction`: `TailDirection::LowerIsWorse`,
     `READ_ONLY_CYCLE_WINDOW = 16`, header noting lower-is-worse read-only
     cycling), wired into `SIGNALS` and `format_report`.
   - `samples` replays `run_.tool_calls` turn-by-turn through
     `measure_read_only_cycle(&deque, READ_ONLY_CYCLE_WINDOW)` and returns
     `vec![min]` (or `vec![]` if no full read-only window was reached).
5. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `read_only_cycle_window` (default `16`, `0` disables) and
      `read_only_cycle_distinct_max` (default `4`, `0` disables) exist on
      `GovernorConfig`, `ModelOverride`, `Config::resolve_for_model`,
      `mcp/src/init.rs`, and `README.md`.
- [ ] `HardFailSignal::ReadOnlyCycle`, `read_only_cycle_keys`,
      `measure_read_only_cycle`, and `check_read_only_cycle` exist in
      `executor/src/governor/hard_fail.rs` and collapse re-plumbed bash cycles
      without collapsing distinct `read_file` line ranges or `search` patterns.
- [ ] The agent loop nudges once (`SessionEvent::NudgeInjected { kind: "read_only_cycle", .. }`)
      on a read-only cycle, terminates with `HardFailSignal::ReadOnlyCycle` if
      the next call stays inside the cycle, and counts the nudge as heeded
      (without hard-failing) if the next call edits a file or runs a new command
      outside the cycle.
- [ ] `mcp/src/calibrate_governor.rs` replays `read_only_cycle_min_distinct`
      via `measure_read_only_cycle` with `LowerIsWorse` tail reporting.
- [ ] Every test named in § Test plan passes.
- [ ] Preservation: `NoveltyAction::Advisory` remains the default,
      `check_oscillation` and `check_identical_repetition` semantics are
      unchanged, and all existing tests pass (including
      `mcp/tests/readme_config_reference.rs` and `mcp/src/init.rs`).
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/config.rs`:
  - `read_only_cycle_knobs_default_and_allow_zero_and_override` — defaults are
    `16` and `4`; `0` for either loads cleanly; per-model `[models."..."]`
    overrides resolve via `resolve_for_model`.
- In `executor/src/governor/hard_fail.rs`:
  - `read_only_cycle_catches_replumbed_grep_cycle` — alternating re-plumbed
    bash calls (`grep -rn "foo" src/a.rs | wc -l`,
    `grep -rn 'foo' src/a.rs | head -2`, `grep -rn "bar" src/b.rs | wc -l`,
    `grep -rn 'bar' src/b.rs | head -2`) collapse to 2 distinct keys and fire
    `HardFailSignal::ReadOnlyCycle { window: 16, distinct_calls: 2 }`.
  - `read_only_cycle_does_not_collapse_distinct_read_file_slices_or_search_patterns`
    — negative case: 16 `read_file` calls on the same file with distinct
    `start_line`/`end_line` ranges (or 16 `search` calls with distinct patterns)
    measure `distinct_calls == 16` and return `None` from `check_read_only_cycle`.
  - `read_only_cycle_resets_on_file_progress_and_disabled_at_zero` — negative
    case: an intervening `patch` or `changed_tree: true` bash call resets the
    trailing read-only run below `window`; `window == 0` or `distinct_max == 0`
    returns `None`.
- In `executor/src/agent/tests.rs`:
  - `read_only_cycle_nudges_once_then_terminates_if_unchanged` — with
    `read_only_cycle_window: 8, read_only_cycle_distinct_max: 4`, alternating
    two re-plumbed bash calls for 8 turns injects one `read_only_cycle` nudge
    (`NudgeInjected { kind: "read_only_cycle", .. }`), and a 9th call inside the
    cycle terminates with `HardFailSignal::ReadOnlyCycle` (`nudges_injected == 1`,
    `nudges_heeded == 0`).
  - `read_only_cycle_nudge_heeded_by_new_command_does_not_hard_fail` — after
    the cycle nudge at turn 8 (with `distinct_max: 4` and a 2-command cycle),
    calling a brand-new read command `C` on turn 9 counts as heeded
    (`nudges_injected == 1`, `nudges_heeded == 1`), does **not** hard-fail even
    though `{A, B, C}.len() <= 4`, and allows the run to complete cleanly.
  - `read_only_cycle_nudge_heeded_by_edit_resets_and_completes` — after the
    cycle nudge at turn 8, editing a file via `write_file` counts as heeded and
    allows the run to complete cleanly.
  - `read_only_cycle_disabled_when_window_zero` — negative case:
    `read_only_cycle_window = 0` injects zero cycle nudges and does not
    hard-fail on an 8-call alternating bash cycle.
- In `mcp/src/calibrate_governor.rs`:
  - `read_only_cycle_min_distinct_replays_via_measure_read_only_cycle` —
    replaying a session with a 16-call re-plumbed 2-command grep cycle produces
    sample `[2]` (and `vec![]` when below 16 read-only calls or reset by an edit),
    with `TailDirection::LowerIsWorse`.
  - `read_only_cycle_signal_appears_in_report` — `run()` emits a
    `signal: read_only_cycle_min_distinct` section with low-tail headers
    (`P50  P10  P1`).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-04.txt; : > "$A"
for t in read_only_cycle_knobs_default_and_allow_zero_and_override read_only_cycle_catches_replumbed_grep_cycle read_only_cycle_does_not_collapse_distinct_read_file_slices_or_search_patterns read_only_cycle_resets_on_file_progress_and_disabled_at_zero read_only_cycle_nudges_once_then_terminates_if_unchanged read_only_cycle_nudge_heeded_by_new_command_does_not_hard_fail read_only_cycle_nudge_heeded_by_edit_resets_and_completes read_only_cycle_disabled_when_window_zero; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
for t in read_only_cycle_min_distinct_replays_via_measure_read_only_cycle read_only_cycle_signal_appears_in_report readme_documents_every_governor_knob readme_documents_every_per_model_override template_spells_out_every_governor_knob template_offers_every_per_model_override; do
  cargo test -p rexymcp "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test prints `ok` with `exit=0` and a non-zero passed count.

## Authorizations

- `executor/src/config.rs`
- `executor/src/governor/hard_fail.rs`
- `executor/src/agent/mod.rs`
- `executor/src/agent/tests.rs`
- `mcp/src/calibrate_governor.rs`
- `mcp/src/init.rs`
- `mcp/src/runner.rs`
- `README.md`

No new dependencies.

## Out of scope

- Changing `LowNoveltyStall`'s default `NoveltyAction::Advisory` or `check_oscillation`.
- Per-task no-edit cap and `[budget] checkpoint_turns` (Phase 05).
- Async `continue_phase` (Phase 06) and Unix process-group cleanup (Phase 07).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-24 19:20 (started)

**Executor:** executor-run (headless)
**Scope:** Tasks 1–5 from the Spec; beginning with config knobs, then detector signal, agent-loop staging, calibration replay, and end-to-end evidence.

### Update — 2026-11-24 19:52 (end-to-end verification)

Ran the § End-to-end verification block verbatim; artifact below (mechanically
captured into `target/e2e/m51-phase-04.txt`). Positive control: every listed test
asserts a non-trivial outcome (a fired `ReadOnlyCycle` signal with
`distinct_calls: 2`, a nudge event with `kind: "read_only_cycle"`, heeded
counters, or `vec![2]` calibration samples) — each would fail if the detector
did not run; the negative cases assert no signal on distinct slices/patterns and
no nudge at `window = 0`. Every listed test prints `ok` with `exit=0` and a
non-zero passed count.

```
test config::tests::read_only_cycle_knobs_default_and_allow_zero_and_override ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.00s
read_only_cycle_knobs_default_and_allow_zero_and_override exit=0
test governor::hard_fail::read_only_cycle_tests::read_only_cycle_catches_replumbed_grep_cycle ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.00s
read_only_cycle_catches_replumbed_grep_cycle exit=0
test governor::hard_fail::read_only_cycle_tests::read_only_cycle_does_not_collapse_distinct_read_file_slices_or_search_patterns ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.00s
read_only_cycle_does_not_collapse_distinct_read_file_slices_or_search_patterns exit=0
test governor::hard_fail::read_only_cycle_tests::read_only_cycle_resets_on_file_progress_and_disabled_at_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.00s
read_only_cycle_resets_on_file_progress_and_disabled_at_zero exit=0
test agent::tests::read_only_cycle_nudges_once_then_terminates_if_unchanged ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.05s
read_only_cycle_nudges_once_then_terminates_if_unchanged exit=0
test agent::tests::read_only_cycle_nudge_heeded_by_new_command_does_not_hard_fail ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.05s
read_only_cycle_nudge_heeded_by_new_command_does_not_hard_fail exit=0
test agent::tests::read_only_cycle_nudge_heeded_by_edit_resets_and_completes ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.05s
read_only_cycle_nudge_heeded_by_edit_resets_and_completes exit=0
test agent::tests::read_only_cycle_disabled_when_window_zero ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1136 filtered out; finished in 0.05s
read_only_cycle_disabled_when_window_zero exit=0
test calibrate_governor::read_only_cycle_tests::read_only_cycle_min_distinct_replays_via_measure_read_only_cycle ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 709 filtered out; finished in 0.01s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
read_only_cycle_min_distinct_replays_via_measure_read_only_cycle exit=0
test calibrate_governor::read_only_cycle_tests::read_only_cycle_signal_appears_in_report ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 709 filtered out; finished in 0.01s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s
read_only_cycle_signal_appears_in_report exit=0
test readme_documents_every_governor_knob ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
readme_documents_every_governor_knob exit=0
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

Gates (separate invocations, all clean):

- `cargo fmt --all` then `cargo fmt --all --check` → OK
- `cargo build` → Finished, no warnings
- `cargo clippy --all-targets --all-features -- -D warnings` → Finished, no warnings
- `cargo test` → 710 + 2 (rexymcp), 1135 + 0 (rexymcp-executor), 0 failed

Literal grep (spec-pinned names land in the files the phase populates):
`read_only_cycle_window` present in `executor/src/config.rs` (12),
`mcp/src/init.rs` (2), `README.md` (2), `mcp/src/runner.rs` (2);
`HardFailSignal::ReadOnlyCycle` / `read_only_cycle_keys` / `measure_read_only_cycle`
/ `check_read_only_cycle` present in `executor/src/governor/hard_fail.rs`,
`executor/src/agent/mod.rs`, and `mcp/src/calibrate_governor.rs`.
