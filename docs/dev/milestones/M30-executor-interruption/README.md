# M30 — Executor Interruption

**Goal:** Give both the user and the architect a way to interrupt and stop a
running executor mid-phase — a capability that does not exist today.

**Status:** in-progress

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
| 04b | Blocking `run-phase` sentinel honoring (split from 04) | — |
| 05 | Async-polling skill-loop rewrite + contract-doc updates (planned) | — |

## Notes

Phases 02–05 are sketched, not drafted — the architect expands them on demand
(WORKFLOW § Milestones) since phase-01's `CancelSignal`/`Cancellation` types are
the load-bearing surface the later phases import, and their exact shape may shift
during phase-01 review. Phase-01 is deliberately executor-crate-only: it ships no
runtime cancellation entrypoint (no MCP/CLI caller wires a real signal until
phase-02), so it is exercised entirely by hermetic unit tests firing a
`CancelSignal` against a `MockAiClient`/parking `AiClient`.

<!-- retrospective appended at milestone close -->