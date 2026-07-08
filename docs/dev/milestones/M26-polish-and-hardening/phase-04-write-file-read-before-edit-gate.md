# Phase 04: `write_file` read-before-edit gate

**Milestone:** M26 — Polish & Hardening
**Status:** done
**Depends on:** none
**Estimated diff:** ~270 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

The read-before-edit gate (`read_before_edit_refusal`) protects `patch` but not
`write_file`: a model can **blind-overwrite an existing file it never read**,
which is precisely the data-loss the gate exists to prevent (codebase review
2026-07-07 § "Known no-ops to close out"). Extend the same gate to `write_file`
so an *overwrite* of an existing file requires a prior read (or a re-read after
the file changed on disk), mirroring `patch` — while leaving the two write forms
that cannot blind-clobber unread content ungated: **creating a new file** and
**appending** (`append: true`).

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` § "Known no-ops to close out" — the
  finding: "`read_before_edit` gates `patch` but not `write_file` … a model can
  blind-overwrite a file it never read."
- `docs/dev/WORKFLOW.md` § "Specs pin behavior, not rendering" (esp. **"Pin
  negative cases, not just positive ones"**) — the create/append exemptions are
  the load-bearing negatives here.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The gate today — `executor/src/agent/tools.rs:43-75`

`read_before_edit_refusal` is `patch`-only. Verbatim:

```rust
/// The read-before-edit gate (07d). Refuse a `patch` on a file the model has not
/// read this session, or one whose on-disk mtime no longer matches what was read.
/// `None` = allowed. Pure over `working_set` so the mtime-mismatch case is
/// unit-testable without mid-session filesystem hooks. `patch`-only — `write_file`
/// (whole-file create/overwrite) is not gated.
pub(super) fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "patch" {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    match working_set.get(&path) {
        None => Some(format!(
            "refusing to patch {}: you have not read it this session. Use read_file on it first.",
            path.display()
        )),
        Some(recorded) => {
            let current = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            match current {
                Some(now) if now == *recorded => None,
                _ => Some(format!(
                    "refusing to patch {}: it changed on disk since you read it. Re-read it with read_file first.",
                    path.display()
                )),
            }
        }
    }
}
```

### The refusal seam — `executor/src/agent/mod.rs:980-985`

The gate is already wired here; **no change to this seam is needed** — it passes
the whole `tool_call` through, so extending the helper is enough:

```rust
        let (succeeded, content, tool_meta) = if let Some((reference, _, _)) = &dedupe {
            (true, reference.clone(), None)
        } else {
            match destructive_restore_refusal(&tool_call, &pre_edit_content, deps.project_root)
                .or_else(|| read_before_edit_refusal(&tool_call, &working_set, deps.project_root))
            {
                Some(refusal) => (false, refusal, None),
                None => { /* baseline capture, pre_edit_content, dispatch */ }
```

On a refusal the edit short-circuits: no baseline, no `pre_edit_content` capture,
no dispatch — but it is still a model-visible failure that feeds back and counts
toward hard-fail. `edit_target` (`tools.rs:36`) already includes `write_file`, so
baseline capture / `pre_edit_content` / superseded-read eviction already run for a
write_file that is *allowed*; nothing there changes.

### The working-set recording block — `executor/src/agent/mod.rs:1096-1103`

Verbatim:

```rust
        // Record the working set: a read makes a file patch-eligible; a successful
        // patch refreshes its mtime so a follow-up patch needs no re-read.
        if succeeded
            && (tool_call.name == "read_file" || tool_call.name == "patch")
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            record_mtime(&mut working_set, &path);
        }
```

This is why gating `write_file` *requires* also recording its mtime: without
that, a model that **creates** a file (allowed) and then tries to overwrite it in
a later turn would be refused ("you have not read it this session"), because the
overwrite now sees an existing on-disk file that is not in the working set. A
whole-file write establishes the model's knowledge of the file's full content, so
it must refresh the working set exactly as `patch` does. An **append** must NOT —
the model still has not seen the file's pre-existing content, so a later
*overwrite* must still force a re-read.

### The `write_file` tool — `executor/src/tools/write_file.rs`

`write_file` takes `path`, `content`, and an optional `append: Option<bool>`
(default false = overwrite/create). The gate reads only the *arguments* (via
`tool_call.arguments`), not the tool impl; the tool itself is **unchanged** by
this phase.

## The behavior to implement (read this before the Spec)

`write_file` splits into three cases; only the first is gated:

| Case | `append` | Target on disk | Gated? |
|---|---|---|---|
| **Overwrite** | absent / `false` | **exists** | **Yes** — require prior read / fresh re-read |
| **Create** | absent / `false` | absent | No — nothing to clobber |
| **Append** | `true` | either | No — does not clobber unread content |

`patch` behavior is **unchanged**: always gated, exactly as today.

## Spec

1. **Extend `read_before_edit_refusal` to cover `write_file` overwrites** — in
   `executor/src/agent/tools.rs`, replace the `patch`-only guard and the two
   `format!("refusing to patch …")` strings so the helper handles both tools.
   Suggested shape (adapt names/wording freely; the **behavior** and the message
   substrings pinned in the Test plan are what matter):

   ```rust
   /// The read-before-edit gate (07d, extended M26 for `write_file`). Refuse an
   /// edit-class call on a file the model has not read this session, or one whose
   /// on-disk mtime no longer matches what was read. `None` = allowed.
   ///
   /// `patch` is always gated. `write_file` is gated **only when it would
   /// overwrite an existing file**: a create (target absent on disk) and an
   /// append (`append: true`) are allowed unconditionally — neither blind-clobbers
   /// content the model never read. The on-disk `metadata`/`exists` stats mirror
   /// the mtime branch's existing filesystem touch, so every arm stays
   /// `TempDir`-testable.
   pub(super) fn read_before_edit_refusal(
       tool_call: &ToolCall,
       working_set: &HashMap<PathBuf, SystemTime>,
       project_root: &Path,
   ) -> Option<String> {
       let verb = match tool_call.name.as_str() {
           "patch" => "patch",
           "write_file" => "overwrite",
           _ => return None,
       };
       let path = resolve_path(tool_call, project_root)?;
       if tool_call.name == "write_file" {
           let appending = tool_call
               .arguments
               .get("append")
               .and_then(|v| v.as_bool())
               .unwrap_or(false);
           if appending || !path.exists() {
               return None;
           }
       }
       match working_set.get(&path) {
           None => Some(format!(
               "refusing to {verb} {}: you have not read it this session. Use read_file on it first.",
               path.display()
           )),
           Some(recorded) => {
               let current = std::fs::metadata(&path)
                   .ok()
                   .and_then(|m| m.modified().ok());
               match current {
                   Some(now) if now == *recorded => None,
                   _ => Some(format!(
                       "refusing to {verb} {}: it changed on disk since you read it. Re-read it with read_file first.",
                       path.display()
                   )),
               }
           }
       }
   }
   ```

2. **Refresh the working set after a whole-file `write_file`** — in
   `executor/src/agent/mod.rs`, extend the recording block at `1096-1103` so a
   successful non-append `write_file` records its mtime alongside `read_file` /
   `patch`. An **append must be excluded**. Suggested shape:

   ```rust
       // Record the working set: a read makes a file edit-eligible; a successful
       // patch or whole-file write_file (not an append) refreshes its mtime so a
       // follow-up edit needs no re-read. An append does NOT — the model has still
       // not seen the file's full content, so a later overwrite must re-read.
       let refreshes_working_set = matches!(tool_call.name.as_str(), "read_file" | "patch")
           || (tool_call.name == "write_file"
               && tool_call.arguments.get("append").and_then(|v| v.as_bool()) != Some(true));
       if succeeded
           && refreshes_working_set
           && let Some(path) = resolve_path(&tool_call, deps.project_root)
       {
           record_mtime(&mut working_set, &path);
       }
   ```

3. **Unit tests for the extended gate** — in `tools.rs`'s `#[cfg(test)] mod
   tests` (a `#[test]` block; these are synchronous pure calls, `TempDir` for the
   on-disk cases). Build `write_file` `ToolCall`s the same way the file's existing
   helpers build calls (`ToolCall { name, arguments: json!({...}), origin:
   Origin::Native }`). Cover, at minimum, the behavior table:
   - **create allowed**: `write_file` to a path that does **not** exist on disk,
     empty working set → `None`.
   - **append allowed**: `write_file` with `"append": true` to an **existing**
     file, empty working set → `None`.
   - **overwrite of unread existing file refused**: existing file (write it to a
     `TempDir` first), empty working set, no `append` → `Some(msg)` where `msg`
     contains `"overwrite"` and the path.
   - **overwrite after read allowed**: existing file, working set holds the path
     with the file's **current** mtime (use `record_mtime`) → `None`.
   - **overwrite with stale mtime refused**: existing file, working set holds the
     path with a deliberately wrong mtime (`SystemTime::UNIX_EPOCH`) → `Some(msg)`
     containing `"changed on disk"`.
   - **patch still gated (regression)**: a `patch` call for a path not in the
     working set → `Some(msg)` containing `"patch"` (proves the `write_file`
     changes did not weaken the `patch` arm).

