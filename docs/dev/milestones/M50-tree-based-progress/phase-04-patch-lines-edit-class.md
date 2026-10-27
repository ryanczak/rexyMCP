# Phase 04: `patch_lines` is an edit-class call

**Milestone:** M50 — Tree-based progress
**Status:** in-progress
**Depends on:** phase-03
**Estimated diff:** ~120 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`patch_lines` replaces a line range in a file the model may never have read,
or may have read before the file changed, and nothing stops it: the
read-before-edit gate and the rest of the edit-class treatment cover only
`write_file` and `patch`. A `patch_lines` over a drifted file has silently cut
live code before. Make `patch_lines` an edit-class call like `patch`.
`delete_file` and `move_file` stay ungated (user decision, 2026-10-27).

## Architecture references

- `docs/dev/milestones/M50-tree-based-progress/README.md` § Notes, decision 5.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc.
3. Confirm a clean tree.

## Current state

In `executor/src/agent/tools.rs`, `edit_target` names the file an edit-class
call targets and `read_before_edit_refusal` gates such a call; both recognise
only `write_file` and `patch`. In the loop (`executor/src/agent/mod.rs`),
`edit_target`'s path drives the verifier baseline, superseded-read eviction,
the post-write hooks and the post-edit verify, and a separate check decides
which successful calls refresh the working set's mtime.

## Spec

1. **Gate and classify `patch_lines` like `patch`** — in `agent/tools.rs`,
   `edit_target` returns the `path` of a `patch_lines` call, and
   `read_before_edit_refusal` refuses a `patch_lines` of a file not read
   this session, or whose on-disk mtime no longer matches the read. The
   refusal message names the operation so the model knows which call was
   refused. `patch_lines` is always gated: there is no create or append case.
2. **Keep the file live after a successful `patch_lines`** — in the loop,
   a successful `patch_lines` refreshes the working set's mtime for its
   path, as `patch` does, so a following `patch` or `patch_lines` on the same
   file is not refused for staleness the model itself caused.
3. **`delete_file` and `move_file` are unchanged** — not gated, not in
   `edit_target`. Do not add them.
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] Every test named in § Test plan passes.
- [ ] Preservation: every existing test passes unmodified, including the
      phase-03 `patch_lines_edit_appears_in_phase_diff` (its script already
      reads the file first).
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- Gate, in `agent/tools.rs` (alongside the existing `gate_*` / `patch_of_*`
  tests):
  - `gate_refuses_patch_lines_of_unread_file`
  - `gate_allows_patch_lines_of_read_unchanged_file`
  - `gate_refuses_patch_lines_when_mtime_changed`
  - `gate_leaves_delete_and_move_ungated` — `delete_file` and `move_file` on
    an unread file are not refused.
  - `edit_target_includes_patch_lines_only` — `edit_target` yields the path
    for `patch_lines` and `None` for `delete_file` and `move_file`.
- Loop, in `agent/tests.rs` (`MockAiClientScript`; registry as in
  `run_with_all_edit_tools`):
  - `patch_lines_without_prior_read_is_refused` — the call fails, the file
    on disk is unchanged, and the refusal text reaches the model.
  - `patch_lines_after_read_then_patch_lines_again_is_allowed` — read, then
    two successive `patch_lines` on the same file both succeed.
  - `patch_lines_triggers_post_edit_verify` — the verifier is invoked on the
    file after a successful `patch_lines` (the mock verifier in that file
    records or is scripted per call).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m50-phase-04.txt; : > "$A"
for t in gate_refuses_patch_lines_of_unread_file gate_allows_patch_lines_of_read_unchanged_file gate_refuses_patch_lines_when_mtime_changed gate_leaves_delete_and_move_ungated edit_target_includes_patch_lines_only patch_lines_without_prior_read_is_refused patch_lines_after_read_then_patch_lines_again_is_allowed patch_lines_triggers_post_edit_verify patch_lines_edit_appears_in_phase_diff; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test `ok`, each `exit=0`, and each filter shows a
`test result` line with a non-zero passed count.

## Authorizations

- `executor/src/agent/mod.rs`, `executor/src/agent/tools.rs`,
  `executor/src/agent/tests.rs`.
- `executor/templates/executor_contract.md` **only if** it lists which tools
  require a prior read; then add `patch_lines` to that list and nothing else.

No new dependencies.

## Out of scope

- Gating `delete_file` or `move_file`.
- The `patch_lines` tool itself (`executor/src/tools/patch_lines.rs`).
- The bash classifier, `diff_targets`, the governors.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-27 01:00 (started)

**Executor:** Claude (rexyMCP executor run).
**Doing:** gate and classify `patch_lines` like `patch`; loop mtime refresh;
delete/move stay ungated; end-to-end evidence.

### Update — 2026-10-27 01:10 (end-to-end verification)

Ran the phase doc's § End-to-end verification block verbatim from the repo root;
artifact at `target/e2e/m50-phase-04.txt`, pasted below (mechanically captured,
not retyped). Every listed test `ok`, every `exit=0`, every filter shows a
`test result` line with a non-zero passed count — matches the doc's expected
shape. Positive control: `patch_lines_without_prior_read_is_refused` and
`patch_lines_edit_appears_in_phase_diff` fail (not pass) without the gate and
the refresh respectively — they were run before the loop-side wiring landed and
failed, then passed after it; the artifact captures the after state.

```
test agent::tools::tests::gate_refuses_patch_lines_of_unread_file ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.00s
gate_refuses_patch_lines_of_unread_file exit=0
test agent::tools::tests::gate_allows_patch_lines_of_read_unchanged_file ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.00s
gate_allows_patch_lines_of_read_unchanged_file exit=0
test agent::tools::tests::gate_refuses_patch_lines_when_mtime_changed ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.00s
gate_refuses_patch_lines_when_mtime_changed exit=0
test agent::tools::tests::gate_leaves_delete_and_move_ungated ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.00s
gate_leaves_delete_and_move_ungated exit=0
test agent::tools::tests::edit_target_includes_patch_lines_only ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.00s
edit_target_includes_patch_lines_only exit=0
test agent::tests::patch_lines_without_prior_read_is_refused ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.01s
patch_lines_without_prior_read_is_refused exit=0
test agent::tests::patch_lines_after_read_then_patch_lines_again_is_allowed ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.01s
patch_lines_after_read_then_patch_lines_again_is_allowed exit=0
test agent::tests::patch_lines_triggers_post_edit_verify ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.01s
patch_lines_triggers_post_edit_verify exit=0
test agent::tests::patch_lines_edit_appears_in_phase_diff ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1094 filtered out; finished in 0.01s
patch_lines_edit_appears_in_phase_diff exit=0
```
