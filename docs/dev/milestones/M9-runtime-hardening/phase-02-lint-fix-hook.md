# Phase 02: lint-fix in the post-write hook

**Milestone:** M9 — Executor runtime hardening
**Status:** done
**Depends on:** M9/phase-01 (the post-write hook helper + call site exist and run
the `format` command). **phase-01 must be done before this phase starts.**
**Estimated diff:** ~120 lines (config field + helper extension + ~10 mechanical
test-literal updates + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Extend the post-write hook (phase-01) to also run an **autofixing linter** before
the formatter, so fixable lint diagnostics are resolved unconditionally each turn
rather than accumulating into a `VerifierFailurePersistent` hard-fail. This adds a
new optional `lint_fix` command to `CommandConfig` (the existing `lint` command is
a *checker* that gates, not a fixer) and runs it in the hook **before** `format`,
so the formatter always has the last word on the bytes that land on disk.

## Architecture references

Read before starting:

- WORKFLOW.md § "Post-write formatting is a runtime concern, not a spec concern" —
  names `lint --fix` as the optional second half of the runtime hook.
- WORKFLOW.md § "Prefer additive change shapes; avoid wide-blast-radius breaking
  changes" — **this phase's central risk.** Adding a field to `CommandConfig`
  breaks every exhaustive struct-literal that constructs it. The complete site
  list is pre-injected in § Spec task 3; follow the build-after-each-file
  discipline.
- M9/phase-01 — the `run_post_write_hooks` (or `run_format_hook`) helper and its
  call site you are extending.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above and phase-01's completion Update Log
   (to see the exact helper name/signature phase-01 shipped).
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes, and that
   phase-01 is `done`.
5. Read `executor/src/config.rs` in full (~120 lines, safe to read whole) and
   `executor/src/agent/contract.rs:19–37`.
6. **Do NOT read `executor/src/agent/mod.rs` whole** — it is ~4 000 lines (~165 KB)
   and will trip the `RunawayOutput` hard-fail detector. All excerpts you need are
   pre-injected in § "Current state" below. Do not issue a `read_file` on that path.

## Current state

`executor/src/agent/mod.rs` is too large to read whole. Work from the excerpts
below. If you need narrow surrounding context for a `patch` call, use `read_file`
with an explicit `offset`/`limit` — never the whole file.

### The existing hook — call site `mod.rs:668–685`, helper `mod.rs:1215–1219`

Phase-01 shipped these two pieces. Your job is to **extend** them (rename + add
the `lint_fix` step), not re-add them.

**Call site (between working-set recording and Step 6 verify):**

```rust
        // Post-write format hook (M9/phase-01).
        if succeeded && edit_path.is_some() && deps.commands.format.is_some() {
            {
                let emit = EmitCtx {
                    progress: deps.progress,
                    log_handle: &log_handle,
                    redactor: &redactor,
                    clock: deps.clock,
                    pre_edit_content: &pre_edit_content,
                    project_root: deps.project_root,
                    turn: turns,
                };
                emit_progress(&emit, "format".to_string());
            }
            run_format_hook(deps.runner, deps.commands, deps.project_root).await;
        }
```

**Helper (after `run_command_set`/`run_one`):**

```rust
async fn run_format_hook(runner: &dyn CommandRunner, commands: &CommandConfig, cwd: &Path) {
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}
```

After this phase: rename `run_format_hook` → `run_post_write_hooks`, add the
`lint_fix` step before `format`, and widen the call-site guard to
`commands.format.is_some() || commands.lint_fix.is_some()`.

### `EMPTY_COMMANDS` const and the 7 test literals in `mod.rs`

All are in `#[cfg(test)]`. Add `lint_fix: None` to each. Current form and
line numbers (verified 2026-06-04 post phase-01):

**`mod.rs:1424` — `const EMPTY_COMMANDS`** (must use explicit field, not
`..Default::default()` — `const` does not allow that):
```rust
    const EMPTY_COMMANDS: CommandConfig = CommandConfig {
        format: None,
        build: None,
        lint: None,
        test: None,
    };
```

**`mod.rs:2937`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };
```

**`mod.rs:2965`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("b".to_string()),
            lint: None,
            test: None,
        };
```

