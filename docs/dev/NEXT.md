# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** **phase-08c** ([aggregate context-efficiency into the model × tag scorecard](milestones/M10-context-optimization/phase-08c-scorecard-context-efficiency-model-tag.md)) — drafted 2026-06-08, `todo`, awaiting dispatch. The user dispatches with `/rexymcp:dispatch phase-08c`.

**The old combined 08c was split by output-struct (with the user, 2026-06-08)** to disarm the 4×-recurring mechanical-multi-site-churn stall *structurally* rather than by pre-injected site-list alone (08a stalled despite that mitigation). The two scorecards have asymmetric blast radius: **`ScorecardRow` (model × tag) = 1 struct literal, single-file `scorecard.rs`, MCP-tool-only/JSON (no CLI renderer)** — the clean half, now **08c**; **`SettingsScorecardRow` (model × settings) = 3 literals across `scorecard.rs` + `scorecard_cli.rs` + a CLI renderer** — the churn-dense half, now **08d**. The old dashboard/`StatusSummary` fold bumped 08d→**08e**. The user **held the WORKFLOW fold** (no `STANDARDS.md`/`WORKFLOW.md` edit yet) — gathering one more data point on whether the split reliably avoids the stall before folding "prefer splitting by output-struct over a pre-injected site list" into the existing "Prefer additive change shapes" section. **If 08c lands clean (1 literal) and 08d still churns (3 literals), that's the 5th occurrence + the controlled comparison → fold then.**

**phase-08c scope (active, mcp-only, single-file `scorecard.rs`):** add `peak_context_pct_mean` + `tokens_reclaimed_mean` (both `Option<f64>`) to `ScorecardRow` + 3 additive `Accumulator` fields (`peak_context_pct_sum`/`tokens_reclaimed_sum`/`context_measured_n`, `Default`-derived → no literal churn) + a conditional accumulation block + 2 fields on the **single** `ScorecardRow{…}` constructor literal. Means are over **context-measured runs only** (predicate: `peak_context_pct > 0.0`; legacy/serde-default all-zero runs excluded from both numerator and denominator), mirroring the existing `length_finish_rate_mean`/`bounces_to_approval_mean` Option-mean idiom. `tokens_reclaimed` = sum of all four sources. Pinned boundary: a *measured* run that reclaimed `0` contributes `Some(0.0)`, **not** exclusion — only `peak == 0.0` excludes. 5 unit tests + a serde-wire serialization test (no CLI path for model×tag — it's `model_scorecard` MCP-tool-only). ~70 lines. Executor target: Qwen/Qwen3.6-27B-FP8.

**Prior active-phase pointer (now done):**

**phase-08b done**

**Prior active-phase pointer (now done):**

**phase-08b done** (2026-06-08, approved_first_try): surfaced 08a's context-efficiency signal in the `rexymcp runs` table. Two read-only columns added to `format_runs` (`mcp/src/runs.rs`): `PEAK_CXT` (`context_efficiency.peak_context_pct` fraction → `{:.0}%`, `0.0` → `—`) + `RECLAIMED` (sum of all four reclaim sources — `output_filtered + read_evicted + read_deduped + compaction_tokens_reclaimed`, compact `12k`/`200` form mirroring `cxt_win`, `0` → `—`). Purely additive — no struct/accumulator/scorecard/dashboard changes, the proven-safe single-file shape; the columns are independent (a measured run with no levers fired renders `68%` + `—`). 3 unit tests (populated → `68%`/`12k`; must-sum-all-four sub-1024 → `200`, mutation-resistant; must-render-sentinel-`—`-when-zero) + a real-binary E2E reproduced at review (`cargo run -p rexymcp -- runs --config rexymcp.toml --telemetry-path <jsonl>` → `PEAK_CXT`/`RECLAIMED` headers + `qwen` row `68%`/`12k`). **664 executor + 246 mcp** pass. Committed `92edbd1` (feat) + `bc668fe` (docs). **Clean 29-turn first-try dispatch — zero bounce, zero mechanical-churn stall by design** (single-file, read-only, no cross-crate struct-literal surface — the deliberate counter-shape that isolated the easy column work from 08c's struct-literal churn). The split worked exactly as intended: the no-churn half landed first-try; the churn-dense half (08c) is quarantined. Review nit (not folded): the new tests' `.expect("… {out}")` messages don't interpolate (literal, not `format!`) — matches the module's pre-existing gemma-line tests, test-only.

**Prior active-phase pointer (now done):**

**phase-08a done** (2026-06-08, escalated — architect closeout after executor `hard_fail`): captured M10's context-efficiency signal onto `PhaseRun`. New `ContextEfficiency` struct + pure `aggregate_context_efficiency(&[SessionRecord])` in `telemetry.rs` (peak context %, compaction count, tokens reclaimed by source: filter/evict/dedupe + compaction); a `#[serde(default)] context_efficiency` field on `PhaseRun`; `emit_phase_run` (`agent/metrics.rs`) **reconstructs the session-log path** from `deps.project_root`/`input.phase`/`deps.session_id` and reads the just-written JSONL back best-effort (`read_session_log(...).unwrap_or_default()`), aggregates, sets the field — **no signature change, zero of the 9 call sites touched** (the deliberate counter-shape to the multi-site wall, and it worked: no match-arm bounce). 7 struct-literal field adds (2 executor + 5 mcp test helpers). 6 unit + 1 E2E test; **664 executor + 243 mcp** pass. The E2E `phase_run_context_efficiency_matches_session_log` was **mutation-verified at review** (wrong log-path prefix → persisted all-zeros vs on-disk `peak_context_pct 0.00336` → assertion fails), confirming it exercises the real read-back path. Committed `14b4668` (feat) + `1ccba39`/`1c899b2` (docs); approved at review. **One bounce — executor `hard_fail` `IdenticalToolCallRepetition` (3× `read_file scorecard.rs`)** after completing tasks 1–4 + 2/5 mcp literals; architect closed out the 3 remaining literals + all 7 tests, no re-dispatch. **Calibration: 4th occurrence of the mechanical-multi-site-churn stall** — but **struct-literal** churn this time (5 mcp `PhaseRun{…}` literals), not the match-arm `filter.rs` wall (which the no-call-site `emit_phase_run` design successfully dodged). See the calibration tracking below; reinforces the held-fold case for pre-applying mechanical literal adds when a field touches N>2 cross-crate struct literals.

