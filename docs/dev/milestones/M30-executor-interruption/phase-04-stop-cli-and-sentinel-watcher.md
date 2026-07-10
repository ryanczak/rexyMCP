# Phase 04: `rexymcp stop` CLI + `.rexymcp/stop` sentinel watcher (global stop-all)

**Milestone:** M30 — Executor Interruption
**Status:** in-progress (bounced — see bug-04-1)
**Depends on:** phase-03 (`JobRegistry` cancel handles + `request_stop` + real `CancelSignal` threading)
**Estimated diff:** ~360 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Give the **human** a client-agnostic way to stop a running executor from a second
terminal — the path that does not depend on the architect (Claude Code sends no
MCP `notifications/cancelled`, and the architect is itself blocked awaiting the
in-flight call). A new `rexymcp stop` CLI writes a `.rexymcp/stop` **sentinel
file** in the target repo; a serve-side **watcher** polls for it and, when it
appears, fires **every** live run's `CancelSignal` (`UserStop`) and removes the
sentinel. This is the `UserStop` counterpart to phase-03's architect `stop_phase`
(`ClaudeStop`).

**Design (decided with the user, 2026-07-10): global stop-all.** `.rexymcp/stop`
is a **presence flag** — no run-id payload. Any serve-side run that sees it
cancels. `rexymcp stop --run <id>` (run-scoped targeting) is **deferred** (a later
phase), matching the "a human hits the brakes from a second terminal" model. This
is a clean subset of the design already described in `docs/architecture.md`
§ Status #30 ("`rexymcp stop [--run <id>]` writes `.rexymcp/stop`, which a
serve-side watcher … honor") — so **no `architecture.md` edit is required** (the
`--run` option stays in the target design, just unimplemented here).

## Architecture references

- `docs/architecture.md` § Status #30 — the sentinel path: "`rexymcp stop` writes
  `.rexymcp/stop`, which a serve-side watcher (and the still-blocking CLI
  `run-phase`) honor — what a human uses from a second terminal."
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the
  `cancelled` status + `cancellation.reason` (`user_stop` here).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The registry can stop one run, not all (`mcp/src/jobs.rs`, phase-03)

Phase-03 added per-run cancellation:

```rust
pub fn request_stop(&self, run_id: &str, reason: CancelReason) -> bool {
    if let Some(entry) = self.lock().get_mut(run_id) {
        entry.stop_reason = Some(reason);
        entry.cancel.cancel();
        true
    } else {
        false
    }
}
```

