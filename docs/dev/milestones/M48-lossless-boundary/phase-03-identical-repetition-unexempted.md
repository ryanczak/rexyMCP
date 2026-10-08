# Phase 03: Identical repetition fires on non-mutating windows

**Milestone:** M48 — Lossless Boundary, Loud Backstop, Exact Repetition
**Status:** todo
**Depends on:** none
**Estimated diff:** ~110 lines (a four-line deletion and a doc comment in `hard_fail.rs`; two tests inverted, two added; one agent-level test)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`identical_call_threshold` byte-identical consecutive tool calls trip
`IdenticalToolCallRepetition` whether or not the window contains a
file-mutating call. The non-mutating exemption stays on the oscillation
detector, which is the one the `sed -n` inspection loops needed.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #48 — the decision and what it reverses.
- `docs/architecture.md` § Status #37 — the exemption as shipped, and its
  scope note (`bash` counts as non-mutating).
- `docs/dev/milestones/M48-lossless-boundary/README.md` § Exit criteria,
  third bullet, and § Notes on the partial reversal.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Line numbers are current as of drafting (2026-10-08, master `f7613e4`);
re-derive with `grep -n` before editing.

**`executor/src/governor/hard_fail.rs`**

- The check (`:184-209`), with the lines that go marked:

  ```rust
  /// Read-only repetitions are exempt — left to `check_read_only_stall`.   // ← delete
  fn check_identical_repetition(
      recent: &VecDeque<ToolCallSnapshot>,
      threshold: usize,
  ) -> Option<HardFailSignal> {
      if recent.len() < threshold {
          return None;
      }
      // Read-only repetition is diagnosis, not thrash — left to check_read_only_stall.   // ← delete
      if !window_has_mutation(recent, threshold) {                                        // ← delete
          return None;                                                                    // ← delete
      }                                                                                   // ← delete
      let last_n: Vec<_> = recent.iter().rev().take(threshold).collect();
      …
  ```

  The doc comment above `:184` (starting `/// Identical repetition: the
  last `threshold` tool calls …`) stays; only the one-line
  "Read-only repetitions are exempt" sentence goes.
- `window_has_mutation` (`:298-305`) has two production callers: `:193`
  (this check) and `:322` (`check_oscillation`). After this phase it has
  one. Its doc comment (`:289-297`) describes the oscillation rationale
  and needs no change.
- `evaluate` (`:158-180`) calls `check_identical_repetition` first; the
  order is pinned by `check_order_repetition_precedes_verifier`.
- `ToolCallSnapshot { tool, arguments, succeeded }`;
  `crate::tools::mutates_files` is `Category::Write` only
  (`tools/router.rs:29`), and `bash` is `Category::Run` (`router.rs:19`).
- The two tests that pin the exemption, to invert:

  ```rust
  // hard_fail.rs:1397-1412
  #[test]
  fn identical_repetition_exempts_read_only_window() {
      let mut recent = VecDeque::new();
      let call = ToolCallSnapshot {
          tool: "read_file".to_string(),
          arguments: serde_json::json!({"path": "a.txt"}),
          succeeded: true,
      };
      let threshold = 6;
      for _ in 0..threshold {
          recent.push_back(call.clone());
      }
      assert!(
          evaluate(&recent, &[], None, &GovernorConfig::default()).is_none(),
          "read-only identical repetition must be exempt"
      );
  }
  ```

  and `identical_repetition_still_exempts_read_only_window` (`:1654-1677`):
  six `read_file` snapshots whose `path` differs only in whitespace
  (`"a.txt"`, `" a.txt"`, `"a.txt "`, `"  a.txt  "`, `"a\n.txt"`,
  `"a .txt"`), asserting `evaluate(…).is_none()`.
- The positive shape to copy — `identical_repetition_still_fires_for_write_tool`
  (`:1444-1462`):

  ```rust
  let signal = evaluate(&recent, &[], None, &GovernorConfig::default())
      .expect("identical repetition must still fire for write tools");
  assert!(matches!(
      signal,
      HardFailSignal::IdenticalToolCallRepetition {
          tool,
          consecutive_count: 6
      } if tool == "write_file"
  ));
  ```

