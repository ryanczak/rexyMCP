# Phase 05b: Split `verifier.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** review
**Depends on:** phase-05a (ordering only — no code dependency; same refactor class)
**Estimated diff:** ~0 net lines (pure move — no logic changes)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`executor/src/governor/verifier.rs` is **1 163 lines** — ~494 lines of production
code (the `FileVerifier` trait, `RustVerifier`, and supporting types) followed by
a single ~666-line `#[cfg(test)] mod tests { … }` block. Move the test block into
a new sibling file `executor/src/governor/verifier_tests.rs`, leaving
`verifier.rs` as production-code-only.

**Zero logic changes.** Every test moves byte-for-byte; the only production-source
change is replacing the inline `mod tests { … }` with a file-module declaration.
All gates pass identically before and after; the executor test count is unchanged.
This is the **same refactor as phase-05a** (`server.rs` → `server_tests.rs`,
landed clean first-try via `sed`) — follow that recipe exactly. It is the final
file-decomposition phase of M11.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M11 §"File decomposition" names this phase
  (`verifier.rs` is one of the four largest source files); pure move refactor, no
  logic changes.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor 2>&1 | tail -3` and record the result line
   (expected: **665 passed; 0 failed; 2 ignored**). The same counts must hold
   after the move — note that the **2 ignored** are the `#[ignore]`-gated live
   `rustc` tests in this very block; they must stay ignored, not vanish.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Boundaries verified on HEAD:

```
494:                 ← last production line (blank; the closing brace of the last
                       production item is on an earlier line)
495:#[cfg(test)]
496:mod tests {
497:    use super::*;
498:    use std::fs;
        … ~664 lines, 24 fns (helpers + 35 #[test]/#[tokio::test], 2 of them
          #[ignore]-gated live rustc tests) …
1162:    }            ← closes the final test fn
1163:}                ← closes `mod tests {`
```

So the **inner body** of the test module is **lines 497–1162**, and production
code is **lines 1–494**.

> **These line numbers were grep-verified against HEAD while drafting.** Re-confirm
> as the first step; if a rebase shifted them, adapt the `sed` ranges to the
> confirmed numbers.

`verifier` is declared `pub mod verifier;` in `executor/src/governor/mod.rs:3` —
a **single-file module** (`executor/src/governor/verifier.rs`, not
`verifier/mod.rs`), which determines the declaration form in Step 3 below.

**The test block contains both `#[tokio::test]` async tests and 2
`#[ignore]`-gated live `rustc`-subprocess tests.** This changes **nothing** about
the move — `sed` moves the attributes and bodies verbatim, and `use super::*;`
resolves the same imports from the sibling file. Do **not** add, remove, or
rewrite any `#[tokio::test]`, `#[test]`, or `#[ignore]` attribute.

## Spec

### The method: move with `sed`, do NOT retype the body

This is the same move just done for `mcp/src/server.rs` →
`mcp/src/server_tests.rs` (phase-05a, landed clean first-try via `sed`). **Do
not reproduce the ~666-line test body with `write_file` or `patch`** — a verbatim
regeneration risks truncation/transcription errors and the repeated-patch churn
that has stalled split refactors before. Let the shell move the bytes losslessly.
The `bash` tool permits `sed`/`mv`/`printf` and in-scope redirects (the classifier
blocks only device writes / `mkfs` / `git push` / `rm -rf /` etc.).

### Step 1 — confirm the boundaries

```bash
wc -l executor/src/governor/verifier.rs
grep -n '^#\[cfg(test)\]$' executor/src/governor/verifier.rs   # expect: 495
sed -n '495,497p' executor/src/governor/verifier.rs            # expect: #[cfg(test)] / mod tests { / use super::*;
tail -n 2 executor/src/governor/verifier.rs                    # expect: a closing `}` (line 1163)
```

Use the confirmed numbers: `BODY_START` = the `use super::*;` line (expected 497),
`PROD_END` = the line before `#[cfg(test)]` (expected 494), `BODY_END` = total
lines − 1 (the line before the final `}`, expected 1162).

### Step 2 — extract the test body to `verifier_tests.rs`

The new file is the module *body* — the inner content **without** the outer
`mod tests { … }` wrapper and **without** the `#[cfg(test)]` attribute (those are
supplied by the declaration in Step 3). So it starts at `use super::*;` and ends
at the last test fn's closing `}`:

```bash
sed -n '497,1162p' executor/src/governor/verifier.rs > executor/src/governor/verifier_tests.rs
```

### Step 3 — trim `verifier.rs` and add the file-module declaration

Cannot redirect into `verifier.rs` while reading it; write to a temp then move:

```bash
{ sed -n '1,494p' executor/src/governor/verifier.rs; \
  printf '#[cfg(test)]\n#[path = "verifier_tests.rs"]\nmod tests;\n'; } \
  > executor/src/governor/verifier.rs.new
mv executor/src/governor/verifier.rs.new executor/src/governor/verifier.rs
```

**The `#[path = "verifier_tests.rs"]` attribute is REQUIRED and load-bearing.**
`verifier` is a *single-file* module (`executor/src/governor/verifier.rs`, not
`verifier/mod.rs`). A bare `mod tests;` inside it makes the compiler look for
`executor/src/governor/verifier/tests.rs` or
`executor/src/governor/verifier/tests/mod.rs` — **not** the sibling
`verifier_tests.rs`, so the build would fail with "file not found for module
`tests`". The `#[path]` attribute points the `tests` module at the sibling file
explicitly. (This is exactly the phase-05a situation: `server` was likewise a
single-file module and required `#[path = "server_tests.rs"]`. Contrast
`executor/src/agent/tests.rs`, which needed no `#[path]` because `agent` is a
*directory* module.)

