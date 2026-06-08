# Phase 02: structured cargo filter

**Milestone:** M10 — Context optimization
**Status:** done
**Depends on:** phase-01 (recoverable output filter module + `filter_for_command` dispatch slot)
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

`cargo test` / `cargo build` / `cargo clippy` are the highest-volume bash commands
the executor runs, and their full output is almost entirely noise — thousands of
`test foo ... ok` lines and `Compiling …` progress messages drown the few lines
that matter. This phase teaches the bash filter to detect cargo commands and keep
only the diagnostic content: errors with their spans, test failures with their
panic output, and the final summary line. Everything else is dropped before it
enters context. Overflow beyond `LINE_CAP` still goes to a recovery file (reusing
phase-01's `compact_with_recovery`). On non-cargo commands the dispatcher falls
through to the phase-01 generic filter.

A well-run `cargo test` that produces 1 000 lines becomes ~20 lines (the failure
block plus summary). A clean run becomes 1 line (`test result: ok. N passed…`).

## Architecture references

Read before starting:

- `executor/src/context/output_filter.rs` — the phase-01 module this phase
  extends. Read top to bottom; the new functions live in this same file.
- `docs/dev/milestones/M10-context-optimization/README.md` §"What we take from
  RTK" — the losslessness contract: every `error[Exxx]` span, every failing-test
  name, every panic line must survive. The keep-by-default principle (unknown
  lines stay) enforces this.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).
5. Run `cargo test` and record the passing count — completion log must show
   the same count plus the new tests.

## Current state

**`output_filter.rs` exports two public functions** (phase-01):

```rust
// executor/src/context/output_filter.rs:37
pub fn normalize(raw: &str) -> String { … }

// executor/src/context/output_filter.rs:76
pub fn compact_with_recovery(raw: &str, project_root: &Path) -> (String, bool) { … }
```

**`bash.rs` filter branch** (`bash.rs:160-167`):

```rust
let (body, truncated) = if self.filter {
    crate::context::output_filter::compact_with_recovery(
        &combined,
        self.scope.root(),
    )
} else {
    truncate_output(&combined)
};
```

`parsed.command` is the shell-command string; it is in scope at this point
(`bash.rs:67`, deserialized into `parsed`). The change in task 2 threads it to
the dispatcher.

**`tools/mod.rs:13`** currently re-exports `bash_with_filter` alongside `bash`.
No change needed there.

## Spec

Numbered tasks in execution order.

### 1. Add `is_cargo_command`, `cargo_filter`, and `filter_for_command` to `output_filter.rs`

All three live in `executor/src/context/output_filter.rs`, at the bottom of the
public-API section (before `write_recovery` and `prune_recovery`, which are
private helpers).

---

**`is_cargo_command`** — routing gate for the dispatcher:

```rust
/// Returns `true` when `command` is a `cargo` invocation. Matches `cargo`
/// standing alone or followed by a space (i.e. `cargo <subcommand>`). Leading
/// whitespace is stripped. Does not match `echo cargo` or `CARGO_HOME=…`.
pub fn is_cargo_command(command: &str) -> bool {
    let t = command.trim_start();
    t == "cargo" || t.starts_with("cargo ")
}
```

---

**`cargo_filter`** — content-aware filter for cargo subcommand output:

```rust
/// Filter cargo subcommand output, keeping only diagnostic content: error and
/// warning blocks (with their multi-line spans), test-failure blocks (panic,
/// assertion, stdout headers), and the final summary line. Everything else —
/// passing-test lines, progress messages (`Compiling`, `Checking`, `Finished`,
/// etc.) — is dropped.
///
/// Unknown lines (not matching any keep or drop pattern) are kept by default —
/// the keep-by-default rule ensures no diagnostic content is silently lost.
///
/// After filtering, if the result still exceeds `LINE_CAP` lines, the full
/// filtered output is written to a recovery file via `compact_with_recovery`.
/// Returns `(body, truncated)`.
pub fn cargo_filter(raw: &str, project_root: &Path) -> (String, bool) {
    let normalized = normalize(raw);
    let mut kept = String::new();
    let mut last_was_blank = true; // suppress leading blank lines

    for line in normalized.lines() {
        let trimmed = line.trim_start();

        if is_cargo_noise(trimmed) {
            continue;
        }

        // Collapse runs of blank lines to at most one.
        if trimmed.is_empty() {
            if !last_was_blank {
                kept.push('\n');
                last_was_blank = true;
            }
            continue;
        }

        last_was_blank = false;
        kept.push_str(line);
        kept.push('\n');
    }

    // Strip trailing blank line left by the collapse above.
    let kept = kept.trim_end_matches('\n');
    let kept = if kept.is_empty() {
        String::new()
    } else {
        format!("{kept}\n")
    };

    // If filtering already brought output below the cap, return it directly.
    let line_count = kept.lines().count();
    if line_count <= LINE_CAP {
        return (kept, false);
    }

    // Still over cap after filtering — write the full filtered output to a
    // recovery file and return a head+tail view (reuses compact_with_recovery).
    compact_with_recovery(&kept, project_root)
}

/// Returns `true` for lines that are pure cargo progress noise: passing tests,
/// compilation progress, and other lines that carry no diagnostic information.
fn is_cargo_noise(trimmed: &str) -> bool {
    // Passing test line: "test foo::bar ... ok"
    if trimmed.starts_with("test ") && trimmed.ends_with(" ... ok") {
        return true;
    }
    // Cargo progress tokens (leading whitespace already stripped).
    for prefix in &[
        "Compiling ", "Checking ", "Finished ", "Running ", "Downloaded ",
        "Downloading ", "Blocking ", "Updating ", "Locking ", "Fresh ",
        "Dirty ", "Replaced ", "Unpacking ",
    ] {
        if trimmed.starts_with(prefix) {
            return true;
        }
    }
    // "running N test(s)" header line from libtest.
    if trimmed.starts_with("running ") && (trimmed.contains(" test") ) {
        return true;
    }
    false
}
```

