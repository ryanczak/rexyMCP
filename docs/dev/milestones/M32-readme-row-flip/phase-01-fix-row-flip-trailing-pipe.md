# Phase 01: Fix the doubled trailing pipe in `flip_readme_row`

**Milestone:** M32 — README Row-Flip Fix
**Status:** todo
**Depends on:** none
**Estimated diff:** ~40 lines (one production character + test hardening)
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`flip_readme_row` (`mcp/src/finalize.rs:178`) emits a doubled trailing pipe on
every flip — `| 01 | … | review ||` — because the replacement's suffix slice
includes the final `|` it just re-emitted. Fix the one-character off-by-one
and harden the tests from substring checks (which the malformed output also
passes) to exact-equality assertions with pinned negatives.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #27 — the server-authored bookkeeping this
  function belongs to.
- The milestone [README](README.md) — the four production misfires and the
  partial-fix history.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`mcp/src/finalize.rs:191-195`, inside `flip_readme_row`:

```rust
// Replace the cell content between the last two `|` delimiters
format!(
    "{} review |{}",
    &line[..second_last_pipe + 1],
    &line[last_pipe..]
)
```

The prefix `&line[..second_last_pipe + 1]` is correct (everything up to and
including the delimiter before the status cell). The bug is the suffix:
`&line[last_pipe..]` starts **at** the final `|`, so for a well-formed row
ending in `|` the output is `…| review |` + `|` = `…| review ||`.

Worked example — input row:

```
| 01 | rmcp 2.2 bump ([phase-01.md](phase-01.md)) | todo |
```

Current (buggy) output:

```
| 01 | rmcp 2.2 bump ([phase-01.md](phase-01.md)) | review ||
```

Required output:

```
| 01 | rmcp 2.2 bump ([phase-01.md](phase-01.md)) | review |
```

**Why the five existing tests miss it** (`mcp/src/finalize.rs:368-420`):
every positive assertion is a substring check like
`updated.contains("| review |")` — and `"| review |"` is a substring of the
malformed `"| review ||"` as well, so the tests pass on the buggy output.

## Spec

1. **Fix the suffix slice** — in `mcp/src/finalize.rs`, change the
   `format!` suffix argument from `&line[last_pipe..]` to
   `&line[last_pipe + 1..]`:

   ```rust
   // AFTER:
   format!(
       "{} review |{}",
       &line[..second_last_pipe + 1],
       &line[last_pipe + 1..]
   )
   ```

   (For a row that ends exactly at the final `|`, `&line[last_pipe + 1..]` is
   the empty string; for a row with trailing whitespace/text after the final
   `|`, it preserves that tail. `last_pipe` is a byte index of the ASCII `|`
   found by `rfind('|')`, so `+ 1` cannot split a UTF-8 character.)

2. **Harden the existing tests to exact equality.** In the
   `#[cfg(test)]` module (`mcp/src/finalize.rs:368+`), strengthen the flip
   assertions so the malformed shapes cannot pass:

   - In `flip_readme_row_flips_matching_row_only`: replace the
     `lines[0].contains("| review |")` assertion with an **exact full-row
     equality** against
     `"| 03a | Server-authored finalize ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | review |"`,
     and keep the sibling-row assertion as-is. Add the two pinned negatives:
     `assert!(!lines[0].contains("||"));` and
     `assert!(!lines[0].contains("in-progress"));`.
   - In `flip_readme_row_flips_todo_cell`: assert the flipped line is
     **exactly** `"| 01 | Phase ([phase-01.md](phase-01.md)) | review |"`
     and add `assert!(!new.contains("||"));`.
   - In `flip_readme_row_flips_bounced_row` (line 412): add the
     `assert!(!<flipped>.contains("||"));` negative alongside its existing
     assertions (adapt to that test's local variable names; do not weaken
     anything it already asserts).

3. **Add one regression test** in the same module:

   ```rust
   #[test]
   fn flip_readme_row_emits_single_trailing_pipe() {
       let readme = "| 02 | Structured output ([phase-02-structured-tool-output.md](phase-02-structured-tool-output.md)) | in-progress |\n";
       let result = flip_readme_row(readme, "phase-02-structured-tool-output.md");
       let updated = result.expect("row should flip");
       assert_eq!(
           updated.lines().next().unwrap(),
           "| 02 | Structured output ([phase-02-structured-tool-output.md](phase-02-structured-tool-output.md)) | review |"
       );
       assert!(!updated.contains("||"), "no doubled pipe anywhere: {updated}");
   }
   ```

4. **Run the four gates** as separate invocations (`cargo fmt --all --check`,
   `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
   `cargo test`) and fill the Update Log.

## Acceptance criteria

- [ ] `mcp/src/finalize.rs`'s flip `format!` suffix reads
      `&line[last_pipe + 1..]` (grep: `grep -n 'last_pipe + 1' mcp/src/finalize.rs`
      → one match inside `flip_readme_row`).
- [ ] `flip_readme_row_emits_single_trailing_pipe` exists and passes.
- [ ] The strengthened tests assert exact row equality and the `!contains("||")` /
      `!contains("in-progress")` negatives per Spec 2.
- [ ] Reverting the one-character production fix makes
      `flip_readme_row_emits_single_trailing_pipe` (and the exact-equality
      assertions) fail — i.e. the tests are no longer blind to the malformed
      shape. (Verify mentally from the assertions; do not commit a reverted
      state.)
- [ ] No other production behavior changed: the `None` cases
      (`already_review`, `row_absent`) and the bounce-note dropping are
      untouched, and their existing tests pass unmodified.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes.

## Test plan

Covered by Spec 2–3: one new regression test
(`flip_readme_row_emits_single_trailing_pipe`, exact-equality + the `||`
negative) and exact-equality/negative hardening of the three existing flip
tests. No new production surface beyond the one-character fix, so no further
tests.

## End-to-end verification

The shipped artifact is the finalize path of the `rexymcp` binary, exercised
at the next real phase completion. The executor's obligation here:

1. `cargo test -p rexymcp flip_readme_row 2>&1 | tail -5` — quote the summary
   line (expect 6 passing `flip_readme_row` tests) in the completion Update
   Log.

(A live finalize round-trip requires a real `execute_phase` completion — that
is the architect's next-milestone-dispatch observation, not this phase's
work.)

## Authorizations

None. (One production expression + tests in `mcp/src/finalize.rs` only; no
dependency, no doc, no config change.)

## Out of scope

- Rewriting `flip_readme_row`'s parsing approach (the pipe-index scan stays).
- Touching `finalize_complete`, `flip_status_to_review`, or any other
  finalize helper.
- Editing `docs/dev/milestones/M27-autonomous-escalation-loop/bugs/bug-03a-1.md`
  (the architect closes it at milestone close).
- Repairing historical malformed rows in old milestone READMEs (all already
  hand-repaired).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
