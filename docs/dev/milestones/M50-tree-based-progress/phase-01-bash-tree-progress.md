# Phase 01: Bash tree changes count as progress

**Milestone:** M50 — Tree-based progress
**Status:** done
**Depends on:** none
**Estimated diff:** ~250 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The governors decide "the model made file progress" from the tool name alone,
so a `bash` call that rewrites files never resets the no-progress stall run.
Runs editing through bash are hard-failed as `NoProgressStall` while their
edits are on disk. Make a `bash` call that changed the working tree count as
progress.

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` § Notes — the
  user's design decisions; this phase implements 1 and 2.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`check_read_only_stall` and `window_has_mutation` in
`executor/src/governor/hard_fail.rs` test each `ToolCallSnapshot` with
`tools::mutates_files` (`executor/src/tools/router.rs`), which is false for
`bash`. The agent loop (`executor/src/agent/mod.rs`) pushes one snapshot per
tool call and knows nothing about what a `bash` call did to disk.

## Spec

1. **Tree fingerprint** — new module `executor/src/agent/tree_fingerprint.rs`.
   A fingerprint of the project root records every non-ignored file's
   relative path, size and mtime. Comparing two fingerprints yields the
   relative paths added, removed or changed, sorted (phase 02 consumes this
   list, so return the paths, not just a bool). Best-effort: an entry that
   cannot be read is skipped; the walk never returns an error.
   Walk rules, each one a gotcha:
   - `.gitignore` must be honoured even when the root is **not** a git repo
     (tests run in a bare `TempDir`). `ignore`'s `WalkBuilder` honours
     gitignore files only inside a git repo unless told otherwise.
   - Hidden files count (an edit to `.github/workflows/ci.yml` is progress),
     but `WalkBuilder` skips hidden entries by default.
   - The `.git/` directory never counts. Pruning it during the walk, rather
     than filtering its files afterwards, keeps the walk cheap.

   `ignore` 0.4.26 (`Cargo.lock`), the builder methods involved:
   `WalkBuilder::new(root)`, `.hidden(bool)` ("enables ignoring hidden
   files", default `true`), `.require_git(bool)` ("whether a git repository
   is required to apply git-related ignore rules", default `true`),
   `.filter_entry(|e: &DirEntry| -> bool)` (an entry for which the predicate
   returns `false` is skipped, and a directory is not descended into).
2. **Snapshot carries the tree signal** — `ToolCallSnapshot` gains a
   `changed_tree: bool`, `#[serde(default)]` so a snapshot serialized before
   this phase still deserializes. Add a method on `ToolCallSnapshot` that is
   true when `mutates_files(tool)` or `changed_tree`. That method is now the
   one definition of "made file progress"; `check_read_only_stall` and
   `window_has_mutation` both use it. Update their doc comments, and the doc
   comment on `mutates_files`, which currently calls itself the single
   source of truth for file progress.
