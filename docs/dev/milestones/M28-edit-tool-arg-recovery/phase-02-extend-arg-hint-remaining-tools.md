# Phase 2: Extend the `missing_args_hint` recovery message to the remaining 8 arg-parsing tools

**Milestone:** M28 — Edit-Tool Arg Recovery
**Status:** done
**Depends on:** phase-01 (the `missing_args_hint` helper it reuses is `done`)
**Estimated diff:** ~200 lines (8 uniform arm rewrites + tests + 2 test updates)
**Tags:** language=rust, kind=bugfix, size=m

## Goal

Phase-01 replaced the dead-end raw `invalid arguments: <serde error>` message
with the actionable `missing_args_hint(tool, required, present)` recovery
message for `write_file` and `patch`. Eight other tools still surface the raw
serde error on a deserialization failure. This phase extends the **same
helper** (unchanged) to all eight, so a truncated/malformed call to any tool
gives the model a recoverable message instead of a dead end. Purely a
failure-message change: happy paths stay byte-identical.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #24 (M24 — Edit-Loop Recovery) and #28
  (this milestone) — the precedent of enriching a dead-end tool error.
- `executor/src/tools/registry.rs:34` — the `missing_args_hint` helper (already
  written and tested in phase-01; **reuse as-is, do not modify it**).
- `phase-01-edit-tool-missing-field-hint.md` § Spec task 2 — the worked
  arm-rewrite this phase repeats eight times (quoted below).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

All eight tools share the **identical** failing arm — the raw serde error is
surfaced verbatim. Example, `executor/src/tools/read_file.rs:71-78`:

```rust
let parsed = match serde_json::from_value::<ReadFileArgs>(args) {
    Ok(a) => a,
    Err(e) => {
        return Ok(ToolResult {
            output: String::new(),
            error: Some(format!("invalid arguments: {e}")),
            metadata: None,
        });
    }
};
```

The helper is already `pub(crate)` in `registry.rs` and is imported in the
`write_file`/`patch` sibling modules as `super::registry::missing_args_hint`
(see `write_file.rs:75`). Same import works for all eight.

### The worked rewrite to repeat (from phase-01, proven)

Compute the present required keys from `&args` **before** the `from_value`
move (do **not** clone `args` — some payloads are large), then call the helper
on failure:

```rust
async fn execute(&self, args: Value) -> Result<ToolResult> {
    let required = ["path", "content"];               // <- per-tool list, see table
    let present: Vec<&str> = args
        .as_object()
        .map(|m| required.iter().copied().filter(|k| m.contains_key(*k)).collect())
        .unwrap_or_default();
    let parsed = match serde_json::from_value::<ReadFileArgs>(args) {
        Ok(a) => a,
        Err(_) => {
            return Ok(ToolResult {
                output: String::new(),
                error: Some(super::registry::missing_args_hint(
                    "read_file", &required, &present,
                )),
                metadata: None,
            });
        }
    };
```

Everything after the deserialization is unchanged.

### Per-tool required-field list (the `required` slice for each)

The `required` slice is exactly the **non-`Option`** fields of each tool's
args struct, in declaration order (verified from source at draft time). The
tool name string is the tool's `name()` (matches the file/struct):

| Tool (file) | Args struct | `required` slice |
|---|---|---|
| `patch_lines.rs:65` | `PatchLinesArgs` | `["path", "start_line", "end_line", "new_content"]` |
| `move_file.rs:53` | `MoveFileArgs` | `["from", "to"]` |
| `delete_file.rs:48` | `DeleteFileArgs` | `["path"]` |
| `bash.rs:67` | `BashArgs` | `["command"]` |
| `search.rs:85` | `SearchArgs` | `["pattern"]` |
| `find_files.rs:68` | `FindFilesArgs` | `["pattern"]` |
| `symbols.rs:122` | `SymbolsArgs` | `[]` — see note |
| `read_file.rs:71` | `ReadFileArgs` | `["path"]` |

