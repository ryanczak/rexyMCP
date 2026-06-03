# M8 — Live session dashboard

**Goal:** Give the user real-time visibility into a running `execute_phase` call.
An `execute_phase` call is opaque *and* blocking — the Claude Code MCP client sends
no `progressToken`, so MCP progress notifications never reach the UI. The user is
blind for minutes at a time. `rexymcp dashboard` is the cure: a `ratatui`-powered,
continuously-refreshed read-only TUI that tails the per-record-flushed session JSONL
and shows turn/stage/tool, parse and verifier signals, files changed, and budget
consumption in a `btop`-style paned layout.

**Status:** in progress — phase-01–06b done. Both original Exit criteria are met
("parse/verifier signal" via phase-04's Activity panel; "budget consumed" via 06b's
Budget panel). **Reopened from the milestone-close gate (2026-06-03, user decision)**
to fix a dashboard usability bug: phase-08 makes the dashboard stay open until the
user quits and auto-follow a newly-started session (phase-01's auto-exit-on-`ended`
caused it to flash up and exit when no phase is running). The optional **phase-07**
(compaction events) remains undrafted; the close-gate decision is deferred until
phase-08 lands.

**Depends on:** M7 (done) — the session JSONL and `status.rs` are the data source.
`rexymcp status` is the one-shot predecessor; the dashboard is its live, paned
sibling built on the same `summarize()` core.

**Exit criteria:**

- `rexymcp dashboard` launches in a terminal alongside a running Claude Code session,
  tails the active session log, and continuously shows turn, stage, last tool,
  parse/verifier signal, files changed so far, and budget consumed — without
  requiring any changes to the executor loop or MCP server.
- The display refreshes at ≤ 1 s cadence and exits cleanly on `q` / `Esc` /
  `Ctrl-C`.
- `rexymcp status` is unchanged — still available for scripting / CI.
- The code is hermetically testable: the TUI rendering layer is separated from the
  data-reading layer so tests can exercise the latter without spawning a real
  terminal.

## Architecture references

- `docs/architecture.md` § Layer 2 "Liveness (pull, not push)" — the design
  rationale and the open `status` vs. `dashboard` decision (resolved: keep both).
- `mcp/src/status.rs` — the existing `load_status` / `summarize` / `format_status`
  pipeline that this milestone builds on.
- `executor/src/store/sessions/event.rs` — the nine `SessionEvent` variants the JSONL
  carries; the dashboard reads `Progress`, `Verify`, `HardFail`, `ParseFailed`, and
  `SessionEnd` specifically.

## Phases

| #   | Phase                                                                               | Status |
|-----|-------------------------------------------------------------------------------------|--------|
| 01  | `ratatui` scaffold — event loop + single summary pane ([phase-01-dashboard-scaffold.md](phase-01-dashboard-scaffold.md)) | done |
| 02  | paned layout — Session · Heartbeat · Files ([phase-02-dashboard-panels.md](phase-02-dashboard-panels.md)) | done |
| 03  | executor bugfix — think-only completion treated as clean exit ([phase-03-think-only-fix.md](phase-03-think-only-fix.md)) | done |
| 04  | Activity panel — surface parse/verify/tool/hard-fail signals ([phase-04-activity-signals.md](phase-04-activity-signals.md)) | done |
| 05  | executor resilience — retry on mid-stream connection drop ([phase-05-stream-retry-resilience.md](phase-05-stream-retry-resilience.md)) | done |
| 06a | metrics event — executor emits per-turn `SessionEvent::Metrics` (tokens + context %) ([phase-06a-metrics-event.md](phase-06a-metrics-event.md)) | done |
| 06b | Budget panel — render the live token / context-window metrics ([phase-06b-budget-panel.md](phase-06b-budget-panel.md)) | done |
| 07  | Compaction events — record `compact()` firings as a `SessionEvent` *(not yet drafted)* | planned |
| 08  | Dashboard stays open + follows a newly-started session ([phase-08-stay-open-follow-session.md](phase-08-stay-open-follow-session.md)) | review |

## Design decisions

**`ratatui` + `crossterm`** — the standard Rust TUI crate; `crossterm` is its
backend. Both are new to the dependency tree (phase-01 is authorized to add them).

