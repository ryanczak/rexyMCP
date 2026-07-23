# M36 — Budget Truth Pass

**Goal:** Make the Budget surface answer one question honestly — *what is
consuming tokens?* — by counting the architect spend that is currently invisible
and dropping the "Baseline" framing that reads as a cost but isn't.

**Status:** in-progress *(opened 2026-07-23)*

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
| 03 | Rename the `other` bucket to `architect chat` ([phase-03-architect-chat-bucket.md](phase-03-architect-chat-bucket.md)) | review |

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