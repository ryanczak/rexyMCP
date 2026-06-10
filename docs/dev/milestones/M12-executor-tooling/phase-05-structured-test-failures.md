# Phase 05: Structured `cargo test` failure digest

**Milestone:** M12 — Executor Tooling
**Status:** todo
**Depends on:** none (additive enrichment of the existing M10 cargo output filter)
**Estimated diff:** ~180 lines (≈80 production + ≈100 tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

When `cargo test` fails, libtest prints each failure as a multi-line block: a
`---- <name> stdout ----` header, a `thread '<name>' … panicked at <loc>:` line,
and the assertion detail (`left`/`right` values, a custom message, or a panic
string). Today that whole block reaches the model as raw text — the M10 cargo
filter (`output_filter::cargo_filter`) *preserves* it (good) but does nothing to
**distill** it. The weak executor then has to re-read and re-parse a verbose,
interleaved failure section every retry turn.

This phase adds a compact **failure digest** prepended to the filtered cargo
output: one high-signal line per failed test naming the test, its source
location, and the assertion's expected-vs-actual values. The model sees the
structured summary *first*, before the verbose blocks. Example — given libtest's

```
---- tests::it_adds stdout ----

thread 'tests::it_adds' (3787800) panicked at src/lib.rs:6:20:
assertion `left == right` failed
  left: 4
 right: 5
```

the model gets a leading line like

```
=== Test failures (1) ===
test tests::it_adds failed at src/lib.rs:6:20 — assertion `left == right` failed (left: 4, right: 5)
```

This is the same cheap, additive, message-enrichment shape as phase-04
(suggested-fixes): **one module, one new pure parser + one prepend hook in
`cargo_filter`, no new struct field on any existing type, no new `SessionEvent`,
no consumer to wire** — the digest flows to the model through the bash tool
output it already reads. When `cargo test` *passes* (or the command isn't a test
run), the parser finds nothing and the output is **byte-identical to today**.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M12 Arc B, third bullet: "**Structured
  test-failure parsing** — turn `cargo test` failures into expected-vs-actual
  records for the verifier-retry loop (extends the M10 cargo filter, which
  already preserves test summaries)."
- `docs/dev/STANDARDS.md` §2.2 — "No premature abstraction." The one
  module-private `TestFailure` struct this phase adds is justified because its
  consumer (the digest formatter) ships **in the same phase and same module**;
  it is **not** serialized, **not** exported, and adds no derive to any shared
  type (per WORKFLOW § "Derive intentionally" / "wired-in state").
- `docs/dev/WORKFLOW.md` § "Prefer additive change shapes" — the reason this is a
  prepend to `cargo_filter`'s return rather than a new field on `ToolResult` /
  `SessionEvent`: an additive prefix changes one function and breaks zero call
  sites or struct literals. § "Verify external APIs against live docs" — libtest's
  panic format drifts between Rust versions (the `(3787800)` thread-id shown above
  is recent and absent from older releases); the Pre-flight pins verifying it
  live.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Verify the current libtest failure format before coding.** The fixtures in
   § "Reference: real cargo-test output" below were captured on 2026-06-09 from a
   real `cargo test --color=never` run and are accurate for this toolchain, but
   the format drifts across Rust versions (notably the `(<threadid>)` token
   between the quoted test name and `panicked at`, which older Rust omits).
   Sources, in priority order: a real `cargo test` run you trigger yourself on a
   deliberately-failing test; the Rust release notes. **Trust the live compiler
   output over this sketch.** Pin the *behavior* (one digest line per failed test,
   carrying name + location + detail); let the exact extraction adapt to the real
   shape. Flag any divergence in "Notes for review" — do not silently work around
   a mismatch you can't reconcile from this doc; file a blocker.

## Toolchain dependencies

None new. This phase parses text already captured by the existing `bash`-tool
shell-out to the project's `test` command (`cargo test`). It adds **no** new
shell-out and **no** new runtime binary — `cargo` is already a Tier-0
requirement. No new crate dependency (`regex` is already in
`executor/Cargo.toml` and `output_filter.rs` already uses it).

## Current state

Everything in this phase lives in **one file**:
`executor/src/context/output_filter.rs`. Two functions are relevant.

### The function you extend

`output_filter.rs:124-167` — `cargo_filter`. It normalizes the raw output, drops
cargo noise (passing-test lines, `Compiling`/`Finished`/`Running`, …) while
keeping diagnostic and test-failure blocks, then truncates with a recovery file
if still over the line cap. It returns `(body, truncated)`:

```rust
pub fn cargo_filter(raw: &str, project_root: &Path) -> (String, bool) {
    let normalized = normalize(raw);
    let mut kept = String::new();
    // ... drop is_cargo_noise lines, collapse blank runs into `kept` ...
    let line_count = kept.lines().count();
    if line_count <= LINE_CAP {
        return (kept, false);
    }
    compact_with_recovery(&kept, project_root)   // still over cap → head+tail + recovery file
}
```

`cargo_filter` is reached only for cargo commands, via `filter_for_command`
(`output_filter.rs:206`), which the `bash` tool calls. Non-cargo commands
(`make test`, `pytest`, …) go to the generic `compact_with_recovery` and are
**out of scope** for this phase.

The libtest failure blocks (`---- … stdout ----`, `thread … panicked at …`,
`left:`/`right:`) are **not** matched by `is_cargo_noise`, so they already
survive `cargo_filter` into `kept`. This phase reads them out of the normalized
text and prepends a distilled digest.

### Why a prepend, not a new record type

The architecture frames this as "expected-vs-actual records for the
verifier-retry loop." The **record's consumer is the model reading the bash tool
output** — which exists today. There is no separate structured-record consumer
(no scorecard field, no `SessionEvent`, no dashboard panel queries test
failures), so per WORKFLOW § "Derive intentionally" we do **not** invent one:
wiring state whose consumer doesn't exist is dead structure (the M10 08a/08d
struct-literal-churn lesson). The digest is plain text prepended to the output
the model already reads, exactly like phase-04 enriched the diagnostic `message`
string. `TestFailure` stays a module-private parse intermediate.

