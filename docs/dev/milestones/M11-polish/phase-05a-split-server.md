# Phase 05a: Split `server.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** done
**Depends on:** phase-04 (ordering only — no code dependency; same refactor class)
**Estimated diff:** ~0 net lines (pure move — no logic changes)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`mcp/src/server.rs` is **1 225 lines** — ~518 lines of production code (the
`rmcp` MCP server impl: `RexyMcpServer`, `McpProgressNotifier`, the `_inner`
tool entrypoints, the `#[rmcp::tool_router]` impl, `ServerHandler`) followed by
a single ~704-line `#[cfg(test)] mod tests { … }` block. Move the test block into
a new sibling file `mcp/src/server_tests.rs`, leaving `server.rs` as
production-code-only.

**Zero logic changes.** Every test moves byte-for-byte; the only production-source
change is replacing the inline `mod tests { … }` with a file-module declaration.
All gates pass identically before and after; the mcp test count is unchanged.
This is the **same refactor as phase-04** (`scorecard.rs` → `scorecard_tests.rs`,
landed clean first-try via `sed`) — follow that recipe exactly.

## Architecture references

Read before starting:

- `docs/architecture.md#status` — M11 §"File decomposition" names this phase
  (`server.rs` is one of the four largest source files); pure move refactor, no
  logic changes.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp 2>&1 | tail -3` and record the passing test count
   (expected: **270**). The same count must pass after the move.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Boundaries verified on HEAD:

```
518:                 ← last production line (blank; the closing brace of the last
                       production item is on an earlier line)
519:#[cfg(test)]
520:mod tests {
521:    use super::*;
522:    use tempfile::TempDir;
        … ~702 lines, 34 fns (helpers + 28 #[test]/#[tokio::test]) …
1224:    }            ← closes the final test fn
1225:}                ← closes `mod tests {`
```

So the **inner body** of the test module is **lines 521–1224**, and production
code is **lines 1–518**.

> **These line numbers were grep-verified against HEAD while drafting.** Re-confirm
> as the first step; if a rebase shifted them, adapt the `sed` ranges to the
> confirmed numbers.

`server` is declared `mod server;` in `mcp/src/main.rs:15` — a **single-file
module**, which determines the declaration form in Step 3 below.

**The test block contains `#[tokio::test]` async tests** (e.g. at lines 556, 570,
586, 601, 1138, 1196). This changes **nothing** about the move — `sed` moves the
attributes and bodies verbatim, and `use super::*;` resolves the same imports from
the sibling file. Do not add, remove, or rewrite any `#[tokio::test]`/`#[test]`
attribute.

## Spec

### The method: move with `sed`, do NOT retype the body

This is the same move just done for `mcp/src/scorecard.rs` →
`mcp/src/scorecard_tests.rs` (phase-04, landed clean first-try via `sed`). **Do
not reproduce the ~704-line test body with `write_file` or `patch`** — a verbatim
regeneration risks truncation/transcription errors and the repeated-patch churn
that has stalled split refactors before. Let the shell move the bytes losslessly.
The `bash` tool permits `sed`/`mv`/`printf` and in-scope redirects (the classifier
blocks only device writes / `mkfs` / `git push` / `rm -rf /` etc.).

### Step 1 — confirm the boundaries

```bash
wc -l mcp/src/server.rs
grep -n '^#\[cfg(test)\]$' mcp/src/server.rs   # expect: 519
sed -n '519,521p' mcp/src/server.rs            # expect: #[cfg(test)] / mod tests { / use super::*;
tail -n 2 mcp/src/server.rs                    # expect: a closing `}` (line 1225)
```

Use the confirmed numbers: `BODY_START` = the `use super::*;` line (expected 521),
`PROD_END` = the line before `#[cfg(test)]` (expected 518), `BODY_END` = total
lines − 1 (the line before the final `}`, expected 1224).

### Step 2 — extract the test body to `server_tests.rs`

