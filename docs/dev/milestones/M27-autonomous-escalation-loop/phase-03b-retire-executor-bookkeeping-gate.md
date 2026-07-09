# Phase 03b: Retire the executor bookkeeping gate + amend the executor contract

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** done
**Depends on:** phase-03a (done)
**Estimated diff:** ~230 lines (mostly deletions)
**Tags:** language=rust, kind=refactor, size=m

## Goal

Server-authored bookkeeping, half 2 of 2 (D8/D9). Phase-03a **added** the
server-side finalize (Status flip + baseline Update Log entry + README-row flip +
separate `docs:` commit) but left it **dormant**: it no-ops whenever the phase
doc is already `**Status:** review`, which is exactly the state the executor
leaves it in today. This phase **activates** finalize by flipping authorship —
it retires the executor's pre-completion bookkeeping gate (`command::bookkeeping_feedback`
and its loop block) and amends the embedded executor contract so the executor
**stops authoring the completion tail** (the `in-progress → review` flip and the
`(complete)` Update Log entry). With the gate gone, a completed run leaves the
doc at `in-progress`, finalize sees `in-progress` + `Complete`, and writes the
bookkeeping itself. This deletes the M22-class failure where a MEDIUM-tier model
writes correct code and then dies in the bookkeeping tail.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/phase-03a-server-authored-finalize.md`
  — the finalize this phase activates. Read its § "Current state" and § Spec so
  you understand what the server now does and *why* the executor must leave the
  doc at `in-progress`.
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § Exit criteria,
  the bullet beginning "On a `complete` `execute_phase`, the **server** writes…"
  ("…the executor contract no longer instructs the executor to author them").
- `docs/architecture.md` § Status #27 — "server-authored bookkeeping (D8/D9)".

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The executor bookkeeping gate (to retire)

`executor/src/agent/command.rs` — the pure gate function (lines 182–233):

```rust
/// Bookkeeping gate: checks that the executor updated the phase doc's `**Status:**`
/// line and wrote at least one Update Log entry before declaring done. Re-reads the
/// phase doc from disk so in-session edits are visible. Returns `Some(msg)` when
/// either check fails; `None` when both pass or when the file cannot be read
/// (IO failures here are not a bookkeeping problem and should not block completion).
pub(super) fn bookkeeping_feedback(phase_doc_path: &std::path::Path) -> Option<String> {
    let content = match std::fs::read_to_string(phase_doc_path) {
        Ok(c) => c,
        Err(_) => return None,
    };
    // ... status_still_open + has_update_log_entry checks ...
}
```

It is consumed in **two** places in `executor/src/agent/mod.rs`:

1. The M22 stuck-gate-feedback **peek** chain (~lines 708–715), as the third
   `.or_else` arm:

```rust
                    let pending_gate_feedback =
                        command::gate_failure_feedback(&gates, &command_outputs)
                            .or_else(|| command::task_coverage_feedback(&seeded, &task_states))
                            .or_else(|| {
                                command::bookkeeping_feedback(std::path::Path::new(
                                    &input.phase_doc_path,
                                ))
                            });
```

2. The actual bookkeeping **gate block** (~lines 889–941), which sits after the
   task-coverage block and immediately before the "true completion" tail:

```rust
                    // Bookkeeping gate: phase doc status must be updated and
                    // Update Log must have at least one entry.
                    if let Some(feedback) =
                        command::bookkeeping_feedback(std::path::Path::new(&input.phase_doc_path))
                    {
                        // ... log Progress, push feedback, budget_exceeded at turn cap, continue ...
                    }
                    // All configured gates passed — this is a true completion.
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
                    ...