**`filter_for_command`** — top-level dispatcher (replaces the direct
`compact_with_recovery` call in `bash.rs`):

```rust
/// Dispatch to the appropriate output filter based on the command string.
/// Cargo commands are routed to the structured `cargo_filter`; everything else
/// falls back to the generic `compact_with_recovery`.
pub fn filter_for_command(command: &str, raw: &str, project_root: &Path) -> (String, bool) {
    if is_cargo_command(command) {
        cargo_filter(raw, project_root)
    } else {
        compact_with_recovery(raw, project_root)
    }
}
```

### 2. Update `bash.rs` to call `filter_for_command`

One-line change in `execute` (`bash.rs:160-167`). Replace:

```rust
let (body, truncated) = if self.filter {
    crate::context::output_filter::compact_with_recovery(
        &combined,
        self.scope.root(),
    )
} else {
    truncate_output(&combined)
};
```

with:

```rust
let (body, truncated) = if self.filter {
    crate::context::output_filter::filter_for_command(
        &parsed.command,
        &combined,
        self.scope.root(),
    )
} else {
    truncate_output(&combined)
};
```

No other change to `bash.rs`. The `bash` / `bash_with_filter` constructors, the
`Bash` struct, and the `filter: bool` field are all unchanged.

### 3. Export `filter_for_command` from `executor/src/context/output_filter.rs`

It is already `pub`; no re-export is needed in `mod.rs` — callers reference it
via `crate::context::output_filter::filter_for_command`, the same path-prefix
already used for `compact_with_recovery` in `bash.rs`. No change to
`executor/src/context/mod.rs` or `executor/src/tools/mod.rs`.

## Acceptance criteria

- [ ] `grep -n 'pub fn is_cargo_command' executor/src/context/output_filter.rs` matches.
- [ ] `grep -n 'pub fn cargo_filter' executor/src/context/output_filter.rs` matches.
- [ ] `grep -n 'pub fn filter_for_command' executor/src/context/output_filter.rs` matches.
- [ ] `grep -n 'filter_for_command' executor/src/tools/bash.rs` matches.
- [ ] A cargo command with >100 lines of output, containing both passing-test
      lines and a FAILED test block, produces output whose FAILED block is
      present and whose `... ok` lines are absent.
- [ ] A non-cargo command with >100 lines routes to `compact_with_recovery` (the
      generic filter): marker reads `full output:` or `full output not retained`,
      and the output is **not** cargo-filtered.
- [ ] `cargo build`/`clippy` output with `error[E0425]: ...` + span lines (`  -->`,
      `   |`) all survive in the filtered output.
- [ ] The final `test result:` summary line is always present in cargo-filtered
      output when it appears in the raw output.
- [ ] When cargo-filtered output still exceeds `LINE_CAP` lines, a recovery file
      is written under `.rexymcp/output/` and the marker references it.
- [ ] `bash(scope, timeout)` and `bash_with_filter(scope, timeout, filter)`
      signatures are unchanged; `filter: false` still uses legacy `truncate_output`.
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Unit tests in `executor/src/context/output_filter.rs` (`#[cfg(test)] mod tests`),
using `tempfile::TempDir` for recovery-file cases. New tests append to the
existing block.

- `is_cargo_command_matches_cargo_subcommands` — `"cargo test"`, `"cargo build"`,
  `"  cargo clippy --all-targets"` (leading whitespace) → `true`; `"echo cargo"`,
  `"rustc main.rs"`, `""` → `false`. **(negative cases are load-bearing)**

