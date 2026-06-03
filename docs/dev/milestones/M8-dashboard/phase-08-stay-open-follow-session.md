# Phase 08: Dashboard stays open and follows a newly-started session

**Milestone:** M8 — Live session dashboard
**Status:** done
**Depends on:** phase-01 (done) — this fixes phase-01's auto-exit behavior and
builds on its event loop and `load_status` path-resolution.
**Estimated diff:** ~90 lines (`mcp/src/dashboard.rs` loop edit + `mcp/src/status.rs`
small extraction + tests).
**Tags:** language=rust, kind=bugfix, size=s

## Goal

`rexymcp dashboard --repo <path>` currently flashes up and exits after ~2 seconds
whenever the most recent session in the log is already finished. The cause is an
auto-exit branch in the event loop that breaks the loop as soon as the latest
session has `ended`. That branch was meant for the live-watch case (watch a running
session, it finishes, show the final frame briefly, close) but it misfires when the
dashboard is launched while no phase is actively running — the very common case.

This phase makes the dashboard a persistent, attach-and-follow monitor:

1. **Stay open until the user quits** (`q` / `Esc` / `Ctrl-C`) — never auto-exit.
2. **Follow the newest session.** When launched with no `--session` pin, the
   dashboard re-resolves "newest session log" on every poll, so when a *new*
   executor session starts (its log file appears / gets its first write), the
   dashboard attaches to it automatically on the next refresh.
3. **Respect an explicit pin.** When launched with `--session <needle>`, the
   dashboard stays on that one session and must **not** jump to a newer one.

**mcp-crate only.** No executor change, no MCP-server change, no new dependency.
Read-only as the rest of M8: the dashboard never writes the JSONL or talks to the
running executor.

## Architecture references

Read before starting:

- M8 README § "Design decisions" — "Read-only, no side effects" and "Hermetic data
  layer": the dashboard polls by re-reading the JSONL (not inotify), and the data
  layer is tested without a terminal. This phase keeps both invariants.
- `docs/architecture.md` § Layer 2 "Liveness (pull, not push)" — `rexymcp status`
  and `dashboard` are the pull-based liveness path; this is why a poll loop that
  re-resolves the newest log is the right mechanism.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read `mcp/src/dashboard.rs` end to end (small) — you edit the event loop.
3. Read `mcp/src/status.rs` end to end (small) — you do a tiny extraction here.
4. Read this entire phase doc before touching code.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green before changing anything.

## Current state

### The auto-exit branch to remove (`mcp/src/dashboard.rs`, in `run_loop`)

The loop polls every 500 ms, redraws, and checks for a quit key. Then it has this
trailing branch (lines ~293–301):

```rust
        if data.summary.ended.is_some() {
            let now_ms2 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            terminal.draw(|frame| render_dashboard(frame, frame.area(), &data, now_ms2))?;
            std::thread::sleep(Duration::from_secs(2));
            break;
        }
```

This is the entire bug: `data.summary.ended` reflects the *latest resolved session's*
state, which is already `Some(..)` on the first iteration when no phase is running, so
the loop draws once, sleeps 2 s, and breaks.

The rest of `run_loop` is correct and stays as-is — in particular the quit handling:

```rust
        if event::poll(Duration::from_millis(500))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                _ => {}
            }
        }
```

