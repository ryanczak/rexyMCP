# M8 — Live session dashboard

**Goal:** Give the user real-time visibility into a running `execute_phase` call.
An `execute_phase` call is opaque *and* blocking — the Claude Code MCP client sends
no `progressToken`, so MCP progress notifications never reach the UI. The user is
blind for minutes at a time. `rexymcp dashboard` is the cure: a `ratatui`-powered,
continuously-refreshed read-only TUI that tails the per-record-flushed session JSONL
and shows turn/stage/tool, parse and verifier signals, files changed, and budget
consumption in a `btop`-style paned layout.

**Status:** in progress — phase-01 todo.

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
| 01  | `ratatui` scaffold — event loop + single summary pane ([phase-01-dashboard-scaffold.md](phase-01-dashboard-scaffold.md)) | review |
| 02  | paned layout — turn/stage · parse/verify · files · budget ([phase-02-dashboard-panels.md](phase-02-dashboard-panels.md)) *(not yet drafted)* | planned |

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

*(retrospective written at milestone close)*
