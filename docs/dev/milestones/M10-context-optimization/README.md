# M10 — Context optimization

**Goal:** Treat the local executor's context window as a first-class, scarce
resource and manage it **proactively and content-aware**, instead of the current
reactive, value-blind scheme. Two levers: (A) **filter tool/command output at the
boundary** before it enters context — diagnostic-preserving, RTK-inspired, native;
and (B) **manage the context lifecycle semantically** — evict superseded file
reads, dedupe redundant re-reads, and rank compaction by value — using state
rexyMCP already tracks (the read-before-edit working set). Every change is
**scorecard-measurable**, so we can prove an optimization raises first-pass rate
or lowers compaction frequency rather than asserting it.

**Status:** design drafted (2026-06-07) — scoped into the phase roadmap below; no
phase doc authored or dispatched yet. Milestone start is human-gated; the user
advances with `/rexymcp:architect next` to draft phase-01.

**Depends on:**
- **M4** (the agent loop, context budget + compactor, the read-before-edit
  working set, the `PhaseRun` metrics record) — the substrate every phase here
  builds on.
- **M7** (`PhaseRun` telemetry + scorecard) — the measurement loop that lets us
  evaluate whether each optimization actually helps.
- **M9** (`read_file` 500-line cap) — the first, narrow instance of boundary
  capping; M10 generalizes it from one tool to all tool/command output.

## Why this milestone (the thesis)

The executor runs against a **local LLM with no token cost** — so unlike RTK (whose
pitch is dollar savings on cloud APIs), the lever here is **not money, it's the
context window itself**. A 32k–128k local model fills up fast, and the entire
M8/M9 history is evidence that context pressure is the executor's dominant failure
mode:

- M9/phase-03 capped `read_file` because the executor kept tripping
  `RunawayOutput` on 149 KB whole-file reads (`governor/hard_fail.rs:7`,
  `RUNAWAY_OUTPUT_BYTES = 100 * 1024`).
- Repeated `IdenticalToolCallRepetition` stalls (M8/phase-10b, M9/phase-04) were
  the model re-reading the same large content while stuck
  (`governor/hard_fail.rs:5`, `IDENTICAL_CALL_THRESHOLD = 3`).
- The dashboard (M8) exists largely to watch `context_pct` and compaction firings —
  we built an instrument for context pressure because it matters.

Less context pressure → fewer compactions (which lose information mid-phase) →
fewer overflow/repetition hard-fails → more useful turns before
`max_context_pct` → **higher executor success rate**, which the M7 scorecard
already measures. That makes context efficiency squarely on-mission, the same vein
as M9 (runtime hardening), not scope creep.

## Current state (what M10 changes)

Grounded in the code as of 2026-06-07:

| Concern | Today | File anchor |
|---|---|---|
| Compaction | Two-pass, **value-blind**: signaturize oldest tool-results → `[compacted: N bytes]`, then evict oldest non-system message, down to 75% of ceiling. Never considers *what* it drops. | `context/compactor.rs:19` (`TARGET_FRACTION = 0.75`), pass 1/2 at `:58`+ |
| Command output → context | The `bash` tool returns **raw, uncapped** stdout+stderr into context; only a 100 KB `RunawayOutput` hard-fail bounds it. A failing `cargo test`/`build` dumps the full log. | `agent/tools.rs` `append_tool_exchange`; `governor/hard_fail.rs:7` |
| `read_file` | Capped at 500 lines + truncation notice (M9). The one existing boundary filter. | `tools/read_file.rs:18` |
| Final command set | Tailed to 4 000 chars — but only at phase end, **not fed to the model mid-loop**. | `agent/command.rs:66` (`MAX_COMMAND_TAIL_CHARS`) |
| Superseded reads | A file read at turn 3 then edited at turn 7 leaves the **stale turn-3 content** in context until generic oldest-first compaction happens to reach it. Nothing evicts it on the basis that it's now wrong. | working set `agent/mod.rs:131`, refreshed on edit `:659` |
| Redundant re-reads | Re-reading an unchanged file re-injects its full content; no "you already have this". | — |
| Context metrics | `context_pct` is emitted per turn (M8/06a) and compaction events are logged (M8/07), but **not summarized onto `PhaseRun`** for cross-run comparison. | `PhaseRun` (architecture.md §scorecard) |