## Spec

All changes are in `executor/src/context/output_filter.rs`.

1. **Add a module-private `TestFailure` parse intermediate.** A small struct
   holding what one digest line needs. Keep it module-private (no `pub`), and add
   **only** the derives the tests need (`Debug`, `PartialEq`) — it is never
   serialized and crosses no protocol boundary, so do **not** add `Serialize` /
   `Deserialize`.

   ```rust
   #[derive(Debug, PartialEq)]
   struct TestFailure {
       /// Test path from the `---- <name> stdout ----` header, e.g. `tests::it_adds`.
       name: String,
       /// Source location from `panicked at <loc>:`, e.g. `src/lib.rs:6:20`. `None`
       /// if the block had no parseable `panicked at` line.
       location: Option<String>,
       /// The distilled detail: the `assertion … failed` line plus single-line
       /// `left:`/`right:` values when present, or the custom message / panic
       /// string otherwise. May be empty if the block had only a header.
       detail: String,
   }
   ```

2. **Add a pure parser** `fn parse_test_failures(normalized: &str) -> Vec<TestFailure>`.
   Walk the lines of the normalized output. For each `---- <name> stdout ----`
   header, open a block; within it capture:
   - the **location** from the first line containing `panicked at ` — take the
     substring after `panicked at ` and strip a single trailing `:` (the line is
     `thread '<name>' (<id>) panicked at src/lib.rs:6:20:` — note the optional
     `(<id>)` token, which you ignore);
   - the **detail** from the lines *after* the `panicked at` line until the block
     ends. The block ends at the next `---- ` header, at a line equal to
     `failures:` (the trailing name-list section), or at the `test result:`
     summary line. **Exclude** `note:`-prefixed lines (e.g. `note: run with
     RUST_BACKTRACE=1 …`) and blank / collapsed-blank (` (xN)`) lines from the
     detail. Join the kept detail lines with `", "` (or a single space — your
     call; pin only that the `left`/`right` values appear).

   Return one `TestFailure` per block. **A block with no `panicked at` line still
   yields a `TestFailure`** (location `None`) as long as it had a `----` header —
   but in practice every failure block has one. **Passing output contains no
   `---- … stdout ----` headers, so the result is empty** — this is the pinned
   negative.

   Notes that are load-bearing:
   - Key off the `---- <name> stdout ----` header for the test name, **not** the
     `thread '…'` line — the header name is clean (`tests::it_adds`), and a panic
     can occur on a spawned thread whose name differs.
   - Do **not** fabricate expected/actual when libtest didn't print `left`/`right`
     (a bare `assert!` with a message, or a `panic!`): surface the message text
     verbatim as the detail. Inventing "expected/got" for a non-equality assertion
     would mislead the model.
   - The `left`/`right` labels are surfaced **verbatim** (`left: 4, right: 5`),
     **not** relabeled to "expected/actual": in `assert_eq!(a, b)` the mapping of
     left/right to expected/actual depends on argument order, which we cannot
     know. Preserving libtest's own labels is the honest rendering.
   - Multi-line `left:`/`right:` values (debug-formatted structs spanning several
     lines) are an edge case: capture the single-line form when libtest prints it
     on one line; if the value spans lines, it is acceptable to include only the
     `assertion … failed` line. Do not over-engineer multi-line reconstruction.

