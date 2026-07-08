# M26 — Polish & Hardening

**Goal:** Close the consistency and hardening gaps flagged by the post-M25
codebase review — stale contract docs, dead code paths, silent degradations, and
executor-loop blind spots — without adding new features or new dependencies.

**Status:** in-progress (kicked off 2026-07-07; phase-01 drafted)

**Depends on:** none

## Why now

M25 closed with the doc contract stable and the recurring e2e failure classes
structurally handled (M19/M21/M22–M24). A whole-codebase review
([codebase-review-2026-07-07.md](../../codebase-review-2026-07-07.md)) then
surfaced a different kind of debt: seams that no e2e run trips because they fail
*silently* — config knobs `rexymcp calibrate` writes that nothing consumes, a
corroboration path hardcoded to an empty roots list, CLI phase runs that skip
telemetry, contract docs asserting a milestone frontier that rotted, and governor
blind spots (oscillation, cumulative output, wall clock) adjacent to the ones
M22/M23 fixed. Unlike prior milestones, M26 is seeded from a review rather than a
dogfooding failure; each phase still carries its own verifiable exit condition
and review gate.

Two threads, grouped by subsystem so each phase is one executor session:

| Thread | Phases | Area |
|---|---|---|
| Housekeeping / corrections | 01–03 | contract docs, plugin manifests, `mcp` crate seams |
| Executor-loop hardening | 04–08 | tools gates, hooks, budget knobs, governor, verifier |

## Exit criteria

- `REXYMCP.md` no longer asserts a milestone frontier (must NOT match:
  `Active work is M7`, `clap CLI exposing`) and describes the `mcp` crate as the
  `rmcp` stdio server + CLI it actually is.
- One plugin identity: `plugin/plugin.json`, `plugin/.claude-plugin/plugin.json`,
  and `.claude-plugin/marketplace.json` `plugins[0]` agree on name `rexymcp` and
  description; no `rexymcp-plugin` reference remains in `README.md` or `plugin/`.
- `rexymcp run-phase` writes a `PhaseRun` record by default when telemetry is
  configured; a `--no-telemetry` flag suppresses it.
- ~~`execute_phase` corroborates `repo_path` against the client's real
  `roots/list`~~ — **deferred** (see Notes § "Roots corroboration deferred");
  rmcp 1.8.0 deprecated `list_roots` per MCP SEP-2577.
- An empty-or-missing `docs/dev/STANDARDS.md` and a phase doc that parses to an
  empty `## Goal` / `## Acceptance criteria` section each produce an
  architect-visible warning in `PhaseResult.warnings` instead of silence. (The
  third review item — an unwritable session-log directory — is deferred; it is
  generated deep inside the loop and needs different plumbing. See the phase-03
  doc § Out of scope.)
- `write_file` to an existing file the executor has not read (or whose mtime
  changed since the read) is refused with the read-first advisory, mirroring
  `patch`.
- The post-write format hook actually rewrites a deliberately misformatted
  touched file (today it runs the verify-only `--check` form — a no-op).
- `[budget] gate_retries` (tier-derived) is consumed by the M19 gate-retry loop:
  a persistently-red gate terminates as `budget_exceeded` after the resolved
  retry budget, not only at the turn cap. `[executor] tier` is thereby live for
  loop control as well as telemetry. **`[budget] escalation_slots` and
  `[escalation] max_assists` are deferred to M27** (the architect-side autonomous
  `/loop` owns escalation budgeting — decided with the user 2026-07-07; see
  Notes § "Escalation budgeting moved to M27").
- The governor halts an A,B,A,B two-call oscillation and a windowed
  cumulative-output flood; an optional wall-clock ceiling terminates a run when
  configured.
- The TS verifier resolves `tsc` via `node_modules/.bin`, then `npx`, then PATH.
- All four gates green after every phase; no new dependency; session-event /
  telemetry schema changes additive only (`#[serde(default)]`).

