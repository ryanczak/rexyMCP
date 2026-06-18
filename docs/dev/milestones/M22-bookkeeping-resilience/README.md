# M22 — Bookkeeping-Loop Resilience

**Goal:** Make the executor's completion/bookkeeping tail resilient to the four
loop-failure modes diagnosed in the netviz end-to-end run — empty-output death
spirals, stuck gate-feedback loops, bogus task seeding, and self-reverts — so a
model that writes correct code can reliably *finish* the phase instead of burning
the turn budget or hard-failing in the bookkeeping step.

**Status:** done

**Depends on:** M19 (gate-retry loop in the `NoToolCall` arm), M21 (task-coverage
gate — we harden the same loop), M16 (seeder robustness — we tighten the same
`seed_from_spec`)

## Why now

A live e2e run drove the `google/gemma-4-26b-a4b-qat` (MEDIUM-tier) executor
through netviz phases 01–04. The model produced **correct code every time** — all
four gates were green before trouble started in every failing session — but it
could not reliably complete the bookkeeping loop. Three distinct mechanisms,
each invisible to the existing guardrails, recurred across phases 02/03/04
(sessions in `/home/matt/src/netviz/.rexymcp/sessions/`):

1. **Empty-output death spiral** (`session-phase-04-6a32f806`, budget_exceeded@200).
   After a `write_file` null-args error, the model emitted **147 consecutive empty
   completions** (`raw: ""`). The `NoToolCall` guard at `mod.rs:516` only catches
   a `<think>`-only completion (it requires `completion.contains("</think>")`), so
   a blank string falls through and is treated as a *completion attempt* — it
   re-runs the gates, trips `task_coverage_retry`, re-injects the identical
   feedback, and loops. The governor's `IdenticalToolCallRepetition` stall is
   blind to it because empty completions never produce a tool call, so
   `recent_tool_calls` never grows. The run burned to the 200-turn cap.

2. **Bogus task seeding → update_task fixation** (`session-phase-04-6a32f237`,
   hard_fail@26). `seed_from_spec` greedily parsed a **prose numbered algorithm**
   in `## Spec` (`1. If packet.tcp…`, `2. …`) as tasks, producing two tasks with
   **byte-identical truncated titles**. The model couldn't tell them apart and
   re-marked task 1 repeatedly until `IdenticalToolCallRepetition` hard-failed —
   after 8 wasted turns and an unhelpful "identical update_task ×6" briefing.

3. **Self-revert** (same session). The model ran `git checkout
   src/flow-table.test.ts` — discarding its own green edit — because a test was
   red. `bash_classify` already blocks the wholesale forms (`git reset --hard`,
   `git checkout .`) but not a single-file `git checkout <path>` of a file the
   executor edited this session.

These are structural gaps a phase-spec instruction cannot close — the same class
M19/M21 closed for `false_completion`. M22 closes them in the executor.

## Exit criteria

- A run of ≥ N consecutive empty/blank model completions terminates as
  `hard_fail` with a named signal, not `budget_exceeded` at the turn cap.
- A truly-empty completion is routed to recoverable "emit a tool call" feedback,
  not treated as a completion attempt.
- The same gate feedback re-injected ≥ K times with no intervening state change
  terminates as `hard_fail`, not an unbounded loop.
- `seed_from_spec` does **not** seed prose numbered list items (algorithm steps);
  it seeds only convention-conforming task items, and never two tasks with the
  same id or the same title.
- The `update_task` tool result names the still-incomplete task ids and flags a
  redundant re-mark, so the model can self-correct instead of refixating.
- A `git checkout`/`git restore` of a file the executor edited this session is
  refused with a model-visible advisory naming the file.
- All pre-existing tests pass unmodified (every change is additive / backward-
  compatible).

## Architecture references

- `docs/architecture.md` § Status — M19 (gate enforcement), M21 (task-coverage
  gate); this milestone extends the same turn-cycle seam.
