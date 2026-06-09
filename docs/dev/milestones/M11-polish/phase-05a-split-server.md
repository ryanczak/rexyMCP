# Phase 05a: Split `server.rs` — extract test suite

**Milestone:** M11 — Polish
**Status:** todo
**Depends on:** phase-04 (ordering only)
**Estimated diff:** ~0 net lines (move only)
**Tags:** language=rust, kind=refactor, size=s

## Goal

`mcp/src/server.rs` is 1 225 lines: ~519 lines of production code (MCP server
handler, inner logic functions, tool implementations) followed by ~706 lines of
`#[cfg(test)] mod tests { … }`. Extract the test block into
`mcp/src/server_tests.rs`.

**Zero logic changes.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Run `cargo test -p rexymcp` and confirm all tests pass on HEAD.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`mcp/src/server.rs` (1 225 lines):
- Lines 1–519 (approx): production code — `RexyMcpServer`, `McpProgressNotifier`,
  `execute_phase_inner`, `executor_health_inner`, log query inners,
  `model_scorecard_inner`, the `#[rmcp::tool_router]` impl, `ServerHandler`.
- Lines 520–1225 (approx): `#[cfg(test)] mod tests { … }`.

## Spec

### Step 1: determine exact boundary

```bash
grep -n "^#\[cfg(test)\]" mcp/src/server.rs
wc -l mcp/src/server.rs
```

### Step 2: create `mcp/src/server_tests.rs`

Read `server.rs` in two range-reads (~650 lines each). Write `server_tests.rs`
containing the inner body of `mod tests { … }` — everything between the outer
braces, not including the `mod tests { … }` wrapper itself.

The file will begin with the `use` imports from the original test block (including
`use super::*;` or whatever the test imports actually are — copy them exactly).

### Step 3: trim `server.rs` and add declaration

Remove the `#[cfg(test)]` block from `server.rs` and append:

```rust
#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
```

(Same `#[path]` pattern as phase-04 — necessary because the file is named
`server_tests.rs`, not `tests.rs`.)

### Step 4: verify and format

```bash
cargo build -p rexymcp
cargo test -p rexymcp
rustfmt mcp/src/server.rs mcp/src/server_tests.rs
cargo fmt --all --check
```

## Acceptance criteria

- [ ] `mcp/src/server.rs` is ≤ 530 lines (production code only).
- [ ] `mcp/src/server_tests.rs` exists and contains all tests.
- [ ] `cargo build -p rexymcp` succeeds with zero warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test -p rexymcp` passes — same test count as before.
- [ ] `cargo fmt --all --check` passes.
- [ ] No logic change of any kind.

## Notes for executor

- `server.rs` is 1 225 lines — readable in two range-reads of ~650 lines each.
  Do NOT attempt a full single read.
- The `server_tests.rs` file will contain test helpers that call the `_inner`
  functions directly (e.g. `execute_phase_inner_with_client`). Those functions
  are `pub(crate)` in `server.rs` and will remain accessible to the test module
  via `use super::*;`.
- Check whether the test block imports `use super::*;` or individual items — copy
  exactly.

## Update Log

<!-- entries appended below this line -->
