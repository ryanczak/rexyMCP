# M14 phase-02 — Deferred cleanup sweep: prod `eprintln!`, stale doc-comment, `symbols` copy bug

**Status:** review

**Milestone:** [M14 — Cleanup](README.md)

**Depends on:** phase-01 (done)

**Estimated diff:** ~40 lines

**Tags:** language=rust, kind=chore, size=s

## Goal

Sweep three small, long-deferred cleanup items carried since M11/M12 — all
noted in prior reviews as "fix in a future sweep," none worth a dispatch on
their own. They are independent; this phase gathers them into one chore so the
milestone closes clean:

1. Two production `eprintln!` diagnostics in the MCP server request handler
   (`mcp/src/server.rs:426`, `:450`) — debug-print cruft per STANDARDS §1.
2. A stale doc-comment in `executor/src/tools/read_file.rs:17` referencing the
   compile-time constant `RUNAWAY_OUTPUT_BYTES`, which M11 phase-01 moved to
   the `[governor]` config field `runaway_output_bytes`.
3. A copy-paste bug in `executor/src/tools/symbols.rs:601`: the
   **references-mode** truncation note tells the model to "add a kind filter,"
   but references mode **rejects** the `kind` arg (`symbols.rs:174`).

## Architecture references

Read before starting:

- `docs/dev/STANDARDS.md` §1 — DoD: "No `dbg!`, `println!` debug calls" (the
  two `server.rs` `eprintln!` are the same debug-print class).
- `docs/dev/STANDARDS.md` §2.3 — Comments: a comment that has rotted (names a
  symbol that no longer exists) is exactly the kind this section forbids.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.
4. Confirm gates are green before touching any file:

```
cargo fmt --all --check
cargo build
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Current state

### Item 1 — `mcp/src/server.rs` (two diagnostic `eprintln!`)

The `execute_phase` handler corroborates `repo_path` against the client's
roots, then logs two unconditional diagnostics to stderr. The first is in the
`NoSources` corroboration arm; the second reports whether the client sent a
`progressToken`:

```rust
// server.rs:421-457 (current)
match roots::corroborate(&repo_path, &roots_list, project_dir.as_deref()) {
    roots::Corroboration::Matched(_) => {
        let _ = ();
    }
    roots::Corroboration::NoSources => {
        eprintln!(
            "execute_phase: no roots or CLAUDE_PROJECT_DIR available for \
             corroboration of repo_path={}",
            repo_path.display()
        );
    }
    roots::Corroboration::Mismatch { .. } => {
        return Err(rmcp::ErrorData::invalid_params(
            roots::format_mismatch_error(
                &repo_path,
                &roots_list,
                project_dir.as_deref(),
            ),
            None,
        ));
    }
}

let progress_token = request.meta.as_ref().and_then(|m| m.get_progress_token());
// Diagnostic: whether the client opted into live progress. The
// MCP spec only permits notifications/progress for a request
// that carried a progressToken, so a "no token" line here
// explains absent live status (the logged Progress records and
// `rexymcp status` are unaffected — they don't need the token).
eprintln!(
    "execute_phase: client progress_token {}",
    if progress_token.is_some() {
        "present (live notifications enabled)"
    } else {
        "absent (no live notifications; use `rexymcp status`)"
    }
);
let progress_callback: Option<Box<dyn ProgressCallback>> =
    progress_token.map(|token| {
        Box::new(McpProgressNotifier {
            peer: context.peer.clone(),
            progress_token: token,
        }) as Box<dyn ProgressCallback>
    });