**Two-phase decomposition.** Phase 01 gets the event loop, polling cadence, resize
handling, and clean exit right with a minimal single-pane rendering (TUI-wrapped
`StatusSummary`). Phase 02 adds the btop-style paned layout (split-screen, four
panels, collapsible). Separating scaffold from layout keeps each dispatch to a
manageable diff and means phase 01 produces a usable (if minimal) tool.

**`rexymcp status` preserved.** `status --json` is the scripting/CI path; the
dashboard is the interactive path. They share `status.rs`'s `summarize()` core.

**Read-only, no side effects.** The dashboard never writes to the session JSONL, never
communicates with the running executor, and never blocks the Claude Code session.
It opens the JSONL read-only, polls by re-reading it (not via inotify), and has
no MCP dependency.

**Hermetic data layer.** The `DashboardData` / `DataReader` type that wraps
`load_status` + `summarize` is a plain function or thin struct, testable without a
terminal. The TUI rendering layer takes `DashboardData` as input and produces
nothing except terminal output — it is *not* unit-tested directly (terminal output
is an integration concern), but it is simple enough to review by inspection.

## Notes

### Measurement roadmap (designed 2026-06-02 with the user)

**Core finding — a measurement asymmetry.** The executor measures a lot at
*run-end* for the M7 scorecard (`PhaseRun`: `parse_failure_rate`,
`tool_success_rate`, `verifier_retries`, `tokens`, `length_finish_rate`,
`context_window`), but flushes almost none of it to the *live* session JSONL the
dashboard reads. The scorecard and the dashboard are two consumers of the same
conceptual substrate, but today they read different, unequal stores: `PhaseRun`
(rich, post-hoc, in the telemetry dir) vs. the session JSONL (impoverished, live,
per-record). Closing M8's Exit criteria means narrowing that gap. Three classes:

- **Gap A — Surfacing (data already in the JSONL, `summarize` drops it).** The loop
  logs `ParseFailed`, `Verify`, `ToolResult`, `HardFail` records, but
  `status::summarize` ignores them (`_ => {}`). Fix is **mcp-crate-only**: extend
  `StatusSummary` + `summarize` + add a panel. → **phase-04** (Activity panel).
- **Gap B — Capture (computed but never written to the JSONL per-turn).** Token
  usage accumulates in `RunMetrics` (fed by `AiEvent::Done`) and lands only in the
  end-of-run `PhaseRun`. Context-window fullness is checked against `Budget.ceiling`
  but never recorded. To show "budget consumed" *live*, the executor must emit a new
  per-turn `SessionEvent::Metrics { input_tokens, output_tokens, context_pct }`.
  → **phase-06a** (executor emits the event) → **phase-06b** (the Budget panel that
  renders it). `Budget::fraction_used` already computes context %, and `cap.rs`'s
  catch-all passes the new variant through, so 06a is small.
- **Gap C — Not measured anywhere (even `PhaseRun` lacks it).**
  - *Live context-window utilization* — "prompt at 68% of the 256k window, +4%/turn."
    `PhaseRun` has the window *size* and *cumulative* tokens, never the per-turn
    "how full right now." Highest-value missing live metric; delivered by phase-06a's
    `context_pct` and rendered by phase-06b.
  - *Compaction occurrences* — `compact()` (`agent/mod.rs:182`) fires silently. We
    never record when it ran or how much it evicted. (The M7 decisions doc flagged
    "Compaction monitoring (insufficient data)".) Fix: emit `SessionEvent::Compaction
    { evicted_tokens, … }`. → **phase-07**; feeds both dashboard and scorecard.
  - *Model latency / tokens-per-second / per-tool time* — derivable from record `ts`
    deltas but never computed. Lower priority; a later phase if wanted.

**The unifying move for B and C:** flush incremental metric snapshots to the JSONL
(`SessionEvent::Metrics` per turn, `SessionEvent::Compaction` on compaction). One
decision that (1) gives the live dashboard parity with the scorecard and (2)
retroactively enriches the JSONL as a forensic replay record.

*(milestone retrospective written at milestone close)*
