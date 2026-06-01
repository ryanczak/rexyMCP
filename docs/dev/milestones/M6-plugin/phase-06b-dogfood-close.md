# Phase 06b: dogfood execution + M6 retrospective + close

**Milestone:** M6 — Plugin + architect/review skills
**Status:** done
**Depends on:** phase-06a (done) — the dogfood procedure + log template + clean pre-flight. Phases 07a / 07b (done) — the two dogfood-surfaced executor-resilience fixes whose deferred design questions this phase decides.
**Estimated diff:** retrospective + two doc decisions + one architecture amendment; no Rust code
**Tags:** language=markdown, kind=retrospective, size=m

## Goal

Close M6. Synthesize the dogfood-log observations into the M6 retrospective,
decide the two design questions 07a/07b deferred, decide the
compaction-monitoring question, fold the one empirically-decisive architecture
amendment (push→pull liveness), and flip M6 to `done`.

**This phase is architect-authored, not executor-dispatched.** Like 06a flagged,
06b's job is the architect synthesizing the user-captured dogfood log — there is
no spec for a local model to implement. The deliverable is this retrospective +
the milestone README Notes + the architecture amendment + the status flips.

## What the dogfood actually exercised

The user ran the dogfood against `~/src/ms_pacman` (a bare Node project, scope:
faithful Ms. Pac-Man clone) with Claude Code 2.1.159, rexymcp 0.1.0 installed
via `--plugin-dir`, against `Qwen/Qwen3.6-27B-FP8` at `http://brain:8000/v1`
(262144 context). The run covered **bootstrap + design end-to-end**; it did not
proceed to a live `execute_phase` dispatch.

