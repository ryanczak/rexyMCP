# M30 — Executor Interruption

**Goal:** Give both the user and the architect a way to interrupt and stop a
running executor mid-phase — a capability that does not exist today.

**Status:** done

**Depends on:** none

## Why now

The executor is currently **uninterruptible**. `execute_phase` runs the turn
loop (`executor/src/agent/mod.rs:280`) to completion, bounded only by budgets
(max turns, wall-clock, context) checked at turn boundaries and per-call
timeouts — there is no stop signal anywhere in the loop. Auto mode
(`plugin/skills/auto/SKILL.md`) is a skill-layer loop of sequential **blocking**
`execute_phase` calls, so a runaway or unwanted phase churns a local LLM for
minutes with no off switch.

Two facts from the design investigation constrain what is buildable:

1. **Claude Code sends no MCP `notifications/cancelled`.** When the user hits
   Esc, Claude stops awaiting locally and *orphans* the request; the executor
   keeps running server-side. rmcp 1.8 *does* hand the tool handler a
   `context.ct` cancellation token (`mcp/src/server.rs:530`, currently ignored),
   but Claude Code never fires it — so it is a dead channel for the primary path.
2. **While `execute_phase` is in-flight, the architect is itself blocked**
   awaiting the tool result, so Claude cannot issue a second MCP call (e.g. a
   stop tool) mid-phase either.

Therefore any real interrupt must arrive **out-of-band and be polled by the
executor loop**. The design (decided with the user) is an **async job model**:
`execute_phase` returns a `run_id` immediately and the run becomes pollable and
cancellable, plus a filesystem **stop sentinel** that a human writes from a
second terminal.

## The design (decided)

- **Cooperative cancel primitive.** The executor loop's cancel test becomes
  `signal.is_cancelled()`, evaluated at (a) the top of the turn loop
  (`mod.rs:280`) and (b) a new third `tokio::select!` branch (`mod.rs:384`) so a
  stuck in-flight model stream aborts too. The primitive is a
  `tokio::sync::watch`-based `CancelSignal` — **no new dependency** (`tokio`'s
  `sync` feature is already enabled).
- **New `cancelled` outcome.** A fourth `PhaseStatus`, `Cancelled`, whose
  `PhaseResult` **leaves the working tree dirty** (no revert, no auto-commit) and
  reports the partial diff (the existing `diff`/`files_changed` fields) plus a
  `Cancellation { reason, stage, turns_done }` record for the architect/human to
  triage.
- **Async MCP `execute_phase`.** Spawns the run in the serve process, registers
  it under a `run_id`, returns immediately. `get_run_status(run_id)` is a bounded
  long-poll (terminal state or ~15s); `stop_phase(run_id)` cancels the run's
  in-memory `CancelSignal` handle (the architect's mid-flight abort, at
  poll granularity).
- **Filesystem stop sentinel.** `rexymcp stop [--run <id>]` writes `.rexymcp/stop`
  from a separate process; a serve-side watcher (and the still-blocking CLI
  `run-phase`) bridge the sentinel to a `CancelSignal` handle. This is the
  client-agnostic path a human uses from a second terminal.
- **CLI `run-phase` stays blocking** (its owner can Ctrl-C that process) but
  still honors the sentinel.

## Exit criteria

- A running executor can be stopped mid-phase by **the user** (via `rexymcp
  stop` from a second terminal) and by **the architect** (via `stop_phase` between
  polls), and returns a `PhaseResult` with status `cancelled`.
- The `cancelled` result leaves the working tree **dirty** (unreverted,
  uncommitted) and reports the partial diff + stage + turns-done.
- No new dependency is added for the cancel primitive.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #30 (this milestone) and § Layer 2
  "Liveness" (the "opaque *and blocking*" characterization M30 supersedes).
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" — the value
  `execute_phase` returns, which gains the `cancelled` status.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Executor `CancelSignal` + `cancelled` outcome ([phase-01-cancel-signal-and-outcome.md](phase-01-cancel-signal-and-outcome.md)) | done |
