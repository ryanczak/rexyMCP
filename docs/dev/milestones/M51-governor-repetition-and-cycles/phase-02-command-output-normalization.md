# Phase 02: Shared command-plumbing & output-timing normalizer

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** done
**Depends on:** phase-01
**Estimated diff:** ~220 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Provide pure, shared normalizers for bash trailing output plumbing and
tool-output timing fields so Phases 03 (repeat nudge) and 04 (`ReadOnlyCycle`)
reuse one canonical definition, and wire command-plumbing normalization into
`normalize_target` without altering `check_identical_repetition`'s exact-match
hard-fail behavior.

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Notes.
- `docs/dev/issues/2026-11-24-executor-never-batches-repeated-commands.md` § Proposal 2.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/src/governor/hard_fail.rs` defines `normalize_arguments` (recursive
whitespace collapse on JSON string leaves, used by `check_identical_repetition`)
and `normalize_target` (which for `Category::Run` / `bash` only strips ASCII
digits from `command`). `executor/src/tools/bash.rs` prepends a wall-clock
status header (`✓ exit 0 (0.3s)` / `✗ exit 1 (1.2s)`) to every bash tool
output. `executor/src/governor/mod.rs` re-exports the governor modules.

## Spec

1. **Create `executor/src/governor/normalize.rs` and export it from
   `executor/src/governor/mod.rs`** — expose three pure `pub fn` helpers
   (using existing crate dependencies such as `regex` / `std::sync::LazyLock`;
   no new dependencies, no `.unwrap()` on untrusted input):
   - **`normalize_bash_command(cmd: &str) -> String`** — collapses whitespace
     runs, iteratively strips trailing output-plumbing segments from the right
     until stable, and normalizes trivial single-vs-double quoting on simple
     tokens:
     - Trailing temp-file capture-and-cat forms: `> /tmp/<name> && cat /tmp/<name>`,
       `> /tmp/<name>; cat /tmp/<name>`, and `| tee /tmp/<name>` (including
       `$TMPDIR/...` equivalents and quoted paths, with optional `2>&1` before
       or after the redirect).
     - Trailing output-slicing / counting pipes: `| tail ...`, `| head ...`,
       `| wc ...` (e.g. `| wc -l`), `| cat`.
     - Trailing stream redirects: `2>&1`, `2>/dev/null`, `>/dev/null`.
     - Quoting differences on simple literal tokens (e.g. `grep -rn "refused"`
       and `grep -rn 'refused'` normalize equal).
     - **Must NOT strip:** substantive pipeline stages (`| grep ...`,
       `| awk ...`, `| sort ...`, `| xargs ...`), redirects to non-temp
       project paths (`> src/foo.rs`), or distinct base command arguments.
   - **`normalize_tool_args_for_repeat(tool: &str, args: &serde_json::Value) -> serde_json::Value`** —
     for `"bash"`, normalizes all string leaves via whitespace collapse and
     normalizes the `"command"` string leaf with `normalize_bash_command`. For
     every other tool (`read_file`, `search`, `patch`, …), delegates to
     whitespace normalization (`normalize_arguments`) so distinct file slices
     and search patterns stay distinct.
   - **`normalize_output_for_repeat(output: &str) -> String`** — strips
     volatile wall-clock timing fields so identical outcomes with different
     durations compare equal:
     - `bash.rs` status-line elapsed durations (`✓ exit 0 (0.3s)` vs
       `✓ exit 0 (1.8s)` -> equal; `✓ exit 0 (0.3s)` vs `✗ exit 1 (0.3s)` ->
       **not** equal).
     - Test/build timing clauses such as `finished in 0.00s` / `finished in 1.42s`
       and `Finished ... in 0.12s`.
     - **Must NOT strip:** exit codes, test pass/fail counts (`1 passed; 0 failed`
       vs `0 passed; 1 failed`), error messages, or arbitrary non-timing output.
2. **Wire `normalize_bash_command` into `normalize_target`** — in
   `executor/src/governor/hard_fail.rs`:
   - In `normalize_target` for `Some(Category::Run)`, pass `cmd` through
     `normalize_bash_command` before stripping ASCII digits, so re-plumbed
     bash variants (e.g. `grep -rn "refused" f.rs | wc -l` vs
     `grep -rn "refused" f.rs | head -2`) collapse to a single target in
     `measure_novelty` and `rexymcp calibrate-governor`.
   - **Constraint:** leave `check_identical_repetition` calling
     `normalize_arguments` directly — re-plumbed bash variants must **not**
     trigger `IdenticalToolCallRepetition`.
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `executor/src/governor/normalize.rs` exists and `pub mod normalize;` is
      exported from `executor/src/governor/mod.rs`.
- [ ] Zero `.unwrap(` calls appear in production code in
      `executor/src/governor/normalize.rs` (`grep -n '\.unwrap(' executor/src/governor/normalize.rs`
      exits non-zero with no matches; `LazyLock<Regex>` uses `.expect("...")`
      with an invariant message per `STANDARDS.md` §2.1).
- [ ] `normalize_bash_command` strips trailing `> /tmp/... && cat /tmp/...`,
      `> "$TMPDIR/..." && cat "$TMPDIR/..."`, and `| tee /tmp/...` even without
      a preceding `| tail`, and **never** strips non-temp paths (`src/...`) or
      substantive stages/redirects that follow `| cat` / `| tail` / `| head` /
      `| wc`.
- [ ] Every test named in § Test plan passes (including the two bug-02-1 tests).
- [ ] Preservation: `check_identical_repetition` still uses exact
      whitespace-only normalization (`identical_repetition_does_not_collapse_output_plumbing`
      passes) and every existing test passes unmodified.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/governor/normalize.rs`:
  - `strips_trailing_tail_head_wc_and_stderr_plumbing` — asserts `cmd | tail -4`,
    `cmd | tail -4 > /tmp/x && cat /tmp/x`, `cmd 2>&1 | tail -6`,
    `cmd | head -2`, `cmd | wc -l`, and `cmd > /tmp/x; cat /tmp/x` all
    normalize to the same string as `cmd`.
  - `strips_temp_capture_and_cat_with_double_ampersand_and_tmpdir` — asserts
    `cmd > /tmp/x && cat /tmp/x`, `cmd > "$TMPDIR/x" && cat "$TMPDIR/x"`,
    `cmd 2>&1 > /tmp/x && cat /tmp/x`, and `cmd | tee /tmp/x` all normalize to
    the same string as `cmd` (with no preceding `| tail`).
  - `does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe` —
    negative cases: `echo foo | cat | grep bar` != `echo foo | cat | grep baz`;
    `cargo test | tail -4 > src/a.txt` != `cargo test | tail -4 > src/b.txt`;
    `echo a > src/x.rs; cat src/x.rs` != `echo a > src/y.rs; cat src/y.rs`;
    `cargo test | tee src/a.txt` != `cargo test | tee src/b.txt`.
  - `normalizes_simple_quote_differences` — asserts
    `grep -rn "refused" src/a.rs | wc -l` and
    `grep -rn 'refused' src/a.rs | head -2` normalize equal.
  - `preserves_substantive_pipes_and_non_temp_redirects` — negative cases:
    `cargo test | grep foo` != `cargo test | grep bar`; `echo a > src/x.rs` !=
    `echo a > src/y.rs`; `cargo test foo | tail -4` != `cargo test bar | tail -4`.
  - `tool_args_normalizer_only_strips_plumbing_for_bash` — asserts bash
    `"command"` strips plumbing while `read_file` line ranges and `search`
    patterns remain distinct.
  - `output_normalizer_ignores_bash_status_and_cargo_timings` — asserts
    outputs differing only in `✓ exit 0 (0.3s)` vs `✓ exit 0 (2.1s)` and
    `finished in 0.00s` vs `finished in 0.45s` normalize equal.
  - `output_normalizer_preserves_exit_code_and_test_counts` — negative cases:
    `✓ exit 0 (0.3s)` != `✗ exit 1 (0.3s)`; `1 passed; 0 failed` !=
    `0 passed; 1 failed`.
- In `executor/src/governor/hard_fail.rs`:
  - `novelty_collapses_replumbed_bash_grep_variants` — alternating
    `grep -rn "refused" src/a.rs | wc -l`, `grep -rn "refused" src/a.rs | head -2`,
    `grep -rn "kind" src/b.rs | wc -l`, `grep -rn "kind" src/b.rs | head -2`
    over a 24-call window yields `distinct_targets: 2` (down from 4).
  - `identical_repetition_does_not_collapse_output_plumbing` — 6 calls cycling
    `cmd | tail -4`, `cmd | tail -4 > /tmp/x && cat /tmp/x`, and
    `cmd 2>&1 | tail -6` do **not** trip `IdenticalToolCallRepetition`.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-02.txt; : > "$A"
grep -n '\.unwrap(' executor/src/governor/normalize.rs >> "$A"; echo "no_unwrap exit=$?" >> "$A"
for t in strips_trailing_tail_head_wc_and_stderr_plumbing strips_temp_capture_and_cat_with_double_ampersand_and_tmpdir does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe normalizes_simple_quote_differences preserves_substantive_pipes_and_non_temp_redirects tool_args_normalizer_only_strips_plumbing_for_bash output_normalizer_ignores_bash_status_and_cargo_timings output_normalizer_preserves_exit_code_and_test_counts novelty_collapses_replumbed_bash_grep_variants identical_repetition_does_not_collapse_output_plumbing; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: `no_unwrap exit=1` (zero `.unwrap(` matches in `normalize.rs`), every
listed test prints `ok` with `exit=0`, and each `test result` line shows a
non-zero passed count.

## Authorizations

- `executor/src/governor/normalize.rs` (new module)
- `executor/src/governor/mod.rs`
- `executor/src/governor/hard_fail.rs`

No new dependencies.

## Out of scope

- Repeat-nudge injection, `repeat_nudge_threshold`, `SessionEvent::NudgeInjected`,
  and telemetry (Phase 03).
- `ReadOnlyCycle` detector and `calibrate-governor` replay additions (Phase 04).
- Changing `check_identical_repetition` or `check_oscillation` semantics.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-25 02:14 (started, resume after bug-02-1)

**Executor:** Executor (phase-02 fix run)

Resuming phase-02 after bounce on [bug-02-1.md](bugs/bug-02-1.md). Rewrote the
`strip_trailing_plumbing` regexes: `TEMP_CAT` now matches `&&`/`;` separators
and restricts capture-and-cat and `tee` paths to `/tmp/...` / `$TMPDIR/...`
(via an `is_temp_path` guard); `TAIL_PIPE` no longer consumes past the trailing
stage (`[^|;&>]*$`), so substantive stages and non-temp redirects after
`| tail` / `| cat` survive. All ten `.unwrap()` calls in
`executor/src/governor/normalize.rs` are replaced with `.expect(...)` messages
stating the compile-time-literal invariant; the two `caps.get(0).unwrap()`
calls are removed entirely (the capture path is now taken from named capture
groups with a `filter_map`, no indexing panic). The two bug-02-1 tests
(`strips_temp_capture_and_cat_with_double_ampersand_and_tmpdir`,
`does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe`) and the
full prior test set pass.

### Update — 2026-11-25 02:20 (end-to-end verification)

Ran the phase doc's E2E block verbatim against the real artifacts
(`executor/src/governor/normalize.rs`, `executor/src/governor/hard_fail.rs`).
Artifact file: `target/e2e/m51-phase-02.txt`, contents as captured mechanically:

```
no_unwrap exit=1
test governor::normalize::tests::strips_trailing_tail_head_wc_and_stderr_plumbing ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
strips_trailing_tail_head_wc_and_stderr_plumbing exit=0
test governor::normalize::tests::strips_temp_capture_and_cat_with_double_ampersand_and_tmpdir ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
strips_temp_capture_and_cat_with_double_ampersand_and_tmpdir exit=0
test governor::normalize::tests::does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe exit=0
test governor::normalize::tests::normalizes_simple_quote_differences ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
normalizes_simple_quote_differences exit=0
test governor::normalize::tests::preserves_substantive_pipes_and_non_temp_redirects ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
preserves_substantive_pipes_and_non_temp_redirects exit=0
test governor::normalize::tests::tool_args_normalizer_only_strips_plumbing_for_bash ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
tool_args_normalizer_only_strips_plumbing_for_bash exit=0
test governor::normalize::tests::output_normalizer_ignores_bash_status_and_cargo_timings ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
output_normalizer_ignores_bash_status_and_cargo_timings exit=0
test governor::normalize::tests::output_normalizer_preserves_exit_code_and_test_counts ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
output_normalizer_preserves_exit_code_and_test_counts exit=0
test governor::hard_fail::tests::novelty_collapses_replumbed_bash_grep_variants ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
novelty_collapses_replumbed_bash_grep_variants exit=0
test governor::hard_fail::tests::identical_repetition_does_not_collapse_output_plumbing ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1118 filtered out; finished in 0.00s
identical_repetition_does_not_collapse_output_plumbing exit=0
```

`no_unwrap exit=1` (zero `.unwrap(` matches in `normalize.rs` — the bug-02-1
Definition-of-done item that failed with 10 matches yesterday), all ten listed
tests print `ok` with `exit=0`, and every `test result` line shows a non-zero
passed count (1 passed each). Positive control: the negative-case tests
(`does_not_strip_non_temp_capture_or_tee_or_stages_after_tail_pipe`,
`preserves_substantive_pipes_and_non_temp_redirects`,
`output_normalizer_preserves_exit_code_and_test_counts`) assert `!=` on inputs
the buggy regexes collapsed yesterday (`echo foo | cat | grep bar` vs `... baz`,
`cargo test | tail -4 > src/a.txt` vs `... src/b.txt`), so a regression to the
bug-02-1 behavior fails them; `novelty_collapses_replumbed_bash_grep_variants`
asserts `distinct_targets == 2`, which a no-op would report as 4.

Gate results (separate invocations): `cargo fmt --all --check` clean;
`cargo build` — `Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.12s`;
`cargo clippy --all-targets --all-features -- -D warnings` — `Finished` with no
warnings; `cargo test` — 708 + 2 + 1117 passed, 0 failed (doc-tests included).

### Update — 2026-11-25 01:30 (started)

**Executor:** Executor (phase-02 run)

Status flipped to `in-progress` and the milestone README phase table updated. Beginning the Spec: new `executor/src/governor/normalize.rs` with the three pure normalizers, wiring `normalize_bash_command` into `normalize_target`, and the test-plan suites in both files.

### Update — 2026-11-25 01:55 (end-to-end verification)

Ran the phase doc's E2E block verbatim against the real artifacts
(`executor/src/governor/normalize.rs`, `executor/src/governor/hard_fail.rs`).
Artifact file: `target/e2e/m51-phase-02.txt`, contents as captured mechanically:

```
test governor::normalize::tests::strips_trailing_tail_head_wc_and_stderr_plumbing ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
strips_trailing_tail_head_wc_and_stderr_plumbing exit=0
test governor::normalize::tests::normalizes_simple_quote_differences ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
normalizes_simple_quote_differences exit=0
test governor::normalize::tests::preserves_substantive_pipes_and_non_temp_redirects ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
preserves_substantive_pipes_and_non_temp_redirects exit=0
test governor::normalize::tests::tool_args_normalizer_only_strips_plumbing_for_bash ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
tool_args_normalizer_only_strips_plumbing_for_bash exit=0
test governor::normalize::tests::output_normalizer_ignores_bash_status_and_cargo_timings ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
output_normalizer_ignores_bash_status_and_cargo_timings exit=0
test governor::normalize::tests::output_normalizer_preserves_exit_code_and_test_counts ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
output_normalizer_preserves_exit_code_and_test_counts exit=0
test governor::hard_fail::tests::novelty_collapses_replumbed_bash_grep_variants ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
novelty_collapses_replumbed_bash_grep_variants exit=0
test governor::hard_fail::tests::identical_repetition_does_not_collapse_output_plumbing ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1116 filtered out; finished in 0.00s
identical_repetition_does_not_collapse_output_plumbing exit=0
```

Every listed test prints `ok` with `exit=0`, and every `test result` line shows a
non-zero passed count (1 passed each). Positive control: the negative-case tests
(`preserves_substantive_pipes_and_non_temp_redirects`,
`output_normalizer_preserves_exit_code_and_test_counts`) assert `!=` on inputs
the normalizer must keep distinct, so a no-op normalizer would fail them; and
`novelty_collapses_replumbed_bash_grep_variants` asserts `distinct_targets == 2`,
which a no-op would report as 4.

Gate results (separate invocations): `cargo fmt --all` clean;
`cargo build` — `Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.49s`;
`cargo clippy --all-targets --all-features -- -D warnings` — `Finished` with no
warnings; `cargo test` — 708 + 1115 passed, 0 failed (plus 2 doc-tests).

### Update — 2026-11-25 01:42 (complete, server-authored)

**Summary:** </think>Committed and clean. Final summary.</think>
**Summary**

Phase 02 is complete and committed (`feat:` commit `9bf43bc`, working tree clean). I created `executor/src/governor/normalize.rs` exporting the three pure normalizers — `normalize_bash_command` (fixed-point trailing plumbing strip for temp-file capture-and-cat, `tail`/`head`/`wc`/`cat` pipes, `2>&1`/`/dev/null` redirects, plus simple quote normalization), `normalize_tool_args_for_repeat` (full plumbing normalization for bash `command`, `normalize_arguments` delegation for every other tool), and `normalize_output_for_repeat` (strips bash status-line durations and cargo `in <t>s` clauses while preserving exit codes and pass/fail counts). `normalize_target` for `Category::Run` now passes commands through `normalize_bash_command` before digit-stripping, and `check_identical_repetition` still calls `normalize_arguments` directly. All eight tests from the spec's test plan pass, all four gates are green (`fmt`, `build`, `clippy -D warnings`, `test`: 708 + 1115 passed, 0 failed), and the E2E block ran verbatim with its artifact pasted into a dedicated `(end-to-end verification)` Update Log entry, including positive-control justification.

**Notes for review**

- One adaptation: `normalize_arguments` was widened from private to `pub(crate)` so the new module can reuse it as the non-bash delegate — `check_identical_repetition`'s behavior is unchanged.
- The phase status remains `in-progress` with no hand-written `(complete)` entry, per the executor contract — the server authors that tail.
- Mid-run I used a temporary throwaway test module to debug why the `caps.name("1")` group lookups never matched; it was removed before commit and no debug code remains (`git status` clean, `lib.rs` restored).

**Executor:** GLM-5.3-Flash

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
ools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::symbols::tests::references_exclude_substring ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test agent::tests::bash_edit_resets_read_only_stall ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test store::telemetry::tests::append_is_atomic_under_concurrent_appenders ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1115 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.11s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` — +1 -1
- `docs/dev/milestones/M51-governor-repetition-and-cycles/phase-02-command-output-normalization.md` — +53 -1
- `executor/src/governor/hard_fail.rs` — +62 -4
- `executor/src/governor/mod.rs` — +4 -0
- `executor/src/governor/normalize.rs` — +239 -0

**Commit:** 9bf43bcd6da9bac133abe80402989a4663949e09

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Update — 2026-11-25 02:21 (complete, server-authored)

**Summary:** </think>Wait — the grep for spec-pinned literals: nothing pinned. Also I should verify fmt --check passed after the last write to normalize.rs — I ran cargo fmt --all before git add, good. Gates all ran. Status remains in-progress, no (complete) entry written — correct.

Done. Final summary.</think>
**Summary**

Bug-02-1 is fixed and committed (`fix:` commit `c0d44fc`, working tree clean). In `executor/src/governor/normalize.rs`: `TEMP_CAT` now matches `&&`/`;` separators and restricts capture-and-cat and `| tee` targets to `/tmp/...` / `$TMPDIR/...` paths via an `is_temp_path` guard; `TAIL_PIPE` is anchored with `[^|;&>]*$` so it strips only a genuinely trailing stage and preserves substantive stages (`| grep`) and non-temp redirects after `| tail`/`| cat`; all ten production `.unwrap()` calls are gone — the `LazyLock<Regex>` initializers use `.expect(...)` with invariant messages and the two `caps.get(0).unwrap()` indexings were removed in favor of `filter_map`-based named-group extraction. Both bug-02-1 tests and the full prior test set pass, all four gates are green as separate invocations (`fmt --check` clean, `build`, `clippy -D warnings`, `test`: 708 + 2 + 1117 passed, 0 failed), and the E2E block ran verbatim with the artifact (`no_unwrap exit=1`, all ten tests `ok`/`exit=0`) pasted into a new `(end-to-end verification)` Update Log entry with positive-control justification. Phase status left at `in-progress`; no hand-written `(complete)` entry.

**Notes for review**

- The plumbing strip now runs to a fixed point, so multi-part plumbing (`cmd 2>&1 | tail -6 > /tmp/x && cat /tmp/x`) peels one segment per pass; `normalize_target` wiring and `check_identical_repetition` semantics from the original run are untouched.
- One behavior worth noting: the negative test `echo foo | cat | grep bar` vs `... grep baz` passes because `| cat | grep …` keeps everything from `| grep` — the intermediate `| cat` before another pipe is left in place, matching the spec's "must not strip stages that follow `| cat`".

**Executor:** GLM-5.3-Flash

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


TEST
sts::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::symbols::tests::references_finds_call_sites ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test agent::tests::bash_edit_resets_read_only_stall ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test store::telemetry::tests::append_is_atomic_under_concurrent_appenders ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1117 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.11s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-16cf1b6da1e99042)
     Running tests/readme_config_reference.rs (target/debug/deps/readme_config_reference-764277bf47a28483)
     Running unittests src/lib.rs (target/debug/deps/executor-5472e17a8f71c962)
   Doc-tests executor

```

**Files changed:**

- `docs/dev/milestones/M51-governor-repetition-and-cycles/phase-02-command-output-normalization.md` — +75 -0
- `executor/src/governor/normalize.rs` — +112 -31

**Commit:** c0d44fcedd044e5db2a3fd2e06a670b297137673

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-11-24

- **Verdict:** approved_after_1
- **Bounces:** 1 (bugs: bug-02-1 — major)
- **Executor:** GLM-5.3-Flash
- **Scope deviations:** none (`normalize_arguments` widened from private to `pub(crate)` for reuse; accepted).
- **Calibration:** leaked `</think>` fragment ahead of `**Summary:**` in `completion_summary` reached 3rd occurrence (M50 phase-03, M51 phase-01, M51 phase-02 — candidate runtime fix in `strip_think_blocks`, held for human sign-off at milestone close); round-2 Update Log entries were prepended above round-1 entries rather than appended at the bottom (1st occurrence, held).
