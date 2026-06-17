# M20 — Tier Calibration and Cost Visibility

**Goal:** Give the user a single knob (`/calibrate LARGE|MEDIUM|SMALL`) that
configures how rexyMCP interacts with the local LLM — how much hand-holding the
Architect provides, how many retries the executor gets, and when mid-phase
Architect escalation fires. Track every real cost, including Architect assists,
so the dashboard shows honest savings, not optimistic ones.

**Status:** done (4/4 phases approved, 2026-06-16)

**Depends on:** M19 (gate-retry loop already in executor; M20 builds the config
layer on top of it)

## Why now

After M19, false_completion through a red gate is structurally impossible — the
executor self-corrects until gates pass or the turn budget is exhausted. The
next gap is that every model gets the same fixed parameters regardless of its
capability, and every phase doc gets the same level of detail regardless of
whether the model needs it. Architect assist costs (when Claude intervenes) are
also untracked, so the dashboard savings figure is incomplete.

M20 closes those gaps with three phases that each ship independently:

- **Phase-01** defines the config schema and `/calibrate` CLI — no executor
  behavior changes yet; the tier is recorded and parsed.
- **Phase-02** adds telemetry fields for tier, doc_level, and Architect assist
  cost so every run is fully instrumented.
- **Phase-03** surfaces the cost breakdown in the dashboard: phase outcome feed
  gains an escalation column; the summary gains an Architect-cost line separate
  from local cost; net savings is honest.

## Exit criteria

- `rexymcp calibrate MEDIUM --config rexymcp.toml` writes the correct tier and
  tier-derived budget defaults to `rexymcp.toml` and prints a confirmation.
- `[escalation]` and `[architect]` sections parse from an existing config file
  without error; their absence is backward-compatible.
- `ArchitectConfig` auto-fills rates for known Claude model IDs (shared registry
  with `DashboardConfig`).
- Telemetry `PhaseRun` records carry `tier`, `doc_level`, `escalation_count`,
  `architect_input_tokens`, and `architect_output_tokens`.
- A new `EscalationEvent` record is appended to the telemetry store each time a
  mid-phase Architect assist fires (M21 wires the firing; M20 defines the
  record).
- The dashboard budget summary shows local cost and Architect cost as separate
  line items; net savings subtracts both from the cloud baseline.

## Architecture references

- `executor/src/config.rs` — all config structs; the single file to extend in
  phase-01.
- `mcp/src/dashboard/panels.rs` — existing `model_rates` / `BudgetRates`; the
  known-model registry moves to `executor/src/config.rs` in phase-01.
- `mcp/src/main.rs` — clap `Commands` enum; gains a `Calibrate` variant in
  phase-01.
- `executor/src/store/telemetry.rs` — `PhaseRun`, `PhaseReview`; gains new
  fields in phase-02.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Config schema, `/calibrate` CLI, known-model registry ([phase-01-config-and-calibrate.md](phase-01-config-and-calibrate.md)) | done |
| 02 | Tier/cost telemetry fields and `EscalationEvent` record ([phase-02-telemetry-fields.md](phase-02-telemetry-fields.md)) | done |
| 03 | Dashboard cost breakdown (Executor/Architect/Net per scope, Assists counter) ([phase-03-dashboard-cost-breakdown.md](phase-03-dashboard-cost-breakdown.md)) | done |
| 04 | Documentation sync — README and architecture.md ([phase-04-documentation-sync.md](phase-04-documentation-sync.md)) | done |

## Notes

### Retrospective — 2026-06-16

**Outcome:** 4/4 phases done. Verdicts: phase-01 approved_first_try, phase-02
approved_after_2 (bug-02-1 then bug-02-2 — discriminator-test mutation
resistance), phase-03 approved_first_try, phase-04 approved_after_1
(false_completion on first dispatch). Executor Qwen/Qwen3.6-27B-FP8 throughout
(phase-01 implementation was Claude direct).

**What worked:**
- The phase-01→02→03 split (config schema → telemetry substrate → dashboard
  surface) kept each phase additive and independently shippable. The
  `#[serde(default)]` nested-struct pattern for `TierTelemetry` (mirroring
  `ContextEfficiency`) again made an ~11-site `PhaseRun` literal addition a
  one-line-per-site change — no literal cascade, no churn stall.
- Phase-04 as an explicit docs-sync phase (rather than folding doc edits into
  each implementation phase) paid off: one authorized `architecture.md`/`README.md`
  edit pass with exact pinned before/after text, all 10 passages landed verbatim.

**Calibration data (no folds yet — all 1st-occurrence or already-tracked):**
1. **`false_completion` on a docs/no-code phase (NEW class variant, phase-04).**
   The first dispatch reported `complete` with 2/10 tasks done. All four gates
   were green *by construction* — a docs phase has no code, so fmt/build/lint/test
   pass regardless of how many spec tasks were actually completed. **M19's
   gate-retry loop structurally cannot catch this**: it loops on a *red* gate, and
   there is no red gate to trip. This is a genuine blind spot in the M19 guarantee,
   surfaced for the first time here. 1 occurrence — **data, not a fold.** If it
   recurs on another no-gate-coverage phase (docs, config-only, comment-only), the
   candidate fix is a runtime "task-coverage" check (seeded-task completion before
   accepting `complete`) or an architect-side rule to keep no-code phases tiny and
   always review-diff task-by-task. Flagged for the user.
2. **`false_completion` remains the dominant cross-milestone class** (carried from
   M18/M19). M19 resolved the *red-gate* variant structurally; phase-04 shows the
   *no-gate-coverage* variant is still open. Worth a talk-through with the user.
3. **`bounced` twice on phase-02 over the same discriminator-test mutation-resistance
   issue** (bug-02-1, bug-02-2). Consistent with the M18 bug-01-1 lesson (pin the
   `.record` filter as load-bearing). Already a tracked pattern; pre-injection of
   the guard-test lesson held for the eventual fix.

**Pending fold decisions carried into M21 kickoff (need user sign-off):** the
no-gate-coverage `false_completion` variant (item 1) and the still-open dominant
`false_completion` class (item 2). Neither folded yet.
