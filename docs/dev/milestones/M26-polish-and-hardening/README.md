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
| Housekeeping / corrections | 01–04 | contract docs, plugin manifests, `mcp` crate seams |
| Executor-loop hardening | 05–09 | tools gates, hooks, budget knobs, governor, verifier |

## Exit criteria

- `REXYMCP.md` no longer asserts a milestone frontier (must NOT match:
  `Active work is M7`, `clap CLI exposing`) and describes the `mcp` crate as the
  `rmcp` stdio server + CLI it actually is.
- One plugin identity: `plugin/plugin.json`, `plugin/.claude-plugin/plugin.json`,
  and `.claude-plugin/marketplace.json` `plugins[0]` agree on name `rexymcp` and
  description; no `rexymcp-plugin` reference remains in `README.md` or `plugin/`.
- `rexymcp run-phase` writes a `PhaseRun` record by default when telemetry is
  configured; a `--no-telemetry` flag suppresses it.
- `execute_phase` corroborates `repo_path` against the client's **real**
  `roots/list` when the client advertises roots; the tool description matches
  actual behavior (today `roots_list` is hardcoded empty at
  `mcp/src/server.rs:490` while the description claims corroboration).
- A missing `docs/dev/STANDARDS.md`, an unwritable session-log directory, and a
  phase doc that parses to an empty goal/acceptance-criteria each produce an
  architect-visible warning instead of silence.
- `write_file` to an existing file the executor has not read (or whose mtime
  changed since the read) is refused with the read-first advisory, mirroring
  `patch`.
- The post-write format hook actually rewrites a deliberately misformatted
  touched file (today it runs the verify-only `--check` form — a no-op).
- The `[budget] gate_retries` / `escalation_slots`, `[executor] tier`, and
  `[escalation] max_assists` knobs are either consumed by the loop or removed
  from config + `calibrate`; no knob `calibrate` writes is a silent no-op.
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
| 02 | `run-phase` telemetry parity (`--no-telemetry` opt-out) ([phase-02-run-phase-telemetry-parity.md](phase-02-run-phase-telemetry-parity.md)) | review |
| 03 | Wire `roots/list` corroboration in `execute_phase` | todo (not drafted) |
| 04 | Surface silent degradations as architect-visible warnings | todo (not drafted) |
| 05 | `write_file` read-before-edit gate | todo (not drafted) |
| 06 | Post-write format hook: writing form on touched files | todo (not drafted) |
| 07 | Wire or retire the dead budget/tier knobs | todo (not drafted) |
| 08 | Governor blind-spot detectors (oscillation, windowed output, wall clock) | todo (not drafted) |
| 09 | Verifier `tsc` resolution (`node_modules/.bin` → `npx` → PATH) | todo (not drafted) |

Phases are drafted **on demand** via `/rexymcp:architect next`; the rows above
are the milestone plan, not final specs. Numbering is the suggested dispatch
order (housekeeping first, smallest blast radius first).

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
- **Phase-07 is wire-*or*-retire, decided at draft time with the user.** Wiring
  honors `calibrate`'s intent (tier-derived `effective_max_turns` /
  `gate_retries` bounding the loop); retiring shrinks config surface but guts
  `calibrate`. The choice touches `[escalation]`/`[architect]` semantics from
  M20, so it gets a talk-through before the phase doc is written.
- **Phase-04 shape:** prefer an additive `warnings: Vec<String>` field on
  `PhaseResult` with `#[serde(default)]` (WORKFLOW § "Prefer additive change
  shapes"); no existing field changes meaning.
- **Phase-08 may re-split** (08a detectors / 08b wall-clock ceiling) at draft
  time if the combined diff estimate exceeds one session.

<!-- retrospective appended at milestone close -->
