# Phase 05b: fold the async-poll + interrupt model into the contract docs

**Milestone:** M30 — Executor Interruption
**Status:** done
**Depends on:** phase-05a (the skills already describe the async/interrupt model)
**Estimated diff:** ~70 lines (Markdown docs only)
**Tags:** language=markdown, kind=refactor, size=s

## Goal

Bring the **contract docs** into line with what M30 shipped and phase-05a wired
into the skills: `execute_phase` is now an **async job** (poll `get_run_status`),
a running phase is **interruptible** (`rexymcp stop` / `stop_phase` → `cancelled`),
and the autonomous loop has a **fifth stop condition**. The code + skills already
carry this; this phase updates the prose that documents them so the docs don't
lie. **Last phase of M30** before the milestone-boundary human gate.

## Execution note

**Direct execution (Claude-authored).** Pure contract-doc editing — the
architect's domain, done with user sign-off (the phase-05 talk-through). No
`execute_phase` dispatch.

## Current state (the four descriptive sites, verbatim anchors)

1. **`docs/architecture.md` § Layer 2 "Liveness" (~line 441)** — ends: "…the
   opacity of a **blocking** `execute_phase` call is exactly what leaves the user
   without insight mid-phase, and a one-shot `status` only partly answers it." The
   "blocking" framing is now superseded (M30 made it async + interruptible).
2. **`docs/architecture.md` § Layer 2 skills list (~line 488)** — "`dispatch` —
   thin glue around `execute_phase`: pre-flight `executor_health`, …" (no mention
   of polling or `cancelled`).
3. **`docs/dev/WORKFLOW.md` § "Opt-in autonomous loop" (~line 445-461)** — "The
   loop stops for the human on: a milestone boundary (always), any blocker …,
   exhaustion of the per-phase assist budget …, or the loop-level runaway
   backstop." — **four** stop conditions, no async/interrupt mention.
4. **`plugin/templates/WORKFLOW.md` (~line 387)** — the generalized mirror of (3);
   must stay in sync (WORKFLOW.md § intro: "Keep the two in sync").

## Spec

Additive/corrective prose only — no restructuring.

### 1. `architecture.md` § Liveness — async + interruptible

Amend the closing sentences of the "Liveness (pull, not push)" bullet so it
reflects M30: `execute_phase` is now an **async job** — the architect is no longer
blocked inside one long call; it **polls `get_run_status`**, so mid-phase blindness
is bounded by the poll cadence, and the run is **interruptible out-of-band**
(`rexymcp stop` sentinel for the human, `stop_phase` for the architect between
polls → a `cancelled` `PhaseResult` with the partial diff). Keep the
`progressToken`/`rexymcp status`/`dashboard` facts (still true); just retire the
"blocking" framing.

### 2. `architecture.md` § skills list — `dispatch` async note

Extend the `dispatch` bullet: it **drives the async `execute_phase` contract**
(detect-and-adapt: poll `get_run_status` on a `run_id`, or use a direct
`PhaseResult` from the old-serve / `run-phase` path) and **surfaces the
`cancelled` outcome**. One clause, not a rewrite.

### 3. `WORKFLOW.md` § "Opt-in autonomous loop" — fifth stop + async/interrupt

Two changes to the paragraph:
- Add a sentence: the loop **drives `execute_phase`'s async contract** — dispatch
  polls `get_run_status` to reap each run — and a running phase is
  **interruptible** (`rexymcp stop` / `stop_phase`), which the loop treats as a
  deliberate human signal.
- Extend the stop-condition list from four to **five**: "…, the loop-level runaway
  backstop, **or a phase returning `cancelled`** (a deliberate `rexymcp stop` /
  `stop_phase` interrupt — the loop surfaces the partial work and hands back)."

### 4. `plugin/templates/WORKFLOW.md` — mirror change 3

Apply the **same** two edits to the generalized template paragraph so the mirror
stays in sync. (The template is project-agnostic — keep any rexyMCP-specific
phrasing out; describe the capability generically, matching the surrounding
template tone.)

## Acceptance criteria

- [ ] `architecture.md` § Liveness no longer calls `execute_phase` "blocking" as a
      present-tense property; it describes the async poll + interrupt model.
