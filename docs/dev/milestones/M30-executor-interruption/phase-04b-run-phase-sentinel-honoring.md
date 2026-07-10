# Phase 04b: blocking `run-phase` honors the `.rexymcp/stop` sentinel

**Milestone:** M30 — Executor Interruption
**Status:** review
**Depends on:** phase-04 (the sentinel helpers + watcher; `stop.rs`, `stop_watcher.rs`)
**Estimated diff:** ~110 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Close the one gap phase-04 deferred: the **blocking CLI `rexymcp run-phase`** does
not yet honor the `.rexymcp/stop` sentinel. Phase-04 wired the sentinel to the
async `execute_phase` (serve) path; the CLI `run-phase` arm still builds
`RunPhaseConfig { cancel: CancelSignal::never() }`, so a `rexymcp stop` from a
second terminal does nothing to a run started via `run-phase`.

This phase gives `run-phase` a **real** `CancelSignal` plus a single-handle
sentinel watcher (it has no `JobRegistry`, so it fires exactly its own run's
handle — not `request_stop_all`). After this, both entrypoints — the serve async
runs and the blocking CLI — respond to `rexymcp stop`, matching the
`docs/architecture.md` § Status #30 statement that "a serve-side watcher (**and
the still-blocking CLI `run-phase`**) honor" the sentinel. **No `architecture.md`
edit is required** — that behavior is already described there.

## Architecture references

- `docs/architecture.md` § Status #30 — "…a serve-side watcher (and the
  still-blocking CLI `run-phase`) honor" `.rexymcp/stop`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The serve watcher fires **all** runs via the registry (`mcp/src/stop_watcher.rs`, phase-04)

```rust
pub async fn watch_stop_sentinel(
    repo_path: PathBuf,
    registry: Arc<JobRegistry>,
    run_id: String,
    poll: Duration,
) {
    loop {
        tokio::time::sleep(poll).await;
        if !registry.is_running(&run_id) {
            return;
        }
        if stop::sentinel_present(&repo_path) {
            registry.request_stop_all(CancelReason::UserStop);
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}
```

`run-phase` has **no** `JobRegistry` and runs exactly one phase, so it needs a
sibling that fires a single `CancelHandle` (not the whole registry) and is stopped
by the caller when the run finishes.

### The `run-phase` CLI arm still uses a dead signal (`mcp/src/main.rs`, the `RunPhase` match arm)

```rust
        Commands::RunPhase {
            config,
            phase_doc,
            repo,
            model,
            no_telemetry,
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
                telemetry_dir: runner::resolve_telemetry_dir(&cfg, no_telemetry),
                progress: None,
                project_id,
                resume: None,
                test_client: None,
                cancel: rexymcp_executor::agent::CancelSignal::never(),
            })
            .await?;

            println!( /* … serialize result … */ );
            Ok(())
        }
```

### The cancel primitive (phase-01, `executor/src/agent/cancel.rs`)

```rust
pub struct CancelHandle { /* … */ }
impl CancelHandle { pub fn cancel(&self) { /* flips the signal */ } }

#[derive(Clone)]
pub struct CancelSignal { /* … */ }
impl CancelSignal {
    pub fn new() -> (CancelHandle, CancelSignal) { /* … */ }
    // …
}
```

Import path: `use rexymcp_executor::agent::{CancelHandle, CancelSignal};`.

## Spec

Additive: one new watcher function + a localized change to the one `run-phase`
arm. Nothing else is touched.

### 1. Single-handle sentinel watcher (`mcp/src/stop_watcher.rs`)

Add a sibling of `watch_stop_sentinel` that fires **one** `CancelHandle` (no
registry). Import `CancelHandle` at the top
(`use rexymcp_executor::agent::CancelHandle;`).

```rust
/// Sentinel watcher for the blocking CLI `run-phase`, which has no `JobRegistry`.
/// Polls `<repo>/.rexymcp/stop`; on first sight, fires this run's single
/// `cancel` handle (`UserStop` is stamped by the caller path) and clears the
/// sentinel, then exits. Unlike `watch_stop_sentinel` it has no run-liveness
/// check — the caller aborts this task once the run finishes.
pub async fn watch_stop_sentinel_single(repo_path: PathBuf, cancel: CancelHandle, poll: Duration) {
    loop {
        tokio::time::sleep(poll).await;
        if stop::sentinel_present(&repo_path) {
            cancel.cancel();
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}
```

(No `run_id`/registry needed. `CancelHandle` moves in — there is exactly one per
run.)

### 2. Wire it into the `run-phase` arm (`mcp/src/main.rs`)