```

The gate is **inert in the existing test suite**: every integration test in
`executor/src/agent/tests.rs` uses `phase_doc_path =
"docs/dev/milestones/M0-test/phase-01-test.md"` (a relative path that does not
exist on disk), so `bookkeeping_feedback` reads → `Err` → returns `None`. That is
why the completion tests already reach `Complete` without flipping any status —
and why removing the gate does **not** change any existing test's outcome. This
phase adds a test that plants a **real** `in-progress` doc to prove the removal is
load-bearing.

### The bookkeeping tests (to delete)

`command.rs` `mod tests` contains six `bookkeeping_feedback_*` tests (lines
403–472) plus two helpers used **only** by them: `write_phase_doc` (384–388) and
the `GOOD_DOC` const (390–401). The `task_coverage_feedback_*` tests just above do
**not** use either helper. Deleting the six tests therefore also requires deleting
`write_phase_doc` and `GOOD_DOC`, or clippy's `dead_code` lint (`-D warnings`)
will fail on the now-unused helper and const.

### The stale doc comment

`executor/src/governor/hard_fail.rs` line ~185, on `check_repeated_gate_feedback`:

```rust
/// Stuck gate-feedback stall: the loop re-injected byte-identical gate feedback
/// (gate-retry / task-coverage / bookkeeping) `consecutive_repeats` times in a row
/// with no intervening state change.
```

The `/ bookkeeping` reference becomes inaccurate once the bookkeeping arm leaves
the peek chain.

### The executor contract (to amend)

`executor/templates/executor_contract.md` is embedded by the MCP server and
prepended to every phase's system prompt. Its § "Phase lifecycle — you own this"
currently instructs the executor to author the whole completion tail (steps 6–7:
write the `(complete)` entry, flip `in-progress → review`) and to commit "the
phase doc's status flip + Update Log additions." Task 5 rewrites that section and
the completion checklist so the executor stops authoring the completion tail.

## Spec

Numbered tasks in execution order.

### 1. Remove the bookkeeping gate block from the loop

In `executor/src/agent/mod.rs`, delete the entire bookkeeping gate block — the
comment `// Bookkeeping gate: phase doc status must be updated and` down through
its closing `}` and `continue;` (the `if let Some(feedback) =
command::bookkeeping_feedback(...) { … }` at ~889–941). Leave the following `//
All configured gates passed — this is a true completion.` line and everything
after it **byte-identical**. After this edit, the task-coverage block is
immediately followed by the true-completion tail.

### 2. Remove the bookkeeping arm from the peek chain

In `executor/src/agent/mod.rs`, in the `pending_gate_feedback` binding (~708),
delete the third `.or_else(|| { command::bookkeeping_feedback(...) })` arm so the
chain is:

```rust
                    let pending_gate_feedback =
                        command::gate_failure_feedback(&gates, &command_outputs)
                            .or_else(|| command::task_coverage_feedback(&seeded, &task_states));
```

This keeps the M22 stuck-gate-feedback peek symmetric with the two gate blocks
that remain (gate-retry, task-coverage). Do not touch the `match
&pending_gate_feedback { … }` body below it.

### 3. Delete `bookkeeping_feedback` and its tests

In `executor/src/agent/command.rs`:

- Delete the `bookkeeping_feedback` function and its doc comment (~182–233).
- Delete the six `bookkeeping_feedback_*` tests in `mod tests` (~403–472).
- Delete the now-orphaned `write_phase_doc` helper (~384–388) and the `GOOD_DOC`
  const (~390–401) — they are used only by those six tests (grep-confirm before
  and after: `grep -n "write_phase_doc\|GOOD_DOC" executor/src/agent/command.rs`
  must return **zero** matches after the edit).

Leave `task_coverage_feedback`, `gate_failure_feedback`, `run_post_write_hooks`,
`run_one`, `tail`, and all their tests untouched.

### 4. Fix the stale doc comment

In `executor/src/governor/hard_fail.rs`, change the parenthetical on
`check_repeated_gate_feedback` from `(gate-retry / task-coverage / bookkeeping)`
to `(gate-retry / task-coverage)`. No behavior change.

### 5. Amend the executor contract — flip authorship of the completion tail