- `executor/src/agent/mod.rs` — the `NoToolCall` completion arm (parse-failure
  branch ~510–569; gate blocks ~578–697; Step 7 hard-fail emission ~1062–1103;
  pre-dispatch refusal seam ~832).
- `executor/src/governor/hard_fail.rs` — `HardFailSignal`, `evaluate`, the
  per-signal pure checks; the model for the new empty-completion stall.
- `executor/src/agent/tasks.rs` — `seed_from_spec` / `parse_task_line` /
  `extract_title`.
- `executor/src/tools/update_task.rs` — the tool result the model reads.
- `executor/src/agent/tools.rs` — `read_before_edit_refusal`, the model for the
  self-revert refusal; `pre_edit_content` (the edited-this-session set).
- `executor/src/security/bash_classify.rs` — the existing git blocklist C7
  complements.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Empty-completion routing + governor stall (A1+A2) ([phase-01-empty-completion-stall.md](phase-01-empty-completion-stall.md)) | done |
| 02 | Stuck gate-feedback stall (A3) ([phase-02-stuck-gate-feedback-stall.md](phase-02-stuck-gate-feedback-stall.md)) | done |
| 03 | Seeder precision: bold-name tasks + de-dup (B4+B5) ([phase-03-seeder-precision.md](phase-03-seeder-precision.md)) | done |
| 04 | update_task result echoes remaining ids (B6) ([phase-04-coverage-feedback-echo.md](phase-04-coverage-feedback-echo.md)) | done |
| 05 | Self-revert guard (C7) ([phase-05-self-revert-guard.md](phase-05-self-revert-guard.md)) | done |

Dispatch in order and review-gate each. Phases 01 and 02 both edit the
`NoToolCall` arm of `mod.rs`; 02's anchors are written against pre-M22 code, so
after 01 lands, re-locate 02's insertion point by the quoted anchor text, not the
line number. Phases 03 (`tasks.rs`), 04 (`update_task.rs`), and 05
(`tools.rs`/`mod.rs`/`bash`) are independent of 01/02 and of each other.

## Notes

### Scope decisions (2026-06-17, with the user)

- **A1+A2 are one phase** (phase-01): routing the empty completion to a nudge and
  adding the governor stall are the detect/correct/terminate halves of the same
  spiral — splitting them would ship a phase that either nudges-but-still-loops
  (A1 alone) or hard-fails-without-trying-to-recover (A2 alone).
- **A3 (phase-02) is the general guard**; A1+A2 already fix the *observed* 147×
  spiral (it was empty-completion-driven). A3 catches a stuck *non-empty* gate
  loop that isn't an identical tool call.
- **B4 keys off the documented bold-name convention.** `WORKFLOW.md` § Spec
  already specifies the list-item task format as `N. **<Task name>** — …`. The
  prose algorithm steps that mis-seeded (`1. If packet.tcp…`) have **no bold
  name**; the real tasks (`1. **"FlowTable creates…"**`) do. Requiring a bold span
  for the `N.` list form excludes the prose and — because bold names are
  single-line and complete — eliminates the mid-word title truncation too. The
  `### N.` / `### Task N` heading forms are unchanged.
- **D8/D9 (pre-filled / server-authored bookkeeping) are deferred** — to be
  discussed before authoring. M22 makes the executor's *own* bookkeeping loop
  survivable; D8/D9 would change *who authors* the bookkeeping and touches the
  executor contract.

### Retrospective — 2026-06-18

**Outcome: 5/5 phases `approved_first_try`, zero bounces.** The cleanest
milestone to date. Each phase closed one diagnosed loop-failure mechanism from
the netviz e2e run:

