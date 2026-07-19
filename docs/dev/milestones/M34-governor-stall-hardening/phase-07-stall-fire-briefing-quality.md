# Phase 07: Stall-fire briefing quality

**Milestone:** M34 — Governor Stall Hardening
**Status:** todo (planned — full spec drafted on a later `/rexymcp:architect next`)
**Depends on:** phase-03 (the stall signals), phase-05 (advisory-demotion, which
narrows this phase's scope — see below)
**Estimated diff:** ~120 lines (estimate — firmed at drafting)
**Tags:** language=rust, kind=feature, size=m

> **Planned stub.** Records the design intent so the phase-table link resolves and
> the boundaries are fixed. Not yet a dispatch-ready spec.
>
> **Scope narrowed by phase-05 (advisory-demotion).** Once `LowNoveltyStall` is
> advisory-by-default it no longer terminates, so "make the *fired* stall briefing
> actionable" now applies only to (a) the still-terminating raw `NoProgressStall`
> backstop and (b) any detector explicitly configured back to `Terminate`. This
> phase is deliberately sequenced **after** demotion (05) and the metrics/calibration
> work (06) so its final scope reflects which detectors still terminate. Revisit
> whether it's still worth doing once 05/06 land.

## Goal

When a stall fires, make the escalation briefing **actionable**. Today the
briefing's "Current blocker" section renders only `signal.describe()` — e.g.
*"only 2 distinct read targets across the last 24 read-only calls (low-novelty
churn)"* — which tells the architect *that* churn happened but not *which* targets
the executor kept re-probing. Name the churned targets / the trailing read-only
run so escalation lands on the actual stuck spot instead of re-deriving it.

## Current state (to expand at drafting)

- `executor/src/phase/briefing.rs` `Briefing::render` (line ~99) renders
  `Blocker::HardFail(signal)` as bare `signal.describe()`.
- `HardFailSignal::LowNoveltyStall { window, distinct_targets }` carries counts
  but **not** the target strings.
- `normalize_target` (`hard_fail.rs:302`) is the target-key logic (currently
  private; phase-04 does not change its visibility).

## Design fork (resolve with the user at drafting)

Two ways to get the churned target names into the briefing:

- **(a) Carry them on the signal** — `LowNoveltyStall { window, distinct_targets,
  targets: Vec<String> }`. Self-describing; but mutates a serialized
  (`JsonSchema`) enum variant and touches `describe()` + every construction/test
  site (a bounded but real cascade).
- **(b) Recompute in the briefing** — make `normalize_target` (or a small
  `distinct_targets(recent, window)` helper) `pub`, and have `render` (or the
  briefing builder) derive the target list from `recent_tool_calls` when the
  blocker is a stall. Keeps the signal lean; the briefing already receives
  `recent_tool_calls`.

Lean-additive preference (STANDARDS § "Prefer additive change shapes") leans
toward **(b)**, but confirm with the user before drafting.

## Scope boundaries (fixed now)

**In scope:** enriching the rendered briefing for `LowNoveltyStall` and
`NoProgressStall` blockers with the churned targets / read-only run summary.

**Out of scope:**
- Changing when stalls fire (detector behavior is frozen by phases 01/03).
- The observability sample (phase-04).
- Reworking the other five briefing sections or the `AttemptSummary` list shape.

## Update Log

<!-- entries appended below this line -->
