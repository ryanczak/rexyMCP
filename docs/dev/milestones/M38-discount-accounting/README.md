# M38 — Discount Accounting

**Goal:** Present rexyMCP's actual value proposition — executor tokens are a
**discount** on architect token cost — as a two-line ledger with a net, priced
from a single Claude rate.

**Status:** done *(opened 2026-07-23, closed 2026-07-24)*

**Depends on:** M36 (this corrects the surface M36 shipped)

## Why this milestone exists

M36 reframed the Budget panel so Architect is the only debit and executor token
usage is a saving. Using it surfaced that the reframe stopped one step short of
the actual idea, and that the number it depends on is sourced from the wrong
place.

**The product's premise, stated plainly:** work the executor does is work Claude
was not billed for. So executor tokens have a dollar value, and that value is
**the architect model's rate** (`[architect] model`). rexyMCP's "costs" are
therefore *discounts* on what the architect model would have charged. There is
only ever **one** Claude rate in play, and it should serve both sides of the
equation — what Claude cost, and what Claude did not cost.

Three defects follow from that:

1. **The rate comes from a duplicate table.** `costs.rs:223-224` and
   `main.rs:882` read `cfg.dashboard.effective_rates()` — the
   `[dashboard] saved_input_per_mtok` / `saved_output_per_mtok` /
   `saved_model` knobs — while architect cost reads
   `cfg.architect.rates_for(..)`. Two independently-configured prices for the
   same fact. They agree in this repo only by coincidence: `saved_*` is
   hand-set to `5.0`/`25.0` and `[architect] model = "claude-opus-4-8"` also
   resolves to `(5.0, 25.0)`. Change `[architect] model` to `claude-fable-5`
   ($10/$50) and the discount silently reports at half its true value.

2. **It is dark out of the box.** `init.rs:78-83` scaffolds every `[dashboard]`
   rate commented out, so `DashboardConfig::default()` gives `0.0`/`0.0`,
   `no_saved_rates` is true, and a fresh project renders `SAVED —` and `NET —`
   in every scope. The single number that justifies the product is invisible
   until the user hand-configures a table duplicating one they already filled in.

3. **The layout hides the executor entirely.** `debit_row`
   (`panels.rs:682-691`) suppresses any row that is `$0.00` or `—` in every
   scope — and the Executor row is `$0.00` in every scope for an unpriced local
   model, always. So the executor is suppressed on the dashboard, and in the CLI
   its contribution appears only as a far-right `SAVED` column not visually tied
   to it. Executor **token counts** are computed into `ScopeCosts` and then
   discarded: they reach neither `format_costs` nor `--json`.

## The target shape

Debits in parentheses, credits plain — the accounting convention carries the
semantics, so no `SAVED`/`avoided` label is needed. The dashboard's existing
`b` key toggles the two modes; `rexymcp costs` gets a `--tokens` flag to mirror
it, dollars by default.

```
Budget            Session  Milestone     Project      Budget (tok)   Session  Milestone   Project
  Architect:          (—)        (—)  ($1781.15)        Architect:         —          —      1.5B
  Executor:         $2.96     $46.47    $1275.19        Executor:          —       9.1M    252.6M
  Net:                  —          —    ($505.95)       Net:               —          —         —
```

The dollars column is a real ledger that adds up:

```
Executor    $1275.19   credit — Claude cost avoided (executor tokens × [architect] rate)
Architect  ($1781.15)  debit  — Claude cost incurred
Net         ($505.95)  = sum
```

This replaces today's three-term `NET = SAVED − EXECUTOR − ARCHITECT`, which
needs a legend to explain. Under the new layout Net is the two rows added and
the parens carry the signs. It also absorbs the priced-executor case with no
special-casing: the Executor row carries `avoided − local cost`, so it stays one
number that renders as a credit normally and flips to a parenthesised debit if
local inference ever costs more than it saves.

## Exit criteria

- The discount rate derives from **`[architect].effective_rates()`**. No code
  path reads a separate `[dashboard]` rate; `DashboardConfig` is gone.
- A project that sets only `[architect] model` gets a populated Budget report —
  no second rate table to configure, nothing dark by default.
- Both surfaces render **Architect / Executor / Net**, debits parenthesised,
  negative `Net` parenthesised, from **one** implementation — the CLI and the
  dashboard cannot drift.
- The Executor row renders and is never suppressed: token counts in tokens mode,
  the avoided-cost credit in dollars mode.
