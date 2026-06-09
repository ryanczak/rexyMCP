# Phase 03: Split `agent/mod.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** review
**Depends on:** phase-01 (the test block references `GovernorConfig`, added in phase-01 — already on HEAD)
**Estimated diff:** ~0 net lines (pure move — no logic changes)
**Tags:** language=rust, kind=refactor, size=m

## Goal

`executor/src/agent/mod.rs` is **4 431 lines / ~163 KB** — about 881 lines of
production code followed by a single ~3 547-line `#[cfg(test)] mod tests { … }`
block. The file is well over the 100 KB `runaway_output_bytes` limit, so the
executor can only ever range-read it, never see it whole. Move the entire test
block into a new sibling file `executor/src/agent/tests.rs`, leaving `mod.rs` as
production-code-only (≤ 900 lines).

**Zero logic changes.** Every test moves byte-for-byte; nothing is added,
removed, renamed, or reordered. The only production-source change is replacing the
inline `mod tests { … }` with a `mod tests;` file-module declaration. All gates
pass identically before and after, and the executor test count is unchanged.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M11 §"File decomposition" names this phase and
  the ≤ 900-line target; it is a *pure move refactor, no logic changes*.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor 2>&1 | tail -3` and record the passing
   test count (expected: **665**). The same count must pass after the move.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

```
executor/src/agent/
  mod.rs        ← 4 431 lines.  Production: lines 1–881.  Tests: lines 882–4431.
  command.rs    ┐
  contract.rs   │ existing sibling file-modules, declared in mod.rs lines 9–18
  progress.rs   │ (`pub mod command;` → command.rs, `mod log;` → log.rs, …).
  prompt.rs     │ This phase adds `tests.rs` as one more sibling, declared
  verify.rs     │ `#[cfg(test)] mod tests;`.
  log.rs        │
  metrics.rs    │
  outcome.rs    │
  tools.rs      ┘
```

The test block, verified on HEAD:

```
882:#[cfg(test)]
883:mod tests {
884:    use super::*;
885:    use crate::agent::command::{CommandResult, MAX_COMMAND_TAIL_CHARS};
886:    use crate::ai::testing::{MockAiClientScript, MockCall};
        … ~3 540 lines of helpers + #[test] fns …
4430:    }                ← closes the final #[test] fn
4431:}                    ← closes `mod tests {`
```

So the **inner body** of the test module is **lines 884–4430**, and production
code is **lines 1–881** (line 880 is `}`, line 881 is blank).

> **These line numbers were grep-verified against HEAD while drafting.** Re-confirm
> them as the very first step (the boundary, not the literals, is what matters —
> if a rebase shifted them, adapt the `sed` ranges below to the confirmed numbers).

## Spec

### The method: move with `sed`, do NOT retype the body

The test body is ~3 547 lines. **Do not attempt to reproduce it with `write_file`
or `patch`** — a verbatim 3 500-line regeneration will either truncate at the
token limit or introduce transcription errors, and the repeated patch attempts are
exactly the churn that has stalled prior split phases. Instead, let the shell move
the bytes losslessly. The `bash` tool permits `sed`/`mv`/`printf` and in-scope file
redirects (only device writes / `mkfs` / `git push` / `rm -rf /` etc. are blocked),
so this is a handful of allowed commands.

### Step 1 — confirm the boundaries

```bash
wc -l executor/src/agent/mod.rs
grep -n '^#\[cfg(test)\]$' executor/src/agent/mod.rs   # expect: 882
sed -n '882,884p' executor/src/agent/mod.rs            # expect: #[cfg(test)] / mod tests { / use super::*;
tail -n 2 executor/src/agent/mod.rs                    # expect: a closing `}` (line 4431)
```

Let `CFG` = the `#[cfg(test)]` line (expected 882), `OPEN` = `CFG+1` (the
`mod tests {` line, 883), `BODY_START` = `CFG+2` (the `use super::*;` line, 884),
`PROD_END` = `CFG-1` (last production line, 881), and `BODY_END` = total lines − 1
(the line *before* the final `}`, expected 4430). Use the confirmed numbers in the
commands below.

### Step 2 — extract the test body to `tests.rs`

The new file is the module *body* — the inner content **without** the outer
`mod tests { … }` wrapper and **without** the `#[cfg(test)]` attribute. Those are
supplied by the `mod tests;` declaration, not by the file (exactly as `log.rs` is
the body of `mod log;` declared at `mod.rs:15`). So `tests.rs` starts at the
`use super::*;` line and ends at the last test fn's closing `}`:

```bash
sed -n '884,4430p' executor/src/agent/mod.rs > executor/src/agent/tests.rs
```

### Step 3 — trim `mod.rs` to production + the module declaration

Cannot redirect into `mod.rs` while reading it (the shell truncates it on open
before `sed` reads). Write to a temp file, then move it into place:

```bash
{ sed -n '1,881p' executor/src/agent/mod.rs; printf '#[cfg(test)]\nmod tests;\n'; } \
  > executor/src/agent/mod.rs.new
mv executor/src/agent/mod.rs.new executor/src/agent/mod.rs
```

After this, `mod.rs` is lines 1–881 of production code followed by:

```rust
#[cfg(test)]
mod tests;
```

### Step 4 — format the two touched files

