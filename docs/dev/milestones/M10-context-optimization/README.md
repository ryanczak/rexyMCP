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
| 01 | **Recoverable output filter for bash output** ([phase-01-recoverable-output-filter.md](phase-01-recoverable-output-filter.md)). New `context/output_filter` module: ANSI strip + consecutive-dup collapse + truncate-with-**recovery file** under `.rexymcp/output/` (rotated), wired into the `bash` tool's existing truncation, gated by a `[context] output_filter` kill-switch (default on). Turns bash's current lossy "full output not retained" truncation into a recoverable one. Establishes the diagnostic-preservation contract + recovery-file primitive that phase-02 reuses. | A | todo |
| 02 | **Structured cargo filter (test/build/clippy).** Keyed on detecting `cargo` in the command: failures + diagnostics + summary only, block aggregation, preserve every `error[Exxx]` span, cap the list with recovery-file overflow (reuses phase-01's module). Introduces the per-command filter-selection abstraction (deferred from phase-01 — built once a second filter justifies it). The high-value filter for the project's own Rust toolchain. | A | todo |
| 03 | **Superseded-read eviction.** On edit, mark prior `read_file` results for that path stale (via the working set); compaction evicts them first with a re-read breadcrumb. | B | todo |
| 04 | **Redundant-read dedupe.** Re-reading an unchanged file (working-set mtime match) returns a compact "unchanged since turn N" reference instead of re-injecting content; forced re-read still available. | B | todo |
| 05 | **Content-aware compaction priority.** Replace oldest-first eviction with a value rank (superseded reads → noisy/dup output → old reasoning; protect diagnostics, phase doc, last K turns). Enrich `CompactionReport`. | B | todo |
| 06 | **Context-efficiency metrics on `PhaseRun`.** Peak context %, compaction count, tokens reclaimed (filtering + superseded eviction); surface in `rexymcp runs` / scorecard so M10's effect is measurable across runs. | — | todo |

Phases may split or merge at draft time (e.g. 02 could split per command, or 03+05
could merge if the eviction policy is small). The table is the roadmap, not a
contract; each phase doc is the contract.

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

*(milestone retrospective written at milestone close)*

The architecture.md § Status list does not yet carry an M10 entry — adding it is a
documentation edit to a human-gated source-of-truth file (a hard rule: no
unauthorized edits to `docs/architecture.md`). That entry should be added with the
user's sign-off when M10 is formally kicked off.