```

**Note:** the many `eprintln!` in `mcp/src/main.rs` and `mcp/src/init.rs` are
**legitimate** user-facing CLI stderr at the binary entry point (STANDARDS §2.1
permits user-visible output at `main`). They are **out of scope** — touch only
the two in `server.rs`.

There is **no `tracing`/`log` facility** in the `mcp` crate; adding one is a
dependency decision out of scope for this chore. The fix is **removal**, not
rerouting.

### Item 2 — `executor/src/tools/read_file.rs:15-18` (stale doc-comment)

```rust
/// Maximum lines returned in a single read_file call. Callers needing more must
/// use start_line/end_line to read in slices. Keeps output well below the
/// governor's RUNAWAY_OUTPUT_BYTES (100 KB) threshold for typical source files.
const MAX_OUTPUT_LINES: usize = 500;
```

`RUNAWAY_OUTPUT_BYTES` no longer exists as a constant — M11 phase-01 moved it
to `GovernorConfig.runaway_output_bytes` (`executor/src/config.rs:55`, default
`100 * 1024`). The comment names a dead symbol.

### Item 3 — `executor/src/tools/symbols.rs` (references-mode copy bug)

References mode rejects the `kind` arg:

```rust
// symbols.rs:171-176 (references + kind → advisory error)
error: Some("kind filter is not supported in references mode".to_string()),
```

…yet the references-mode truncation note tells the model to add one:

```rust
// symbols.rs:599-604 (format_references — the BUG)
if truncated {
    output.push_str(&format!(
        "\n[… truncated at {} references; narrow your path or add a kind filter to see more …]",
        hits.len()
    ));
}
```

The **definitions-mode** note (`format_output`, `symbols.rs:558-562`) says the
same thing — but there `kind` **is** a valid filter, so that note is **correct**
and must be left unchanged:

```rust
// symbols.rs:558-562 (format_output — CORRECT, do not touch)
if truncated {
    output.push_str(&format!(
        "\n[… truncated at {} definitions; narrow your path or add a kind filter to see more …]",
        hits.len()
    ));
}
```

In references mode the only knobs the model has are `path` and `max_results`
(`symbols.rs:59`, `:181`). The note should point at those.

## Spec

### 1. Remove the two diagnostic `eprintln!` in `mcp/src/server.rs`

**a.** Collapse the `NoSources` arm into the `Matched` arm — both are
"corroboration is acceptable, proceed" with no action. Replace the two arms
(`Matched(_)` and `NoSources`) with a single combined arm, deleting the
`eprintln!` and the now-pointless `let _ = ();`:

```rust
// new — Matched and NoSources both proceed without action; only Mismatch errors.
roots::Corroboration::Matched(_) | roots::Corroboration::NoSources => {}
roots::Corroboration::Mismatch { .. } => {
    return Err(rmcp::ErrorData::invalid_params(
        roots::format_mismatch_error(
            &repo_path,
            &roots_list,
            project_dir.as_deref(),
        ),
        None,
    ));
}
```

Behavior is unchanged: `NoSources` continues to soft-allow (it was never an
error path — only `Mismatch` returns `Err`).

**b.** Delete the second `eprintln!` (the `progress_token` diagnostic). **Keep**
the `let progress_token = …;` line and the `let progress_callback = …;` block —
they are load-bearing logic, not diagnostics. **Keep** the explanatory comment
but trim it to drop the reference to the deleted stderr line (it documents a
real MCP-spec invariant worth keeping):

```rust
let progress_token = request.meta.as_ref().and_then(|m| m.get_progress_token());
// The MCP spec only permits notifications/progress for a request that carried
// a progressToken; without one, live progress can't fire (the logged Progress
// records and `rexymcp status` are unaffected — they don't need the token).
let progress_callback: Option<Box<dyn ProgressCallback>> =
    progress_token.map(|token| {
        Box::new(McpProgressNotifier {
            peer: context.peer.clone(),
            progress_token: token,
        }) as Box<dyn ProgressCallback>
    });
```

After this task, `grep -n 'eprintln!' mcp/src/server.rs` returns **nothing**.

### 2. Fix the stale doc-comment in `executor/src/tools/read_file.rs`

Update the third line of the `MAX_OUTPUT_LINES` doc-comment to name the live
config field instead of the deleted constant. Keep the `(100 KB)` parenthetical
— that is still the default value:

```rust
/// Maximum lines returned in a single read_file call. Callers needing more must
/// use start_line/end_line to read in slices. Keeps output well below the
/// governor's `runaway_output_bytes` config (default 100 KB) for typical source
/// files.
const MAX_OUTPUT_LINES: usize = 500;
```

### 3. Fix the references-mode truncation note in `executor/src/tools/symbols.rs`

In `format_references` (the `if truncated` block, `symbols.rs:599-604`), change
the remedy from "add a kind filter" — invalid in references mode — to "raise
`max_results`":

```rust
if truncated {
    output.push_str(&format!(
        "\n[… truncated at {} references; narrow your path or raise `max_results` to see more …]",
        hits.len()
    ));
}
```

**Do not touch** the `format_output` (definitions) note at `symbols.rs:558-562`
— "add a kind filter" is correct there.

## Acceptance criteria

- [ ] `grep -n 'eprintln!' mcp/src/server.rs` returns nothing.
- [ ] The `mcp/src/main.rs` and `mcp/src/init.rs` `eprintln!` calls are
      **unchanged** (out of scope).
- [ ] `mcp/src/server.rs` corroboration `match` proceeds on both `Matched` and
      `NoSources` and still returns `Err` on `Mismatch` (behavior unchanged);
      the existing server tests pass unmodified.
- [ ] `read_file.rs`'s `MAX_OUTPUT_LINES` doc-comment no longer contains the
      string `RUNAWAY_OUTPUT_BYTES`; it names `runaway_output_bytes`.
- [ ] The `format_references` truncation note no longer contains the substring
      `kind filter`; it mentions `max_results`.
- [ ] The `format_output` (definitions) truncation note is **unchanged** and
      still contains `kind filter`.
- [ ] Test `references_truncation_note_omits_kind_filter` passes.
- [ ] All four gates pass: `cargo fmt --all --check`, `cargo build` (zero
      warnings), `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test`.
- [ ] No `TODO` / `FIXME` / `dbg!` / `println!` / commented-out code.
- [ ] No `unwrap()` / `expect()` / `panic!()` in new production paths (test
      code is exempt).

## Test plan

Items 1 and 2 are not unit-testable (absence of stderr output; a doc-comment).
They are verified by the grep acceptance criteria and the existing server-test
suite staying green. Item 3 gets one new test pinning the **negative** boundary
— the references note must not suggest the rejected `kind` filter.

### New test in `executor/src/tools/symbols.rs` (append to the `#[cfg(test)] mod tests` block)