Per the project convention, format only the files this phase touched — **never**
run the writing form `cargo fmt --all`:

```bash
rustfmt executor/src/agent/mod.rs executor/src/agent/tests.rs
```

### Step 5 — verify

```bash
cargo build -p rexymcp-executor 2>&1 | tail -5
cargo clippy -p rexymcp-executor --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp-executor 2>&1 | tail -3
cargo fmt --all --check
wc -l executor/src/agent/mod.rs
```

The test count must equal the Pre-flight count (665). `wc -l mod.rs` must be
≤ 900.

## Acceptance criteria

- [ ] `executor/src/agent/mod.rs` is ≤ 900 lines and contains **no**
  `#[cfg(test)] mod tests { … }` inline block — only the `#[cfg(test)] mod tests;`
  declaration.
- [ ] `executor/src/agent/tests.rs` exists and contains the full test body
  (begins with `use super::*;`, ends with the last test fn's closing brace).
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes with the **same test count as
  Pre-flight (665)** — no test added, removed, renamed, or skipped.
- [ ] `cargo fmt --all --check` passes.
- [ ] `git diff --stat` shows exactly two files: `mod.rs` (large deletion) and
  `tests.rs` (large addition), with no net change to test *content*.

## Test plan

No new tests. This is a pure file-split move — the executor's existing ~665 tests
provide complete coverage and must pass unchanged. The acceptance criterion
"same test count before and after" is the regression guard: any transcription or
boundary error shows up as a changed count or a compile failure.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. It is a pure internal
> file-split refactor: no production behavior, CLI surface, or config changes; the
> only build-visible effect (test compilation) is already covered by
> `cargo test`.

## Authorizations

None. (No new dependency; no `unsafe`; no edit to `Cargo.toml`, the architecture
doc, or any other phase doc. `tests.rs` is a new source file the spec explicitly
requires, so it is not an unauthorized new file.)

## Out of scope

- Do **not** sub-split `tests.rs` into multiple files — one flat file is the goal.
- Do **not** edit, rename, reorder, add, or delete any test, helper fn, `use`
  statement, or `#[allow]`. Move the body byte-for-byte.
- Do **not** touch any other file (`command.rs`, `verify.rs`, etc.) — if a test
  references them it already did so via `use super::*;` / `crate::…`, which still
  resolves unchanged from `tests.rs`.
- Do **not** change any production code in `mod.rs` lines 1–881.

## Notes for executor

- **Why `sed`, not `write_file`:** the test body is ~3 547 lines. Regenerating it
  through `write_file` would truncate at the token limit and/or corrupt content,
  and repeated `patch` attempts on a block this size are the exact churn that has
  stalled large-move refactors before. `sed` moves the bytes losslessly in one
  command — no regeneration, no transcription risk. This is the prescribed method.
- **Never `read_file` the whole `mod.rs`** — at 163 KB it exceeds
  `runaway_output_bytes` and will hard-fail. You don't need to read the body at
  all; `sed` operates on it without loading it into the conversation.
- **The file-module relationship:** `#[cfg(test)] mod tests;` in `mod.rs` tells
  Rust the file `tests.rs` *is* the module body and compiles only under
  `cfg(test)`. This is the same relationship `mod log;` (`mod.rs:15`) has with
  `log.rs`. `use super::*;` inside `tests.rs` resolves to items in `mod.rs` (the
  parent) — identical to before the move.
- If `cargo test` reports a different count or a compile error after the move, the
  cause is a wrong boundary line in Step 2/3 — re-run the Step 1 greps and fix the
  `sed` ranges. Do not "fix" it by editing test contents.
- Commit as a single `refactor:` commit; the body explains *why* (the file
  exceeded the read limit), not *what*.

## Update Log

<!-- entries appended below this line -->

### Update — 2025-07-15 (started)

**Executor:** rexyMCP executor
**Action:** Begin phase 03 — split `agent/mod.rs` test suite into `tests.rs`
**Pre-flight test count:** 665 passed, 2 ignored
**Boundaries confirmed:** `#[cfg(test)]` at line 882, `mod tests {` at 883, `use super::*;` at 884, file ends at line 4431

### Update — 2025-07-15 (complete)

**Executor:** rexyMCP executor
**Action:** Split `agent/mod.rs` test suite into `tests.rs`

**Command output:**
- `cargo build -p rexymcp-executor`: success, zero warnings
- `cargo clippy -p rexymcp-executor --all-targets --all-features -- -D warnings`: passed
- `cargo test -p rexymcp-executor`: 665 passed, 2 ignored, 0 failed (matches pre-flight)
- `cargo fmt --all --check`: passed
- `wc -l executor/src/agent/mod.rs`: 883 lines (≤ 900 ✓)

**Files changed:**
- `executor/src/agent/mod.rs`: trimmed from 4 431 to 883 lines (production code + `#[cfg(test)] mod tests;`)
- `executor/src/agent/tests.rs`: new file, 3 541 lines (full test body, byte-for-byte move)

**Verification summary:** build clean, clippy clean, fmt clean, 665/665 tests pass, mod.rs ≤ 900 lines.

**End-to-end verification:** N/A — pure internal file-split refactor, no runtime-loadable artifact.

**Notes for review:** None. Pure move — zero logic changes.

**Commit:** single `refactor:` commit