4. **Loop integration tests** — in `executor/src/agent/tests.rs`, add tests
   mirroring the existing `write_file` / gate integration tests (use `native(...)`
   scripts + `run_with_verifier` or `run_full`, per the neighbours):
   - `write_file_overwrite_of_unread_file_is_refused` — `TempDir` with an existing
     file (`std::fs::write` in setup), a one-turn `write_file` overwrite with **no
     prior read**, then `done`. Assert the file's on-disk content is **unchanged**
     (the overwrite was refused) and the run still reaches `Complete` (a refusal
     is model-visible, not a hard_fail). Optionally assert the refused
     `write_file` tool result fed back contains `"refusing to overwrite"`.
   - `write_file_after_read_overwrites` — existing file, turn 1 `read_file`, turn
     2 `write_file` overwrite, then `done`. Assert the on-disk content **is** the
     new content (the read unlocked the overwrite).
   - `write_file_append_to_unread_file_is_allowed` — existing file, one-turn
     `write_file` with `"append": true` and **no prior read**, then `done`. Assert
     the on-disk content is the original **plus** the appended content.

5. **Adjust three existing tests that relied on the un-gated behavior.** Each
   currently overwrites an *existing* file with no prior read; under the new gate
   that write is refused. Fix them to keep exercising a **real** write (not a
   silently-refused one):

   - **`self_revert_of_edited_file_is_refused`** (`tests.rs:4238`) — **required,
     goes red otherwise.** Its turn-1 `write_file` overwrites the pre-written
     `src/flow.ts` to populate `pre_edit_content`; refused now, so the later `git
     checkout` is no longer seen as reverting an edited file. Prepend a
     `read_file` turn so the overwrite is allowed, and bump the asserted call
     index by one. Concretely, change the script from `[write, bash, done]` to:

     ```rust
     let client = MockAiClientScript::new(vec![
         vec![native("read_file", json!({ "path": path }))],
         vec![native(
             "write_file",
             json!({ "path": path, "content": "edited content" }),
         )],
         vec![native(
             "bash",
             json!({ "command": "git checkout src/flow.ts" }),
         )],
         vec![token("done")],
     ]);
     ```

     and update the bash-refusal assertion to read `&client.calls()[3].messages`
     (was `[2]`). The refusal-content assertions are otherwise unchanged.

   - **`unchanged_file_is_absent_from_files_changed`** (`tests.rs:1760`) — its
     `write_file` of identical content to the pre-written `t.txt` is now refused,
     so it passes only *because* nothing was written (wrong reason). Prepend a
     `read_file` turn (`vec![native("read_file", json!({ "path": path }))]`) so the
     identical-content overwrite actually executes and genuinely produces no diff.
     Assertions unchanged.

   - **`loop_does_not_log_read_evicted_without_prior_read`** (`tests.rs:1514`) —
     its intent is "a *successful* write with no prior read evicts nothing." A
     pre-written file makes the write a refused overwrite. Make the target a
     **create** instead: delete the `std::fs::write(&file, "original content")`
     setup line so `write_file` creates the file, succeeds, and (no prior read)
     logs no `ReadEvicted`. Assertion unchanged.

   Do **not** touch any other `write_file` test: the format-hook tests
   (`format_hook_*`, `hook_*`, `lint_fix_*`), `new_file_diff_is_all_added`, and
   `write_file_without_read_is_allowed` all target files that do **not** pre-exist
   (creates) and stay green unchanged; `loop_does_not_dedupe_after_edit` reads
   before it writes and stays green.