In `executor/templates/executor_contract.md`, replace the **entire** `## Phase
lifecycle — you own this` section (from that heading through the line "If the last
two boxes aren't checked, you are **not** complete. Uncommitted work is invisible
to review.") with the text below. This is the load-bearing change: it is what
makes the executor stop authoring the completion tail so finalize activates.

**Replace this exact current block** (verbatim, including the nested checklist
fence):

````markdown
## Phase lifecycle — you own this

You keep phase status accurate. The reviewer evaluates based on what status says.

1. **Start:** flip the phase's `Status:` from `todo` (or `review` with bugs) to
   `in-progress`. Update the milestone README's phase table to match.
2. **Started entry:** append a progress entry to the phase's Update Log. Name
   yourself.
3. **Work:** implement the Spec tasks in order. Add progress entries when
   something surprising happens or you finish a chunk.
4. **Blocker:** if you cannot proceed, append a blocker entry and **stop**. Leave
   status `in-progress`.
5. **Verify:** every acceptance criterion ticked. Run the required verification
   commands.
6. **Complete:** append the completion entry with command output, files changed,
   commits, notes for review.
7. **Flip status:** `in-progress` → `review`. Update the README phase table.
8. **Run `{FORMAT_COMMAND}` before staging.** Your write tool does not guarantee
   formatted output. Running the formatter now prevents a lint/format gate failure
   that would otherwise require a re-dispatch just to fix whitespace or import order.
9. **`git commit` everything.** Stage all changes — source, tests, the phase
   doc's status flip + Update Log additions, the README status flip — and commit
   with a conventional-commit message. **Then run `git status` and confirm the
   working tree is clean.** A dirty tree at "completion" is not complete.
10. **Stop.** Do not start the next phase. Do not "while you're at it" anything.

The Update Log is **append-only**. Never edit prior entries.

**Completion checklist** (run through it before reporting complete):

```
[ ] Phase doc's Status: line says `review`.
[ ] Milestone README's phase table row says `review`.
[ ] Update Log has a "(complete)" entry with all required fields filled in.
[ ] `{FORMAT_COMMAND}` was run immediately before `git add` (not just checked — actually run).
[ ] All verification commands ran clean; output pasted in the Update Log.
[ ] Complete entry includes a one-line verification summary naming each gate.
[ ] End-to-end verification section filled in (per phase doc) OR declared N/A with reason.
[ ] `git status --short` shows nothing — every change is committed.
[ ] `git log -1 --stat` shows the commit includes every file you touched.
```

If the last two boxes aren't checked, you are **not** complete. Uncommitted work
is invisible to review.
````

**With this exact new block:**

````markdown
## Phase lifecycle — what you own, what the server writes

You own the **start** of the phase and all of the **code**. The rexyMCP server
authors the **completion bookkeeping** itself — the `in-progress → review` status
flip and the completion Update Log entry — from data it already holds, and
commits it as a separate `docs:` commit after your run returns. This split exists
because a headless model reliably writes correct code but is unreliable at the
multi-field completion tail; the server does that part so you don't stall on it.

1. **Start:** flip the phase's `Status:` from `todo` (or `review` with bugs) to
   `in-progress`, and update the milestone README's phase-table row to match.
   **Leave the status at `in-progress`. Do NOT flip it to `review` yourself, and
   do NOT write a `(complete)` Update Log entry.** The server flips the status to
   `review` and writes the completion entry when it finalizes your run. If you
   flip it to `review` early, the server sees an already-finalized doc, writes
   nothing, and you are back to owning the bookkeeping tail this split removes.
2. **Started entry:** append **one** progress entry to the phase's Update Log
   naming yourself. This is your attribution in the doc and the only Update Log
   entry you write.
3. **Work:** implement the Spec tasks in order. Add a progress entry only when
   something surprising happens.
4. **Blocker:** if you cannot proceed, append a blocker entry and **stop**. Leave
   status `in-progress`.
5. **Verify:** confirm every acceptance criterion is met, and run the required
   verification commands (`{FORMAT_COMMAND}`, `{BUILD_COMMAND}`, `{LINT_COMMAND}`,
   `{TEST_COMMAND}`) and confirm they pass. The loop re-runs them as the final
   gate set; a failing gate sends the feedback back to you to fix.
6. **Run `{FORMAT_COMMAND}` before staging.** Your write tool does not guarantee
   formatted output; running the formatter now prevents a format-gate failure
   that would otherwise require a re-dispatch just to fix whitespace or import order.
7. **`git commit` your code.** Stage and commit your source, tests, and the
   start-of-phase status flip from step 1 (a plain "stage everything" is fine —
   the only doc change present at this point is that start flip and your started
   entry). Use a conventional-commit message. Do **not** flip the status to
   `review` and do **not** write a `(complete)` entry — the server authors and
   commits both, separately, after your run. Then run `git status` and confirm
   the working tree is clean.
8. **Signal completion with a Summary.** Your final message — the turn with no
   tool call that signals you are done — must be a concise **Summary + Notes for
   review** (2–6 sentences): what you built, any deviation from the spec and why,
   the result of the phase's end-to-end verification, and anything the reviewer
   should know. The server captures this message verbatim and splices it into the
   completion entry it writes, as the **Summary**. This is how your qualitative
   account reaches the reviewer now that you no longer hand-write the entry — so
   make it substantive; do not make it a bare "done."
9. **Stop.** Do not start the next phase. Do not "while you're at it" anything.

The Update Log is **append-only**. Never edit prior entries.

**Completion checklist** (run through it before reporting complete):

```
[ ] Phase doc's Status: line still says `in-progress` (you flipped it there at start; the server flips it to `review`).
[ ] Milestone README's phase-table row still says `in-progress`.
[ ] You did NOT hand-write a `(complete)` Update Log entry — the server writes it from your final message.
[ ] `{FORMAT_COMMAND}` was run immediately before `git add` (not just checked — actually run).
[ ] All verification commands ran clean.
[ ] Your final message is a substantive Summary + Notes for review (what you built, deviations, E2E result).
[ ] `git status --short` shows nothing — every change is committed.
[ ] `git log -1 --stat` shows the commit includes every file you touched.
```

If the last two boxes aren't checked, you are **not** complete. Uncommitted work
is invisible to review.
````

Leave every other section of the contract (What you already have, Begin
immediately, Hard rules, Error handling, Testing, Comments, Commits, When you're
stuck, Source of truth precedence) **unchanged**.

### 6. Add the two activation tests

**6a. Executor-loop test** in `executor/src/agent/tests.rs` — prove a completion
with a real `in-progress` doc is no longer blocked. Plant a real phase doc at
`**Status:** in-progress` in the `TempDir`, point `phase_doc_path` at its absolute
path, script a single completing turn, and assert `Complete` in one turn. Mirror
the shape of `tool_call_then_no_tool_call_completes` (tests.rs ~251) for the
harness, overriding only `phase_doc_path`:

```rust
#[tokio::test]
async fn completes_without_flipping_status_now_that_gate_is_gone() {
    let dir = TempDir::new().unwrap();
    let scope = Scope::new(dir.path()).unwrap();
    let registry = registry_over(scope);
    let phase_doc = dir.path().join("phase-01-test.md");
    std::fs::write(
        &phase_doc,
        "# Phase 01: Test\n\n**Status:** in-progress\n\n## Update Log\n\n### Update — 2026-01-01 (started)\n",
    )
    .unwrap();
    let inp = PhaseInput {
        phase_doc_path: phase_doc.to_string_lossy().into_owned(),
        ..input()
    };
    let client = MockAiClientScript::new(vec![vec![token("all done")]]);
    let budget = Budget::new(1_000_000);

    let result = execute_phase(&inp, deps(&client, &registry, &budget, 8, dir.path()))
        .await
        .unwrap();

    assert_eq!(result.status, PhaseStatus::Complete);
    assert_eq!(
        client.calls().len(),
        1,
        "an in-progress doc must not trigger a bookkeeping re-loop"
    );
}
```

The `calls().len() == 1` assertion is the mutation catcher: if the bookkeeping
gate is ever restored, the `in-progress` status forces a second turn (the gate
re-injects feedback), so this pins the removal.

**6b. Server-activation end-to-end test** in `mcp/src/runner.rs` `mod tests` —
prove that with the gate gone, `run_phase_with` completes an `in-progress` doc and
**finalize flips it to `review`**. Mirror `run_phase_with_assembles_and_returns_result`
(runner.rs ~553) exactly, changing only the fixture doc's status to
`**Status:** in-progress` and adding the post-run doc assertions:

```rust
#[tokio::test]
async fn run_phase_with_finalizes_an_in_progress_doc_to_review() {
    // ... identical harness to run_phase_with_assembles_and_returns_result,
    //     except the fixture doc's Status line is `**Status:** in-progress` ...
    let result = run_phase_with(&inp, &seams).await;
    let phase_result = result.unwrap();
    assert_eq!(
        phase_result.status,
        rexymcp_executor::phase::PhaseStatus::Complete
    );
    let doc_after = std::fs::read_to_string(&phase_doc_path).unwrap();
    assert!(
        doc_after.contains("**Status:** review"),
        "finalize must flip the completed in-progress doc to review: {doc_after}"
    );
    assert!(
        doc_after.contains("(complete, server-authored)"),
        "finalize must append the server-authored completion entry: {doc_after}"
    );
}
```

`NoopRunner` (already used by that test) satisfies finalize's `git` calls (their
failures are swallowed best-effort); the Status flip + entry append are plain
`std::fs` writes, so the on-disk assertions hold regardless of git. This is the
definitive proof that phase-03a's finalize is now active.