3. **Add a pure formatter** `fn format_failure_digest(failures: &[TestFailure]) -> String`.
   - **Returns an empty `String` when `failures` is empty** — this makes the
     prepend in step 4 a no-op and guarantees byte-identical output on the
     no-failure path. This is a pinned property.
   - Otherwise returns a block: a header line containing the count (e.g.
     `=== Test failures (3) ===`) followed by one line per failure of the shape
     `test <name> failed at <location> — <detail>` (omit ` at <location>` when
     `location` is `None`), then a trailing blank line to separate the digest from
     the verbose body.

4. **Prepend the digest in `cargo_filter`.** Compute the digest once from the
   normalized text, then prefix it onto whatever body the existing logic returns,
   preserving the existing `truncated` flag and the existing truncation/recovery
   behavior unchanged:

   ```rust
   pub fn cargo_filter(raw: &str, project_root: &Path) -> (String, bool) {
       let normalized = normalize(raw);
       let digest = format_failure_digest(&parse_test_failures(&normalized));
       // ... existing filtering + cap/compact logic, producing (body, truncated) ...
       (format!("{digest}{body}"), truncated)
   }
   ```

   The structural placement (refactor the two early returns into a single tail
   return, or prepend at both return sites) is yours — but the digest must be
   prepended on **every** return path, and the existing `truncated` boolean and
   recovery-file behavior must be **unchanged**. Because the digest lives at the
   head and `compact_with_recovery` always preserves the head, the digest is never
   truncated away.

5. **Do not touch anything else.** Not `normalize`, not `compact_with_recovery`,
   not `is_cargo_noise`, not `is_cargo_command`, not `filter_for_command`, not the
   `bash` tool, not the verifier, not any `SessionEvent` / dashboard / `log_query`
   code. No new public export from the module. No `is_cargo_test` helper is needed
   — the digest is gated purely on whether `parse_test_failures` found any blocks,
   which only appear in test output.

## Reference: real cargo-test output

These are **real, verbatim** `cargo test --color=never` captures (2026-06-09).
Use them as test fixtures (Rust raw strings `r#"…"#`). The failing fixture has
three failures exercising the three detail shapes: `assert_eq!` (left/right), a
bool `assert!` with a custom message, and a bare `panic!`.

**Fixture FAIL — three failed tests, one passing:**

```
running 4 tests
test tests::it_adds ... FAILED
test tests::it_bools ... FAILED
test tests::it_panics ... FAILED
test tests::it_passes ... ok

failures:

---- tests::it_adds stdout ----

thread 'tests::it_adds' (3787800) panicked at src/lib.rs:6:20:
assertion `left == right` failed
  left: 4
 right: 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- tests::it_bools stdout ----

thread 'tests::it_bools' (3787801) panicked at src/lib.rs:8:21:
sum too small

---- tests::it_panics stdout ----

thread 'tests::it_panics' (3787802) panicked at src/lib.rs:10:22:
boom 7


failures:
    tests::it_adds
    tests::it_bools
    tests::it_panics

test result: FAILED. 1 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

**Fixture PASS — all tests pass (the pinned negative):**

```
running 2 tests
test tests::a ... ok
test tests::b ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

For Fixture FAIL the parsed digest must name all three tests with their
locations, surface `left: 4` and `right: 5` for `it_adds`, `sum too small` for
`it_bools`, and `boom 7` for `it_panics`. For Fixture PASS the digest is empty
and `cargo_filter`'s output is byte-identical to today (no `=== Test failures`
header).

## Acceptance criteria

- [ ] `parse_test_failures(FIXTURE_FAIL)` returns 3 `TestFailure`s, named
      `tests::it_adds` / `tests::it_bools` / `tests::it_panics`, each with a
      `location` of the form `src/lib.rs:N:C`, and details carrying `left: 4` +
      `right: 5`, `sum too small`, and `boom 7` respectively.
- [ ] `parse_test_failures(FIXTURE_PASS)` returns an empty `Vec` (no
      `---- … stdout ----` headers present).