The new file is the module *body* — the inner content **without** the outer
`mod tests { … }` wrapper and **without** the `#[cfg(test)]` attribute (those are
supplied by the declaration in Step 3). So it starts at `use super::*;` and ends
at the last test fn's closing `}`:

```bash
sed -n '521,1224p' mcp/src/server.rs > mcp/src/server_tests.rs
```

### Step 3 — trim `server.rs` and add the file-module declaration

Cannot redirect into `server.rs` while reading it; write to a temp then move:

```bash
{ sed -n '1,518p' mcp/src/server.rs; \
  printf '#[cfg(test)]\n#[path = "server_tests.rs"]\nmod tests;\n'; } \
  > mcp/src/server.rs.new
mv mcp/src/server.rs.new mcp/src/server.rs
```

**The `#[path = "server_tests.rs"]` attribute is REQUIRED and load-bearing.**
`server` is a *single-file* module (`mcp/src/server.rs`, not `server/mod.rs`). A
bare `mod tests;` inside it makes the compiler look for `mcp/src/server/tests.rs`
or `mcp/src/server/tests/mod.rs` — **not** the sibling `server_tests.rs`, so the
build would fail with "file not found for module `tests`". The `#[path]` attribute
points the `tests` module at the sibling file explicitly. (This is exactly the
phase-04 situation: `scorecard` was likewise a single-file module and required
`#[path = "scorecard_tests.rs"]`. Contrast `executor/src/agent/tests.rs`, which
needed no `#[path]` because `agent` is a *directory* module.)

### Step 4 — format the two touched files

Format only the touched files — **never** run the writing form `cargo fmt --all`:

```bash
rustfmt mcp/src/server.rs mcp/src/server_tests.rs
```

### Step 5 — verify

```bash
cargo build -p rexymcp 2>&1 | tail -5
cargo clippy -p rexymcp --all-targets --all-features -- -D warnings 2>&1 | tail -5
cargo test -p rexymcp 2>&1 | tail -3
cargo fmt --all --check
wc -l mcp/src/server.rs
```

The test count must equal Pre-flight (270). `wc -l server.rs` must be ≤ 525
(production lines 1–518 + the 3-line declaration).

## Acceptance criteria

- [ ] `mcp/src/server.rs` is ≤ 525 lines and contains **no** inline
  `#[cfg(test)] mod tests { … }` block — only the
  `#[cfg(test)] #[path = "server_tests.rs"] mod tests;` declaration.