## Acceptance criteria

- [ ] `command::bookkeeping_feedback` is gone; `grep -rn "bookkeeping_feedback"
      executor/ mcp/` returns **zero** matches.
- [ ] `grep -n "write_phase_doc\|GOOD_DOC" executor/src/agent/command.rs` returns
      zero matches (orphaned test helpers removed).
- [ ] The `pending_gate_feedback` peek chain in `mod.rs` has exactly two arms
      (`gate_failure_feedback`, `task_coverage_feedback`); the bookkeeping arm is
      removed.
- [ ] The bookkeeping gate block between the task-coverage block and the
      true-completion tail in `mod.rs` is removed; the true-completion tail is
      byte-identical.
- [ ] `hard_fail.rs` `check_repeated_gate_feedback` doc comment reads
      `(gate-retry / task-coverage)` (no `bookkeeping`).
- [ ] `executor_contract.md` § "Phase lifecycle" is replaced per Task 5: it
      instructs the executor to leave status at `in-progress`, not write a
      `(complete)` entry, and put its Summary/Notes in the final message; the
      completion checklist no longer requires a `review` status or a hand-written
      completion entry.
- [ ] New test `completes_without_flipping_status_now_that_gate_is_gone` passes
      and asserts `Complete` in one turn on a real `in-progress` doc.