**`symbols` is the special case: it has _no_ required fields** (every field of
`SymbolsArgs` is `Option`). Pass `required = []` (an empty `[&str; 0]`). With
an empty `required`, the helper computes `missing = []` and `present = []`, so
it returns its **type-mismatch** branch ("all required fields were present but
one has an invalid type or value…"). That is the correct message for symbols:
a `from_value` failure there can only be a wrong field *type*
(e.g. `max_results` given a string), never a dropped required field — nothing
is mandatory to drop. Do **not** invent a required field for symbols to make
the example shape prettier; the empty-required, type-mismatch path is right.

### Two existing tests assert the OLD message — they will break; update them

These currently assert the removed `"invalid arguments"` string and **must**
be updated as part of this phase (else `cargo test` fails):

- `bash.rs:461` (`rejects_malformed_args`, args `{"timeout_secs": 5}` — the
  required `command` is missing): replace
  `assert!(result.error.as_ref().unwrap().contains("invalid arguments"));`
  with an assertion that the message names the missing `command` field and is
  **not** the raw serde string — e.g.
  ```rust
  let err = result.error.as_ref().unwrap();
  assert!(err.contains("command"), "should name the missing field: {err}");
  assert!(!err.contains("invalid arguments: missing field"), "no raw serde text: {err}");
  ```
- `read_file.rs:357` (args `{"wrong_field": "value"}` — the required `path` is
  missing): the same update, asserting the message names `path` and is not the
  raw serde string. Keep the existing `assert!(result.output.is_empty());`.

## Spec

For each of the eight tools in the table above, in its own file:

1. **`patch_lines.rs`** — rewrite the `from_value::<PatchLinesArgs>` failing arm
   (line ~65) to the worked pattern with
   `required = ["path", "start_line", "end_line", "new_content"]` and tool name
   `"patch_lines"`.
2. **`move_file.rs`** — arm at ~53, `required = ["from", "to"]`, name
   `"move_file"`.
3. **`delete_file.rs`** — arm at ~48, `required = ["path"]`, name
   `"delete_file"`.
4. **`bash.rs`** — arm at ~67, `required = ["command"]`, name `"bash"`. Then
   update the `rejects_malformed_args` test (line ~461) per "Two existing
   tests" above.
5. **`search.rs`** — arm at ~85, `required = ["pattern"]`, name `"search"`.
6. **`find_files.rs`** — arm at ~68, `required = ["pattern"]`, name
   `"find_files"`.
7. **`symbols.rs`** — arm at ~122, `required = []` (empty), name `"symbols"`.
   The `present` computation with an empty `required` yields `[]`; that is
   fine. (Because `required` is empty, the `.filter(...)` produces nothing —
   this compiles and needs no special-casing.)
8. **`read_file.rs`** — arm at ~71, `required = ["path"]`, name `"read_file"`.
   Then update the malformed-args test (line ~357) per "Two existing tests".

Then add the per-tool tests below and run the four gates (separate
invocations).

**Do not touch** `registry.rs`'s `missing_args_hint` / `example_shape` — they
are correct as-is. In particular, `example_shape` renders every field as
`"<string>"` regardless of its real JSON type (so `patch_lines`'s numeric
`start_line`/`end_line` show as `"<string>"` in the example). That is an
accepted, deliberate limitation — the hint's load-bearing content is the
**field names**, and the tool's own `schema()` remains the type authority.
Improving `example_shape` to encode types is explicitly out of scope (it would
churn the shared helper used by the phase-01 tools).

## Acceptance criteria

- [ ] `grep -rn "invalid arguments" executor/src/tools/` returns **nothing**
      (no production arm and no test assertion still references the removed
      string across any tool).
- [ ] Each of the eight tools' deserialization-failure arm calls
      `missing_args_hint` with the tool's name and the required slice from the
      table.
- [ ] A `read_file` call `{"start_line": 1}` (no `path`) returns a message
      naming `path` missing — not `invalid arguments: missing field \`path\``.
- [ ] A `move_file` call `{"from": "a"}` (no `to`) names `to` missing and
      echoes `from` as supplied.
- [ ] A `symbols` call with a wrong-typed field (e.g.
      `{"max_results": "lots"}`) returns the hint (type-mismatch branch) with
      **no** raw serde text and no panic.
- [ ] A **valid** call to each tool still behaves byte-identically (happy path
      unchanged; the hint fires only on a `from_value` failure).
- [ ] Non-object args (e.g. `json!(5)`) to any rewired tool return the hint
      without a panic.
- [ ] The updated `bash` and `read_file` tests pass and assert the new hint
      shape (not the old `"invalid arguments"` substring).
- [ ] `cargo build` (zero new warnings), `clippy -D warnings`,
      `fmt --all --check`, and `cargo test` all pass.
- [ ] `registry.rs` is unchanged (helper reused, not modified).

## Test plan

Mirror the phase-01 tool tests, in each tool's existing `#[cfg(test)] mod
tests` against a `TempDir` scope where the tool needs one. Names describe
behavior:

- `read_file_missing_path_returns_recovery_hint` — `execute(json!({"start_line":
  1}))` → error names `path`, is not the raw serde string.
- `move_file_missing_to_returns_recovery_hint` — `{"from": "a"}` → names `to`
  missing, echoes `from` supplied (assert both, distinctly).
- `delete_file_missing_path_returns_recovery_hint`.
- `patch_lines_missing_fields_returns_recovery_hint` — `{"path": "x"}` → names
  the three missing numeric/content fields.
- `search_missing_pattern_returns_recovery_hint`.
- `find_files_missing_pattern_returns_recovery_hint`.
- `symbols_type_mismatch_returns_recovery_hint` — `{"max_results": "lots"}` →
  hint present, **no** raw serde text, no panic.
- `bash_missing_command_returns_recovery_hint` — either fold into the updated
  `rejects_malformed_args` or add alongside it.
- For at least two tools, a `non_object_args_do_not_panic` test over
  `json!(5)` / `json!("oops")` asserting a hint (not a panic), matching the
  phase-01 coverage shape.

Make the missing-vs-supplied assertions distinct (missing field named in the
missing clause, supplied field in the supplied clause) so a mislabeled helper
call fails.

## End-to-end verification

> Not applicable as a standalone CLI/binary E2E — these tools have no command
> surface; they run only through `execute_phase`. The unit tests call the
> **real** `execute` methods against `TempDir` scopes (the shipped code path),
> which is the same verification basis phase-01 used. Quote the
> `grep -rn "invalid arguments" executor/src/tools/` empty result and the
> `cargo test` pass line in the completion Update Log.

## Authorizations

None. (No new dependency; no `docs/architecture.md` edit — the milestone is
already recorded in § Status, and this phase adds no architectural surface;
`registry.rs` is not modified.)

## Out of scope

- Modifying `missing_args_hint` or `example_shape` in `registry.rs` (including
  type-aware example shapes) — reuse the helper unchanged.
- **Auto-reconstructing** any missing field value, or **context-pressure
  guards** (issue #1 solutions 1 & 3) — still deferred, as in phase-01.
- Changing any tool's `schema()` / args struct / required JSON — the fields
  stay as they are; only the *failure message* changes.
- Adding a required field to `symbols` to prettify its example shape — its
  empty-required, type-mismatch path is correct.
- Any tool not in the eight-row table (`write_file`/`patch` are already done).

## Update Log

### Update — 2026-07-11 04:30 (started)

**Executor:** Phase 2 executor

**Progress:** Starting implementation — extending `missing_args_hint` to 8 remaining tools.

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1783745252488 (complete, server-authored)

**Summary:** Summary + Notes for review:

Extended the `missing_args_hint` recovery message from phase-01's two tools (`write_file`, `patch`) to the remaining 8 arg-parsing tools: `patch_lines`, `move_file`, `delete_file`, `bash`, `search`, `find_files`, `symbols`, and `read_file`. Each tool's deserialization-failure arm was rewritten to compute present required keys from `&args` before the `from_value` call, then invoke `super::registry::missing_args_hint` on failure. The `registry.rs` helper was left unchanged as specified. Updated the two existing tests (`bash::rejects_malformed_args` and `read_file::rejects_malformed_args`) to assert the new hint shape. Added per-tool recovery-hint tests and non-object-args panic tests for `patch_lines`, `move_file`, `delete_file`, `search`, `find_files`, and `symbols`. Verified that `grep -rn "invalid arguments"` returns zero production `format!` calls (only test assertions remain, all negating the old string). All gates pass: `cargo build` (0 warnings), `clippy -D warnings`, `fmt --check`, and `cargo test` (960 passed, 2 ignored).

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
 ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::symbols::tests::references_finds_call_sites ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 960 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M28-edit-tool-arg-recovery/README.md` — +1 -1
- `docs/dev/milestones/M28-edit-tool-arg-recovery/phase-02-extend-arg-hint-remaining-tools.md` — +7 -1
- `executor/src/tools/bash.rs` — +24 -4
- `executor/src/tools/delete_file.rs` — +45 -2
- `executor/src/tools/find_files.rs` — +31 -2
- `executor/src/tools/move_file.rs` — +46 -2
- `executor/src/tools/patch_lines.rs` — +54 -2
- `executor/src/tools/read_file.rs` — +23 -4
- `executor/src/tools/search.rs` — +29 -2
- `executor/src/tools/symbols.rs` — +49 -2

**Commit:** 22e23a807626c83cb024fc41c699b6ea4efc1b22

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).