**Decision (user-confirmed): the bootstrap+design run is sufficient to close M6.**
The happy path of the load-bearing new surface — the plugin loading, the MCP
server connecting, `/rexymcp:architect` bootstrapping a non-Rust repo and
producing a design — was validated, and the two things that *broke* (tools not
advertised; live progress can't fire) were caught and fixed. The dispatch /
review / telemetry / compaction sections of the log stay blank by design; the
data they would have produced is carried forward, not blocked on.

### What worked (from the log)

- **Bootstrap, 4/4 artifacts, zero overrides.** `rexymcp.toml`,
  `docs/dev/STANDARDS.md`, `docs/dev/WORKFLOW.md`, `CLAUDE.md`, and the
  `.mcp.json` `rexymcp` entry all written; placeholders resolved with **zero
  residuals**; `npx`-prefixed Vite/Vitest/ESLint/Prettier command set detected
  and confirmed interactively. **No `AGENTS.md` written** (the load-bearing
  contract-is-embedded discipline held against the real plugin).
- **The interactive-modify discipline held.** Bootstrap prompted for the four
  decisions it could not responsibly default (toolchain, endpoint, model,
  product scope) and explicitly flagged the one field it *did* default
  (`[telemetry] dir`) rather than silently choosing — exactly the §"Bootstrap
  pitfalls" rule. The user confirmed no corrections were needed.
- **Design quality 5/5 across the board** (architecture doc, milestone
  decomposition, phase-01 draft, pre-injection). The architect produced a
  three-layer design (pure Simulation / fixed-timestep Engine / browser Shell),
  a 6-milestone roadmap, and a detailed phase-01 with pre-injected
  Vite/Vitest/ESLint config snippets + a pre-flight to verify those schemas
  against live docs — the offline-pre-injection discipline applied correctly to
  a fresh project.
- **The gate held.** The architect stopped before drafting phase-01 on the
  bootstrap pass, surfacing `/rexymcp:architect next` as the explicit next act.

### What broke (both fixed before this close)

1. **Tools not advertised in the MCP handshake (critical, fixed).** rexyMCP did
   not declare the `tools` capability in `get_info`, so Claude Code never called
   `tools/list` and the tools looked missing. Fixed in commit `b78a081`. This is
   the single highest-severity dogfood finding — the plugin was unusable until
   it landed, and no unit test could have caught it because the gap was in the
   live `initialize` handshake against the real client, not in any tool's logic.
2. **Live MCP progress notifications never fire (client limitation, fixed
   around).** See the architecture amendment below.

## Decisions

### Decision 1 — terminal backend `Err` should degrade to a `hard_fail` `PhaseResult` (conditional)

**The deferred question:** should a terminal backend `Err` degrade to a
structured `hard_fail` `PhaseResult` (preserving partial work) rather than abort
`execute_phase`?

**Current behavior** (grounded in code, not speculation):
`executor/src/agent/mod.rs:238` does `result.map_err(|e| Error::Backend(...))?`
and `:271-273` does `AiEvent::Error(e) => return Err(Error::Backend(e))`. Both
**abort the entire `execute_phase` MCP call** — Claude receives an MCP tool
error, and the partial diff, the working set, and the escalation briefing are
all discarded. A test at `:1545` currently *pins* this ("AiEvent::Error must
surface as Err, not a PhaseResult").

**Decision: YES, with a boundary.** A terminal backend error that occurs **after
the loop has made progress** (≥1 completed turn / any edit applied) should be
caught and converted to a `hard_fail` `PhaseResult` carrying the briefing and the
partial artifacts, exactly like the hard-fail-detector path at `:527-549`. A
backend error that occurs **before any work** (endpoint unreachable at turn 0,
auth failure, model-not-found) should stay an `Err` — there is nothing to
preserve, and Claude should learn the endpoint is down, not receive a briefing
implying the model tried and got stuck.

**Why:** the architecture's whole escalation philosophy is "return a structured
result, let the host re-invoke" (§"Escalation = Claude Code itself" /
§"The `PhaseResult` / briefing contract"). An aborted MCP call on a mid-phase
network blip violates that contract and throws away recoverable work — precisely
the case re-dispatch-with-refined-spec or session-takeover is designed to handle.
The boundary keeps a genuine "can't reach the model at all" honest as an error
rather than dressing it up as a stuck executor.

**Where it lands:** an M7-adjacent implementation phase (it's a real loop change:
restructure `:238`/`:271-273` to branch on "have we made progress," assemble a
`Blocker::Backend`-style briefing, and update the `:1545` test to assert the
post-progress case degrades while the pre-progress case still errs). **Not built
in 06b** — 06b decides; the code is a separately-dispatched phase. Recorded as an
M7 input in the README.

### Decision 2 — cross-dispatch resume / `continue_phase` is NOT built now (stays a candidate)

**The deferred question:** is a cross-dispatch resume / "continue if phase status
≠ done" mechanism warranted?

**Decision: NO — keep re-dispatch-with-refined-spec as the default; resume stays
an uncommitted architecture candidate.** The architecture already records
`continue_phase(session_id, guidance)` as a *candidate, not committed* lever
(§"Escalation", Resume bullet) with an explicit caveat: resume preserves the
local model's accumulated **context rot** along with its progress. The dogfood
produced **no hard_fail and no evidence of an "almost there, hit one specific
wall" pattern** — the failure class resume is designed for. Building it now would
be speculative, against the same "wait for data" discipline that governs the
compaction decision.

**Why:** re-dispatch from a clean slate with a sharpened spec is, per the
architecture, frequently the *most* robust lever for weak models precisely
because it discards context rot. Resume only pays off when the discarded work is
large and the rot is small — a situational bet we have zero telemetry to size
yet. Revisit only if `PhaseRun` data (M7 onward) shows a recurring
high-progress / single-blocker failure pattern that re-dispatch wastes.

### Decision 3 — compaction monitoring: insufficient data; do NOT build summarization

**The deferred question** (M6 README phase-06): does heuristic eviction hurt
completion quality enough to justify a summarization-based-compaction milestone?

**Decision: INSUFFICIENT DATA — do not build summarization-based compaction; carry
the instrumentation forward.** The dogfood never dispatched, so the
`CompactionReport` totals the README pinned as the decision input were never
produced. Additionally, the run that *did* happen was against a 262144-context
Qwen — the large-context regime where heuristic eviction is *least* likely to
bite. The concern the README actually raised is **small-context local models
(32k–128k)**, which this dogfood did not exercise at all.

Per the README's own guidance ("Do not build summarization speculatively — wait
for the dogfood data"), the right move is to **keep the heuristic compactor as-is**
and gather `CompactionReport` data on the first real small-context dispatch
(M7 / continued dogfooding). No follow-up milestone is justified by current data;
none is foreclosed either.

## Architecture amendment (folded in this phase)

**Layer 2 § "Liveness" is empirically wrong against the real client and is
amended.** The doc claims the server "emits MCP progress notifications … so
Claude and the user see motion." The dogfood proved (via the
`execute_phase: client progress_token absent` stderr diagnostic) that **Claude
Code's MCP client sends no `progressToken`**, and the MCP spec only permits
`notifications/progress` for a request that opted in with one. The
`McpProgressNotifier` wiring is spec-correct but **unreachable with this client**.

Two fixes already shipped on master and are now the documented liveness path:
- logged `SessionEvent::Progress` records were wrongly gated on the live callback
  being present — decoupled so progress is always logged (commit `c4567fb`);