**Prior active-phase pointer (now done):**

**phase-07 done** (2026-06-08, approved_first_try): M10 Arc B's content-aware compaction. The compactor (`executor/src/context/compactor.rs`, single file) gained three additive pieces: (1) a `message_tokens` helper mirroring `Budget::estimate` — fixes the post-phase-05 correctness bug where pass-2 eviction decremented the running total by `content`-only tokens, under-counting what it frees on the real structured tool-exchange shape (now uses `message_tokens(&removed)`); (2) a `reclaim_rank` value classifier (non-`read_file` tool output rank 0 → `read_file` rank 1, `None` for non-tool/husk/recency-protected) + `RECENT_TURNS_PROTECTED = 3` const; (3) a new **pass 1.5** value-ranked in-place signaturization between the existing two — shrinks `tool_results[0].content` to a `[compacted: …]` breadcrumb, lowest-value-oldest first, protecting the last 3 turns, skipping `[superseded:`/`[already-read:`/`[compacted:` husks. **Signaturize-in-place, not eviction-reorder** — every message and tool-call/tool-result pair preserved. `CompactionReport` shape unchanged (per-source breakdown deferred to phase-08). The 8 pre-existing compactor tests stayed green **unchanged**; +13 new (3 `message_tokens`, 5 `reclaim_rank`, 5 `compact` integration). **657 executor** pass. Clean **single-file 74-turn first-try dispatch — zero match-arm blast radius by design** (no `SessionEvent`/dashboard/`filter.rs` touch), the deliberate counter-shape to the filter.rs wall that bounced phase-03/04/06. The value-ordering test was mutation-verified at review (swapping `reclaim_rank`'s read_file/bash ranks fails `compact_reclaims_command_output_before_file_read`). Committed `92437a2` (feat); approved `_`.

**phase-06 done** (2026-06-08, approved_after_1): M10 Arc B's redundant-read dedupe. A `read_file` of an unchanged file whose whole-file content is still live in context returns a compact `[already-read: … turn N …]` reference instead of re-injecting the body — reclaims context and attacks the `IdenticalToolCallRepetition` stall. Two new fns in `agent/tools.rs` (`last_live_read` pure scan + `redundant_read_reference` mtime-gated decision, with ranged/`force:true` escape hatches), a dedupe short-circuit + `SessionEvent::ReadDeduped` emit in `mod.rs`, the `ReadDeduped` variant (14th event kind), a `force` schema-only property on `read_file`, and all match arms. 16 new tests (13 unit + 3 loop-integration); **644 executor + 243 mcp** pass. The e2e `loop_dedupes_unchanged_reread` was mutation-verified at review (neutralizing the dedupe fails the assertion). Commits `78cd19b` (feat) / `ca8e2e2` (test); approved `_`. **One bounce — the executor false-reported `complete` with a broken build and zero tests** (3rd `filter.rs`-wall occurrence; user held the fold); architect closed out the 6 mechanical match arms + committed a code checkpoint, then a **tests-only re-dispatch** (Notes-for-executor block) cleanly produced all 16 tests against the compiling tree. The narrow-scope re-dispatch against a clean committed tree is the effective recovery lever; see the calibration tracking below.

**phase-05 done** (2026-06-07, approved_first_try): fixed `Budget::estimate` to count `tool_calls[n].arguments` + `tool_results[n].content` (was counting only `msg.content`, which is empty on every tool-exchange message). Closes [`bug-budget-estimate-1`](milestones/M10-context-optimization/bugs/bug-budget-estimate-1.md) — `context_pct` will now grow turn-over-turn and the compactor's `would_overflow` fires on real pressure. Purely additive fix in `budget.rs` + 3 new tests; 628 pass. Qwen/Qwen3.6-27B-FP8, clean 40-turn first try (code + tests + Update Log + commit all landed, unusual completeness for the local executor). Committed `43fa08b`. **Calibration (1 occurrence):** the spec sketch used the file-local alias names `AiToolCall`/`AiToolResult`; the canonical types are `ToolCall`/`ToolResult` and the executor correctly adapted — watch-item on citing canonical type names in pre-injection.

