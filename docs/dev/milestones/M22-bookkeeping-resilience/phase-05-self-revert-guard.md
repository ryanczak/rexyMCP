# Phase 05: Self-revert guard

**Milestone:** M22 — Bookkeeping-Loop Resilience
**Status:** todo
**Depends on:** none (independent of phases 01–04)
**Estimated diff:** ~150 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Stop the executor from discarding its own work. Refuse a single-file `git
checkout <path>` / `git restore <path>` when `<path>` is a file the executor has
**edited this session** — it would silently throw away uncommitted changes the
model itself just made.

In `session-phase-04-6a32f237`, the model ran `git checkout
src/flow-table.test.ts` to escape a red test, reverting its own green edit, then
spun into the update_task loop. `security::bash_classify` already blocks the
**wholesale** destructive forms (`git reset --hard`, `git checkout .`, `git
restore .`, `git clean -f`), but not a single-file restore of an edited file.

## Architecture references

Read before starting:

- `executor/src/agent/tools.rs` — `read_before_edit_refusal` (lines 48–78) is the
  model to mirror: a pure, `working_set`-style pre-dispatch refusal returning
  `Option<String>` (a model-visible advisory, not `Result::Err`). `resolve_path`
  (top of file) and `edit_target` are alongside it.
- `executor/src/agent/mod.rs` — the pre-dispatch refusal seam (~826–869): the
  `dedupe` check, then `match read_before_edit_refusal(…) { Some(refusal) => (false,
  refusal, None), None => { …dispatch… } }`. `pre_edit_content:
  HashMap<PathBuf, Option<String>>` (declared ~171) is the **edited-this-session**
  set — its keys are exactly the files the model has edited (captured pre-edit).
- `executor/src/security/bash_classify.rs` — the existing git blocklist
  (`BLOCK_SUBSTRINGS`, `GIT_RESET_HARD_RE`). This phase complements it; do not
  duplicate the wholesale forms here.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm `cargo build` and `cargo test` exit 0.

## Current state

`read_before_edit_refusal` — the seam to mirror:

```rust
pub(super) fn read_before_edit_refusal(
    tool_call: &ToolCall,
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "patch" {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    match working_set.get(&path) {
        None => Some(format!("refusing to patch {}: …", path.display())),
        Some(recorded) => { /* mtime check */ }
    }
}
```

Wiring in `mod.rs` (~832):

```rust
match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
    Some(refusal) => (false, refusal, None),
    None => { /* baseline capture + dispatch */ }
}
```

`pre_edit_content` keys are produced by `edit_target` → `resolve_path` =
`project_root.join(path)` — **not** canonicalized. The git-command path argument
is relative to the repo root (`project_root`), so `project_root.join(arg)` matches
the same way. (A `./`-prefixed or absolute arg that doesn't join-match is a
conservative miss — acceptable; the wholesale forms are already blocked.)

## Spec

### Task 1 — Add `destructive_restore_refusal` (`executor/src/agent/tools.rs`)

A pure refusal mirroring `read_before_edit_refusal`, keyed off the
edited-this-session set:

```rust
/// Refuse a single-file `git checkout <path>` / `git restore <path>` of a file the
/// executor has edited this session — it would silently discard the model's own
/// uncommitted work. `None` = allowed. `bash`-only; the wholesale forms
/// (`git checkout .` / `git reset --hard` / …) are already blocked by
/// `security::bash_classify`. Pure over `edited` for unit-testability.
pub(super) fn destructive_restore_refusal(
    tool_call: &ToolCall,
    edited: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> Option<String> {
    if tool_call.name != "bash" {
        return None;
    }
    let command = tool_call.arguments.get("command").and_then(|v| v.as_str())?;
    for token in restore_path_tokens(command) {
        let resolved = project_root.join(token);
        if edited.contains_key(&resolved) {
            return Some(format!(
                "refusing to run `{command}`: it would discard your uncommitted edits to {} \
                 this session. Do not revert your own work — fix forward from the current \
                 state, and only commit if you need a checkpoint.",
                resolved.display()
            ));
        }
    }
    None
}

/// Path-like argument tokens of a `git checkout` / `git restore` sub-command, across
/// `&&` / `;` / `|`-joined segments. Conservative and NOT a shell parser: it returns
/// every non-flag token after a `checkout`/`restore` subcommand (skipping `-x` flags
/// and a `--` separator marker). Branch names like `main` are harmless — the caller
/// gates on membership in the edited set, which a branch name is never in.
fn restore_path_tokens(command: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for segment in command.split(['&', ';', '|']) {
        let mut toks = segment.split_whitespace();
        // advance to a `git` invocation
        let mut found_git = false;
        let mut in_restore = false;
        for tok in toks.by_ref() {
            if !found_git {
                if tok == "git" {
                    found_git = true;
                }
                continue;
            }
            if !in_restore {
                match tok {
                    "checkout" | "restore" => in_restore = true,
                    // any other subcommand in this segment: stop scanning it
                    _ => break,
                }
                continue;
            }
            // in_restore: collect non-flag, non-`--` tokens as candidate paths
            if tok == "--" || tok.starts_with('-') {
                continue;
            }
            out.push(tok);
        }
    }
    out
}
```