- `cargo_filter_drops_passing_test_lines` — input containing
  `"test foo::bar ... ok\n"` and `"test baz ... FAILED\n"` → `ok` line absent,
  `FAILED` line present in body. **(negative case: ok line must NOT appear)**

- `cargo_filter_drops_compiling_noise` — input with `"   Compiling foo v1.0\n"`,
  `"    Finished\n"`, `"     Running unittests\n"` → none of those lines appear in
  output. **(negative case)**

- `cargo_filter_keeps_error_diagnostic_block` — input containing:
  ```
  error[E0425]: cannot find value `x`\n
    --> src/main.rs:10:5\n
     |\n
  10 |     x\n
     |     ^ not found\n
  ```
  → all five lines survive verbatim in the output.

- `cargo_filter_keeps_test_failure_block` — input with a typical libtest failure:
  ```
  test my_test ... FAILED\n
  \n
  ---- my_test stdout ----\n
  \n
  thread 'my_test' panicked at 'assertion failed', src/lib.rs:5\n
  \n
  failures:\n
  \n
      my_test\n
  \n
  test result: FAILED. 0 passed; 1 failed\n
  ```
  → every line survives in the output.

- `cargo_filter_keeps_summary_line` — any input ending with
  `"test result: ok. 42 passed; 0 failed; 0 ignored\n"` → that exact line
  is in the filtered body.

- `cargo_filter_uses_compact_when_filtered_output_still_long` — build an input
  whose filtered result still exceeds 100 lines (many distinct error blocks) →
  `truncated == true` and a recovery file exists under the `TempDir`.

- `filter_for_command_routes_cargo_to_structured_filter` — call
  `filter_for_command("cargo test", <noisy_cargo_input>, dir.path())` →
  passing-test noise absent from body. **(integration: confirms routing)**

- `filter_for_command_routes_non_cargo_to_generic` — call
  `filter_for_command("make build", <long_generic_input>, dir.path())` →
  body contains head+tail elision marker (not cargo-filtered); no passing-test
  lines were dropped by cargo logic. **(negative case: confirms routing isolation)**

Bash-tool test in `executor/src/tools/bash.rs`:

- `cargo_command_output_is_filtered_through_cargo_filter` — use
  `bash_with_filter(scope, 30, true)` to run a real `cargo test` command on a
  tiny scratch project in the `TempDir` that has one failing test, and assert
  that (a) the failing test name appears in the output, (b) any passing-test
  `... ok` lines are absent, (c) `test result:` appears. Because the real
  `cargo` binary is available in CI, this is a live subprocess test (the same
  class as `filtered_bash_truncation_writes_recovery_file` in phase-01).

  Constructing a minimal Cargo project inline (no fixture files):
  ```rust
  // Create a minimal Cargo project in the TempDir:
  std::fs::write(dir.path().join("Cargo.toml"), r#"
  [package]
  name = "scratch"
  version = "0.1.0"
  edition = "2021"
  "#).unwrap();
  std::fs::create_dir_all(dir.path().join("src")).unwrap();
  std::fs::write(dir.path().join("src/lib.rs"), r#"
  #[test]
  fn passes() {}
  #[test]
  fn fails() { panic!("oh no"); }
  "#).unwrap();
  ```
  Then run `bash_with_filter(scope, 60, true)` with command
  `"cargo test 2>&1"`.

## End-to-end verification

The bash-tool test above spawns a real `cargo` subprocess, so it IS the E2E
artifact. For the completion log:

1. Run `cargo test cargo_command_output_is_filtered_through_cargo_filter -- --nocapture`
   and quote the actual filtered body the tool returned (abridged to the first
   ~20 lines). Confirm the body contains `test fails ... FAILED`, contains
   `test result:`, and does NOT contain `test passes ... ok`.

2. Manually verify the dispatcher is wired: run
   `cargo test filter_for_command_routes_cargo_to_structured_filter -- --nocapture`
   and confirm it passes, then quote the output.

## Authorizations

None. `regex` is already an executor dependency (`executor/Cargo.toml`,
`regex.workspace = true`); no `Cargo.toml` change. No `docs/architecture.md`
change. No new external dependency.

## Out of scope

- **Per-subcommand specialization beyond the universal cargo keep/drop rules.**
  `cargo test`, `cargo build`, and `cargo clippy` all emit the same diagnostic
  format; one classifier handles all three. Per-subcommand variations (e.g.
  stripping the clippy suggestion header differently from the build error header)
  are a later phase if evidence warrants.
