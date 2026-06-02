# Bug 1 on phase-03a: `#[allow(clippy::too_many_arguments)]` silences a diagnostic instead of following the project's grouping idiom

**Severity:** minor
**Status:** verified
**Filed:** 2026-06-01
**Verified:** 2026-06-02 — `#[allow]` removed, replaced by `RunFullArgs` struct
(`mod.rs:2597`); clippy clean with no shim; all tests green.

## What's wrong

The phase added a `#[allow(clippy::too_many_arguments)]` to the `run_full` test
helper to absorb the new `bench_suite` argument (the helper went from 7 to 8
params, over clippy's default threshold):

```rust
// executor/src/agent/mod.rs:2595
    #[allow(clippy::too_many_arguments)]
    /// Full run with injectable command runner + command config + telemetry dir.
    async fn run_full(
        dir: &TempDir,
        client: &dyn AiClient,
        verifier: &dyn FileVerifier,
        runner: &dyn CommandRunner,
        commands: &CommandConfig,
        telemetry_dir: Option<&Path>,
        bench_suite: Option<&str>,
        max_turns: usize,
    ) -> PhaseResult {
```

This is a **prohibited lint-silencing shim**. `STANDARDS.md` §1 DoD: *"No
`#[allow(...)]`, `#[ignore]`, or lint-silencing shims to mask diagnostics."*
`CLAUDE.md` § Hard rules lists *"add `#[allow]`/`#[ignore]` to mask a
diagnostic"* as a stop-and-file-a-blocker trigger — it requires explicit
phase-doc authorization, which phase-03a does **not** grant (its Authorizations
section authorizes no `#[allow]`).

The `#[allow]` is load-bearing: `run_full` now has 8 parameters, and removing the
attribute makes `cargo clippy --all-targets --all-features -- -D warnings` fail
with `clippy::too_many_arguments`. So it is genuinely masking a live diagnostic,
not a no-op.

## What should happen

The project already has a **documented idiom** for exactly this lint: group the
parameters into a struct rather than silencing the warning. See
`mcp/src/runner.rs:200`:

```rust
/// Configuration parameters for `run_phase`, grouped to stay under
/// clippy's argument limit (same pattern as `AssemblyInput` / `Seams`).
pub struct RunPhaseConfig<'a> { … }
```

The same crate groups `Seams` and `AssemblyInput` for the same reason. A test
helper is not exempt from the standard — the helper should follow the grouping
idiom, not reach for `#[allow]`.

## How to fix

In `executor/src/agent/mod.rs`:

1. Remove the `#[allow(clippy::too_many_arguments)]` at line 2595.
2. Introduce a small params struct for `run_full` (e.g. `RunFullArgs<'a>` holding
   the references + `telemetry_dir` + `bench_suite` + `max_turns`), mirroring the
   `RunPhaseConfig` / `Seams` grouping pattern in `mcp/src/runner.rs`. `dir`,
   `client`, `verifier`, `runner` can stay as direct params if that keeps it
   under 7, or fold them in too — the only requirement is **no `#[allow]`** and
   the helper reads cleanly. Pick whichever grouping lands clippy-clean with the
   fewest moving parts.
3. Update the `run_full(...)` call sites accordingly (all in the same
   `#[cfg(test)] mod tests` block).

Do **not** raise clippy's `too-many-arguments-threshold` in `clippy.toml`
(that file is off-limits per `STANDARDS.md` §5), and do **not** re-add the
`#[allow]` in any form.

The behavior under test (the `bench_suite` stamp) is correct and already covered
by `emit_stamps_bench_suite_when_set` / `emit_leaves_bench_suite_none_for_production`;
this bug is purely about *how* the new argument was absorbed. Leave those tests'
assertions intact.

## Verification

- [ ] `grep -rn '#\[allow' executor/src/agent/mod.rs` returns nothing for
      `too_many_arguments` (no lint-silencing shim remains).
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes with no
      `#[allow]` masking it.
- [ ] `cargo test` passes — `emit_stamps_bench_suite_when_set` and
      `emit_leaves_bench_suite_none_for_production` still green.
- [ ] `cargo fmt --all --check` clean.
