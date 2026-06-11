# M13 — Dashboard Polish

**Goal:** Make the live `rexymcp dashboard` presentable — fix legibility
(contrast, wrapping, autoscroll), surface the captured-but-hidden Activity
payloads (injected context, tool-call arguments, `<think>` blocks), and polish
the Session/Budget/Tasks panels — without touching the executor, the loop, the
config, or the `SessionEvent` schema.

**Status:** done

**Depends on:** M8 (dashboard wireframe), M10 (Reclaim panel), M12 (Tasks panel)
— all complete.

## Why now

The dashboard is the user's only window into an otherwise-opaque, blocking
`execute_phase` run (the MCP client sends no `progressToken`, so live progress
notifications never fire — see `architecture.md` Layer 2 § "Liveness"). It works,
but it's visually rough: dark-grey secondary text is hard to read, long Activity
lines run off the panel edge, the transcript hides the prompt text and tool-call
arguments it already has in the feed, and the Session/Tasks panels under-use the
data they hold. This milestone is pure presentation — every byte it renders is
already in the JSONL log.

## The load-bearing constraint: display only

**M13 changes how the existing feed is displayed; it does not change the feed.**
No new `SessionEvent` variants, no config, no executor/loop/governor edits. This
deliberately sidesteps the two stall classes the calibration history flags (NEXT.md):

- **The new-`SessionEvent`-variant exhaustive-match wall** (`dashboard/filter.rs`'s
  seven per-event-kind sites, `transcript.rs::record_lines`,
  `log_query::event_type_str`/`event_kind`) — M13 adds **no** variant, so this wall
  is never touched.
- **The cross-crate `LoopDeps`/struct-literal churn** (phase-08a/08d) — M13 adds
  fields only to `StatusSummary`, which **derives `Default` and is built mutably
  in `summarize()`** (`status.rs:18,99`), so a new field is a one-line struct add +
  one `summarize` assignment, **not** an N-site literal cascade. Confirm at draft
  time that no full `StatusSummary { … }` literals exist in production (tests use
  `..Default::default()` or build via `summarize`); if any are found, treat it as a
  watch-item and pre-inject the complete site list.

## Phases

Run roughly in order; phases are largely independent (they touch different panels /
files) so the order is convenience, not dependency, except where noted. The
architect expands each phase doc on demand (`/rexymcp:architect next`), not all at
once.

| Phase | Title | Status | Kind | Size | Items |
|---|---|---|---|---|---|
| 01 | Legibility — raise all `Color::DarkGray` text to `Rgb(200,200,200)` ([phase-01-contrast.md](phase-01-contrast.md)) | done | feature | s | #1 |
| 02 | Activity — surface injected context (`Prompt.rendered`) + tool-call arguments (`Parsed.tool_call.arguments`) ([phase-02-payloads.md](phase-02-payloads.md)) | done | feature | s | #2, #3 |
| 03 | Activity — line wrapping + tail-follow autoscroll over wrapped lines + scrollbar ([phase-03-wrapping.md](phase-03-wrapping.md)) | done | feature | m | #8, #9, R1 |
| 04 | Activity — distinct `<think>`/`</think>` block formatting in Completion bodies ([phase-04-think.md](phase-04-think.md)) | done | feature | m | #6 |
| 05 | Session/Budget — session `duration:` + move `last update:` to Budget (new `started_at` capture) ([phase-05-timing.md](phase-05-timing.md)) | done | feature | m | #4, #5 |
| 06 | Session — full-width spinner on its own bottom line ([phase-06-spinner.md](phase-06-spinner.md)) | done | feature | m | #10, R5 |
| 07 | Tasks — named tasks with checkbox/check glyphs + done/total progress gauge ([phase-07-tasks.md](phase-07-tasks.md)) | done | feature | m | #7, R3 |
| 08 | Activity — per-event relative timestamps ([phase-08-timestamps.md](phase-08-timestamps.md)) | done | feature | s | R2 |

The **Items** column maps each phase to the user's original request list (#1–#10)
and the agreed enhancements (R1 scrollbar, R2 timestamps, R3 task gauge, R5 spinner
status text; R4 — dim tool-call arguments — is folded into phase-02 as styling).

## Exit criteria

- [ ] No `Color::DarkGray` remains in `mcp/src/dashboard/`; all former dark-grey
      text renders at `Rgb(200,200,200)`.
- [ ] The Activity transcript shows the full injected-context (`Prompt.rendered`)
      text and the tool-call `arguments`, both truncation-bounded by the existing
      body machinery, gated by the existing `prompt` / `tool_call` filter toggles.
- [ ] Long Activity lines wrap within the panel (no horizontal overflow); the
      transcript tail-follows new records over the **wrapped** line count, and a
      scrollbar shows position.