The token estimator is a chars/4 heuristic (`context/tokens.rs`) — *intentionally
kept*. RTK uses the identical heuristic; a real tokenizer is not worth the
dependency or the per-turn cost, and relative savings are what matter.

## What we take from RTK (and what we deliberately don't)

RTK is a **stateless CLI proxy** — it filters one command's output with no notion
of a conversation, a working set, or an edit history. We borrow its **filtering
techniques** and reject its **architecture** (a shell-out to an external binary is
non-hermetic and violates rexyMCP's no-unauthorized-deps discipline — we build
native).

**Borrow (Arc A):**
- **Diagnostic-level losslessness, list-level compression.** RTK preserves every
  `error[Exxx]` block with its full `file:line:col` span and message, and only
  caps the *list* (top-N errors, overflow saved to a "tee" file). This is the
  exact contract our executor needs — it *acts* on the output, so a dropped span
  is a phase it can't fix.
- **Failure-only test filtering** — drop passing-test lines, keep failures +
  summary (80–99% reduction when green).
- **Block aggregation / grouping** — collect a diagnostic block start→continuation,
  group by rule, show counts.
- **The "tee" recovery file** — when filtering caps output, write the full raw
  output to disk and leave a `[full output: <path>]` hint so nothing is truly
  lost. rexyMCP already has `<repo>/.rexymcp/sessions/`; the recovery file is a
  natural sibling.
- **ANSI stripping + repeated-line dedupe-with-counts** as a universal first pass.

**Reject:**
- Shelling out to `rtk`. Native Rust filters only, hermetically tested.
- RTK's 100+-command breadth. We ship a **generic language-agnostic filter** that
  always applies, plus a **small set of structured filters** for the project's
  *configured* toolchain (rexyMCP already knows `format`/`build`/`lint`/`test`
  from `rexymcp.toml`). We do not chase per-command coverage.
- RTK's aggressive "signatures-only" file read. For an agent that edits code,
  stripping function bodies is dangerous; our `read_file` cap stays
  line-based + range-addressable.

## What is novel to rexyMCP (Arc B — the part RTK structurally cannot do)

RTK has no conversation state, so these are ours alone:

1. **Superseded-read eviction.** The working set
   (`HashMap<PathBuf, SystemTime>`, `agent/mod.rs:131`) already records the
   read→edit transition. When `patch`/`write_file` edits a file, every earlier
   `read_file` result for that path in the message history is now *stale* — the
   model could act on pre-edit content. M10 marks those results superseded and
   evicts/signaturizes them **first** (highest priority), with a breadcrumb
   ("file since edited — re-read for current state"). This recovers context *and*
   removes a real correctness hazard.

2. **Redundant re-read awareness.** When the model re-reads a file unchanged since
   its last read this session (working-set mtime match), return a compact
   "unchanged since your read at turn N" reference instead of re-injecting the
   full content — while still allowing a forced re-read. Directly attacks the
   `IdenticalToolCallRepetition` stall class.

3. **Content-aware compaction priority.** Replace value-blind oldest-first with a
   value rank: evict **superseded reads → noisy/duplicate command output → old
   reasoning**, while protecting **diagnostics, the phase doc/system contract, and
   the last K turns**. Same `CompactionReport` plumbing (M8 already renders it),
   richer signal.

4. **Scorecard-measured optimization.** Every other token tool self-reports
   estimates. rexyMCP can *prove* a win: add context-efficiency fields to
   `PhaseRun` and read them across runs. This is the differentiator that ties the
   milestone to the project's identity.

## Exit criteria