- [ ] New test `run_phase_with_finalizes_an_in_progress_doc_to_review` passes:
      the on-disk doc reads `**Status:** review` and contains
      `(complete, server-authored)` after the run.
- [ ] All pre-existing executor and mcp tests pass unchanged (the gate was inert
      in them).
- [ ] `cargo build` zero new warnings; `cargo clippy --all-targets --all-features
      -- -D warnings` passes; `cargo fmt --all --check` passes; `cargo test`
      passes.

## Test plan

- `completes_without_flipping_status_now_that_gate_is_gone` (executor
  `agent/tests.rs`) — real `in-progress` doc + one completing turn → `Complete`,
  `calls().len() == 1`.
- `run_phase_with_finalizes_an_in_progress_doc_to_review` (mcp `runner.rs`) — an
  `in-progress` fixture doc through `run_phase_with` → `Complete` and the doc on
  disk becomes `review` with a `(complete, server-authored)` entry.
- Deletion of the six `bookkeeping_feedback_*` unit tests is expected and correct
  (their subject no longer exists); do not port them.

## End-to-end verification

`run_phase_with_finalizes_an_in_progress_doc_to_review` **is** the end-to-end
verification: it drives the real `run_phase_with` assembler (the same path the MCP
`execute_phase` tool and the CLI `run-phase` funnel through) against a real
on-disk phase doc, and asserts the real file was finalized by the server. Quote
its output in the completion Summary. Additionally, paste the two greps proving
`bookkeeping_feedback` and its helpers are gone.

