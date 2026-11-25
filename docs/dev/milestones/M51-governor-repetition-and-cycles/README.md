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
| 06 | Async `continue_phase` (`run_id` + cancel + skill update) ([phase-06-async-continue-phase.md](phase-06-async-continue-phase.md)) | done |
| 07 | Process-group cleanup for `bash` and gate commands ([phase-07-process-group-cleanup.md](phase-07-process-group-cleanup.md)) | done |

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

## M51 retrospective — closed 2026-11-24

**Outcome:** [Issue #18](https://github.com/ryanczak/rexyMCP/issues/18) and
[Issue #19](https://github.com/ryanczak/rexyMCP/issues/19) — including both
companion runtime observations from #18 — are implemented across all seven
planned phases, with zero escalations, zero architect takeovers, and zero
hard-fails. First full milestone driven end-to-end with Google Antigravity
acting as Architect through the bundled `rexymcp-plugin` MCP server (`8731480`).

| Phase | Verdict | Executor turns (`GLM-5.3-Flash`) | Notes |
|---|---|---|---|
| 01 Contract batching rule & classifier coverage | `approved_first_try` | 56 | |
| 02 Shared command-plumbing & output-timing normalizer | `approved_after_1` | 122 + 62 | `bug-02-1`: regex boundary / non-temp stripping + prod `.unwrap()` |
| 03 Recoverable repeat nudge + `NudgeInjected` & telemetry | `approved_after_1` | 196 + 88 | `bug-03-1`: fake `repeat_nudge_silent_when_outputs_differ` test |
| 04 Plumbing-normalized `ReadOnlyCycle` & calibration replay | `approved_first_try` | 210 | |
| 05 Per-task no-edit guard & optional soft checkpoint | `approved_first_try` | 255 | |
| 06 Async `continue_phase` (`run_id` + cancel + skill update) | `approved_first_try` | 187 | |
| 07 Process-group cleanup for `bash` and gate commands | `approved_first_try` | 72 | |

**Totals:** 7/7 `done` (5 `approved_first_try`, 2 `approved_after_1`; 2 bugs
filed and verified on round 2), 1,248 executor turns across 9 runs, all four
gates green (`712 + 2 + 1148 = 1862` passing tests, 0 failed, 0 warnings).

**What worked:**
- Short, behavior-pinned phase docs naming exact analogue symbols and negative
  cases (especially empirically checking Linux `kill -9 -- -<pgid>` and
  container `Z`-state zombie behavior before drafting phase-07, which landed
  first try in 72 turns).
- Reviewing test diffs for mutation validity caught `bug-03-1` (a negative test
  varying the command string instead of issuing identical calls) before merge,
  and the refreshed failing-criterion bounce sequence fixed both `bug-02-1` and
  `bug-03-1` cleanly in a single round-2 dispatch.
- Authorizing `mcp/src/init.rs`, `mcp/src/runner.rs`, and `README.md` upfront
  on every phase adding a `GovernorConfig` / `ModelOverride` field eliminated
  compiler-forced scope deviations in phases 04 and 05.

**Calibration, for human sign-off (held — no unilateral contract/runtime folds):**
1. **Leaked `</think>` fragment in `completion_summary` (6 occurrences: M50
   phase-03 + M51 phases 01, 02, 03, 05, 06 — past 3× threshold; runtime fix
   candidate):** `GLM-5.3-Flash` emits an unmatched closing `</think>` when the
   opening tag is template-injected, leaving `</think>Done. Final summary.</think>`
   ahead of `**Summary:**`. Candidate fix in `strip_think_blocks`: strip up
   through the last unmatched `</think>` when no preceding `<think>` exists.
2. **Duplicated `**Summary:**` prefix in server-authored completion entries (3
   occurrences in M51: phase-03 round 2, phase-04, phase-07 — at 3× threshold;
   runtime fix candidate):** the server prepends `**Summary:** ` onto a model
   completion already starting with `**Summary:**`. Candidate fix: strip a
   leading `**Summary:**` / `**Summary**` heading before formatting.
3. **Executor self-misreporting model identity in started Update Log entries
   (recurred in phase-07 as `Executor: Claude (executor run)` — 5th+ occurrence
   across M43, M44, M48, M50, M51):** telemetry is unaffected because the
   server-authored completion tail writes the true model. Candidate `WORKFLOW.md`
   fold: drop `**Executor:**` from executor-authored Update Log templates so the
   server owns that field exclusively.

**Action items for the human:**
- Reinstall the CLI binary (`cargo install --path mcp`) and restart any running
  `rexymcp serve` so the live MCP server picks up M51's new governor detectors,
  async `continue_phase`, and Unix process-group cleanup.
- Close GitHub issues [#18](https://github.com/ryanczak/rexyMCP/issues/18) and
  [#19](https://github.com/ryanczak/rexyMCP/issues/19).
- Decide on the small runtime-fix follow-up (items 1 & 2 above), the
  `WORKFLOW.md` fold (item 3), and what milestone comes next.
