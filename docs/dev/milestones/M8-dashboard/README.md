# M8 — Live session dashboard

**Goal:** Give the user real-time visibility into a running `execute_phase` call.
An `execute_phase` call is opaque *and* blocking — the Claude Code MCP client sends
no `progressToken`, so MCP progress notifications never reach the UI. The user is
blind for minutes at a time. `rexymcp dashboard` is the cure: a `ratatui`-powered,
continuously-refreshed read-only TUI that tails the per-record-flushed session JSONL
and shows turn/stage/tool, parse and verifier signals, files changed, and budget
consumption in a `btop`-style paned layout.

**Status:** done — all 16 phases complete (2026-06-07). Both original Exit criteria
met; full redesign through phase-13 shipped. Retrospective below.

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
| 07  | Compaction events — emit `SessionEvent::Compaction` from `compact()` ([phase-07-compaction-event.md](phase-07-compaction-event.md)) | done |
| 08  | Dashboard stays open + follows a newly-started session ([phase-08-stay-open-follow-session.md](phase-08-stay-open-follow-session.md)) | done |
| 09  | Redesign: header band + Compactions panel + Files left-trim ([phase-09-redesign-layout-compactions.md](phase-09-redesign-layout-compactions.md)) | done |
| 10a | Activity transcript: raw-record reader + scroll mechanics + plain text ([phase-10a-activity-transcript-scroll.md](phase-10a-activity-transcript-scroll.md)) | done |
| 10b | Activity transcript: color + multi-line + tool output + tail-follow ([phase-10b-transcript-rich-render.md](phase-10b-transcript-rich-render.md)) | done |
| 11a | Budget panel: Tokens/Sec throughput (JSONL-derived) ([phase-11a-budget-tokens-per-sec.md](phase-11a-budget-tokens-per-sec.md)) | done |
| 11b | Budget panel: "$ saved" (configurable $/Mtok rate, config plumbing) ([phase-11b-budget-dollars-saved.md](phase-11b-budget-dollars-saved.md)) | done |
| 12  | Activity pane spinner (🐾 throbber while model is running + auto-scroll anchor) ([phase-12-activity-spinner.md](phase-12-activity-spinner.md)) | done |
| 13  | Activity pane event filter (per-type toggle, progress off by default) ([phase-13-activity-filter.md](phase-13-activity-filter.md)) | done |

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

### Retrospective (2026-06-07)

**16 phases** across two distinct arcs: the original measurement roadmap (01–08), and
a full UI redesign triggered by user wireframes (09–13).

**What shipped:**
- **Phase-01:** `rexymcp dashboard` scaffold — `ratatui` event loop, single pane,
  `q`/`Esc`/`Ctrl-C` exit, 500 ms poll of the live session JSONL.
- **Phase-02:** btop-style three-panel layout (Session · Heartbeat · Files).
- **Phase-03:** executor bugfix — think-block-only completions distinguished from clean
  prose exits (`ParseResult::NoToolCall` branch); closed bug-executor-1.
- **Phase-04:** Activity panel — `summarize` folds `ParseFailed`/`Verify`/`ToolResult`/
  `HardFail` (previously dropped as `_ => {}`); four-panel 2×2 grid.
- **Phase-05:** stream-retry resilience — mid-stream connection drops retry up to 3×
  (250 ms / 500 ms / 1 s backoff); closed bug-executor-2.
- **Phase-06a/b:** per-turn `SessionEvent::Metrics` (executor emit) + Budget panel
  (live token counts, colored context-window gauge).
- **Phase-07:** `SessionEvent::Compaction` emitted from `compact()` — closed Gap C.
- **Phase-08:** dashboard stays open after session end, auto-follows a newly-started
  session; extracted testable `resolve_session_log`.
- **Phase-09:** redesign — four-panel header band (Session · Budget · Compactions ·
  _merged_), Compactions panel, Files left-trim (`trim_path_left`, `FILE_PATH_MAX=40`).
- **Phase-10a/b:** fully scrollable transcript — raw-record reader, scroll-key
  handling (Up/Down/PgUp/PgDn/Home/End), plain-text then color + multi-line + tool
  output + tail-follow anchor.
- **Phase-11a/b:** Budget panel gains Tokens/Sec (JSONL-derived Δoutput/Δsec) and
  "$ saved" (configurable `saved_input_per_mtok`/`saved_output_per_mtok` in
  `rexymcp.toml`, config plumbing through the `dashboard` CLI command).
- **Phase-12:** activity pane spinner — 9-frame dog-chasing-brain throbber while the
  model is running, auto-scroll anchor.
- **Phase-13:** event-type filter — per-type toggle, `Progress` off by default.

**Executor (Qwen/Qwen3.6-27B-FP8) quality:** clean first-try on all single-concern
phases — 06a, 06b, 07, 08, 09, 10a, 11a, 11b, 12, 13. Escalations limited to three
cases:

1. **Phase-01 (architect spec gap):** `crossterm 0.28` pin couldn't unify with
   `ratatui 0.30`'s `crossterm 0.29`; architect fixed and took over after a
   backend-glitched no-op re-dispatch.
2. **Phase-02 (executor bug):** Qwen3.6-35B-A3B-FP8 produced three false-`complete`
   no-ops — root cause was bug-executor-1 (think-only completion mistreated as clean
   exit), fixed in phase-03.
3. **Phase-10b (executor stall):** executor wrote all production code correctly but
   looped on `IdenticalToolCallRepetition` during mechanical test-update churn
   (multi-edit patch repetition). Architect takeover finished the test updates.

**Calibration folds from M8:**
- bug-executor-1 (think-only → false clean exit) → phase-03 runtime fix. No
  WORKFLOW fold; the fix is in the runtime.
- Phase-10b stall: mechanical test-update churn → `IdenticalToolCallRepetition`.
  Second occurrence after M8/phase-10b was later seen in M9/phase-04 (deletion
  churn). Two occurrences = trend; if a third fires, fold a WORKFLOW rule: split
  mechanical-deletion/edit from companion steps.
- Stash ambient changes before re-dispatch: a dirty working tree let the executor
  sweep unrelated changes into a commit. One occurrence (phase-01 area). Data point
  carried to NEXT.md; a second occurrence in M9/phase-01 confirmed the fold.