**`mod.rs:2993`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: None,
        };
```

**`mod.rs:3159`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };
```

**`mod.rs:3194`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: None,
        };
```

**`mod.rs:3741`:**
```rust
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };
```

**Workflow note:** the line numbers above are accurate as of the phase-01 commit.
Verify with `grep -n "CommandConfig {" executor/src/agent/mod.rs` before patching —
if they shifted, trust the grep, not these numbers.

## Spec

### 1. Add the `lint_fix` config field (additive)

In `executor/src/config.rs`, add a fifth field to `CommandConfig`:

```rust
pub struct CommandConfig {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
    pub lint_fix: Option<String>,
}
```

Match the sibling fields' style exactly (no per-field serde attribute — `Option`
defaults to `None` when the TOML key is absent). The `#[derive(... Default)]` makes
`CommandConfig::default()` set it to `None` automatically. This is the only
production source of the value.

### 2. Run `lint_fix` in the hook — before `format`

Extend the phase-01 helper so it runs **`lint_fix` first, then `format`**:

```rust
async fn run_post_write_hooks(runner: &dyn CommandRunner, commands: &CommandConfig, cwd: &Path) {
    if let Some(cmd) = commands.lint_fix.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}
```

**Order is load-bearing.** An autofixer can rewrite lines in a way that violates the
formatter (reflowed expressions, reordered imports), so `format` MUST run **after**
`lint_fix` to normalize the final on-disk bytes — the same bytes the final
`fmt --check` gate inspects. Running `format` before `lint_fix` could leave the
file unformatted, reintroducing the exact failure class phase-01 fixed.

