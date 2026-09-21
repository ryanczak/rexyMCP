# Phase 02: Replay refile signal

**Milestone:** M47 — Verifier Persistence Keyed on Re-editing
**Status:** todo
**Depends on:** phase-01 (uses the `VerifySample` + `verifier_persistence_streaks` it exports)
**Estimated diff:** ~180 lines (one field on `RunReplay` + its 6 construction sites, one `Signal` variant with 3 match arms, the `format_report` label list, pairing in `replay`, 5 tests)
**Tags:** language=rust, kind=feature, size=s

## Goal

`rexymcp calibrate-governor` reports a `verifier_refile_run` signal beside
the existing `verifier_persistence_run`, computed by the **same** streak
function the live rule uses. Both columns stay, so every run where the old
and new rules disagree is visible in the report.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #47 — the rule change and why the replay
  keeps both signals.
- `docs/dev/milestones/M47-verifier-persistence-refile/README.md` § Exit
  criteria — the replay-parity requirement.
- `docs/architecture.md` § Status #34 — the calibration framework this
  extends (`Signal` as an extension seam).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Confirm phase-01 landed**:
   `grep -c 'pub fn verifier_persistence_streaks(' executor/src/governor/hard_fail.rs`
   must print `1`. If it prints `0`, stop and file a blocker — this phase
   cannot be built against a tree without it.

## Current state

Everything in this phase is in **`mcp/src/calibrate_governor.rs`** except the
imported API.

**Line numbers below are current as of drafting and you are about to shift
them** — every one is given with the code it names, so match on the quoted
text, not the number. Re-derive any you need with
`grep -n '<quoted text>' mcp/src/calibrate_governor.rs`.

**What phase-01 exports** (`executor/src/governor/hard_fail.rs`), used here
as-is:

```rust
pub struct VerifySample {
    pub author_errors: usize,
    pub path: PathBuf,
}
/// Streak length after each sample (refile semantics).
pub fn verifier_persistence_streaks(samples: &[VerifySample]) -> Vec<usize>;
```

**The replay struct** (`calibrate_governor.rs:19-30`) carries five data
fields today:

```rust
struct RunReplay {
    model: String,
    outcome: String,
    tool_calls: Vec<ToolCallSnapshot>,
    /// Author-attributed verifier error count per `Verify` event, in order.
    verifier_error_counts: Vec<usize>,
    completion_empty: Vec<bool>,
    output_bytes: Vec<usize>,
}
```

**It is constructed in six places** — one production, five in the test module
(which starts at `calibrate_governor.rs:485`). Adding a field breaks all six;
they are listed in § Authorizations:

| Line | Where |
|---|---|
| `:475` | `replay()` — production |
| `:599` | test `novelty_distinct_*` |
| `:639` | test `max_read_only_run_*` |
| `:907` | test (novelty, short input) |
| `:930` | helper `make_replay_with_verify_and_completion` |
| `:1182`, `:1200` | the two `OutputFloodWindowedBytes` tests |

**The existing verifier signal** (`calibrate_governor.rs:144-163`) computes
the **shipped** rule's longest streak from `verifier_error_counts` and
returns one sample per run. It stays exactly as it is:

```rust
Signal::VerifierPersistenceRun => {
    if run_.verifier_error_counts.is_empty() {
        return vec![];
    }
    let mut max = 0usize;
    // ... non-decreasing-positive streak over counts ...
    vec![max]
}
```

**`replay()`** (`calibrate_governor.rs:441-483`) is a linear scan over
`SessionRecord`s in log order. Its two relevant arms:

```rust
SessionEvent::Parsed { tool_call } => tool_calls.push(ToolCallSnapshot {
    tool: tool_call.name.clone(),
    arguments: tool_call.arguments.clone(),
    succeeded: true, // the 06a stall signals key on tool+args, not success
}),
SessionEvent::Verify { diagnostics } => {
    verifier_error_counts.push(diagnostics.len());
}
```

**Two separate lists name the signals, and both must be edited.**
`SIGNALS` (`:192-200`) drives computation; `format_report` (`:224-232`) has
its **own hardcoded `&[&str]`** of labels and prints only signals found in
it:

```rust
const SIGNALS: &[Signal] = &[ /* … 7 variants … */ ];

fn format_report(rows: &[ReportRow]) -> String {
    // …
    let signals: &[&str] = &[
        "novelty_distinct_targets",
        // … 6 more string labels …
    ];
```

**Adding the variant to `SIGNALS` alone computes rows that are never
printed**, and the text report silently looks unchanged. This is the single
most likely way to fail this phase.

## Spec

### 1. Carry the paired samples on `RunReplay`

Add to the imports
(`calibrate_governor.rs:9`, the `rexymcp_executor::governor::hard_fail` line):
`VerifySample`, `verifier_persistence_streaks`. Add `use std::path::PathBuf;`
to the existing `std::path` import.

Add one field to `RunReplay`, after `verifier_error_counts`:

```rust
    /// Per `Verify` event, the author-error count paired with the path of the
    /// most recent preceding verified write. A `Verify` with no preceding
    /// write is dropped (the live loop only verifies after an edit).
    verify_samples: Vec<VerifySample>,
```

