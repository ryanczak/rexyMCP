# M36 — Budget Truth Pass

**Goal:** Make the Budget surface answer one question honestly — *what is
consuming tokens?* — by counting the architect spend that is currently invisible
and dropping the "Baseline" framing that reads as a cost but isn't.

**Status:** done *(opened and closed 2026-07-23)*

**Depends on:** M35 (this revises M35's `costs` / Budget-panel output based on
first real use)

## Why this milestone exists

M35 shipped the accounting. Using it surfaced three defects in the *presentation*
and one in the *counting*:

1. **`Baseline` reads as a spend but is a counterfactual.** `costs.rs:57-64`
   prices executor tokens at cloud rates — money *not* spent. It leads the table
   and the dashboard block is titled `Savings`, so the first number the user sees
   is hypothetical. The user's framing, which this milestone adopts: **Architect
   is the only debit; Executor token usage is not a cost, it is a saving.**
2. **`other` is 18.7 % of architect spend and has an uninformative name.**
   Verified across all 61 project transcripts: it is untagged Claude work —
   both whole non-skill sessions and the user↔architect conversation between
   phase runs. Both are architect spend. It should be named for what it is.
3. **Subagent token usage is never harvested.** `harvest.rs:201` and
   `sweep.rs:51` both use a non-recursive `read_dir`. Claude Code writes
   `Agent`-tool subagent transcripts to `<session-id>/subagents/*.jsonl`, one
   directory down. Measured on this project: **36 files, 1,133 messages,
   59.6 M tokens uncounted** — ~10 % of spend in `/rexymcp:auto` sessions, which
   is exactly where it matters, since `plugin/skills/auto/SKILL.md:76-82`
   delegates dispatch and review to subagents by design.

**Explicitly investigated and ruled out: there is no double-counting.**
`attributionSkill` is single-valued per message, messages dedup by `message.id`,
`isSidechain` is `false` on all 20,464 lines, and there are **zero** nested
`Skill` invocations anywhere in the corpus. `rexymcp:auto` is disjoint from
`dispatch`/`review`/`escalate`; the per-skill rows sum to the project total
exactly once. No phase in this milestone changes `auto` accounting.

## Exit criteria

- No user-visible surface says "Baseline". `rexymcp costs` and the dashboard
  Budget panel present **Executor** and **Architect** as the two buckets, with
  the counterfactual reframed as **Saved** — a property of the Executor row, not
  a leading cost line. `Net` survives as the bottom line.
- The `other` architect bucket displays as **`architect chat`** everywhere it
  is rendered (`costs` table and the dashboard top-skill hint).
