# M18 — Capability-Aware Adaptation

**Goal:** rexyMCP systematically characterizes each local model's strengths and
failure modes and compensates for them — at draft time and at runtime — instead
of relearning them by per-phase trial-and-error.

**Status:** done (all in-scope phases 01–07 approved; phase-08 shelved)

**Depends on:** M7 (scorecard / `PhaseRun` telemetry), M12 (`task_tracking`,
configurable governor thresholds)

## Why now

The architect already characterizes a model's strengths and weaknesses every
time it dispatches, reviews, and re-dispatches a phase. But that learning has
**nowhere durable to land**: it lives in phase-doc Update Logs and milestone
retrospectives as prose, in the architect's working memory, and in NEXT.md
calibration notes. The next phase starts from near-zero. That is the
"trial-and-error" this milestone replaces with a data loop:
**characterize → compensate → measure → refine.**

### The foundational gap this milestone fixes first

The supervision half of the eval loop is **not wired**. The executor appends
every `PhaseRun` with `warnings` / `bugs_filed` / `bounces_to_approval` /
`architect_verdict` hardcoded to `None` (`executor/src/agent/metrics.rs:121-124`),
the telemetry store is **append-only** (`executor/src/store/telemetry.rs:161`),
and **no code path ever writes the architect's verdict back** — there is no
`rexymcp review` subcommand, no annotate tool, nothing. The architecture says
the architect fills these "later" (`telemetry.rs:5`); no code does. So the M7
scorecard aggregates supervision columns that are always empty, and the
"trial-and-error" cannot compound because the labels never reach the store.

Every M18 thread consumes the supervision label (verdict + the new
failure-class). So phase-01 builds the **write-back substrate** first; the rest
build on it.

## Exit criteria

- A `rexymcp review` CLI records an architect verdict — including a structured
  **failure-class** — as an append-only `PhaseReview` annotation; the
  read/aggregation layer folds the latest annotation onto its matching
  `PhaseRun`, so supervision columns in `runs` / `scorecard` / `model_scorecard`
  carry real data.
- A `model_profile` view aggregates folded runs into per-`(model, tag)`
  strengths (first-pass rate, reliability means) **and** ranked failure classes
  with counts — queryable by the architect at draft time.
- At least one runtime knob (`task_tracking`, a governor threshold, router
  breadth, or sampling) resolves a **per-model** default derived from config,
  overriding the global default.
- Thread 4 (cold-start calibration battery) is explicitly **deferred** — shelved
  for later revisit, not in M18's committed scope (with the user, 2026-06-13).

## Architecture references

- `docs/architecture.md#model-effectiveness-metrics--the-scorecard` — the
  `PhaseRun` record, the supervision label, the `model × tag` matrix.
- `docs/architecture.md#layer-2--mcp-crate-binary` — `model_scorecard`, the
  `rexymcp runs` / `rexymcp scorecard` CLIs.
- `docs/architecture.md#layer-3--plugin-package` — pre-injection as the
  architect's primary capability-transfer channel (thread 2 feeds it).
- `docs/architecture.md#configuration` — the config that thread 3 makes
  per-model.

## Threads → phases

Four threads, sequenced so each builds on the prior's data. The substrate
(thread 1) must land first.

**Thread 1 — Supervision write-back substrate (the eval loop).**
Capture the architect's verdict + a structured failure-class, durably, in the
telemetry store.

**Thread 2 — Capability profile + draft-time aid.**
Fold the now-populated runs into a per-model competency *and* weakness profile;
surface it to the architect at draft time so pre-injection is data-driven.

**Thread 3 — Model-conditioned runtime knobs.**
Make `task_tracking` / governor thresholds / router breadth / sampling resolve
per-model defaults from config instead of global constants.

**Thread 4 — Cold-start calibration battery (SHELVED).**
A fixed known-answer probe set, run once to seed a new model's profile. This is
**active** measurement and departs from the architecture's "passive production
telemetry" principle. **Deferred for later revisit** (with the user,
2026-06-13) — not in M18's committed scope. When picked up it needs a
talk-through and a `docs/architecture.md` precedence decision before its phase is
drafted. See Notes.

## Phases

| #  | Phase | Thread | Status |
|----|-------|--------|--------|
| 01 | review write-back substrate (`PhaseReview` annotation + fold-on-read + failure-class taxonomy) ([phase-01-review-substrate.md](phase-01-review-substrate.md)) | 1 | done |
| 02 | `rexymcp review` CLI + fold into runs/scorecard reads + skill wiring ([phase-02-review-cli-fold.md](phase-02-review-cli-fold.md)) | 1 | done |
| 03 | `model_profile` aggregation (strengths + ranked failure classes) ([phase-03-model-profile.md](phase-03-model-profile.md)) | 2 | done |
| 04 | surface the profile to the architect (MCP tool + `rexymcp profile` CLI) ([phase-04-profile-surface.md](phase-04-profile-surface.md)) | 2 | done |
| 05 | per-model config-override resolution layer ([phase-05-model-config-overrides.md](phase-05-model-config-overrides.md)) | 3 | done |
| 06 | wire the per-model override resolution into the dispatch path ([phase-06-wire-model-overrides.md](phase-06-wire-model-overrides.md)) | 3 | done |
| 07 | executor tooling improvements (`write_file` append, `search` context, `find_files` depth, `patch_lines`/`delete_file`/`move_file`) ([phase-07-tooling-improvements.md](phase-07-tooling-improvements.md)) | cleanup | done |
| 08 | cold-start calibration battery | 4 | **shelved — revisit later** |