- [ ] `architecture.md` `dispatch` skill bullet mentions the async poll + `cancelled`.
- [ ] `WORKFLOW.md` autonomous-loop paragraph lists **five** stop conditions
      (incl. `cancelled`) and mentions the async/interrupt model.
- [ ] `plugin/templates/WORKFLOW.md` carries the same five-condition + async change
      (mirror in sync).
- [ ] No other doc claims `execute_phase` is synchronous/blocking as current design
      (historical Status entries describing *past* milestones may keep their
      then-accurate wording — do not rewrite history).
- [ ] `cargo build` / `cargo test` green (sanity — no Rust changed).

## Test plan

Not applicable (prose). Verify by inspection against the acceptance criteria +
the greps below.

## End-to-end verification

- `grep -n "blocking" docs/architecture.md` — confirm the remaining hits are
  historical Status entries or the CLI `run-phase` (which **is** still blocking),
  not the § Liveness present-tense claim.
- `grep -n "cancelled" docs/dev/WORKFLOW.md plugin/templates/WORKFLOW.md` — the
  fifth stop condition is present in both.
- Diff `docs/dev/WORKFLOW.md` vs `plugin/templates/WORKFLOW.md` autonomous-loop
  paragraphs — the async/five-stop content matches (allowing project-agnostic
  wording in the template).
- `cargo build` + `cargo test` once (no Rust touched).

## Authorizations

- [ ] May touch `docs/architecture.md`: **Yes** — this phase's deliverable
      (user-authorized via the phase-05 design talk-through).
- [ ] May touch `docs/dev/WORKFLOW.md` + `plugin/templates/WORKFLOW.md`: **Yes** —
      same.
- [ ] May add dependencies: **No** (no code).

## Out of scope

- **Rewriting historical Status entries** (e.g. the M8 "opaque *and* blocking"
  rationale) — those were accurate for their milestone; leave them.
- **The § Status #30 roadmap status marker** — flipping M30 to done there is
  **milestone-close** work (the boundary human gate: retrospective + calibration
  fold), not phase-05b.
- **STANDARDS.md** — untouched (no DoD change).

## Update Log

(Architect-authored; direct execution.)

<!-- entries appended below this line -->

### Update — 2026-07-10 (complete, architect direct execution)

**Summary:** Folded M30's async-poll + interrupt model into the contract docs
across the four sites. `architecture.md` § Liveness: the M8 rationale is now
past-tense ("then-blocking … was"), followed by an "As of M30, `execute_phase` is
an async job" paragraph (poll `get_run_status`; interruptible via `rexymcp stop` /
`stop_phase` → `cancelled`). `architecture.md` `dispatch` skill bullet: added the
async detect-and-adapt + `cancelled` clause. `WORKFLOW.md` § "Opt-in autonomous
loop": added the async/interrupt sentence and extended the stop-condition list to
**five** (incl. `cancelled`). `plugin/templates/WORKFLOW.md`: the same two edits,
mirror kept in sync.

**Acceptance criteria:** all met.

**End-to-end verification:** `grep "blocking" docs/architecture.md` — the remaining
hits are the genuinely-blocking CLI `run-phase`, the past-tense M8 § Liveness line,
and historical § Status narrative (M30 "instead of a single blocking call", "still-
blocking CLI run-phase", "supersedes the opaque *and blocking*") — no present-tense
claim that the async `execute_phase` blocks. `grep -c cancelled` = 1 in each
WORKFLOW file (the fifth stop condition). `cargo build` + `cargo test` green (512
mcp + 949 executor) — no Rust changed.

**Files changed:**
- `docs/architecture.md` — § Liveness async/interrupt; `dispatch` skill bullet
- `docs/dev/WORKFLOW.md` — autonomous-loop fifth stop condition + async note
- `plugin/templates/WORKFLOW.md` — mirror

### Review verdict — 2026-07-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** Claude (direct) — architect-authored contract-doc prose, self-reviewed
- **Scope deviations:** none — historical Status entries + the § Status #30 marker left for milestone close, as scoped
- **Calibration:** none
