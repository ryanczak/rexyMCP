# Phase 02: `run-phase` telemetry parity

**Milestone:** M26 — Polish & Hardening
**Status:** done
**Depends on:** none
**Estimated diff:** ~90 lines
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`rexymcp run-phase` (the CLI twin of the `execute_phase` MCP tool) currently
hardcodes `telemetry_dir: None`, so every phase run via the CLI silently skips
writing a `PhaseRun` record — even when `[telemetry] dir` is configured in
`rexymcp.toml`. This contradicts the project's premise that every phase run is
telemetered (`docs/architecture.md` § "Model effectiveness metrics & the
scorecard": the eval dataset is "a byproduct of normal use"). After this phase,
`run-phase` telemetes by default, matching `execute_phase`, with an explicit
`--no-telemetry` opt-out for the cases where you deliberately don't want a run
recorded (e.g. a throwaway smoke test).

## Architecture references

Read before starting:

- `docs/dev/codebase-review-2026-07-07.md` §1 item 4 — the finding this phase fixes.
- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — why
  every run should be telemetered by default.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**The bug**, `mcp/src/main.rs:310-336` (the `Commands::RunPhase` match arm):

```rust
        Commands::RunPhase {
            config,
            phase_doc,
            repo,
            model,
        } => {
            let cfg = Config::load_with_env(&config)?;

            let standards_path = repo.join("docs/dev/STANDARDS.md");
            let standards = std::fs::read_to_string(&standards_path).unwrap_or_default();

            let project_id = rexymcp_executor::config::Config::load(&repo.join("rexymcp.toml"))
                .ok()
                .and_then(|c| c.project.id);

            let result = runner::run_phase(&runner::RunPhaseConfig {
                cfg: &cfg,
                phase_doc_path: &phase_doc,
                repo_path: &repo,
                standards: &standards,
                model_override: model.as_deref(),
                telemetry_dir: None,
                progress: None,
                project_id,
                test_client: None,
            })
            .await?;
```

Note `telemetry_dir: None` — hardcoded, ignoring `cfg.telemetry.dir` entirely.

**The correct pattern already lives in the MCP path**,
`mcp/src/server.rs:76-118` (`execute_phase_inner_with_client`):

```rust
    let telemetry_dir = cfg.telemetry.dir.as_deref();

    let project_id = rexymcp_executor::config::Config::load(&repo_path.join("rexymcp.toml"))
        .ok()
        .and_then(|c| c.project.id);

    let result = runner::run_phase(&runner::RunPhaseConfig {
        cfg: &cfg,
        phase_doc_path: &phase_doc_path,
        repo_path: &repo_path,
        standards: &standards,
        model_override: params.model.as_deref(),
        telemetry_dir,
        progress,
        project_id,
        test_client,
    })
    .await
    .map_err(|e| e.to_string())?;
```

`RunPhaseConfig.telemetry_dir` (`mcp/src/runner.rs:254`) is `Option<&'a Path>` —
`None` disables telemetry (`agent::execute_phase` skips `emit_phase_run` when
it's `None`; you do not need to change anything downstream of `LoopDeps`, only
what `mcp/src/main.rs` passes in).

**The clap `RunPhase` variant**, `mcp/src/main.rs:75-91`:

```rust
    /// Execute a phase against a target repository
    RunPhase {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,

        /// Path to the phase-doc markdown file
        #[arg(long)]
        phase_doc: PathBuf,

        /// Path to the target repository root
        #[arg(long)]
        repo: PathBuf,

        /// Override the model ID from config
        #[arg(long)]
        model: Option<String>,
    },
```

**The existing boolean-flag convention** elsewhere in the same `Commands` enum
(`mcp/src/main.rs:109-111`, the `Status` variant's `json` field — a bare
`#[arg(long)] json: bool` with no `default_value_t`; clap defaults an
undecorated `bool` field to `false` when the flag is absent):

```rust
        /// Emit the status as JSON instead of a human summary
        #[arg(long)]
        json: bool,
```

**`runner.rs` already has a section of small, pure, directly-unit-tested
helper functions** near the top of the file — `milestone_id_from_path`
(`mcp/src/runner.rs:118-131`) and `derive_phase_id` (`mcp/src/runner.rs:77-92`,
tested at `mcp/src/runner.rs:415-425` in this file's own `#[cfg(test)] mod
tests`). This phase adds a third helper in the same style, so the
default-vs-opt-out logic is unit-testable without spinning up the async
executor loop.

**`Config`'s telemetry field**, `executor/src/config.rs:241-243`:

```rust
pub struct TelemetryConfig {
    pub dir: Option<PathBuf>,
}
```

## Spec

1. **Add a pure `resolve_telemetry_dir` helper to `mcp/src/runner.rs`** —
   place it near `milestone_id_from_path` (both are small path/config helpers
   used by the CLI wiring). Signature and behavior:

   ```rust
   /// Resolve the telemetry directory for a CLI-driven `run-phase` invocation:
   /// `--no-telemetry` forces telemetry off regardless of config; otherwise
   /// defer to `cfg.telemetry.dir`, matching the MCP `execute_phase` path
   /// (`server.rs::execute_phase_inner_with_client`), which always telemeters
   /// when `[telemetry] dir` is set.
   pub fn resolve_telemetry_dir(cfg: &Config, no_telemetry: bool) -> Option<&Path> {
       if no_telemetry {
           None
       } else {
           cfg.telemetry.dir.as_deref()
       }
   }
   ```

   `Config` and `Path` are already imported at the top of `runner.rs` (lines 1
   and 10) — no new `use` needed.

2. **Add the `--no-telemetry` flag to the `RunPhase` clap variant** — in
   `mcp/src/main.rs`, add a field to the `RunPhase` variant (`main.rs:75-91`)
   after `model`, following the existing `json: bool` convention quoted above
   exactly (bare `#[arg(long)] <name>: bool`, no `default_value_t`):

   ```rust
        /// Skip writing a PhaseRun telemetry record for this run, even if
        /// [telemetry] dir is configured
        #[arg(long)]
        no_telemetry: bool,
   ```

3. **Wire it into the `Commands::RunPhase` match arm** — in `mcp/src/main.rs`
   (the block quoted in full in § Current state, `main.rs:310-336`):
   - add `no_telemetry` to the destructured pattern alongside `config`,
     `phase_doc`, `repo`, `model`;
   - replace the hardcoded `telemetry_dir: None,` line with
     `telemetry_dir: runner::resolve_telemetry_dir(&cfg, no_telemetry),`.

   Nothing else in the block changes — `standards`, `project_id`, and the rest
   of `RunPhaseConfig`'s fields are untouched.

4. **Unit-test `resolve_telemetry_dir` in `runner.rs`'s existing
   `#[cfg(test)] mod tests`** — two cases: flag absent defers to config
   (both Some and None config states), flag present always forces `None`
   regardless of config. Use `Config::default()` (already used elsewhere in
   this test module) and set `telemetry.dir` directly.

5. **Update the two existing `run-phase` CLI-parsing tests in `mcp/src/main.rs`**
   (`cli_parse_run_phase_with_all_args` and `cli_parse_run_phase_model_optional`,
   `main.rs:606-661`) to destructure the new `no_telemetry` field — both are
   `match` arms with `Commands::RunPhase { config, phase_doc, repo, model }` /
   `Commands::RunPhase { model, .. }`; the first needs `no_telemetry` added to
   the destructure and asserted `false` (the flag is absent in both existing
   test invocations), the second can keep `..` and needs no change beyond
   compiling against the new field. Add a third new test asserting the flag
   parses to `true` when `--no-telemetry` is passed.

## Acceptance criteria

- [ ] `rexymcp run-phase --help` output includes a `--no-telemetry` flag with
      the docstring from task 2.
- [ ] `cargo build` succeeds with the new field wired through.
- [ ] Test `resolve_telemetry_dir_defers_to_config_when_flag_absent` passes.
- [ ] Test `resolve_telemetry_dir_forces_none_when_flag_present` passes.
- [ ] Test `cli_parse_run_phase_no_telemetry_flag_sets_true` passes.
- [ ] `cli_parse_run_phase_with_all_args` still passes after adding
      `no_telemetry` to its destructure and asserting it `false`.
- [ ] All four gates green.

## Test plan

- `resolve_telemetry_dir_defers_to_config_when_flag_absent` in `mcp/src/runner.rs`
  — asserts `resolve_telemetry_dir(&cfg, false)` returns `Some(path)` when
  `cfg.telemetry.dir` is `Some(path)`, and `None` when it's `None`.
- `resolve_telemetry_dir_forces_none_when_flag_present` in `mcp/src/runner.rs`
  — asserts `resolve_telemetry_dir(&cfg, true)` returns `None` even when
  `cfg.telemetry.dir` is `Some(path)`.
- `cli_parse_run_phase_no_telemetry_flag_sets_true` in `mcp/src/main.rs` —
  parses `["rexymcp", "run-phase", "--config", "rexymcp.toml", "--phase-doc",
  "phase-doc.md", "--repo", "/tmp/repo", "--no-telemetry"]` and asserts
  `no_telemetry == true`.
- `cli_parse_run_phase_with_all_args` (existing, modified) — asserts
  `no_telemetry == false` when the flag is absent from the argv it already
  constructs.

## End-to-end verification

The real artifact is the CLI binary's argument parsing and help text. Run and
paste output:

```
cargo run -p rexymcp -- run-phase --help
```

confirming `--no-telemetry` appears in the listed options with its docstring.

This phase does not require a live-LLM run to verify the telemetry-writing
behavior itself — that behavior (whether `agent::execute_phase` calls
`emit_phase_run` when `telemetry_dir` is `Some`/`None`) is unchanged code,
already covered by existing `executor` crate tests; this phase only fixes
what `run-phase` passes into that existing, already-tested parameter. The unit
tests in task 4 are the direct verification that the CLI now passes the right
value.

## Authorizations

None from STANDARDS.md §5. This phase authorizes edits to exactly
`mcp/src/main.rs` and `mcp/src/runner.rs`.

## Out of scope

- Any change to `mcp/src/server.rs` / `execute_phase` — its telemetry wiring
  (`cfg.telemetry.dir.as_deref()`, always on) is already correct and is not
  touched.
- Any change to `agent::execute_phase`, `emit_phase_run`, or any other
  `executor` crate code — the telemetry-writing behavior itself is unchanged;
  only the value `run-phase` passes in changes.
- Adding a `--no-telemetry` (or any) flag to other CLI subcommands.
- Changing the default of `[telemetry] dir` itself, or `rexymcp init`'s
  generated template.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-07 20:45 (started)

**Executor:** rexyMCP executor LLM

Implementing `--no-telemetry` flag for `run-phase` CLI and wiring `resolve_telemetry_dir` helper to restore telemetry parity with the MCP `execute_phase` path.

### Update — 2026-07-07 20:45 (complete)

**Summary:** Added `--no-telemetry` flag to `run-phase` CLI and a pure `resolve_telemetry_dir` helper in `runner.rs`. The `Commands::RunPhase` match arm now calls `runner::resolve_telemetry_dir(&cfg, no_telemetry)` instead of hardcoding `telemetry_dir: None`, matching the MCP `execute_phase` path which always telemeters when `[telemetry] dir` is configured. No deviations from the spec.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
<exit 0, no output>

cargo build 2>&1 | tail -20
<exit 0, 0 warnings>

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
<exit 0, 0 warnings>

cargo test -p rexymcp 2>&1 | tail -5
test result: ok. 432 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:**

```
$ cargo run -p rexymcp -- run-phase --help
...
      --no-telemetry           Skip writing a PhaseRun telemetry record for this run, even if [telemetry] dir is configured
...
```
`--no-telemetry` appears in the listed options with its docstring.

**Literal grep (spec-pinned):**
```
$ grep -rn 'no_telemetry' mcp/src/main.rs mcp/src/runner.rs
mcp/src/main.rs:95:        no_telemetry: bool,
mcp/src/main.rs:320:            no_telemetry,
mcp/src/main.rs:337:                telemetry_dir: runner::resolve_telemetry_dir(&cfg, no_telemetry),
mcp/src/main.rs:634:                no_telemetry,
mcp/src/main.rs:643:                assert!(!no_telemetry);
mcp/src/main.rs:678:    fn cli_parse_run_phase_no_telemetry_flag_sets_true() {
mcp/src/main.rs:693:            Some(Commands::RunPhase { no_telemetry, .. }) => {
mcp/src/main.rs:694:                assert!(no_telemetry);
mcp/src/runner.rs:120:pub fn resolve_telemetry_dir(cfg: &Config, no_telemetry: bool) -> Option<&Path> {
mcp/src/runner.rs:121:    if no_telemetry {
```

**Files changed:**
- `mcp/src/runner.rs` — Added `resolve_telemetry_dir` helper and two unit tests
- `mcp/src/main.rs` — Added `--no-telemetry` flag to `RunPhase` variant, wired into match arm, updated existing CLI-parsing tests, added new `cli_parse_run_phase_no_telemetry_flag_sets_true` test
- `docs/dev/milestones/M26-polish-and-hardening/phase-02-run-phase-telemetry-parity.md` — Status `todo` → `review`, Update Log entries
- `docs/dev/milestones/M26-polish-and-hardening/README.md` — Phase 02 row status `todo` → `review`

**New tests:**
- `resolve_telemetry_dir_defers_to_config_when_flag_absent` in `mcp/src/runner.rs`
- `resolve_telemetry_dir_forces_none_when_flag_present` in `mcp/src/runner.rs`
- `cli_parse_run_phase_no_telemetry_flag_sets_true` in `mcp/src/main.rs`

**Commits:**
- `8fc41ce` — fix: wire telemetry dir through run-phase CLI (--no-telemetry opt-out)

**Notes for review:** None — implementation matches the spec exactly.

### Review verdict — 2026-07-07

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8 (Update Log self-stamped "rexyMCP executor LLM" — telemetry and `rexymcp.toml` confirm the real executor)
- **Scope deviations:** none — diff matched the pre-injected spec snippets essentially verbatim, touched exactly `mcp/src/main.rs` and `mcp/src/runner.rs` as authorized
- **Calibration:** none
