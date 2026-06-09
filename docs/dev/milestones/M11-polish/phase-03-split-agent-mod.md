# Phase 03: Split `agent/mod.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** todo
**Depends on:** phase-01 (test code references `GovernorConfig` after phase-01)
**Estimated diff:** ~0 net lines (move only — no logic changes)
**Tags:** language=rust, kind=refactor, size=m

## Goal

`executor/src/agent/mod.rs` is 4 420 lines — approximately 880 lines of production
code followed by 3 540 lines of `#[cfg(test)] mod tests { … }`. The file exceeds
the 100 KB RunawayOutput limit, making it impossible for the executor to read it
whole. Extract the entire test block into `executor/src/agent/tests.rs`, leaving
`mod.rs` as production-code-only (≤ 900 lines).

**Zero logic changes.** Every test stays exactly as-is; only their file location
changes. All gates must pass before and after.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp-executor` and confirm all tests pass on HEAD.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

```
executor/src/agent/
  mod.rs        ← 4 420 lines:  lines 1–877 production, lines 879–4420 tests
  command.rs
  contract.rs
  progress.rs
  prompt.rs
  verify.rs
  log.rs
  metrics.rs
  outcome.rs
  tools.rs
```

The test block begins with:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    // … 3 540 lines …
}
```

## Spec

### Step 1: determine the exact line boundaries

Before editing, run:

```bash
grep -n "^#\[cfg(test)\]" executor/src/agent/mod.rs
```

This gives the line number where the test block begins (expected: ~879). The file
ends at line 4420. Confirm with `wc -l executor/src/agent/mod.rs`.

### Step 2: create `executor/src/agent/tests.rs`

The new file contains everything that was inside `mod tests { … }` — i.e., the
inner content without the outer `mod tests { … }` wrapper and without the
`#[cfg(test)]` attribute. In Rust, when `mod.rs` declares
`#[cfg(test)] mod tests;`, the file `tests.rs` is the module body — the
`#[cfg(test)]` and `mod tests { … }` wrapper is supplied by the declaration, not
by the file.

Concretely, `tests.rs` should start with the `use super::*;` line and all other
`use` imports from the original block, and end with the last `}` of the test
module body (but NOT the closing `}` of the outer `mod tests { … }`).

Use range-reads and `write_file` to create `tests.rs`. Because the test block is
~3 540 lines, write it in one `write_file` call using content assembled from
multiple range-reads of `mod.rs`. You MUST NOT read the full `mod.rs` in a single
`read_file` call — that will trip RunawayOutput. Read it in segments:

```bash
# Find exact line numbers first
grep -n "^#\[cfg(test)\]\|^mod tests\|^    use super" executor/src/agent/mod.rs | head -5
wc -l executor/src/agent/mod.rs
```

Then range-read in ~500-line chunks, assembling the content for `tests.rs`.

### Step 3: trim `mod.rs`

Remove everything from the `#[cfg(test)]` line through the end of the file, then
add the module declaration:

```rust
#[cfg(test)]
mod tests;
```

Use `patch` to make this change. The patch should:
1. Remove lines from the `#[cfg(test)]` marker to the end of file.
2. Append `#[cfg(test)]\nmod tests;\n`.

Alternatively, range-read lines 1–877 (the production block) and write them to
`mod.rs` directly, appending the two declaration lines.

### Step 4: verify

```bash
cargo build -p rexymcp-executor
cargo test -p rexymcp-executor
```

Both must pass before proceeding to `rustfmt`.

### Step 5: format

```bash
rustfmt executor/src/agent/mod.rs executor/src/agent/tests.rs
```

Then confirm:

```bash
cargo fmt --all --check
```

## Acceptance criteria

- [ ] `executor/src/agent/mod.rs` is ≤ 900 lines (production code only).
- [ ] `executor/src/agent/tests.rs` exists and contains all tests from the
  original `mod tests { … }` block.
- [ ] `cargo build -p rexymcp-executor` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp-executor` passes — every test that passed before
  still passes (same count).
- [ ] `cargo fmt --all --check` passes.
- [ ] No logic change of any kind: no test added, removed, or modified.

## Notes for executor

- This is a **pure file-split refactor**. If any test fails after the move, the
  cause is a transcription error — re-check the boundary lines carefully.
- `use super::*;` in `tests.rs` refers to items in `mod.rs` (the parent), which
  is correct — it is the same relationship as before.
- The `#[cfg(test)] mod tests;` declaration in `mod.rs` makes the entire
  `tests.rs` file compile only under `#[cfg(test)]`, so no production code is
  affected.
- `event_kind` and other test-only helpers in the original block stay in `tests.rs`
  — they are not needed by production code.
- Do NOT try to sub-split `tests.rs` into multiple files. One flat file is the
  goal for this phase.
- Do NOT add or remove any `use` statements, `#[allow]` annotations, or test
  helper structs/fns. Move them exactly as-is.

## Update Log

<!-- entries appended below this line -->