- `rexymcp status --repo <path>` reads the per-record-flushed session JSONL for
  out-of-band, **pull-based** live status (commit `3374336`, verified live).

The amendment rewords Layer 2 § Liveness to make **`rexymcp status` (pull) the
human-liveness path**, and demotes MCP progress (push) to "spec-correct, emitted
when a client supplies a `progressToken`, but Claude Code currently does not."
This is the one decisive fold — it is an external-API fact proven empirically,
not a one-off opinion, so it folds immediately rather than waiting for recurrence.

## Calibration folds (WORKFLOW / STANDARDS)

**None warranted.** The two dogfood breakages were a missing MCP capability
declaration and a client-side protocol limitation — both product bugs / external
facts, not recurring architect spec-writing or executor-discipline patterns. The
07a/07b bounces (a `#[cfg(test)]` duplicate of shipping logic; a spec gap that
mandated `pause()`/`advance()` without authorizing tokio `test-util`) were each
single occurrences; note them as calibration data, hold for recurrence, do not
fold. No `STANDARDS.md` or `WORKFLOW.md` changes.

## Acceptance criteria

- [x] Dogfood log synthesized into this retrospective (what worked / what broke).
- [x] Decision 1 (terminal `Err` → `hard_fail`) recorded with its boundary and a
      pointer to the M7-adjacent implementation phase.
- [x] Decision 2 (resume) recorded: not built, stays an architecture candidate.
- [x] Decision 3 (compaction) recorded: insufficient data, no summarization
      milestone, instrumentation carried forward.
- [x] Architecture Layer 2 § Liveness amended (push→pull).
- [x] Calibration verdict recorded (no folds, with reasons).
- [x] M6 README phase table + milestone status flipped to `done`; retrospective
      written into the README Notes.
- [x] `NEXT.md` updated to the M6→M7 milestone boundary (active phase: none).

## End-to-end verification

> Not applicable — 06b ships no runtime-loadable artifact. It is the architect's
> retrospective + decisions + one documentation amendment. The live `rexymcp`
> binary behavior it references (`rexymcp status --help`, the `tools` capability
> in `get_info`) was already verified by the dogfood and the commits it cites.

## Authorizations

- [x] **May amend** `docs/architecture.md` — Layer 2 § "Liveness" only
      (push→pull rewording per the empirically-decisive dogfood finding).
- [x] **May flip** the M6 README phase table + milestone status to `done` and
      write the retrospective into its Notes.
- [x] **May update** `docs/dev/NEXT.md` to the milestone boundary.
- No Rust code changes. No new dependencies. No `STANDARDS.md` / `WORKFLOW.md`
  changes (none warranted).

## Out of scope

- **Implementing Decision 1** (the terminal-`Err`→`hard_fail` loop change) — an
  M7-adjacent phase, dispatched separately.
- **Building resume / `continue_phase`** (Decision 2) — not warranted.
- **Building summarization-based compaction** (Decision 3) — not warranted by
  current data.
- **Starting M7.** Milestone boundaries are a human gate; M7 begins only on the
  user's explicit sign-off.

## Update Log

### Update — 2026-06-01 (complete — architect)

**Summary:** Closed M6. Synthesized the ms_pacman bootstrap+design dogfood into
the retrospective above; recorded the three carried-forward decisions
(terminal-`Err`→`hard_fail`: yes-conditional, M7 phase; resume: no, stays
candidate; compaction: insufficient data, no summarization milestone); amended
`docs/architecture.md` Layer 2 § Liveness from push to pull; recorded no
calibration folds with reasons. Flipped M6 to `done` and pointed `NEXT.md` at the
M6→M7 human gate.

**Files changed:**
- `docs/dev/milestones/M6-plugin/phase-06b-dogfood-close.md` — this doc (new).
- `docs/dev/milestones/M6-plugin/README.md` — phase table row 06b + milestone
  status → `done`; M6 retrospective written into Notes.
- `docs/architecture.md` — Layer 2 § Liveness amended (push→pull).
- `docs/dev/NEXT.md` — M6→M7 boundary; active phase none.

**Review verdict — 2026-06-01**

- **Verdict:** approved_first_try (architect-authored retrospective; no executor dispatch)
- **Bounces:** none
- **Executor:** Claude Code (direct) — architect synthesis, not a dispatched phase
- **Scope deviations:** none
- **Calibration:** none (two dogfood breakages were product bug + client limitation; 07a/07b bounces single-occurrence — held, not folded)