- [ ] `<think>` reasoning in a Completion body is visually distinct from the
      answer text.
- [ ] The Session panel shows `duration:` (live-growing while running, fixed once
      ended); `last update:` appears in the Budget panel; the spinner spans the
      Session panel width on its own bottom line, with the `turn N, stage X` line
      directly above it (per the 2026-06-10 layout decision — see Notes).
- [ ] The Tasks panel lists task names with `☑`/`☐`-style glyphs and a done/total
      gauge; counts remain correct.
- [ ] Each Activity line carries a relative timestamp.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Non-goals

- **No feed changes.** No new `SessionEvent` variants, no schema changes, no
  executor/loop/governor/config edits. Display only.
- **No new interactivity.** The scroll/filter keys stay as they are; the dashboard
  remains a monitoring view, not an agent surface (M8 Non-goals).
- **No new dependencies.** `ratatui` already provides `Wrap` and `Scrollbar`.
- **No E2E harness for the TUI.** Consistent with prior dashboard-panel phases
  (M8/M10/M12 phase-07), TUI rendering has no headless harness; phases assert on
  the pure line-builder functions (`*_lines`, `record_lines`, `transcript_lines`,
  wrap/scroll helpers) and declare end-to-end N/A with the standard reason.

## Notes

### Kickoff decisions (2026-06-10, with the user)

- **Scope = all 10 requested dashboard improvements**, decomposed into 8
  single-concern phases (combining only where the *mechanism* is shared:
  context+args render together in 02; wrap+autoscroll are coupled because the
  follow math reads the wrapped line count, so they ship together in 03; move
  last-update + add duration are both freshness/timing lines in 05).
- **Four enhancements folded in** (the user selected all offered): R1 scrollbar
  (→ 03), R2 per-event timestamps (→ its own phase 08), R3 Tasks progress gauge
  (→ 07), R5 spinner status text (→ 06). R4 (dim tool-call arguments) is folded
  into 02 as styling, not its own phase.
- **On-demand drafting.** Only phase-01 is drafted at kickoff; later phases are
  drafted via `/rexymcp:architect next` as the user dispatches, so each spec is
  informed by the prior phase landing.

### Phase-06 layout decision (2026-06-10, with the user)

The spinner (#10/R5) is a **full-width line on its own bottom row** of the Session
panel, with the `turn N, stage X` status text on the line **directly above** it
(left-aligned with the rest of the panel) rather than inline on the spinner line. To
make room, the header band grows one row (`Length(9)` → `Length(10)`); the body is
`Constraint::Min(0)`, so the Activity panel and the Tasks/Files column each yield one
row automatically. The fixed-window `SPINNER_FRAMES` walk is retired in favor of a
width-aware dog that trots the full panel width.

### Pre-injection watch-items for the drafting architect

- **No new `SessionEvent` variant in this milestone** — if a phase ever seems to
  need one, stop: it has left M13's display-only scope. The whole point is that
  the data already exists.
- **`StatusSummary` field adds are cheap** (Default-built; one `summarize`
  assignment) — but grep for production `StatusSummary { … }` literals before
  drafting 05/07 and pre-inject any that exist.
- **Quote the real ratatui call shapes** for `Paragraph::wrap(Wrap { trim: false })`
  and `Scrollbar`/`ScrollbarState` as worked examples in 03 — do not say "use
  ratatui wrapping." The autoscroll fix hinges on computing the **wrapped** line
  count (ratatui's `Paragraph::scroll` operates on post-wrap lines, but the current
  `visible_offset`/`clamp_scroll` math in `render.rs:26-38` counts pre-wrap
  `transcript_lines().len()`); pin that as the core behavior, with a pinned test
  that a long line that wraps to N rows advances the follow offset by N, not 1.
- **`<think>` formatting (04)** is greenfield — there is no existing `think`
  handling anywhere in `mcp/src/`. Pin the parsing behavior (split on the literal
  `<think>`/`</think>` markers; handle an unterminated/`</think>`-only body) with
  explicit negative cases (a body with no think tags renders byte-identically;
  a `</think>` with no opening `<think>` still separates).
- **Reuse the existing truncation machinery** for 02 (`body_lines` /
  `TRANSCRIPT_CONTENT_MAX_LINES` in `highlight.rs`, `preview()` /
  `TRANSCRIPT_PREVIEW_MAX` in `transcript.rs`) rather than inventing new caps —
  quote them as worked examples.

### Carried in from the M12 retrospective (not M13 scope, gather separately)

The M12 README flagged a deferred cleanup sweep (two prod `eprintln!` at
`server.rs:426`/`:450`; the stale `RUNAWAY_OUTPUT_BYTES` doc-comment in
`read_file.rs:17`; the symbols `format_references` truncation-note copy bug). None
are dashboard-related; they are **not** part of M13 — gather into a separate
micro-phase if/when the user wants it.

**Operational follow-up still open (do before the first M13 dispatch):** restart
`rexymcp serve` so the rebuilt binary picks up M11 phase-06's datetime injection,
ending the executor's hallucinated-date self-stamping in Update Logs (cosmetic;
machine records are correct).