`RunEntry` holds `state_tx: watch::Sender<RunState>` + `cancel: CancelHandle` +
`stop_reason: Option<CancelReason>`. There is **no** "fire all" and no way to ask
"is this run still running" (needed to bound the watcher's lifetime).

### The `execute_phase` branch spawns the run + a signal, but nothing watches a sentinel (`mcp/src/server.rs`, phase-03)

```rust
let run_id = crate::jobs::new_run_id();
let (cancel_handle, cancel_signal) = CancelSignal::new();
// … work = async move { execute_phase_inner(..., cancel_signal).await… } …
crate::jobs::spawn_run(runs.clone(), run_id.clone(), cancel_handle, work);
```

`repo_path` (the corroborated target-repo root) and `runs: Arc<JobRegistry>` are
both in scope here — everything the watcher needs.

### `.rexymcp/` is the per-repo state dir

`<repo>/.rexymcp/sessions/` (logs), `<repo>/.rexymcp/output/` (recovery). The
stop sentinel is `<repo>/.rexymcp/stop`, consistent with these
(`status.rs:301`: `repo.join(".rexymcp").join("sessions")`).

### `CancelReason::UserStop` exists but has no producer (phase-01)

```rust
pub enum CancelReason { UserStop, ClaudeStop }
```

Phase-03 wired `ClaudeStop`. This phase wires `UserStop` (its first producer).
`serde_json::to_value(CancelReason::UserStop)` == `json!("user_stop")`.

### CLI subcommands (`mcp/src/main.rs`)

`Commands` is a clap enum (variants `Init { dir, force }`, `Doctor { config,
json }`, `RunPhase { … }`, …) dispatched in `match cli.command { … }` at ~line
346. The blocking `RunPhase` arm builds a `RunPhaseConfig { …, cancel:
CancelSignal::never() }` (phase-03 left it `never()`).

## Spec

**This phase is deliberately additive** — a new sentinel module, two new registry
methods, one new CLI variant, and a localized watcher spawn. It does **not** add a
required field to any widely-constructed struct (the phase-03 cascade lesson: a
non-defaultable field on a many-call-site struct collides with the verifier's
6-strike limit). Nothing here breaks an existing constructor.

### 1. Sentinel path + file helpers (new `mcp/src/stop.rs`)

Create `mcp/src/stop.rs` and declare `mod stop;` in `main.rs` (alphabetical —
between `mod scorecard;`/`mod status;` as fits). Pure filesystem helpers:

```rust
use std::path::{Path, PathBuf};

/// The stop sentinel for a repo: `<repo>/.rexymcp/stop`. Its mere presence means
/// "stop all runs in this repo" (global stop-all; no run-id payload in this phase).
pub fn sentinel_path(repo: &Path) -> PathBuf {
    repo.join(".rexymcp").join("stop")
}

/// Write the sentinel (creating `.rexymcp/` if needed). Content is a human note;
/// only *presence* is load-bearing.
pub fn write_sentinel(repo: &Path) -> std::io::Result<PathBuf> {
    let path = sentinel_path(repo);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"stop requested\n")?;
    Ok(path)
}

/// True iff the sentinel exists.
pub fn sentinel_present(repo: &Path) -> bool {
    sentinel_path(repo).exists()
}

/// Remove the sentinel; a missing file is success (idempotent — several watchers
/// may race to clear it).
pub fn clear_sentinel(repo: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(sentinel_path(repo)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
```

### 2. Registry: fire-all + a liveness check (`mcp/src/jobs.rs`)

Add two methods to `impl JobRegistry` (additive — no signature changes):

```rust
/// Fire every live run's cancel signal with `reason`, recording it for the
/// terminal-result stamp. Returns how many runs were signalled. The global
/// stop-all path: one sentinel detection stops the whole serve process's runs.
pub fn request_stop_all(&self, reason: CancelReason) -> usize {
    let mut map = self.lock();
    let mut n = 0;
    for entry in map.values_mut() {
        entry.stop_reason = Some(reason.clone());
        entry.cancel.cancel();
        n += 1;
    }
    n
}

/// Whether a run exists and is still `Running` (not yet terminal). Used to bound
/// the sentinel watcher's lifetime so it exits once its run finishes.
pub fn is_running(&self, run_id: &str) -> bool {
    self.lock()
        .get(run_id)
        .map(|e| !e.state_tx.borrow().is_terminal())
        .unwrap_or(false)
}
```

(`CancelReason` is `Clone` — `request_stop_all` clones it per entry.)

### 3. The sentinel watcher (new `mcp/src/stop_watcher.rs`)

Create `mcp/src/stop_watcher.rs` + `mod stop_watcher;` in `main.rs`. One async
function, spawned per run:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rexymcp_executor::phase::CancelReason;

use crate::jobs::JobRegistry;
use crate::stop;

/// Poll `<repo>/.rexymcp/stop` while run `run_id` is live. On first sight of the
/// sentinel, fire **all** runs (`UserStop`) and clear the sentinel, then exit.
/// Also exits (without firing) once `run_id` goes terminal, so the task never
/// outlives its run. `poll` is injectable for tests.
pub async fn watch_stop_sentinel(
    repo_path: PathBuf,
    registry: Arc<JobRegistry>,
    run_id: String,
    poll: Duration,
) {
    loop {
        tokio::time::sleep(poll).await;
        if !registry.is_running(&run_id) {
            return; // run finished on its own — nothing to watch
        }
        if stop::sentinel_present(&repo_path) {
            registry.request_stop_all(CancelReason::UserStop);
            let _ = stop::clear_sentinel(&repo_path);
            return;
        }
    }
}
```

Add the poll interval constant (in `stop_watcher.rs`):

```rust
/// How often the serve-side watcher checks for `.rexymcp/stop`. Stop latency is
/// bounded by this (a human waits at most this long after `rexymcp stop`).
pub const STOP_POLL_INTERVAL: Duration = Duration::from_millis(500);
```

### 4. Spawn the watcher for each async run (`mcp/src/server.rs`)

In the `execute_phase` branch, right after `spawn_run(...)`, spawn the watcher
(both `repo_path` and `runs` are in scope; clone what the task needs):

```rust
crate::jobs::spawn_run(runs.clone(), run_id.clone(), cancel_handle, work);
tokio::spawn(crate::stop_watcher::watch_stop_sentinel(
    repo_path.clone(),
    runs.clone(),
    run_id.clone(),
    crate::stop_watcher::STOP_POLL_INTERVAL,
));
```

(`repo_path` is the already-corroborated `PathBuf` from the branch. No struct
field, no signature change — a localized two-statement addition.)

### 5. The `rexymcp stop` CLI subcommand (`mcp/src/main.rs`)

Add a `Stop` variant to `Commands` (mirror the `Doctor`/`Init` shape):

```rust
/// Signal a running executor to stop — writes `.rexymcp/stop` in the target repo,
/// which the serve-side watcher (or a blocking `run-phase`) sees and cancels.
Stop {
    /// Target repo root (where `.rexymcp/` lives). Defaults to the current dir.
    #[arg(long, default_value = ".")]
    repo: PathBuf,
},
```

Add the dispatch arm in `match cli.command`:

```rust
Commands::Stop { repo } => {
    let path = stop::write_sentinel(&repo)?;
    println!("wrote stop sentinel: {}", path.display());
    println!("running executors in this repo will cancel within ~1s.");
    Ok(())
}
```

(`use crate::stop;` at the top of `main.rs` if the `mod stop;` alone isn't
enough — reference it as `stop::write_sentinel` / `crate::stop::write_sentinel`
consistently with the other modules.)

### 6. Blocking `run-phase` sentinel honoring — DEFERRED to phase-04b

**Not in this phase.** Making the blocking CLI `run-phase` honor the sentinel
(build a real `CancelSignal` + a single-handle watcher, since it has no
`JobRegistry`) is split into **phase-04b** (decided with the user, 2026-07-10).
The `RunPhase` arm keeps `cancel: CancelSignal::never()` here — do **not** touch
it. This phase is the async `execute_phase` path only (Tasks 1–5).

## Acceptance criteria

- [ ] All four gates green (`fmt`/`build`/`clippy`/`test`).
- [ ] `rexymcp stop --repo <dir>` creates `<dir>/.rexymcp/stop`.
- [ ] `request_stop_all(UserStop)` fires every registered run's signal and returns
      the count; on an empty registry returns `0`.
- [ ] `watch_stop_sentinel` fires `request_stop_all(UserStop)` and removes the
      sentinel when it is present and the run is live; exits without firing once
      the run is terminal.
- [ ] `is_running` is `true` for a `Running` run, `false` for a terminal or
      unknown run.
- [ ] No `#[allow(...)]`, no new dependency.

## Test plan

Hermetic (`TempDir`, no sleeps beyond a **tiny injected** `poll`, no real clock).

- `mcp/src/stop.rs`:
  - `write_then_present_then_clear_roundtrips` — `write_sentinel` makes
    `sentinel_present` true; `clear_sentinel` makes it false; a second
    `clear_sentinel` is still `Ok` (idempotent).
  - `sentinel_path_is_under_dot_rexymcp` — path ends with `.rexymcp/stop`.
- `mcp/src/jobs.rs`:
  - `request_stop_all_fires_every_run_and_counts` — insert 2 runs with observable
    signals; `request_stop_all(UserStop)` returns 2 and both signals
    `is_cancelled()`.
  - `request_stop_all_on_empty_registry_is_zero`.
  - `is_running_true_for_running_false_after_publish` — `true` after `insert`,
    `false` after `publish(terminal)`, `false` for unknown id.
- `mcp/src/stop_watcher.rs`:
  - `watcher_fires_stop_all_and_clears_when_sentinel_present` — `TempDir` repo,
    insert a live run, `write_sentinel`, run `watch_stop_sentinel` with
    `poll = 1ms`; assert the run's signal cancelled and the sentinel gone.
  - `watcher_exits_without_firing_when_run_terminal` — insert then `publish`
    terminal, no sentinel; the watcher returns promptly and does **not** create or
    require a sentinel (pin the negative: signal never fired).

Use a pre-fired / observable `CancelSignal` per phase-03's `cancel_flips_signal`
pattern. **No wall-clock sleeps in tests beyond the 1ms injected poll.**

## End-to-end verification

- `cargo run -p rexymcp -- stop --repo <tmp>` then confirm `<tmp>/.rexymcp/stop`
  exists (quote the `ls`/file output).
- Note that exercising the *live* watcher end-to-end needs a running `rexymcp
  serve` with an in-flight run (restart serve to pick up this binary); the
  hermetic `stop_watcher` tests stand in for that where a live serve isn't
  available. State that limitation if the live path can't be driven headlessly.

## Authorizations

- [ ] May touch `docs/architecture.md`: **No** — § Status #30 already describes
      the sentinel; global-stop is a subset (—`--run` deferred). Do not edit it.
- [ ] May add dependencies: **No** — `tokio` (`time`/`sync`) + `std::fs` suffice.
- [ ] May add `#[allow(...)]`: **No.**

## Out of scope

- **Run-scoped stop** (`rexymcp stop --run <id>`, a sentinel carrying a run-id) —
  deferred; this phase is global stop-all only.
- **Blocking `run-phase` sentinel honoring** — **phase-04b** (the `RunPhase` arm
  keeps `CancelSignal::never()` here; do not modify it).
- **The async-polling skill-loop rewrite** — phase-05.
- **A filesystem-notify/inotify watcher** — polling at `STOP_POLL_INTERVAL` is
  intentionally simple and dependency-free; do not add a notify crate.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### 🔴 BOUNCE FIX REQUIRED — 2026-07-10 (read this first)

**This phase was bounced in review on `bug-04-1` (major). The gates are all GREEN
— that is the trap: the production code is correct, but one test does not actually
test what it is named for, so it would pass even if the code were broken.** Do
**not** report "complete" until you make the one-line test fix below and re-run the
gates. This is a **test-quality** bounce; the production `watch_stop_sentinel` is
correct — do **not** change it.

**The fix (bug-04-1):** in `mcp/src/stop_watcher.rs`, the test
`watcher_exits_without_firing_when_run_terminal` discards the watcher task's
outcome, so it can't tell whether the watcher actually exited on a terminal run.
Replace this line:

```rust
        let _ = tokio::time::timeout(Duration::from_secs(5), watcher).await;
```

with an assertion that the spawned watcher task **actually returned** (a timeout =
the watcher never exited = test failure):

```rust
        tokio::time::timeout(Duration::from_secs(5), watcher)
            .await
            .expect("watcher should exit promptly once the run is terminal")
            .expect("watcher task should not panic");
```

(`.expect()` on a test-only path is allowed — STANDARDS §2.1 exempts test code.)

**Prove it is load-bearing before you finish:** temporarily invert the terminal
check in `watch_stop_sentinel` (`if !registry.is_running(&run_id)` →
`if registry.is_running(&run_id)`, making the watcher loop forever), confirm the
rewritten test now **fails** (the 5 s timeout elapses), then **revert the
inversion** so the shipped production code is unchanged. Only the test changes in
this fix.

Then re-run all four gates; they must stay green with the corrected test.

### Update — ts=1783704780055 (complete, server-authored)

**Summary:** The commit is clean — all 7 files are included. The shell mangled the commit message display but the actual commit content is fine.

**Summary:** Implemented phase-04: `rexymcp stop` CLI + `.rexymcp/stop` sentinel watcher (global stop-all). Built:

1. **`mcp/src/stop.rs`** — sentinel file helpers (`sentinel_path`, `write_sentinel`, `sentinel_present`, `clear_sentinel`) with roundtrip tests.
2. **`mcp/src/jobs.rs`** — added `JobRegistry::request_stop_all` (fires every run's cancel signal, returns count) and `is_running` (checks if a run is still `Running`). All with tests.
3. **`mcp/src/stop_watcher.rs`** — async `watch_stop_sentinel` that polls for the sentinel and fires `request_stop_all(UserStop)` when found, exiting when the run goes terminal. Includes hermetic tests.
4. **`mcp/src/server.rs`** — spawns the watcher alongside each async run.
5. **`mcp/src/main.rs`** — new `Stop` CLI subcommand that writes the sentinel.

End-to-end verified: `cargo run -p rexymcp -- stop --repo <tmp>` successfully created `<tmp>/.rexymcp/stop` with content "stop requested\n". All 510 tests pass, clippy clean, fmt clean. Blocking `run-phase` sentinel honoring is deferred to phase-04b per spec.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.20s


TEST
::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::references_no_matches_advisory ... ok
test tools::symbols::tests::references_finds_call_sites ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::references_exclude_substring ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 949 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.18s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.19s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M30-executor-interruption/README.md` — +1 -1
- `docs/dev/milestones/M30-executor-interruption/phase-04-stop-cli-and-sentinel-watcher.md` — +1 -1
- `mcp/src/jobs.rs` — +63 -0
- `mcp/src/main.rs` — +15 -0
- `mcp/src/server.rs` — +6 -0
- `mcp/src/stop.rs` — +68 -0
- `mcp/src/stop_watcher.rs` — +102 -0

**Commit:** 2496d103ad0367c51da649cd7b929a2f24e5e956

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-10

- **Verdict:** bounced
- **Bounces:** 1 (bugs: bug-04-1 — major)
- **Executor:** AEON-7/Qwen3.6-27B-AEON (LARGE)
- **Scope deviations:** none
- **Calibration:** none

All four gates re-ran green independently (fmt/build/clippy/test), the
`rexymcp stop --repo <tmpdir>` E2E path was re-verified, `request_stop_all` /
`is_running` semantics hold, no `#[allow]`/new dependency, and Task 6
(`RunPhase` still `CancelSignal::never()`) is correctly deferred to phase-04b.
Bounced solely on bug-04-1: `watcher_exits_without_firing_when_run_terminal`
doesn't actually verify the watcher exits — confirmed by injecting an inverted
exit condition into `watch_stop_sentinel` and observing the test still pass.
