# Phase 07: Process-group cleanup for `bash` and gate commands

**Milestone:** M51 — Governor repetition, cycles & process hygiene
**Status:** in-progress
**Depends on:** phase-06
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=reliability, size=s

## Goal

Spawn `bash` tool commands and `RealCommandRunner` gate commands in their own
Unix process group (`process_group(0)`) behind an RAII `ProcessGroupGuard`, so
timeouts and async cancellations (`stop_phase` / dropped futures) signal the
entire process group (`-pgid`) and never leave deadlocked `cargo test` or shell
grandchildren orphaned. Implements related observation #2 of
[issue #18](https://github.com/ryanczak/rexyMCP/issues/18).

## Architecture references

- `docs/dev/milestones/M51-governor-repetition-and-cycles/README.md` § Exit criteria and § Notes #1.
- `docs/architecture.md` § Status, M51 entry.

## Pre-flight

1. Read `docs/dev/STANDARDS.md`.
2. Read this phase doc and the milestone README.
3. Confirm a clean tree.

## Current state

`executor/src/tools/bash.rs` spawns `sh -c <command>` without placing the child
in its own process group, and on timeout runs `kill -9 <pid>` against only the
parent `sh` PID — leaving descendant processes (such as test binaries spawned
by `cargo test`) orphaned. If `Bash::execute` is cancelled (future dropped)
before the timeout expires, no kill runs at all. `executor/src/agent/command.rs`
(`RealCommandRunner::run`) awaits `tokio::process::Command::new("sh")...output().await`
with inherited stdin and no process-group isolation or drop cleanup.

## Spec

1. **Shared process-group helper in `executor/src/process_group.rs`** (declared
   `pub(crate) mod process_group;` in `executor/src/lib.rs`):
   - `pub(crate) fn configure_process_group(cmd: &mut tokio::process::Command)`:
     under `#[cfg(unix)]`, calls `cmd.process_group(0)` (safe API in Tokio/std;
     no `unsafe`, no new dependencies); under `#[cfg(not(unix))]`, no-op.
   - `pub(crate) fn kill_process_group(pgid: u32)`:
     - Guard `if pgid == 0 { return; }` — **critical Unix safety invariant**:
       passing `-0` to `kill` signals the caller's own process group and would
       kill the server / test runner itself.
     - Under `#[cfg(unix)]`, runs `std::process::Command::new("kill")` with
       args `["-9", "--", &format!("-{pgid}")]`, `Stdio::null()` on stdin,
       stdout, and stderr, ignoring exit status (the group may already have
       exited).
     - Under `#[cfg(not(unix))]`, no-op.
   - `pub(crate) struct ProcessGroupGuard { pgid: Option<u32> }`:
     - `pub(crate) fn new(pgid: Option<u32>) -> Self` storing `pgid.filter(|&id| id > 0)`.
     - `pub(crate) fn disarm(&mut self)` setting `self.pgid = None` (called
       after the child finishes normally so dropping the guard never signals a
       later-recycled PID).
     - `pub(crate) fn kill_now(&mut self)` taking `self.pgid.take()` and
       calling `kill_process_group(id)` if `Some(id)`.
     - `impl Drop for ProcessGroupGuard` calling `self.kill_now()` so dropping
       an in-flight async future immediately kills the entire process group.
2. **Wire process-group cleanup into `executor/src/tools/bash.rs`**:
   - Call `crate::process_group::configure_process_group(&mut cmd)` before
     `cmd.spawn()`.
   - Immediately after `cmd.spawn()` succeeds, construct
     `let mut pg_guard = crate::process_group::ProcessGroupGuard::new(child.id())`.
   - On `Ok(Ok(output))` and `Ok(Err(e))`: call `pg_guard.disarm()`.
   - On `Err(_)` (timeout): call `pg_guard.kill_now()` instead of the old
     single-PID `kill -9 <id>`.
   - Preserve all existing behavior: `Stdio::null()` stdin, `ALLOWED_ENV_KEYS`
     env allowlist, output filtering, and timeout error/metadata shape.
3. **Wire process-group cleanup into `executor/src/agent/command.rs`
   (`RealCommandRunner::run`)**:
   - Build `tokio::process::Command::new("sh")` with `-c <command>`,
     `current_dir(cwd)`, `.stdin(std::process::Stdio::null())`,
     `.stdout(std::process::Stdio::piped())`,
     `.stderr(std::process::Stdio::piped())`, and
     `crate::process_group::configure_process_group(&mut cmd)`.
   - Spawn the child, wrap `child.id()` in `ProcessGroupGuard::new(child.id())`,
     await `child.wait_with_output().await`, and call `pg_guard.disarm()` once
     `wait_with_output` returns. If the `run` future is dropped mid-execution,
     `Drop for ProcessGroupGuard` signals the entire gate process group.
   - **Constraints:**
     - Zero `unsafe` blocks and zero new crate dependencies.
     - No `.unwrap()` / `.expect()` on production paths (`STANDARDS.md` §2.1).
     - **Gotcha for Unix liveness assertions in tests:** inside containers/PID
       namespaces, a killed grandchild can remain briefly as a zombie (`Z`
       state) until PID 1 reaps it, and `kill -0 <pid>` still succeeds on a
       zombie! Check liveness via `ps -o state= -p <pid>`: treat an empty
       output **or** a state starting with `'Z'` as dead (polled in a short
       bounded loop, e.g. up to 500ms, never a fixed multi-second sleep), and
       verify the grandchild writes its PID inside `TempDir` before the kill /
       drop fires.
4. **Capture the end-to-end evidence** — run the block in § End-to-end
   verification verbatim and paste the artifact file into a new Update Log
   entry headed `### Update — <date> (end-to-end verification)`.

## Acceptance criteria

- [ ] `executor/src/process_group.rs` provides `configure_process_group`,
      `kill_process_group` (with `pgid == 0` guard and `-9 -- -<pgid>`), and
      `ProcessGroupGuard` with `disarm()`, `kill_now()`, and `Drop`, using zero
      `unsafe` and zero new dependencies.
- [ ] `executor/src/tools/bash.rs` spawns children in their own Unix process
      group and kills the entire group on both timeout and future cancellation.
- [ ] `executor/src/agent/command.rs` (`RealCommandRunner::run`) sets
      `Stdio::null()` stdin, spawns gate commands in their own Unix process
      group, and kills the entire group when dropped mid-flight.
- [ ] Every test named in § Test plan passes.
- [ ] Preservation: all existing `tools::bash::tests` and `agent::command::tests`
      pass unchanged.
- [ ] All four gates green (`cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo test` — separate invocations).

## Test plan

- In `executor/src/process_group.rs`:
  - `kill_process_group_zero_is_noop` — negative safety test:
    `kill_process_group(0)` and dropping `ProcessGroupGuard::new(Some(0))` are
    no-ops and do not terminate the test process.
  - `disarmed_guard_does_not_kill_process_group_on_drop` (`#[cfg(unix)]`) —
    negative test: spawning a child in its own process group, disarming its
    `ProcessGroupGuard`, and dropping the guard leaves the child running until
    explicitly cleaned up at test end.
- In `executor/src/tools/bash.rs` (`#[cfg(unix)]`):
  - `timeout_kills_grandchild_in_process_group` — runs a bash command
    (`timeout_secs: 1`) that spawns a background `sleep 60` grandchild, records
    the grandchild PID to a file inside `TempDir`, and waits; asserts the tool
    returns `timed_out: true` and the grandchild PID is dead/zombie (`ps -o state= -p <pid>`
    empty or starting with `'Z'`).
  - `dropped_bash_future_kills_grandchild_in_process_group` — spawns
    `bash.execute(...)` with a 30s timeout running `sleep 60 & echo $! > pid.txt; wait`
    inside `TempDir`, drops/aborts the future once `pid.txt` contains the
    grandchild PID, and asserts the grandchild PID is dead/zombie.
- In `executor/src/agent/command.rs` (`#[cfg(unix)]`):
  - `real_command_runner_kills_grandchild_when_dropped` — starts
    `RealCommandRunner.run("sleep 60 & echo $! > pid.txt; wait", dir.path())`,
    drops the future once `pid.txt` contains the grandchild PID, and asserts
    the grandchild PID is dead/zombie.
  - `real_command_runner_normal_exit_captures_output_and_null_stdin` — runs
    `cat; echo out; echo err >&2`, asserts `success == true`, and output
    contains both `out` and `err` (proving stdin hit EOF immediately via
    `Stdio::null()` and normal exit disarms the guard cleanly).

## End-to-end verification

```bash
mkdir -p target/e2e
A=target/e2e/m51-phase-07.txt; : > "$A"
for t in kill_process_group_zero_is_noop disarmed_guard_does_not_kill_process_group_on_drop timeout_kills_grandchild_in_process_group dropped_bash_future_kills_grandchild_in_process_group real_command_runner_kills_grandchild_when_dropped real_command_runner_normal_exit_captures_output_and_null_stdin times_out_advisory_failure bash_child_stdin_is_dev_null; do
  cargo test -p rexymcp-executor "$t" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E '^test |test result: .* [1-9][0-9]* passed' >> "$A"; echo "$t exit=${PIPESTATUS[0]}" >> "$A"
done
! grep -rn '\bunsafe\b' executor/src/process_group.rs executor/src/tools/bash.rs executor/src/agent/command.rs >> "$A"; echo "no_unsafe exit=$?" >> "$A"
```

Expected: every listed test prints `ok` with `exit=0` and a non-zero passed
count, and `no_unsafe exit=0`.

## Authorizations

- `executor/src/lib.rs`
- `executor/src/process_group.rs` (new file)
- `executor/src/tools/bash.rs`
- `executor/src/agent/command.rs`

No new dependencies. No `unsafe`.

## Out of scope

- Verifier subprocesses (`cargo check` / `tsc` / `ruff` in `governor/verifier.rs`)
  or git helpers (`head_content.rs`).
- Windows Job Object support (Unix `process_group(0)` only; non-Unix builds
  remain clean via `#[cfg(unix)]`).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-11-25 04:39 (started)

**Executor:** Claude (executor run)
**Status flipped:** todo → in-progress (phase doc + milestone README).
**Next:** implement `executor/src/process_group.rs`, wire into `tools/bash.rs` and `agent/command.rs`, add the § Test plan tests, then capture § End-to-end verification.

### Update — 2026-11-25 04:39 (end-to-end verification)

Ran the § End-to-end verification block verbatim; artifact pasted below from
`target/e2e/m51-phase-07.txt` (captured mechanically, not retyped). Every listed
test prints `ok` with `exit=0` and a non-zero passed count; `no_unsafe exit=0`.
Positive control: each kill test waits until the grandchild has verifiably
written its PID into a `TempDir` file *before* the kill/drop fires, then asserts
the PID is dead/zombie via `ps -o state= -p <pid>` in a bounded 500 ms poll —
if the process-group kill were not live, the `sleep 60` grandchild would still
report state `S` and the test would fail.

```
test process_group::tests::kill_process_group_zero_is_noop ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.00s
kill_process_group_zero_is_noop exit=0
test process_group::tests::disarmed_guard_does_not_kill_process_group_on_drop ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.10s
disarmed_guard_does_not_kill_process_group_on_drop exit=0
test tools::bash::tests::timeout_kills_grandchild_in_process_group ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 1.01s
timeout_kills_grandchild_in_process_group exit=0
test tools::bash::tests::dropped_bash_future_kills_grandchild_in_process_group ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.01s
dropped_bash_future_kills_grandchild_in_process_group exit=0
test agent::command::tests::real_command_runner_kills_grandchild_when_dropped ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.01s
real_command_runner_kills_grandchild_when_dropped exit=0
test agent::command::tests::real_command_runner_normal_exit_captures_output_and_null_stdin ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.00s
real_command_runner_normal_exit_captures_output_and_null_stdin exit=0
test tools::bash::tests::times_out_advisory_failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 1.01s
times_out_advisory_failure exit=0
test tools::bash::tests::bash_child_stdin_is_dev_null ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1149 filtered out; finished in 0.01s
bash_child_stdin_is_dev_null exit=0
no_unsafe exit=0
```

Gate commands (separate invocations, all green): `cargo fmt --all --check`,
`cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test` (1148 + 712 passed, 0 failed).