3. **The loop sets it** — in `executor/src/agent/mod.rs`, take a fingerprint
   immediately before a `bash` call is dispatched and another after it
   returns; `changed_tree` is true when they differ. Fingerprint only around
   `bash` calls that actually dispatch: not around other tools, and not
   around a `bash` call that was refused or deduplicated before dispatch
   (it ran nothing). Every other snapshot gets `false`.
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`. The
   server-authored `(complete)` entry does not satisfy this.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] `check_read_only_stall` and `window_has_mutation` contain no direct
      call to `mutates_files`; both go through the snapshot method.
- [ ] Preservation: every existing test in `executor/src/governor/` and
      `executor/src/agent/tests.rs` passes unmodified apart from the new
      field in snapshot literals.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

All hermetic: `TempDir` roots, no git binary, no network.

- `tree_fingerprint.rs`:
  - `detects_written_file` — a new file, and an existing file rewritten to a
    different size, each appear in the changed paths.
  - `detects_deleted_file` — a removed file appears in the changed paths.
  - `unchanged_tree_compares_equal` — two walks with no write between them
    yield no changed paths.
  - `gitignored_write_is_not_a_change` — with `target/` in a `.gitignore`
    and no `.git` directory, writing under `target/` yields no changed paths.
  - `dot_git_write_is_not_a_change` — writing under `.git/` yields no
    changed paths.
  - `hidden_file_write_is_a_change` — writing `.github/ci.yml` does.
- `hard_fail.rs`:
  - `read_only_stall_resets_on_tree_changing_bash` — a run of read-only
    calls interrupted by a `bash` snapshot with `changed_tree: true` counts
    only the calls after it.
  - `read_only_stall_counts_bash_without_tree_change` — the same run with
    `changed_tree: false` fires at threshold, as today.
  - `oscillation_applies_to_tree_changing_bash_window` — an A,B,A,B window
    of `bash` calls where at least one changed the tree is not exempt and
    fires; the same window with no tree change stays exempt.
  - `tool_call_snapshot_deserializes_without_changed_tree` — JSON with no
    `changed_tree` key deserializes with `false`.
- `agent/tests.rs` (driven by `MockAiClientScript`, real `bash` tool in a
  temp root). **How to set the threshold:** `run_with_verifier` uses
  `GovernorConfig::default()` (threshold far above a short script), so these
  two tests build `LoopDeps` explicitly with a `governor: GovernorConfig {
  read_only_stall_threshold: <small>, …, ..GovernorConfig::default() }`
  field, the way the M34 novelty tests in the same file already do (search
  `novelty_distinct_floor` for the analogue). In that config, neutralise the
  other detectors so only the stall can fire: give every scripted call
  distinct arguments (identical repetition), and set `oscillation_window: 0`
  and `novelty_distinct_floor: 0`. End the script with a final `token(…)`
  turn so the positive test terminates without a hard-fail.
  - `bash_edit_resets_read_only_stall` — a scripted run whose read-only call
    count would cross a small `read_only_stall_threshold`, except that a
    `bash` call writing a file sits in the middle, does not end in
    `NoProgressStall`.
  - `read_only_bash_still_trips_stall` — the same script with the middle
    `bash` call writing nothing ends in `NoProgressStall`.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-01.txt; : > "$A"
for t in tree_fingerprint read_only_stall_resets_on_tree_changing_bash read_only_stall_counts_bash_without_tree_change oscillation_applies_to_tree_changing_bash_window tool_call_snapshot_deserializes_without_changed_tree bash_edit_resets_read_only_stall read_only_bash_still_trips_stall; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count (a filter that matched no
test prints none).

## Authorizations

Files that hold a `ToolCallSnapshot { … }` literal or are otherwise touched:

- `executor/src/agent/tree_fingerprint.rs` (new), `executor/src/agent/mod.rs`,
  `executor/src/agent/tests.rs`
- `executor/src/governor/hard_fail.rs`
- `executor/src/phase/briefing.rs` (test literals only)
- `mcp/src/calibrate_governor.rs` (snapshot literals only: the replay sets
  `changed_tree: false`, see § Out of scope)
- `executor/src/tools/router.rs` (the `mutates_files` doc comment only)

No new dependencies (`ignore` is already one).

## Out of scope

- The `PhaseResult` diff, `files_changed` and the progress numstat — phase 02.
- The bash classifier. Bash-side writes stay allowed; do not extend
  `RefuseInPlaceEdit`.
- Fingerprinting around any tool other than `bash`.
- Governor thresholds and config.
- Recording `changed_tree` in the session log. `rexymcp calibrate-governor`
  replays snapshots from logs, so its replays stay tool-name-only for now;
  the replay passes `false`.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-10-26

Two prior runs both ended `NoProgressStall` at 120. Tasks 1–3 are **done and
verified green** by the architect (all four gates, 1064 executor tests) — do
not re-read or re-edit `tree_fingerprint.rs`, `hard_fail.rs`, `router.rs`,
`briefing.rs`, `calibrate_governor.rs` or the loop wiring in `agent/mod.rs`.
What remains is exactly: (a) the two `agent/tests.rs` loop tests
`bash_edit_resets_read_only_stall` and `read_only_bash_still_trips_stall`,
(b) the E2E capture, (c) the gates. The second run spent all 120 turns
looking for how to set a small stall threshold in a loop test; § Test plan
now names the mechanism. Write the first test within your first ~15 calls.
Make every edit with `patch` / `patch_lines` / `write_file`, never through
bash: the server running you still has the old governor, so bash-side edits
do not reset the stall counter. Finish condition: `cargo test -p
rexymcp-executor` reports **1066** passed (1064 + the two new tests).

### Update — 2026-10-26 12:56 (progress)

Started phase-01 (executor). Flipped status to `in-progress` and the milestone
README row to match. Reading the touched modules (`agent/mod.rs`,
`governor/hard_fail.rs`, `tools/router.rs`, `calibrate_governor.rs` snapshot
literals) before writing the `tree_fingerprint` module.

### Update — 2026-10-26 13:20 (progress)

Tree fingerprint, snapshot field and loop wiring in place; 1059 executor tests
green. Wiring notes:

- `fingerprint` builds the `WalkBuilder` as `let mut builder = …;` then calls
  the builder methods as statements — chaining `WalkBuilder::new(root).hidden(…)`
  as an expression borrows the temporary for the chain, and `build()` inside the
  same expression extended that borrow past its lifetime (E0716).
- One stray `ToolCallSnapshot` literal in `hard_fail.rs` tests (json! spread
  over multiple lines) was missed by the scripted field insertion and needed a
  hand patch, as did the single-literal `snap` helper in `briefing.rs` and the
  log-replay literal in `calibrate_governor.rs` (`changed_tree: false` per §
  Out of scope).
- While writing the scripted insertion I corrupted `hard_fail.rs` (a brace-
  depth scan ate field lines); rebuilt the file from git HEAD and re-applied
  the struct change + field insertions mechanically. Verified the diff is
  strictly additive against HEAD (55 insertions, 0 deletions beyond the struct).

Now adding the governor unit tests.

### Update — 2026-10-26 14:10 (escalation)

**Chosen lever:** resume
**Rationale:** `NoProgressStall` at 120 with the phase nearly finished — all four
gates green on the partial tree, 10 of 12 named tests present and passing — and
the stall is the very bug this phase fixes (the executor edited through scripted
bash, which the running server's old governor counts as read-only), followed by
~60 calls of off-scope exploration. Remaining: the two `agent/tests.rs` loop
tests, the E2E capture, and the Update Log.

### Update — 2026-10-26 15:05 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** the resume (session `6adfbc6d`) stalled genuinely — 120 read-only
calls, zero edits — searching for how to drive a loop test at a small
`read_only_stall_threshold`; that is a spec gap (the Test plan said "a small
threshold" without naming the `LoopDeps { governor: … }` mechanism), now
closed in § Test plan with the in-file analogue and the detectors to disable.

### Update — 2026-10-26 16:20 (escalation)

**Chosen lever:** session takeover
**Rationale:** the refined re-dispatch (session `6adfcb6d`) repeated the second
run's failure class exactly — 120 read-only calls, zero edits, re-auditing the
already-verified loop wiring despite the Notes block — so per the escalation
table (same-class failure after one refinement) the architect finished the
remaining two loop tests and the E2E capture directly.

### Update — 2026-10-26 16:40 (architect takeover)

- Added `bash_edit_resets_read_only_stall` / `read_only_bash_still_trips_stall`
  in `executor/src/agent/tests.rs` over a shared `run_bash_stall_script`
  helper (threshold 6; 4 reads, a middle bash call, 4 reads; repetition,
  oscillation and novelty neutralised). The positive test also asserts the
  middle call's file really landed in the root.
- **Latent defect found by the negative test, fixed:** the loop writes its
  session log to `<root>/.rexymcp/sessions/` (and bash spills output to
  `.rexymcp/output/`) on every call. In a root that does not gitignore
  `.rexymcp/` — a bare `TempDir`, or any target repo whose `.gitignore` lacks
  it — every `bash` call therefore read as a tree change and the stall
  detector could never fire on bash. `fingerprint` now prunes the root's
  `.rexymcp/` unconditionally; new tests
  `rexymcp_state_write_is_not_a_change` and `nested_rexymcp_dir_is_a_change`
  (only the root's is pruned). The spec missed this.
- Renamed the executor's extra test `made_file_progress_orrs_…` to
  `made_file_progress_is_mutating_tool_or_tree_change`.
- Mutation check: making `made_file_progress` ignore `changed_tree` fails
  `bash_edit_resets_read_only_stall`; removing the `.rexymcp` prune fails
  `read_only_bash_still_trips_stall` (observed before the fix). Restored.
- Gates, separate invocations: `cargo fmt --all --check` exit 0,
  `cargo build` exit 0, `cargo clippy --all-targets --all-features -- -D
  warnings` exit 0, `cargo test` exit 0 (executor lib 1068 passed: 1064 + 4).

### Update — 2026-10-26 16:45 (end-to-end verification)

```
test agent::tree_fingerprint::tests::dot_git_write_is_not_a_change ... ok
test agent::tree_fingerprint::tests::detects_deleted_file ... ok
test agent::tree_fingerprint::tests::unchanged_tree_compares_equal ... ok
test agent::tree_fingerprint::tests::nested_rexymcp_dir_is_a_change ... ok
test agent::tree_fingerprint::tests::rexymcp_state_write_is_not_a_change ... ok
test agent::tree_fingerprint::tests::gitignored_write_is_not_a_change ... ok
test agent::tree_fingerprint::tests::hidden_file_write_is_a_change ... ok
test agent::tree_fingerprint::tests::detects_written_file ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1062 filtered out; finished in 0.00s
tree_fingerprint exit=0
test governor::hard_fail::tests::read_only_stall_resets_on_tree_changing_bash ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.00s
read_only_stall_resets_on_tree_changing_bash exit=0
test governor::hard_fail::tests::read_only_stall_counts_bash_without_tree_change ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.00s
read_only_stall_counts_bash_without_tree_change exit=0
test governor::hard_fail::tests::oscillation_applies_to_tree_changing_bash_window ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.00s
oscillation_applies_to_tree_changing_bash_window exit=0
test governor::hard_fail::tests::tool_call_snapshot_deserializes_without_changed_tree ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.00s
tool_call_snapshot_deserializes_without_changed_tree exit=0
test agent::tests::bash_edit_resets_read_only_stall ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.04s
bash_edit_resets_read_only_stall exit=0
test agent::tests::read_only_bash_still_trips_stall ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1069 filtered out; finished in 0.03s
read_only_bash_still_trips_stall exit=0
```

### Review verdict — 2026-10-26

- **Verdict:** escalated
- **Bounces:** 2 (resume `6adfbc6d`, refined re-dispatch `6adfcb6d`), then
  session takeover
- **Executor:** nvidia/GLM-5.3-Flash-FP4-Local (tasks 1–3, first run
  `6adfa798`); Claude (direct) for the loop tests, E2E and the `.rexymcp`
  prune
- **Scope deviations:** `measure_novelty`'s trailing run now also resets on
  `made_file_progress` (consistent with the stall detector it mirrors;
  accepted). One extra governor test. Architect added the `.rexymcp/` prune.
- **Calibration:** the first run's stall was the bug under repair (scripted
  bash edits under the old governor) — not a model-quality signal. Runs 2 and
  3 were genuine stalls: zero edits, re-reading verified work despite explicit
  "do not re-read" guidance. Spec defect: the original Test plan named "a small
  threshold" without the mechanism, and missed the `.rexymcp/` self-write.
