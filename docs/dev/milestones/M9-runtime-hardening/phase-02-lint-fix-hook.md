# Phase 02: lint-fix in the post-write hook

**Milestone:** M9 — Executor runtime hardening
**Status:** todo
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
5. Read these surfaces:
   - `executor/src/config.rs:93–99` — `CommandConfig { format, build, lint, test }`,
     each `Option<String>`, `#[derive(... Default)]`. Note the sibling fields carry
     **no** per-field `#[serde(default)]`: serde already treats an `Option` field
     as defaulting to `None` when absent, which is why partial `[commands]` tables
     load today. The new field inherits that behavior.
   - The phase-01 hook helper and its call site in `executor/src/agent/mod.rs`.
   - `executor/src/agent/contract.rs:19–37` — the contract template substitutes
     `{FORMAT,BUILD,LINT,TEST}_COMMAND`. **Do not add a `lint_fix` placeholder** —
     see § Spec task 4 (the hook is a silent runtime step; the model never runs it).

## Current state

After phase-01, the hook runs `commands.format` after every successful edit-class
turn, before the verifier. There is **no** autofixing-lint step and **no**
`lint_fix` config field. The verifier (`cargo check`/`tsc`/`ruff check`) surfaces
lint/type diagnostics as author feedback, and three consecutive author-diagnostic
turns trip `HardFailSignal::VerifierFailurePersistent` — even when the diagnostics
were mechanically autofixable. `CommandConfig` is constructed in production **only**
via serde (`Config` deserialization); every Rust `CommandConfig { … }` literal is
in a `#[cfg(test)]` module (the grep list in Spec task 3).

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

Extend phase-01's real-`RealCommandRunner` artifact test (or add a sibling): drive a
single `write_file` turn with `RealCommandRunner`, a real LLM-free `lint_fix`
command and a real `format` command over a `TempDir`, where `lint_fix` writes an
intermediate marker and `format` overwrites with the final formatted form. After the
turn, assert the **on-disk content** equals the `format` output (proving `format`
ran last) and that the `lint_fix` side effect occurred (proving both ran, in order).
Paste the actual output in the completion Update Log.

If a deterministic real-command ordering test proves infeasible, fall back to the
`hook_runs_lint_fix_before_format` mock test as the artifact check and state why.

## Authorizations

- [x] **May modify** `executor/src/config.rs` (add the `lint_fix` field to
      `CommandConfig` **only** — no other config change).
- [x] **May modify** `executor/src/agent/mod.rs` (extend the hook helper + its call
      site + update the seven test literals), `executor/src/agent/contract.rs` and
      `executor/src/agent/prompt.rs` (update the test literals only — template logic
      unchanged).
- [x] **May modify** `executor/tests/` (extend/add the E2E artifact test).
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