Every one of the six construction sites gets `verify_samples` — the
production one from the pairing below, the five test ones `Vec::new()`
(none of them exercises this signal).

### 2. Pair each verify with the preceding write, in `replay()`

Declare `let mut verify_samples = Vec::new();` and
`let mut last_write: Option<PathBuf> = None;` beside the existing
accumulators, extend the two arms, and add the field to the returned struct:

```rust
SessionEvent::Parsed { tool_call } => {
    if is_verified_write(&tool_call.name)
        && let Some(p) = path_arg(&tool_call.arguments)
    {
        last_write = Some(p);
    }
    tool_calls.push(ToolCallSnapshot {
        tool: tool_call.name.clone(),
        arguments: tool_call.arguments.clone(),
        succeeded: true, // the 06a stall signals key on tool+args, not success
    })
}
SessionEvent::Verify { diagnostics } => {
    verifier_error_counts.push(diagnostics.len());
    if let Some(p) = last_write.as_ref() {
        verify_samples.push(VerifySample {
            author_errors: diagnostics.len(),
            path: p.clone(),
        });
    }
}
```

with these two free functions above `replay()`:

```rust
/// The write tools whose edits run the post-edit verifier — mirrors
/// `edit_target` in `executor/src/agent/tools.rs`. Deliberately **not**
/// `tools::mutates_files`, which is a wider set: `patch_lines`, `delete_file`
/// and `move_file` mutate but run no verifier, so pairing on them would
/// attribute a verify to a file that never triggered it.
fn is_verified_write(tool: &str) -> bool {
    tool == "write_file" || tool == "patch"
}

/// The `path` string argument of a logged tool call, if present.
fn path_arg(arguments: &serde_json::Value) -> Option<PathBuf> {
    arguments.get("path")?.as_str().map(PathBuf::from)
}
```

`verifier_error_counts` keeps taking **every** `Verify`, unchanged — the two
signals' populations differ only for a verify with no preceding write, which
the live loop cannot produce.

### 3. The `VerifierRefileRun` signal

Add the variant to `Signal` after `VerifierPersistenceRun`:

```rust
    /// Longest refile streak — the shipped rule re-keyed on re-editing a file
    /// already edited in the streak (M47). Reported beside
    /// `VerifierPersistenceRun` so the two rules' disagreements are visible.
    VerifierRefileRun,
```

Three match arms follow from it:

- `label()` → `"verifier_refile_run"`.
- `direction()` — no arm needed; the `_ => TailDirection::HigherIsWorse`
  catch-all is correct.
- `samples()`:

  ```rust
  Signal::VerifierRefileRun => {
      if run_.verify_samples.is_empty() {
          return vec![];
      }
      let max = verifier_persistence_streaks(&run_.verify_samples)
          .into_iter()
          .max()
          .unwrap_or(0);
      vec![max]
  }
  ```

### 4. Register it in **both** lists

- `SIGNALS` (`:192`) gains `Signal::VerifierRefileRun,` immediately after
  `Signal::VerifierPersistenceRun,`.
- `format_report`'s `signals: &[&str]` (`:224`) gains `"verifier_refile_run",`
  in the same position.

Neither list may be reordered otherwise.

### 5. Tests

Per § Test plan.

### 6. Capture the end-to-end evidence

Run the two blocks in § End-to-end verification **verbatim and unmodified**,
paste the artifact file into a new Update Log entry headed
`### Update — <date> (end-to-end verification)`, and append the
`PASTE MATCH` / `PASTE MISMATCH` line the self-check prints. On
`PASTE MISMATCH`, fix the pasted fence from the file and re-run the
self-check until it prints `PASTE MATCH`. The server-authored `(complete)`
entry does not satisfy this task.

## Acceptance criteria

- [ ] Test `refile_signal_takes_longest_streak` passes.
- [ ] Test `refile_signal_absent_without_verify_samples` passes.
- [ ] Test `replay_pairs_verify_with_preceding_write` passes.
- [ ] Test `replay_ignores_unverified_write_tools_when_pairing` passes.
- [ ] Test `refile_signal_appears_in_report` passes.
- [ ] **Preservation criterion:** every existing test in
      `mcp/src/calibrate_governor.rs` still passes, in particular
      `remaining_signals_appear_in_report` and
      `format_report_labels_oscillation_tail_low`.
- [ ] `grep -c 'verifier_refile_run' mcp/src/calibrate_governor.rs` prints
      `3` or more (now 0) — the `label()` arm, the `format_report` list
      entry, and at least one test assertion.
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' mcp/src/calibrate_governor.rs | grep -c 'verify_samples'`
      prints `5` or more (now 0): the struct field, the accumulator, the
      push, the struct-literal field in `replay`, and the `samples()` read.
      Write the arithmetic out in the Update Log when reporting it.
- [ ] `cargo run -q -p rexymcp -- calibrate-governor --sessions-dir .rexymcp/sessions --min-runs 0`
      prints a `verifier_refile_run` block **and** a
      `verifier_persistence_run` block.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