**`executor/src/agent/tests.rs`** — the agent-level shape to copy,
`identical_tool_call_repetition_trips_hard_fail` (`:1366-1389`): six
`native("write_file", …)` turns through `MockAiClientScript`, a
`MockFileVerifier::new(vec![])`, `run_with_verifier(&dir, &client,
&verifier, 10)`, then
`assert_eq!(result.status, PhaseStatus::HardFail)` and a `matches!` on
`result.briefing.unwrap().current_blocker` against
`Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })`.
`native` is at `:178`, `token` at `:174`. The default `GovernorConfig`
(`identical_call_threshold: 6`, `oscillation_window: 8`,
`read_only_stall_threshold: 60`) is what `run_with_verifier` uses, so six
identical calls reach the identical-repetition check before any other
detector can fire.

**`mcp/src/calibrate_governor.rs`** — `Signal::IdenticalRun` (`:120-135`)
counts the longest run of consecutive identical `(tool, arguments)` with
no mutation predicate. It already matches the post-phase live rule. Do not
touch it.

## Spec

### 1. Remove the exemption from `check_identical_repetition`

Delete the four marked lines and the one-line doc sentence shown in
§ Current state. Nothing else in the function changes; `window_has_mutation`
stays for `check_oscillation`.

### 2. Document the asymmetry

Replace the deleted doc sentence with:

```rust
/// Fires regardless of whether the window mutated a file: N byte-identical
/// consecutive calls carry no diagnostic information after the first repeat.
/// The non-mutating exemption applies to `check_oscillation` only.
```

### 3. Invert the two exemption tests

- Rename `identical_repetition_exempts_read_only_window` →
  `identical_repetition_fires_on_read_only_window`; replace the
  `is_none()` assertion with the `expect` + `matches!` shape from
  `identical_repetition_still_fires_for_write_tool`, with
  `tool == "read_file"` and `consecutive_count: 6`.
- Rename `identical_repetition_still_exempts_read_only_window` →
  `identical_repetition_fires_on_whitespace_varied_read_only_window`; same
  assertion change (the whitespace-normalised arguments are identical, so
  it fires with `tool == "read_file"`).

### 4. New `hard_fail.rs` tests

