# Phase 01: Retire the cargo filter

**Milestone:** M48 — Lossless Boundary, Loud Backstop, Exact Repetition
**Status:** todo
**Depends on:** none
**Estimated diff:** ~550 lines, almost all deletions (≈230 production lines and 16 tests removed from `output_filter.rs`; two tests removed and one call site simplified in `bash.rs`; two small tests added)
**Tags:** language=rust, kind=refactor, size=m

## Goal

Command output crosses the tool boundary lossless. The structured cargo
filter — which dropped passing-test and progress lines, and in issue #13
dropped the exact lines a piped `grep` had selected — is deleted. The
lossless path (`normalize` + `compact_with_recovery`) handles every
command.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #48 — why the filter goes.
- `docs/architecture.md` § Status #10 Arc A — what the module did; only
  the phase-01 generic path survives.
- `docs/dev/milestones/M48-lossless-boundary/README.md` § Exit criteria,
  first bullet.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

Line numbers are current as of drafting (2026-10-08, master `f7613e4`);
re-derive with `grep -n` before editing.

**`executor/src/context/output_filter.rs`** (933 lines; `#[cfg(test)]` at
`:378`). Production functions, in file order:

| Lines | Symbol | Fate |
|---|---|---|
| `:37` | `pub fn normalize` | keep |
| `:76` | `pub fn compact_with_recovery` | keep |
| `:104-114` | `struct TestFailure` | delete |
| `:116-194` | `fn parse_test_failures` | delete |
| `:196-221` | `fn format_failure_digest` | delete |
| `:223-229` | `pub fn is_cargo_command` | delete |
| `:231-285` | `pub fn cargo_filter` | delete |
| `:287-319` | `fn is_cargo_noise` | delete |
| `:321-330` | `pub fn filter_for_command` | delete |
| `:335` | `fn write_recovery` | keep |

The module doc comment (`:1-8`) describes only the generic path and needs
no change. `use regex::Regex` stays in use by `ansi_re`.

The dispatcher being deleted, for reference:

```rust
pub fn filter_for_command(command: &str, raw: &str, project_root: &Path) -> (String, bool) {
    if is_cargo_command(command) {
        cargo_filter(raw, project_root)
    } else {
        compact_with_recovery(raw, project_root)
    }
}
```

**`executor/src/tools/bash.rs`**. The two production call sites:

```rust
// bash.rs:196-203
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

```rust
// bash.rs:223-235
if self.filter {
    let filter_label =
        if crate::context::output_filter::is_cargo_command(&parsed.command) {
            "cargo"
        } else {
            "generic"
        };
    metadata["output_filter"] = json!({
        "tokens_before": crate::context::tokens::count(&combined),
        "tokens_after": crate::context::tokens::count(&body),
        "filter": filter_label,
    });
}
```

`parsed.command` has no other use in that block after this change; it is
still used by the spawn above, so it does not become dead.

**Consumers of the `filter` label.** `executor/src/agent/mod.rs:1127-1147`
reads `output_filter.filter` from the tool metadata and copies it into
`SessionEvent::OutputFiltered { filter }` (`store/sessions/event.rs:109`),
whose doc comment (`event.rs:105-108`) says the label is `"generic"` or
`"cargo"`. `mcp/src/log_query.rs:28` only maps the event to its name. The
event and its `filter` field stay — session logs on disk carry it.

**Tests to delete** (16 in `output_filter.rs` `mod tests`):
`is_cargo_command_matches_cargo_subcommands`,
`cargo_filter_drops_passing_test_lines`, `cargo_filter_drops_compiling_noise`,
`cargo_filter_keeps_error_diagnostic_block`,
`cargo_filter_keeps_test_failure_block`, `cargo_filter_keeps_summary_line`,
`cargo_filter_uses_compact_when_filtered_output_still_long`,
`filter_for_command_routes_cargo_to_structured_filter`,
`filter_for_command_routes_non_cargo_to_generic`,
`parse_test_failures_extracts_all_failed_tests`,
`parse_test_failures_empty_on_passing_output`,
`parse_test_failures_preserves_left_right_labels`,
`format_failure_digest_empty_for_no_failures`,
`cargo_filter_prepends_failure_digest`,
`cargo_filter_no_digest_on_passing_output`,
`parse_test_failures_handles_bare_panic_without_left_right`.
And 2 in `bash.rs` `mod tests`:
`cargo_command_output_is_filtered_through_cargo_filter` (`:707`; it spawns
a real `cargo test` in a `TempDir`, so it is also the module's only
non-hermetic test) and `cargo_command_records_cargo_filter_label` (`:797`).

**Tests that stay and must keep passing:** the ten `normalize_*` /
`compact_*` / `dedupe_*` / `recovery_rotation_*` tests in
`output_filter.rs`; in `bash.rs`,
`filtered_bash_truncation_writes_recovery_file` (`:650`),
`kill_switch_off_uses_legacy_truncation_without_recovery`,
`filter_on_records_output_filter_metadata` (`:767`, already asserts the
label is `"generic"`), `filter_off_records_no_output_filter_metadata`.

## Spec

### 1. Delete the cargo path from `output_filter.rs`

Remove `TestFailure`, `parse_test_failures`, `format_failure_digest`,
`is_cargo_command`, `cargo_filter`, `is_cargo_noise` and
`filter_for_command` with their doc comments, so that between
`compact_with_recovery` and `write_recovery` nothing remains. Do not
touch `normalize`, `compact_with_recovery`, `write_recovery`, the
constants, or the module doc comment.

### 2. `bash.rs` calls the lossless path directly

Replace the first call site with:

```rust
let (body, truncated) = if self.filter {
    crate::context::output_filter::compact_with_recovery(&combined, self.scope.root())
} else {
    truncate_output(&combined)
};
```

and the metadata block with:

```rust
if self.filter {
    metadata["output_filter"] = json!({
        "tokens_before": crate::context::tokens::count(&combined),
        "tokens_after": crate::context::tokens::count(&body),
        "filter": "generic",
    });
}
```

The key `filter` and the value `"generic"` are load-bearing: the agent loop
reads both before it emits `OutputFiltered`
(`agent/mod.rs:1127-1147`), and `filter_on_records_output_filter_metadata`
asserts the value.

### 3. Delete the 18 tests listed in § Current state

The 16 in `output_filter.rs` and the 2 in `bash.rs`. Remove any test-only
helper or `use` that becomes unused with them (clippy under `-D warnings`
reports it).

### 4. Update the `OutputFiltered` doc comment

In `executor/src/store/sessions/event.rs:105-108`, replace the sentence
naming the two labels with: *`filter` is always `"generic"` (the lossless
normalize + head/tail path); the field is kept because persisted session
logs carry it.* Keep the tokens sentence.

### 5. New test: passing-test lines survive the boundary (`output_filter.rs`)

`passing_test_lines_survive_the_boundary` — the issue's exact shape:

```rust
#[test]
fn passing_test_lines_survive_the_boundary() {
    let dir = tempfile::TempDir::new().unwrap();
    let raw = "test a ... ok\ntest b ... ok\ntest result: ok. 2 passed; 0 failed\n";
    let (body, truncated) = compact_with_recovery(raw, dir.path());
    assert!(!truncated);
    assert_eq!(body, raw, "no line may be dropped on content");
}
```

(`tempfile` is already a dev-dependency used by this module's tests; follow
the `TempDir` construction the neighbouring tests use.)

### 6. New test: a piped cargo-style command is not filtered (`bash.rs`)

`piped_cargo_style_output_is_not_filtered`, modelled on
`filtered_bash_truncation_writes_recovery_file` (`bash.rs:650`): build the
tool with `bash_with_filter(scope, 30, true)`, execute

```json
{ "command": "sh -c 'printf \"test a ... ok\\ntest b ... ok\\ntest result: ok. 2 passed; 0 failed\\n\" | grep -E \"^test \"'" }
```

and assert `result.error.is_none()`, that `result.output` contains
`"test a ... ok"`, `"test b ... ok"` **and** `"test result: ok"`, and that
`metadata["output_filter"]["filter"]` is `"generic"`. The command runs
`sh`/`printf`/`grep` only — hermetic, no cargo.

### 7. Capture the end-to-end evidence

Run the block in § End-to-end verification **verbatim and unmodified**,
paste the artifact file into a new Update Log entry headed
`### Update — <date> (end-to-end verification)`, and append the
`PASTE MATCH` / `PASTE MISMATCH` line the self-check prints. On
`PASTE MISMATCH`, fix the pasted fence from the file and re-run the
self-check until it prints `PASTE MATCH`. The server-authored `(complete)`
entry does not satisfy this task.

**Read the artifact with `read_file`, never from the command's stdout.**
The server you are running under still carries the old filter; stdout of
any command beginning with `cargo` is still filtered until this phase is
built and the server restarted. The file is not.

## Acceptance criteria

- [ ] Test `passing_test_lines_survive_the_boundary` passes.
- [ ] Test `piped_cargo_style_output_is_not_filtered` passes.
- [ ] **Preservation criteria:** `filtered_bash_truncation_writes_recovery_file`,
      `filter_on_records_output_filter_metadata`,
      `kill_switch_off_uses_legacy_truncation_without_recovery`, and the ten
      `normalize_*`/`compact_*`/`dedupe_*`/`recovery_rotation_*` tests still
      pass.
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/context/output_filter.rs | grep -c 'fn cargo_filter(\|fn is_cargo_command(\|fn is_cargo_noise(\|fn parse_test_failures(\|fn format_failure_digest(\|fn filter_for_command('`
      prints `0` (now 6).
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/context/output_filter.rs | grep -c 'struct TestFailure'`
      prints `0` (now 1).