## Acceptance criteria

- [ ] `read_before_edit_refusal` refuses a `write_file` overwrite of an existing
      file not in the working set (or with a changed mtime) and **allows** a
      create (absent file) and an append (`append: true`); `patch` is unchanged.
- [ ] A successful non-append `write_file` records the file's mtime in the
      working set; an append does not.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new), including the three adjusted tests.

## Test plan

Unit tests (`tools.rs`, pure + `TempDir`) — names describe behavior:

- `write_file_create_is_allowed_without_read` — absent path → `None`.
- `write_file_append_is_allowed_without_read` — existing file, `append: true` →
  `None`.
- `write_file_overwrite_of_unread_existing_file_is_refused` — `Some`, message
  contains `"overwrite"` and the path.
- `write_file_overwrite_after_read_is_allowed` — working set holds current mtime
  → `None`.
- `write_file_overwrite_with_changed_mtime_is_refused` — stale recorded mtime →
  `Some`, message contains `"changed on disk"`.
- `patch_of_unread_file_still_refused` — `patch` arm regression → `Some`, message
  contains `"patch"`.

Integration tests (`tests.rs`, `MockAiClientScript` + `TempDir`):

- `write_file_overwrite_of_unread_file_is_refused` — on-disk content unchanged;
  run reaches `Complete`.