## Authorizations

- [ ] May edit `executor/templates/executor_contract.md` (specifically the
      "Phase lifecycle — you own this" section and its completion checklist, per
      Task 5).

No new dependency, no `Cargo.toml` edit, no `architecture.md` / `STANDARDS.md` /
`WORKFLOW.md` edit, no edit to `mcp/src/finalize.rs` (03a's finalize is complete
and correct — this phase only activates it by removing the executor gate).

## Out of scope

- **The start-of-phase flip (`todo → in-progress`) stays with the executor.**
  Do not move it to the server and do not touch `mcp/src/finalize.rs`. finalize
  keys off the `in-progress` status as its activation-and-idempotency signal
  (03a, § Out of scope); that contract is unchanged. If the executor ever fails
  to make the start flip, finalize correctly no-ops on the resulting `todo` doc —
  a degraded-but-safe outcome the reviewer catches, not something to guard here.
- **Widening finalize to act on a `todo` doc.** That would reopen 03a's design;
  it is not this phase.
- **`mcp/src/finalize.rs`, `PhaseResult.completion_summary`, and the loop's
  populate-on-complete site** — all landed in 03a. Leave them alone.
- **The `docs:` commit message / entry format** — pinned in 03a. Not this phase.
- **STANDARDS.md / WORKFLOW.md prose about the executor authoring bookkeeping.**
  Those describe the *architect/executor workflow* generally (and the plugin
  templates mirror them); reconciling them with server-authored bookkeeping is a
  separate docs pass, not this refactor. Note any divergence you spot in "Notes
  for review"; do not edit those files.

## Gotchas (pre-injected)

- **The contract amendment is the load-bearing change, not the code deletion.**
  Removing the Rust gate lets a completed run *reach* finalize at `in-progress`;
  the contract edit is what stops the executor from pre-flipping to `review` (which
  would make finalize no-op). If you only delete the gate but leave the contract
  telling the executor to flip to `review`, finalize stays dormant and the phase
  achieves nothing. Both halves are required.
- **Graceful degradation is intended.** If a future executor ignores the amended
  contract and flips to `review` anyway, the doc reaches finalize at `review` →
  finalize no-ops → behavior falls back to executor-authored bookkeeping. That is
  a safety net, not a bug — but it is also why the contract wording must be
  forceful: the in-repo phase docs all show executor-authored `(complete)` entries,
  so imitation pressure is real and the contract must clearly override it.
- **Delete the orphaned test helpers.** `write_phase_doc` + `GOOD_DOC` are used
  only by the deleted bookkeeping tests; leaving them tripwires clippy `dead_code`
  under `-D warnings`. Grep to confirm zero references after the edit.
- **Keep the true-completion tail byte-identical.** When you remove the gate block
  in `mod.rs`, do not disturb the `// All configured gates passed …` block that
  follows it (it holds the 03a `artifacts.completion_summary =
  strip_think_blocks(&completion)` line). `command_outputs` is not consumed by the
  gate block, so removing it leaves the tail's `build_artifacts(..., command_outputs)`
  move intact.