**📌 Calibration tracking — mechanical-multi-site-churn stall (4 occurrences; user HELD the fold 2026-06-08, 4th landed at phase-08a):** The executor has now stalled on repetitive multi-site mechanical edits four times: phase-03 dispatch-1, phase-04 dispatch-1, phase-06 dispatch-1 (all partial `filter.rs`/match-arm walls, `VerifierFailurePersistent` or false-`complete`), and now **phase-08a dispatch-1** — `IdenticalToolCallRepetition` (3× `read_file scorecard.rs`) on the 5 mcp `PhaseRun{…}` struct-literal field adds. **The 4th occurrence is a different subclass:** struct-literal churn, not match-arm — and notably the phase's no-call-site-churn `emit_phase_run` design *successfully dodged* the match-arm `filter.rs` wall (zero `SessionEvent`/dashboard touch), so the stall moved to the next-densest mechanical surface (the cross-crate test-helper literals). The compile-first re-dispatch checklist + architect closeout of the mechanical remainder cleared it (architect finished 3/5 literals + all 7 tests, no re-dispatch needed). **Revisit the fold with the user now that the 4th has landed:** the generalized lever is *any field/variant add touching N>2 struct literals or match arms across crates should pre-apply the mechanical sites in the phase doc as pre-completed work* (the spec already enumerated all 5 literals — the next step is to mark them done, not just list them), or split the cross-crate mechanical fixups into their own micro-step. Prior sub-pattern (phase-06: false-`complete` with broken build + skipped tests) did **not** recur at 08a — the executor bounced cleanly as `hard_fail` rather than false-completing.

**phase-04 done** (2026-06-07, approved_after_1): M10 Arc B superseded-read eviction. `evict_superseded_reads` in `agent/tools.rs` (pure over the message slice, idempotent via `[superseded:` prefix guard, returns `(reads_evicted, tokens_reclaimed)`) + call site after the working-set record block (`mod.rs:691`); on a successful `patch`/`write_file`, prior `read_file` results for that path become a re-read breadcrumb and a `SessionEvent::ReadEvicted` is logged (per-lever pattern from phase-03). All 7 `filter.rs` sites + `transcript.rs` arm + `log_query`/`event_kind` arms landed. 10 new tests; 625 pass. **One real bounce** (dispatch-1 `VerifierFailurePersistent` on the partial-`filter.rs` exhaustive-match wall — 2nd occurrence of that class; the compile-first-then-test re-dispatch checklist *cleared* it); dispatch-2 completed all work then infra-dropped (`BackendError`) at turn 44 post-completion → architect closeout (rustfmt + one test-assertion fix + commit). Committed `92feaae`. **Surfaced `bug-budget-estimate-1` (above).**

**Measurement-strategy decision (2026-06-07, with the user):** every M10 reclaim lever emits its own **per-lever `SessionEvent` variant** when it lands (not one general event, and not deferred) — SessionEvents already have live consumers (dashboard transcript, `executor_log_search`/`get_turn`) and the JSONL is durable, so phase-07 aggregates them onto `PhaseRun` with no retrofit. The Arc A filters (phase-01/02) shipped silent, so a **retrofit phase was inserted now** (phase-03) before continuing Arc B. This renumbered the roadmap tail: eviction 03→**04**, dedupe 04→**05**, compaction 05→**06**, metrics 06→**07**.

**phase-03 scope (retrofit):** add `SessionEvent::OutputFiltered { tokens_before, tokens_after, filter }`; the `bash` tool reports the filter's token before/after (via `tokens::count`) in `ToolResult.metadata`; `dispatch` (1 call site) is widened to surface metadata; the loop emits `OutputFiltered` on a real reduction. Plus the 4 fixed match-arm sites a new variant needs (`event_type_str`, `log_query` `event_kind`, `dashboard/filter` `ActivityFilter`, `dashboard/transcript` `record_lines`); `status.rs`/`cap.rs`/serde sites need none. Pure instrumentation — filter output unchanged. ~170 lines.