- `rexymcp costs --tokens` shows the token view; dollars is the default.
  `--json` carries executor token counts.
- All four gates green.

## Architecture references

- `docs/architecture.md` § Status #38 — this milestone's design summary.
- `docs/architecture.md` § Status #36 — the surface this corrects.
- `docs/dev/milestones/M36-budget-truth-pass/README.md` § retrospective — the
  incomplete exit criteria that let this through.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Single rate source: derive the discount from `[architect]`, remove `DashboardConfig` ([phase-01-single-rate-source.md](phase-01-single-rate-source.md)) — approved_after_1 (bug-01-1 verified; mutation-checked by reviewer) | done |
| 02 | Ledger layout + `--tokens`: collapse SAVED into the Executor row, shared renderer ([phase-02-ledger-layout-shared-renderer.md](phase-02-ledger-layout-shared-renderer.md)) — approved_after_2 (bug-02-1 major, bug-02-2 minor; both verified, 5 reviewer mutations) | done |

Split deliberately. Phase 01 is a config-removal cascade across 5 files but
touches no rendering. Phase 02 rewrites `savings_lines` — the file that
oscillated 4× during M35 — and extracts a shared renderer. Keeping them apart
follows the M35 07d/07e/07f lesson: small focused phases beat one big risky one
in the TUI-adjacent code. **Dispatch 01 first**; 02 depends on it (it renders
the rate 01 rewires).

## Notes

**Provenance.** Found 2026-07-23 immediately after the M36 close, by the user
running `rexymcp costs` and observing that executor savings were not shown. The
first diagnosis (mine) was that a token line was missing; the user corrected the
framing — executor tokens are valued at the architect model's rate, making
rexyMCP's costs *discounts* — which is what exposed the duplicate rate table and
the dark-by-default behavior. The layout collapse (`()` carries the semantics,
so `SAVED`/`avoided` is redundant; use the existing `b` toggle for units) is also
the user's.

**This is M36's incomplete exit criteria, not a regression.** M36 delivered what
it promised; what it promised was missing a requirement stated in the design
conversation that preceded it ("it must track Executor token usage"). Recorded
in M36's retrospective watch list as a third instance of the same architect-side
family — asserting or specifying without deriving from the source of truth.

**Sequencing against M37.** M37 (governor read-only calibration) is planned and
unstarted; it remains a human gate. M38 leads because it is user-facing and the
user raised it. The two are independent — M37 is `executor/governor`, M38 is
`mcp/` display and `executor/config`.

---

## M38 retrospective (close: 2026-07-24)

**Two phases, five dispatches, three bounces.** That ratio is the milestone's
real story, and it is not an executor story — see § Calibration.

### What shipped

`rexymcp costs` and the dashboard Budget panel now state rexyMCP's premise
directly: executor tokens are work Claude was not billed for, priced at the
architect model's rate, so the report is a **discount ledger**.

```
Spend          Session Milestone   Project
  Architect:     (—)       (—)  ($1866.23)
  Executor:      $3.54   $111.53  $1386.72
  Net:             —         —   ($479.50)

Executor = Claude cost avoided at [architect] rates; ( ) = debit.
```

- **One Claude rate.** `DashboardConfig` is gone; the discount derives from
  `[architect].effective_rates()`. The duplicate table could silently halve the
  reported discount by changing one config key, and did not agree with
  `[architect]` by design — only by coincidence.
- **Not dark by default.** `rexymcp init` no longer scaffolds a second rate
  table. Setting `[architect] model` — needed anyway — lights the whole report.
- **Parens carry the semantics.** No `SAVED` column, no `avoided:` row: a debit
  is parenthesised, a credit is plain, and the rows add up
  (`1386.72 − 1866.23 = −479.51`). Replaces the three-term
  `NET = SAVED − EXECUTOR − ARCHITECT` that needed a legend to parse.
- **`—` means "not attributable", never `$0.00`.** Architect cost is
  project-scope-only, so Session and Milestone `Net` are genuinely unknown.
  Rendering them as zero was a false financial statement (bug-02-1).
- **One renderer, two surfaces.** `savings_lines` is a thin adapter over
  `ledger_lines`; `savings_lines_delegates_to_ledger_lines` makes drift
  impossible. The same de-duplication M35 06b-i applied to the aggregation.
- **`--tokens` mirrors the dashboard `b` key**; `--json` carries
  `executor_tokens` / `architect_tokens`, which were computed and discarded before.