- **Nested fence when editing the contract.** The contract's completion checklist
  is inside a ```` ``` ```` fenced block. Replace the whole section as one edit;
  don't lose the inner fence.
- **This is a refactor: existing behavior for existing tests is unchanged.** The
  gate was inert in every test (non-existent `phase_doc_path`), so no pre-existing
  test outcome flips. Only the two new tests exercise the new behavior. If a
  pre-existing test breaks, stop and file a blocker — something else is wrong.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-09 00:33 (started)

**Executor:** rexyMCP executor (phase-03b)

### Update — 2026-07-09 00:33 (complete)

**Summary:** Retired the executor bookkeeping gate and amended the executor contract to flip authorship of the completion tail to the server. Removed `command::bookkeeping_feedback` and its 6 tests plus orphaned helpers from `command.rs`, deleted the bookkeeping gate block and peek arm from `mod.rs`, fixed the stale doc comment in `hard_fail.rs`, and replaced the entire "Phase lifecycle" section in `executor_contract.md` to instruct the executor to leave status at `in-progress` (the server now flips to `review` and writes the completion entry). Added two activation tests: `completes_without_flipping_status_now_that_gate_is_gone` (executor loop) and `run_phase_with_finalizes_an_in_progress_doc_to_review` (MCP runner end-to-end).

**Acceptance criteria:** All met. `bookkeeping_feedback` is gone (grep confirms zero matches). `write_phase_doc`/`GOOD_DOC` removed. Peek chain has exactly two arms. Bookkeeping gate block removed; true-completion tail byte-identical. Doc comment fixed. Contract amended per Task 5. Both new tests pass. All 915 pre-existing tests pass unchanged.

**Commands output:**
- `cargo fmt --all --check`: passed
- `cargo build`: passed, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings`: passed
- `cargo test`: 915 passed, 0 failed, 2 ignored

**End-to-end verification:** `run_phase_with_finalizes_an_in_progress_doc_to_review` passed — the on-disk doc reads `**Status:** review` and contains `(complete, server-authored)` after the run, proving finalize is now active. Grep confirms `bookkeeping_feedback` and its helpers are gone from the codebase.

**Files changed:** executor/src/agent/mod.rs, executor/src/agent/command.rs, executor/src/agent/contract.rs, executor/src/agent/tests.rs, executor/src/governor/hard_fail.rs, executor/templates/executor_contract.md, mcp/src/runner.rs, phase doc, milestone README

**New tests:** `completes_without_flipping_status_now_that_gate_is_gone`, `run_phase_with_finalizes_an_in_progress_doc_to_review`

**Commits:** `5d35df2` refactor: retire executor bookkeeping gate and amend the contract

**Notes for review:** The contract sentinel count test (`unset_command_renders_as_unconfigured_sentinel`) required updating from 5 to 8 sentinels because the amended contract section uses `{FORMAT_COMMAND}` 4× (vs 3×), `{LINT_COMMAND}` 2× (vs 1×), and `{TEST_COMMAND}` 2× (vs 1×) — these are the new explicit command references in the amended lifecycle steps.

### Review verdict — 2026-07-09

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — `docs/dev/NEXT.md` was already dirty pre-dispatch (architect's prior drafting session had advanced the pointer to 03b) and was swept into the commit by the contract's "stage everything" guidance, not an out-of-scope edit by the executor.
- **Calibration:** none. Independent re-run confirms all four gates green (executor crate 915 passed / 2 ignored, mcp crate 459 passed / 0 ignored). Verified byte-for-byte that the `executor_contract.md` § "Phase lifecycle" replacement matches the phase doc's pre-injected block exactly (programmatic diff, zero output). Confirmed the true-completion tail in `mod.rs` is untouched (including the 03a `completion_summary` line), the peek chain has exactly two arms, and both new tests fail-fast in isolation (ran each individually). Test-count arithmetic reconciles: 920 (phase-03a) − 6 deleted bookkeeping tests + 1 new executor-loop test = 915.
