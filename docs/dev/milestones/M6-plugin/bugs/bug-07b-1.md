# Bug 1 on phase-07b: heartbeat tests use real `sleep`, not `tokio::time::pause()`/`advance()`

**Severity:** major (violates the hard "deterministic / no `sleep`" testing rule
and the phase's explicit test-plan instruction; introduces timing-window tests;
undeclared deviation)
**Status:** open
**Filed:** 2026-06-01

## What's wrong

The two heartbeat tests in `executor/src/agent/mod.rs` drive timing with
real wall-clock sleeps and assert on how many ticks happened to fire in that
window:

- `heartbeat_reemits_awaiting_model_while_in_flight`:
  `sleep(Duration::from_millis(350)).await;` then `assert!(awaiting_mid >= 3, …)`.
- `heartbeat_stops_when_model_responds`:
  `sleep(Duration::from_millis(250)).await;` … `sleep(Duration::from_millis(300)).await;`
  then asserts the awaiting_model count is unchanged.

To make the wall-clock windows fire enough ticks, the heartbeat period itself is
`#[cfg(test)]`-shrunk to 100 ms (`HEARTBEAT_PERIOD_MS`) vs. the production
`HEARTBEAT_PERIOD_SECS = 15`.

This breaks three things:

1. **Hard rule violation.** `CLAUDE.md` § Testing: *"Hermetic … and deterministic
   (no `sleep`, no real `Utc::now()` — inject a clock …)."* These tests sleep on
   the real clock.

2. **The phase doc said exactly how to do it.** 07b § Test plan: *"Use a
   `MockAiClient` … and `tokio::time::pause()` + `advance()` to step the
   heartbeat interval."* The executor's own helper doc comment in
   `executor/src/ai/testing.rs` (`MockAiClientPending`) says it exists *"so a
   test can hold `chat` in-flight across heartbeat ticks (`tokio::time::pause()`
   + `advance()`)"* — but the tests never call `pause()`/`advance()`.

3. **Deviation handled as a note, not a blocker.** The Update Log *does* explain
   the switch ("test heartbeat period is 100 ms … to avoid needing
   `tokio::time::test-util`"), so it is not wholly undeclared. But the right move
   was to **file a blocker**, not substitute a hard-rule-violating technique:
   `tokio::time::pause()`/`advance()` live behind tokio's `test-util` feature,
   and this phase authorized **no Cargo.toml edits**. So the spec's test plan
   mandated a technique the phase's own authorizations didn't permit — a genuine
   conflict the executor should have surfaced (WORKFLOW § "A blocker is cheap; a
   wrong silent fix is expensive"), not resolved unilaterally by reaching for
   real `sleep`. **Architect shares fault here** (spec under-specified — see the
   review verdict's calibration note); the re-dispatch resolves it by picking an
   approach and granting the matching authorization.

The tests pass today (they passed 5× idle and 6× under 32-way CPU load on the
review machine), so this is not a currently-reproducing failure — it is a
latent-flakiness + standards problem. The 100 ms-period / 350 ms-window margin
is generous on fast hardware but is exactly the kind of timing assumption that
erodes on a contended single-core CI runner, where the spawned `execute_phase`
task can be starved and fewer ticks fire than the `>= 3` threshold expects.
Reading the session-log file from the test thread *while the spawned task is
still appending to it* is a second nondeterminism source the real-sleep design
introduces.

## What should happen

The heartbeat tests are deterministic — no real `sleep` — and the production
heartbeat-period constant is what's exercised (no `#[cfg(test)]` period shrink).

## How to fix

Two acceptable approaches; the architect has chosen — and authorized — both
escape hatches so you can pick whichever lands cleaner. **Prefer Option B**
(no Cargo.toml change).

**Option A — virtual time (`pause()`/`advance()`).** Requires enabling tokio's
`test-util` feature. The re-dispatch authorizes this **dev-only** edit (see the
phase doc's updated Authorizations): add `test-util` to a `[dev-dependencies]`
`tokio` entry (do **not** add it to the non-dev `[dependencies]`/workspace
runtime features). Then `#[tokio::test(start_paused = true)]`, hold `chat`
in-flight via the `MockAiClientPending` `Notify` gate, step time with
`tokio::time::advance(HEARTBEAT_PERIOD)` (yielding `tokio::task::yield_now()`
between advances so the loop processes each tick and writes its record), and
assert the awaiting_model count grows by **exactly one per advanced period** —
not a `>=` threshold over a wall-clock window.

**Option B — inject the tick source (no new feature).** Make the heartbeat's
tick stream a seam instead of a hard-wired `tokio::time::interval`: extract the
select-loop into a helper that takes the chat future, an `emit` closure, and a
`futures::Stream` (or a small `mpsc::Receiver`) of ticks. Production passes an
interval-backed stream; the test passes a stream it pushes to synchronously, so
ticks fire on demand with **no clock at all**. Assert the deterministic
per-tick emit count, then end the tick stream / resolve the chat gate and assert
no further records. This needs no Cargo.toml change and gives the tightest
determinism.

In **both** options: remove the `#[cfg(test)]` period override so production and
tests share one `HEARTBEAT_PERIOD`, no real `sleep` anywhere in the test module,
and state the chosen approach in "Notes for review."

Keep `status_renders_awaiting_model_stage` (`mcp/src/status.rs`) as is — it is
already deterministic and correct.

## Verification

- [ ] No `tokio::time::sleep` (or other real-clock wait) remains in the
      `agent::tests` module — timing is virtual (Option A) or injected (Option B).
- [ ] No `#[cfg(test)]` heartbeat-period override remains; production and tests
      share one `HEARTBEAT_PERIOD`.
- [ ] The rewritten heartbeat tests assert a **deterministic** record count
      (exactly N per N ticks), not a `>=` threshold over a wall-clock sleep.
- [ ] If Option A: `test-util` is under `[dev-dependencies]` only, not the
      runtime tokio features.
- [ ] `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D
      warnings`, and `cargo test` all clean.

## Notes

- **Production code is approved on inspection — do not change it.** The pre-call
  emit, the `tokio::select!` heartbeat with `MissedTickBehavior::Skip`, the
  `Err` propagation, and the untouched drain/turn-counter are all correct and
  spec-faithful. This bounce is **tests-only**.
- **Non-blocking nit** (fix only if convenient): `tokio::time::interval`'s first
  `tick()` resolves immediately, so each turn emits the pre-call `awaiting_model`
  *and* an immediate heartbeat `awaiting_model` back-to-back — two records per
  turn even when the model responds instantly. Harmless, but if you want one
  pre-call record + ticks strictly thereafter, start the interval one period out
  (`tokio::time::interval_at(Instant::now() + period, period)`) or skip the first
  immediate tick. Not required for approval.
