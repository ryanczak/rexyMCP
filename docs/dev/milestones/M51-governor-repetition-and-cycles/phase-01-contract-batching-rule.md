# Phase 01: Contract batching rule & classifier coverage

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress
**Depends on:** none
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Teach the local executor to batch repeated shell work (stress loops, file
sweeps) in a single shell loop or `$TMPDIR` script instead of repeating one
tool call at a time, and pin that the bash classifier allows those worked
examples without weakening its kill-loop blocks. Implements Proposal 1 of
[issue #19](https://github.com/ryanczak/rexyMCP/issues/19).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md`.
- `docs/dev/issues/2026-11-24-executor-never-batches-repeated-commands.md` § Proposal 1.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/templates/executor_contract.md` is embedded into the binary by
`executor/src/agent/contract.rs` (`assemble_executor_contract`), which has
unit tests including `placeholder_set_is_exactly_the_four_authorized` and
`unset_command_renders_as_unconfigured_sentinel`. `executor/src/security/bash_classify.rs`
classifies bash commands (`classify` -> `Severity::{Allow, Block, RefuseInPlaceEdit}`).

## Spec

1. **Add the batching rule to the executor contract** — in
   `executor/templates/executor_contract.md`, under `## Hard rules — non-negotiable`
   (alongside the existing tool-usage guidance), add a `### Batch repeated work`
   subsection carrying the rule and both worked examples from issue #19:
   - Rule: if you would run the same or nearly the same command more than 3
     times (a stress loop, a sweep over files, one check per case), write one
     shell loop, or a script in `$TMPDIR`, that does all of it. Run it **once**
     and paste its summary line. Never repeat a tool call to collect
     repetitions.
   - Example 1 (parallel stress loop printing the failure count):
     `BIN=$(cargo test --lib --no-run 2>&1 | grep -o 'target/debug/deps/<crate>-[0-9a-f]*' | head -1)`
     followed by
     `seq 300 | xargs -P 24 -I{} sh -c "$BIN --exact <test> -q >/dev/null 2>&1 || echo F" | grep -c F`.
   - Example 2 (one check per file, one summary):
     `for f in src/**/*.rs; do grep -q 'pattern' "$f" || echo "missing: $f"; done`.
   - Constraint: do not introduce any new `{…_COMMAND}` placeholders in the
     template (so `unset_command_renders_as_unconfigured_sentinel` stays exact).
2. **Pin the contract rule and handle empty `{}` in the placeholder guard** —
   in `executor/src/agent/contract.rs`:
   - Gotcha: `placeholder_set_is_exactly_the_four_authorized` currently checks
     `inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')`, which is
     vacuously `true` for empty `{}` in `xargs -P 24 -I{}`. Tighten that check
     so empty `{}` is not treated as an unauthorized placeholder, while any
     non-empty `{FOO}` placeholder is still caught.
   - Add the contract snapshot test named in § Test plan.
3. **Pin bash classifier acceptance of the batching examples** — in
   `executor/src/security/bash_classify.rs`, add tests verifying `classify`
   returns `Severity::Allow` for both worked examples and for writing/running a
   `$TMPDIR` script, and `Severity::Block` when a `for` or `xargs -P` loop
   executes a blocked kill command.
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `executor/templates/executor_contract.md` contains the
      `### Batch repeated work` heading and both worked examples (`xargs -P 24`
      stress loop and `for f in src/**/*.rs` file sweep).
- [ ] Every test named in § Test plan passes.
- [ ] Preservation: every existing test passes (`placeholder_set_is_exactly_the_four_authorized`
      updated only for empty `{}` as specified; no other existing test weakened).
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/agent/contract.rs`:
  - `contract_contains_batch_repeated_work_rule` — asserts
    `assemble_executor_contract(&CommandConfig::default())` contains
    `### Batch repeated work`, the `xargs -P 24` stress-loop example, the
    `for f in src/**/*.rs` file-sweep example, and `$TMPDIR`.
  - `placeholder_set_is_exactly_the_four_authorized` (tightened) — passes with
    empty `{}` in the template, and still detects a non-empty unauthorized
    placeholder.
- In `executor/src/security/bash_classify.rs`:
  - `allows_contract_batching_examples` — asserts `classify(...) == Severity::Allow`
    for (a) the two-line `BIN=$(cargo test --lib --no-run ...)` + `seq 300 | xargs -P 24 -I{} sh -c ...`
    stress loop, (b) `for f in src/**/*.rs; do grep -q 'pattern' "$f" || echo "missing: $f"; done`,
    and (c) writing and running a temporary script in `$TMPDIR` via heredoc + `sh "$TMPDIR/..."`.
  - `blocks_kill_inside_batching_examples` — negative case asserting
    `classify(...) == Severity::Block` for `seq 10 | xargs -P 4 -I{} sh -c "kill -9 {}"`
    and `for p in 1 2; do pkill -x bash; done`.

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-01.txt; : > "$A"
grep -n "### Batch repeated work" executor/templates/executor_contract.md >> "$A"; echo "heading exit=$?" >> "$A"
for t in contract_contains_batch_repeated_work_rule placeholder_set_is_exactly_the_four_authorized allows_contract_batching_examples blocks_kill_inside_batching_examples; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
```

Expected: the heading grep matches with `exit=0`, every listed test prints `ok`
with `exit=0`, and each `test result` line shows a non-zero passed count.

## Authorizations

- `executor/templates/executor_contract.md`
- `executor/src/agent/contract.rs`
- `executor/src/security/bash_classify.rs`

No new dependencies.

## Out of scope

- Governor normalization, repeat nudges, cycle detection, config knobs, or
  telemetry (Phases 02–05).
- Changing `bash_classify` production rules unless a worked example is
  unexpectedly blocked (if so, stop and file a blocker).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-24 17:11 (progress)

Started phase 01. Read STANDARDS.md, the phase doc, and the milestone README;
confirmed a clean tree. Flipping status to in-progress and updating the
milestone README's phase table, then adding the `### Batch repeated work`
subsection to `executor/templates/executor_contract.md`, tightening the
placeholder guard in `executor/src/agent/contract.rs` for empty `{}` in
`xargs -I{}`, and adding the classifier tests in
`executor/src/security/bash_classify.rs`.

### Update — 2026-11-24 17:15 (end-to-end verification)

Ran the phase doc's E2E block verbatim. Positive control: the heading grep on
`executor/templates/executor_contract.md` returns the real line number (174)
with `exit=0` — it would fail with exit 1 if the heading were missing — and
each named test filter reports `1 passed` (non-zero) with `exit=0`, which
would read `0 passed` if the tests did not exist. Artifact pasted verbatim
below from `target/e2e/m51-phase-01.txt`:

```
174:### Batch repeated work
heading exit=0
test agent::contract::tests::contract_contains_batch_repeated_work_rule ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1108 filtered out; finished in 0.00s
contract_contains_batch_repeated_work_rule exit=0
test agent::contract::tests::placeholder_set_is_exactly_the_four_authorized ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1108 filtered out; finished in 0.00s
placeholder_set_is_exactly_the_four_authorized exit=0
test security::bash_classify::tests::allows_contract_batching_examples ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1108 filtered out; finished in 0.01s
allows_contract_batching_examples exit=0
test security::bash_classify::tests::blocks_kill_inside_batching_examples ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1108 filtered out; finished in 0.01s
blocks_kill_inside_batching_examples exit=0
```