Create a real signal, spawn the single-handle watcher, pass the **signal** into
`RunPhaseConfig`, and **abort the watcher** once the run returns (so a run that
finishes without a stop doesn't leave a task polling forever):

```rust
            let (cancel_handle, cancel_signal) = rexymcp_executor::agent::CancelSignal::new();
            let stop_watcher = tokio::spawn(stop_watcher::watch_stop_sentinel_single(
                repo.clone(),
                cancel_handle,
                stop_watcher::STOP_POLL_INTERVAL,
            ));

            let result = runner::run_phase(&runner::RunPhaseConfig {
                // … all existing fields unchanged …
                cancel: cancel_signal,
            })
            .await;

            stop_watcher.abort();
            let result = result?;

            println!( /* … existing serialize/print, unchanged … */ );
            Ok(())
```

Notes:
- Split the `.await?` into `.await` + `stop_watcher.abort()` + `?` so the watcher
  is always aborted even on the error path (don't `?` before aborting).
- `repo` is a `PathBuf` in scope; `repo.clone()` for the task.
- `STOP_POLL_INTERVAL` is the existing phase-04 constant (500 ms).

### 3. `CancelReason::UserStop` attribution (note — no code needed here)

`run-phase` returns the `PhaseResult` directly and does not go through the
registry's `spawn_run` reason-stamp, so its `cancelled` result's
`cancellation.reason` stays `None` (the executor loop leaves it `None`). Stamping
the CLI path's reason is **out of scope** for this phase — the async path is the
one the skill loop consumes. Do **not** add reason-stamping plumbing to
`run-phase` here; just make the cancel *fire*.

## Acceptance criteria

- [ ] All four gates green (`fmt`/`build`/`clippy`/`test`).
- [ ] `grep -n "CancelSignal::never()" mcp/src/main.rs` → **no output** (the
      `run-phase` arm now builds a real signal).
- [ ] `watch_stop_sentinel_single` fires its handle and clears the sentinel when
      the sentinel is present; does **not** fire when it is absent.
- [ ] No `#[allow(...)]`, no new dependency, no `dead_code` (the new fn is called
      from the `run-phase` arm).

## Test plan

Hermetic (`TempDir`, tiny injected `poll`, no wall-clock sleeps beyond it). In
`mcp/src/stop_watcher.rs` tests:

- `single_watcher_fires_handle_and_clears_when_sentinel_present` — `TempDir` repo,
  `let (h, s) = CancelSignal::new();` write the sentinel, spawn
  `watch_stop_sentinel_single(repo, h, 1ms)`, **await the JoinHandle with a
  timeout and assert it returned** (mirror the bug-04-1 fix — do not discard the
  outcome), then assert `s.is_cancelled()` and the sentinel is gone.
- `single_watcher_does_not_fire_without_sentinel` — no sentinel; spawn the
  watcher, let a few polls pass (e.g. `sleep(10ms)` on a 1ms poll), assert
  `!s.is_cancelled()`, then `watcher.abort()` (pin the negative: it never fires
  absent a sentinel).

**Make the "fires" test load-bearing** (bug-04-1 lesson): assert the task actually
completed (await its `JoinHandle` and `.expect()` it), not just that the signal
flipped.

## End-to-end verification

Not applicable as a fully-live check without a long-running phase, but state the
mechanism: after this phase, `cargo run -p rexymcp -- run-phase …` in one terminal
and `cargo run -p rexymcp -- stop --repo <same repo>` in another cancels the run
(the run returns a `cancelled` `PhaseResult`, working tree left dirty). If you can
drive a quick version headlessly, quote it; otherwise state that the hermetic
`watch_stop_sentinel_single` tests stand in for the live path and that the binary
must be rebuilt for a live `run-phase` to pick up the change.

## Authorizations

- [ ] May touch `docs/architecture.md`: **No** — § Status #30 already covers
      `run-phase` honoring; do not edit it.
- [ ] May add dependencies: **No.**
- [ ] May add `#[allow(...)]`: **No.**

## Out of scope

- **Reason-stamping the CLI `run-phase` `cancelled` result** — the async path owns
  that; here we only make the cancel fire (Task 3 note).
- **Run-scoped sentinel** (`--run <id>`) — still deferred.
- **The async-polling skill-loop rewrite** — phase-05.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-10 17:52 (started)

**Executor:** executor (phase-04b)

**Started:** Implementing single-handle sentinel watcher for `run-phase` and wiring it into the CLI arm.

### Update — 2026-07-10 17:52 (progress)

**Task 1:** Added `watch_stop_sentinel_single` to `stop_watcher.rs` with two hermetic tests.
**Task 2:** Wired the new watcher into the `run-phase` CLI arm in `main.rs`. Replaced `CancelSignal::never()` with a real `CancelSignal::new()` pair, spawned the watcher, and abort it after the run completes.
**Task 3:** `CancelReason::UserStop` attribution — no code needed per spec note.
### Update — ts=1783706303652 (complete, server-authored)

**Summary:** All gates pass and the working tree is clean.

**Summary:** Implemented phase 04b — the blocking `run-phase` CLI now honors the `.rexymcp/stop` sentinel. Added `watch_stop_sentinel_single` in `stop_watcher.rs` (a single-handle variant with no `JobRegistry` dependency) and wired it into the `RunPhase` CLI arm in `main.rs`. The old `CancelSignal::never()` dead signal is replaced with a real `CancelSignal::new()` pair; the watcher task is spawned before the run and aborted after the run completes (success or failure), preventing a leaked polling task. Two hermetic tests verify the watcher fires the handle when the sentinel is present and stays quiet when absent. `grep -n "CancelSignal::never()" mcp/src/main.rs` returns zero matches. All four verification gates (fmt, build, clippy, test) pass clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
alid_state_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 949 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.08s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M30-executor-interruption/README.md` — +1 -1
- `docs/dev/milestones/M30-executor-interruption/phase-04b-run-phase-sentinel-honoring.md` — +13 -1
- `mcp/src/main.rs` — +12 -2
- `mcp/src/stop_watcher.rs` — +71 -0

**Commit:** a035a5760bd88d7af1def8962d850b718e2f443b

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