- [ ] `format_failure_digest(&[])` returns `""` (empty string).
- [ ] `cargo_filter(FIXTURE_FAIL, dir)` returns a body that **starts with** a
      `=== Test failures (3) ===` header line and contains, before the verbose
      blocks, one digest line per failed test with name + location + detail; the
      original verbose failure blocks still survive below the digest.
- [ ] `cargo_filter(FIXTURE_PASS, dir)` returns a body **byte-identical** to the
      pre-phase behavior (no digest header) — assert it does **not** contain
      `=== Test failures`. This is the pinned negative; make it an exact
      must-not-contain so a "always prepend" impl fails it.
- [ ] All existing `output_filter` tests (`cargo_filter_*`, `normalize_*`,
      `compact_*`, `filter_for_command_*`) pass **unchanged** — the digest is
      additive and absent when there are no failures.
- [ ] `cargo build` (zero new warnings), `cargo clippy --all-targets
      --all-features -- -D warnings`, `cargo fmt --all --check`, `cargo test` all
      pass.

## Test plan

New `#[test]`s in the existing `#[cfg(test)] mod tests` block at the bottom of
`executor/src/context/output_filter.rs`, mirroring the existing `cargo_filter_*`
style (raw-string fixture + `tempfile::tempdir()`). Names are pinned;
placement/count details are yours.

- `parse_test_failures_extracts_all_failed_tests` — Fixture FAIL → 3 failures
  with the names/locations/details above (assert on the `left: 4`/`right: 5`,
  `sum too small`, `boom 7` content).
- `parse_test_failures_empty_on_passing_output` — Fixture PASS → empty `Vec`.
  Pinned negative.
- `parse_test_failures_preserves_left_right_labels` — assert the `it_adds`
  detail contains both `left` and `right` with values `4` and `5`, and does
  **not** invent the words `expected`/`actual` (a `!detail.contains("expected")`
  assertion guards against relabeling).
- `format_failure_digest_empty_for_no_failures` — `format_failure_digest(&[])`
  `== ""`.
- `cargo_filter_prepends_failure_digest` — Fixture FAIL → returned body
  `starts_with("=== Test failures (3) ===")` (or `.contains` the header if you
  add a leading newline — pin the header presence and the per-test lines) **and**
  still contains `panicked at` (verbose block preserved below the digest).
- `cargo_filter_no_digest_on_passing_output` — Fixture PASS → body does **not**
  contain `=== Test failures`, and the passing-test summary line
  (`test result: ok.`) is handled exactly as today. Pinned negative.
- *(Optional, if cheap)* `parse_test_failures_handles_bare_panic_without_left_right`
  — a single-block fixture whose only detail is a `panic!` message → one
  `TestFailure` with the message as detail and no fabricated `left`/`right`.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable CLI/binary artifact. The
> filter runs only inside a full `execute_phase` local-LLM loop (the `bash` tool
> calls `filter_for_command` → `cargo_filter`), which cannot be exercised
> hermetically. **However**, the unit tests are *de facto* end-to-end: they feed
> **real, verbatim `cargo test --color=never` output** (captured 2026-06-09, see
> § "Reference") to the real `cargo_filter`. There is no hand-written or mocked
> test output in the path. Quote the digest produced for Fixture FAIL and the
> byte-identical-for-PASS assertion in the completion Update Log.

## Authorizations

None. No new dependencies (`regex` and `serde` are already in
`executor/Cargo.toml`; this phase adds no shell-out and no new runtime binary).
No `docs/architecture.md` edit. No files outside
`executor/src/context/output_filter.rs`.

## Out of scope

- **No new `ToolResult` / `SessionEvent` field, no new public type, no new
  consumer.** The digest is plain text prepended to the existing filtered output.
  If you believe a structured record needs to be threaded somewhere, stop and
  file a blocker — do not wire a consumer that doesn't exist.
- **No non-cargo test runners.** `pytest`, `tsc`/`vitest`, `make test`, etc. go
  through the generic `compact_with_recovery` path and are untouched. This is a
  cargo/libtest-only feature this phase.
- **No relabeling of `left`/`right` to "expected"/"actual".** Surface libtest's
  own labels verbatim (see Spec step 2).
- **No change to `normalize`, `compact_with_recovery`, `is_cargo_noise`,
  `is_cargo_command`, `filter_for_command`, the `bash` tool, or the verifier.**
- **No multi-line debug-value reconstruction.** Single-line `left:`/`right:`
  capture only (Spec step 2).
- **Not the other Arc B phases / Arc A.** No task tracking (phase-06/07).
- **No new dependency, no `unsafe`, no `#[allow]`.** If you think you need any,
  stop and file a blocker.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