(`Ctrl-C` exits via the terminal's default SIGINT handling, unchanged.)

### The log-resolution this phase makes testable (`mcp/src/status.rs`, in `load_status`)

`load_status` already re-resolves which log to read on every call, and `run_loop`
already calls `load_data` → `load_status` every poll. So "follow the newest" *already*
falls out of the existing per-poll resolution — **the only thing defeating it is the
auto-exit branch above.** This phase does not add a watcher; it removes the early exit
and then locks the follow-vs-pin semantics with tests.

The resolution lives inline in `load_status` (lines ~199–218):

```rust
    let log_path = match session {
        Some(needle) => {
            let entries = std::fs::read_dir(&dir)
                .map_err(|e| format!("no session logs under {}: {}", dir.display(), e))?;
            entries
                .flatten()
                .map(|e| e.path())
                .find(|p| {
                    p.extension().and_then(|e| e.to_str()) == Some("jsonl")
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.contains(needle))
                })
                .ok_or_else(|| {
                    format!("no session log matching '{needle}' under {}", dir.display())
                })?
        }
        None => find_latest_session_log(&dir)
            .ok_or_else(|| format!("no session logs found under {}", dir.display()))?,
    };
```

`find_latest_session_log` (lines ~122–140) picks the most-recently-modified `*.jsonl`:

```rust
pub fn find_latest_session_log(sessions_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}
```

There is no test that pins the *pinned-vs-follow* distinction (that a needle pin
stays put while the unpinned case moves to a newer file). This phase adds it.

## Spec

Numbered tasks in execution order.

1. **Remove the auto-exit branch** — in `mcp/src/dashboard.rs`, delete the entire
   `if data.summary.ended.is_some() { … }` block quoted in Current state (the
   `now_ms2` recompute, the extra `terminal.draw`, the `sleep(2s)`, and the `break`).
   After removal, `run_loop` breaks **only** on the `q` / `Esc` key match. Do not
   leave the block commented out (STANDARDS: no commented-out code). The `Duration`
   import stays — it is still used by `event::poll(Duration::from_millis(500))`.

2. **Extract the log-resolution into a named, testable function** — in
   `mcp/src/status.rs`, lift the `let log_path = match session { … }` block out of
   `load_status` into a sibling public function:

   ```rust
   /// Resolve which session log to read this poll. `session = None` follows the
   /// most-recently-modified log (so a newly-started session is picked up on the
   /// next poll); `session = Some(needle)` pins to the log whose file name contains
   /// `needle` and never moves off it, regardless of which log is newest.
   pub fn resolve_session_log(repo: &Path, session: Option<&str>) -> Result<PathBuf, String> {
       let dir = sessions_dir(repo);
       match session {
           Some(needle) => { /* the existing needle find, unchanged */ }
           None => find_latest_session_log(&dir)
               .ok_or_else(|| format!("no session logs found under {}", dir.display())),
       }
   }
   ```

   Then `load_status` calls `resolve_session_log(repo, session)?` and keeps the rest
   (`read_session_log` + `summarize`) as-is. This is a behavior-preserving extraction
   — `load_status`'s output for any input must be identical to today's. The point is
   only to give the resolution a name and a unit-test seam.

3. **No state added to `run_loop`.** The "follow" behavior is emergent: each poll
   already calls `load_data(repo, session)` afresh, which now calls
   `resolve_session_log` afresh. Do **not** add a "remember the previously-followed
   log" field — re-resolving each poll is the mechanism.

## Behavior to pin (positive **and** negative)

- **Stays open:** with `summary.ended.is_some()`, the loop does **not** break on its
  own — only a `q`/`Esc` press breaks it. (Verified by inspection + the absence grep
  in E2E; see below — the interactive break itself is not unit-testable headlessly.)
- **Unpinned follows newest:** `resolve_session_log(repo, None)` returns the
  most-recently-modified `*.jsonl`. When a second log is made newer than the first,
  a subsequent call returns the **second**.
- **Pinned does NOT move (negative case):** `resolve_session_log(repo, Some(needle))`
  returns the needle-matched log **even when a different, non-matching log is the
  newest**. Make the non-matching log strictly newer in the test and assert the
  result is *still* the pinned one — this is the must-NOT-jump case.
- **Empty / launched-before-any-session:** `resolve_session_log(repo, None)` against a
  repo with no logs returns `Err(..)` (today's behavior). In the live loop this
  surfaces as the error pane and the dashboard **stays open**; when a session later
  appears, the next poll resolves it. (No new code — this falls out of removing the
  auto-exit. Pin only the `Err` at the resolution level.)

## Acceptance criteria

- [ ] The `if data.summary.ended.is_some()` block is gone from `mcp/src/dashboard.rs`
      (`grep -n "ended.is_some" mcp/src/dashboard.rs` prints nothing).
- [ ] `mcp/src/dashboard.rs` `run_loop` contains exactly one `break`, inside the
      `KeyCode::Char('q') | KeyCode::Esc` arm.
- [ ] `pub fn resolve_session_log` exists in `mcp/src/status.rs` and `load_status`
      calls it; `cargo build` is clean.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` is clean (run `rustfmt mcp/src/dashboard.rs
      mcp/src/status.rs` if needed — do **not** run the writing form of `cargo fmt`).
- [ ] `cargo test -p rexymcp` passes, including the new resolution tests.

## Test plan

Add to the `#[cfg(test)] mod tests` block in `mcp/src/status.rs`. Use a `TempDir` and
set file mtimes deterministically — **do not rely on write order for mtime ordering**
(filesystem timestamp granularity makes that flaky). Use `std::fs::File::set_modified`
(stable std, **no new dependency**) to stamp controlled times. Pattern:

```rust
use std::time::{Duration, SystemTime};

fn write_log_with_mtime(dir: &std::path::Path, name: &str, mtime: SystemTime) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "").unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.set_modified(mtime).unwrap();
    path
}
```

- `resolve_unpinned_picks_newest_log` — write `session-phase-01-aaa.jsonl` at
  `t0` and `session-phase-02-bbb.jsonl` at `t0 + 10s`; assert
  `resolve_session_log(repo, None)` returns the `bbb` (phase-02) path.
- `resolve_unpinned_follows_when_newer_log_appears` — start with only the `aaa` log;
  assert resolution returns `aaa`; then add a strictly-newer `bbb` log; assert a
  second `resolve_session_log(repo, None)` now returns `bbb` (the follow-on-new-session
  behavior, with no loop state).
- `resolve_pinned_ignores_newer_nonmatching_log` — write the pinned `aaa` log at `t0`
  and a **newer** non-matching `bbb` log at `t0 + 10s`; assert
  `resolve_session_log(repo, Some("aaa"))` returns the `aaa` path (the must-NOT-jump
  negative case).
- `resolve_unpinned_errs_when_no_logs` — empty sessions dir (or absent); assert
  `resolve_session_log(repo, None)` is `Err`.
- `resolve_pinned_errs_when_no_match` — one `aaa` log present; assert
  `resolve_session_log(repo, Some("zzz"))` is `Err`.

Keep the existing `load_data` / `summarize` / `find_latest_session_log` tests green
(the extraction is behavior-preserving). Note `sessions_dir(repo)` is the
`<repo>/.rexymcp/sessions` join, so tests build that path under the `TempDir`, as the
existing `dashboard.rs` tests do (`crate::status::sessions_dir`).

## End-to-end verification

The "stays open until `q`" criterion is interactive CLI behavior that requires a real
TTY; the executor is headless and cannot drive `crossterm` key events, so it is **not**
fully runnable end-to-end here. Verify the shippable surface as follows and paste the
output in the completion log:

1. **Auto-exit is gone (the actual fix), by grep:**
   ```
   grep -n "ended.is_some\|sleep(Duration::from_secs(2))" mcp/src/dashboard.rs
   ```
   Expected: no matches.
2. **Single break in the loop:**
   ```
   grep -n "break" mcp/src/dashboard.rs
   ```
   Expected: one match, in the `q`/`Esc` arm.
3. **Resolution behavior** is proven by the new unit tests — paste the
   `cargo test -p rexymcp resolve_` output.

The interactive "stays open / attaches to the new session" behavior is then confirmed
by inspection: with the early-exit branch removed, `run_loop` only exits on the quit
key, and each poll re-resolves via `resolve_session_log`. State this reasoning in one
line in "Notes for review".

## Authorizations

None. (No new dependency — `File::set_modified` is stable std. No
`docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edit.)

## Out of scope

- **The "attached to <session>" flash cue.** When the followed log switches, the
  Session panel already shows the new session id / phase, so the switch is visible.
  A dedicated one-frame banner is a possible later nicety, not this phase.
- **inotify / filesystem-watch.** Keep polling; the M8 "Read-only, no side effects /
  poll by re-reading" decision stands.
- **Any executor or MCP-server change.** This is mcp-crate dashboard/status only.
- **Changing the 500 ms cadence, the panel layout, or `rexymcp status`.**
- **A "session ended" indicator change** beyond what the Session panel already shows
  (`state: ended (..)`). Do not add new UI for the ended state.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-18 (started)

**Executor:** rexyMCP executor

**Tasks started:** Remove auto-exit branch from `dashboard.rs` event loop; extract log-resolution into `resolve_session_log` in `status.rs`; add 5 unit tests for resolve behavior.

### Update — 2025-07-18 (complete)

**Tasks completed:**

1. Removed the auto-exit `if data.summary.ended.is_some() { … }` block from `run_loop` in `mcp/src/dashboard.rs`. The loop now breaks **only** on `q`/`Esc`.
2. Extracted log-resolution into `pub fn resolve_session_log(repo, session)` in `mcp/src/status.rs`; `load_status` now calls it. Behavior-preserving extraction.
3. Added 5 unit tests for `resolve_session_log`: unpinned picks newest, unpinned follows when newer appears, pinned ignores newer non-matching, unpinned errs when no logs, pinned errs when no match.

**Files changed:** `mcp/src/dashboard.rs`, `mcp/src/status.rs`

**Verification commands:**

```
$ cargo fmt --all --check
(exit 0, no output)

$ cargo build 2>&1 | tail -20
   Compiling rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.91s

$ cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.46s

$ cargo test -p rexymcp 2>&1 | tail -40
test result: ok. 180 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test -p rexymcp resolve_ 2>&1
running 5 tests
test status::tests::resolve_unpinned_errs_when_no_logs ... ok
test status::tests::resolve_unpinned_follows_when_newer_log_appears ... ok
test status::tests::resolve_pinned_ignores_newer_nonmatching_log ... ok
test status::tests::resolve_pinned_errs_when_no_match ... ok
test status::tests::resolve_unpinned_picks_newest_log ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 175 filtered out
```

**End-to-end verification:**

1. Auto-exit block removed — `grep -n "data.summary.ended.is_some\|sleep(Duration::from_secs(2))" mcp/src/dashboard.rs` prints nothing. (Note: `summary.ended.is_some()` on line 61 is the pre-existing styling branch in `session_lines`, not the auto-exit.)
2. Single `break` in `run_loop` — `grep -n "break" mcp/src/dashboard.rs` shows exactly one match at line 288 in the `q`/`Esc` arm.
3. Resolution behavior proven by 5 unit tests (output pasted above).

**Verification summary:** build clean, clippy clean, fmt clean, 180 tests pass (175 existing + 5 new).

**Notes for review:** With the early-exit branch removed, `run_loop` only exits on the quit key (`q`/`Esc`), and each poll re-resolves the session log via `resolve_session_log` — so the dashboard stays open and auto-follows a newly-started session when unpinned, or stays pinned when `--session` is given.

**Commit:** `fix: dashboard stays open and follows newly-started session`

### Review verdict — 2026-06-03

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — all three spec tasks implemented exactly; the
  optional "attached to" cue was correctly left out of scope.
- **Calibration:** none. (Cosmetic: the executor's Update Log is date-stamped
  `2025-07-18` from model clock drift; real date 2026-06-03. Not a defect — a
  recurring local-LLM clock quirk, not worth a fold.)
- **Independent re-run:** fmt clean, build clean, clippy clean, `cargo test -p
  rexymcp` 180 passed. Auto-exit grep empty; single `break` in the `q`/`Esc`
  arm. `resolve_session_log` extraction is behavior-preserving (identical logic
  + error strings); the pinned-ignores-newer test is a genuine negative case.