Both steps stay **best-effort** (phase-01's error model): a non-zero exit or
spawn failure is discarded — never an `Err`, never a model-visible message, never a
hard-fail count. (If phase-01 named the helper `run_format_hook` and the call site
guards on `commands.format.is_some()` for its progress emit, generalize that guard
to `commands.format.is_some() || commands.lint_fix.is_some()` so a lint-fix-only
config still emits liveness, and rename the helper to `run_post_write_hooks`. The
single call site updates with it.)

### 3. Update every `CommandConfig` literal (multi-site — follow the order)

Adding the field breaks all 11 exhaustive literals below (10 test sites + the
struct def). Production has **none** (serde only). Add `lint_fix: None` to each
literal (the `const EMPTY_COMMANDS` must list it explicitly — `..Default::default()`
is not allowed in a `const`; the non-const test literals may use either form).
**Verify the list is still complete** before editing by running
`grep -rn "CommandConfig {" executor mcp` — trust the grep over this list if they
diverge, and flag the divergence in "Notes for review".

Grep-verified sites (2026-06-04), edit in this order, **`cargo build` after each
file** before moving to the next:

1. `executor/src/config.rs:94` — the struct definition (Spec task 1).
2. `executor/src/agent/contract.rs` — two literals (≈ lines 46, 67), both in
   `#[cfg(test)]`.
3. `executor/src/agent/prompt.rs` — two literals (≈ lines 32, 56), both in
   `#[cfg(test)]`.
4. `executor/src/agent/mod.rs` — **seven** literals (≈ lines 1399 `const
   EMPTY_COMMANDS`, 2912, 2940, 2968, 3134, 3169, 3716), all in `#[cfg(test)]`.

`mcp` has no `CommandConfig` literal (confirm with the grep). If the grep shows one,
stop and file a blocker — the list above would be stale.

### 4. Do NOT surface `lint_fix` to the model

`assemble_executor_contract` (`contract.rs`) substitutes the four existing command
placeholders into the contract the model reads. **Leave it unchanged.** `lint_fix`
is a silent runtime hook the *runtime* runs, not a command the *model* invokes;
adding a `{LINT_FIX_COMMAND}` placeholder would both expand scope and wrongly invite
the model to run it. The contract continues to advertise only `format`/`build`/
`lint`/`test`.

### 5. Error model

Unchanged from phase-01: best-effort, side-effect only, no `Err`/feedback/count, no
`.unwrap()`/`.expect()`/`panic!()`, no `tracing`, no new `SessionEvent` variant.

## Acceptance criteria

- [ ] `CommandConfig` has a `lint_fix: Option<String>` field that deserializes to
      `None` when the `[commands]` table omits it, and `CommandConfig::default()`
      sets it to `None`.
- [ ] After a successful edit-class turn, the hook runs `commands.lint_fix` (when
      configured) **before** `commands.format`, both in `project_root`.
- [ ] When `lint_fix` is `None`, the hook runs no lint-fix command (negative); the
      phase-01 format behavior is unchanged.
- [ ] A `lint_fix` command that exits non-zero does **not** halt the turn, append a
      message, or produce a `hard_fail` (negative).
- [ ] The contract template still substitutes exactly the four existing command
      placeholders — no `lint_fix` placeholder added (negative).
- [ ] `cargo build` is green after **each** file in the Spec task 3 order (no
      mid-cascade broken state).
- [ ] No new dependency; `governor/**` / `phase/**` unmodified; no new
      `SessionEvent`; no `tracing`.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Reuse phase-01's recording `CommandRunner` mock and loop scaffolding.

- `lint_fix_field_defaults_to_none_when_absent` — deserialize a `[commands]` table
  with only `format`; assert `lint_fix == None` (and the existing fields still
  load). A `config.rs` unit test.
- `hook_runs_lint_fix_before_format` — `lint_fix = Some("<fix>")`,
  `format = Some("<fmt>")`; one edit turn; assert the recorder saw `<fix>` **then**
  `<fmt>`, both in `project_root`, in that order.
- `hook_skips_lint_fix_when_unconfigured` (**negative**) — `lint_fix = None`,
  `format = Some(..)`; assert only the format command ran in the hook (phase-01
  behavior intact).
- `lint_fix_failure_does_not_halt_turn` — recorder returns `success: false` for the
  lint-fix command; assert the loop still reaches `PhaseStatus::Complete` with no
  extra model-visible message and no `hard_fail`.
- `contract_omits_lint_fix` (**negative**) — render the contract with a populated
  `lint_fix`; assert the rendered string does not contain the `lint_fix` value
  (it is never advertised to the model).

## End-to-end verification

Phase-01 shipped no `RealCommandRunner` artifact test (the attempt was blocked by
`executor` crate-visibility constraints — `executor/tests/` cannot reach the loop
internals). The `hook_runs_lint_fix_before_format` mock test is the ordering
artifact: it drives `execute_phase` through the production call path with a
`MockCommandRunner` recording invocations in order. That test IS the E2E check for
this phase — `MockCommandRunner` exercises the same `run_post_write_hooks →
deps.runner.run()` dispatch that `RealCommandRunner` would, just without shelling
out.

Paste the `cargo test hook_runs_lint_fix_before_format` output in the Update Log.

## Authorizations

- [x] **May modify** `executor/src/config.rs` (add the `lint_fix` field to
      `CommandConfig` **only** — no other config change).
- [x] **May modify** `executor/src/agent/mod.rs` (extend the hook helper + its call
      site + update the seven test literals), `executor/src/agent/contract.rs` and
      `executor/src/agent/prompt.rs` (update the test literals only — template logic
      unchanged).
- [ ] **Do NOT create `executor/tests/`** — the crate is a lib; external test files
      cannot reach the loop internals. All tests go in `mod.rs`'s `#[cfg(test)]`
      block (same as phase-01).
- [ ] **No new dependencies.**
- [ ] May **NOT** modify `executor/src/agent/command.rs`, `executor/src/governor/**`,
      `executor/src/phase/**`, the `mcp` crate, `Cargo.toml`, `rustfmt.toml`,
      `clippy.toml`, `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **Surfacing `lint_fix` to the model** (a contract placeholder) — Spec task 4
  forbids it.
- **A `FormatHook`/`LintFix` `SessionEvent`** or any session-log schema change.
- **Touching the final command set.** `run_command_set` continues to run only
  `format`/`build`/`lint`/`test` at completion; `lint_fix` is a hook-only command.
  (If a future phase wants the final set to gate on a clean lint-fix, that is its
  own decision.)
- **Choosing the user's `lint_fix` command.** The runtime runs whatever string is
  configured; the user is responsible for a non-interactive, dirty-tree-safe
  command (e.g. `cargo clippy --fix --allow-dirty --allow-staged --all-targets`).
  Do not hardcode or validate the command's shape.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-04 (started)

**Executor:** rexyMCP executor LLM
**Tasks:** Implement lint_fix field in CommandConfig, extend post-write hook, update all CommandConfig literals, add tests.

### Update — 2026-06-04 (escalation)

**Chosen lever:** refined re-dispatch
**Rationale:** Good progress (config field + 6/7 literals + contract/prompt done) then `IdenticalToolCallRepetition` on `read_file` of `mod.rs` — executor re-read the file repeatedly to find patch context for the 7th literal and the hook function. Spec gap: the pre-injected excerpts showed what the code looks like but didn't provide the exact `old_str`/`new_str` pairs needed for `patch`. Fixed by providing precise patch targets for all 3 remaining `mod.rs` edits.

### Notes for executor — 2026-06-04 (dispatch 2)

**Already done — do NOT redo:**
- `executor/src/config.rs` — `lint_fix: Option<String>` added ✅
- `executor/src/agent/contract.rs` — both test literals updated ✅
- `executor/src/agent/prompt.rs` — both test literals updated ✅
- `executor/src/agent/mod.rs` — 6 of 7 test literals updated ✅ (lines 1429, 2943,
  2972, 3001, 3168, 3204 now all have `lint_fix: None`)

**Remaining work — exactly 8 edits, described as exact patch targets below:**

**Do NOT call `read_file` on `executor/src/agent/mod.rs` for any reason.** Use
`patch` directly with the `old_str` values below. You may call `read_file` on
`executor/src/config.rs` and `executor/src/agent/contract.rs` (both are small and
safe to read whole).

---

#### Remaining edit 1 — 7th test literal in `mod.rs` (≈ line 3741)

`patch` on `executor/src/agent/mod.rs`:

```
old_str:
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
        };
        let budget = Budget::new(1_000_000);