### Executor performance

| phase | dispatches | turns | outcome |
|---|---|---|---|
| 01 | 2 | 87 + 40 | approved_after_1 |
| 02 | 3 | 273 + 59 + 32 | approved_after_2 |

**Zero oscillations across all five runs**, including three consecutive runs on
`panels.rs` — the file that hard-failed on oscillation four times during M35.
The M35 folds (pre-inject compiler-error recovery; pin the fixture that makes
the row appear) are holding. Scorecard for `Qwen/Qwen3.6-27B-FP8` now reads
N=8, gates 1.00, AFT 0.60 — the M38 bounces pulled the first-try rate down from
the N=3 reading of 1.00, which is the honest correction to a sample that was
always too small to mean anything.

**Both bounces were defects no gate could see**, which is the case for having a
review gate at all:

- **bug-02-1 (major)** — `$0.00` where `—` was correct. Failed no test *because
  the guard test had been rewritten to assert the wrong thing*.
- **bug-02-2 (minor)** — a new production `.unwrap()`. A STANDARDS violation
  rather than a behavior, so nothing could fail.

Nine reviewer-run mutations across the milestone; every one bit.

### Calibration — the architect spec-error family, TEN occurrences

**This is the milestone's dominant failure mode, ahead of anything the executor
did.** The pattern: *the architect states a fact in a spec — a line number, a
flag, a file list, a figure, an assertion that must hold — without deriving it
from the tool that defines it.*

| # | occurrence | surfaced |
|---|---|---|
| 1 | M36: `+59.6M` quoted pre-dedup; real recovery `+36.1M` | at review |
| 2 | M36 phase-02: file list from memory, missed `main.rs` | by the executor |
| 3 | M38: dropped "must track Executor token usage" between design and spec | by the user |
| 4 | phase-01 AC5 demanded zero `saved_*` matches while Task 3 required a fixture keeping them | at review |
| 5 | phase-01 E2E used `init --config`; the flag is `--dir` | by the executor |
| 6 | phase-02 cited drifted line numbers | pre-dispatch, by me |
| 7 | phase-02 Task 8 named 3 renames; the phase invalidated 3 more | at review |
| 8 | bug-02-1's worked fix cited `align_value` after the restructure inlined it | pre-dispatch, by me |
| 9 | bug-02-1's verification demanded a cross-field alignment equality that is unsatisfiable | by the executor |
| 10 | this review's own status edits no-opped against strings already rewritten | by checking the result |

Only **two** were caught before dispatch. The executor caught three of them and
adapted correctly each time, declaring the deviation — exactly the
declare-deviations discipline WORKFLOW asks for.

**A `WORKFLOW.md` fold is drafted and awaiting user sign-off** (per architect
skill § 6 prohibition 5). It is not landed in this commit.

### Calibration — green-bounce refined re-dispatch, FOUR occurrences

A phase bounced on test quality or a standards violation has green gates and a
clean tree, so a plain re-dispatch reads as already-done and no-ops. Confirmed
on M30 phase-01 (returned `complete`, empty diff), then countered successfully
three times in M38: phase-01, and phase-02 twice.

The working treatment: a loud header stating green gates are **expected and not
evidence of completion**, the remaining work enumerated, the fix inlined as a
worked example, and a **falsifiable finish condition** — with the refinement
that the condition should be *inverted* when the fix should add nothing
(bug-02-2's was "647, **not** 648", so a rising count signalled scope creep
rather than success).

**Also drafted for `plugin/skills/escalate/SKILL.md`, awaiting sign-off.**

### Carried forward

- **M37 phase-05** — server-authored completion bookkeeping, now 3 defects and
  8+ occurrences (unticked criteria, no E2E block, `Executor:` from model
  self-report).
- **M37 phase-02** — `FAILURE_CLASSES` gaps: `oscillation_stall` **and**
  `missing_spec_test`, both recorded open-vocab.
- **Duplicate rate tests** — `panels.rs` keeps three `architect_effective_rates_*`
  tests duplicating `config.rs`. Deliberately left: deleting tests was out of
  scope for a rendering phase and a duplicate passing test costs nothing.
- **Executor cache buckets read zero.** All 41 in-schema runs report
  `cache_read`/`cache_write` of 0 — either vLLM does not surface prefix-cache
  hits in its OpenAI-compatible response, or they are not captured. The buckets
  M35 06b-i wired up receive nothing. Not investigated; no milestone yet.