- Tool/command output is filtered **at the boundary** before entering executor
  context, with **every error message, `file:line:col` span, and failing-test name
  preserved** — verified by explicit must-NOT-drop test cases. Overflow beyond the
  list cap is written to a recovery file under `<repo>/.rexymcp/` with a hint the
  model can act on.
- The executor's configured `test`/`build`/`lint` commands, when run by the model,
  yield failures + diagnostics + summary rather than full raw logs.
- A file edited after being read no longer leaves stale pre-edit content occupying
  context; compaction prefers evicting superseded/low-value content over recent
  reasoning and diagnostics.
- `PhaseRun` carries context-efficiency metrics (peak context %, compaction count,
  estimated tokens reclaimed by filtering + superseded eviction) and they surface
  in `rexymcp runs` / the scorecard, so a before/after comparison is possible.
- Everything stays **hermetic** (no real network/host state; filters are pure over
  captured output) and adds **no unauthorized dependency**. `read_file`'s existing
  behavior and the security/scope boundary are unchanged.

## Architecture references

- `docs/architecture.md#the-executor-turn-cycle` — steps 2 (apply budget, compact),
  5 (tool dispatch), 6 (verify), 8 (final command set). M10 touches the output path
  of 5/6/8 and the compaction policy of 2.
- `docs/architecture.md` § "The `PhaseResult` / briefing contract" and § "Model
  effectiveness metrics & the scorecard" — where context-efficiency metrics land.
- `executor/src/context/{compactor,budget,tokens}.rs` — the compaction substrate.
- `executor/src/agent/{tools,command}.rs` — where tool/command output is captured
  and appended to context (`append_tool_exchange`, `run_command_set`).
- `executor/src/governor/hard_fail.rs` — `RunawayOutput` / `IdenticalToolCall`
  thresholds M10 should reduce the firing rate of (not change).

## Phases (roadmap — authored on demand)

Expanded into phase docs one at a time per the project's "expand on demand" rule
(architecture.md § Status). Single-concern, small-diff, additive-where-possible
(per the phase-10b / M9 calibration: keep executor phases narrow). Order runs the
**universal, highest-coverage** filter first, then the project-specific structured
filter, then the novel semantic levers, then measurement.