new_str:
        let commands = CommandConfig {
            format: None,
            build: Some("cargo build".to_string()),
            lint: None,
            test: Some("cargo test".to_string()),
            lint_fix: None,
        };
        let budget = Budget::new(1_000_000);
```

Run `cargo build` after this patch before continuing.

---

#### Remaining edit 2 — hook helper: rename + add `lint_fix` step (≈ line 1215)

`patch` on `executor/src/agent/mod.rs`:

```
old_str:
async fn run_format_hook(runner: &dyn CommandRunner, commands: &CommandConfig, cwd: &Path) {
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}

new_str:
async fn run_post_write_hooks(runner: &dyn CommandRunner, commands: &CommandConfig, cwd: &Path) {
    if let Some(cmd) = commands.lint_fix.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
    if let Some(cmd) = commands.format.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
}
```

---

#### Remaining edit 3 — call site: widen guard + rename (≈ line 671)

`patch` on `executor/src/agent/mod.rs`:

```
old_str:
        if succeeded && edit_path.is_some() && deps.commands.format.is_some() {
            {
                let emit = EmitCtx {
                    progress: deps.progress,
                    log_handle: &log_handle,
                    redactor: &redactor,
                    clock: deps.clock,
                    pre_edit_content: &pre_edit_content,
                    project_root: deps.project_root,
                    turn: turns,
                };
                emit_progress(&emit, "format".to_string());
            }
            run_format_hook(deps.runner, deps.commands, deps.project_root).await;
        }