**`references_truncation_note_omits_kind_filter`** — adapt the existing
`references_respects_max_results` setup (3+ refs, `max_results: 1` → truncated),
but assert on the **output text** rather than only the `truncated` metadata
bool:

```rust
#[tokio::test]
async fn references_truncation_note_omits_kind_filter() {
    let dir = tempfile::TempDir::new().unwrap();
    write_files(
        dir.path(),
        &[("src/lib.rs", "fn foo() { foo(); foo(); foo(); }\n")],
    );

    let tool = symbols(make_scope(&dir));
    let result = tool
        .execute(json!({
            "name": "foo",
            "mode": "references",
            "max_results": 1,
            "path": dir.path().to_string_lossy()
        }))
        .await
        .unwrap();

    // Truncation fired (more than max_results refs exist).
    assert!(result.metadata.as_ref().unwrap()["truncated"].as_bool().unwrap());
    let body = &result.output;
    assert!(
        body.contains("truncated at"),
        "expected a truncation note, got: {body}"
    );
    assert!(
        !body.contains("kind filter"),
        "references-mode truncation note must not suggest the rejected `kind` filter: {body}"
    );
    assert!(
        body.contains("max_results"),
        "references-mode truncation note should point at `max_results`: {body}"
    );
}
```

The rendered body is `ToolResult.output: String` (`registry.rs:12`). Reuse the
existing `symbols`, `make_scope`, `write_files` helpers — do **not** invent new
ones.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable real artifact. Items 1–2 are
> a stderr-diagnostic removal and a doc-comment edit (no behavior); item 3's
> model-visible truncation note is exercised end-to-end by the new unit test
> against the real `format_references`, which is the only path that renders it
> (the `symbols` tool has no CLI surface).

## Authorizations

None. (All three edits are within ordinary phase scope: `mcp/src/server.rs`,
`executor/src/tools/read_file.rs`, `executor/src/tools/symbols.rs`. No
dependency, no `unsafe`, no protected-file edit.)

## Out of scope

- The `eprintln!` calls in `mcp/src/main.rs` and `mcp/src/init.rs` — legitimate
  CLI stderr at the binary entry point. Leave them.
- Introducing a `tracing`/`log` facility in the `mcp` crate (a dependency
  decision — would need its own authorized phase).
- The `format_output` (definitions) truncation note — correct as written.
- Any change to corroboration **logic** (the `Mismatch` error path stays).
- Any `Cargo.toml` edit; any new crate dependency.
- The operational `rexymcp serve` restart follow-up (not a code change).

## Update Log

<!-- entries appended below this line -->

### Update — 2026-06-11 02:35 (started)

**Executor:** Claude (executor)
**Tasks:** 1 (remove `eprintln!`), 2 (fix stale doc-comment), 3 (fix references-mode truncation note)

### Update — 2026-06-11 02:36 (complete)

**Tasks completed:** 1, 2, 3

**Files changed:**
- `mcp/src/server.rs` — removed two `eprintln!` diagnostics; collapsed `Matched`/`NoSources` arms; trimmed progress-token comment
- `executor/src/tools/read_file.rs` — fixed stale doc-comment referencing `RUNAWAY_OUTPUT_BYTES` → `runaway_output_bytes`
- `executor/src/tools/symbols.rs` — fixed references-mode truncation note ("add a kind filter" → "raise `max_results`"); added test `references_truncation_note_omits_kind_filter`

**Verification commands (all passed):**
- `cargo fmt --all --check` — clean
- `cargo build` — zero warnings
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test` — 731 passed, 0 failed, 2 ignored

**Grep checks:**
- `grep -n 'eprintln!' mcp/src/server.rs` → no matches (exit 1) ✓
- `grep -n 'eprintln!' mcp/src/main.rs` → 6 matches (unchanged, out of scope) ✓
- `grep -n 'eprintln!' mcp/src/init.rs` → 1 match (unchanged, out of scope) ✓
- `grep -n 'RUNAWAY_OUTPUT_BYTES' executor/src/tools/read_file.rs` → no matches ✓
- `grep -n 'runaway_output_bytes' executor/src/tools/read_file.rs` → 1 match at line 17 ✓
- `grep -n 'truncated at.*references' executor/src/tools/symbols.rs` → line 601 contains `max_results` not `kind filter` ✓
- `grep -n 'kind filter' executor/src/tools/symbols.rs` → only at line 174 (error message), 560 (definitions note), 1407 (test assertion) ✓

**End-to-end verification:** N/A — phase ships no runtime-loadable real artifact. Items 1–2 are a stderr-diagnostic removal and a doc-comment edit (no behavior change); item 3's model-visible truncation note is exercised end-to-end by the new unit test against the real `format_references`.

**Notes for review:** None — straightforward cleanup, no adaptations needed.