| 02 | MCP job registry + async `execute_phase` + `get_run_status` ([phase-02-job-registry-and-async-execute.md](phase-02-job-registry-and-async-execute.md)) | done |
| 03 | `stop_phase` MCP tool + reason/partial-diff enrichment ([phase-03-stop-phase-tool-and-reason-threading.md](phase-03-stop-phase-tool-and-reason-threading.md)) | done |
| 04 | `rexymcp stop` CLI + `.rexymcp/stop` sentinel watcher (async path) ([phase-04-stop-cli-and-sentinel-watcher.md](phase-04-stop-cli-and-sentinel-watcher.md)) | done |
| 04b | Blocking `run-phase` sentinel honoring ([phase-04b-run-phase-sentinel-honoring.md](phase-04b-run-phase-sentinel-honoring.md)) | done |
| 05a | Async-polling skill rewrite + `cancelled` handling ([phase-05a-async-polling-skill-rewrite.md](phase-05a-async-polling-skill-rewrite.md)) | done |
| 05b | Contract-doc fold: async/interrupt model ([phase-05b-contract-doc-fold.md](phase-05b-contract-doc-fold.md)) | done |

## Notes

Phases 02–05 are sketched, not drafted — the architect expands them on demand
(WORKFLOW § Milestones) since phase-01's `CancelSignal`/`Cancellation` types are
the load-bearing surface the later phases import, and their exact shape may shift
during phase-01 review. Phase-01 is deliberately executor-crate-only: it ships no
runtime cancellation entrypoint (no MCP/CLI caller wires a real signal until
phase-02), so it is exercised entirely by hermetic unit tests firing a
`CancelSignal` against a `MockAiClient`/parking `AiClient`.

<!-- retrospective appended at milestone close -->

## Retrospective — 2026-07-10

**Shipped:** a complete mid-phase interrupt path. `execute_phase` became an async
job (`run_id` + `get_run_status` long-poll, phase-02); `stop_phase` gives the
architect a `claude_stop` abort (phase-03); `rexymcp stop` → `.rexymcp/stop`
(global stop-all) gives the human a `user_stop` abort honored by both the serve
async path (phase-04) and the blocking `run-phase` CLI (phase-04b); a fourth
`cancelled` `PhaseStatus` carries the partial diff on a dirty tree (phase-01); and
the skills + contract docs were realigned to the async/interrupt model with a 5th
`auto` stop condition `STOP(cancelled)` (phase-05a/05b).

**Verdicts:** 01 approved_after_2 · 02 approved_after_1 · 03 **escalated
(takeover, 2 hard_fails)** · 04 approved_after_1 · 04b approved_first_try · 05a
approved_first_try (Claude direct) · 05b approved_first_try (Claude direct).

**Calibration — no new folds** (both are recurrences of already-folded patterns):

1. **Required-field cascade vs. the governor's 6-verifier-strike limit** (phase-03,
   2 hard_fails → takeover). Adding a non-defaultable `cancel: CancelSignal` field
   across ~14 call sites means the crate can't compile until *every* site is fixed,
   so the verifier strikes out mid-cascade before the executor finishes. This is
   the WORKFLOW § "Prefer additive change shapes" pattern (already folded, M7
   phase-05a/05b) — recurring, not new. **The mitigation worked:** once phases 04/
   04b/05 were designed *additively* (new modules/methods, no cascade), the executor
   completed them cleanly (04 first-try, 04b/05a/05b first-try, one one-line test
   bounce on 04). The forward lesson for a future required-field change: either
   split so intermediate states compile, or instruct "make all cascade edits before
   any build," or pre-flag the phase as takeover-likely.
2. **Green-bounce refined re-dispatch** held 3× (bug-02-1, bug-02-2, bug-04-1): when
   a bounce leaves all gates green (a slow test, a masking `#[allow]`, a weak test),
   a plain re-dispatch no-ops; a loud "read this first" header with the exact inline
   fix converts it. Consistent with the `plain-redispatch-noops-on-green-bounce`
   calibration — reconfirmed, no doc change.

**Design decisions recorded:** async-job model on MCP `execute_phase` (decided at
kickoff); global stop-all sentinel with `--run <id>` scoping deferred; CLI-path
`cancelled` reason-stamping deferred (async path owns it); `cancelled` → STOP-for-
human in the auto loop with human-only stop agency (the loop never fires
`stop_phase` itself); detect-and-adapt dispatch for the stale-serve/`run-phase`
transition.

**Not exercised live:** the async `execute_phase` / `stop_phase` / `rexymcp stop`
path was validated by hermetic tests only — the connected `rexymcp serve` ran the
pre-M30 blocking binary all run (the `stale-rexymcp-serve-after-rebuild` pattern),
so every dispatch returned a synchronous `PhaseResult`. A serve restart + live
smoke test of the interrupt path is the recommended first action next session.