new_str:
        if succeeded
            && edit_path.is_some()
            && (deps.commands.format.is_some() || deps.commands.lint_fix.is_some())
        {
            {
                let emit = EmitCtx {
                    progress: deps.progress,
                    log_handle: &log_handle,
                    redactor: &redactor,
                    clock: deps.clock,
                    pre_edit_content: &pre_edit_content,
                    project_root: deps.project_root,
                    turn: turns,
                };
                emit_progress(&emit, "format".to_string());
            }
            run_post_write_hooks(deps.runner, deps.commands, deps.project_root).await;
        }
```

Run `cargo build` after edits 2 and 3 are both applied.

---

#### Remaining edits 4–6 — three loop tests in `mod.rs`

Add these three tests **before the final closing `}`** of the `#[cfg(test)] mod tests`
block. The current last test (`format_hook_runs_on_every_edit_turn`) ends with this
exact text — use it as `old_str`:

```
old_str:
        assert_eq!(
            count,
            3,
            "expected 3 format runs (2 hooks + 1 final command set), got {}: {:?}",
            count,
            runner.ran()
        );
    }
}

new_str:
        assert_eq!(
            count,
            3,
            "expected 3 format runs (2 hooks + 1 final command set), got {}: {:?}",
            count,
            runner.ran()
        );
    }

    #[tokio::test]
    async fn hook_runs_lint_fix_before_format() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "hello\n" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("ok");
        let commands = CommandConfig {
            lint_fix: Some("echo fix".into()),
            format: Some("echo fmt".into()),
            ..EMPTY_COMMANDS
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        let ran = runner.ran();
        // Hook fires lint_fix then format; final command set fires format again.
        // Assert the first two invocations are in order: fix before fmt.
        assert!(
            ran.len() >= 2,
            "expected at least 2 runner invocations, got: {:?}",
            ran
        );
        assert_eq!(
            ran[0], "echo fix",
            "lint_fix must run before format, got: {:?}",
            ran
        );
        assert_eq!(
            ran[1], "echo fmt",
            "format must run after lint_fix, got: {:?}",
            ran
        );
    }

    #[tokio::test]
    async fn hook_skips_lint_fix_when_unconfigured() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "hello\n" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("ok");
        let commands = CommandConfig {
            lint_fix: None,
            format: Some("echo fmt".into()),
            ..EMPTY_COMMANDS
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        let ran = runner.ran();
        assert!(
            !ran.iter().any(|c| c == "echo fix"),
            "lint_fix must not run when unconfigured, got: {:?}",
            ran
        );
        assert!(
            ran.iter().any(|c| c == "echo fmt"),
            "format must still run when lint_fix is None, got: {:?}",
            ran
        );
    }

    #[tokio::test]
    async fn lint_fix_failure_does_not_halt_turn() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("t.txt");
        let path = file.to_string_lossy().to_string();
        let client = MockAiClientScript::new(vec![
            vec![native(
                "write_file",
                json!({ "path": path, "content": "hello\n" }),
            )],
            vec![token("done")],
        ]);
        let verifier = MockFileVerifier::new(vec![]);
        let runner = MockCommandRunner::new("ok").failing("bad-fix");
        let commands = CommandConfig {
            lint_fix: Some("bad-fix".into()),
            format: Some("echo fmt".into()),
            ..EMPTY_COMMANDS
        };

        let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

        assert_eq!(result.status, PhaseStatus::Complete);
        assert!(
            result.briefing.is_none(),
            "expected no hard_fail on lint_fix failure"
        );
    }
}
```

---

#### Remaining edit 7 — `lint_fix_field_defaults_to_none_when_absent` in `config.rs`

Read `executor/src/config.rs` (safe to read whole — ~120 lines). Add this test to
the `#[cfg(test)] mod tests` block, before the final `}`:

```rust
    #[test]
    fn lint_fix_field_defaults_to_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("rexymcp.toml");
        std::fs::write(
            &path,
            r#"
[executor]
model = "m"
base_url = "http://localhost:1234/v1"

[commands]
format = "cargo fmt"
"#,
        )
        .unwrap();

        let cfg = Config::load(&path).unwrap();
        assert_eq!(
            cfg.commands.lint_fix, None,
            "lint_fix must default to None when absent from [commands]"
        );
        assert_eq!(cfg.commands.format.as_deref(), Some("cargo fmt"));
    }
```

