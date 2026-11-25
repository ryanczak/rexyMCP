# Phase 02: Shared command-plumbing & output-timing normalizer

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress
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
- [ ] Every test named in § Test plan passes.
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
for t in strips_trailing_tail_head_wc_and_stderr_plumbing normalizes_simple_quote_differences preserves_substantive_pipes_and_non_temp_redirects tool_args_normalizer_only_strips_plumbing_for_bash output_normalizer_ignores_bash_status_and_cargo_timings output_normalizer_preserves_exit_code_and_test_counts novelty_collapses_replumbed_bash_grep_variants identical_repetition_does_not_collapse_output_plumbing; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: every listed test prints `ok` with `exit=0`, and each `test result`
line shows a non-zero passed count.

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