- `identical_repetition_fires_on_repeated_bash_command` — six
  `ToolCallSnapshot { tool: "bash", arguments: json!({"command": "cargo test --lib -- a b 2>&1 | grep -E '^test |^test result'"}), succeeded: true }`
  (the issue's command); `evaluate` with `GovernorConfig::default()`
  returns `IdenticalToolCallRepetition { tool, consecutive_count: 6 }` with
  `tool == "bash"`.
- `identical_repetition_silent_below_threshold_on_read_only_window` — five
  identical `read_file` snapshots; `evaluate` returns `None`. (The negative
  pin: the change removes the exemption, not the threshold.)

### 5. New agent-level test (`agent/tests.rs`)

`repeated_identical_bash_trips_hard_fail`, modelled on
`identical_tool_call_repetition_trips_hard_fail`: six turns of
`native("bash", json!({ "command": "echo same" }))`, then
`token("unreached")`; `MockFileVerifier::new(vec![])`;
`run_with_verifier(&dir, &client, &verifier, 10)`. Assert
`result.status == PhaseStatus::HardFail` and that
`result.briefing.unwrap().current_blocker` matches
`Blocker::HardFail(HardFailSignal::IdenticalToolCallRepetition { .. })`.
`bash` is a registered tool in that harness and `echo` is permitted by the
command classifier; no file is mutated, so this is exactly the window the
shipped rule exempts today.

### 6. Capture the end-to-end evidence

Run the block in § End-to-end verification **verbatim and unmodified**,
paste the artifact file into a new Update Log entry headed
`### Update — <date> (end-to-end verification)`, and append the
`PASTE MATCH` / `PASTE MISMATCH` line the self-check prints. On
`PASTE MISMATCH`, fix the pasted fence from the file and re-run the
self-check until it prints `PASTE MATCH`. The server-authored `(complete)`
entry does not satisfy this task.

## Acceptance criteria

- [ ] Test `identical_repetition_fires_on_read_only_window` passes.
- [ ] Test `identical_repetition_fires_on_whitespace_varied_read_only_window`
      passes.
- [ ] Test `identical_repetition_fires_on_repeated_bash_command` passes.
- [ ] Test `identical_repetition_silent_below_threshold_on_read_only_window`
      passes.
- [ ] Test `repeated_identical_bash_trips_hard_fail` passes.
- [ ] **Preservation criteria:** `oscillation_exempts_read_only_window`,
      `identical_repetition_still_fires_for_write_tool`,
      `identical_tool_call_repetition_trips_hard_fail`,
      `check_order_repetition_precedes_verifier`,
      `read_only_stall_still_terminates_after_exemption`, and
      `identical_run_counts_longest_consecutive_identical`
      (`calibrate_governor.rs`) still pass.
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/governor/hard_fail.rs | grep -c 'window_has_mutation'`
      prints `2` (now 3: definition + two callers → definition + one).
- [ ] `grep -c 'Read-only repetitions are exempt' executor/src/governor/hard_fail.rs`
      prints `0` (now 1).
- [ ] `grep -c 'exempts_read_only_window' executor/src/governor/hard_fail.rs`
      prints `1` (now 3; the surviving one is `oscillation_exempts_read_only_window`).
- [ ] `grep -c '#\[test\]' executor/src/governor/hard_fail.rs` prints `71`
      — (now 69) + 2 named tests (the two inversions are renames in place).
- [ ] `grep -c '#\[tokio::test\]' executor/src/agent/tests.rs` prints `135`
      — (now 134) + 1.
- [ ] The E2E Update Log entry ends with the line `PASTE MATCH`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

In `executor/src/governor/hard_fail.rs` `mod tests`: the two inversions
(Spec §3) and two additions (Spec §4). In `executor/src/agent/tests.rs`:
`repeated_identical_bash_trips_hard_fail` (Spec §5). Names as listed in
§ Acceptance criteria. Each positive test asserts the signal's `tool` and
`consecutive_count`, so a detector that fires on the wrong window cannot
hide behind `is_some()`.

## End-to-end verification

The real artifact is the executor library's governor behaviour, exercised
by the named tests. Run exactly this block from the repo root; it is
Spec's final task:

```bash
mkdir -p target/e2e
A=target/e2e/m48-phase-03.txt; : > "$A"
cargo test -p rexymcp-executor identical_repetition 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor repeated_identical_bash_trips_hard_fail 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp identical_run_counts_longest_consecutive_identical 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
echo "whm_sites=$(sed -n '1,/^#\[cfg(test)\]/p' executor/src/governor/hard_fail.rs | grep -c 'window_has_mutation')" >> "$A"
```

Expected: the first run lists every `identical_repetition_*` test as `ok`
(including the four renamed/new names) with `0 failed`; the second and
third each show one `ok` line and `1 passed`; all three `exit=0`;
`whm_sites=2`.

Paste `target/e2e/m48-phase-03.txt` byte-for-byte into a new Update Log
entry headed `### Update — <date> (end-to-end verification)`, then run the
paste self-check and append its verdict line to the same entry:

```bash
D=docs/dev/milestones/M48-lossless-boundary/phase-03-identical-repetition-unexempted.md
L=$(grep -n '^### Update .*(end-to-end verification)' "$D" | tail -1 | cut -d: -f1)
tail -n +"$L" "$D" | awk '/^```/{c++; next} c==1{print} c==2{exit}' > target/e2e/pasted-m48-03.txt
diff target/e2e/pasted-m48-03.txt target/e2e/m48-phase-03.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

The server-authored `(complete)` entry does not satisfy this section.

## Authorizations

None. No dependencies, no `Cargo.toml`, no config change
(`identical_call_threshold` stays 6).

## Out of scope

- **`check_oscillation`** and its exemption — unchanged. Do not touch
  `window_has_mutation` or its doc comment.
- **`check_read_only_stall`** — unchanged (phase-02 touches only its doc).
- **`mcp/src/calibrate_governor.rs`** — already in agreement; do not touch.
- **`identical_call_threshold`** — no retune, no new knob.
- **`docs/architecture.md`, `README.md`, plugin skills** — architect-owned.