- `rexymcp harvest` and the `serve` sweep read `<session>/subagents/*.jsonl`,
  folding subagent usage into its parent session's bucket, and **not** any other
  session subdirectory (`tool-results/`).
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #35 — the accounting this milestone revises.
- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — the exit
  criterion this milestone supersedes ("`rexymcp costs` reports Baseline /
  Executor / Architect / Net").

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Harvest subagent transcripts ([phase-01-subagent-transcript-harvest.md](phase-01-subagent-transcript-harvest.md)) — approved_first_try; +36.1M tokens recovered after dedup | done |
| 02 | Budget reframe: Baseline → Executor `saved` ([phase-02-budget-reframe-saved.md](phase-02-budget-reframe-saved.md)) — approved_first_try; the no-additive-shape rename landed first try in 167 turns | done |
| 03 | Rename the `other` bucket to `architect chat` ([phase-03-architect-chat-bucket.md](phase-03-architect-chat-bucket.md)) — approved_first_try; the fold test pins the map-at-the-key design | done |

Phase 01 is deliberately standalone: it is the only phase that changes a
*number* rather than a label, and it needs its own tests (nested-dir discovery,
session-id derivation, negative-path exclusion, re-harvest idempotency).
Phases 02 and 03 are independent of each other; 03 is sequenced after 01 only to
avoid two phases editing `harvest.rs`-adjacent display code concurrently.

## Notes

**Both opening prerequisites are cleared (2026-07-23):**

- `docs/architecture.md` § Status now carries an **M36** entry (§36), and M35's
  §35 entry is marked done with the Baseline framing explicitly superseded.
- **M35 is closed** (retrospective + calibration folds written; two folds
  landed in `WORKFLOW.md`, two deferred to M37, two on the watch list). This
  milestone did not reopen it — the two accounting gaps M36 fixes predate M35
  and are recorded in its retrospective as found-at-close, not as regressions.

No phase in this milestone is authorized to edit `docs/architecture.md`.

**Design decision (phase 03):** the `other` → `architect chat` rename is
**display-layer only**. The stored ledger key stays `other`, so no migration is
needed and no already-harvested record goes stale. See phase-03 § Current state.

---

## M36 retrospective (close: 2026-07-23)

**All three phases `approved_first_try`, zero bounces, zero oscillations, one
working day.** Every exit criterion met and verified against the real binary,
not just unit fakes.

| # | Phase | Turns | Outcome |
|---|---|---:|---|
| 01 | subagent-transcript harvest | 58 | +36.1M tokens recovered |
| 02 | budget reframe `baseline` → `saved` | 167 | the no-additive-shape rename, first try |
| 03 | `other` → `architect chat` | 38 | fold test pins the design decision |

### What the milestone actually fixed

The Budget surface now answers "what is consuming tokens?" without a
counterfactual leading the table. **Architect is the only debit; executor token
usage is a saving.** `rexymcp costs` reads:

```
SCOPE         EXECUTOR ARCHITECT       NET     SAVED
Project          $0.00  $1773.03  $-497.84  $1275.19

SAVED = executor tokens priced at Claude rates — work not billed to Claude.
NET   = SAVED − EXECUTOR − ARCHITECT.

By skill (architect)
rexymcp:dispatch        955.2M   $736.60   41.5%
architect chat          383.5M   $339.93   19.2%
rexymcp:review          358.2M   $304.45   17.2%
rexymcp:architect       222.0M   $237.48   13.4%
rexymcp:escalate        134.2M   $117.89    6.6%
rexymcp:auto             38.1M    $34.63    2.0%
```

Every token is now in a bucket a human can act on, and the two accounting holes
are closed: subagent transcripts are harvested (they were ~10 % of spend in
`/rexymcp:auto` runs and 100 % invisible), and the largest unnamed bucket has a
name.

### Calibration

**Confirmed working — the leaf-first fold, 3rd occurrence and the first
*preventive* one.** WORKFLOW § "Prefer additive change shapes" says a
multi-site mutation with no additive alternative needs a grep-verified site
list in leaf-first order with per-file build checkpoints. Phase-02 was exactly
that shape — a public struct field across ~14 production and ~56 test sites,
where the crate stops compiling the instant the field changes. **Both prior
phases of this shape hard-failed before landing** (M30 phase-03: 2 hard_fails →
session takeover; M31 phase-02: hard_fail at 6 strikes → refined re-dispatch).
This one landed first try in 167 turns. The previous two occurrences proved the
countermeasure *repairs* the failure; this is the first evidence it *prevents*
it. No doc change needed — the fold is already written; this is the data point
that says leave it alone.

**Watch list — two architect-side lessons, 1× each, not folded.** Both are the
same family: *the architect asserted a fact in a spec without deriving it from
the tool that defines it.* Fold into WORKFLOW § "Specs pin behavior" if either
recurs.

1. **A corpus measurement quoted without its dedup state.** Phase-01's spec
   claimed 59.6M uncounted tokens; the real recovery was 36.1M, because the
   harvester dedups globally by `message.id` (6,069 duplicates that run) and my
   figure was a raw pre-dedup count. The fix worked correctly — the *number in
   the spec* was wrong. When quoting a corpus measurement, state whether it is
   pre- or post-dedup, and prefer measuring through the same path the code
   takes.
2. **A file list written from memory rather than from the defining grep.**
   Phase-02's task list named `costs.rs`, `panels.rs`, `README.md` — but its own
   acceptance criterion was a repo-wide grep for `Baseline`, which also hits
   `main.rs`'s clap `about` string. The executor found it and fixed it, correctly
   treating the criterion as authoritative over the task list. When an acceptance
   criterion *is* a grep, derive the file list by running that grep at draft time.

**Filed to M37 phase-05 — server-authored completion bookkeeping, now 6
occurrences and 3 distinct defects.** All three M36 phases reproduced the two
known ones (acceptance criteria left unticked; no `End-to-end verification`
block), and phase-03 surfaced a third: the `Executor:` line is written from the
model's **self-report**, and phase-03's entry claims `Claude Sonnet 4.5` on a run
that `rexymcp.toml`, `executor_health`, and `PhaseRun.model` all record as
`Qwen/Qwen3.6-27B-FP8`. Cosmetic — every aggregator reads the config-derived
telemetry field — but the phase doc is the human-readable record a retrospective
is read from. Not an executor defect and not fixable by re-dispatch; the
executor no longer owns that output. Each of the six reviews absorbed it by
verifying and ticking manually.

### Executor performance — a new model identity

M36 is the first milestone on `Qwen/Qwen3.6-27B-FP8`; every M35 phase ran as
`AEON-7/Qwen3.6-27B-AEON`, so the scorecard forked at the M35/M36 boundary.

```
MODEL                       N  GATES  AFT_RATE  TURNS_MEAN  VERIF_RET  PEAK_CXT
AEON-7/Qwen3.6-27B-AEON    38   0.71      0.54      136.50       3.76       33%
Qwen/Qwen3.6-27B-FP8        3   1.00      1.00       87.67       4.33       21%
```

**Read this cautiously: N=3.** Three runs at a 100 % approved-first-try rate is
encouraging, not evidence — the AEON row has 38 runs behind it, and M36's phases
were also *smaller and better-specified* than M35's dashboard-TUI work, which is
where most of AEON's bounces came from. The honest conclusion is "no reason for
concern after the model swap, revisit at N≥15," not "the FP8 build is better."
The one number that is suggestive rather than noisy is **peak context 21 % vs
33 %**, which is a property of phase size more than of the model.

Notably, **zero oscillations across all three phases** — including phase-02's
`panels.rs` work, the file that oscillated 4× during M35. That is consistent
with the M35 fold (pre-inject compiler-error-driven recovery; pin the fixture
that makes the row appear) doing its job, though M37 phase-01's read-only
exemption is still the durable fix.

### Nothing deferred

M36 closes with no carried debt. The three follow-ups it generated
(`Executor:` line, and the two watch-list items above) are filed to M37
phase-05 and this Notes section respectively.
