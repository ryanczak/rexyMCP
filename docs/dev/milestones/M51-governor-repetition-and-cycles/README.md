# M51 — Governor repetition, cycles & process hygiene

**Goal:** Teach the executor to batch repeated shell work, intervene recoverably
on repeated or cyclic read-only commands before burning hundreds of turns or
killing working runs, and close the companion `continue_phase` timeout and
orphaned-process gaps.

**Status:** in-progress *(opened 2026-11-24; seven planned phases)*

**Depends on:** M50.

**Why:** [Issue #18](https://github.com/ryanczak/rexyMCP/issues/18) and
[Issue #19](https://github.com/ryanczak/rexyMCP/issues/19) (write-up:
`docs/dev/issues/2026-11-24-executor-never-batches-repeated-commands.md`).
Both come from local-model (GLM-5.3-Flash) runs looping on shell commands:
- In #19, a 300-run stress criterion produced 6 re-plumbed `cargo test` variants
  followed by 10 byte-identical calls and an abrupt `IdenticalToolCallRepetition`
  hard-fail on working code.
- In #18, a 4-command grep cycle (`grep … | wc -l` and `grep … | head -2`
  alternating) ran 403 edit-free turns to `max_turns = 1000` because
  `check_oscillation` exempts read-only windows, `LowNoveltyStall` is advisory,
  and neither strips trailing output plumbing. A second run spent 390 turns on
  one active task before hard-failing. Two companion runtime gaps surfaced in
  the same session: synchronous `continue_phase` hitting the 1800s MCP client
  timeout, and orphaned deadlocked `cargo test` children after `bash` timeouts.

**Exit criteria:**
- The embedded executor contract carries the batching rule with worked examples
  (`xargs -P`, `for` loop), pinned by a snapshot test, and the bash classifier
  allows both patterns plus `$TMPDIR` scripts.
- Re-plumbed bash commands (`| tail -N`, `| head -N`, `| wc -l`, `2>&1`,
  `> /tmp/x && cat /tmp/x`) and timing-only output differences normalize for
  nudges and cycle detection without altering `check_identical_repetition`'s
  exact-match hard-fail semantics.
- Consecutive repeated calls with identical normalized outputs inject one
  recoverable nudge at `repeat_nudge_threshold` (default `3`, `0` disables),
  emit `SessionEvent::NudgeInjected`, and record nudge counts in `PhaseRun`
  telemetry; `IdenticalToolCallRepetition` remains the backstop.
- A plumbing-normalized `ReadOnlyCycle` detector catches cyclic read-only/bash
  loops (nudging once, then terminating if unchanged) and is replayed by
  `rexymcp calibrate-governor`; `LowNoveltyStall` remains advisory by default.
- A per-task no-edit cap bounds edit-free turns while a task stays `active`,
  and `[budget] checkpoint_turns` (default `0`) provides an opt-in resumable
  checkpoint returning `BudgetExceeded` with a briefing.
- `continue_phase` runs asynchronously via `JobRegistry` (`{ run_id }`, polled
  with `get_run_status`, stoppable with `stop_phase`/`.rexymcp/stop`), matching
  `execute_phase`.
- `bash` and gate commands spawn in their own process group on Unix and signal
  the whole process group on timeout/cancellation so test children are never
  orphaned.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status, M51 entry.
- `docs/architecture.md` § Status, M48 entry (exact repetition on non-mutating
  windows, loud `read_only_stall_threshold = 0` refusal).
- `docs/architecture.md` § Status, M34 / M37 entries (`LowNoveltyStall` advisory
  default and `check_oscillation` read-only exemption).
- `docs/architecture.md` § Layer 2 (`execute_phase` async `run_id` contract).
- `docs/dev/issues/2026-11-24-executor-never-batches-repeated-commands.md`.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Contract batching rule & classifier coverage ([phase-01-contract-batching-rule.md](phase-01-contract-batching-rule.md)) | done |
| 02 | Shared command-plumbing & output-timing normalizer ([phase-02-command-output-normalization.md](phase-02-command-output-normalization.md)) | done |
| 03 | Recoverable repeat nudge + `NudgeInjected` event & telemetry ([phase-03-repeat-nudge.md](phase-03-repeat-nudge.md)) | done |
| 04 | Plumbing-normalized `ReadOnlyCycle` detector & calibration replay ([phase-04-read-only-cycle.md](phase-04-read-only-cycle.md)) | done |
| 05 | Per-task no-edit guard & optional soft checkpoint ([phase-05-task-guard-and-checkpoint.md](phase-05-task-guard-and-checkpoint.md)) | done |
| 06 | Async `continue_phase` (`run_id` + cancel + skill update) *(not yet drafted)* | todo |
| 07 | Process-group cleanup for `bash` and gate commands *(not yet drafted)* | todo |

## Notes

Design decisions, 2026-11-24 (user):

1. **Scope includes #18 related observations #1 and #2.** Async `continue_phase`
   (phase 06) and Unix process-group cleanup for `bash`/gate commands (phase 07)
   ship in M51 alongside the #19 and #18 governor work. (Related observation on
   transient network errors was checked in `executor/src/ai/mod.rs`:
   `send_with_retry_inner` already retries connect/send/5xx errors twice with
   2s+4s backoff, so no phase is scheduled for it.)
2. **New plumbing-normalized `ReadOnlyCycle` detector (not flipping
   `LowNoveltyStall` to `Terminate`).** `LowNoveltyStall` strips arbitrary
   `read_file` line ranges and `search` patterns, which false-fired on legit
   multi-chunk file reading (M34). A dedicated `ReadOnlyCycle` detector
   normalizes trailing bash output plumbing and quotes without collapsing
   distinct file slices or search queries, while `LowNoveltyStall` stays
   advisory by default.
3. **Soft checkpoint defaults to `0` (opt-in).** `[budget] checkpoint_turns = 0`
   leaves existing runs unchanged; when set to a positive turn count, reaching
   it terminates cleanly as `BudgetExceeded` with a briefing recommending
   `continue_phase`, avoiding a breaking change to `PhaseStatus`.