## Architecture references

- [`docs/dev/codebase-review-2026-07-07.md`](../../codebase-review-2026-07-07.md)
  — the review this milestone is drawn from (§1 housekeeping, §2 improvements).
- `docs/architecture.md` § Status #26 (added at kickoff).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Contract-doc & plugin-manifest consistency ([phase-01-contract-docs-and-manifests.md](phase-01-contract-docs-and-manifests.md)) | done |
| 02 | `run-phase` telemetry parity (`--no-telemetry` opt-out) ([phase-02-run-phase-telemetry-parity.md](phase-02-run-phase-telemetry-parity.md)) | done |
| 03 | Surface silent degradations as architect-visible warnings ([phase-03-silent-degradation-warnings.md](phase-03-silent-degradation-warnings.md)) | done |
| 04 | `write_file` read-before-edit gate ([phase-04-write-file-read-before-edit-gate.md](phase-04-write-file-read-before-edit-gate.md)) | done |
| 05 | Post-write format hook: writing form ([phase-05-post-write-format-hook-writing-form.md](phase-05-post-write-format-hook-writing-form.md)) | done |
| 06 | Wire `gate_retries` into the gate-retry loop ([phase-06-wire-gate-retries.md](phase-06-wire-gate-retries.md)) | done |
| 07a | Governor oscillation & windowed-output detectors ([phase-07a-governor-oscillation-and-output-detectors.md](phase-07a-governor-oscillation-and-output-detectors.md)) | done |
| 07b | Governor wall-clock ceiling (`[budget] wall_clock_secs`) ([phase-07b-governor-wall-clock-ceiling.md](phase-07b-governor-wall-clock-ceiling.md)) | done |
| 08 | Verifier `tsc` resolution (`node_modules/.bin` → `npx` → PATH) | todo (not drafted) |
| —  | ~~Wire `roots/list` corroboration~~ | **deferred** (rmcp 1.8.0 / SEP-2577) |

Phases are drafted **on demand** via `/rexymcp:architect next`; the rows above
are the milestone plan, not final specs. Numbering is the suggested dispatch
order (housekeeping first, smallest blast radius first). Phases 04–08 were
renumbered down by one when the roots phase (originally 03) was deferred
2026-07-07.

## Notes

### Kickoff decisions (2026-07-07, with the user)

- **Seeded from a review, not an e2e failure.** First milestone of this kind.
  The review doc is the backlog; the exit criteria above are the review's
  findings restated as verifiable conditions.
- **Phase-01 manifest unification assumption:** Antigravity consumes
  `plugin/plugin.json` as display metadata and keys the install on the
  *directory* the user copies/symlinks (per `README.md` § Google Antigravity),
  so renaming the manifest to `rexymcp` is safe and the install-path examples
  change with it. If review of phase-01 shows Antigravity binds to the manifest
  `name`, bounce the phase and revisit.
- **Phase-06 (was 07) — resolved at draft time with the user (2026-07-07).** The
  talk-through concluded **wire, split, and defer**: (1) `gate_retries` is wired
  into the M19 gate-retry loop this phase (real loop effect, honors `calibrate`'s
  intent, executor-internal and safe); (2) the escalation knobs (`escalation_slots`,
  `max_assists`/`EscalationConfig`) are **not** executor-internal at all — their
  natural consumer is the *architect-side* autonomous `/loop`, so they move to M27
  (see below). Retiring them now would be churn before M27 re-introduces them at the
  right layer; wiring them into the executor loop now would contradict the
  architecture non-goal *"rexyMCP never links a cloud provider"* and violate WORKFLOW
  § "Derive intentionally." Phase-06 corrects their stale "wired in M21" doc comments
  and otherwise leaves them untouched.