| #  | Phase | Arc | Status |
|----|-------|-----|--------|
| 01 | **Recoverable output filter for bash output** ([phase-01-recoverable-output-filter.md](phase-01-recoverable-output-filter.md)). New `context/output_filter` module: ANSI strip + consecutive-dup collapse + truncate-with-**recovery file** under `.rexymcp/output/` (rotated), wired into the `bash` tool's existing truncation, gated by a `[context] output_filter` kill-switch (default on). Turns bash's current lossy "full output not retained" truncation into a recoverable one. Establishes the diagnostic-preservation contract + recovery-file primitive that phase-02 reuses. | A | done |
| 02 | **Structured cargo filter (test/build/clippy).** Keyed on detecting `cargo` in the command: failures + diagnostics + summary only, block aggregation, preserve every `error[Exxx]` span, cap the list with recovery-file overflow (reuses phase-01's module). Introduces the per-command filter-selection abstraction (deferred from phase-01 — built once a second filter justifies it). The high-value filter for the project's own Rust toolchain. ([phase-02-structured-cargo-filter.md](phase-02-structured-cargo-filter.md)) | A | done |
| 03 | **Arc A reclaim events (`OutputFiltered`).** Per-lever `SessionEvent` recording how much the phase-01/02 boundary filters reclaimed (tokens before/after, generic vs cargo), emitted from the loop via the bash tool's `ToolResult.metadata`. Pure instrumentation — filter output unchanged. Establishes the per-lever reclaim-event pattern phases 04/05 reuse. ([phase-03-arc-a-reclaim-events.md](phase-03-arc-a-reclaim-events.md)) | A | done |
| 04 | **Superseded-read eviction (`ReadEvicted`).** On edit, replace prior `read_file` results for that path with a re-read breadcrumb (reclaim context + kill the stale-content hazard); emit a `ReadEvicted` event. Eager at edit time; no compactor change. ([phase-04-superseded-read-eviction.md](phase-04-superseded-read-eviction.md)) | B | done |
| 05 | **Fix `Budget::estimate` — count tool exchange content.** `estimate` currently ignores `tool_calls[n].arguments` and `tool_results[n].content`, so `context_pct` is always ~15% (system-prompt only), the compactor's `would_overflow` never fires on real pressure, and phase-08's metrics would aggregate wrong values. Purely additive fix in `budget.rs` + 3 new tests. ([phase-05-budget-estimate-fix.md](phase-05-budget-estimate-fix.md)) | — | done |
| 06 | **Redundant-read dedupe.** Re-reading an unchanged file (working-set mtime match) returns a compact "unchanged since turn N" reference instead of re-injecting content; forced re-read still available. Emits its own per-lever reclaim event. ([phase-06-redundant-read-dedupe.md](phase-06-redundant-read-dedupe.md)) | B | done |
| 07 | **Content-aware compaction priority.** Replace value-blind compaction with a value-ranked **in-place signaturization** pass: shrink lowest-value tool output first (noisy command output before file reads), protect the last K turns, preserve tool-call/tool-result pairing. Also fixes the compactor's post-phase-05 token under-count. Single-file (`compactor.rs`); `CompactionReport` shape unchanged (per-source breakdown deferred to phase-08). ([phase-07-content-aware-compaction.md](phase-07-content-aware-compaction.md)) | B | done |
| 08a | **Context-efficiency aggregation onto `PhaseRun`** (executor-only). New `ContextEfficiency` struct + pure `aggregate_context_efficiency` over the session-log records (peak context %, compaction count, tokens reclaimed by source); `emit_phase_run` reconstructs the log path and folds the aggregate onto `PhaseRun` as a `#[serde(default)]` field. The data-capture foundation; nothing surfaces it until 08b/08c. ([phase-08a-context-efficiency-phaserun.md](phase-08a-context-efficiency-phaserun.md)) | — | done |
| 08b | **Surface context-efficiency in `rexymcp runs`** (mcp-only, single-file). Add two per-run columns — `PEAK_CXT` (peak context utilization) + `RECLAIMED` (total tokens reclaimed by all four levers) — to `format_runs`, reading the 08a `PhaseRun.context_efficiency` field. Purely additive (read-only), no struct changes. ([phase-08b-runs-context-efficiency-columns.md](phase-08b-runs-context-efficiency-columns.md)) | — | done |
| 08c | **Aggregate context-efficiency into the model × tag scorecard** (mcp-only, single-file). Add `peak_context_pct_mean` + `tokens_reclaimed_mean` (both `Option<f64>`, mean over context-measured runs) to `ScorecardRow` + its `Accumulator` + the `aggregate` function — all in `mcp/src/scorecard.rs`, exactly **one** struct literal. MCP-tool-only (no CLI); new fields serialize through `model_scorecard` automatically. ([phase-08c-scorecard-context-efficiency-model-tag.md](phase-08c-scorecard-context-efficiency-model-tag.md)) | — | done |
| 08d | **Aggregate context-efficiency into the model × settings scorecard** ([phase-08d-scorecard-context-efficiency-model-settings.md](phase-08d-scorecard-context-efficiency-model-settings.md)) (mcp-only). The same two means on `SettingsScorecardRow` + `SettingsAccumulator` + `aggregate_by_settings` + new columns in the `format_settings_scorecard` CLI renderer. Three struct-literal sites across `scorecard.rs` + `scorecard_cli.rs` — the churn-dense half, split out from 08c so each dispatch is single-concern. | — | done |
| 08e | **Fold reclaim events into `StatusSummary` / dashboard** ([phase-08e-reclaim-events-statussummary-dashboard.md](phase-08e-reclaim-events-statussummary-dashboard.md)) (mcp-only, live view). `summarize` gains three arms folding `OutputFiltered`/`ReadEvicted`/`ReadDeduped` (currently in the `_ => {}` catch-all) into six additive `StatusSummary` counters; the dashboard's Compactions panel is repurposed into an aggregate **Reclaim** panel (`compactions_lines`→`reclaim_lines`); `format_status` gains a `reclaimed:` line. The three variants already exist (03/04/06) and are already handled in `log_query`/`transcript`/`filter` — **no new variant, no match-arm wall**; field-adds are additive (every `StatusSummary` literal spreads `..default()`). | — | done |

Phases may split or merge at draft time. The table is the roadmap, not a
contract; each phase doc is the contract. **Measurement is per-lever:** each
reclaim phase (03/04/06/07) emits its own `SessionEvent` variant when it lands, and
**phase-08a** reads those durable JSONL events back into `PhaseRun` — so no lever
ships un-instrumented and 08a needs no retrofit.

## Design decisions

**Filter at the tool boundary, not the verifier's pass/fail.** The verifier
parses raw command output to decide gate pass/fail (`agent/command.rs`); that
determination must stay on **raw** output. Filtering applies only to the
**model-facing presentation** (the `ToolResult` content the model reads, and the
diagnostic feedback fed back on verifier failure) — never to the boolean the gate
computes. Keep the two paths separate.

**Generic-first, structured-second.** The universal filter (phase-01) is the most
rexyMCP-appropriate move: language-agnostic, always applies, preserves the tail
where errors live, and needs no per-command knowledge. Structured filters
(phase-02) layer on top only for the *configured* toolchain. This avoids RTK's
100-command maintenance surface while still capturing the biggest structured win
(the project's own test/build output).

**Losslessness is the safety rail.** The executor *acts* on output. Every Arc-A
phase must pin **must-NOT-drop** cases (a real `error[Exxx]` with its span; a
failing test name; the last N lines of a panic) alongside the must-drop noise.
This is the same "pin negative cases" discipline the architect applies to scope
confinement.

**Recovery, never deletion.** Capped output is written to a recovery file the
model can re-read on demand (the RTK "tee" pattern), and the session JSONL already
retains the full record. M10 compresses what's *in context*; it never destroys the
forensic record.

**Measure, don't assert.** Phase-06 exists so M10 is falsifiable. Small models are
high-variance (architecture.md § scorecard), so the claim isn't "phase-02 saves
80%" — it's "across N runs, first-pass rate / mean compactions moved this way, at
this sample size." If a lever doesn't move the scorecard, that's data, and we say
so.

**Don't touch the security boundary or `read_file`'s range semantics.** Scope
confinement, the bash classifier, redaction, and `read_file`'s line/range
addressing are out of scope and unchanged. M10 is a presentation/lifecycle layer
over already-captured, already-redacted output.

## Out of scope

- Any cloud tokenizer or new dependency. The chars/4 heuristic stays.
- Per-command filter breadth beyond the project's configured toolchain + the
  generic fallback. (Adding a pytest/ruff/go filter is a future phase *if a target
  project needs it*, not part of M10's core.)
- Changing hard-fail thresholds (`RUNAWAY_OUTPUT_BYTES`, `IDENTICAL_CALL_THRESHOLD`).
  M10 aims to reduce their *firing rate* by reducing pressure, not to widen the
  governor.
- The architect-side (Claude's) token spend on pre-injection. That's a different
  layer with real cloud cost; not this milestone.
- Resume / `continue_phase` (still an uncommitted candidate, architecture.md
  § Escalation).

## Resolved decisions (2026-06-07, with the user)

1. **Filter activation:** **on by default with a kill-switch** — `[context]
   output_filter` in `rexymcp.toml`, default `true`. Losslessness is the contract
   and the recovery file backs it; the kill-switch restores raw truncation.
2. **Recovery files:** `<repo>/.rexymcp/output/`, **rotated** (keep last 20),
   git-ignored (already covered by `/.rexymcp`). The model can re-read them with
   `read_file` (the path is scope-confined).
3. **Phase-02 first toolchain:** **cargo first** — rexyMCP is itself Rust and it's
   the most-exercised dogfood path. A more general "any `--format json` runner"
   filter is a later phase if a target project needs it.

## Notes

### Milestone retrospective (2026-06-08, M10 complete — all 13 phases done)

**What shipped.** Two arcs landed end-to-end. **Arc A** (boundary output filtering):
phase-01 recoverable generic filter + phase-02 structured cargo filter. **Arc B**
(semantic context lifecycle): phase-04 superseded-read eviction, phase-06 redundant-read
dedupe, phase-07 value-ranked content-aware compaction — all built on the M4
read-before-edit working set. The cross-cutting **measurement spine**: phase-03 retrofit
of per-lever reclaim events (`OutputFiltered`), phase-05 `Budget::estimate` correctness
fix (it was counting only `msg.content`, so `context_pct` never grew), and the phase-08
surfacing fan-out — 08a aggregation onto `PhaseRun`, 08b `runs` columns, 08c model×tag
scorecard, 08d model×settings scorecard, 08e (this phase) the live `StatusSummary` /
dashboard / `rexymcp status` view. Every reclaim lever is now visible both **post-hoc**
(runs + scorecards) and **live** (dashboard Reclaim panel + `status --json`).

**What worked.** The **per-lever-`SessionEvent` measurement strategy** (decided with the
user 2026-06-07) paid off exactly as intended: because each lever emitted a durable JSONL
event the moment it landed, the 08-series surfacing phases were pure additive reads with
zero retrofit. The **split-by-output-struct** decomposition of the surfacing work (08a–08e)
kept each dispatch single-concern and let the no-churn halves land first-try.

**What broke — the dominant calibration story.** The local executor (Qwen/Qwen3.6-27B-FP8)
stalled on **repetitive multi-site mechanical edits five times** across M10: phase-03/04/06
(match-arm `filter.rs` walls — `VerifierFailurePersistent` or false-`complete`), phase-08a
(5 cross-crate struct-literal field-adds — `IdenticalToolCallRepetition`), and phase-08d
(3 cross-file literals — `VerifierFailurePersistent`). This was confirmed by a **controlled
A/B**: 08c (1 literal, single file) landed clean first-try in 66 turns; its split sibling
08d (3 literals across two files) stalled exactly as predicted, same pre-injection quality
on both arms — isolating **literal/site-count** as the stall driver, not task difficulty.
**08e was drafted as the low-churn counter-shape** (six purely additive `..default()` fields,
three new arms before an intact `_ => {}`, no struct literal to touch) and **landed clean,
first-try** — the predicted payoff, further confirming additive-shape as the lever.

**Recovery levers that worked:** (1) architect session takeover / closeout of the mechanical
remainder after a `hard_fail` (08a, 08d); (2) narrow-scope re-dispatch against a clean
committed tree (06 tests-only); (3) the compile-first-then-test re-dispatch checklist (03,
04); (4) the interim governor-threshold raise 3→6 (commit `e543f57`).

**Held calibration fold (for the user, at this retrospective).** Five occurrences + a
controlled A/B is well past the three-strikes fold bar. The proposed `WORKFLOW.md` addition
to the "Prefer additive change shapes" guidance: *"prefer splitting a feature by
output-struct so each executor dispatch touches ≤1 non-`Default` struct literal; a
pre-injected site-list alone does not prevent the stall (08a, 08d both stalled despite
complete site-lists)."* Per the hard rule, `WORKFLOW.md` is not edited without explicit
user sign-off — this fold is surfaced for that decision, not applied here.

**Outstanding (human-gated, unchanged by this phase):** `docs/architecture.md` § Status
still carries no M10 entry — adding it is an edit to a human-gated source-of-truth file
and is deferred to formal M10 sign-off (see below).

The architecture.md § Status list does not yet carry an M10 entry — adding it is a
documentation edit to a human-gated source-of-truth file (a hard rule: no
unauthorized edits to `docs/architecture.md`). That entry should be added with the
user's sign-off when M10 is formally kicked off.