### Retrospective — 2026-06-10

**M13 — Dashboard Polish is complete.** All **8 phases approved_first_try, zero
bounces, zero escalations** (Qwen/Qwen3.6-27B-FP8 executor) — the cleanest milestone
to date alongside M11. All 10 requested dashboard improvements (#1–#10) plus the four
agreed enhancements (R1 scrollbar, R2 timestamps, R3 task gauge, R5 spinner) shipped.
Every exit criterion is met.

**What worked — the display-only constraint thesis held end to end.** The milestone's
central bet (NEXT.md kickoff): confine M13 to the presentation layer
(`mcp/src/dashboard/` + read-only `StatusSummary` adds), touching **no** `SessionEvent`
variant, loop, governor, or config. This deliberately sidestepped both documented
stall classes, and **neither recurred across all 8 phases**:

- *The new-`SessionEvent`-variant exhaustive-match wall* was never approached — M13
  added no variant. Phase-08 even surfaced an existing-but-unread field
  (`SessionRecord.ts`) rather than adding one, the same move as phase-07 (`TaskUpdate.title`).
- *The cross-crate `LoopDeps`/struct-literal churn* was avoided — the only struct
  growth (05 `started_at`, 07 `tasks: Vec<TaskRow>`) landed on `Default`-built
  `StatusSummary`, a one-line add + one `summarize` assignment each.

**What worked — additive, one-layer-up shapes kept blast radius tiny.** The two phases
that could have forced multi-site signature churn (06 dropping `spinner` from
`session_lines`; 08 adding a timestamp to every header) were both speccable as
*additive / one-layer-up* changes: 06 moved the spinner into a new `render.rs`-composed
`spinner_line` (the `dollars_saved_line`/`last_update_line` precedent), and 08 prepended
the timestamp in `transcript_lines` so `record_lines`'s signature and its ~15 test call
sites stayed untouched. This is the WORKFLOW § "Prefer additive change shapes" guidance
applied at draft time, and it paid off as cleanly as the M12 06a/06b/06c split did.

**What worked — reuse over reinvention, pinned as worked examples.** Recurring wins:
05/08 reused `humanize_age`; 07's gauge reused the context-gauge *style*; 02 reused
`body_lines`/`preview` truncation; 04 reused the `body_lines` shape for byte-identical
no-marker output. Each was pinned in the phase doc as a quoted worked example with a
"do the same shape" instruction — the highest-leverage pre-injection form, and the
through-line behind the first-try rate.

**Calibration — no new folds.** Per WORKFLOW § Calibration (one occurrence is data,
two a trend, three a fix), M13 surfaced no new recurring pattern warranting a
STANDARDS/WORKFLOW edit:

- *Dirty-tree-at-dispatch* (phase-03, phase-06 calibration notes) did **not** recur in
  07 or 08 — the architect committed the draft *before* dispatch both times, and the
  executor committed cleanly on its own. The existing lesson was simply applied; no
  doc change needed.
- *Local-LLM clock/identity self-stamping* in Update Logs persisted every phase
  (cosmetic; machine records correct). This is **not** a doc fold — it is the known
  operational item below: the fix (M11 phase-06's datetime injection) is already in the
  binary and lands the moment `rexymcp serve` is restarted. **This restart never
  happened during M13** and remains the one open operational follow-up.

**Open items carried out of M13 (not M13 defects):**

1. **Restart `rexymcp serve`** to activate M11 phase-06's datetime injection and end
   the cosmetic self-stamping. Outstanding since M11; still the single highest-value
   operational action.
2. **The M12 deferred cleanup sweep** (two prod `eprintln!` at `server.rs:426`/`:450`;
   stale `RUNAWAY_OUTPUT_BYTES` doc-comment in `read_file.rs:17`; the `symbols`
   `format_references` truncation-note copy bug) remains ungathered — a candidate
   micro-phase whenever the user wants it.
3. **Optional A/B analysis** of `task_tracking` on/off (`bounces_to_approval` /
   `first_pass_rate`) via the scorecard, available whenever the user wants to validate
   M12 Arc A.

**Next:** M13 is at a milestone boundary — a human sign-off gate. `NEXT.md` is set to
"none". The user kicks off the next milestone (or a cleanup micro-phase) explicitly.