All in the existing `#[cfg(test)] mod tests` block of
`mcp/src/calibrate_governor.rs`. The `RunReplay`-building tests use the
existing helper style; extend
`make_replay_with_verify_and_completion` (`:925`) with a
`verify_samples: Vec<VerifySample>` parameter, or add a sibling helper —
either is acceptable, but every existing caller must keep compiling.

- `refile_signal_takes_longest_streak` — a run whose `verify_samples` are
  `1@a, 1@a, 1@b, 1@b, 1@b` yields `vec![3]` from
  `Signal::VerifierRefileRun.samples(&run, 24)`.
- `refile_signal_absent_without_verify_samples` — empty `verify_samples`
  yields an empty sample vector (so the run contributes no row).
- `replay_pairs_verify_with_preceding_write` — a synthetic record sequence
  `Verify(2 diags)`, `Parsed(patch path=a.rs)`, `Verify(1 diag)` replays to
  exactly one `VerifySample { author_errors: 1, path: "a.rs" }`: the verify
  before any write is dropped, and `verifier_error_counts` still has **two**
  entries.
- `replay_ignores_unverified_write_tools_when_pairing` — `Parsed(patch
  path=a.rs)`, `Parsed(patch_lines path=b.rs)`, `Verify(1 diag)` pairs the
  verify with `a.rs`, not `b.rs`.
- `refile_signal_appears_in_report` — build a session file with
  `make_session_file` (`:491`), append `Parsed`/`Verify` records the way
  `remaining_signals_appear_in_report` (`:1110`) appends its events, run
  `run(&args)` and assert the output contains `verifier_refile_run`. **This
  is the test that catches registering the signal in only one of the two
  lists.**

## End-to-end verification

The real artifact is the `rexymcp calibrate-governor` CLI against this
repo's own session corpus (`.rexymcp/sessions`, 288 logs at drafting). Run
exactly:

```bash
mkdir -p target/e2e
A=target/e2e/m47-phase-02.txt; : > "$A"
cargo test -p rexymcp refile 2>&1 | tail -20 >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo run -q -p rexymcp -- calibrate-governor --sessions-dir .rexymcp/sessions --min-runs 0 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -A 6 'verifier_' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
echo "refile_label=$(grep -c 'verifier_refile_run' mcp/src/calibrate_governor.rs)" >> "$A"
echo "samples_field=$(sed -n '1,/^#\[cfg(test)\]/p' mcp/src/calibrate_governor.rs | grep -c 'verify_samples')" >> "$A"
```

Expected: the test run reports `0 failed` with `exit=0`; the report excerpt
shows **both** a `verifier_persistence_run` and a `verifier_refile_run`
block with `exit=0`; `refile_label` is 3 or more; `samples_field` is 5 or
more.

Then paste `target/e2e/m47-phase-02.txt` byte-for-byte into a new Update Log
entry headed `### Update — <date> (end-to-end verification)` and run the
paste self-check, appending its verdict line to that same entry:

```bash
D=docs/dev/milestones/M47-verifier-persistence-refile/phase-02-replay-refile-signal.md
L=$(grep -n '^### Update .*(end-to-end verification)' "$D" | tail -1 | cut -d: -f1)
tail -n +"$L" "$D" | awk '/^```/{c++; next} c==1{print} c==2{exit}' > target/e2e/pasted-02.txt
diff target/e2e/pasted-02.txt target/e2e/m47-phase-02.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

## Authorizations

- **May edit all six `RunReplay` construction sites** in
  `mcp/src/calibrate_governor.rs` — `:475` (production, in `replay`), and the
  test-module sites `:599`, `:639`, `:907`, `:930` (the
  `make_replay_with_verify_and_completion` helper), `:1182`, `:1200`. Adding
  a struct field forces every literal to change; these are the complete set
  in the crate (`grep -n 'RunReplay {' mcp/src/calibrate_governor.rs`).
- **May change the signature of the test helper**
  `make_replay_with_verify_and_completion` (`:925`) and update its callers.

No dependencies, no `Cargo.toml`, no config-schema changes, no executor-crate
edits.

## Out of scope

- **`executor/` entirely.** `VerifySample` and
  `verifier_persistence_streaks` are consumed as phase-01 shipped them. If
  the streak semantics look wrong, file a blocker — do not re-implement the
  streak here, and do not "fix" it in the executor crate.
- **`Signal::VerifierPersistenceRun`** stays exactly as it is. The point of
  this phase is having both columns; do not delete, rename or re-key the old
  one.
- **The corpus comparison** against the downstream project's logs is an
  architect check at review — that directory is outside this repo and
  outside the executor's scope. Do not attempt to read or replay it.
- **`tools::mutates_files`** (`:100`) keeps its current single use in the
  read-only classification. Do not route the new pairing through it, and do
  not widen it.
- **Report layout** — column order, percentile choice, the `(all)` summary
  row: unchanged. The new signal is one more block in the existing shape.
- **`rexymcp.toml` / CLI flags** — no new flag gates this signal; it is
  always reported.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
