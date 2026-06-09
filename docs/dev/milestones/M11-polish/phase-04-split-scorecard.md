# Phase 04: Split `scorecard.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** todo
**Depends on:** phase-03 (ordering only — no code dependency)
**Estimated diff:** ~0 net lines (pure move — no logic changes)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`mcp/src/scorecard.rs` is **1 153 lines** — ~391 lines of production code
(`ScorecardRow`, `SettingsScorecardRow`, `ScorecardFilter`, `Accumulator`,
`SettingsAccumulator`, `gates_all_pass`, `aggregate`, `aggregate_by_settings`,
`MAX_ROWS`) followed by a single ~759-line `#[cfg(test)] mod tests { … }` block.
Move the test block into a new sibling file `mcp/src/scorecard_tests.rs`, leaving
`scorecard.rs` as production-code-only (≤ 400 lines).

**Zero logic changes.** Every test moves byte-for-byte; the only production-source
change is replacing the inline `mod tests { … }` with a file-module declaration.
All gates pass identically before and after; the mcp test count is unchanged.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M11 §"File decomposition" names this phase and
  the ≤ 400-line target; pure move refactor, no logic changes.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp 2>&1 | tail -3` and record the passing test count
   (expected: **270**). The same count must pass after the move.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Boundaries verified on HEAD:

```
391:                 ← last production line (blank; line 390 is the closing `}` of aggregate_by_settings)
392:#[cfg(test)]
393:mod tests {
394:    use super::*;
395:    use rexymcp_executor::ai::types::TokenBreakdown;
396:    use rexymcp_executor::store::telemetry::{ContextEfficiency, GenerationParams};
        … ~756 lines, 34 test fns …
1152:    }              ← closes the final #[test] fn
1153:}                  ← closes `mod tests {`
```

So the **inner body** of the test module is **lines 394–1152**, and production
code is **lines 1–391**.

> **These line numbers were grep-verified against HEAD while drafting.** Re-confirm
> as the first step; if a rebase shifted them, adapt the `sed` ranges to the
> confirmed numbers.

`scorecard` is declared `mod scorecard;` in `mcp/src/main.rs:13` — a **single-file
module**, which determines the declaration form in Step 3 below.

## Spec

### The method: move with `sed`, do NOT retype the body

This is the same move just done for `executor/src/agent/mod.rs` →
`executor/src/agent/tests.rs` (landed clean first-try via `sed`). **Do not
reproduce the ~759-line test body with `write_file` or `patch`** — a verbatim
regeneration risks truncation/transcription errors and the repeated-patch churn
that has stalled split refactors before. Let the shell move the bytes losslessly.
The `bash` tool permits `sed`/`mv`/`printf` and in-scope redirects (the classifier
blocks only device writes / `mkfs` / `git push` / `rm -rf /` etc.).

### Step 1 — confirm the boundaries

```bash
wc -l mcp/src/scorecard.rs
grep -n '^#\[cfg(test)\]$' mcp/src/scorecard.rs   # expect: 392
sed -n '392,394p' mcp/src/scorecard.rs            # expect: #[cfg(test)] / mod tests { / use super::*;
tail -n 2 mcp/src/scorecard.rs                    # expect: a closing `}` (line 1153)
```

Use the confirmed numbers: `BODY_START` = the `use super::*;` line (expected 394),
`PROD_END` = the line before `#[cfg(test)]` (expected 391), `BODY_END` = total
lines − 1 (the line before the final `}`, expected 1152).

### Step 2 — extract the test body to `scorecard_tests.rs`

The new file is the module *body* — the inner content **without** the outer
`mod tests { … }` wrapper and **without** the `#[cfg(test)]` attribute (those are
supplied by the declaration in Step 3). So it starts at `use super::*;` and ends
at the last test fn's closing `}`:

```bash
sed -n '394,1152p' mcp/src/scorecard.rs > mcp/src/scorecard_tests.rs
```

### Step 3 — trim `scorecard.rs` and add the file-module declaration

Cannot redirect into `scorecard.rs` while reading it; write to a temp then move:

```bash
{ sed -n '1,391p' mcp/src/scorecard.rs; \
  printf '#[cfg(test)]\n#[path = "scorecard_tests.rs"]\nmod tests;\n'; } \
  > mcp/src/scorecard.rs.new
mv mcp/src/scorecard.rs.new mcp/src/scorecard.rs
```

