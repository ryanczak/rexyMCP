# Phase 05: Post-write format hook — writing form

**Milestone:** M26 — Polish & Hardening
**Status:** todo
**Depends on:** none
**Estimated diff:** ~280 lines
**Tags:** language=rust, kind=bugfix, size=m

## Goal

The post-write format hook is a no-op today. After every successful edit-class
turn the loop calls `run_post_write_hooks`, which runs `commands.format` — but
`format` is the **verify-only** gate command (`cargo fmt --all --check`), so it
prints a diff and rewrites nothing. The file the executor just wrote reaches the
verifier (and, later, the reviewer's independent `--check`) still misformatted,
which is exactly the executor-vs-reviewer fmt divergence seen in M21 phase-01
(codebase review 2026-07-07 § "Known no-ops to close out").

The `lint` side already solved this: `commands.lint` is the check form (gate) and
`commands.lint_fix` is the writing form the hook runs. **This phase gives
`format` the same split**: add a writing `format_fix` field, run it in the hook
instead of the check-form `format`, and leave the `format` gate command
untouched so the Definition-of-Done gates still verify.

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` § "Known no-ops to close out" — the
  finding this phase closes.
- `docs/dev/STANDARDS.md` § 2.2 ("No fallbacks for if X is missing") — governs the
  unset-`format_fix` behavior (skip cleanly, do **not** fall back to the check form).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The hook body (`executor/src/agent/command.rs:235-246`)

```rust
pub(super) async fn run_post_write_hooks(
    runner: &dyn CommandRunner,
    commands: &CommandConfig,
    cwd: &Path,
) {
    if let Some(cmd) = commands.lint_fix.as_deref() {
        let _ = runner.run(cmd, cwd).await;
    }
    if let Some(cmd) = commands.format.as_deref() {   // ← runs the CHECK form; no-op
        let _ = runner.run(cmd, cwd).await;
    }
}
```

### The hook guard (`executor/src/agent/mod.rs:1149-1169`)

```rust
        // Post-write format hook (M9/phase-01). Runs the configured format
        // command after every successful edit-class turn, before the verifier,
        // so the on-disk file is always formatted when verify reads it.
        if succeeded
            && edit_path.is_some()
            && (deps.commands.format.is_some() || deps.commands.lint_fix.is_some())
        {
            {
                let emit = EmitCtx { /* … */ };
                emit_progress(&emit, "format".to_string());
            }
            run_post_write_hooks(deps.runner, deps.commands, deps.project_root).await;
        }
```

### The config type (`executor/src/config.rs:337-344`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CommandConfig {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
    pub lint_fix: Option<String>,
}
```

### The gate that MUST keep using `format` (`executor/src/agent/command.rs:77-80`)

```rust
    if commands.format.is_some() {
        // …
    }
    let (format, fmt_ok) = run_one(runner, commands.format.as_deref(), cwd).await;
```

This is the final command set (the DoD fmt gate). **Do not touch it.** It runs
the check form on purpose — the gate verifies, it does not rewrite.

## Spec

Numbered tasks in execution order.

1. **Add the `format_fix` field** — in `executor/src/config.rs`, add
   `pub format_fix: Option<String>,` to `CommandConfig` (after `lint_fix`). The
   struct already derives `Default` and `Deserialize`, so an absent
   `[commands] format_fix` deserializes to `None` — no `#[serde(default)]`
   attribute is needed on the field (the whole struct is `Default`, and serde
   fills absent `Option` fields with `None`). Then add a unit test
   `format_fix_field_defaults_to_none_when_absent`, a direct mirror of the
   existing `lint_fix_field_defaults_to_none_when_absent`
   (`executor/src/config.rs:858-882`) — load a TOML `[commands]` block with only
   `format` set and assert `cfg.commands.format_fix == None`.

2. **Run `format_fix` in the hook** — in `executor/src/agent/command.rs`,
   `run_post_write_hooks`, change the second block from `commands.format` to
   `commands.format_fix`:

   ```rust
       if let Some(cmd) = commands.format_fix.as_deref() {
           let _ = runner.run(cmd, cwd).await;
       }
   ```

   The `lint_fix` block above it is unchanged. **Do not** add a fallback to
   `commands.format` when `format_fix` is `None` — an unset writing command means
   "no auto-format," exactly as an unset `lint_fix` means "no auto-lint-fix"
   (STANDARDS § 2.2). Running the check form as a fallback would reintroduce the
   no-op this phase removes.

3. **Update the hook guard** — in `executor/src/agent/mod.rs:1152-1155`, change
   the guard condition from `deps.commands.format.is_some()` to
   `deps.commands.format_fix.is_some()`:

   ```rust
           if succeeded
               && edit_path.is_some()
               && (deps.commands.format_fix.is_some() || deps.commands.lint_fix.is_some())
   ```

   Rationale: the hook now has nothing to do unless a *writing* command
   (`format_fix` or `lint_fix`) is configured. A config with only the check-form
   `format` set must not fire the hook (it would run nothing useful). Leave the
   surrounding progress-event emission and the doc comment as they are.

4. **Add `format_fix` to `rexymcp doctor`** — in `mcp/src/doctor.rs:61-67`, add a
   sixth entry to the `tier0_commands` array, mirroring `lint_fix`:

   ```rust
       let tier0_commands = [
           ("format", commands.format.as_deref()),
           ("build", commands.build.as_deref()),
           ("lint", commands.lint.as_deref()),
           ("test", commands.test.as_deref()),
           ("lint_fix", commands.lint_fix.as_deref()),
           ("format_fix", commands.format_fix.as_deref()),
       ];
   ```

   Update the adjacent comment ("Walk the five configured commands…") to "six".
   Existing doctor tests set `format_fix: None`, so the walk `continue`s past it —
   no doctor test output changes.

5. **Document `format_fix` in the init template** — in `mcp/src/init.rs`, in the
   commented `[commands]` block (around line 51-56), add a commented line after
   `# lint_fix = …`:

   ```
   # format_fix = "cargo fmt --all"    # writing form; run by the post-write hook
   ```

6. **Close the E0063 cascade from the new field.** Adding a struct field breaks
   every `CommandConfig { … }` literal that enumerates all fields (E0063
   "missing field `format_fix`"). Literals that use a `..EMPTY_COMMANDS` /
   `..Default::default()` spread are **unaffected** — do not touch them. Run
   `cargo build` and let the compiler list every E0063 site; add
   `format_fix: None,` to each. The grep-verified complete list of full-literal
   sites (21, current line numbers) is:

   - `executor/src/config.rs` — the new field itself (Task 1).
   - `executor/src/agent/tests.rs:72` — the `EMPTY_COMMANDS` **const** (this is the
     load-bearing one: the spread sites inherit `format_fix` from it).
   - `executor/src/agent/tests.rs:116` — `all_commands_configured()`.
   - `executor/src/agent/tests.rs` — 1899, 1928, 1964, 2138, 2181, 2733.
   - `executor/src/agent/contract.rs` — 46, 68, 144.
   - `executor/src/agent/prompt.rs` — 151, 176.
   - `mcp/src/doctor.rs` — 232, 252, 282, 302, 320, 342, 361, 380.

   All get `format_fix: None,` except where a test deliberately sets it (Task 7).
   These are mechanical — the compiler is the source of truth; the list above is a
   completeness check. **Do not** change `format:` values at these sites; only add
   the missing field.

7. **Convert the behavior-affected hook tests.** Distinct from the E0063 sites:
   six hook tests in `executor/src/agent/tests.rs` set `format: Some(…)` and
   depend on the hook *running* it. Under the new semantics the hook runs
   `format_fix`, not `format`, so these must switch the field. Each uses
   `..EMPTY_COMMANDS`, so they have **no** E0063 error — they change for
   *behavior*, and three go **red** if not converted:

   **Must convert (go red otherwise):**

   - `format_hook_runs_before_verify` (~3205): `format: Some("echo fmt".into())` →
     `format_fix: Some("echo fmt".into())`. (Otherwise the guard is false, no
     `format` progress event fires, and `format_pos.is_some()` fails.)
   - `format_hook_failure_does_not_halt_turn` (~3351): `format: Some("fmt".into())`
     → `format_fix: Some("fmt".into())`. (Otherwise the hook runs nothing and the
     `ScriptedCommandRunner`'s leading `false` is consumed by the completion fmt
     **gate**, failing it and diverting into the M19 gate-retry loop instead of
     completing.)
   - `format_hook_runs_on_every_edit_turn` (~3381): `format: Some("echo fmt".into())`
     → `format_fix: Some("echo fmt".into())`, **and** change the asserted count
     from `3` to `2` and its message to reflect "2 hooks (the completion command
     set runs the check-form `format`, which is now unset)". With `format` unset,
     only the two hook invocations run `echo fmt`.

   **Convert to stay honest (green either way, but would otherwise be tested by the
   completion gate rather than the hook):**

   - `format_hook_runs_after_successful_edit` (~3176): `format:` → `format_fix:`.
   - `hook_runs_lint_fix_before_format` (~3419): `format: Some("echo fmt".into())` →
     `format_fix: Some("echo fmt".into())`; rename the test to
     `hook_runs_lint_fix_before_format_fix`. The `ran[0] == "echo fix"` /
     `ran[1] == "echo fmt"` ordering assertions still hold (hook runs `lint_fix`
     then `format_fix`).
   - `hook_skips_lint_fix_when_unconfigured` (~3462): `format: Some("echo fmt".into())`
     → `format_fix: Some("echo fmt".into())` (keep `lint_fix: None`). The
     assertions (no `echo fix`; `echo fmt` present) still hold — now the `echo fmt`
     comes from the hook's `format_fix` rather than the completion gate.

   **Leave unchanged** (they assert the hook is *skipped* / count the completion
   gate's own `format` run, which is untouched): `format_hook_skipped_after_non_edit_call`,
   `format_hook_skipped_after_failed_edit`, `format_hook_skipped_when_no_format_configured`,
   `lint_fix_failure_does_not_halt_turn`. Verify each still passes; do not edit them.

8. **Add the crux negative-pin test** `hook_runs_format_fix_not_the_check_form` in
   `executor/src/agent/tests.rs`. Set **both** commands so the two are
   distinguishable:

   ```rust
   let commands = CommandConfig {
       format: Some("echo CHECK".into()),
       format_fix: Some("echo FIX".into()),
       ..EMPTY_COMMANDS
   };
   ```

   Script a single `write_file` create then `done`, run via `run_full` with a
   `MockCommandRunner`. Assert: the hook ran `echo FIX` (`ran()[0] == "echo FIX"`),
   and `echo CHECK` appears only **after** it (from the completion command set,
   not the hook) — e.g. assert the first `echo CHECK` index is greater than the
   `echo FIX` index. This pins that the hook uses the writing form, never the
   check form.

9. **Add the end-to-end real-artifact test**
   `format_fix_hook_rewrites_file_on_disk` in `executor/src/agent/tests.rs`, using
   the production `RealCommandRunner` (not a mock) so a real subprocess actually
   rewrites the file:

   ```rust
   #[tokio::test]
   async fn format_fix_hook_rewrites_file_on_disk() {
       let dir = TempDir::new().unwrap();
       let file = dir.path().join("f.txt");
       let path = file.to_string_lossy().to_string();
       // write_file CREATES f.txt (create is ungated by the read-before-edit gate);
       // the format_fix hook then overwrites it via a real subprocess.
       let client = MockAiClientScript::new(vec![
           vec![native("write_file", json!({ "path": path, "content": "unformatted\n" }))],
           vec![token("done")],
       ]);
       let verifier = MockFileVerifier::new(vec![]);
       let runner = RealCommandRunner;
       let commands = CommandConfig {
           format_fix: Some("printf 'formatted\\n' > f.txt".into()),
           ..EMPTY_COMMANDS
       };

       let result = run_full(&dir, &client, &verifier, &runner, &commands, None, 8).await;

       assert_eq!(result.status, PhaseStatus::Complete);
       let on_disk = std::fs::read_to_string(&file).unwrap();
       assert_eq!(
           on_disk, "formatted\n",
           "post-write hook's format_fix must rewrite the file on disk, got: {on_disk:?}"
       );
   }
   ```

   `RealCommandRunner` is `pub` in `executor::agent::command`; import it in the
   test module the same way `MockCommandRunner`/`ScriptedCommandRunner` are
   available. The `format_fix` command runs with `cwd` = the TempDir (the hook
   passes `deps.project_root`), so the bare `f.txt` path resolves inside the
   sandbox — hermetic, no host writes. **Create** (not overwrite) is used
   deliberately so the phase-04 read-before-edit gate does not refuse the
   `write_file`.

## Acceptance criteria

- [ ] `CommandConfig` has a `format_fix: Option<String>` field; an absent
      `[commands] format_fix` loads as `None`.
- [ ] `run_post_write_hooks` runs `commands.format_fix` (writing form), not
      `commands.format` (check form); the final command set still runs
      `commands.format` as the fmt gate.
- [ ] The hook guard fires only when `format_fix` or `lint_fix` is set.
- [ ] `rexymcp doctor` lists a `format_fix` tier-0 entry when configured.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes (existing + new).
- [ ] Test `format_fix_field_defaults_to_none_when_absent` passes.
- [ ] Test `hook_runs_format_fix_not_the_check_form` passes.
- [ ] Test `format_fix_hook_rewrites_file_on_disk` passes.

## Test plan

- `format_fix_field_defaults_to_none_when_absent` in `executor/src/config.rs` —
  a `[commands]` TOML with only `format` set loads `format_fix == None`.
- `hook_runs_format_fix_not_the_check_form` in `executor/src/agent/tests.rs` —
  with both `format` and `format_fix` set, the hook invokes `format_fix`; the
  check-form `format` appears only from the completion command set, after it.
- `format_fix_hook_rewrites_file_on_disk` in `executor/src/agent/tests.rs`
  (E2E, `RealCommandRunner`) — a real subprocess `format_fix` rewrites the
  just-created file; on-disk content changes.
- Converted hook tests (Task 7) continue to assert the same hook behavior against
  `format_fix`.

## End-to-end verification

`format_fix_hook_rewrites_file_on_disk` is the end-to-end check: it drives the
real loop with the production `RealCommandRunner`, so an actual `sh -c` subprocess
rewrites the file the executor wrote, and the assertion reads the mutated bytes
back off disk. Quote its output (and a `cargo test format_fix` run) in the
completion Update Log. The config half is additionally verified against the real
`Config::load` in `format_fix_field_defaults_to_none_when_absent`.

## Authorizations

None. (No new dependency; no `Cargo.toml`, `architecture.md`, `STANDARDS.md`, or
`WORKFLOW.md` edit. Session-event/telemetry schema unchanged.)

## Out of scope

- **Scoping the format command to only the touched file.** The hook runs the whole
  `format_fix` command (e.g. `cargo fmt --all`), exactly as it runs the whole
  `lint_fix` command today. "Rewrites the touched file" describes the *effect*, not
  a requirement to pass a path to the formatter. Do not build per-file scoping.
- **Changing the `format` gate command** (`command.rs:77-80`, the final command
  set). It stays the check form; the DoD fmt gate verifies, it does not rewrite.
- **Removing or renaming `format`.** Both fields coexist: `format` = check (gate),
  `format_fix` = write (hook), mirroring `lint`/`lint_fix`.
- **Touching the `lint_fix` block or the progress-event emission** in the hook.
- **The other M26 items** (budget knobs, governor detectors, tsc resolution) —
  later phases.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
