# Phase 01: M48 test follow-ups

**Milestone:** M49 — Light-shape phase trial
**Status:** in-progress
**Depends on:** none
**Estimated diff:** ~15 lines, test code only
**Tags:** language=rust, kind=test, size=s

## Goal

Two tests shipped by M48 are weaker than they should be. Fix both.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc.
3. Confirm a clean tree.

## Spec

1. **Make the boundary regression test exercise the retired routing** — in
   `executor/src/tools/bash.rs`, `piped_cargo_style_output_is_not_filtered`
   runs a `sh -c` pipeline. The cargo filter M48 deleted only ever acted on
   commands whose text began with `cargo`, so this test passes against the
   pre-M48 code too and guards nothing. Change the command so it begins with
   `cargo` and still produces the same three lines through the same `grep`
   (a leading `cargo --version >/dev/null && ` is one way; the test must
   stay hermetic, so nothing may depend on a Cargo project existing in the
   temp dir). Assertions unchanged.
2. **Delete dead setup** — in `executor/src/agent/tests.rs`,
   `repeated_identical_bash_trips_hard_fail` builds a tool registry that is
   never passed to the run. Remove it and any import that becomes unused.
   The test's assertions are unchanged and must still pass.
3. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`. The
   server-authored `(complete)` entry does not satisfy this.

## Acceptance criteria

- [ ] `piped_cargo_style_output_is_not_filtered` passes, and its command
      string begins with `cargo`.
- [ ] `repeated_identical_bash_trips_hard_fail` passes, and the function
      body constructs no `ToolRegistry`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m49-phase-01.txt; : > "$A"
cargo test -p rexymcp-executor piped_cargo_style_output_is_not_filtered 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor repeated_identical_bash_trips_hard_fail 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
grep -A6 'async fn piped_cargo_style_output_is_not_filtered' executor/src/tools/bash.rs | grep -o '"command": "[^ ]*' >> "$A"
```

Expected: both tests `ok` with `exit=0`, and the last line shows the
command string starting with `cargo`.

## Authorizations

None.

## Out of scope

- Production code. This phase touches test modules only.
- Any other test in either file.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-10-08 17:05 (progress)

Started phase-01 (executor: GLM-5.3-Flash-NVFP4-Spark under rexyMCP). Flipped
phase doc Status and the milestone README's phase-01 row to `in-progress`.
Verified the pre-M48 routing claim against commit `2860dd2^` before editing:
the old `filter_for_command` dispatched to `cargo_filter` when
`is_cargo_command` matched, i.e. the deleted routing acted only on command
text beginning with `cargo` (after trim), and the old `bash.rs` metadata
labeled that path `"cargo"` rather than `"generic"` — so the fix is to make
the test's command string begin with `cargo`. Implementing Spec §1 then §2.
