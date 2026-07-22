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
- Architect usage is a **transcript-native ledger** (added mid-milestone,
  2026-07-21): harvest reads the Claude Code project transcripts directly,
  dedups by `message.id`, persists records keyed (session × model × skill)
  with **all** project usage counted (non-skill messages bucketed `other`),
  and prices per architect model from a built-in Claude price table
  (config-overridable, 5m/1h cache-write rates distinguished). The
  time-window `ArchitectActivity` enrichment and single-`ArchitectRates`
  pricing are gone (compat break user-approved).
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
| 02 | Capture gaps: generation speed + output bytes ([phase-02-capture-gaps-speed-output-bytes.md](phase-02-capture-gaps-speed-output-bytes.md)) | done |
| 03 | Shared metrics & cost core + `[models]` pricing ([phase-03-shared-metrics-cost-core.md](phase-03-shared-metrics-cost-core.md)) | done |
| 04a | Run-level surface: `runs` cost/speed columns + run id ([phase-04a-runs-cost-speed-columns.md](phase-04a-runs-cost-speed-columns.md)) | done |
| 04b | Run-level surface: `runs show <id>` detail ([phase-04b-runs-show-detail.md](phase-04b-runs-show-detail.md)) | done |
| 05a-i | Scorecard: unified `aggregate_scorecard` core behind wrappers ([phase-05a-i-scorecard-unified-core.md](phase-05a-i-scorecard-unified-core.md)) | done |
| 05a-ii | Scorecard: migrate MCP `model_scorecard` onto the core; retire the Tag wrapper (`aggregate`/`ScorecardRow`) ([phase-05a-ii-scorecard-mcp-migration.md](phase-05a-ii-scorecard-mcp-migration.md)) | done |
| 05a-iii | Scorecard: `--by model\|tag\|settings` CLI + dropped columns; retire the Settings wrapper (`aggregate_by_settings`/`SettingsScorecardRow`) ([phase-05a-iii-scorecard-by-cli.md](phase-05a-iii-scorecard-by-cli.md)) | done |
| 05b | `profile --cost` tokens & cost to ship, per approved phase ([phase-05b-profile-phase-cost.md](phase-05b-profile-phase-cost.md)) | done |
| 06a | `rexymcp costs` CLI + shared cost-report core ([phase-06a-costs-cli-core.md](phase-06a-costs-cli-core.md)) | done |
| 06b-i | Dashboard Budget panel → `costs` core + cache buckets; de-dup the copied aggregation ([phase-06b-i-dashboard-rewire-cache.md](phase-06b-i-dashboard-rewire-cache.md)) | done |
| 06b-ii | Dashboard `b`-key tokens⇄currency toggle ([phase-06b-ii-budget-tokens-toggle.md](phase-06b-ii-budget-tokens-toggle.md)) | done |
| 06c-i | Architect ledger core: transcript-native harvest rewrite ([phase-06c-i-architect-ledger-core.md](phase-06c-i-architect-ledger-core.md)) | done |
| 06c-ii | Per-model architect pricing: built-in Claude price table + config override, 5m/1h cache-write split ([phase-06c-ii-architect-pricing.md](phase-06c-ii-architect-pricing.md)) | done |
| 06c-iii-a | Rewire costs + dashboard architect cost onto the ledger (per-model); milestone architect = `—`; restore doc ([phase-06c-iii-a-ledger-cost-rewire.md](phase-06c-iii-a-ledger-cost-rewire.md)) | done |
| 06c-iii-b | Ledger surface: per-skill architect breakdown (`costs` table SKILL/TOKENS/COST/% + one-line dashboard top-skill hint) ([phase-06c-iii-b-per-skill-breakdown.md](phase-06c-iii-b-per-skill-breakdown.md)) | done |
| 06d | Dashboard correctness: full `phase_id` (fixes session milestone + phase display; also bug-05b-1 root) + budget `b`-toggle border hint ([phase-06d-dashboard-fixes.md](phase-06d-dashboard-fixes.md)) | done |
| 06d-2 | ~~Dashboard trailing-row (issue 3)~~ — **closed won't-fix** (accept the blank; see Notes) | closed |
| 06e | Auto-telemetry: periodic background **harvest** sweep inside `serve` + liveness marker + `costs` liveness line ([phase-06e-auto-telemetry-sweep.md](phase-06e-auto-telemetry-sweep.md)) — journal-reconcile + CLI-deprecation deferred (serve can't reconcile assists) | done |
| 07a | Reporting debt: calibration reports the **low tail** for the lower-is-worse `oscillation_min_distinct` signal ([phase-07a-oscillation-low-tail.md](phase-07a-oscillation-low-tail.md)) | done |
| 07b | Reporting debt: `output_bytes` output-flood **signal** in `calibrate-governor` (replay captures `ToolResult`; new `Signal` + percentile) ([phase-07b-output-flood-signal.md](phase-07b-output-flood-signal.md)) | done |
| 07c | Reporting debt: `calibrate-governor` **rendering alignment** (move `percentile`→shared `metrics.rs`) + **discoverability** ("See also" cross-refs across runs/scorecard/profile/costs/calibrate-governor) ([phase-07c-calibrate-alignment-discoverability.md](phase-07c-calibrate-alignment-discoverability.md)) — reporting-debt complete; **M35 held open for cleanup** | done |
| 07d | M35-close cleanup batch: fix `profile` help wording + remove Budget `Assists:` row + Budget border `[b=toggle view]` ([phase-07d-budget-cleanup-batch.md](phase-07d-budget-cleanup-batch.md)) — bounced (bug-07d-1: deleted an unrelated test) | in-progress |
| 07e | M35-close cleanup: Budget savings **negative-value column alignment** (parenthesized debits align with non-paren values) | not drafted |
| 07f | M35-close cleanup: **trailing blank row** on Session/Budget/Context header panels (reopens 06d-2; drafted after 07d settles Budget height) | not drafted |

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

**Calibration fold — in-place shell edits (2026-07-20, user-approved mid-milestone).**
Three M35 phases hard-failed to the same mechanism: after a series of
`patch`/`patch_lines` edits shifted a file's lines, the model's next `patch`
failed (`0 matches for old_str` / `it changed on disk since you read it`) and it
escaped to `sed -i`, which edits by raw line number and bypasses both the
`old_str`-match and read-before-edit guards — so on the drifted file it corrupted
things (phase-04a: a `sed -i '178,179d'` loop cannibalized ~300 lines of
`runs.rs`). Root cause = *tooling escape hatch*, not a model preference (it used
the edit tools correctly until they refused on drift). **Fixed two ways:** (1) a
**hard guard** — the `bash` classifier now returns `Severity::RefuseInPlaceEdit`
for `sed -i`/`perl -i` (matched case-sensitively so perl's include `-I` is not
conflated), and the tool refuses with an advisory that steers to
`write_file`/`patch`/`patch_lines` and pins the *re-read-then-patch* recovery on
a drift failure; (2) a **contract Hard Rule** in
`executor/templates/executor_contract.md` saying the same. Read-only `sed`
(`sed -n …p`) is unaffected. **Also surfaced (deferred to phase-07):** the
Oscillation detector missed the 3-distinct-command sed loop (its window needs ≤2
distinct calls), so the run burned the full 600-turn budget — a real
oscillation-calibration data point.
**Architect-ledger design pass (2026-07-21, with the user — the 06c arc).**
Post-06b-ii audit found the architect side of the accounting structurally
inaccurate: `rexymcp harvest` (mcp/src/harvest.rs) attributes transcript usage
to `ArchitectActivity` journal time-windows (messages after the last boundary
are *dropped*), discards `message.model` (a four-model corpus — opus-4-8 ×3780,
sonnet-5 ×659, fable-5 ×423, sonnet-4-6 ×115 — priced at one
`effective_architect_rates()`), destroys per-skill/per-session detail at
ingest, and only runs in the `/rexymcp:auto` loop (the interactive workflow
never harvests; Claude Code prunes transcripts ~30 days, so unharvested
history evaporates). Meanwhile the transcripts themselves carry direct
`attributionSkill` (`rexymcp:dispatch` etc.), `sessionId`, `message.model`,
and a 5m/1h `cache_creation` split. Measured corpus (59 files, deduped by
`message.id` — 4,387 duplicate records from resume/compaction rewrites):
4,979 messages; 531k in / 43.1M cache-create / 1.507B cache-read / 3.55M out.
**Four forks resolved with the user:** (1) **transcript-native ledger** —
new record keyed (sessionId × model × attributionSkill), idempotent re-harvest
folds last-wins; the window-enrichment path is deleted (compat break
approved); (2) **all project usage counts** — non-skill messages (1,144)
bucketed as `other`, not dropped; (3) **built-in Claude price table +
config override**, with 5m and 1h cache-write rates distinguished (the token
record splits `cache_creation` into 5m/1h buckets); (4) **sequenced before
phase-07** as 06c-i/ii/iii — accounting correctness first, then 07's
reporting polish lands on accurate numbers.

**M35-close contract folds (design decisions with the user, 2026-07-21; folds 1–2
LANDED 2026-07-21 at the user's direction, ahead of the retrospective).**
Two folds, both surfaced by the 06c-i dispatch episode (architect cancelled two runs
that looked stuck; the next run pushed through the same step unaided — and the
monitoring poll-loop burned heavy Claude-Code tokens). Landed in
`plugin/templates/WORKFLOW.md` + `docs/dev/WORKFLOW.md` (new section "Governing a
running phase — the governor terminates, not the architect") and
`plugin/skills/dispatch/SKILL.md` (§2 reap protocol, the Stopping-a-phase policy,
and §7):

1. **Architect cancellation policy (WORKFLOW + dispatch/architect skills).** Sharpen
   "stopping is a deliberate act" into an **enumerated** allow-list: the architect
   may `stop_phase` **only** for (a) explicit human instruction, (b) a clearly
   mis-dispatched run (wrong phase/repo/config), or (c) a confirmed infra fault the
   governor can't see. **Never** for "slow"/"stuck"/"long generation" — those belong
   to the governor's terminators (no-progress stall, oscillation, identical-
   repetition, `max_turns`, `wall_clock_secs`). Rationale: cancelling out of
   impatience pre-empts the governor, downgrades a `hard_fail`+briefing into a weak
   `claude_stop`, and destroys stall-calibration evidence.
2. **Token-efficient monitoring protocol (dispatch skill).** Replace "keep polling
   until terminal" with **dispatch → confirm started → stop active polling → human
   watches `rexymcp status`/dashboard → reap when signalled/next turn.** No 15s poll
   loop, no turn-by-turn narration, no repeated session-jsonl `tail`/`grep`. Claude
   Code sends no MCP progressToken, so the human is already the live watcher.

**Token deep-dive (quantify now from raw transcripts, user decision 2026-07-21).**
Measure Claude-Code token cost in two use cases — (i) per rexymcp skill invocation
(the full SKILL.md loads on every call) and (ii) in-flight monitoring (the poll/parse
loop) — directly from `~/.claude/projects/-home-matt-src-rexyMCP/*.jsonl` (dedup by
`message.id`), independent of the 06c ledger. Goal: cut both cost centers hard
without impairing review rigor or governor-calibration signal. Feeds folds 1–2 and
informs whether the skills themselves need trimming.

**Dashboard fixes + auto-telemetry — 2 new phases queued (2026-07-21, with the user).**
Five pre-close issues, grouped into **06d** (dashboard fixes, issues 2–5) and **06e**
(auto-telemetry, issue 1); both land before phase-07. Code locations triaged:

- **[06e] Auto-telemetry (issue 1).** On launch, `harvest`/`journal`/`review` telemetry
  must be captured **in the background with zero user input** — deriving the transcript
  dir from the cwd (`~/.claude/projects/<munged-cwd>`) and project-id from
  `rexymcp.toml`. **User decision:** a **periodic background sweep inside `rexymcp
  serve`** (re-run harvest + journal reconciliation on an interval so telemetry stays
  continuously current), not just a one-shot at startup. `review` is reconciled from
  existing session-log outcomes where derivable — a *verdict* remains a judgment, so
  the sweep records what's inferable, it does not invent verdicts. Today these are
  explicit CLI subcommands (`Commands::{Harvest,Journal,Review}` in `mcp/src/main.rs`)
  invoked with args by the skills; 06e makes them self-service inside serve.

  **Deprecation design note (open question for 06e — user-raised 2026-07-21).** Once
  the sweep automates capture, do the `harvest`/`journal` *CLI subcommands* still earn
  their keep? The underlying **functions** (`harvest()`, `journal::record_activity()`)
  stay — the sweep is built on them; this is only about the CLI entry points + the
  `/auto` skill's explicit calls (the sole invokers — no interactive skill uses them).
  - **`harvest` CLI → deprecate.** Fully automatable by the sweep (derives everything);
    keep the function, drop the user-facing command (or demote to a hidden `--backfill`
    debug path) and remove the `/auto` call.
  - **`journal` CLI → decide in 06e, gated on what the sweep can reconcile.** Post-06c-iii
    the ledger takes over token attribution, leaving `ArchitectActivity`'s **only**
    remaining consumer the **assist count** (`activity == "assist"`, read at
    `costs.rs:205` + `dashboard/mod.rs:63`). The ledger can't derive it (aggregated
    `session×model×skill` — knows escalate *happened*, not *how many* assists), but
    **serve can** — it observes every dispatch / hard_fail / refine-re-dispatch cycle in
    its own run history. **If 06e's sweep reconciles assist counts from serve's run
    history, then `journal` + `ArchitectActivity` + the `/auto` journal/harvest calls all
    retire together** (a real simplification). If not, `harvest` still goes but
    `journal`/assists stay in an auto-reconciled form.
  - **Do not deprecate anything before 06c-iii + 06e land** — pulling the CLIs before the
    sweep replaces them would break `/auto` mid-milestone.
- **[06d] Dashboard fixes (issues 2–5), all in `mcp/src/dashboard/`:**
  1. **Budget `b`-toggle border hint (2).** Mirror the Activity panel's
     `.title(" Activity [f=filter] ")` (`render.rs:297`) — the Budget panel border
     should advertise `[b=$/tok]` (or similar) the same way.
  2. **Trailing blank row (3).** Session / Budget / Context panels show an extra empty
     row at the bottom (content underfills the fixed `Layout::vertical([Length(11),…])`
     area, `render.rs:192`). Trim it.
  3. **Session Milestone usually wrong (4).** `render.rs:209` renders `data.milestone`;
     its derivation in `dashboard/mod.rs` (`load_data`) is buggy — fix the milestone
     resolution.
  4. **Session Phase truncated (5).** `panels.rs:64` shows `summary.phase`, sourced
     coarse from the session record (`status.rs:148` → e.g. `phase-06`, not
     `phase-06b`). **This is a recurrence of bug-05b-1** (coarse `phase_id` vs the
     doc-stem) on the dashboard surface — the session panel must show the full phase
     name. Fix at the source or derive the stem from the phase_doc_path.
**06c-iii-b scope refinement + freshness → 06e (2026-07-21, with the user).** The
user observed that **06e's periodic background sweep makes a harvest-*freshness*
display redundant** — the sweep keeps the ledger continuously fresh, so a per-`costs`
"is this stale?" footer loses its purpose (it was a workaround for forgotten *manual*
harvests). The residual concern is *sweep liveness* ("is the sweep running / when did
it last run?"), which belongs with the sweep in **06e**, not on `costs`. So:
**harvest-freshness is dropped from 06c-iii-b and folded into 06e** as a sweep-liveness
indicator. **06c-iii-b is now per-skill-breakdown only:** (1) `rexymcp costs` **always
appends** a per-skill architect table — **SKILL / TOKENS / COST / %-of-architect**,
project-scoped, per-model priced, sorted by cost desc (the deep-dive found dispatch ≈
49%); (2) the dashboard Budget panel gains a **one-line top-skill hint** (not a full
mini-panel — keeps the TUI change small, given the executor's struggles there).
**06d scope + issue-3 finding (2026-07-21).** The four pre-close dashboard issues split:
06d takes **issues 2 (budget `b`-hint), 4 (wrong milestone), 5 (truncated phase)** — 4+5
share one root, `derive_phase_id` collapsing `phase-06c-iii-b-…` to coarse `"phase-06"`;
the user approved fixing it at the root (`derive_phase_id` → full id), which also fixes
**bug-05b-1's root** and makes the telemetry `phase_id` grouping finer (06a vs 06b vs
06c-i now distinct; back-compat already waived). **Issue 3 (trailing blank row) turned out
NOT to be a simple trim:** the three header panels (Session/Budget/Context) share one
fixed-height horizontal band sized to the tallest (Budget ≈9 rows — `budget_lines` 3 +
savings ~5 + the 06c-iii-b top-skill line 1); Session/Context (~8) are shorter so show one
blank, and the band is already right-sized to Budget, so trimming would **clip Budget**.
Eliminating the blank needs a **design decision** — per-panel heights (a layout change),
or moving the top-skill line out of Budget's content (e.g. into its border title, though
that competes with the `[b=…]` hint), or accepting the blank. Deferred as **06d-2** pending
that call; not scoped in 06d.
**Issue 3 / 06d-2 — closed won't-fix (2026-07-21, user decision "accept the blank").** After
06d landed, the trailing-row layout call was put to the user with four options (defer to
06e / accept the blank / relocate the top-skill line into Budget's border title / per-panel
header restructure). **Decision: accept the blank.** The blank is intrinsic to a
shared-fixed-height horizontal band (`render.rs` `Layout::vertical([Length(11), Min(0)])`,
render.rs:191–192): Session/Budget/Context share one height sized to the tallest (Budget, 9
content rows = `budget_lines` 3 + `savings_lines` ~5 + the 06c-iii-b top-skill line 1), so
the shorter Session/Context (~8) each show one blank row. Trimming the band clips Budget;
relocating the top-skill line competes with the `[b=$/tok]` hint and truncates on narrow
terminals; a per-panel restructure is the largest change of the four and is TUI-adjacent
(the 06c-i/iii-a sed-repetition hard_fails all were). A one-row cosmetic gap does not
justify any of these. **No code change; no dispatchable phase.** 06d-2 is removed from the
pipeline — remaining M35 is **06e → 07**.