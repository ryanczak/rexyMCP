# M38 — Discount Accounting

**Goal:** Present rexyMCP's actual value proposition — executor tokens are a
**discount** on architect token cost — as a two-line ledger with a net, priced
from a single Claude rate.

**Status:** in-progress *(opened 2026-07-23)*

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
| 02 | Ledger layout + `--tokens`: collapse SAVED into the Executor row, shared renderer ([phase-02-ledger-layout-shared-renderer.md](phase-02-ledger-layout-shared-renderer.md)) — bug-02-1 fixed & verified; bounced on bug-02-2 (minor: new production `.unwrap()`) | in-progress |

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