- **Phase-03 (was 04) shape:** an additive `warnings: Vec<String>` field on
  `PhaseResult` with `#[serde(default)]` (WORKFLOW § "Prefer additive change
  shapes"); no existing field changes meaning. Implemented at draft time by
  stamping the warnings onto the returned `PhaseResult` in `runner::run_phase_with`
  (the single choke point both the MCP and CLI paths route through), avoiding an
  11-site change to `PhaseInput` constructors.
- **Phase-07 (was 08) re-split at draft time (2026-07-08)** into **07a**
  (oscillation + windowed-output detectors — both governor tool-call-history pure
  detectors folded into the Step-7 hard-fail seam) and **07b** (the wall-clock
  ceiling — a clock-based `[budget]` terminal, a distinct mechanism). The combined
  estimate (~570 lines) exceeded one session; the two halves are different
  subsystems (governor detector vs. budget terminal). 07a is drafted; 07b is drafted
  on demand after 07a lands.

### Escalation budgeting moved to M27 (2026-07-07, with the user)

The phase-06 wire-or-retire talk-through opened a larger design thread. The user's
goal is an **autonomous Architect↔Executor virtuous cycle** — a fully autonomous
`/loop` where the executor escalates well and Claude (the architect) responds
without a human in the inner loop. That reframes what `[escalation] max_assists`
and `[budget] escalation_slots` are *for*: not executor-internal knobs, but the
budget for the **architect-side** autonomous loop that reads a returned briefing,
applies an escalate lever (re-dispatch / resume), and stops for the human after
`max_assists` round-trips.

Two decisions fixed this direction (both with the user):
- **Split.** Phase-06 lands the mechanical, executor-internal win (`gate_retries`)
  now. The autonomous escalation cycle becomes its own milestone, **M27 — Autonomous
  Escalation Loop**, which *starts with a design talk-through* and amends
  `docs/architecture.md`, the executor contract
  (`executor/templates/executor_contract.md`), and `WORKFLOW.md` before any executor
  phase is drafted.
- **Architect-side owner.** The executor stays a single-shot unit that returns a
  structured briefing (as it already does); the assist counter and the `/loop`
  driver live in the plugin/skill layer. This is consistent with the existing design
  (`architecture.md` § "Escalation = Claude Code itself", the escalate skill's three
  levers) and with the non-goal — rexyMCP never calls a cloud provider; Claude Code,
  already the architect, is the escalation target. M27 also naturally absorbs the
  queued **resume lever** (review §3.1) and **D8/D9 server-authored bookkeeping**
  (review §3.2), both of which were already flagged as needing a talk-through.

M27 is **not** kicked off yet — it is a human-gated milestone boundary. Recorded
here (not in `architecture.md`) because it is a milestone-scoping decision; the
`architecture.md` amendment happens at M27 kickoff.

### Roots corroboration deferred (2026-07-07, with the user)

The originally-planned phase-03 was to wire the client's real `roots/list` into
`execute_phase`'s corroboration (today `roots_list` is a hardcoded empty `Vec` at
`mcp/src/server.rs:490`, while the tool description claims roots/list
corroboration). At draft time the architect found that **rmcp 1.8.0** — the
resolved version — marks `Peer::list_roots` `#[deprecated(since = "1.8.0")]`
because MCP itself is removing the roots feature per **SEP-2577** ("Roots is
deprecated … and will be removed in a future release"). Wiring it would (a)
require `#[allow(deprecated)]`, which the hard rules forbid as masking a
diagnostic, and (b) build on a protocol feature slated for removal — the next
rmcp bump likely breaks it. The user chose to **defer** rather than adopt a
doomed API or drop the roots claim from the description now. Revisit if/when MCP
settles on a roots replacement. The `roots.rs` corroboration logic (already
complete and tested) and the env-var (`CLAUDE_PROJECT_DIR` /
`ANTIGRAVITY_PROJECT_DIR`) path stay as they are; only the aspirational roots
half is parked. Recorded here rather than in `architecture.md`'s roadmap because
it is a milestone-scoping decision, not a design change.

<!-- retrospective appended at milestone close -->
