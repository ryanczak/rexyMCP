# Phase 01: Route governor + briefing through `tools::mutates_files`

**Milestone:** M33 — Governor Mutating-Tool Classifier Unification
**Status:** done
**Depends on:** none
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

Make the router's `Category::Write` set the single source of truth for "this tool
mutated a file," and have the no-progress stall governor and the escalation
briefing ask it instead of each carrying a stale hardcoded `["patch",
"write_file"]` list. Fixes the false `NoProgressStall` hard_fail on executors that
edit via `patch_lines` (issue #2).

## Architecture references

- `docs/architecture.md` § Layer 1 — the router and tool categories.

## Current state

- `executor/src/tools/router.rs:17` — canonical: `"write_file" | "patch" |
  "patch_lines" | "delete_file" | "move_file" => Category::Write`.
- `executor/src/governor/hard_fail.rs:245` — stale `const MUTATING_TOOLS: [&str;
  2] = ["patch", "write_file"]`, used at the read-only stall reset.
- `executor/src/phase/briefing.rs:150` — same stale pair gating
  `collect_working_files`; extracts only a `path` argument.

## Spec

1. **Add `mutates_files`** — in `executor/src/tools/router.rs`, add
   `pub fn mutates_files(tool_name: &str) -> bool { categorize(tool_name) ==
   Some(Category::Write) }`, and re-export it from `executor/src/tools/mod.rs`.
2. **Governor** — in `executor/src/governor/hard_fail.rs`, delete
   `MUTATING_TOOLS` and reset the no-progress run on
   `crate::tools::mutates_files(&call.tool)`; update the doc comment.
3. **Briefing** — in `executor/src/phase/briefing.rs`, gate
   `collect_working_files` on `tools::mutates_files(&snapshot.tool)` and pick the
   path key per tool: `to` for `move_file` (its `from` no longer exists
   post-edit), `path` otherwise. A `delete_file` target reads as unreadable and
   is skipped by the existing guard — correct (no content survives a delete).

## Acceptance criteria

- [x] `cargo clippy --all-targets --all-features -- -D warnings` clean.
- [x] `cargo test` green (976 passed, 2 ignored).
- [x] No private mutating-tools list remains outside the router
  (`grep -rn 'MUTATING_TOOLS'` → no hits).

## Test plan

- `mutates_files_covers_every_write_tool` / `mutates_files_false_for_non_write_tools`
  in `router.rs` — the helper agrees with `Category::Write`.
- `read_only_stall_counts_every_write_tool_as_progress` in `hard_fail.rs` —
  a `patch_lines`/`delete_file`/`move_file` mutation resets the no-progress
  counter (mutation-proof: fails under the old two-name list).
- `collect_working_files_includes_patch_lines_edits` and
  `collect_working_files_resolves_move_file_via_to_key` in `briefing.rs`.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact beyond the governor
logic, which is exercised by the hermetic unit tests above (the false hard_fail
was an in-process governor decision, not a CLI/serve surface). The next real
phase dispatch to a `patch_lines`-favoring executor is the live confirmation.

## Authorizations

None.

## Out of scope

- Any change to the set of tools the router classifies as `Category::Write`.
- Touching the `read_only_stall_threshold` default or other governor knobs.

## Update Log

<!-- entries appended below this line -->

### Update — 2026-07-16 (complete)

**Summary:** Added `tools::mutates_files` (delegating to the router's
`Category::Write`) and routed both the no-progress stall governor and the
briefing's working-file collector through it, deleting the two stale
`["patch", "write_file"]` lists. The briefing now resolves `move_file` via its
`to` key. No deviations from the spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean — after rustfmt on the one touched file, router.rs)

cargo build 2>&1 | tail -3
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.21s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -3
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.15s

cargo test 2>&1 | tail -3
test result: ok. 976 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.08s
```

**End-to-end verification:** N/A — governor-internal decision, covered by the
hermetic regression tests (see phase doc's E2E section).

**Files changed:**
- `executor/src/tools/router.rs` — new `mutates_files` helper + 2 unit tests.
- `executor/src/tools/mod.rs` — re-export `mutates_files`.
- `executor/src/governor/hard_fail.rs` — delete `MUTATING_TOOLS`, call the
  helper; regression test for the three previously-missed write tools.
- `executor/src/phase/briefing.rs` — gate on the helper, per-tool path key;
  two new tests (`patch_lines` and `move_file`-via-`to`).

**New tests:**
- `mutates_files_covers_every_write_tool`, `mutates_files_false_for_non_write_tools`
  in `router.rs`
- `read_only_stall_counts_every_write_tool_as_progress` in `hard_fail.rs`
- `collect_working_files_includes_patch_lines_edits`,
  `collect_working_files_resolves_move_file_via_to_key` in `briefing.rs`

**Notes for review:** The `read_only_stall_counts_every_write_tool_as_progress`
test is mutation-proof against a revert — under the old two-name list the three
new write tools don't reset the counter and it fails.

### Review verdict — 2026-07-16

- **Verdict:** approved_first_try
- **Bounces:** 0 (bugs: none)
- **Executor:** Claude Code (direct)
- **Scope deviations:** none
- **Calibration:** Recurring shape — a fact owned by one canonical classifier
  duplicated into a private list that drifts. Same family as prior "single
  source of truth" cleanups; noted, not folded (1st occurrence of *this* class).