- `write_file_after_read_overwrites` — on-disk content is the new content.
- `write_file_append_to_unread_file_is_allowed` — on-disk content is original +
  appended.

Adjusted (must stay green): `self_revert_of_edited_file_is_refused`,
`unchanged_file_is_absent_from_files_changed`,
`loop_does_not_log_read_evicted_without_prior_read`.

## End-to-end verification

The real artifact is the executor loop's live refusal behavior, exercised by the
`tests.rs` integration tests against the **real** `write_file` tool + registry +
gate (not a pure-helper fake): `write_file_overwrite_of_unread_file_is_refused`
drives a real overwrite attempt through `execute_phase` and asserts the file on
disk was not clobbered. Paste that test's `cargo test` output line into the
completion Update Log. (No CLI/MCP surface changes ship in this phase, so there
is no binary entrypoint to run by hand.)

## Authorizations

None from STANDARDS.md §5. This phase authorizes edits to exactly
`executor/src/agent/tools.rs` and `executor/src/agent/mod.rs` (production) and
`executor/src/agent/tests.rs` (tests). Do **not** edit `tools/write_file.rs`, the
refusal seam at `mod.rs:980-985`, or any other file.

## Out of scope

- **Changing the `write_file` tool** (`executor/src/tools/write_file.rs`). The
  gate reads the call's arguments in the loop; the tool impl is untouched.
- **Gating append or create.** They cannot blind-clobber unread content; keeping
  them ungated is the explicit design (see the behavior table). Do not add a
  read requirement to either.
- **The redundant-read dedupe / superseded-read eviction paths.** They already
  handle `write_file` via `edit_target`; leave `redundant_read_reference`,
  `evict_superseded_reads`, and `last_live_read` alone.
- **The `destructive_restore_refusal` bash guard.** Unrelated; only the
  `self_revert_*` test's *script* changes (a prepended read), not that helper.
- **Any post-write format-hook change** — that is phase-05.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Notes for executor — 2026-07-07

**Refined re-dispatch after hard_fail.** All five spec tasks were completed and
marked `done` (the gate extension, the working-set refresh, and all three test
groups) — the production code in `tools.rs`/`mod.rs` is correct and the working
tree already has these changes uncommitted; do not redo that work. The run then
hard-failed on `IdenticalToolCallRepetition` (6× identical `read_file` on
`executor/src/agent/tests.rs:1395-1410`) after spinning for ~140 unproductive
turns.

**The one real defect:** the new integration test
`write_file_overwrite_of_unread_file_is_refused` (`tests.rs` around line 1568)
asserts the refusal string appears in `client.calls()[0].messages` — but index
`0` is the *initial* prompt sent before any tool call runs. The refused
tool-result is fed back on the *next* model call, so the assertion must read
`client.calls()[1].messages`, exactly like the neighboring pre-existing
`patch_after_reading_is_allowed`/`patch_of_unread_file_is_refused` pair at
`tests.rs:1395-1410` (`let second = &client.calls()[1].messages;`) — the same
lines this run kept re-reading without ever applying the pattern. Confirmed via
`cargo test --lib agent::tests::` at escalation time: **exactly this one test
fails**, all 762 others (including the three phase-04-adjusted tests) pass.