---

#### Remaining edit 8 — `contract_omits_lint_fix` in `contract.rs`

Read `executor/src/agent/contract.rs` (safe to read whole — ~141 lines). Add this
test to the `#[cfg(test)] mod tests` block, before the final `}`:

```rust
    #[test]
    fn contract_omits_lint_fix() {
        let commands = CommandConfig {
            format: Some("cargo fmt".to_string()),
            build: Some("cargo build".to_string()),
            lint: Some("cargo clippy".to_string()),
            test: Some("cargo test".to_string()),
            lint_fix: Some("cargo clippy --fix".to_string()),
        };
        let output = assemble_executor_contract(&commands);
        assert!(
            !output.contains("cargo clippy --fix"),
            "lint_fix value must not appear in the assembled contract"
        );
    }
```

---

**After all 8 edits**, run `cargo fmt --all --check`, `cargo build`, `cargo clippy`,
`cargo test` and paste the output in the Update Log completion entry.

Run `cargo test hook_runs_lint_fix_before_format` to confirm the ordering test passes
and paste that output under "End-to-end verification."

### Update — 2026-06-04 (complete)

**Summary:** `lint_fix: Option<String>` added to `CommandConfig`; `run_format_hook`
renamed to `run_post_write_hooks` and extended to run `lint_fix` then `format`
(format last); call-site guard widened to fire on either command. All 7 `mod.rs`
test literals + `contract.rs`/`prompt.rs` literals carry `lint_fix: None`. 5 new
tests. (Executor flipped status but did not write this entry — added at review;
calibration noted in the verdict.)

**Commands (architect re-run at review):**
```
cargo fmt --all --check        → clean (exit 0)
cargo build                    → Finished, zero warnings
cargo clippy ... -D warnings   → Finished, zero warnings
cargo test                     → 579 passed, 0 failed, 2 ignored (5 new)
```

**End-to-end verification:** `hook_runs_lint_fix_before_format` drives `execute_phase`
through the production `run_post_write_hooks → deps.runner.run()` path with a
recording `MockCommandRunner`, asserting `ran[0]=="echo fix"` then `ran[1]=="echo fmt"`
— proving lint_fix runs before format. Passes.

### Review verdict — 2026-06-04

- **Verdict:** approved_after_1
- **Bounces:** 1 (no bug filed). Dispatch-1 hard_fail = `IdenticalToolCallRepetition`
  on `read_file` of `mod.rs` — the executor re-read the large file to find patch
  context. **Architect spec gap** (excerpts shown but no exact `old_str`/`new_str`
  patch targets); fixed by pre-injecting precise patch targets for all 3 remaining
  `mod.rs` edits. Dispatch-2 clean.
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none. Config field, hook rename+extension, guard, all 11
  literals, and 5 specced tests all present. Independently re-ran all four gates
  (fmt/build/clippy clean, 579 pass). Clean rename verified (no orphan
  `run_format_hook`). Contract template unchanged (4 placeholders; `contract_omits_lint_fix`
  confirms `lint_fix` is not surfaced to the model). Spot-checked
  `hook_runs_lint_fix_before_format` as discriminating (asserts fix-before-fmt order).
- **Calibration:** (1) **Large-file `read_file` → hard_fail recurred** — phase-01
  dispatch-1 (`RunawayOutput`) and phase-02 dispatch-1 (`IdenticalToolCallRepetition`)
  both traced to the executor reading `mod.rs` (~150–165 KB) whole. **Two occurrences
  = trend**; candidate WORKFLOW fold (pre-inject exact `old_str`/`new_str` patch
  targets for edits to large files; never instruct a whole-file read), pending user
  sign-off. The runtime-side `read_file` offset/limit + truncation fix is separately
  queued (M9/phase-03). (2) Executor flipped status to `review` without a completion
  Update Log entry — one occurrence, data only.