- [ ] `grep -c 'is_cargo_command\|filter_for_command\|cargo_filter' executor/src/tools/bash.rs`
      prints `0` (now 4: two production sites, two test names).
- [ ] `sed -n '1,/^#\[cfg(test)\]/p' executor/src/tools/bash.rs | grep -c 'compact_with_recovery'`
      prints `1` (now 0).
- [ ] `grep -c '#\[test\]' executor/src/context/output_filter.rs` prints
      `11` — (now 26) − 16 deleted + 1 added.
- [ ] `grep -c '#\[tokio::test\]\|#\[test\]' executor/src/tools/bash.rs`
      prints `27` — (now 28) − 2 deleted + 1 added.
- [ ] `grep -c '"cargo"' executor/src/store/sessions/event.rs` prints `0`
      (now 1, the doc comment).
- [ ] The E2E Update Log entry ends with the line `PASTE MATCH`.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- `passing_test_lines_survive_the_boundary` in
  `executor/src/context/output_filter.rs` — asserts the lossless path
  returns three cargo-shaped lines byte-identical, not truncated.
- `piped_cargo_style_output_is_not_filtered` in `executor/src/tools/bash.rs`
  — asserts a `bash` call whose output is `test … ok` lines plus a summary
  delivers all of them with the `generic` label.
- Deletions as listed; no other test changes.

## End-to-end verification

The real artifact is the executor library's `bash` tool behaviour,
exercised by the two named tests, plus the symbol counts. Run exactly this
block from the repo root; it is Spec's final task:

```bash
mkdir -p target/e2e
A=target/e2e/m48-phase-01.txt; : > "$A"
cargo test -p rexymcp-executor passing_test_lines_survive_the_boundary 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
cargo test -p rexymcp-executor piped_cargo_style_output_is_not_filtered 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result' >> "$A"; echo "exit=${PIPESTATUS[0]}" >> "$A"
echo "cargo_fns=$(sed -n '1,/^#\[cfg(test)\]/p' executor/src/context/output_filter.rs | grep -c 'fn cargo_filter(\|fn is_cargo_command(\|fn is_cargo_noise(\|fn parse_test_failures(\|fn format_failure_digest(\|fn filter_for_command(')" >> "$A"
echo "bash_cargo_refs=$(grep -c 'is_cargo_command\|filter_for_command\|cargo_filter' executor/src/tools/bash.rs)" >> "$A"
echo "bash_compact=$(sed -n '1,/^#\[cfg(test)\]/p' executor/src/tools/bash.rs | grep -c 'compact_with_recovery')" >> "$A"
```

Expected: each test run shows its `test … ok` line and a
`test result: ok. 1 passed` line with `exit=0`; `cargo_fns=0`;
`bash_cargo_refs=0`; `bash_compact=1`.

Paste `target/e2e/m48-phase-01.txt` byte-for-byte into a new Update Log
entry headed `### Update — <date> (end-to-end verification)`, then run the
paste self-check and append its verdict line to the same entry:

```bash
D=docs/dev/milestones/M48-lossless-boundary/phase-01-retire-cargo-filter.md
L=$(grep -n '^### Update .*(end-to-end verification)' "$D" | tail -1 | cut -d: -f1)
tail -n +"$L" "$D" | awk '/^```/{c++; next} c==1{print} c==2{exit}' > target/e2e/pasted-m48-01.txt
diff target/e2e/pasted-m48-01.txt target/e2e/m48-phase-01.txt && echo "PASTE MATCH" || echo "PASTE MISMATCH"
```

The server-authored `(complete)` entry does not satisfy this section.

## Authorizations

None. No dependencies, no `Cargo.toml`, no config-schema change (the
`[context] output_filter` switch keeps its meaning: on = lossless path with
recovery file, off = legacy head/tail with no recovery file).

## Out of scope

- **`[context] output_filter`** — do not remove or rename the switch.
- **`SessionEvent::OutputFiltered`** and its `filter` field — keep; only the
  doc comment changes (Spec §4).
- **`compact_with_recovery`'s thresholds** (`LINE_CAP`, `HEAD_LINES`,
  `TAIL_LINES`) — unchanged.
- **`docs/architecture.md`, `README.md`, plugin skills** — architect-owned;
  the README sentences describing the structured compressor are updated at
  milestone close.
- **Governor changes** — phases 02 and 03.