| # | Mechanism | Fix | Executor | Turns |
|---|---|---|---|---|
| 01 | Empty-output death spiral (147× blank completions) | `NoToolCall` empty-guard broadened to route a blank `raw:""` to the recovery nudge + governor `EmptyCompletionStall` (consecutive-empty counter → `hard_fail`, default 3) | Qwen/Qwen3.6-27B-FP8 | 98 |
| 02 | Stuck non-empty gate-feedback loop | additive peek-guard above the 3 gate blocks: same feedback re-injected ≥ K=5× with no state change → `StuckGateFeedback` hard_fail | Qwen/Qwen3.6-27B-FP8 | 121 |
| 03 | Bogus prose-algorithm seeding + identical-title fixation | `N.` list form requires a `**bold**` name (matches the documented Spec convention; excludes prose steps, kills title truncation) + de-dup seeded tasks by id & title | Qwen/Qwen3.6-27B-FP8 | 64 |
| 04 | `update_task` refixation | tool result echoes still-incomplete ids (seeded order, inside the lock) + flags a redundant re-mark; metadata shape unchanged | claude-code (direct) | — |
| 05 | Self-revert (`git checkout <edited-file>`) | working-set-aware `destructive_restore_refusal` mirroring `read_before_edit_refusal`, chained `.or_else()` at the refusal seam; complements the stateless `bash_classify` blocklist | Qwen/Qwen3.6-27B-FP8 | 84 |

**Why it went clean — what to keep doing:**

- **All five fixes were additive.** New governor signals, a new pure refusal
  fn, an appended tool-result clause, a tightened parser predicate — every
  change kept the codebase compiling at each step and let all pre-existing tests
  pass unmodified (phase-03's 3 intentional test updates were the one exception,
  and that fork was resolved with the user up front). The "prefer additive change
  shapes" discipline (WORKFLOW.md) is doing exactly what it's for.
- **Worked examples quoted inline carried the load.** Phases 02/05 quoted the
  exact seam to mirror (`gate_failure_feedback`, `read_before_edit_refusal`) in a
  fenced block with the structure to replicate — the executor pattern-matched
  rather than implemented-from-scratch. This is the highest-leverage
  pre-injection and it showed.

**Calibration data (no folds warranted):**

1. **Anchor refresh at activation works.** Phase-05 was drafted up front against
   pre-M22 `mod.rs`; phases 01/02 shifted its refusal seam from `~832` to line
   966. At activation the architect re-verified by the quoted anchor *text* (not
   the line number), found the structure intact, and corrected the stale refs
   before dispatch — clean landing. This is the existing "re-locate by anchor
   text, not line number" guidance paying off; no new fold.
2. **Identity self-stamp quirk persists (cosmetic).** Phase-05's Update Log
   self-stamps "Claude (sonnet-4-5-20250514)" though the executor was
   Qwen/Qwen3.6-27B-FP8 (dispatch ran against `http://brain:8000/v1`). The
   **date** is correct (`2026-06-18`). Long-recurring, cosmetic only, machine
   records (telemetry, `executor_health`) correct. No fold.
3. **Mid-milestone NEXT.md "staleness" is by-design, not a defect.** Phase-04's
   approve commit correctly left the NEXT.md pointer for `/rexymcp:architect
   next` to advance (review skill §7d: only the *last* in-scope phase sets NEXT
   to "none"; mid-milestone advances are the architect-next step's job). The
   pointer reading "phase-04 active" between approval and the next `architect
   next` is the intended gate window. Re-flagged here only to retire the earlier
   over-flag — no process change needed.

**M22 retrospective: no new code-quality or spec-writing patterns; no folds to
STANDARDS.md or WORKFLOW.md.**

**Deferred, awaiting a design conversation:** D8/D9 (pre-filled / server-authored
bookkeeping — rexyMCP writing the Status flip + a baseline Update Log entry
itself). M22 made the executor's *own* bookkeeping loop survivable; D8/D9 would
move *who authors* the bookkeeping from executor to server and touches the
executor contract — a talk-through, not a quiet next milestone.