- [ ] `mcp/src/server_tests.rs` exists and contains the full test body
  (begins with `use super::*;`, ends with the last test fn's closing brace).
- [ ] `cargo build -p rexymcp` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp` passes with the **same test count as Pre-flight
  (270)** — no test added, removed, renamed, or skipped.
- [ ] `cargo fmt --all --check` passes.
- [ ] `git diff --stat` shows exactly two source files: `server.rs` (large
  deletion) and `server_tests.rs` (large addition).

## Test plan

No new tests. This is a pure file-split move — the existing test fns in the block
provide complete coverage and must pass unchanged. The "same test count before and
after" criterion is the regression guard: any transcription or boundary error
shows up as a changed count or a compile failure.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. Pure internal
> file-split refactor: no production behavior, MCP-tool surface, or config
> changes; the only build-visible effect (test compilation) is covered by
> `cargo test`.

## Authorizations

None. (No new dependency; no `unsafe`; no edit to `Cargo.toml`, the architecture
doc, or any other phase doc. `server_tests.rs` is a new source file the spec
explicitly requires, so it is not an unauthorized new file.)

## Out of scope

- Do **not** sub-split `server_tests.rs` — one flat file is the goal.
- Do **not** edit, rename, reorder, add, or delete any test, helper fn, `use`
  statement, `#[allow]`, `#[test]`, or `#[tokio::test]`. Move the body
  byte-for-byte.
- Do **not** touch `main.rs`, `runner.rs`, or any other file — the tests
  reference production items via `use super::*;` / `crate::…`, which still
  resolves unchanged from `server_tests.rs`.
- Do **not** change any production code in `server.rs` lines 1–518.

## Notes for executor

- **Why `sed`, not `write_file`:** the test body is ~704 lines. Regenerating it
  through `write_file` risks truncation/corruption and the repeated-patch churn
  that stalls large moves. `sed` moves the bytes losslessly in one command — no
  regeneration, no transcription risk. This is the prescribed method.
- **The `#[path]` attribute is the one thing that will break the build if
  omitted** — see Step 3. It is required because `server` is a single-file
  module, exactly as in phase-04's `scorecard` split.
- The `#[tokio::test]` async tests in the block need **no special handling** —
  they move with the body verbatim. Do not "convert" or touch them.
- The test helpers call `pub(crate)` `_inner` functions directly (e.g.
  `execute_phase_inner_*`); they remain accessible from `server_tests.rs` via
  `use super::*;` unchanged.
- `server.rs` is 1 225 lines, but you do **not** need to read the test body to
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

Moved `#[cfg(test)] mod tests { … }` from `mcp/src/server.rs` (1 225 lines) into sibling `mcp/src/server_tests.rs` via `sed`, per phase-04 recipe.

### Update — 2025-07-09 (complete)

**Executor:** rexyMCP executor

**Verification commands (all clean):**
- `cargo build -p rexymcp` — success, zero warnings
- `cargo clippy -p rexymcp --all-targets --all-features -- -D warnings` — success, zero warnings
- `cargo test -p rexymcp` — **270 passed** (matches pre-flight count)
- `cargo fmt --all --check` — clean
- `wc -l mcp/src/server.rs` → 521 (≤ 525 ✓)
- `grep '#\[path' mcp/src/server.rs` → `#[path = "server_tests.rs"]` present ✓
- `head -1 mcp/src/server_tests.rs` → `use super::*;` ✓
- `tail -1 mcp/src/server_tests.rs` → `}` ✓

**Files changed:**
- `mcp/src/server.rs` — trimmed from 1 225 → 521 lines (production code + 3-line module declaration)
- `mcp/src/server_tests.rs` — new, 704 lines (full test body, byte-for-byte move via `sed`)

**Commit:** `refactor: split server.rs test suite into server_tests.rs`

**Notes for review:** None — clean first-try split, identical to phase-04 pattern.

**End-to-end verification:** N/A — pure internal file-split refactor; no runtime-loadable artifact.

### Review verdict — 2026-06-09

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (the Update Log self-labels "rexyMCP
  executor" and stamps `2025-07-09` — the recurring local-LLM identity/clock
  quirk, cosmetic)
- **Scope deviations:** none — the diff is exactly two source files
  (`server.rs` 708-line deletion, `server_tests.rs` 702-line addition) plus the
  two doc files. **Production lines 1–518 are byte-identical to the parent**
  (verified by `diff`); `server.rs` ends at 521 lines with the required
  `#[cfg(test)] #[path = "server_tests.rs"] mod tests;` declaration.
- **Independent re-run:** `cargo fmt --all --check` ✅, `cargo build` ✅ (zero
  warnings), `cargo clippy --all-targets --all-features -- -D warnings` ✅,
  `cargo test` → **270 mcp + 665 executor** pass, zero failures. All 38 test fns
  moved losslessly (test-fn-name diff between parent block and `server_tests.rs`
  is empty). The body landed at 702 lines vs. the spec's ~704 estimate — rustfmt
  collapsed two trailing blank lines (Step 4 authorized `rustfmt` on the touched
  files); behaviorally identical.
- **Calibration:** none new. The `sed`-move recipe landed clean first-try
  again (27 turns) — **fifth consecutive split refactor** (M8 ×2, M11 phases
  03/04/05a) on the prescribed lossless-move method. The `#[tokio::test]`
  async-tests-move-verbatim pre-injection was correct (no special handling was
  needed). Two pre-existing `eprintln!` calls in production (`server.rs:426`,
  `:450`) are untouched by this phase — note for a future sweep, not a
  phase-05a defect.