Imports: `restore_path_tokens` is private; `destructive_restore_refusal` uses
`std::collections::HashMap`, `std::path::{Path, PathBuf}`, `crate::…::ToolCall` —
all already imported in `tools.rs` (it already has `read_before_edit_refusal` with
the same imports).

### Task 2 — Wire it into the refusal seam (`mod.rs`, ~832)

Chain the new refusal **before** `read_before_edit_refusal` (they are disjoint —
bash vs patch — so order is cosmetic):

```rust
match destructive_restore_refusal(&tool_call, &pre_edit_content, deps.project_root)
    .or_else(|| read_before_edit_refusal(&tool_call, &working_set, deps.project_root))
{
    Some(refusal) => (false, refusal, None),
    None => { /* unchanged: baseline capture + dispatch */ }
}
```

The refused command short-circuits to a model-visible `(false, refusal, None)` —
no dispatch, the refusal text feeds back through the normal tool-exchange path,
exactly as the read-before-edit refusal does. It is **not** a hard-fail and does
not count as a governor strike.

### Task 3 — Unit tests (`tools.rs` `#[cfg(test)] mod tests`)

Build a `ToolCall` for `bash` with a given command and an `edited` map; assert:

- `refuses_checkout_of_edited_file` — `git checkout src/x.ts` with
  `project_root.join("src/x.ts")` in `edited` → `Some`, message names the file.
- `refuses_restore_of_edited_file` — `git restore src/x.ts` (edited) → `Some`.
- `refuses_checkout_head_dashdash_form` — `git checkout HEAD -- src/x.ts` (edited)
  → `Some` (the `HEAD` and `--` tokens are skipped; `src/x.ts` matches).
- `allows_checkout_of_unedited_file` — `git checkout src/y.ts` with only
  `src/x.ts` edited → `None`.
- `allows_branch_switch` — `git checkout -b feature` and `git checkout main` with
  some edited files (none named `feature`/`main`) → `None`.
- `ignores_non_bash_calls` — a `patch`/`read_file` `ToolCall` → `None`.
- `refuses_in_compound_command` — `npm test && git checkout src/x.ts` (edited) →
  `Some` (segment-split finds the restore).

### Task 4 — Integration test (`executor/src/agent/tests.rs`)

- `self_revert_of_edited_file_is_refused` — drive `execute_phase` so the model (1)
  edits a file (`write_file`/`patch` to put it in `pre_edit_content`), then (2)
  issues `bash` `git checkout <that file>`. Assert the bash call's tool result is
  the refusal (model-visible failure, run continues — not a hard_fail). Model the
  setup on an existing `tests.rs` test that already exercises a `write_file`
  followed by a `bash` call. If no close analogue exists, a focused unit-level
  assertion on `destructive_restore_refusal` (Task 3) plus a smaller integration
  check (the refusal text appears in the transcript) is acceptable — pin the
  behavior, not a specific harness shape.

**Pinned negatives** (the false-positive guards): `allows_checkout_of_unedited_file`,
`allows_branch_switch`, `ignores_non_bash_calls` — the refusal must fire **only**
for a restore of an edited file, never for branch operations or unedited files.

## Acceptance criteria

- [ ] All Task-3 unit tests pass (3 refusals, 3 allows, 1 compound).
- [ ] `self_revert_of_edited_file_is_refused` (or the equivalent pinned-behavior
      integration check) passes.
- [ ] A `git checkout -b <branch>` / `git checkout <branch>` is **not** refused.
- [ ] All pre-existing tests pass unmodified.
- [ ] `cargo fmt --all --check`, `cargo build`, `cargo clippy`, `cargo test` exit 0.

## Test plan

- Seven `destructive_restore_refusal` unit tests (above), hermetic over an
  in-memory `edited` map and a `TempDir` project root.
- One integration test asserting the refusal reaches the model and the run
  continues.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact. The refusal is a
library-internal seam; the integration test exercises it through `execute_phase`.
(The pre-dispatch refusal is observable in the session transcript as a failed
`bash` tool result, exercised by the integration test.)

## Authorizations

None. No new dependency, no `Cargo.toml`/`architecture.md` edit, no
`bash_classify.rs` edit (this phase is the loop-level, working-set-aware
complement to the existing stateless blocklist).

## Out of scope

- Extending `security::bash_classify` with new stateless patterns — the wholesale
  forms are already blocked there; the working-set-aware refusal belongs in the
  loop (it needs `pre_edit_content`, which the classifier cannot see).
- Blocking `git stash` / `git reset` (non-`--hard`) — those are not destructive
  to the working tree in the same way; scope to `checkout`/`restore`.
- A full shell parser — `restore_path_tokens` is deliberately conservative; the
  edited-set membership check is what prevents false positives.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