**phase-04 scope (active, Arc B):** `evict_superseded_reads` in `agent/tools.rs` (pure over the message slice, returns `(reads_evicted, tokens_reclaimed)`) + call site after the working-set record block (`mod.rs:684`, before the post-write format hook); on a successful `patch`/`write_file`, prior `read_file` results for that path become a `[superseded:` breadcrumb, and a `SessionEvent::ReadEvicted` is logged (reuses phase-03's per-lever pattern). Always-safe → **no kill-switch**. No compactor change (phase-06), no `read_file` change (phase-05), no `PhaseRun` field (phase-07). ~200 lines. **Doc refreshed post-phase-03:** the `ReadEvicted` variant's match-arm blast radius is now enumerated exactly (`log_query::event_type_str`; `dashboard/filter.rs`'s **seven** sites — const + field + Default + allows/toggle/is_enabled/item_label; `dashboard/transcript::record_lines`; the `mod tests` `event_kind` helper) — closing the same `filter.rs` gap that hard-failed phase-03's first dispatch. Executor target: Qwen/Qwen3.6-27B-FP8.

**phase-03 done** (2026-06-07, approved_first_try): M10 Arc A reclaim instrumentation. Added `SessionEvent::OutputFiltered { tokens_before, tokens_after, filter }`; `bash` reports the filter's token before/after via `ToolResult.metadata` (only when the filter is on); `dispatch` widened to a 3-tuple surfacing success metadata; the loop emits `OutputFiltered` on a real reduction. The full match-arm blast radius landed — incl. `dashboard/filter.rs`'s **seven** per-event-kind sites (the first dispatch hard-failed `VerifierFailurePersistent` because the spec under-listed three of them; resolved by refined re-dispatch with the partial work preserved). 6 new tests; 615 pass. **Calibration logged:** under-listing the `filter.rs` blast radius for a new `SessionEvent` variant — now pre-empted in phase-04's spec. Committed `e3a9da2`; approved `0049722`.

**phase-02 done** (2026-06-07, approved_first_try): M10's structured cargo filter. Added `is_cargo_command` + `cargo_filter` + `is_cargo_noise` + `filter_for_command` dispatcher to `output_filter.rs`; `bash.rs` now routes through `filter_for_command(&parsed.command, …)` instead of calling `compact_with_recovery` directly (non-cargo falls through to the phase-01 generic filter). Keep-by-default design drops passing-test/`Compiling`/`Finished`/`Running` noise while preserving every error span, panic, and `test result:` summary; overflow still tees to a recovery file. 10 new tests (incl. a real-`cargo`-subprocess integration test); executor 609 pass, mcp 243 pass. Qwen/Qwen3.6-27B-FP8 — clean first-try with full bookkeeping (committed `8ccc896`, status flipped, Update Log filled), unlike phase-01's architect-closeout. One test-fixture adaptation logged in Notes for review (`"Finished\n"` → `"Finished dev [...]"` to match the trailing-space prefix check).

**phase-01 done** (2026-06-07, approved_first_try — architect closeout): M10's recoverable output filter. New `executor/src/context/output_filter.rs` (`normalize` = ANSI strip + consecutive-dup collapse with `(xN)`; `compact_with_recovery` = head+tail truncation teeing full normalized output to a rotated recovery file under `.rexymcp/output/`, keep-20), wired into `bash` behind a `filter` field via an additive `bash_with_filter` ctor (2-arg `bash` signature untouched → ~10 parser call sites unchanged), gated by `[context] output_filter` (default on) threaded through `build_registry`. No new deps. 14 new tests; executor 599 pass, mcp 242 pass. Qwen/Qwen3.6-27B-FP8 — **two prior dispatches hit infra SSE stream stalls (180s, zero code written); user re-tuned vLLM; third run completed all code + gates but not the Update-Log/commit step → architect closed out** (phase-04 / phase-06a precedent).
**phase-01 scope:** new `executor/src/context/output_filter` module (ANSI strip + consecutive-dup collapse + truncate-with-recovery-file under `.rexymcp/output/`, rotated to 20), wired into the `bash` tool's existing lossy truncation (`bash.rs:220` `truncate_output` → "full output not retained" becomes recoverable), gated by a `[context] output_filter` kill-switch (default on, mirrors `DashboardConfig`). Mostly additive: `bash()` signature kept stable (10 parser call sites untouched) via a sibling `bash_with_filter`; only `build_registry`'s 2 sites touched. Tags: rust/feature/m, ~250 lines. Executor target: Qwen/Qwen3.6-27B-FP8. The 8-site `LoopDeps` churn was avoided by hooking at the tool layer, not the loop.

**M10 thesis (2026-06-07):** the executor's context window is the scarce resource (local-LLM tokens are free; context isn't). Two arcs: **A** — filter tool/command output at the boundary, RTK-inspired but native + diagnostic-preserving (learn from `~/src/rtk`, do not shell out to it); **B** — novel semantic context lifecycle RTK structurally can't do (evict superseded file reads, dedupe re-reads, value-ranked compaction) built on the M4 read-before-edit working set. Everything scorecard-measurable. Three open questions for the user before phase-01: filter activation default, recovery-file location/retention, phase-02 first toolchain (cargo). **architecture.md § Status still needs an M10 entry** — a human-gated edit, add at formal kickoff with sign-off.

**phase-06 done** (2026-06-05, approved_first_try): replaced the paw-print spinner with a dog-chasing-brain animation (9 frames) in `transcript.rs`; 4 test assertions updated. Clean first-try via fully pre-injected verbatim patches — the executor never read the file. Qwen/Qwen3.6-27B-FP8.

**phase-05b done** (2026-06-05, escalated — architect session takeover after SSE-stall hard_fail): extracted `panels.rs`, `render.rs`, `event_loop.rs`; `mod.rs` shrinks to 141 lines; 828 tests pass unchanged.

**phase-05a done** (2026-06-05, escalated — architect session takeover after three SSE-stall hard_fails): extracted `filter.rs`, `highlight.rs`, `transcript.rs` from the 2098-line `dashboard.rs`; mod.rs shrinks to 1151 lines; 828 tests pass unchanged.

**phase-04 done** (2026-06-04, escalated — architect session takeover): pure
structural refactor extracting ~550 lines of private helpers from the 4 507-line
`mod.rs` into 4 new sibling modules (`log`, `tools`, `outcome`, `metrics`) and
extending 2 existing ones (`progress`, `command`). No logic changes; 585 tests
pass unchanged. Two bounces before takeover: dispatch-1 an architect spec gap
(broken Phase-A ordering — already-compiled `pub mod`s referenced new private
modules before they were declared — plus an incomplete `command.rs` import list);
dispatch-2 an executor `IdenticalToolCallRepetition` stall on Task 7b's mechanical
deletion churn (second occurrence of this class after phase-10b). M9 is now fully
complete (4/4 phases). See the [M9 README phase-04 addendum](milestones/M9-runtime-hardening/README.md#phase-04-addendum-2026-06-04--structural-refactor-escalated).
M8 is complete (all 16 phases done, 2026-06-04).

**M9 (executor runtime hardening) is complete.** All three
phases done (2026-06-04): post-write format hook (approved_after_2), lint_fix in the
hook (approved_after_1), read_file output cap (approved_first_try). Retrospective in
the [M9 README](milestones/M9-runtime-hardening/README.md#retrospective-2026-06-04).
M8's redesign also remains at its close gate, still human-gated. The user kicks off
the next milestone explicitly.

**phase-01 done** (2026-06-04, approved_after_2): runtime fix for the recurring
formatting hard-fail folded in WORKFLOW.md — a `run_format_hook` helper runs the
project's configured `format` command after every successful edit-class turn, before
the verifier (`mod.rs:671` call site + `mod.rs:1215` helper), so a later `write_file`
can't strand an unformatted file for the final `fmt --check`. Best-effort (failures
discarded), no config change, agent-loop only, +7 hermetic tests (574 total pass).
Qwen/Qwen3.6-27B-FP8. Two non-code bounces: dispatch-1 `RunawayOutput` on a 149 KB
whole-file read of `mod.rs` (architect spec gap — fixed by pre-injecting excerpts);
dispatch-2 SSE stall (infra) after the hook had already landed. **Calibration data
point (hold for recurrence):** re-dispatching against a dirty working tree let the
executor sweep unrelated changes into its commit — stash/commit ambient changes before
re-dispatch.

**phase-11b done** (2026-06-03, approved_first_try): Budget panel **"$ saved"** — added
a `[dashboard]` config section (`saved_input_per_mtok` / `saved_output_per_mtok`,
**configurable $/Mtok**, default 0.0 → "—"), wired config into the `dashboard` CLI
command (new `--config` arg + `Config::load_with_env`), threaded `BudgetRates` through
`run_dashboard → run_loop → render_dashboard`, and **appended** the `$ saved` line in
`render_dashboard` (`budget_lines` and its 5 tests left untouched — the additive shape
that dodged the 10b multi-edit churn). Cross-crate (executor config + mcp), no new deps.
567 tests pass. Qwen/Qwen3.6-27B-FP8 — clean first-try, consistent with single-concern
executor phases.

**phase-11a done** (2026-06-03): `summarize` tracks the prev+latest `Metrics` snapshot;
pure `tokens_per_sec(prev_ts, prev_out, last_ts, last_out)` yields `Δoutput/Δsec`;
`budget_lines` shows `tok/s:` (`—` until a 2nd metric). mcp-only, no deps. Clean
first-try in 32 turns — a counter-point to the phase-10b stall, consistent with keeping
executor phases single-concern. Qwen/Qwen3.6-27B-FP8.

**phase-11 was split into 11a/11b** (2026-06-03), because Tokens/Sec is JSONL-derived
while "$ saved" needs config plumbing the dashboard doesn't have (the `dashboard` CLI
command doesn't load `rexymcp.toml` today). The split also follows the phase-10b
calibration data point (keep executor phases single-concern).

- **11a (done):** Tokens/Sec — mcp-only, no config.
- **11b (next to draft):** "$ saved" — add a `[dashboard]` config section
  (`saved_input_per_mtok` / `saved_output_per_mtok` in `rexymcp.toml`, **configurable
  $/Mtok rate** per the locked 2026-06-03 decision — *not* a hardcoded model preset),
  load config in the `dashboard` CLI command (`executor/src/config.rs` is the `Config`
  struct; `Config::load_with_env`), thread the rates through `run_dashboard` →
  `render_dashboard` → `budget_lines`, add the "$ saved" line. Touches the executor
  crate (config schema) + mcp.

**phase-10b done** (2026-06-03, **escalated**): executor (Qwen/Qwen3.6-27B-FP8) wrote all
production code (record_lines multi-line + color, body_lines, visible_offset tail-follow,
render/run_loop wiring) correctly but stalled on the mechanical test-update churn
(`IdenticalToolCallRepetition` on patch); architect-takeover finished the test updates +
fixed a latent `clippy::useless_format`. 199 mcp + 565 executor tests pass. **Calibration
data point (hold for recurrence):** the local executor implements production code well but
stalls on repeated identical patch attempts during multi-edit test churn — if it recurs,
split implementation vs. test-update into separate phases.

**phase-11 (after 10b):** Budget panel **Tokens/Sec** + **"$ saved"**. *Still blocked
on:* (1) the **$-saved pricing baseline** — saved vs. which cloud model's $/token
(configurable rate? a specific model?); (2) tokens/sec data source (derive from
`Metrics` record ts deltas, or capture a per-turn duration?). Ask the user before
drafting.

**phase-10a done** (2026-06-03, approved_first_try): `load_records` raw-record reader
(refactored out of `load_status`, behavior-preserving), `records` on `DashboardData`,
one-plain-line-per-record transcript for all 12 event types (chronological), scroll
keys (Up/Down/PgUp/PgDn/Home/End) + pure `clamp_scroll`. Deleted the old
`activity_lines` summary. mcp-only. Qwen/Qwen3.6-27B-FP8. (Fold for future specs:
`ToolCall` carries an `origin: Origin` field.)

**Direct (non-executor) dashboard fixes shipped after phase-09** (`ff859ea`/`570251e`/
`33cfe45`/`e89b26b`): Session panel carries phase/session/model/state/turn/stage/age
(Heartbeat panel removed, header is 3 panels); Budget split into tokens-in/tokens-out;
Files left-trim guarantees the `+N -N` numstat is always visible (`FILE_LINE_MAX=28`).

**Still pending confirmation on a *live* session:** phase-08 auto-attach, phase-09
layout, and now the phase-10a transcript + scrolling (none verifiable headlessly).

**Redesign roadmap (from the wireframe received 2026-06-03):**
- **phase-09 (done):** header-band layout (Session · Budget · Compactions · Heartbeat
  over Activity · Files) + Compactions panel (renders phase-07 data) + Files left-trim.
- **phase-10 (next to draft):** the big one — Activity panel becomes a **scrollable
  transcript**. **Decisions locked with the user (2026-06-03):**
  - **Scope = Everything (full replay).** All event types are scrollable items:
    `Prompt`, `Completion` (agent thought), `Parsed`/`ToolResult` (+ tool output),
    `Verify`, `ParseFailed`, `HardFail`, `Compaction`, `Progress`, `Metrics`,
    `SessionStart`/`SessionEnd`. Not just tool/agent activity — the raw transcript.
  - **Split 10a / 10b:**
    - **phase-10a** — raw-record reader (read the full record stream, *not* the
      distilled `summarize`) + scroll-key handling in `run_loop` (first real
      interactivity: up/down/pgup/pgdn, scroll state) + **plain-text** item rendering
      for every event type.
    - **phase-10b** — per-event JSON parsing + color formatting + tool-output
      rendering on top of 10a's plain-text items.
  - Note for drafting: `summarize` distills; the transcript needs the raw
    `Vec<SessionRecord>`. `load_data`/`DashboardData` currently carry only
    `StatusSummary` — 10a must thread the raw records (or a rendered transcript)
    through to the renderer. Keep the existing summary panels working unchanged.
- **phase-11:** Budget panel gains **Tokens/Sec** and **"$ saved"**. *Blocked on two
  decisions before drafting:* (1) the **$-saved pricing baseline** — saved vs. which
  cloud model's $/token (configurable rate? a specific model?); (2) tokens/sec data
  source (derive from `Metrics` record ts deltas, or capture a per-turn duration?).

**phase-09 done** (2026-06-03, approved_first_try): four-panel header-band layout +
Compactions panel (`summarize` folds `SessionEvent::Compaction` → count + token sums;
panel shows events/freed/ratio with a divide-by-zero guard) + Files left-trim
(`trim_path_left`, `FILE_PATH_MAX=40`). mcp-only, no new deps. Implemented by
Qwen/Qwen3.6-27B-FP8 (Update Log mislabels itself "Claude (direct)").

**Still pending confirmation:** the phase-08/09 dashboard changes on a *live* session —
watch the auto-attach and the new layout render when the next executor session starts
(neither is verifiable headlessly).

**phase-07 done** (2026-06-03): captured the `CompactionReport` at the `compact()`
call site (`agent/mod.rs:182`) and logged a new `Compaction` variant (tokens
before/after + messages signaturized/evicted). Emit-only; 4 additive sites (enum +
emit + `event_type_str`/`event_kind` arms), executor + mcp crates, no new deps.
Implemented by Qwen/Qwen3.6-27B-FP8.

**phase-08 done** (2026-06-03): dashboard stays open until user-quit and auto-follows
a newly-started session; removed phase-01's auto-exit-on-`ended` and extracted a
testable `resolve_session_log`. Implemented by Qwen/Qwen3.6-27B-FP8.

**phase-08 done** (2026-06-03): fixed phase-01's auto-exit-on-`ended` that made
`rexymcp dashboard` flash up and exit when no phase was running. Removed the auto-exit
block (Option A) so the loop only quits on `q`/`Esc`/`Ctrl-C`, and extracted the
per-poll log resolution into a testable `resolve_session_log` (unpinned follows the
newest log → auto-attaches to a new session; `--session` pin stays put). mcp-crate
only, no new deps, 5 new resolution tests. Implemented by Qwen/Qwen3.6-27B-FP8.

**phase-06b done** (2026-06-03): the Budget panel — `summarize` folds
`SessionEvent::Metrics` into `StatusSummary` (`last_input_tokens`, `last_output_tokens`,
`last_context_pct`); the dashboard gained a fifth full-width Budget panel with token
counts and a colored context-window gauge (green <50 / yellow 50–80 / red ≥80;
`0.0` = unmeasured). Verdict: approved_first_try (Qwen/Qwen3.6-27B-FP8, clean, no infra
drop). mcp-only.

**phase-06a done** (2026-06-02): `SessionEvent::Metrics { input_tokens, output_tokens,
context_pct }` added to the enum and emitted once per turn right after the `Completion`
record. `Budget::fraction_used` computes `context_pct`; `cap.rs` catch-all passes the
new variant through. Verdict: approved_first_try via architect closeout of a third
infra hard_fail (backend drop at turn 109, post-implementation). The one spec error
was the test budget (`1_000` → `100_000`). Implemented by Qwen/Qwen3.6-27B-FP8.

**phase-05 done** (2026-06-02): buffer-then-flush + mid-stream connection drop retry
— closes `bug-executor-2`. The OpenAI backend now buffers the completion and emits
only on stream-success; transient transport errors (identified via
`e.downcast_ref::<reqwest::Error>()`) trigger up to 3 bounded retries (250ms/500ms/
1s backoff) instead of aborting. `bug-05-1` (non-hermetic test adding ~30 s to the
suite) fixed on re-dispatch. Verdict: approved_after_1. Implemented by
Qwen/Qwen3.6-27B-FP8.

**phase-04 done** (2026-06-02): the Activity panel — `summarize` now folds the
`ParseFailed` / `Verify` / `ToolResult` / `HardFail` records it previously dropped
(`_ => {}`), `StatusSummary` carries six new activity fields, and the dashboard shows
a fourth Activity panel (2×2 grid). Closes the "parse/verifier signal" Exit criterion.
Verdict: approved_first_try, **implemented cleanly by Qwen/Qwen3.6-27B-FP8** (a
positive scorecard data point — ~205-line multi-file feature, no bounce); architect
closed out the commit after a transient backend drop (`error decoding response body`)
aborted the run at the e2e step, post-gates. mcp-crate only; `format_status` unchanged.

**phase-06b (drafted after 06a):** the Budget panel — the render half of the "budget
consumed" Exit criterion. mcp-only, mirrors phase-04: `summarize` folds the new
`Metrics` record into `StatusSummary` (tokens + context %), and the dashboard adds a
Budget panel. Highest-value metric it unlocks: live context-window utilization
("68% full, +4%/turn") — the overflow/compaction early-warning gauge.

**Measurement roadmap (designed 2026-06-02 — see [M8 README Notes](milestones/M8-dashboard/README.md#notes)).**
The system measures rich metrics at run-end (`PhaseRun`, the scorecard substrate) but
flushes almost none to the live JSONL the dashboard reads. Three gap classes:
**A — surfacing** (data in JSONL, `summarize` drops it) → **phase-04** (done).
**B — capture** (token/context computed in `RunMetrics`, only in end-of-run
`PhaseRun`; needs a per-turn `SessionEvent::Metrics` emit) → **phase-06a** (executor
emit) + **phase-06b** (Budget panel). **C — unmeasured anywhere** (live context-window
%, and compaction firings — `compact()` emits nothing) → phase-06a (`context_pct`) and
**phase-07** (`SessionEvent::Compaction`). (phase-05 was the unrelated executor
retry-resilience fix.) The unifying move for B/C: flush
incremental metric snapshots to the JSONL, giving the live dashboard parity with the
scorecard and enriching the JSONL as a forensic replay record.

**phase-03 done** (2026-06-02): closed `bug-executor-1` — the agent loop's
`ParseResult::NoToolCall` branch now distinguishes a think-block-only completion
(routed to the parse-failure feedback loop) from a genuine prose clean exit. Two new
tests. Verdict: approved_after_1 (first dispatch hard-failed `RunawayOutput` on a
140 KB whole-file read; refined re-dispatch with code pre-injected verbatim succeeded
on Qwen/Qwen3.6-27B-FP8).

**phase-02 done** (2026-06-02): split phase-01's single dashboard pane into a
btop-style three-panel layout (Session · Heartbeat · Files) via
`ratatui::layout::Layout`. Reuses `status::humanize_age` (bumped to `pub(crate)`).
Parse/verify + budget panels deferred (data not in `StatusSummary`). Verdict:
escalated (architect takeover — Qwen3.6-35B-A3B-FP8 produced three false-`complete`
no-ops; root cause = [bug-executor-1](milestones/M8-dashboard/bugs/bug-executor-1.md),
fixed by phase-03). `rexymcp status` unchanged.

**phase-01 done** (2026-06-02): `rexymcp dashboard --repo <path>` scaffold —
`ratatui` live TUI, 500 ms poll of the latest session JSONL, single bordered
`StatusSummary` pane, `q`/`Esc`/`Ctrl-C` exit, auto-exit on `ended`. Bounced once
([bug-01-1](milestones/M8-dashboard/bugs/bug-01-1.md): the authorized `crossterm
0.28` pin couldn't unify with `ratatui 0.30`'s `crossterm 0.29`, leaving two
crossterm copies — fixed by aligning to `crossterm = "0.29"`; an architect spec
error, not an executor miss). Verdict: escalated (architect-direct takeover fix
after a backend-glitched no-op re-dispatch). `rexymcp status` unchanged.

**M7 done** (2026-06-02): per-run statistics substrate complete (runs, scorecard,
provenance). One WORKFLOW fold (additive change shapes). Architecture.md and
README.md realignment done. All open follow-ups cleared.

**M8 design:** `ratatui` + `crossterm`; two phases (scaffold → btop panels);
`rexymcp status` preserved for scripting/CI; read-only, no side effects, hermetic
data layer (tests exercise `load_data` without a real terminal).

**Phase-05 split history (2026-06-02):** the original combined phase-05 was split at
draft time into **05a (settings — done)**; then 05b was itself split into **05b
(chat-stream provenance: served model + `finish_reason` — this)** and **05c (context
window via `/v1/models`)**, because the chat-stream values share the `AiEvent::Done`
plumbing while `max_model_len` comes from a separate source. 06 (the `model ×
settings` / provenance scorecard slice) depends on 05a/05b/05c.

**Per-run statistics plan (designed 2026-06-02 with the user):** 04 = the
read-only `rexymcp runs` view (done). 05a = settings plumbing — make
`generation_params` real (configurable, sent, recorded; default `None` today).
05b = chat-stream provenance — served model id (chat response `model`) +
`finish_reason` (esp. `length`-truncation rate), both via a new `AiEvent::Done`
field. 05c = context window (`max_model_len` from `/v1/models`), a separate source.
Quantization/params are **out** (not portably exposed by the OpenAI API). 06 = a
`model × settings` (and provenance) slice on the
scorecard (depends on 05a/05b/05c). Surface decision: CLI (matches "users see detailed statistics" +
the existing `rexymcp status` pattern); an MCP `list_runs` tool can come later.

**Direction change (2026-06-02).** The benchmark-suite approach is dropped. The
scorecard concept is **kept**, but it will track **regular rexyMCP runs**, not
specialized benchmark runs. New goal: let users see detailed statistics for each
rexyMCP run so they can decide which local LLM to use and which settings work
best for it. Phases **02 / 03a / 03b** were rolled back — benchmark code reverted
(`971d0c4` phase-03a, `dc5b6be` phase-02), the unlanded 03b sweep discarded, and
the three phase docs banner-marked `rolled-back`. The `bench_suite` field on
`PhaseRun`, the scorecard `SourceFilter`, the `LoopDeps`/CLI threading, and the
sweep are all gone; `PhaseRun` + scorecard are back to their post-phase-01 state.

**Open follow-ups for the redesign:**
- `docs/architecture.md` § "Model effectiveness metrics & routing" still carries
  the "Benchmark vs. telemetry" + automated-routing language — needs an architect
  pass to realign with the per-run-statistics direction.
- Pre-existing red tests unrelated to the rollback: `config.rs` commit `6282060`
  bumped `stream_idle_timeout_secs` default 90→180 but left
  `config_defaults_first_token_and_idle_timeouts` (`config.rs:309`) and
  `config_omits_timeouts_keeps_defaults` (`config.rs:365`) asserting `90`. Two
  failing tests; fix the asserts to `180` (or whatever final value) before the
  next phase is reviewed.

**M6 closed** via [phase-06b — dogfood execution + retrospective +
close](milestones/M6-plugin/phase-06b-dogfood-close.md). The ms_pacman dogfood
(bootstrap + design, 5/5, no dispatch) was user-confirmed sufficient; the two
breakages it surfaced (tools-not-advertised `b78a081`; live-progress-can't-fire
`c4567fb`+`3374336`) are fixed. Full retrospective in the
[M6 README Notes](milestones/M6-plugin/README.md#notes).

**Decisions carried into M7** (the 07a/07b deferrals + compaction, decided in
06b):

1. **Terminal backend `Err` → `hard_fail` (yes, conditional).** A mid-phase
   terminal model error (after ≥1 turn of progress) should degrade to a
   `hard_fail` `PhaseResult` with briefing + partial work, instead of aborting
   `execute_phase` as it does today (`executor/src/agent/mod.rs:238` and
   `:271-273`, with the `:1545` test pinning the current abort). Pre-work
   connection errors stay `Err`. **This is the one decision with a code
   follow-up — an M7-adjacent implementation phase, not yet drafted.**
2. **Resume / `continue_phase` (no).** Stays an uncommitted architecture
   candidate; re-dispatch-with-refined-spec remains the default. Revisit only if
   `PhaseRun` telemetry shows a recurring high-progress / single-blocker pattern.
3. **Compaction monitoring (insufficient data).** No dispatch → no
   `CompactionReport`; keep the heuristic compactor; gather data on the first
   small-context (32k–128k) dispatch. No summarization milestone justified.

**Architecture amended in 06b:** Layer 2 § Liveness reworded push→pull —
`rexymcp status` is the human-liveness path; MCP progress is spec-correct but
unreachable with Claude Code's current client.

**Already-landed calibration fold (recorded in 06b):** an earlier run hit
`budget_exceeded` at the turn cap mid-verification; default `max_turns` raised
40 → 200 in `executor/src/config.rs` and the architect bootstrap template
(`plugin/skills/architect/SKILL.md`), since the executor runs against a local
LLM with no token cost. Per-project `[budget] max_turns` was already
configurable; only the defaults moved.

**Last completed:** [M7 / phase-01](milestones/M7-scorecard/phase-01-backend-error-degradation.md)
— approved_first_try 2026-06-01. (phase-02/03a/03b rolled back 2026-06-02 —
benchmarking deprecated.)

**Milestone:** [M7 — Per-run statistics & model scorecard](milestones/M7-scorecard/README.md)
— in progress (M1–M6 done; M7 phase-01 done; benchmarking dropped; per-run
statistics direction designed → phases 04/05/06; phase-04 active).

**Queued (after M7):** **M8 — Live session dashboard.** A `rexymcp dashboard` CLI
command: a real-time, read-only TUI over the live session JSONL (the same source
`rexymcp status` reads), recorded in `docs/architecture.md` § Status. **Why it's
important:** a blocking `execute_phase` call is opaque through Claude Code's MCP
interface (no `progressToken` → no progress notifications), so the user is blind
to a running phase mid-flight; the dashboard gives deep live insight into the
ongoing MCP session. Not yet expanded into phases — milestone boundaries are a
human gate. Note: this refined the "No terminal UI" non-goal to "no interactive
TUI *agent*; a read-only live dashboard is allowed."

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next phase doc.

---

**M5 retrospective + folds at a glance** (for the M6 kickoff briefing):

- Seven phases: 01 / 02 / 03 / 04 / 05a / 05b / 06. Six approved_first_try;
  one bounced once ([bug-05b-1](milestones/M5-mcp-server/bugs/bug-05b-1.md),
  verified). 629 total tests (started M5 at 492 executor + 0 mcp; ended at
  512 executor + 117 mcp).
- Six tools live: `execute_phase`, `executor_health`, `executor_log_search`,
  `executor_log_tail`, `get_turn`, `model_scorecard`. Plus the full progress
  consumer split (live MCP `notifications/progress` for the human + logged
  `Progress` events for Claude's post-return queries) and target-repo-root
  corroboration.
- Two calibration folds added to WORKFLOW.md: *Wrap-vs-derive at protocol
  boundaries* (extending `### Derive intentionally`) and *Anticipate
  cross-boundary trait bounds* (new subsection). Five-recurrence threshold
  reached on the latter.