**Fix:** in the failing test, change `&client.calls()[0].messages` to
`&client.calls()[1].messages` (one-line change, `tests.rs` ~line 1596). Do not
touch any other test or production file — everything else is done and green.
Then run the four gate commands and finish normally.

### Update — 2026-07-07 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** first hard_fail, all substantive spec work already complete and
verified correct; the sole failure is a one-line test-index bug the executor
authored itself (not a spec gap), confirmed in isolation by an independent
`cargo test` run — a targeted note is far cheaper than a takeover and keeps the
scorecard data point.

### Update — 2026-07-08 00:19 (complete)

**Summary:** Fixed the one remaining defect from the prior escalation: the integration
test `write_file_overwrite_of_unread_file_is_refused` asserted the refusal string at
`client.calls()[0].messages` (the initial prompt) instead of
`client.calls()[1].messages` (the next model call where the tool result is fed back).
One-line change in `tests.rs`. All production code (gate extension in `tools.rs`,
working-set refresh in `mod.rs`) was already correct from the prior run. All 883 tests
pass.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(exited 0, no output)

cargo build 2>&1 | tail -20
(exited 0, no new warnings)

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
(exited 0, no warnings)

cargo test 2>&1 | tail -30
test result: ok. 883 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

`cargo test --lib agent::tests::write_file_overwrite_of_unread_file_is_refused` —
exited 0, 1 passed. The integration test drives a real overwrite attempt through
`execute_phase` and asserts the file on disk was not clobbered.

**Grep proof:** `grep -rn "refusing to overwrite" executor/src/agent/tools.rs` —
confirms the refusal literal landed in the gate helper.

**Files changed:**
- `executor/src/agent/tests.rs` — fixed call index from `[0]` to `[1]` in
  `write_file_overwrite_of_unread_file_is_refused` (one-line fix).

**New tests:** none (all tests were already present from prior run).

**Commits:**
- pending

**Notes for review:** The production code in `tools.rs` and `mod.rs` was already
correct from the prior escalation run; only the test assertion needed fixing.

### Review verdict — 2026-07-08

- **Verdict:** approved_first_try
- **Bounces:** none (the earlier `IdenticalToolCallRepetition` hard_fail was
  resolved by escalation/refined re-dispatch, not a review bounce — no bug
  report was filed; precedent: M18 phase-03's "handled by refined re-dispatch
  ... not a review bounce")
- **Executor:** Qwen/Qwen3.6-27B-PrismaAURA
- **Scope deviations:** none — all five spec tasks implemented exactly as
  authorized; only `tools.rs`, `mod.rs`, `tests.rs` touched
- **Calibration:** 1st occurrence, data only, no fold. After completing all
  substantive work correctly, the executor entered an ~140-turn unproductive
  loop re-reading an unrelated, already-correct code block
  (`patch_after_reading_is_allowed` at `tests.rs:1395-1410`) that held the
  exact fix pattern it needed (`client.calls()[1]`) for a self-authored
  off-by-one test-index bug, without ever applying it — a "sees the answer
  nearby but doesn't act" fixation distinct from prior
  `IdenticalToolCallRepetition` causes (M18's oversized-write null-args,
  M22's task-remark fixation). Independent re-run confirmed the diagnosis:
  `cargo test --lib agent::tests::` showed exactly one failing test, fixed by
  a one-line index correction. Would fold if a third instance of this
  "adjacent correct pattern, not applied" class recurs.

**Independent verification (architect re-run, 2026-07-08):** `cargo fmt --all
--check` clean; `cargo build` clean; `cargo clippy --all-targets
--all-features -- -D warnings` clean; `cargo test` — 883 passed / 0 failed / 2
ignored. Grepped `tools.rs`/`mod.rs` production sections (before `mod tests` at
`tools.rs:447`): no `unwrap`/`expect`/`panic!`/`unsafe`/`#[allow]`/`TODO`.
Mutation-tested the `mod.rs` working-set refresh (reverted `write_file` from
`refreshes_working_set`): `persistent_verifier_failure_trips_hard_fail` goes
red, confirming the refresh logic is genuinely exercised by the suite (via
repeated same-file `write_file` calls across turns), not merely present.
Spot-checked `write_file_overwrite_of_unread_file_is_refused`: asserts both
on-disk content unchanged and the refusal string fed back — a reverted gate
would fail both assertions, so this is a real test, not a vacuous one.
