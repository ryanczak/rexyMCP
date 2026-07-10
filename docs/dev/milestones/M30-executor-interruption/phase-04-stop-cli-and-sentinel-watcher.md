# Phase 04: `rexymcp stop` CLI + `.rexymcp/stop` sentinel watcher (global stop-all)

**Milestone:** M30 — Executor Interruption
**Status:** todo
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

### 6. (OPTIONAL — architect to confirm at review) Blocking `run-phase` honoring

The `docs/architecture.md` narrative says the blocking CLI `run-phase` also
honors the sentinel. That requires `run-phase` to build a **real** `CancelSignal`
(instead of `never()`) and spawn a single-handle watcher firing that one handle
(it has no `JobRegistry`). This is a **separable** addition.

**Recommendation:** ship Tasks 1–5 (the async `execute_phase` path — the primary,
and the one the phase-05 skill loop will drive) as phase-04, and split `run-phase`
honoring into a small **phase-04b** if it risks pushing this over one session. The
architect will decide at review which way to cut it; if included, add:

- a `watch_stop_sentinel_single(repo_path, cancel_handle, poll)` in
  `stop_watcher.rs` (fires one `CancelHandle`, no registry), and
- change the `RunPhase` arm to `let (h, s) = CancelSignal::new();` + spawn that
  watcher + pass `cancel: s` into `RunPhaseConfig`.

Leave this task out unless the review says otherwise; do **not** silently expand
scope.

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
- **Blocking `run-phase` sentinel honoring** — Task 6, ship only if the review
  says so (else phase-04b).
- **The async-polling skill-loop rewrite** — phase-05.
- **A filesystem-notify/inotify watcher** — polling at `STOP_POLL_INTERVAL` is
  intentionally simple and dependency-free; do not add a notify crate.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