### Step 4 — format the two touched files

Format only the touched files — **never** run the writing form `cargo fmt --all`:

```bash
rustfmt executor/src/governor/verifier.rs executor/src/governor/verifier_tests.rs
```

### Step 5 — verify

```bash
cargo build -p rexymcp-executor 2>&1 | tail -5
cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp-executor 2>&1 | tail -3
cargo fmt --all --check
wc -l executor/src/governor/verifier.rs
```

The test result must equal Pre-flight (665 passed; 2 ignored). `wc -l verifier.rs`
must be ≤ 500 (production lines 1–494 + the 3-line declaration).

## Acceptance criteria

- [ ] `executor/src/governor/verifier.rs` is ≤ 500 lines and contains **no**
  inline `#[cfg(test)] mod tests { … }` block — only the
  `#[cfg(test)] #[path = "verifier_tests.rs"] mod tests;` declaration.
- [ ] `executor/src/governor/verifier_tests.rs` exists and contains the full test
  body (begins with `use super::*;`, ends with the last test fn's closing brace).
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes with the **same counts as Pre-flight
  (665 passed; 2 ignored)** — no test added, removed, renamed, skipped, or
  un-ignored.
- [ ] `cargo fmt --all --check` passes.
- [ ] `git diff --stat` shows exactly two source files: `verifier.rs` (large
  deletion) and `verifier_tests.rs` (large addition).

## Test plan

No new tests. This is a pure file-split move — the existing test fns in the block
provide complete coverage and must pass unchanged. The "same counts before and
after" criterion is the regression guard: any transcription or boundary error
shows up as a changed count or a compile failure. The **2 ignored** count is part
of the guard — if it drops to 0, an `#[ignore]` attribute was lost in the move.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. Pure internal
> file-split refactor: no production behavior, verifier semantics, or config
> changes; the only build-visible effect (test compilation) is covered by
> `cargo test`.

## Authorizations

None. (No new dependency; no `unsafe`; no edit to `Cargo.toml`, the architecture
doc, or any other phase doc. `verifier_tests.rs` is a new source file the spec
explicitly requires, so it is not an unauthorized new file.)

## Out of scope

- Do **not** sub-split `verifier_tests.rs` — one flat file is the goal.
- Do **not** edit, rename, reorder, add, or delete any test, helper fn, `use`
  statement, `#[allow]`, `#[test]`, `#[tokio::test]`, or `#[ignore]`. Move the
  body byte-for-byte.
- Do **not** touch `governor/mod.rs`, `hard_fail.rs`, `scorer.rs`, or any other
  file — the tests reference production items via `use super::*;` / `crate::…`,
  which still resolves unchanged from `verifier_tests.rs`.
- Do **not** change any production code in `verifier.rs` lines 1–494.

## Notes for executor

- **Why `sed`, not `write_file`:** the test body is ~666 lines. Regenerating it
  through `write_file` risks truncation/corruption and the repeated-patch churn
  that stalls large moves. `sed` moves the bytes losslessly in one command — no
  regeneration, no transcription risk. This is the prescribed method.
- **The `#[path]` attribute is the one thing that will break the build if
  omitted** — see Step 3. It is required because `verifier` is a single-file
  module, exactly as in phase-05a's `server` split.
- The `#[tokio::test]` async tests and the 2 `#[ignore]`-gated live `rustc` tests
  move with the body verbatim — **no special handling**. Do not "convert", run, or
  un-ignore them. The pre-flight `2 ignored` count must still read `2 ignored`
  after the move.
- `verifier.rs` is 1 163 lines, but you do **not** need to read the test body to
  move it — `sed` operates on it without loading it into the conversation.
- If `cargo test` reports a different count or a compile error after the move, the
  cause is a wrong boundary line in Step 2/3 or a missing `#[path]` — re-run the
  Step 1 checks. Do not "fix" it by editing test contents.
- Commit as a single `refactor:` commit; the body explains *why* (the file's test
  suite dwarfs its production code), not *what*.

## Update Log

<!-- entries appended below this line -->

### Update — 2025-07-09 (started)

**Executor:** rexyMCP executor

Confirmed boundaries: `verifier.rs` is 1163 lines, `#[cfg(test)]` at line 495,
`use super::*;` at line 497, closing `}` at line 1163. Pre-flight test count:
**665 passed; 0 failed; 2 ignored**. Repo clean.

### Update — 2025-07-09 (complete)

**Tasks completed:** Moved ~666-line `#[cfg(test)] mod tests { … }` block from
`verifier.rs` to sibling `verifier_tests.rs` via `sed` extraction + 3-line
file-module declaration with `#[path = "verifier_tests.rs"]`.

**Verification commands:**
- `cargo build -p rexymcp-executor` — success, zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — success
- `cargo test -p rexymcp-executor` — **665 passed; 0 failed; 2 ignored** (matches pre-flight exactly)
- `cargo fmt --all --check` — success
- `wc -l verifier.rs` — 497 lines (≤ 500 ✓)

**Files changed:**
- `executor/src/governor/verifier.rs` — trimmed from 1163 → 497 lines; inline test block replaced with `#[cfg(test)] #[path = "verifier_tests.rs"] mod tests;`
- `executor/src/governor/verifier_tests.rs` — new file, 666 lines, full test body verbatim

**Commit:** one `refactor:` commit

**Notes for review:** Pure file-split refactor. No logic changes. Same recipe as
phase-05a (`server.rs` → `server_tests.rs`). The `#[path]` attribute is present
and required since `verifier` is a single-file module.

**End-to-end verification:** N/A — pure internal file-split refactor, no runtime artifact.

**Grep check:** `grep -c '#\[path = "verifier_tests.rs"\]' executor/src/governor/verifier.rs` → 1 ✓