### Review verdict — 2026-07-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** AEON-7/Qwen3.6-27B-AEON
- **Scope deviations:** none of substance. Acceptance criterion "grep `invalid
  arguments` returns nothing" is met for **production** (zero
  `format!("invalid arguments…")` arms remain); the 15 remaining hits are
  test negative-assertions (`!err.contains("invalid arguments: …")`) proving
  the old string is gone — the same resolution phase-01 reached. `registry.rs`
  reused unchanged (0-line diff); `symbols` correctly uses empty-required →
  type-mismatch branch.
- **Calibration:** none new. **Live-serve note (the point of this dispatch):**
  the server-authored finalize that ran produced the doubled-pipe `| review ||`
  because the connected `rexymcp serve` (PID 529216, started 21:22:14) predated
  the M32-fixed binary (built 21:45:53) — a `/mcp` reconnect reattaches the
  client but does **not** restart the serve subprocess (the
  `stale-rexymcp-serve-after-rebuild` pattern). Row hand-repaired; stale serve
  killed so the fixed binary serves next. The **fix is verified correct** on the
  fresh build: `cargo test -p rexymcp finalize` (24 tests incl. the real-TempDir
  `finalize_flips_status_and_appends_entry` integration test) is green, and M32
  mutation-proved that reverting the fix fails 4/6 flip tests.

Independent re-run: all four gates green (fmt clean, build zero warnings, clippy
clean, 517 mcp + 960 executor tests, 2 ignored). All 10 arg-parsing tools now
return the `missing_args_hint` recovery message; per-tool + non-object-panic
tests added; the two breaking `bash`/`read_file` assertions updated to the new
shape.