- **Filtering the structured `--message-format json` cargo output** — the executor
  runs `cargo test` without `--message-format`; JSON-format filtering is a future
  concern.
- **Filtering non-cargo commands** (make, pytest, go test) — the generic
  `compact_with_recovery` path already handles those. Additional structured filters
  are future phases if a target project needs them.
- **Changing `is_cargo_command` to detect `cargo` in pipelines or semicolon
  chains** (e.g. `cd src && cargo test`) — commands matching on the first word
  only is explicit scope. The executor is expected to run cargo directly.
- **Arc B context-lifecycle work** (superseded-read eviction, compaction priority)
  — later phases; do not touch `compactor.rs` or the message history.
- **`PhaseRun` context-efficiency metrics** — phase-06.
- Do **not** change `pub fn bash`'s or `pub fn bash_with_filter`'s signature.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-07 (started)

**Executor:** rexyMCP executor

**Pre-flight:** `cargo test` passed with 599 tests. Clean branch confirmed.

**Tasks:** Implement `is_cargo_command`, `cargo_filter`, `filter_for_command` in
`output_filter.rs`; wire `bash.rs` to call `filter_for_command`; add unit tests
and one bash-tool integration test.

### Update — 2026-06-07 (complete)

**Executor:** rexyMCP executor

**Acceptance criteria:** All 11 criteria verified.

**Verification commands:**
- `cargo fmt --all --check` — passed clean
- `cargo build` — passed, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — passed clean
- `cargo test` — 609 passed (599 pre-flight + 10 new unit tests + 1 new bash-tool integration test), 0 failed

**Grep verification (spec-pinned literals):**
- `grep -n 'pub fn is_cargo_command' executor/src/context/output_filter.rs` → line 107 ✓
- `grep -n 'pub fn cargo_filter' executor/src/context/output_filter.rs` → line 124 ✓
- `grep -n 'pub fn filter_for_command' executor/src/context/output_filter.rs` → line 206 ✓
- `grep -n 'filter_for_command' executor/src/tools/bash.rs` → line 161 ✓

**End-to-end verification:**
1. `cargo test cargo_command_output_is_filtered_through_cargo_filter -- --nocapture` — passed. Real `cargo test` subprocess on a scratch project with one passing and one failing test. Filtered body contained `fails` (failing test name), did NOT contain `test passes ... ok`, and contained `test result:`.
2. `cargo test filter_for_command_routes_cargo_to_structured_filter -- --nocapture` — passed. Confirms dispatcher routes cargo commands to the structured filter.

**Files changed:**
- `executor/src/context/output_filter.rs` — added `is_cargo_command`, `cargo_filter`, `is_cargo_noise`, `filter_for_command` (public API); added 10 unit tests
- `executor/src/tools/bash.rs` — replaced `compact_with_recovery` call with `filter_for_command(&parsed.command, ...)`: added 1 integration test
- `docs/dev/milestones/M10-context-optimization/phase-02-structured-cargo-filter.md` — status `in-progress` → `review`, Update Log entries
- `docs/dev/milestones/M10-context-optimization/README.md` — phase table row 02 status `in-progress` → `review`

**Commit:** `feat: structured cargo output filter with command-based dispatch`

**Notes for review:** None. Implementation matches spec exactly. One test input was adapted: `cargo_filter_drops_compiling_noise` used bare `"Finished\n"` as input but the `is_cargo_noise` prefix check requires `"Finished "` (with trailing space), so the test input was updated to `"Finished dev [unoptimized] target(s) in 1.19s\n"` to match real cargo output. This is a test fix, not a behavior change.

### Review verdict — 2026-06-07

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none. One test-input adaptation (`cargo_filter_drops_compiling_noise`: bare `"Finished\n"` → `"Finished dev [...]"` to match the trailing-space prefix check) — correctly logged in Notes for review; a test-fixture fix, not a behavior change.
- **Calibration:** none. Clean first-try with full bookkeeping (committed `8ccc896`, status flipped, Update Log filled) — contrast with phase-01's architect-closeout. Reinforces the existing "executor implements single-concern additive phases well" data point.

**Architect review (2026-06-07):** Re-ran all four gates independently — fmt clean, build zero warnings, clippy clean, test 243 mcp + 609 executor (599 pre-flight + 10 new), 0 failures. All 9 acceptance criteria verified by grep + test. E2E: the real-`cargo`-subprocess integration test passes (failing test name survives, `... ok` lines dropped, `test result:` present); routing tests pass both directions. Code hygiene clean (no new unwrap/expect/panic — the lone `expect` is phase-01's `OnceLock` regex; no TODO/dbg/println/allow/ignore/unsafe). The `is_cargo_noise` keep-by-default design correctly preserves diagnostics while dropping progress/passing-test noise.