**The `#[path = "scorecard_tests.rs"]` attribute is REQUIRED and load-bearing.**
`scorecard` is a *single-file* module (`mcp/src/scorecard.rs`, not
`scorecard/mod.rs`). A bare `mod tests;` inside it makes the compiler look for
`mcp/src/scorecard/tests.rs` or `mcp/src/scorecard/tests/mod.rs` — **not** the
sibling `scorecard_tests.rs`, so the build would fail with "file not found for
module `tests`". The `#[path]` attribute points the `tests` module at the
sibling file explicitly. (Contrast: `executor/src/agent/tests.rs` needed no
`#[path]` because `agent` is a *directory* module — `agent/mod.rs` — so its
submodule files live in the same `agent/` directory. `scorecard` is not a
directory, so the attribute is mandatory here.)

### Step 4 — format the two touched files

Format only the touched files — **never** run the writing form `cargo fmt --all`:

```bash
rustfmt mcp/src/scorecard.rs mcp/src/scorecard_tests.rs
```

### Step 5 — verify

```bash
cargo build -p rexymcp 2>&1 | tail -5
cargo clippy -p rexymcp --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp 2>&1 | tail -3
cargo fmt --all --check
wc -l mcp/src/scorecard.rs
```

The test count must equal Pre-flight (270). `wc -l scorecard.rs` must be ≤ 400.

## Acceptance criteria

- [ ] `mcp/src/scorecard.rs` is ≤ 400 lines and contains **no** inline
  `#[cfg(test)] mod tests { … }` block — only the
  `#[cfg(test)] #[path = "scorecard_tests.rs"] mod tests;` declaration.
- [ ] `mcp/src/scorecard_tests.rs` exists and contains the full test body
  (begins with `use super::*;`, ends with the last test fn's closing brace).
- [ ] `cargo build -p rexymcp` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp` passes with the **same test count as Pre-flight
  (270)** — no test added, removed, renamed, or skipped.
- [ ] `cargo fmt --all --check` passes.
- [ ] `git diff --stat` shows exactly two source files: `scorecard.rs` (large
  deletion) and `scorecard_tests.rs` (large addition).

## Test plan

No new tests. This is a pure file-split move — the existing 34 test fns in the
block provide complete coverage and must pass unchanged. The "same test count
before and after" criterion is the regression guard: any transcription or
boundary error shows up as a changed count or a compile failure.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. Pure internal
> file-split refactor: no production behavior, CLI surface, or config changes;
> the only build-visible effect (test compilation) is covered by `cargo test`.

## Authorizations

None. (No new dependency; no `unsafe`; no edit to `Cargo.toml`, the architecture
doc, or any other phase doc. `scorecard_tests.rs` is a new source file the spec
explicitly requires, so it is not an unauthorized new file.)

## Out of scope

- Do **not** sub-split `scorecard_tests.rs` — one flat file is the goal.
- Do **not** edit, rename, reorder, add, or delete any test, helper fn, `use`
  statement, or `#[allow]`. Move the body byte-for-byte.
- Do **not** touch `scorecard_cli.rs`, `main.rs`, or any other file — the tests
  reference production items via `use super::*;` / `crate::…`, which still
  resolves unchanged from `scorecard_tests.rs`.
- Do **not** change any production code in `scorecard.rs` lines 1–391.

## Notes for executor

- **Why `sed`, not `write_file`:** the test body is ~759 lines. Regenerating it
  through `write_file` risks truncation/corruption and the repeated-patch churn
  that stalls large moves. `sed` moves the bytes losslessly in one command — no
  regeneration, no transcription risk. This is the prescribed method.
- **The `#[path]` attribute is the one thing that will break the build if
  omitted** — see Step 3. It is required because `scorecard` is a single-file
  module, unlike the `agent` directory module split earlier.
- `scorecard.rs` is 1 153 lines (within range-read range in 2 reads if you need
  to inspect it), but you do **not** need to read the test body to move it — `sed`
  operates on it without loading it into the conversation.
- If `cargo test` reports a different count or a compile error after the move, the
  cause is a wrong boundary line in Step 2/3 or a missing `#[path]` — re-run the
  Step 1 checks. Do not "fix" it by editing test contents.
- Commit as a single `refactor:` commit; the body explains *why* (the file's test
  suite dwarfs its production code), not *what*.

## Update Log

<!-- entries appended below this line -->
