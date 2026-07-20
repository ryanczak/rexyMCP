# M35 — Metrics & Cost Accounting Overhaul

**Goal:** Make rexyMCP's two accounting stories — local-LLM performance and
token/cost — one coherent, discoverable CLI surface: every recorded number is
either displayed somewhere or deleted, quality and cost appear side by side,
and the executor finally carries a real (configurable) price.

**Status:** in-progress

**Depends on:** M34 (calibrate-governor is one of the surfaces being aligned)

**Exit criteria:**

- Telemetry records carry an explicit `schema_version`; readers ignore
  records at any other version. Telemetry is **on by default** at an XDG data
  dir (`$XDG_DATA_HOME/rexymcp`, falling back to `~/.local/share/rexymcp`);
  opt-out via `[telemetry] enabled = false` or `--no-telemetry`. The
  legacy-tolerant deserializers (`TokenBreakdown` visitor) and the
  never-populated `doc_level` field are gone.
- Per-run generation speed (tok/s) is recorded, and `ToolResult` output sizes
  are logged (`output_bytes`) so the output-flood detector becomes
  calibratable (M34 deferral).
- One shared metrics/cost module owns every derived number (reclaimed-token
  sums, tok/s, settings labels, `cost()` over all four token classes); the
  four hand-rolled duplicates are gone. Local executor models can carry
  configured $/Mtok rates; unpriced models cost $0.
- `rexymcp runs` shows tokens, cost, and tok/s; `rexymcp runs show <id>`
  drills into one run (full token breakdown incl. cache, gates, verdict,
  bugs/warnings, cost).
- `rexymcp scorecard --by model|tag|settings` unifies the CLI and MCP
  aggregations; previously-computed-but-dropped columns (wall-clock, verifier
  retries, repairs) are displayed; `rexymcp profile` reports tokens & cost
  **per approved phase**.
- `rexymcp costs` reports Baseline / Executor / Architect / Net across
  Session × Milestone × Project; the dashboard Budget panel renders from the
  same core, no longer ignores cache token buckets, and gains a `b`-key
  tokens ⇄ currency toggle (the event panel's `f`-key pattern).
- Oscillation calibration reports low percentiles for its lower-is-worse
  signal; `calibrate-governor` output aligns with the shared rendering.
- All four gates green.

## Architecture references

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" —
  the PhaseRun record, scorecard matrix, and pull-not-push discipline.
- `docs/architecture.md` § Status #35 — this milestone's design summary.
- `docs/dev/milestones/M34-governor-stall-hardening/README.md`
  § "Deferred to the planned metrics & reporting deep-dive".

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Telemetry store foundation ([phase-01-telemetry-store-foundation.md](phase-01-telemetry-store-foundation.md)) | done |
| 02 | Capture gaps: generation speed + output bytes ([phase-02-capture-gaps-speed-output-bytes.md](phase-02-capture-gaps-speed-output-bytes.md)) | in-progress |
| 03 | Shared metrics & cost core + `[models]` pricing | not drafted |
| 04 | Run-level surface: `runs` columns + `runs show` | not drafted |
| 05 | Aggregate surfaces: unified scorecard + profile efficiency | not drafted |
| 06 | `rexymcp costs` + dashboard rewire + `b` toggle | not drafted |
| 07 | Reporting debt: oscillation tail, calibrate-governor alignment, discoverability | not drafted |

Phases 02–07 are titles-only until drafted on demand (`/rexymcp:architect
next`) — earlier phases shape later specs, per WORKFLOW § Milestones.

## Notes

**Origin.** This is the "post-06b metrics & reporting deep-dive" queued at the
M34 close. Design pass run with the user 2026-07-19; the four decision forks
were resolved explicitly:

1. **Local pricing = configurable $/Mtok per model** (a pricing table; unpriced
   models default to $0) rather than a cloud-equivalent shadow price or
   tokens-only. Cost math becomes uniform with architect billing.
2. **Primary cost surface = a new `rexymcp costs` command**, with cost columns
   woven into `runs`/`scorecard`, and the dashboard Budget panel re-rendered
   from the same core plus a `b`-key tokens ⇄ $ toggle.
3. **M34's deferred reporting debt folds in** (oscillation wrong-tail,
   `output_bytes`, calibrate-governor alignment) — it is all reporting-layer
   work.
4. **Telemetry goes default-on with a versioned, cleaned schema.** Backward
   compatibility explicitly waived by the user: pre-M35 records (no
   `schema_version`) are ignored by readers, so history restarts at the
   upgrade. The accumulated pre-M35 corpus stays on disk but goes dark in the
   aggregators; the session-log corpus (calibrate-governor's input) is
   unaffected.

**Design findings the plan rests on** (code audit 2026-07-19): per-run
`TokenBreakdown` (incl. cache buckets) is recorded but shown nowhere; executor
cost is hardcoded $0.00 in the dashboard (`mcp/src/dashboard/panels.rs`
comment); "tokens reclaimed" is hand-summed in four places
(`scorecard.rs` ×2, `runs.rs`, `status.rs`); `verifier_retries`,
`repairs_per_call`, `wall_clock_s`, `bugs_filed`, `warnings`,
`compaction_count` are computed or folded but never rendered;
`tier_telemetry.doc_level` has never been populated; the model×tag scorecard
is MCP-only with no CLI path.