All in-scope phases (01–07) are `done`. Phase-08 (thread 4, cold-start
calibration battery) remains **shelved** — out of M18's committed scope until
the user revisits it (needs a talk-through + `architecture.md` precedence
decision first).

## Notes

### The failure-class taxonomy (thread 1)

The structured failure-class is the field that turns "it bounced" into "it
bounced *because* X" — the difference between data and noise. The starting
vocabulary is drawn from the recurring classes already named in NEXT.md and the
milestone retrospectives:

- `false_completion` — self-reported `complete` on a red gate (the recurring
  self-report-vs-gate-exit disagreement; M17 phase-07/-09, M12 phase-06).
- `prod_unwrap` — `.unwrap()`/`.expect()` in a production path (STANDARDS §2.1;
  the M12 watch-item, held at 2 occurrences).
- `multi_site_break` — a breaking multi-site type change ran out of verifier
  runway (the signature/variant-churn stall; M7 phase-05b fold).
- `parse_format` — tool-call format / forgiving-parser repair churn.
- `masked_diagnostic` — `#[allow]`/`#[ignore]` used to hide a warning/error.
- `scope_deviation` — touched out-of-scope files or widened scope.
- `spec_bug` — **the bounce was the architect's spec fault, not the model's.**
  Load-bearing: keeps the profile from blaming a model for a mismatched-
  expectations bug (WORKFLOW § "The executor is a local LLM": those are spec
  bugs).
- `infra_blip` — transient backend/decode error, not a work defect (M17
  phase-04). Also kept out of the model's competency signal.
- `none` — clean approval.

The vocabulary is an **extensible documented list** (a `Vec<String>` validated
against a canonical const), not a closed enum — new classes fold in as they
recur, per the WORKFLOW calibration discipline. `spec_bug` / `infra_blip` exist
so weaknesses are attributed honestly: a model is judged on what *it* got wrong.

### Thread 4 precedence question (shelved — for when it is revisited)

`docs/architecture.md` § "Model effectiveness metrics & the scorecard" states all
data is **passive production telemetry — a byproduct of normal use, not a
separate benchmark apparatus**, and Non-goals reinforce it. A calibration probe
battery is a deliberate benchmark apparatus. Per source-of-truth precedence
(`docs/architecture.md` > phase doc), phase-07 cannot be drafted as if this is
settled. The decision — amend the principle (and how: a bounded, opt-in,
clearly-separated "calibration" namespace that doesn't pollute the production
matrix?) or defer thread 4 entirely — is a human call to be talked through at
the thread-3/thread-4 boundary, not assumed here.

### Retrospective — 2026-06-15

**Outcome:** M18 closed with all 7 in-scope phases approved; thread 4 (phase-08)
shelved by design. The eval loop the milestone set out to wire is live
end-to-end: `rexymcp review` records a structured verdict + failure-class
(phase-01/02), `model_profile` aggregates folded runs into per-`(model, tag)`
strengths and ranked failure classes (phase-03/04), and per-model config
overrides resolve into the live dispatch path (phase-05/06). Phase-07 was a
cleanup-thread tool-surface expansion (append/context/depth + `patch_lines`/
`delete_file`/`move_file`).

**What worked:** the substrate→consumer phase splits (01→02, 03→04, 05→06) kept
each phase to one session and isolated the load-bearing types from their wiring.
Five of seven phases approved first-try.

**What broke — two bounces, both `false_completion`:**
- phase-03 bug-03-1 (`false_completion` + `spec_bug`): self-reported complete on
  red fmt+clippy; the spec_bug was the lib-vs-bin dead-code asymmetry (`mcp` is a
  binary crate where unused `pub` is denied).
- phase-07 bug-07-1 (`false_completion` + `prod_unwrap`, blocker): reported
  complete with ~half the phase undone — three new tools were orphan files never
  wired into `mod.rs`/`router.rs`/`runner.rs`, so their tests never compiled and
  the green "766 passed" was false confidence; fmt was red; two prod `unwrap`s in
  `search.rs`. Re-dispatch cleared all three findings in one 68-turn pass.

**Calibration — two folds pending user sign-off:**

1. **`prod_unwrap` hits its 3rd occurrence** (M12 watch-item: M12 phase-05 parse,
   M12 phase-06c Mutex lock, now M18 phase-07 F3). Per WORKFLOW "three = fold."
   The watch-item's prescribed fix is a forward-looking gotcha pre-injected into
   any `Mutex`/lock or slice/`.first()`/`.last()` phase doc — **not** a STANDARDS
   edit (the gate text already forbids it; the gap is application). Held for sign-off.

2. **`false_completion` is now the dominant recurring class** across M12/M17/M18
   (self-report vs gate-exit disagreement). Rather than only pre-inject against
   it per-phase, **M19 (Structural Gate Enforcement) is drafted** to make it
   structurally impossible: the executor runtime runs the DoD gates at completion
   and loops on any red gate instead of trusting the model's self-report. This is
   the higher-leverage fix and directly serves the cost goal (less architect
   re-review labour, more work landing on the local model first-try). M19 phase-01
   is staged at `docs/dev/milestones/M19-gate-enforcement/`.

**Note on the architect-labour anti-pattern (raised by the user this milestone):**
pre-injection is mitigation, not cure — it scales linearly with phases and keeps
the verification burden on the architect. M19 moves that burden into the runtime
where it belongs, so the local LLM does the bulk of the work and the architect
reviews a structurally-trustworthy `complete`. Keep this framing for M19 review.
