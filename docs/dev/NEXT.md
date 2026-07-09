# NEXT — Active phase pointer

Single source of truth for which phase is active. The principal engineer
(architect) maintains this file; every session reads it (per `REXYMCP.md`
§ "Read these first") to know which phase to work next.

**M27 phase-06a — done** (2026-07-09, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `e805862` feat). Per-role model delegation config
substrate: two `[architect]` keys on `ArchitectConfig` (`executor/src/config.rs`),
`dispatch_model` / `review_model`, both `Option<String>` defaulting to `None`
(inherit the session model; does **not** fall back to `[architect] model`), plus
their commented lines in the `rexymcp init` `[architect]` template. Additive and
inert — nothing consumes them until 06b's `/rexymcp:auto` skill. Clean first-try;
all four gates green on independent re-run (483 mcp + 928 executor, 2 ignored).
**Two calibration notes (no fold):** (1) the server-authored `completion_summary`
paraphrased E2E outcomes instead of quoting raw command output (STANDARDS §1);
architect re-ran both `doctor` invocations during review to confirm the real
artifact — 2nd occurrence, not yet a pattern. (2) Filed
[bug-03a-1](milestones/M27-autonomous-escalation-loop/bugs/bug-03a-1.md) (minor):
`flip_readme_row` duplicates the status cell instead of replacing it — now seen
twice in production (05a, 06a); manually corrected the malformed row here per the
05a precedent. See the
[phase doc](milestones/M27-autonomous-escalation-loop/phase-06a-delegation-config-substrate.md)
for the full review verdict.

**Active phase: none — M27 at a milestone-boundary decision (human gate).**
All **committed** M27 phases (01–06b) are `done`. The only remaining row is
**phase-07 — advisory model routing in dispatch**, an explicit *stretch* the
README marks "drafted only if appetite remains after 06." So this is a human
gate: either **close M27** (run `/rexymcp:architect` to write the retrospective,
fold any calibration lessons, set this pointer to "none") **or** draft the
stretch phase-07 (`/rexymcp:architect next`). Not auto-advancing — milestone
close and stretch scope are both the human's call.

**M27 phase-06b — done** (2026-07-09, **approved_first_try**, executor **Claude
Code (direct)**; commits `eae27ed` draft + the 06b implementation commit). The
`/rexymcp:auto` loop skill (`plugin/skills/auto/SKILL.md`) + WORKFLOW
plugin-template mirror. Direct-execution (a prose skill orchestrating Claude Code
subagents, not Rust for the local-LLM executor), so authored + reviewed by the
architect. Composes the four existing skills unchanged; delegation role map
(draft/escalate/takeover in the main loop; dispatch/review/refined-re-dispatch in
`Agent` subagents on 06a's `dispatch_model`/`review_model`, inherit-by-default);
loop algorithm + four stop conditions (boundary/budget/blocker/runaway); exact
`rexymcp journal`/`harvest` command forms + the six canonical activity kinds; loop
report = printed session output + a `boundary` journal record (no committed report
file). **One external-API adaptation (data, not a fold):** Pre-flight step 5
corrected the draft's `Task` subagent-tool assumption to **`Agent`** with a
verified per-call `model` override, resolved cleanly from the live Claude Code
subagent docs — the verify-external-APIs discipline working as intended. E2E:
the six-kind `rexymcp journal` round-trip appends 6 `architect_activity` records
with no `unknown activity` warning; frontmatter valid + shape-matches the other
four skills; the template mirror carries all four stop conditions in ASCII-arrow
house style. All four gates green on independent re-run (483 mcp + 928 executor,
2 ignored); no Rust changed.

**M27 phase-05b — done** (2026-07-09, **approved_first_try**; commits `8ff703a`
draft / `eb0ccd7` feat / `b20dc1d` bookkeeping / `ac04678` approve). Architect usage
harvester — the `rexymcp harvest` CLI (`mcp/src/harvest.rs` + `main.rs` clap variant/
dispatch arm mirroring `journal`) reads Claude Code session transcripts (located via
an explicit `--transcript-dir` arg), sums per-message `usage` by class into 05a's
`ArchitectTokens`, dedups streaming lines by `message.id` (first wins), attributes
each message to the `ArchitectActivity` whose journal time-window contains it
(next-boundary: smallest activity `ts ≥ message ts`), and appends enriched activity
copies that 05a's `fold_activities` overlays at read (the fold *is* the idempotency).
Hand-rolled `parse_iso_to_epoch_ms` + `days_from_civil` (no date crate). Filled 05a's
dormant architect token/cost path end-to-end. (The approve commit `ac04678` left
NEXT.md's pointer stale — the recurring approve-time pattern — re-advanced to 06a here
at the next `/rexymcp:architect next`.)

**M27 phase-05a — done** (2026-07-09, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commits `be8ad9b` draft / `2334084` refactor / `11fca95`
bookkeeping / `553c107` approve). Architect token substrate: one coherent
`ArchitectTokens { input, cache_creation, cache_read, output }` (+ `ArchitectRates`
+ `cost(&rates)`) in `telemetry.rs`, migrated `ArchitectActivity` to a nested
`tokens` field, retired the dead `TierTelemetry.architect_*_tokens`, added cache
rates + `effective_architect_rates()` to `[architect]` config (cache-read 0.1× /
cache-creation 1.25× the input rate when the model is known), a `fold_activities`
last-write-wins overlay (by `(phase_id, activity, ts)`), and the cache-aware
dashboard cost path (`ScopeCosts.architect: ArchitectTokens`, `load_data` sums from
**folded activities**). Additive + dormant — every architect token count stays 0
until this phase-05b harvester runs. Diff landed verbatim across all 9 tasks; 926
executor + 472 mcp tests pass. No scope deviation, no fold. (The approve commit
left NEXT.md's pointer stale and the README 05a row malformed `| done | review |`
— both the recurring approve-time pattern, fixed here at the next
`/rexymcp:architect next`.)

**Three design forks resolved with the user at draft time (2026-07-09):**
(1) **per-phase attribution via journal time-windows** (roll up phase → milestone
→ project); (2) **no date crate** — the fixed-format ISO-Zulu → epoch-ms
conversion is an exact hand-rolled `days_from_civil` (bit-identical to a crate for
this UTC format; a crate buys a dependency, not accuracy — the real accuracy risks
are `message.id` dedup and the cache-token policy, both non-time); (3) **separate
cache rates** (bill uncached-input / cache-creation / cache-read / output at real
per-class rates, since cache tokens dominate real usage). Consequent to (3), a
**targeted architect-token-model rewrite** (05a scope above) over the
scattered-additive-fields or full-telemetry-rewrite alternatives. Write path =
**append + fold at read** (last-write-wins). Two transcript gotchas found while
sampling a real `~/.claude` session and pre-injected into 05b's plan: streaming
emits multiple assistant JSONL lines per response sharing one `message.id` with
**identical repeated `usage`** (dedup by `message.id` or 3–4×-overcount), and
cache tokens dominate (sampled turn: input=131, cache_read=89819,
cache_creation=10869 — the cache policy *is* the cost figure).

**M27 phase-04b — done** (2026-07-09, **approved_first_try**; commits `65387a4`
draft / `2d535be` fix / `a12119e` start / `aefb9cb` bookkeeping / `624a396`
approve). Finalize tolerates a bounced status line: extracted a prefix-tolerant
`is_in_progress_status()` shared predicate (matches `**Status:** in-progress`
exactly or with a trailing space-delimited note; the load-bearing space keeps
`in-progressish` out), and `flip_status_to_review` now emits a clean
`**Status:** review` (dropping the stale `(bounced — …)` note via a whitespace-
preserving rebuild, not `str::replace`). 917 tests pass (21 finalize-specific);
the `finalize_flips_bounced_status_and_appends_entry` integration test proves the
`TempDir` end-to-end path. Fixes the 03a no-op surfaced in phase-04 review;
unblocks phase-06's bounce-then-re-dispatch loop. See
[[finalize-noops-on-bounced-phase]]. (NEXT.md pointer was left stale by the
approve commit — the recurring pattern — and re-advanced to 05a here at the next
`/rexymcp:architect next`.)

**M27 phase-04 — done** (2026-07-08, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8; commits `3deb187` feat / `3e075ea` test / `76b38bd`
approve). `continue_phase` briefing-seeded resume: the MCP tool + `mcp/src/resume.rs`
(last-write-wins task-state restore + `git diff HEAD` + seed-safe `# Resume
context` preamble) + additive `PhaseInput.resumed_task_states` + `resume` threaded
through `RunPhaseConfig`/`AssemblyInput` + the "Resuming a phase" contract block +
the un-stubbed escalate resume lever. **Bounced once** (bug-04-1, major,
false_completion): the first dispatch shipped the feature's core behavior
**untested** — nothing set `resumed_task_states: Some(...)`, so the seed-override
path had zero integration coverage, and three test-plan tests were skipped.
Re-dispatch (test-only) added `restored_states_override_seeded_pending`
(architect **mutation-verified** — neutralizing the override loop fails it), the
`continue_phase` server tests, the contract assertion, and a strengthened
seed-safety test. All four gates green on independent re-run (917 executor + 467
mcp). **Two review findings:** (1) the first dispatch hit a **pre-03b stale
`rexymcp serve`** binary (old contract → executor authored its own completion tail
+ review flip); resolved by a mid-review server restart. (2) **Server-authored
finalize no-ops on a bounced phase** — the defect phase-04b (above) fixes; the
phase-04 status flip + completion entry were architect-recorded as a result.

**Design forks resolved with the user at draft time (2026-07-08):** (1) **single
phase** (not split 04a/04b); (2) **programmatic task-state restore** from the
session log (additive `PhaseInput` field), not textual-only — else the M21/M22
task-coverage gate would re-demand done tasks; (3) **amend the executor contract**
with a resume paragraph. Locator decided architect-side: `prior_log_path` is an
explicit `continue_phase` param (no `.rexymcp/sessions/` auto-scan), matching the
no-silent-fallback ethos.

**M27 phase-03b — done** (2026-07-08, **approved_first_try**; commits `5d35df2`
refactor / `f6a3d35` review-flip / `5ea4abd` approve). Retired the executor's
pre-completion bookkeeping gate and amended the executor contract so the executor
keeps the start flip (`todo → in-progress`) but stops authoring the completion
tail — its final message now carries the Summary/Notes the server splices in.
With the gate gone a completed run reaches finalize at `in-progress`, activating
03a's dormant server-authored finalize. (NEXT.md pointer was left stale by the
approve commit — the recurring pattern — and re-advanced to 04 here at the next
`/rexymcp:architect next`.)

**M27 phase-03a — done** (2026-07-08, **approved_first_try**, executor Claude
(Anthropic); commits `9fbc33d` feat / `34f8f93` approve). Added the
`completion_summary` channel to `PhaseResult`/`Artifacts` (populated post-think
on the complete path only), the new `mcp/src/finalize.rs` (`finalize_complete`:
Status flip + baseline Update Log entry + README-row flip + separate `docs:`
commit, staging only doc paths via `git add -- <paths>`, git failures swallowed),
and wired it into `run_phase_with` (finalize error → `warnings`, never `Err`).
Dormant-safe: no-ops on an already-`review` doc, so nothing observable changes
until phase-03b (below) retires the executor gate. Clean first-try; 920 passed /
2 ignored. (NEXT.md pointer was left stale by the approve commit — the recurring
pattern — and re-advanced to 03b here at the next `/rexymcp:architect next`.)

**M27 phase-03a — drafted** (2026-07-08). Design forks resolved with the user:
commit ownership = executor commits code / server commits bookkeeping
separately; qualitative parts = server writes the mechanical entry and splices
an executor Summary+Notes carried through the `completion_summary` field.
Pre-injected verbatim: the `PhaseResult`/`Artifacts` field additions + the
complete-path capture site (`strip_think_blocks(&completion)`), the full
`finalize_complete` skeleton + helper contracts (with pinned negatives for the
status-line match, README-row flip, and `git add -- <paths>` never `-A`), the
baseline-entry format (raw epoch-ms — no date crate, no new dep), and the
`run_phase_with` wiring (finalize error → `warnings`, never `Err`). Split from
the original single phase-03; 03b (gate retirement + contract amendment) stays
planned.

**M27 phase-02b — done** (2026-07-08, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8 on the completing run; Qwen/Qwen3.6-27B-PrismaAURA landed
the diff on the prior hard_fail runs; commits `5209738` approve). One bounce: a
governor `IdenticalToolCallRepetition` hard_fail during exploratory
verification, not an implementation defect — all 6 spec tasks and four gates
were clean once a diff was produced. Refined re-dispatch (spec note confirming
the pre-verified `rexymcp journal` CLI syntax) then completed. Independent gate
re-run (918 passed / 2 ignored) + end-to-end `rexymcp journal` round-trip
reproduced. Retired the orphaned `tier_telemetry.escalation_count`; rewired the
dashboard Assists counter to count `assist` `ArchitectActivity` records.

**M27 phase-02b — drafted** (2026-07-08). Two files
(`executor/src/store/telemetry.rs` field retirement + doc-comment/test fix +
back-compat test; `mcp/src/dashboard/mod.rs` `load_data` rewire + test rewrite +
fixture cleanup). Pre-injected verbatim: the `TierTelemetry` before/after, the
full `load_data` tuple-fold→split replacement (costs fold + separate
`read_architect_activities` assist count), the rewritten count test with two
pinned negatives (non-assist kind, other project), and the back-compat fixture.
Completes the "escalation_count wiring" half of the phase-02 split.

**M27 phase-02 — done** (2026-07-08, **approved_first_try**, executor
Qwen/Qwen3.6-27B-PrismaAURA; commits `d47947e` feat / `c77f5dd` approve).
Loop-journal substrate: added the `ArchitectActivity` append-only record (six
kinds) + store API (`append_architect_activity`/`read_architect_activities`) +
`ARCHITECT_ACTIVITIES` advisory vocabulary + the `rexymcp journal` CLI producer
(mirrors `rexymcp review`); retired the dead M20 `EscalationEvent` (generalize,
not sibling — zero producers/readers). Clean first-try; all four gates green on
independent re-run (917 passed / 2 ignored). End-to-end verified via the real
`rexymcp journal` CLI. No scope deviation, no calibration fold. One cosmetic nit
(doc comments written as single long lines vs multi-`///`-line — no fmt-gate
effect).

**M27 phase-02 — drafted** (2026-07-08). ~430 lines, size=l, three files
(`executor/src/store/telemetry.rs`, new `mcp/src/journal.rs`, `mcp/src/main.rs`).
Pre-injected verbatim: the `EscalationEvent`/`append_escalation`/`read_escalations`
shape to mirror for the new store API, the `FAILURE_CLASSES`/`is_known_failure_class`
advisory-vocabulary pattern, the full `ArchitectActivity` struct + `journal.rs`
`record_activity` body + `main.rs` clap-variant/dispatch-arm (each a 1:1 copy of
the `review.rs`/`Review` analogue). Two cross-discriminator exclusion tests
**converted** (not deleted) from escalation → architect_activity so the
load-bearing `record`-filter pin (M18 bug-01-1) carries forward. Write-side only;
no dashboard/`TierTelemetry` touch, `architect_*_tokens` stay 0 (phase-05
harvester fills them). **Two draft-time forks resolved with the user:** generalize
`EscalationEvent`→`ArchitectActivity`; rewire the Assists counter in phase-02(b).
Also recorded the M27 **per-role model delegation** decision in the README (commit
`a56cf4d`): `/model` picks the architect model, `/rexymcp:auto` delegates
dispatch/review to subagents on `[architect] dispatch_model`/`review_model`
(inherit-by-default, no `draft_model` — drafting stays in the main loop);
accounting implication routed to phase-05.

**M27 phase-01 — done** (2026-07-08, **approved_first_try**, executor
Qwen/Qwen3.6-27B-PrismaAURA; commits `108a2f1` refactor / `fcec7c1` approve).
Retired `[budget] escalation_slots` from `BudgetConfig` (field + `Default` +
~40 fixture lines via `sed`) and redefined `[escalation] max_assists` as the
flat, tier-independent per-phase assist budget (default 3); `calibrate` stops
managing `[escalation]` (user settings survive re-calibrate) and strips the
retired key from old configs; `init` template updated; back-compat pin added
(`Config::load` ignores a stale `escalation_slots` key). Clean first-try; all
four gates green on independent re-run (916 passed / 2 ignored). One accepted
scope deviation (4 `escalation_slots` fixture lines in `mcp/src/server_tests.rs`,
mechanically required for a clean build). No calibration fold.

**M27 phase-01 — drafted** (2026-07-08). Mechanical multi-site churn is the
dominant risk (the 4-occurrence stall class): the spec pre-injects the
compiler-guided ordering (remove field → build → fix the 2 flagged assertions)
and sanctions `sed` for the ~40 fixture-line deletions. Semantics change folded
in with rationale: `max_assists` stops being SMALL-tier-derived, so
`calibrate` neither writes nor removes `[escalation]` on any tier (pinned
negative: a user's explicit section must survive re-calibrate). Back-compat
pinned: `Config::load` must ignore a stale `escalation_slots` key.
`architecture.md` § Configuration amended at draft time (architect-side).
~150 lines, size=m.

**📌 M27 — Autonomous Escalation Loop kicked off (2026-07-08, with the user).**
The architect-side autonomous cycle queued at the M26 phase-06 talk-through.
Design fixed at kickoff via a four-fork talk-through (full-milestone loop; full
review rigor with no per-phase pause + per-activity token/cost accounting; all
three threads in scope; budget consolidated on `[escalation] max_assists`,
`escalation_slots` retired). Seven planned phases in three threads —
**substrate** (01 knob consolidation, 02 loop-journal telemetry), **executor/
server autonomy** (03 D8/D9 server-authored bookkeeping + executor-contract
amendment, 04 `continue_phase` briefing-seeded resume), **architect loop &
accounting** (05 Claude Code transcript usage harvester + dashboard wiring, 06
`/rexymcp:auto` loop skill + loop report, 07 stretch: advisory model routing).
Kickoff amendments landed: `architecture.md` § "Escalation = Claude Code
itself" (resume candidate → committed, briefing-seeded; autonomous-loop
paragraph) + § Status #27; `WORKFLOW.md` § "Phase progression & triggers"
autonomous-loop paragraph expanded (plugin-template mirror deferred to
phase-06). Milestone
[README](milestones/M27-autonomous-escalation-loop/README.md) holds the full
design record. Phases drafted on demand via `/rexymcp:architect next`.

**M26 — Polish & Hardening closed 2026-07-08** (9/9 phases done; see the
[milestone README retrospective](milestones/M26-polish-and-hardening/README.md#retrospective--2026-07-08)).

**M26 phase-08 — done** (2026-07-08, **approved_first_try**; commits `7b52496`
feat / `78895cd` approve). Verifier `tsc` resolution
(`node_modules/.bin` → `npx --no-install tsc` → PATH). Three pure resolver
helpers (`find_local_tsc` — ancestor-walk for `node_modules/.bin/tsc`, catching
monorepo hoisting; `binary_in_dirs` — a PATH-scan mirroring
`doctor::resolve_binary`; `resolve_tsc_command(project_root, npx_on_path) ->
TscCommand` — local → `npx --no-install tsc` → bare `tsc`) plus the one spawn
rewired to use the resolved program + prefix args. Diff landed byte-identical
to the phase doc's pre-injected code. 10 unit tests + 1 `#[cfg(unix)]` E2E test
(plants an executable fake `node_modules/.bin/tsc`, confirms `Checked` with its
emitted diagnostic — proves the local binary is actually spawned). Clean
60-turn first-try; all four gates green on independent re-run (915 passed / 2
ignored). No scope deviation, no calibration fold.

**M26 phase-08 — drafted** (2026-07-08). `verify_typescript`
(`executor/src/governor/verifier.rs:431`) spawns a **bare** `tsc`, so it
`Skipped`s (NotFound) in real Node repos where `tsc` lives in `node_modules/.bin`.
The phase adds three pure resolver helpers (`find_local_tsc` — ancestor-walk for
`node_modules/.bin/tsc`, catching monorepo hoisting; `binary_in_dirs` — a
PATH-scan mirroring `doctor::resolve_binary` since the `mcp` crate that owns it
depends on `executor`, not the reverse; `resolve_tsc_command(project_root,
npx_on_path) -> TscCommand` — local → `npx --no-install tsc` → bare `tsc`) and
rewires the one spawn to use the resolved program + prefix args. Resolution runs
**after** the `find_typescript_project_root` None check, so the "no tsconfig.json"
`Failed` path is byte-identical; `spawn_failure`→`Skipped` unchanged (just a
local-install hint). Pre-injected: the full verify_typescript block to replace,
the `doctor.rs` resolve_binary shape to mirror, verbatim helper bodies, the
`.is_file()` directory-negative pin, and a `#[cfg(unix)]` **fake-local-binary
E2E** (plant an executable `node_modules/.bin/tsc` shell script → assert `Checked`
with its emitted diagnostic — proves the local binary is actually spawned, no
host `tsc` needed). No new dep, no `Cargo.toml`/`architecture.md` edit. ~160
lines, size=m.

**M26 phase-07b — done** (2026-07-08, approved_first_try; commits `82b7830`
draft / `ccaf130` feat / `55e69b7` approve). A clock-based **budget terminal** (not a
governor detector): a new `#[serde(default)] wall_clock_secs: u64` on `BudgetConfig`
(default 0 = disabled) and a sibling `wall_clock_secs: u64` on `LoopDeps`, threaded
to all 15 construction sites via the phase-06 `gate_retries` precedent (mechanical
"add a line after each `gate_retries:`" rule, grep-verified). The loop captures a
`loop_started_ms` baseline off the injected `deps.clock` (no real `SystemTime` in
`executor/`) and adds a "Step 2a" terminal at the top of the loop: when
`wall_clock_secs > 0` and elapsed ≥ ceiling, return `budget_exceeded` (mirroring the
Step-2 context-overflow block). Unlike `gate_retries` it is **flat opt-in** — no tier
derivation, no `ModelOverride`, no `effective_*` helper — and **is** written to the
`rexymcp init` `[budget]` template. Pre-injected: the 15-site `LoopDeps` table + grep
rule, the Step-2 worked example to copy, the `AtomicU64` advancing-clock test helper
(clock is `Fn` not `FnMut`), and the enabled/disabled integration-test pair (ceiling
fires before Step 3, so no mock-exhaustion risk). ~180 lines, size=m.

**M26 phase-07a — done** (2026-07-08, approved_first_try). Two additive, standalone pure detectors
in `executor/src/governor/hard_fail.rs`, chained after `evaluate` in the Step-7
hard-fail seam (mirroring M22's `check_empty_completion_stall` pattern — **not**
folded into `evaluate`, so its ~10 test call sites are untouched):
- **`check_oscillation`** — sliding-window distinct-`(tool, arguments)`-set
  detector; fires `Oscillation` when the last `oscillation_window` (default 8) calls
  collapse to `2..=oscillation_distinct_max` (default 2) distinct calls, catching the
  A,B,A,B read↔patch cycle that `IdenticalToolCallRepetition` (consecutive-identical
  only) misses. Distinct-count 1 is left to identical-repetition; `serde_json::Value`
  isn't `Hash`/`Ord` so distinctness is a linear `Vec` scan.
- **`check_windowed_output`** — sums the last `output_window` (default 6) tool
  outputs (new lockstep `recent_output_bytes: VecDeque<usize>`); fires
  `CumulativeOutputFlood` above `output_window_bytes` (default 256 KB), catching a
  multi-call flood of sub-`runaway_output_bytes` outputs the single-call
  `check_runaway_output` misses.
Four `GovernorConfig` fields + `ModelOverride` counterparts + `resolve_for_model`
application + `rexymcp init` template docs; `window = 0` disables each. Pre-injected:
the standalone-vs-`evaluate` pattern choice, the `.or_else` chain at the Step-7 seam,
the linear distinct-set scan, tuned-`GovernorConfig` integration tests (small windows,
compaction-immune), the `n >= 2` load-bearing negative pin, and the **phase-06
mock-exhaustion gotcha** (script enough `MockAiClientScript` turns or the loop drifts
to the turn cap via the empty-completion branch). **The wall-clock ceiling split out
to phase-07b** (drafted on demand after 07a). ~300 lines, size=m.

**M26 phase-06 — done** (2026-07-08, **escalated** — session takeover after
2nd dispatch `budget_exceeded`; commit pending). Wires `effective_gate_retries(tier)`
into the M19 gate-retry loop: a resolved `gate_retries: u32` field on `LoopDeps`,
a `gate_retry_count` counter, and a `gate_budget_exhausted` check that terminates
as `budget_exceeded` (reason: "gate-retry budget exhausted after N retries") before
the turn cap. `LARGE`/no-tier still resolve to `u32::MAX` (byte-identical to prior
behavior); `mcp/src/runner.rs` resolves the field via
`inp.cfg.budget.effective_gate_retries(inp.cfg.executor.tier)`. Two `config.rs`
doc comments corrected (`gate_retries` wired M26, escalation deferred to M27).
**1st dispatch** hard-failed at turn 3 on a backend infra blip (400 from
`brain:8000` rendering a null-`arguments` tool call back into the next request) —
plain re-dispatch, no spec change. **2nd dispatch** landed all production code
byte-identical to the phase doc's pre-injected fragments (grep-confirmed) but ran
to `budget_exceeded` at 400/400 turns: the two new tests' `MockAiClientScript`
scripted only one model turn, so once exhausted `chat()` sent zero events and the
loop fell into the unrelated empty-completion recovery path instead of re-running
the gate check, drifting to the turn cap. The executor never diagnosed the mock's
turn-exhaustion behavior across ~330 stalled turns. **Session takeover:** scripted
4/3 `"All done."` turns in the two tests (no production-code change); all four
gates green (439 mcp + 888 executor tests, 2 ignored). **Calibration (2nd
occurrence, no fold):** executor stalls on a test-harness/mock subtlety it can't
diagnose, distinct from a production-code gap — flagged for the user.

**M26 phase-05 — done** (2026-07-07, **approved_first_try**; commits `d5acc0c`
draft / `cb9c1f1` fix / `ecdd970` approve). The post-write format hook no longer
no-ops: a new writing `format_fix` field (mirroring `lint`/`lint_fix`) lets the
hook rewrite a just-written misformatted file while the DoD fmt gate keeps running
the check-form `format`. Closes the M21 executor-vs-reviewer fmt divergence.
**Wire-or-retire fork for phase-06 resolved with the user (2026-07-07):** wire
`gate_retries` (phase-06, drafted above); **defer** the escalation knobs
(`escalation_slots`/`max_assists`) to a new **M27 — Autonomous Escalation Loop**
milestone (architect-side `/loop`, starts with a design talk-through + an
`architecture.md`/executor-contract/`WORKFLOW.md` amendment; absorbs the resume
lever + D8/D9 server-authored bookkeeping). M27 is a human-gated boundary, not yet
kicked off. See the [M26 README](milestones/M26-polish-and-hardening/README.md)
§ "Escalation budgeting moved to M27".

**M26 phase-04 — done** (2026-07-07, **approved_first_try**; commits `4cf24da`
draft / `cdd8a98` fix / `bc3313f` approve). `write_file` now honors the
read-before-edit gate: an **overwrite** of an existing unread (or mtime-changed)
file is refused like `patch`, while **create** and **append** stay ungated, and
the working set records mtime after a non-append write. (NEXT.md pointer was left
stale by the approve commit — a recurring pattern — and re-advanced here at the
next `/rexymcp:architect next`.)

M26 phases 01–03 all **done** (2026-07-07, all approved_first_try):
[01](milestones/M26-polish-and-hardening/phase-01-contract-docs-and-manifests.md)
contract docs & manifests,
[02](milestones/M26-polish-and-hardening/phase-02-run-phase-telemetry-parity.md)
run-phase telemetry parity,
[03](milestones/M26-polish-and-hardening/phase-03-silent-degradation-warnings.md)
silent-degradation warnings. The originally-planned roots-corroboration phase was
**deferred** (rmcp 1.8.0 deprecated `list_roots` per MCP SEP-2577 — see the
[milestone README](milestones/M26-polish-and-hardening/README.md) § "Roots
corroboration deferred"). Remaining phases 05–08 are planned in the README but not
yet drafted.

**📌 M26 — Polish & Hardening kicked off (2026-07-07, with the user).** Seeded
from the post-M25 whole-codebase review
([codebase-review-2026-07-07.md](codebase-review-2026-07-07.md)) rather than an
e2e failure — the review surfaced seams that fail silently, so no dogfooding run
trips them. Two threads across nine planned phases: **housekeeping (01–04)** —
stale `REXYMCP.md` contract lines + divergent plugin manifests (01),
untelemetered `run-phase` (02), dead `roots/list` corroboration (03),
silent degradations surfaced as `PhaseResult` warnings (04) — and **loop
hardening (05–09)** — `write_file` read-before-edit gate (05), post-write format
hook writing form (06), wire-or-retire the dead budget/tier knobs (07, decision
with the user at draft time), governor blind-spot detectors (08), verifier `tsc`
resolution (09). Milestone
[README](milestones/M26-polish-and-hardening/README.md) +
`architecture.md` § Status #26 added at kickoff; phases drafted on demand via
`/rexymcp:architect next`. **M25 closed 2026-06-30** at the human-gated boundary
below.

**M25 — Polish & Config Pass — done** (9/9 phases, 2026-06-30; executor
Qwen/Qwen3.6-27B-PrismaAURA). 8 approved_first_try, 1 approved_after_1 (phase-03
`false_completion`, bug-03-1 — missing required negative pin). Two threads: a
polish/config thread (01 `update_task` recovery hint · 02 `enable_thinking` knob ·
03 Budget/Session panel polish · 04 Activity/Tasks panel polish) and a
dependency-bump thread (05 `similar` 2→3 · 06 `tree-sitter` 0.25→0.26 +
`tree-sitter-python` 0.23→0.25 · 07 `toml_edit` 0.22→0.25 · 08 `toml` 0.8→1 · 09
`reqwest` 0.12→0.13). The dep-bump recipe (bump one constraint → `cargo update -p`
→ react only to compiler flags → four gates) ran 5/5 clean with **zero source
edits**, including the reqwest 0.13 rustls/aws-lc default-TLS swap. **No new
calibration folds** (the phase-03 `false_completion` is a known-pattern recurrence,
data not a fold). See the milestone
[README retrospective](milestones/M25-polish-and-config/README.md#retrospective--2026-06-30).

**M24 — committed scope complete** (phase-01 done 2026-06-18; phase-02 — extend
the enrichment to the ambiguous / zero-match arms — held pending a follow-up
netviz e2e that shows the model also stalls there).

**M24 phase-01 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `a6ff4fc` fix). The `patch` no-op arm now returns
recovery context instead of the dead-end `no-op patch: old_str equals new_str`
string: the `old_str == new_str` guard moved from above the file read to below it
(after `match_count`), and a new `noop_hint` free fn — mirroring `fuzzy_hint`'s
windowed shape with a `{lineno:>4} | {line}` gutter — emits the `path:start-end`
location, a line-numbered context window, an `occurrences > 1` multiplicity note,
and a `read_file`/move-on next step; the `content.find` `None` branch handles the
absent-text case without fabricating a location. 3 mutation-resistant tests;
existing `rejects_identical_old_and_new` passed unmodified. Clean 71-turn
first-try; all four gates green on independent re-run (860 passed / 2 ignored).
No scope deviation, no calibration fold. Cosmetic Update-Log identity self-stamp
("Claude (Opus)") recurred — date correct, telemetry records the real Qwen model.

**📌 M24 — Edit-Loop Recovery kicked off (2026-06-18, with the user).** Diagnosed
from a fresh netviz e2e run (`google/gemma-4-26b-a4b-qat`, MEDIUM, phase-03,
`session-phase-03-6a342a42.jsonl`) that hard-failed on a **new** mechanism (distinct
from M22's empty spiral and M23's truncation). Turn 3: a `patch` succeeded but
introduced **duplicate constants** (added `ETHERTYPE_IPV6`/`IPV6_HEADER_LENGTH`
after `ETHERTYPE_IPV4` when they already existed lower in the file). Turns 5–10:
the model tried to remove the dupes but submitted a **byte-identical
`old_str`/`new_str`** (a no-op) — the flat `no-op patch: old_str equals new_str`
error gave it nothing to act on, so it re-emitted the identical call until M22's
`IdenticalToolCallRepetition` stall fired at turn 10 (three turns after the real
failure). Milestone [README](milestones/M24-edit-loop-recovery/README.md) +
`architecture.md` § Status #24 added. **Single phase** (phase-01): move the no-op
check below the file read and replace the dead-end string with a recovery message —
current `path:start-end` location, a line-numbered context window (mirroring
`patch.rs`'s `fuzzy_hint`), an occurrence-count note when the text appears > 1 time
(the duplicate tell), and a `read_file`/move-on next step. **Scope decisions
(2026-06-18, with the user):** enrich the tool error, **no new terminator**
(recover-first per M23; M22's governor stall stays the unchanged backstop); only the
`patch` no-op arm (`patch_lines` and the other `patch` arms untouched); surface
context, don't auto-fix duplicates. A phase-02 extending the same enrichment to the
ambiguous/zero-match arms is held until the follow-up e2e shows a need.

**M23 phase-03 — done** (2026-06-18, **approved_first_try**, executor
**Claude Code (direct)**; commit `eed0213` refactor). The retrospective-cleanup
phase that closed M23. Collapsed the three sampling knobs (`temperature`, `seed`,
`max_tokens`) into a single `Copy` `SamplingParams` struct in `executor/src/ai/mod.rs`
(manual `Default` with `max_tokens: 8192`), threaded as one `sampling:` arg through
`build_chat_body` (5 args), `OpenAiClient::new` (6 args), `chat`, and both call
sites (`make_client` + `mcp/src/runner.rs`) — retiring the phase-01
`#[allow(clippy::too_many_arguments)]` (grep-confirmed gone). Also fixed
`format_no_match` in `feedback.rs` (the 2×-deferred latent panic): `&response_excerpt[..200]`
→ `chars().take(200).collect::<String>()` + `len()` → `chars().count()` guard,
matching `format_truncated`. 9 mechanical `build_chat_body` test-call-site updates
(same struct-literal churn as M23 phase-01's `ModelOverride` literals); 2 new
mutation-resistant tests (`sampling_params_default_max_tokens_is_8192`,
`format_no_match_handles_multibyte_boundary`). Clean 87-turn first-try; all four
gates green on independent re-run (857 executor + 431 mcp pass, 2 ignored). **No
scope deviation, no calibration debt** — both items it was authored to close
(the `too_many_arguments` allow + the `format_no_match` panic) are now retired.

**M23 phase-02 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `6608df3` feat). Acts on `finish_reason`: a new
per-turn `turn_finish_reason` (declared inside the loop body so it resets each
turn), captured in the `AiEvent::Completion` arm; the `NoToolCall` empty branch
guard broadened to `truncated || post_think.trim().is_empty()` with feedback
selected by cause — `format_truncated` for a `length`-cut turn (leaves the empty
counter untouched), `empty_recovery_feedback` for a blank turn (count ≥ 2
escalates to a no-`<think>` directive). Two new `feedback.rs` helpers + 3 unit
tests; 2 integration tests (`truncated_turn_is_not_treated_as_completion` pins
`calls().len() == 2`; `repeated_truncation_reaches_turn_cap_not_completion` pins
`BudgetExceeded`). Clean 89-turn first-try; all four gates green on independent
re-run (855 passed / 2 ignored). M22 empty-stall + gate tests pass unmodified.
The `format_no_match` `[..200]` byte-slice panic was correctly held out of scope
(2nd deferral; `format_truncated` uses char-safe `chars().take(200)`).

**M23 phase-01 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `5eec632` feat / `8a68b06` approve). `max_tokens`
is now a `[executor]` knob (default **8192**, up from the hardcoded 4096) +
per-model `[models."<id>"]` override, threaded through `build_chat_body` /
`OpenAiClient::new` / both call sites exactly like `temperature`/`seed`; the
hardcoded `4096` at `openai.rs:110` is gone (grep-confirmed). 6 new tests (2
`openai.rs` wire-serialization, 4 `config.rs` default/load/override/inherit),
all mutation-resistant. Clean 145-turn first-try; all four gates green on
independent re-run (850 passed / 2 ignored). **Calibration (1st occurrence,
data — no fold):** the spec's "mirror `temperature`/`seed` **exactly**"
instruction pushed `OpenAiClient::new` to 8 args, crossing clippy's
`too_many_arguments` threshold (7) and necessitating a function-scoped
`#[allow(clippy::too_many_arguments)]` — accepted as a spec-mandated
consequence (the only alternative, a builder/params-struct refactor of the
constructor, was out of the phase's authorized scope); flagged for a possible
future fold.

**📌 M23 — Truncation & Empty-Completion Recovery kicked off (2026-06-18, with the
user).** Diagnosed from a fresh netviz e2e run (`google/gemma-4-26b-a4b-qat`,
MEDIUM, `session-phase-03-6a33e58c.jsonl`) that hard-failed on the exact mechanism
M22 phase-01 added the `EmptyCompletionStall` terminator for — but the log shows
the terminator firing on a *symptom*. The model was being truncated mid-`<think>`:
`max_tokens` is **hardcoded to 4096** (`openai.rs:110`), so on turns 12/14/15 it
generated exactly 4096 output tokens of reasoning and was cut off
(`finish_reason == "length"`) before reaching a tool call — at only **45% context
use** (output cap, not context length, was the wall). `finish_reason` is captured
(`mod.rs:414`) but only counted for the scorecard; nothing acts on it, so the
truncated stub falls through and is mis-read as a completion, after which the model
collapses to 0-token EOS responses (turns 16–18) that M22's stall finally
terminates — three turns *after* the real failure. Two phases (milestone
[README](milestones/M23-truncation-recovery/README.md); `architecture.md` § Status
#23 + § Configuration bullet added):
- **phase-01 (config substrate):** make `max_tokens` a `[executor]` /
  `[models."<id>"]` knob (default **8192**, up from 4096), threaded through the
  backend exactly like `temperature`/`seed`.
- **phase-02 (loop behavior):** retain `finish_reason` per turn; in the
  `NoToolCall` arm, route a `length`-truncated turn to a truncation-specific
  recovery nudge instead of the completion path, and escalate the empty-recovery
  feedback to a no-reasoning directive after ≥ 2 consecutive empties.

**Scope decisions (2026-06-18, with the user):** default `max_tokens` = 8192
(16384 lets a runaway turn burn ~2× before cut; 4096 keeps the truncation);
two-phase config/loop split per the M18 substrate→behavior precedent; **recover
first** — no new truncation terminator this milestone (the loop stays bounded by
the turn cap + M22's empty stall; add a `TruncationStall` only if the follow-up
e2e shows truncation loops persist). Dispatch 01 then 02, review-gate each.

**Deferred (D8/D9), to discuss before authoring:** pre-filled / server-authored
bookkeeping — explicitly deferred from M22, requiring a design conversation (it
moves *who authors* the bookkeeping from executor to server and touches the
executor contract). See the
[M22 retrospective](milestones/M22-bookkeeping-resilience/README.md#retrospective--2026-06-18).

**M22 phase-05 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `eded524` feat). C7: working-set-aware
`destructive_restore_refusal` (+ `restore_path_tokens`) in `tools.rs`, chained
`.or_else()` before `read_before_edit_refusal` at the `mod.rs` refusal seam —
refuses a single-file `git checkout/restore <path>` of a file edited this session
(keyed off `pre_edit_content`), as a model-visible advisory, not a hard_fail or
governor strike. Complements the stateless `bash_classify` wholesale-form
blocklist (untouched — the working-set check needs loop state the classifier
can't see). 7 unit tests + 1 integration test (`self_revert_of_edited_file_is_refused`,
mutation-verified). Clean 84-turn first-try; all four gates green on independent
re-run (844 passed / 2 ignored). No new dependency, no scope deviation, no fold.

**M22 phase-04 — done** (2026-06-18, **approved_first_try**, executor
**claude-code (direct)**; commit `83d1805` feat / `afe3216` approve). B6: the
`update_task` tool result now appends the still-incomplete task ids (computed
inside the lock in seeded order) and flags a redundant re-mark (`was_already`
captured before the state flip), giving the model a per-call signal so it can
self-correct instead of refixating on one task. `task_update` metadata shape
untouched (same `id`/`title`/`state` keys — loop shadow intact). Single
production file (`update_task.rs`); four new mutation-resistant tests (the base
remark for `result_lists_remaining_incomplete_ids` contains no `"2"`, so the
id-echo assertions only pass via the `remaining` clause); no pre-existing test
pinned the old output by equality, so all 7 prior `update_task` tests passed
unmodified. Clean first-try; all four gates green on independent re-run (836
passed / 2 ignored). **No scope deviation, no calibration fold.** (NEXT.md
pointer was left stale by the approve commit — which touched only README + the
phase doc — and re-advanced here at the next `/rexymcp:architect next`.)

**M22 phase-03 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `5623004` feat). B4: `parse_task_line` now requires
a `**bold**` name — the `extract_title` fallback deleted; non-bold numbered items
no longer seed as tasks. B5: `seed_from_spec` de-dupes the seeded list by id and
title (first occurrence wins). Three existing contract tests updated; three new
tests added (`ignores_prose_numbered_list_without_bold`, `dedupes_colliding_ids`,
`dedupes_identical_titles`). Two integration tests in `tests.rs` also updated —
their fixtures had non-bold items (`2. Second task — do that`) that would fail
under the new contract; correct and required. Clean 64-turn first-try; 832 passed
/ 2 ignored.

**M22 phase-02 — done** (2026-06-18, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `93dbca8` feat / `d9b9a1e` approve). A3: the
additive peek-guard (`check_repeated_gate_feedback` → `HardFailSignal`) sits
**above** the three untouched M19/M21 gate blocks in the `NoToolCall` arm — the
same gate feedback re-injected ≥ K times (default 5) with no state change now
terminates as `hard_fail` (`StuckGateFeedback`), not an unbounded loop. Clean
121-turn first-try; all four gates green on independent re-run (829 passed / 2
ignored). Both pinned negatives
(`task_coverage_check_loops_until_all_tasks_done`,
`gate_failure_loops_until_gates_pass`) pass unmodified; the integration test
`stuck_task_coverage_feedback_hard_fails` is mutation-verified. **Scope:** the
2-line `mcp/src/runner.rs` touch is the same mechanically-required
`ModelOverride` struct-literal consequence as phase-01, not a widening. **No
calibration fold.**

**M22 phase-01 — done** (2026-06-17, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `e618496` feat). A1 broadened the `NoToolCall`
empty guard to `post_think.trim().is_empty()` (a blank `raw:""` now routes to the
parse-failure recovery nudge instead of the gate/completion path); A2 added a
`consecutive_empty_completions` counter + `HardFailSignal::EmptyCompletionStall`
(pure `check_empty_completion_stall`, default threshold 3 in `GovernorConfig` +
`ModelOverride`), emitted inline in the `NoToolCall` arm with two reset sites
(gate fall-through + tool-dispatch). Clean 98-turn first-try; all four gates green
on independent re-run (825 executor + 431 mcp). The 2-line `mcp/src/runner.rs`
test touch is a mechanically-required struct-literal consequence of the additive
`ModelOverride` field (the executor's "not `#[serde(default)]`" rationale was
imprecise — the struct carries struct-level `#[serde(default)]`; the edit is a
Rust struct-literal requirement, not a serde one). **Calibration (1st-occurrence,
no fold):** `single_empty_completion_then_recovers_does_not_hard_fail` scripts only
one empty (below threshold 3), so it weakly covers the reset path — a spec-shape
limitation (architect-authored), not an executor fault; a future interleaved-empty
test would pin the reset.

**📌 M22 — Bookkeeping-Loop Resilience kicked off (2026-06-17, with the user).**
Diagnosed from a live netviz e2e run (`google/gemma-4-26b-a4b-qat`, MEDIUM) where
the executor reliably wrote correct code (all four gates green every failing
session) but could not finish the bookkeeping loop. Three recurring mechanisms,
each invisible to the existing guardrails, traced from the session logs in
`/home/matt/src/netviz/.rexymcp/sessions/`:
- **Empty-output death spiral** (`session-phase-04-6a32f806`, budget_exceeded@200):
  after a `write_file` null-args error the model emitted **147 consecutive empty
  completions**; the `NoToolCall` guard at `mod.rs:516` only catches a
  `<think>`-only completion (requires `</think>`), so a blank `raw:""` fell through
  and was treated as a completion attempt — re-running the gates, tripping
  `task_coverage_retry`, and re-injecting identical feedback to the turn cap. The
  `IdenticalToolCallRepetition` stall was blind to it (empty completions produce no
  tool call, so `recent_tool_calls` never grew).
- **Bogus seeding → update_task fixation** (`session-phase-04-6a32f237`,
  hard_fail@26): `seed_from_spec` parsed a `#### update()` prose algorithm
  (`1. If packet.tcp…`, `2. …`) as tasks with **byte-identical truncated titles**;
  the model couldn't tell them apart and re-marked task 1 until
  `IdenticalToolCallRepetition` fired.
- **Self-revert** (same session): `git checkout src/flow-table.test.ts` discarded
  the model's own green edit; `bash_classify` blocks the wholesale forms (`git
  reset --hard`, `git checkout .`) but not a single-file restore of an edited file.

**Five phases drafted up front** (user asked for the A+B cluster then C7; dispatch
in order, review-gate each). Milestone
[README](milestones/M22-bookkeeping-resilience/README.md) + `architecture.md`
§Status #22 added.
- **phase-01 (A1+A2):** broaden the `NoToolCall` empty guard to route a truly-empty
  completion to the recovery nudge (not a completion attempt), + a governor
  `EmptyCompletionStall` (consecutive-empty counter → `hard_fail`, default 3).
- **phase-02 (A3):** additive peek-guard above the three gate blocks — the same gate
  feedback re-injected ≥ K times (default 5) with no state change → `hard_fail`
  (`StuckGateFeedback`). Three existing blocks untouched.
- **phase-03 (B4+B5):** seeder precision — the `N.` list form **requires a bold
  name** (matches the documented `WORKFLOW.md` § Spec convention; excludes prose
  algorithm lines and removes title truncation), + de-dup seeded tasks by id and
  title. **Design fork resolved with the user (2026-06-17):** bold-required over
  depth-gating — convention-aligned, at the cost of updating 3 existing tests that
  pinned the old bare-item leniency.
- **phase-04 (B6):** `update_task` result echoes the still-incomplete ids and flags a
  redundant re-mark; metadata shape unchanged (loop shadow intact).
- **phase-05 (C7):** working-set-aware `destructive_restore_refusal` mirroring
  `read_before_edit_refusal` — refuse `git checkout/restore <path>` when `<path>` is
  in `pre_edit_content` (edited this session).

**Deferred (D8/D9), to discuss before authoring:** pre-filled / server-authored
bookkeeping (rexyMCP writing the Status flip + a baseline Update Log entry itself
from data it already holds). This moves a responsibility from executor to server
and touches the executor contract — a design conversation, not a quiet change.

**M21 — Task Coverage Gate — done** (2026-06-16, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8). Single-phase milestone. Closed the `false_completion`
blind spot on docs/no-code phases structurally: `execute_phase` keeps a
`task_states` shadow map (initialised from `seeded`, updated in the task-metadata
block as `update_task` calls land) and, after the M19 gate-retry check in the
`NoToolCall` arm, calls `command::task_coverage_feedback` — if any seeded task is
not `Done`, it injects a named-task list and loops; at the turn cap with tasks
incomplete it returns `BudgetExceeded`, not `Complete`. Zero `LoopDeps` change;
the `seeded.is_empty()` backward-compat pin kept all 807 pre-existing tests
unmodified (now 814 with the 7 new). **Bounced once** (bug-01-1, `parse_format`,
minor): first dispatch self-reported `complete` with a red `cargo fmt --all
--check` (rustfmt wanted the `task_states` init collapsed to one line); notably
M19's gate-retry did not fire because the executor's own gate run that turn
passed `format` while the reviewer's independent `--check` flagged the diff — a
1st-occurrence executor-vs-reviewer fmt divergence (data, not a fold; see
retrospective). Re-dispatch fixed it in one 43-turn pass, logic byte-identical.
**M19 + M21 now cover both `false_completion` variants structurally:** M19 the
red-gate variant, M21 the no-gate-coverage variant.

**M21 phase-01 — drafted** (2026-06-16): two files, two tasks each — (1)
`command.rs`: new `task_coverage_feedback` helper + 5 unit tests (mirror of
`gate_failure_feedback` shape); (2) `mod.rs`: `task_states` shadow map
initialised from `seeded`, updated in the existing task-metadata block, checked
after the gate-retry block. Pre-injected: exact before-text for all three
insertion points (line refs verified at draft time), the gate-retry block quoted
verbatim as the shape to replicate, `TaskState` is `Copy` (safe to use after
`SessionEvent::TaskUpdate { ..., state }`), two integration tests modelled
exactly after `gate_failure_loops_until_gates_pass` /
`gate_failure_at_turn_cap_is_budget_exceeded`. Backward-compat pin:
`seeded.is_empty()` → `task_coverage_feedback` returns `None` → all pre-existing
tests pass unmodified.

**M20 closed** (4/4 phases done, 2026-06-16). M20 phase-04 approval triggered
M21 design immediately (no prior milestone-boundary stop needed — the
false_completion gap was identified during M20 review and the user asked to
address it now).

**M20 phase-04 — done** (2026-06-16, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8). Docs-sync closeout: `architecture.md` (7 passages) +
`README.md` (3 passages) brought current with M19/M20. **First dispatch
false_completion** — reported `complete` with 2/10 tasks done; all gates green
*by construction* (docs phase, no code), so M19's gate-retry loop could not catch
it. Re-dispatch landed all 10 verbatim against the pinned before/after text;
gates green on independent re-run (807+431 tests). Commit `5c2ee5e`. The
no-gate-coverage false_completion is a new calibration data point (1 occurrence,
flagged for the user — see retrospective).

**M20 phase-04 — drafted** (2026-06-16, **already on disk**): the docs-sync
closeout. No code; edits only `docs/architecture.md` (seven stale passages:
status header, turn-cycle step 8 → gate-retry loop, `PhaseRun` schema gains
`tier_telemetry`, Configuration `[escalation]`/`[architect]` bullets, M8 Budget
panel → tabular breakdown, M8 `[dashboard]` config, + M19/M20 Status entries) and
`README.md` (three passages: dashboard CLI entry, new `rexymcp calibrate` bullet,
config example `[architect]` block). All before-text anchors re-verified current
against the live files at activation (status header 3–9, turn step 8 at 134–136,
README dashboard at 209–214 — all match the doc's quoted blocks). The phase doc
was committed incidentally inside phase-03's `feat` commit `41fc075` (the
recurring dirty-tree sweep); `/rexymcp:architect next` found it already drafted,
so this turn only re-pointed `NEXT.md` at it — no re-draft. ~110 lines, size=s,
`kind=docs`. **This is the last M20 phase — after approval, M20 closes and the
next `next` is a human-gated milestone boundary** (retrospective + fold review).

**M20 phase-03 — done** (2026-06-16, **approved_first_try**; commits `41fc075`
feat / `7c38efe` approve). Tabular Baseline/Executor/Architect/Net cost breakdown
per Session+Milestone+Project scope + Assists counter landed in
`mcp/src/dashboard/panels.rs` (`ScopeCosts`, `BudgetRates` architect rates).

**M20 — Tier Calibration and Cost Visibility** (kicked off 2026-06-16).
Four phases (a docs-sync closeout was added as phase-04). Phase-01: config
schema (`Tier`, `EscalationConfig`,
`ArchitectConfig`), known-model rate registry moved to executor lib, and
`rexymcp calibrate LARGE|MEDIUM|SMALL` CLI command. Phase-02: telemetry
fields (`tier`, `doc_level`, `escalation_count`, `architect_*_tokens`).
Phase-03: dashboard cost breakdown — tabular Baseline/Executor/Architect/Net
per Session+Milestone+Project scope, plus Assists counter in Budget panel.

**M20 phase-03 — drafted** (2026-06-16): Budget panel Savings block redesigned
as a tabular 5-row × 2-or-3-column layout. New `ScopeCosts` struct replaces the
`(u32, u32)` tuples in `DashboardData`, accumulating executor + architect token
counts per scope. `BudgetRates` gains `architect_input_per_mtok`/
`architect_output_per_mtok`. `savings_lines` produces: header row with scope
column names + Baseline/Executor/Architect/Net data rows (Executor always $0.00
until local rates wired; Net = Baseline − Architect) + Assists counter from
`project_escalation_count`. Header height grows from 11→13 to fit. ~130 lines.
**Decision (2026-06-16 with user):** (1) Executor cost = $0.00 always now,
row always shown for future paid providers; (2) per-scope (Session/Milestone/
Project) tabular layout Option A; (3) Assists counter satisfies escalation-feed
requirement (no separate feed panel).

**M20 phase-02 — drafted** (2026-06-16): the tier/cost telemetry substrate.
A new nested `TierTelemetry` struct (`tier`/`doc_level`/`escalation_count`/
`architect_input_tokens`/`architect_output_tokens`) added to `PhaseRun` as a
single `#[serde(default)]` field — the `ContextEfficiency` nesting precedent, so
the ~11 `PhaseRun` literal sites each take **one** `Default::default()` line, not
five. Only `tier` is populated (threaded `cfg.executor.tier` → new
`PhaseInput.tier` → `emit_phase_run`, the `project_id`/`milestone_id` path); the
other four default to 0/None until M21/M22. Plus a new append-only
`EscalationEvent` record + `ESCALATION_RECORD_TAG` + `append_escalation`/
`read_escalations`, mirroring the M18 `PhaseReview` discriminated-record
substrate (no producer until M21). **Pre-injected:** the `ContextEfficiency`
nesting + `PhaseReview` discriminator worked examples quoted inline; the M18
bug-01-1 guard-test lesson (pin the `record` filter as load-bearing, not just
structural mismatch); the grep'd complete list of `PhaseRun`/`PhaseInput` literal
sites + the compiler-guided E0063 traversal recipe (M12 phase-06b precedent for
clean multi-site additive churn); an explicit no-`#[allow(dead_code)]` guard
(executor **lib** crate, unlike M18 bug-03-2's `mcp` binary crate) — file a
blocker instead. End-to-end is a `run_phase_with` mock-client test asserting the
configured tier lands in the written `phase_runs.jsonl`. No new dep, no
`Cargo.toml`/`architecture.md` edit. ~320 lines, size=m.

**M20 phase-01 — done** (2026-06-16, **approved_first_try**, executor Claude Code
direct). Config schema (`Tier`, `EscalationConfig`, `ArchitectConfig`,
`known_model_rates`) added to `executor/src/config.rs`; the known-model rate
registry moved to the executor lib and `DashboardConfig`/`ArchitectConfig` share
it via `effective_rates()`; `ExecutorConfig.tier` + `BudgetConfig.gate_retries` +
resolution helpers; new `rexymcp calibrate LARGE|MEDIUM|SMALL` CLI
(`mcp/src/calibrate.rs`, `toml_edit` write-back) + `Calibrate` clap variant. 799
passed. **Two spec-defect observations from review, both fixed in follow-up
commit `4b375b3`:** (1) `[architect]`/`[escalation]` emitted as floating inline
tables above `[executor]` rather than `[section]` headers; (2) `gate_retries`
persisted on re-calibrate *to* LARGE (only skipped-on-write, never removed) —
fixed to write proper TOML headers and strip stale `gate_retries` on LARGE.

**M19 — Structural Gate Enforcement — done** (2026-06-16, **approved_after_1**,
executor Qwen/Qwen3.6-27B-FP8 on re-dispatch; original implementation Claude
direct). Single-phase milestone. Closed the recurring `false_completion` class
structurally: `execute_phase` now inspects the `Gates` result from
`run_command_set` before returning `PhaseResult::Complete`, injects a red gate's
output back into the conversation, and loops. Two files:
`executor/src/agent/command.rs` (new `gate_failure_feedback` helper) +
`executor/src/agent/mod.rs` (restructured `NoToolCall` completion arm). **Bounced
once** (bug-01-1, minor): the original complete gutted
`format_hook_failure_does_not_halt_turn` — swapped its failing format command
for a passing one, leaving the (out-of-scope) post-write hook advisory path
uncovered; re-dispatch restored it with `ScriptedCommandRunner::new(vec![false,
true])` in one pass. **Pending calibration folds awaiting user sign-off** (see
M19 retrospective): (1) `prod_unwrap` 3rd occurrence; (2) `false_completion`
dominant class — now arguably *resolved by M19* rather than a WORKFLOW change;
(3) new M19 datum: `weakened_test` (gutting an existing test to pass trivially)
— one occurrence, data only.

**M18 phase-07 — done** (2026-06-15, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8; commit `b93e9d5` fix). The cleanup-thread tool-surface
expansion: `write_file` append + line count, `search` `context_lines` (cap 5,
no-context path byte-identical), `find_files` `depth` via `WalkBuilder`, and
three new `Category::Write` tools `patch_lines`/`delete_file`/`move_file` wired
into `mod.rs`/`router.rs`/`runner.rs`. **Bounced once** (bug-07-1, blocker): the
first complete shipped the three new tools as **orphan files** never wired in —
their tests never compiled, so "766 passed" was false confidence — plus a red
fmt gate and two prod `unwrap`s in `search.rs`. Re-dispatch cleared all three in
one 68-turn pass: 766 → **780 tests** (the 14 previously-dead new-tool tests now
compile), all four gates green on independent re-run. **Calibration (2 folds
pending user sign-off, see M18 retrospective):** (1) `prod_unwrap` 3rd occurrence
crosses the WORKFLOW "three = fold" line; (2) `false_completion` is now the
dominant recurring class — M19 addresses it structurally rather than by per-phase
pre-injection.

**M18 phase-07 — activated** (2026-06-15): the cleanup-thread tooling phase,
**pre-drafted 2026-06-14** and re-verified current at activation — all
load-bearing line refs still exact after phases 05–06 (`runner.rs`
`build_registry` tool vec at 142–150, above phase-06's resolve edits at 185+;
`write_file.rs` parent-dir guard 75–87 / write-fail arm 94–100; `router.rs`
`categorize` 14–23 / `built_ins` test 76–84; `find_files.rs`/`search.rs`
untouched since May). Seven additive tool-surface improvements, no new
`Cargo.toml` dep: (1) `write_file` append mode + line count; (2) `search`
`context_lines` (capped 5, byte-identical no-context path pinned); (3)
`find_files` `depth` via `WalkBuilder`; new tools (4) `patch_lines`, (5)
`delete_file`, (6) `move_file`, all `Category::Write`, registered in `runner.rs`
tool vec + `router.rs` `categorize` + `mod.rs` re-exports. ~450 lines, size=l.
**Independent of phases 03–06** (additive to existing tools). No MCP/CLI surface
change; E2E is build-green + tests (no fabricated transcript).

**M18 phase-06 — drafted** (2026-06-15): closes thread 3 — wires phase-05's pure
`Config::resolve_for_model` into the live dispatch path (`mcp/src/runner.rs`) so a
model's `[models."<id>"]` overrides actually take effect, and documents the
`[models]` section in the `rexymcp init` template (`mcp/src/init.rs`). **Two
resolve sites by design (pre-injected rationale):** `temperature`/`seed` are
consumed in two functions across the prod/test seam — the **wire client**
(`OpenAiClient::new` in `run_phase`, bakes sampling at construction) and the
**loop deps/telemetry** (`run_phase_with`, builds `GenerationParams`/governor/
`task_tracking`). `run_phase` passes **unresolved** `inp.cfg` down, so each
function resolves its own clone; neither resolve is removable without breaking its
consumer. Only the six overridable knobs switch to the resolved clone; non-
overridable reads (`commands`/`budget`/`context`/timeouts) stay on `inp.cfg`
(also dodges the `commands: &inp.cfg.commands` borrow). **Testability seam:**
`run_phase` uses Real verifier/runner (not hermetic), so the wiring test lives at
`run_phase_with` (Noop seams, `MockAiClient`) and observes resolution via the
real `telemetry::append`→`read` round-trip on `generation_params.temperature`
(the one hermetically-observable resolved knob — governor/task_tracking aren't in
`PhaseResult`, but resolve through the same call). Positive + unknown-model
negative + an `init` template-doc test. `config.rs` untouched. ~130 lines, 2
files.

**M18 phase-05 — drafted** (2026-06-15): opens thread 3 (model-conditioned
runtime knobs). Adds a `[models."<id>"]` override table to `rexymcp.toml` and a
pure `Config::resolve_for_model(&mut self, model)` in `executor/src/config.rs`
that applies a matching model's overrides (`task_tracking`, `temperature`,
`seed`, the three governor thresholds) **in place** over the global
`[executor]`/`[governor]` defaults. **Pure substrate, mirrors the 03→04 split:**
this phase ships the config types + resolution fn with full unit coverage but
**does not wire it into `runner.rs`** — that (plus the `rexymcp init` template
docs) is phase-06, the immediate pinned consumer. In-place mutation is the
low-blast-radius shape: the dispatch path (`runner.rs:185,226-228,232-233`)
already reads `cfg.executor.{task_tracking,temperature,seed}`/`cfg.governor`
directly, so phase-06 becomes a single resolve call before those reads.
**Pre-injected gotchas:** (1) `.cloned()` + `let-else` on the `models.get` lookup
to end the immutable borrow before mutating sibling fields (borrow-check stall
guard); (2) **exact-match only** (pinned negative: `[models."qwen"]` must not
apply to `"qwen2.5-coder"`); (3) `None` override field = inherit, not reset
(pinned); (4) **no `#![allow(dead_code)]`** — `config.rs` is in the `executor`
**lib** crate where unused `pub` is fine, unlike phase-03's `mcp`-binary-crate
`profile.rs` (bug-03-2). **Router breadth scoped out** (no global config home
today — it's a router constant; a separate concern). ~200 lines, single file.

**M18 phase-04 — done** (2026-06-15, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8; commit `5f236a0` feat). The phase-03 profile now has its
two runtime surfaces: a **`rexymcp profile` CLI** (`mcp/src/profile_cli.rs`
`load_profiles` + `format_profiles`, a `Commands::Profile` clap variant +
dispatch arm in `main.rs`) **and** a **`model_profile` MCP tool**
(`server.rs` `ModelProfileParams`/`ModelProfileOutput`/`model_profile_inner` +
`#[rmcp::tool]` method), both mirroring the `scorecard` pair. **Dead-code loop
closed:** `#![allow(dead_code)]` removed from `profile.rs` now that
`aggregate_profiles`/`ModelProfile`/`is_model_attributable` have production
callers. Both gotchas held: no `fold_reviews` (raw runs+reviews to
`aggregate_profiles`), and the formatter calls `is_model_attributable` to
parenthesize `spec_bug`/`infra_blip`. **Clean approved_first_try** (36-turn
re-dispatch). All four gates green on independent re-run (749 passed / 2
ignored); the non-attributable formatter branch mutation-verified; live CLI
against real telemetry rendered `false_completion×1 (spec_bug×1)` (spec_bug
parenthesized, attributable bare) and `--json` parsed as a 26-row
`Vec<ModelProfile>`. **Calibration (no fold):** the *first* dispatch was
interrupted before the bookkeeping step, leaving uncommitted work in the tree
(no bug, no review bounce); the re-dispatch completed it cleanly — a recurrence
of the dirty-tree-at-dispatch operational pattern, not a model defect.

**M18 phase-04 — drafted** (2026-06-14): surface the phase-03 profile to the
architect via a **`rexymcp profile` CLI** (`mcp/src/profile_cli.rs` `load_profiles`
+ `format_profiles`, a `Commands::Profile` clap variant + dispatch arm) **and** a
**`model_profile` MCP tool** (`server.rs` `ModelProfileParams`/`ModelProfileOutput`/
`model_profile_inner` + `#[rmcp::tool]` method) — both mirroring the `scorecard`
pair almost line-for-line. **Phase closes the dead-code loop:** the
`#![allow(dead_code)]` phase-03 added to `profile.rs:1` is **removed** here, since
`aggregate_profiles`/`ModelProfile`/`is_model_attributable` now have real callers.
**Two pre-injected gotchas:** (1) unlike `scorecard_cli`/`model_scorecard`,
**no `fold_reviews` call** — `aggregate_profiles` folds internally and needs the raw
`reviews` for `failure_class`; both surfaces pass raw runs+reviews. (2) the human
formatter **must** call `is_model_attributable` (renders `spec_bug`/`infra_blip`
parenthesized) — that is what gives the fn a production caller so the dead-code
attribute can come out. Reuses `scorecard::MAX_ROWS` + `ScorecardFilter`; no new
dep. ~320 lines.

**M18 phase-03 — done** (2026-06-15, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8; commits `0967f92` fix / `3e44de9` approve). The pure
`model_profile` aggregation layer landed in `mcp/src/profile.rs`: `ModelProfile`
+ `FailureClassCount` + `is_model_attributable` + `aggregate_profiles` (per-`(model,
tag)` strengths from folded runs **and** ranked failure classes joined from the
latest matching review), 8 hermetic tests. **Bounced once** (bug-03-1
`false_completion` — self-reported complete on red fmt+clippy gates; bug-03-2
`spec_bug` — clippy dead-code wall because `mcp` is a **binary** crate where unused
`pub` items are dead code under `-D warnings`, unlike the `executor` lib crate
phase-01 lived in). Fix added an **authorized** `#![allow(dead_code)]` at
`profile.rs:1` with a removal note (phase-04 removes it). An earlier
`IdenticalToolCallRepetition` hard_fail (oversized single `write_file` → `arguments:
null` ×6) was handled by refined re-dispatch with a staged-write instruction, not a
review bounce. Verdict recorded into live telemetry via `rexymcp review` and
verified folded onto the latest run. **Calibration:** 1st lib-vs-bin dead-code
occurrence (data, no fold); recurring dirty-tree-at-dispatch sweep (commit `0967f92`
swept pre-existing dirty `NEXT.md`/`phase-07` files — architect-side Pre-flight-4
miss).

**M18 phase-02 — done** (2026-06-14, **approved_first_try**, executor
Qwen/Qwen3.6-27B-FP8). The write-back loop is now live end-to-end: a new
`rexymcp review` CLI (`mcp/src/review.rs` `record_review` + `Commands::Review`
clap variant, clock injected at `main.rs`) appends a `PhaseReview` annotation,
and `read_reviews`+`fold_reviews` were folded into the three read paths
(`runs.rs`, `scorecard_cli.rs`, `server.rs::model_scorecard`) so supervision
columns carry real data. `/rexymcp:review` SKILL §7/§8 wired to invoke the CLI.
Identity gotcha pinned and held: `--phase-doc` is stored **verbatim** (never
canonicalized) so fold matches the executor's stored `phase_doc_path`.
Mutation-verified at review (removing the fold line fails
`load_runs_folds_review_verdict`). Dogfooded into rexyMCP's own production
telemetry — the phase-02 approval folded onto the real Qwen run while the older
gemma run stayed `None` (correct latest-run-only semantics). Commits `39cee12`
(feat) + `6bd02b9` (approve). One calibration note (not a bounce): a custom-named
`--telemetry-path` override writes to `phase_runs.jsonl` in that file's parent
(documented as "the phase_runs.jsonl path" — matches intent). **Phase-03 (drafted)
opens thread 2:** the pure `model_profile` aggregation layer (`mcp/src/profile.rs`
`ModelProfile` + `aggregate_profiles` + `is_model_attributable`) — per-`(model,
tag)` strengths from folded runs **and** ranked failure classes joined from
reviews; no CLI/MCP surface (that is phase-04).

**M18 phase-01 — done** (2026-06-14, **approved_after_1**, executor
Qwen/Qwen3.6-27B-FP8). Store-layer review write-back substrate landed in
`executor/src/store/telemetry.rs`: `PhaseReview` annotation record +
`REVIEW_RECORD_TAG` discriminator, `FAILURE_CLASSES` vocabulary +
`is_known_failure_class`, `append_review`/`read_reviews`/`fold_reviews`, 9 tests.
Bounce ([bug-01-1](milestones/M18-capability-adaptation/bugs/bug-01-1.md), minor):
the phantom-review guard test was not mutation-resistant — `sample()`'s
`architect_verdict: None` serializes to `null`, which fails to deserialize into
`PhaseReview`'s required `String`, so the run line was dropped by the `.ok()`
parse-failure *before* the `.record` filter ran; fixed by giving the test run a
non-null verdict. **Phase-02 (drafted) closes the loop:** the `rexymcp review`
CLI (producer) + folding `read_reviews`/`fold_reviews` into the three read paths
(`runs.rs:173`, `scorecard_cli.rs:31`, `server.rs:292`) + wiring the
`/rexymcp:review` skill to call the CLI.

**📌 M18 — Capability-Aware Adaptation kicked off (2026-06-13, with the user).**
Make rexyMCP characterize each local model's strengths/failure modes and
compensate for them (draft-time + runtime) instead of per-phase trial-and-error.
Milestone [README](milestones/M18-capability-adaptation/README.md) written;
`architecture.md` §Status #18 added. **Foundational discovery driving phase-01:**
the supervision half of the eval loop was never wired — the executor writes every
`PhaseRun`'s `architect_verdict`/`bounces_to_approval`/`bugs_filed`/`warnings` as
`None` (`executor/src/agent/metrics.rs:121-124`), the store is append-only, and
**no write-back path exists**. The architect's verdict has only ever lived in
phase-doc prose, so "trial and error" can't compound. **Four threads** (README):
(1) supervision write-back substrate — an append-only `PhaseReview` annotation
folded onto its `PhaseRun` by phase identity, written by a new `rexymcp review`
CLI, carrying a structured **failure-class** taxonomy; (2) per-model capability
profile (strengths + ranked failure classes) surfaced to the architect at draft
time; (3) model-conditioned runtime knobs (`task_tracking`/governor
thresholds/router breadth/sampling) from config instead of globals; (4)
cold-start calibration battery — **shelved** for later revisit (with the user,
2026-06-13), out of M18's committed scope; when picked up it needs a talk-through
+ `architecture.md` precedence decision (it departs from the "passive telemetry
only" principle). **Decisions locked with the user (2026-06-13):** write-back is
an **append-only annotation record folded at read time** (not in-place rewrite);
the trigger is a **`rexymcp review` CLI** the `/rexymcp:review` skill calls.
On-demand drafting: only phase-01 is drafted; expand 02–06 via
`/rexymcp:architect next`. Phase-07 (thread 4) is shelved, not blocked.

**M17 — Dashboard Polish (Round 3) is complete** (9/9 phases `done`, 2026-06-12).

**M17 phase-09 — done** (2026-06-12, **approved_after_1**; commit `abd11e6` fix /
approve pending): tool-call presentation — per-tool glyphs on `Parsed` headers
(`📖 read_file`, `⚡ bash`, … default `🔧`), the `ToolResult` paired under its call
(`╰ [status]` connector + `RESULT_INDENT` lead, redundant tool name dropped) via a
`PendingCall` slot generalizing the old `read_file`-only `last_read_path` thread,
and the `tool result` filter toggle merged into `tool call` (`FILTER_ITEM_COUNT`
15→14, contiguous `0..14` renumber, both `Parsed` and `ToolResult` route to
`self.tool_call`). Display-only: no `SessionEvent`/`Cargo.toml`/`log_query.rs`/
`render.rs`/`event_loop.rs` change. **Bounced once — [bug-09-1](milestones/M17-dashboard-polish-3/bugs/bug-09-1.md)
(major):** the first dispatch self-reported `complete` on a red suite — two pairing
tests indexed the result header at `rendered[1]` (where the preceding `read_file`
call's multi-line JSON args body sits) and a clippy `function record_lines is never
used` error was masked with `#[allow(dead_code)]` after the `transcript_lines`
rewrite removed the helper's only production caller. Re-dispatch fixed both: tests
now scan `rendered` with `.iter().find(...)`, and `record_lines` is honestly gated
`#[cfg(test)]`. Clean re-run: all four gates green, **384 mcp + 739 executor + 0
doc, 0 failed** (2 ignored); pairing test mutation-verified (forcing `paired=false`
fails it). **Calibration: 2nd self-report-vs-gate-exit disagreement in M17** (a
trend, not yet a fold — see the [M17 retrospective](milestones/M17-dashboard-polish-3/README.md#retrospective--2026-06-12)).

**M17 phase-07 — done** (2026-06-11, **escalated**; commit pending `docs: approve`):
three-scope savings in the Budget panel — **session** (live), **milestone**
(cumulative `PhaseRun` records for the active milestone), **project** (all
`PhaseRun` records). Recorded `phase_doc_path` in every `PhaseRun` (`#[serde(default)]`
`Option<String>`, legacy records → `None`) and threaded `telemetry_dir` from config
through `run_dashboard`→`run_loop`→`load_data`. `dollars_saved_line` replaced by a
multi-line `savings_lines`: a `Savings` header + value-aligned `Session:`/`Milestone:`/
`Project:` rows (right-aligned so decimals share a column). **Escalated:** Qwen3.6-27B
returned `complete` on a red test suite — `project_savings` was computed only in
`load_data`'s `Ok` branch, so the empty-sessions `Err` path reported `(0,0)`. Architect
took over (no bounce): lifted the telemetry read + `project_savings` fold above the
`match`. Calibration data point logged (self-report vs gate-exit disagreement). All
gates green on independent re-run: **736 executor + 377 mcp** pass.

**M17 phase-05 — done** (2026-06-11, approved_first_try; commit `83bfc15` feat):
two Activity-panel highlighting upgrades on the **existing syntect** path (no new
dependency, no tree-sitter). (1) Markdown-highlight Completion **answer** text via a
per-line `markdown_line` helper (`<think>` text stays dim-italic); (2)
**extension-detected** grammar for `read_file` results —
`highlighted_body_lines_for(content, path)` prefers the file extension's syntax,
falling back to today's content `detect_syntax`. Low-churn shapes held per the
M10/M12 churn calibration: `highlighted_body_lines(content)` and `record_lines(rec)`
stayed as zero-extra-arg **delegating wrappers**, so their existing callers/tests are
untouched; `transcript_lines` rewrote its `flat_map` to a `for` loop threading the
most-recent `read_file` `path` to the following `ToolResult` (consumed after use,
content-detection fallback if the `Parsed` call was filtered out). Clean **82-turn
first-try**; all four gates green on independent re-run (fmt/build/clippy clean;
**734 executor + 370 mcp** pass, 2 ignored). New tests mutation-resistant
(`highlighted_body_lines_for_prefers_extension` distinguishes the extension path from
the `detect_syntax` fallback; `transcript_lines_highlights_read_file_by_extension`
pins the path-threading). The replaced `completion_body_no_markers_matches_plain`
test became `completion_body_no_markers_preserves_content` (styling now differs,
content preserved). No `Cargo.toml`/`SessionEvent`/`detect_syntax`-heuristic change.
E2E is a TUI render (N/A per dashboard-panel precedent; unit tests render the real
`Line`/`Span` output). Two cosmetic, semantically-identical departures from the
spec's literal call shape were accepted (both delegate identically). The local-LLM
Update-Log identity self-stamp ("Claude (Sonnet 4.5)") persists; **date correct**
(`2026-06-11`).

**M17 phase-04 — done** (2026-06-11, approved_first_try; commit `f4bcf46` feat /
`919d217` approve): **pan** overflowing Tasks-panel titles back and forth
(ping-pong triangle wave) within the panel width so the whole name reads over time;
fitting titles don't move. Added a pure `scrolled_title(title, max, tick)` helper +
`TASK_SCROLL_DELAY` const in `panels.rs` and threaded a third `tick: Option<usize>`
arg (the live `state.spinner` clock) through `tasks_lines` and all **6** call sites
(1 prod `render.rs:251` + 5 test) in one pass — no churn stall. The frozen (`None`)
path delegates to `truncate_title` (ellipsized head); the scrolling (`Some`) path is
a raw char-indexed `max`-wide window. 5 new mutation-resistant tests (distinct 30-char
`FIXTURE` recovers true start indices; `scrolled_title_ping_pongs` pins
`max_start == overflow` + non-monotonicity, refuting a wrap-around impl); 363 mcp +
734 executor pass, all four gates green on independent re-run. **Calibration:** the
*first* dispatch (session `phase-04-6a2b00fc`) `hard_fail`ed on a **transient backend
decode error** (`error decoding response body`) at turn 34 — infra blip, not a work
defect; the executor's `scrolled_title` logic was already correct. The refined
re-dispatch (per the escalate skill) also fixed two architect-authored **test-plan**
defects the blip surfaced — a §2-vs-Test-plan contradiction on the frozen-head
ellipsis, and a repeating-digit fixture that breaks `find`-based start recovery —
both folded into the doc before re-dispatch; landed clean. No
`SessionEvent`/config/`Cargo.toml` change.

**M17 phase-03 — done** (2026-06-11, approved_first_try; commit `c251062` feat /
`635dab8` approve): added the `Milestone:` row as the **first** Session-panel line,
its name derived from the milestone *directory* holding the running phase's doc —
no config field, no `SessionEvent`. Added `milestone: Option<String>` to
`DashboardData` + a `TempDir`-tested `resolve_milestone` (prefers the non-`done`
match, falls back to highest-numbered) + `format_milestone_name`/`milestone_number`
helpers in `mod.rs` + a pure `milestone_line` builder in `panels.rs` reusing
`truncate_title`; composed in `render.rs` via the optional-line precedent so
`session_lines`' signature (and its 8 test call sites) stayed untouched. 8 new
tests (7 `mod.rs` + 1 `panels.rs`); 734 pass, all four gates green on independent
re-run. Spec-exact, clean 64-turn first-try. No `session_lines` signature change,
no `SessionEvent`/config/`Cargo.toml`.

**M17 phase-02 — done** (2026-06-11, approved_first_try; commit `9d29eee` feat /
`37c3898` approve): restored the dog-chasing-brain spinner as a **width-parametric**
chase — dog closes on a right-pinned brain, catches it, one `💨` overtake-burst
frame per cycle, chase distance scales with `session_inner_width`. Single-function
body rewrite of `spinner_line` in `panels.rs` (signature unchanged); 5 old
triangle-wave tests swapped for 6 chase tests, mutation-resistant on burst-once
(`_emits_overtake_burst_once_per_cycle`) and width-parametric (`_scales_with_width`:
57 distinct dog offsets at w60 vs 17 at w20). `render.rs`/`event_loop.rs`/`Cargo.toml`
untouched per scope. All four gates green on independent re-run; 350 mcp + 734
combined pass. Clean 42-turn first-try. No `SessionEvent`/config/`Cargo.toml`.

**M17 phase-01 — done** (2026-06-11, approved_first_try; commit `7b905cb`): moved
the Session panel's `last update:` line directly under `duration:` (the
`last_update_line` call relocated out of `render.rs` into `session_lines`) and
capitalized every Session/Budget/Reclaim label (`Phase:`, `Tokens in:`,
`Events:`, …; `$ saved:` left as the symbol). String-literal edits + test
assertion bumps, single concern. No `SessionEvent`/config/`Cargo.toml`.

**📌 M17 — Dashboard Polish (Round 3) kicked off (2026-06-11, with the user).**
Milestone [README](milestones/M17-dashboard-polish-3/README.md) written;
`architecture.md` §Status #17 added (M15/M16 marked done). **Six display-layer
refinements across five phases:** 01 label move + capitalization (xs) · 02
restore full-width dog-chasing-brain spinner (s) · 03 `Milestone:` row from the
milestone directory name (s) · 04 pan overflowing task titles (m) · 05 Markdown +
extension-detected highlighting (m). **No new `SessionEvent`, no new
`Cargo.toml` dependency in any phase.**

**Key design corrections made during drafting (read before dispatching 03/05):**
- **Phase 05 is NOT tree-sitter.** The `mcp` crate already depends on **syntect**
  (`mcp/Cargo.toml:21`); `highlight.rs` already highlights tool-result bodies.
  Phase 05 *extends* the syntect path (Markdown for completions + extension-based
  grammar selection for `read_file` results). The original tree-sitter plan was
  dropped with the user once syntect was found — it would have been a
  dependency-heavy regression. All five phase docs and the README reflect this.
- **Phase 03 milestone source is the filesystem, not config.** The name is
  derived from the milestone *directory* (`M15-dashboard-polish-2` → `M15 —
  Dashboard Polish 2`) by scanning `docs/dev/milestones/` for the running phase's
  doc — no `SessionEvent` or config field.
- **Anti-stall shapes pinned:** phases 03 and 05 use low-churn shapes (compose in
  `render.rs` / add a delegating wrapper) so they do **not** change
  `session_lines` / `record_lines` signatures. Only **phase 04** changes a
  signature (`tasks_lines` gains a `tick` param); its doc enumerates all 6 call
  sites (1 prod + 5 test) to traverse in one pass per the M10/M12 churn-stall
  calibration.

On-demand drafting note: all five phase docs are already drafted (the user asked
for the full milestone up front). Dispatch them in order; review-gate each.

**M15 phase-03 — done** (2026-06-11, approved_first_try): model-aware `$ saved`
pricing. Added `saved_model: Option<String>` to `DashboardConfig` (dropped `Copy`
from the derive — the pinned `Option<String>`-isn't-`Copy` gotcha, traversed
first-try) + a pure `model_rates(&str) -> Option<BudgetRates>` lookup in
`panels.rs` (Fable/Mythos $10/$50, Opus 4.8/4.7/4.6 $5/$25, Sonnet 4.6 $3/$15,
Haiku 4.5 $1/$5) + `mod.rs` re-export + a trivial `Option`-fallback wiring at
`main.rs:369` + `init.rs` template comment. All five files match the spec
byte-for-byte; existing config tests untouched. 3 mutation-resistant `model_rates`
tests; 348 mcp + 734 executor pass, all four gates green on independent re-run.
Reviewer extended the executor's E2E gap: the real binary parses a
`[dashboard] saved_model` toml (load succeeds; only the expected `unreachable`
health error fires), confirming the config-load half end-to-end; the full
`$ saved` render is the TUI dashboard (E2E-N/A per prior dashboard-phase
precedent). Clean 50-turn first-try; commit `38cc819` (feat) + approve. No new
`SessionEvent`, no `Cargo.toml`. The cosmetic identity self-stamp persists ("Claude
(Sonnet)"; executor is Qwen) but the **date is correct** (`2026-06-11`) — M11
phase-06 datetime injection confirmed live post-restart.

**M16 phase-01 — done** (2026-06-10, approved_first_try): extended
`parse_heading_task_line` in `executor/src/agent/tasks.rs` to recognize
`### Task N — Title` / `### Task N: Title` / `### Task N. Title` subheadings
alongside the existing `### N. Title` format. Single production file, additive:
a new `Task ` prefix branch (`strip_prefix("Task ")` →
`split_once(['—', ':', '.'])` → trim/validate digits + non-empty title) sits
*above* a byte-identical dot-branch, so all prior `tasks.rs` tests pass
unmodified. 3 new tests (the E2E `seed_from_spec_parses_task_dash_heading_format`
is mutation-resistant: seeds 0 pre-fix, 2 after). 734 pass, all four gates green
on independent re-run; clean 35-turn first-try; commit `4157480` (feat) +
approve. No `seed_from_spec`/`parse_task_line`/config/`Cargo.toml` change. The
recurring local-LLM Update-Log clock/identity self-stamp quirk did **not** recur
(stamp `2026-06-11 04:48`, plausibly real — M11 phase-06 datetime injection now
appears live post-`rexymcp serve` restart). The seeder now accepts all three
heading variants, closing the M13→M14→M15 format-drift gap. See the
[M16 retrospective](milestones/M16-seeder-robustness/README.md#retrospective--2026-06-10).

**✅ Closed contract-doc item (2026-06-10, commit `2acd6e3`):** the
`WORKFLOW.md` accepted-Spec-formats documentation now lists all three seeder
formats — `N.` list-item, `### N.` numbered subheading, and `### Task N — / : / .`
prefixed subheading — plus the "key updates by task number `N`, not the phase
number" gotcha (the gap that left M15 phase-02 with zero seeded tasks). Folded in
**both** `docs/dev/WORKFLOW.md` and the `plugin/templates/WORKFLOW.md` bootstrap
template (the template had never even carried the M14 two-format docs). No open
contract-doc items remain.

**📌 M16 — Seeder Format Robustness kicked off (2026-06-10, with the user).**
Diagnosed from session `6a2a3907`: M15 phase-02 seeded **zero tasks** because its
`## Spec` used `### Task N — Title` headings, which the seeder doesn't parse
(`parse_heading_task_line` only handles `### N. Title`). The turn-0 warning fired
correctly; the executor improvised `update_task(id="02")` and the tool **correctly**
rejected it (`no task with id "02"`) — the tool is **not** broken. Per the
"Both" decision: (a) M16 phase-01 broadens the parser (code), and (b) M15
phase-03's Spec was reformatted to `### N. Title` (convention). Milestone
[README](milestones/M16-seeder-robustness/README.md) written; `architecture.md`
§Status #16 added. **Pending after M16:** M15 phase-03 (model-aware pricing, the
last M15 dashboard phase) — its Spec headings are now seed-compatible regardless
of M16's landing.

**M15 phase-02 — done** (2026-06-10, approved_first_try): width-aware task title
truncation. Removed the hardcoded `TASK_TITLE_MAX = 24`; `tasks_lines` now takes
a `width: usize` param and derives `title_max = width.saturating_sub(2)`;
`render.rs` computes `tasks_area.width.saturating_sub(2) as usize`. 3 test call
sites updated; new `tasks_lines_uses_full_panel_width` mutation-verified at review
(hardcoding 24 fails the width=60 assertion). Clean 40-turn first-try; 731 pass,
all four gates green on independent re-run; commit `1eced62`. No
`SessionEvent`/config/`Cargo.toml`.

**M15 phase-02 — done** (2026-06-10, approved_first_try): width-aware task title
truncation. Removed the hardcoded `TASK_TITLE_MAX = 24`; `tasks_lines` now takes
a `width: usize` param and derives `title_max = width.saturating_sub(2)`;
`render.rs` computes `tasks_area.width.saturating_sub(2) as usize`. 3 test call
sites updated; new `tasks_lines_uses_full_panel_width` mutation-verified at review
(hardcoding 24 fails the width=60 assertion). Clean 40-turn first-try; 731 pass,
all four gates green on independent re-run; commit `1eced62`. No
`SessionEvent`/config/`Cargo.toml`.

**M15 phase-01 — done** (2026-06-10, approved_first_try): moved the
`last_update_line` push from the budget vec to the session vec in `render.rs`
(after `session_lines()`, before the spinner) so `last update:` shows in the
Session panel; changed the Activity `[+Xs]` timestamp span color from dim grey
`Rgb(128,128,128)` to dull yellow `Rgb(180,150,50)` in `transcript.rs` (test
renamed `transcript_lines_timestamp_span_is_dull_yellow`, mutation-resistant);
updated the stale panic message in `session_lines_omits_last_update`. The two
other `Rgb(128,128,128)` usages (tool-call-args body) correctly untouched. Clean
43-turn first-try; 731 pass, all four gates green on independent re-run; commits
`77c3c27` (feat) + `ef14d74` (docs). No `SessionEvent`/config/`Cargo.toml`.

**📌 M15 — Dashboard Polish (Round 2) kicked off (2026-06-10, with the user).**
Milestone [README](milestones/M15-dashboard-polish-2/README.md) written;
`architecture.md` §Status updated (M13/M14 marked done, M15 entry added). **Three
phases:** 01 cosmetic layout/color (xs, done) · 02 width-aware task titles (xs) ·
03 model-aware pricing (s). Pure display for phases 01–02; phase 03 adds one
optional `String` field to `DashboardConfig`. No new `SessionEvent`, no new
`Cargo.toml` dependency.

**M14 — Cleanup is complete** (2/2 approved_first_try, 2026-06-10; see the
[retrospective](milestones/M14-cleanup/README.md#retrospective--2026-06-10)).

**M14 phase-02 — done** (2026-06-10, approved_first_try): the deferred M12/M13
cleanup sweep — removed the two prod `eprintln!` in `mcp/src/server.rs` (collapsed
`Matched`/`NoSources` into one no-op arm; trimmed the progress-token comment),
fixed the stale `RUNAWAY_OUTPUT_BYTES` doc-comment in `read_file.rs:17` to name the
live `runaway_output_bytes` config field, and fixed the references-mode
truncation-note copy bug in `symbols.rs` `format_references` (`kind filter` →
`max_results`; definitions-mode note left untouched). One mutation-resistant
negative test (`references_truncation_note_omits_kind_filter`); 731 pass, all four
gates green on independent re-run; all grep acceptance criteria confirmed. Clean
45-turn first-try; commit `784ee70` (chore). The cosmetic Update-Log clock/identity
self-stamp quirk recurred (still pending the `rexymcp serve` restart).
([phase-02-cleanup-sweep.md](milestones/M14-cleanup/phase-02-cleanup-sweep.md)).

**📌 Operational:** `rexymcp serve` restarted (2026-06-10) — M11 phase-06's datetime
injection is now live. Executor Update-Log self-stamping should be resolved.

**M14 phase-01 — done** (2026-06-10, approved_first_try): fixed the silent
`seed_from_spec` failure that produced zero tasks for 6 of 8 M13 phases. Stop
condition `starts_with('#')` → `starts_with("## ")` so `### N.` task-subheadings
no longer terminate the scan; new `parse_heading_task_line` chained via `.or_else`
for the `### N. Title` format; the redundant second `seed_from_spec(&input.phase_doc)`
call in `mod.rs` replaced with iteration over `&seeded`; a turn-0
`SessionEvent::Progress` warning (`stage = "task_seeding"`) when `task_tracking`
is on but seeding yields nothing. `WORKFLOW.md` `## Spec` template now documents
both list-item and subheading formats. 5 new tests (4 unit + 1 integration), all
mutation-resistant; 730 executor + 344 mcp pass, all four gates green on
independent re-run. Clean 60-turn first-try; commit `4fb9324` (fix). The
recurring local-LLM Update-Log clock/identity self-stamp quirk persisted
(cosmetic; machine records correct — still pending the `rexymcp serve` restart).
([phase-01-task-seeder.md](milestones/M14-cleanup/phase-01-task-seeder.md)).

**M13 — Dashboard Polish is complete** (8/8 approved_first_try, 2026-06-10; see the
[retrospective](milestones/M13-dashboard-polish/README.md#retrospective--2026-06-10)).

**M13 phase-08 — done** (2026-06-10, approved_first_try): each Activity transcript
header gained a dim `[+3m12s]`-style **relative timestamp** (item R2), measured from
the session's first record (`record.ts − records[0].ts`) and formatted by the
**existing** `crate::status::humanize_age`. **Single production file**
(`transcript.rs`): a pure `relative_ts(ts, base_ts)` helper + a `transcript_lines`
rewrite that prepends a `Color::Rgb(128,128,128)` timestamp span to each record's
**header line only** (`lines.first_mut()`), bodies untouched. The prefix is added one
layer **up** from `record_lines`, so its signature, ~15 test call sites, and the
header-color tests stayed green untouched; relative-to-**start** (not `now_ms`) → no
`render.rs` edit, no clock param. 5 new tests; 725 mcp+executor pass, all four gates
green on independent re-run. Load-bearing tests confirmed mutation-resistant at review
(`records.first()` → `visible.first()` mutation makes the baseline test fail `[+0s]`
vs `[+4s]`). The dirty-tree-at-dispatch quirk did **not** recur — the draft was
committed before dispatch. Clean 33-turn first-try; commit `14cc751` (feat) +
`6457148` (draft) + approve. No `SessionEvent`/config/`Cargo.toml`. Cosmetic-only
quirk: Update Log self-stamps "Claude (Sonnet 4.5)" / `01:22` (the recurring local-LLM
identity/clock quirk; executor is Qwen/Qwen3.6-27B-FP8; fixed once `rexymcp serve` is
restarted — still pending)
([phase-08-timestamps.md](milestones/M13-dashboard-polish/phase-08-timestamps.md)).

Last completed: **M13 phase-07** — Tasks panel: named tasks with glyphs + done/total
progress gauge ([phase-07-tasks.md](milestones/M13-dashboard-polish/phase-07-tasks.md),
`done`, approved_first_try, commit `9e50f24`/approve `cef22df`).

**M13 phase-08 — drafted** (2026-06-10): each Activity transcript header gains a
dim `[+3m12s]`-style **relative timestamp** (item R2), measured from the session's
first record (`record.ts − records[0].ts`) and formatted by the **existing**
`crate::status::humanize_age` (the Session-panel `duration:` formatter — buckets
match). **Single production file** (`transcript.rs`): a tiny pure
`relative_ts(ts, base_ts)` helper + a `transcript_lines` rewrite that prepends a
`Color::Rgb(128,128,128)` timestamp span to each record's **header line only**
(`lines.first_mut()`), bodies untouched. **Deliberate low-blast-radius shape:** the
prefix is added one layer **up** from `record_lines`, so `record_lines(rec)`'s
signature, its ~15 test call sites, and the header-color tests (which call
`record_lines` directly and read `lines[0].spans[0]`) all stay green untouched.
Relative-to-**start** (not `now_ms`) → stable per record, **no `render.rs` edit, no
clock param**. Load-bearing pins:
`transcript_lines_timestamp_relative_to_first_record_not_first_visible` (baseline is
`records.first()`, computed **before** the filter — a hidden opening event must not
reset the baseline to `+0s`) and `relative_ts_formats_offset_from_base` (`+0s` /
`+5s` / `+3m12s` + saturating guard). No `SessionEvent`/config/`status.rs`-mutation/
`Cargo.toml`. ~60 lines.

Last completed: **M13 phase-07** — Tasks panel: named tasks with glyphs + done/total
progress gauge ([phase-07-tasks.md](milestones/M13-dashboard-polish/phase-07-tasks.md),
`done`, approved_first_try, commit `9e50f24`/approve `cef22df`).

**M13 phase-07 — drafted** (2026-06-10): the Tasks panel shows **named** tasks with
checkbox glyphs (`☑` done / `▶` active / `☐` pending) over a **done/total progress
gauge** (items #7/R3). Today the panel renders three bare count lines and `summarize`
discards `TaskUpdate.title` into a count-only `HashMap`. Two files
(`status.rs` + `dashboard/panels.rs`): `summarize` swaps the `HashMap<id,state>` for
an **insertion-ordered `Vec<TaskRow>`** (id/title/state, last-write-wins per id) so
titles + order are retained; the existing `tasks_total/done/active` counts are
**derived from the vec** (identical results — all existing task tests pass unmodified;
the stall-dodge is that `StatusSummary` is `Default`-built + mutated in `summarize`, so
the new field is a **one-line add, no literal cascade**). `tasks_lines` rewritten to
emit a gauge line + one `{glyph} {title}` line per task (title `…`-truncated to
`TASK_TITLE_MAX`); new pure `tasks_gauge_line(done, total)` renders a `GAUGE_CELLS`-wide
`█`/`░` bar + `done/total (pct%)`, colored **progress-oriented** (green ≥80% / yellow
≥40% / grey else — a deliberate inversion of the context gauge's red floor, noted in the
doc). The gauge matches the context-gauge **style** (a single colored `Line`), **not** a
ratatui `Gauge` widget (would break the `Vec<Line>`→`panel()` composition — pinned out
of scope). **No `render.rs`/`filter.rs`/`transcript.rs` edit** (`tasks_lines` signature
unchanged), no `SessionEvent`/config/`Cargo.toml`. Load-bearing pins:
`summarize_captures_task_titles_in_order` (title + first-seen order, mutation-resistant
vs HashMap/`..`-drop) and `tasks_gauge_line_fraction_and_fill` (3/8 → "38%" + 4 `█`,
mutation-resistant vs wrong divisor / floor fill). ~200 lines.

Last completed: **M13 phase-06** — full-width Session-panel spinner on its own bottom
line ([phase-06-spinner.md](milestones/M13-dashboard-polish/phase-06-spinner.md),
`done`, approved_first_try, commit `002f148`/approve `9f7d0bb`).

**M13 phase-06 — done** (2026-06-10, approved_first_try): the spinner moved **out** of
`session_lines` into a width-aware `spinner_line(spinner, width) -> Option<Line>` pushed
in `render.rs` (the `dollars_saved_line`/`last_update_line` precedent); a dog trots a
triangle-wave offset bounded so `offset + SPRITE_CELLS <= width`. The `spinner` param was
dropped from `session_lines` (all 10 call sites updated, clean), `SPINNER_FRAMES`
retired, header band grown `Length(9)`→`Length(10)`. 725 executor pass, all four gates
green; load-bearing `spinner_line_never_exceeds_width` + `spinner_line_bounces_at_right_edge`
mutation-resistant. Clean first-try; commit `002f148` (feat). Calibration: 2nd-plus
dirty-tree-at-dispatch (commit swept the at-dispatch NEXT.md/phase-doc edits) — commit
ambient activation edits *before* dispatch next time.

Last completed: **M13 phase-05** — session `duration:` line + `last update:` moved to
Budget (items #4/#5)
([phase-05-timing.md](milestones/M13-dashboard-polish/phase-05-timing.md), `done`).

**M13 phase-05 — drafted** (2026-06-10): two header-panel timing lines. Three files
(`status.rs` + `panels.rs` + `render.rs`): add `started_at: Option<u64>` to
`StatusSummary` (earliest record ts, **symmetric** with the existing `last_ts` max-ts
fold — one struct line + one `summarize` assignment, no production literal cascade) →
a pure `session_duration_ms(summary, now_ms)` (live `now−start` while running,
**frozen** `last_ts−start` once ended) drives a new Session-panel `duration:` line;
the `last update:` block **moves out** of `session_lines` into a new pure
`last_update_line(summary, now_ms) -> Option<Line>` that `render.rs` **prepends** to
the Budget vec. **Deliberate stall-dodge:** the relocated line reuses the
`dollars_saved_line` optional-line-pushed-in-render precedent so **no `now_ms` param
is added to `budget_lines`** — its 9 test call sites + `session_lines`' signature stay
untouched (no multi-site signature churn). **Pinned negatives:**
`session_duration_ms_ended_uses_last_ts` (frozen-on-end, mutation-resistant vs a
`now_ms` impl), `last_update_line_none_for_empty_log`, and a `session_lines`
must-NOT-contain `last update:` (the line moved, not duplicated). CLI `format_status`
explicitly **out of scope** (separate renderer, keeps its own `last update:`). No
`SessionEvent`/config edit. ~120 lines.

**M13 phase-04 — done** (2026-06-10, approved_first_try): distinct dim-italic styling
for `<think>…</think>` reasoning vs soft-white answer text in Completion bodies. Two
mcp files (`highlight.rs` + `transcript.rs`): a pure `split_think_segments(raw)`
literal-marker tokenizer (initial mode = think iff a `</think>` precedes any
`<think>` — covers stripped-opening output; unterminated `<think>` → rest is
think; `<thinking>` is **not** a marker) + a `completion_body_lines(raw)` renderer
that reuses the existing `body_lines` indent/cap/overflow-marker shape so the
**no-markers path is byte-identical** to today's `plain_body_lines`; the Completion
arm of `record_lines` swaps to it. 9 tests; both README negatives covered +
load-bearing (`completion_body_no_markers_matches_plain`,
`split_think_segments_handles_no_opening_tag`); 725 executor pass, all four gates
green on independent re-run. Clean 42-turn first-try; commits `ab1968a` (draft) +
`a928355` (feat) + `da580f6` (approve)
([phase-04-think.md](milestones/M13-dashboard-polish/phase-04-think.md)). No
`Cargo.toml` (`Modifier` already in `ratatui` 0.30, `Modifier::BOLD` already used).
Display-only per the M13 constraint. Cosmetic-only quirk: Update Log self-stamps
"Claude (Sonnet 4.5)" / its own `cargo test` count — the recurring local-LLM
identity/clock self-stamping quirk (executor target Qwen/Qwen3.6-27B-FP8; machine
records correct; fixed once `rexymcp serve` is restarted — still pending).

**M13 phase-03 — done** (2026-06-10, approved_first_try): Activity line wrapping
+ tail-follow autoscroll over the **wrapped** count + right-edge scrollbar (items
#8, #9, R1). Two prod files (`render.rs` + `event_loop.rs`): pure span-preserving
`wrap_line`/`wrap_lines` hard-wrap each transcript `Line` to the panel
`inner_width`; `render_dashboard` returns the wrapped total so the width-less
event loop clamps the manual offset correctly; the pre-wrapped lines render with
**no** ratatui `Wrap` (so render == counted rows, follow math correct by
construction); `Scrollbar`/`ScrollbarState` on the right border. **Both pinned
gotchas held** — no `Paragraph::line_count` (unstable/feature-gated), no
`Cargo.toml` edit. 6 tests (load-bearing `wrap_lines_total_drives_follow_offset`
pins the wrapped count → follow-offset fix, mutation-resistant); 725 + 312 mcp
pass, all four gates green on independent re-run. Clean 53-turn first-try; commit
`2a8b73b` (feat) + `87669f9` (draft); approved this verdict
([phase-03-wrapping.md](milestones/M13-dashboard-polish/phase-03-wrapping.md)).
Cosmetic-only quirk: the Update Log's "Commits: pending" is stale (it did commit)
and its time stamps are off — the recurring local-LLM self-stamping quirk;
machine records are correct.

**M13 phase-02 — done** (2026-06-10, approved_first_try): surfaced
`Prompt.rendered` (soft-white body) + `Parsed.tool_call.arguments` (dim
`Rgb(128,128,128)`, header-only on `{}`/null) as transcript bodies in
`record_lines`; 4 tests; 725 pass. Commit `1c06116` (feat) + `f6ee6c3` (approve)
([phase-02-payloads.md](milestones/M13-dashboard-polish/phase-02-payloads.md)).
M13 phase-01 — Legibility (dark-grey → `Rgb(200,200,200)`) is **done** and
approved (approved_first_try, 2026-06-10) —
([phase-01-contrast.md](milestones/M13-dashboard-polish/phase-01-contrast.md)).
Phases 04–08 remain `todo` and undrafted; draft the next one on demand with
`/rexymcp:architect next`.

**📌 M13 — Dashboard Polish kicked off (2026-06-10, with the user).** Milestone
[README](milestones/M13-dashboard-polish/README.md) written; `architecture.md`
§ Status #13 added + M12 marked done. **Locked scope: all 10 requested dashboard
improvements + 4 enhancements (R1 scrollbar, R2 timestamps, R3 task gauge, R5
spinner status), decomposed into 8 single-concern phases — pure presentation
layer, `mcp/src/dashboard/` + read-only `StatusSummary`/`summarize` adds, NO
`SessionEvent`/loop/config change.** This display-only constraint deliberately
sidesteps both documented stall classes: no new-variant match-arm wall (no new
`SessionEvent`), and `StatusSummary` is `Default`-built so field adds (duration in
05, task list in 07) are a one-line struct add + one `summarize` assignment, not a
literal cascade. **On-demand drafting:** only phase-01 is drafted; expand 02–08 via
`/rexymcp:architect next` as they're dispatched. Phase map (items → phase): 01 #1 ·
02 #2/#3 · 03 #8/#9/R1 · 04 #6 · 05 #4/#5 · 06 #10/R5 · 07 #7/R3 · 08 R2.

**📌 Do before the first M13 dispatch (operational, still open):** **restart `rexymcp
serve`** so the rebuilt binary picks up M11 phase-06's datetime injection — until
then the executor keeps self-stamping hallucinated dates/identity in Update Logs
(seen across all of M12; cosmetic, machine records are correct).

**📌 Carried from M12 (not M13 scope):** the deferred cleanup sweep (two prod
`eprintln!` at `server.rs:426`/`:450`; stale `RUNAWAY_OUTPUT_BYTES` doc-comment in
`read_file.rs:17`; symbols `format_references` truncation-note copy bug) — gather
into a separate micro-phase if the user wants it. The `task_tracking` A/B is fully
shipped; a scorecard analysis of on/off `bounces_to_approval` / `first_pass_rate`
remains an option whenever the user wants to validate Arc A.

**M12 — Executor Tooling is complete** 🎉 (7 phase-docs / 9 dispatches, all approved,
2026-06-10 — see the
[retrospective](milestones/M12-executor-tooling/README.md#retrospective--2026-06-10)).
Zero escalations/takeovers across the milestone (first since M8); the 06a/06b/06c
split isolated both documented stall classes and neither recurred. Two single-bounce
phases (05, 06c), both the same class — production-path `unwrap`/`expect` vs
STANDARDS §2.1 — now a **2-occurrence trend** (3 = fold; watch-item carried below).

**📌 M12 watch-item (held for a 3rd occurrence, do NOT fold yet):** the executor
reaches for `.unwrap()` on locally-provable-safe values (a just-matched parse; a
`Mutex` it owns), missing that STANDARDS §2.1 forbids `unwrap`/`expect` in prod
regardless. Both M12 instances cleared in one re-dispatch. If a 3rd lands, the fix
is a forward-looking gotcha in any `Mutex`/lock or hot-parse phase doc (poison-
tolerant `.lock().unwrap_or_else(|e| e.into_inner())` per `ai/mod.rs`/`jsonl.rs`;
`strip_prefix`/`split_once`/`if let` for parses) — not a STANDARDS edit (the gate
text already says it; the gap is application).

**Prior active-phase pointer (now done):**

**phase-07 done** (2026-06-10, approved_first_try): M12 Arc A render half — the
dashboard `Tasks` panel (active/pending/done) above a half-height Files panel.
mcp-only, two files: `dashboard/panels.rs` gained `tasks_lines(&summary)` (placeholder
`(no tasks tracked yet)` when `tasks_total == 0`; else `active`/`pending`/`done N/T`,
**pending derived** via `total.saturating_sub(done+active)` — no stored field);
`dashboard/render.rs` split the right column `Layout::vertical([50,50])` into Tasks
over Files. Purely additive on the read side (06a/06c already populate the
`StatusSummary.tasks_*` counts); **no** new field/event/config/match-arm churn — the
lowest-risk shape in M12. **722 executor + 298 mcp passed / 0 failed / 2 ignored**,
all four gates green on independent re-run. 3 `tasks_lines_*` tests, mutation-resistant
on the `total≠pending` distinction (`shows_counts` total=3→pending 1 vs
`derives_pending` total=2/done=0/active=0→pending 2 catches a naive "render total as
pending" impl). E2E declared N/A (TUI, no headless harness) — consistent with prior
M8/M10 dashboard-panel phases. Clean 45-turn first-try with full bookkeeping (status
flip + Update Log committed with the code per WORKFLOW); commit `47aee54` (feat);
approved (this verdict). No new dep, no `status.rs` change. Cosmetic-only quirk: the
Update Log self-stamps `2026-06-10 00:00` / "claude-code" (the recurring local-LLM
clock/identity quirk; fixed once `rexymcp serve` is restarted — see above).

**phase-06c done** (2026-06-10, approved_after_1): M12 Arc A model-facing flips —
the `update_task` tool. The `UpdateTask` tool owns the canonical live list in a
`Mutex<Vec<Task>>` (seeded once at registry-build, single Arc-shared instance →
persists across turns); `execute` validates `{id, state}`, flips in place, returns
`{id,title,state}` in `metadata`; the **loop** transcribes that metadata to a
`SessionEvent::TaskUpdate` (copying the `OutputFiltered` metadata→event block), so
the tool never needs the session log. `pub mod tasks` exposure + `tools/update_task.rs`
+ `tools/mod.rs` re-export + `router`/`registry` `Category::Meta` arm +
`prompt::task_section` + 2 loop hooks + `build_registry` `Option<Vec<Task>>` param.
Unknown id / bad state / malformed args → advisory `ToolResult` error (model-visible,
no emit). **Pinned A/B negative held:** off → no `update_task` schema, no `# Task
tracking` prompt section, zero model-driven `TaskUpdate`. **722 passed / 0 failed /
2 ignored**, all four gates green. **One bounce —
[bug-06c-1](milestones/M12-executor-tooling/bugs/bug-06c-1.md) (major):** the first
dispatch left a production-path `.lock().unwrap()` in `update_task.rs:84` (STANDARDS
§2.1 — no `unwrap` in prod); fixed with the poison-tolerant
`.lock().unwrap_or_else(|e| e.into_inner())` idiom already established in
`ai/mod.rs`/`jsonl.rs` (commit `2648cbb`). Otherwise spec-exact. Commits `5791b01`
(feat) + `2648cbb` (fix); approved `c18c6fc`. **Calibration:** first `Mutex`-lock-
unwrap bounce — a data point, not yet a trend (the poison-tolerant idiom exists in
the tree; a forward-looking gotcha could pre-inject it on the next `Mutex`-touching
phase). Cosmetic-only quirk: the Update Log self-stamps `2026-06-10` / "rexyMCP
executor" (the recurring local-LLM clock/identity quirk; phase-06's datetime
injection fixes it once `rexymcp serve` is restarted — still pending).

**Prior active-phase pointer (now done):**

**phase-06b done** (2026-06-09, approved_first_try): M12 Arc A task-tracking
**gate**. Added `[executor] task_tracking` (bool, default on, via
`#[serde(default = "default_task_tracking")]` since `ExecutorConfig` has no
struct-level serde default) + a `pub task_tracking: bool` field on `LoopDeps`,
wired the prod literal (`runner.rs:200` ← `inp.cfg.executor.task_tracking`), wrapped
06a's turn-0 seeding emit (`mod.rs:185-201`) in `if deps.task_tracking`, and
documented the field in the `rexymcp init` template. Off → **zero** `TaskUpdate`
events, byte-identical to pre-06a. **The headline risk — the `LoopDeps`
struct-literal churn (phase-08a/08d stall class) — did NOT recur:** the executor
cleanly traversed all 12 construction sites (1 struct + 1 prod + `deps()` helper + 9
standalone test literals across `tests.rs` + 3 `ExecutorConfig` literals in
`ai/mod.rs` + 1 in `health.rs`, the latter 4 from the *config*-field add) first-try
via the pinned compiler-guided E0063 recipe. **710 executor + 293 mcp passed / 0
failed / 2 ignored**, all four gates green on independent re-run; E2E reproduced
(`rexymcp init` → generated `rexymcp.toml` carries `# task_tracking = true`). Gate
verified load-bearing at review. 87-turn clean first-try with full bookkeeping
(`feat:` commit `5ce7730`); approved `28e9e88`. **The 06b/06c split's intended
payoff** — isolating the literal churn from the new-tool/router/prompt wiring (06c)
worked exactly as the 06a/06b variant-wall split did. Cosmetic-only quirk: the
Update Log self-stamps `2026-06-10` / "Claude (direct)" (the recurring local-LLM
clock/identity quirk; phase-06's datetime injection fixes it once `rexymcp serve`
is restarted — still pending).

**Prior active-phase pointer (now done):**

**phase-06a done** (2026-06-09, approved_first_try — architect closeout of
bookkeeping): M12 Arc A task-tracking **substrate**. `TaskState` (Pending/Active/
Done) + `SessionEvent::TaskUpdate { id, title, state }` in `event.rs`; a new pure
`executor/src/agent/tasks.rs` `seed_from_spec(phase_doc)` parsing top-level numbered
`## Spec` items into `Pending` tasks (std-string, **no `regex` dep**, no
`unwrap`/`expect` per bug-05-1; pinned negatives: indented sub-items, `1.5x`
decimals, out-of-section, no-Spec → empty); the loop emits one `pending`
`TaskUpdate` per seeded item at turn 0 **unconditionally** (no gate — 06b adds it);
the full new-variant match-arm blast radius landed exactly per the worked example
(`filter.rs` 7 sites + `FILTER_ITEM_COUNT` 14→15, `transcript.rs::record_lines`,
`log_query::event_type_str`, `agent/tests.rs::event_kind`); a `rexymcp status`
consumer (`StatusSummary` task counts via last-write-wins `HashMap` + a
`tasks: D/T done (A active)` line). The three non-exhaustive `_ =>` matches
(`cap.rs`, `matches_tool_name_filter`, `aggregate_context_efficiency`) correctly
left untouched. **706 passed / 0 failed / 2 ignored executor + 293 mcp**, all four
gates green independently. E2E reproduced (`tasks: 1/2 done (0 active)` over a
hand-written `task_update` JSONL fixture). Committed `4658633` (feat); approved
`7583208`. **Clean traversal of the full 12-site variant blast radius first-try,
zero stall — the 06a/06b split's intended payoff** (variant match-arm wall isolated
from the `LoopDeps` literal churn). **Calibration:** the session was interrupted by
the user before the executor reached the Update-Log/commit step; architect closed
out the bookkeeping (no code change) — not a model failure (closeout note corrected
in `d9408fd`).

**Prior active-phase pointer (now done):**

**phase-05 done** (2026-06-09, approved_after_1): M12 Arc B's third
code-intelligence win — a structured `cargo test` failure **digest** prepended to
the M10 cargo filter output. Single-file, additive
(`executor/src/context/output_filter.rs`, +325/-20 across both dispatches).
Module-private `TestFailure { name, location, detail }` (no serde, not exported),
pure `parse_test_failures` (walks libtest `---- <name> stdout ----` /
`panicked at <loc>:` / `left:`/`right:` blocks → one record per failed test), pure
`format_failure_digest` (`""` when empty → byte-identical PASS path), and a prepend
hook in `cargo_filter` (`(format!("{digest}{body}"), truncated)` on the single tail
return — the two early returns were folded into one). The model sees
`=== Test failures (N) ===` + one `test <name> failed at <loc> — <detail>` line per
failure before the verbose blocks. **Pinned boundaries held:** `left`/`right`
surfaced **verbatim** (no fabricated expected/actual — the relabel guard asserts
`!contains("expected")`); bare `assert!`/`panic!` surface the message with no
invented left/right; PASS → empty digest → no `=== Test failures` header (pinned
must-not-contain negative, mutation-resistant). 7 new tests over the two real
verbatim `cargo test --color=never` fixtures (FAIL: assert_eq/assert!-msg/panic!;
PASS). **696 passed / 0 failed / 2 ignored**, all four gates re-run green
independently. No new dep, no shell-out, no new `SessionEvent`/struct-field churn.
**One bounce — [bug-05-1](milestones/M12-executor-tooling/bugs/bug-05-1.md)
(minor):** the first dispatch (clean 47-turn, commit `e853479`) introduced four
new `.unwrap()` in the production `parse_test_failures` path; STANDARDS §2.1
permits only `.expect("…")` with an invariant message. Provably-safe but a
contract miss against the enforced "production clean of unwrap/expect" gate. The
tests-and-everything-else were otherwise clean. Cleared on a 37-turn re-dispatch
(commit `a2cdfc4`): all four rewritten idiomatically
(`strip_prefix().and_then(strip_suffix)`, `if let Some = current.take()`,
`split_once`), behavior byte-identical, 696 unchanged. Approved at review. Process
quirk (cosmetic, no fold): the Update Log again self-stamps `00:00`/"rexyMCP
executor" — the recurring local-LLM clock/identity quirk that phase-06's datetime
injection addresses once `rexymcp serve` is restarted (still pending below).

**Prior active-phase pointer (now done):**

**phase-04 approved 2026-06-09 (approved_first_try).**

**phase-04 done** (2026-06-09, approved_first_try): M12 Arc B's second
code-intelligence win — surfaced rustc's **machine-applicable**
`suggested_replacement` spans to the model. Single-file, additive
(`executor/src/governor/verifier.rs`, +57/-1 prod). Private recursive
`collect_machine_suggestions`/`collect_suggestions_into` walk a rustc
diagnostic's `children` for `help` spans carrying a string `suggested_replacement`
+ `suggestion_applicability == "MachineApplicable"`, and **append** one
`rustc suggests (machine-applicable): replace at line L:C with \`REPL\` — <help>`
line per suggestion to the `Diagnostic.message` inside `parse_cargo_line`. **No
new `Diagnostic` field / no `Suggestion` struct** — the ~33-literal churn was
dodged; `Diagnostic`/`DiagnosticSignature`/`signature()`/`render_diagnostics`
byte-untouched; the suggestion flows to retry message / briefing / JSONL for free.
**Pinned boundary held**: only `MachineApplicable` surfaced — the two exclusion
tests use exact `==` on real Fixtures B (E0308 HasPlaceholders) / C (E0425
MaybeIncorrect), mutation-resistant (a "surface everything" impl fails them);
no-suggestion path byte-identical (existing parse tests green). 4 new tests in
`verifier_tests.rs` over three real verbatim rustc-JSON fixtures. **689 passed /
0 failed / 2 ignored**, all four gates re-run green independently; production
clean of `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]`. Clean **34-turn
first-try** with full bookkeeping (status flip + Update Log + `feat:` commit
`d02f40d`); approved `070928f`. No nits, no fold. No new dep. Cosmetic-only
quirk: Update Log self-stamps `00:00`/"rexyMCP executor" (the recurring local-LLM
clock/identity quirk; phase-06 datetime injection fixes it once `rexymcp serve`
is restarted — still pending below).

**Prior active-phase pointer (now done):**

**phase-03 approved 2026-06-09 (approved_first_try).**

**phase-03 done** (2026-06-09, approved_first_try): M12 Arc B's first
code-intelligence win — find-references in the `symbols` tool. Single-file,
additive (`executor/src/tools/symbols.rs`, +584/-75). New `mode: Option<String>`
arg (`"definitions"` default | `"references"`); two reference tree-sitter queries
(`RUST_REF_QUERY` = `(identifier)`/`(type_identifier)`/`(field_identifier)`,
`PYTHON_REF_QUERY` = `(identifier)`); `parse_references` (exact-token match +
trimmed source-line snippet) + `format_references` (`✓ N references to …`,
`{path,name,references,files,truncated}` metadata); references wired into
`execute` + `execute_single_file` honoring the same gitignore/`.rs`/`.py`/
`max_results`/single-file semantics. **References include the definition site**
(def + N calls → N+1). Loud rejection of `kind`+references and invalid `mode`.
The grep-differentiator is **pinned and mutation-resistant**: string/comment
occurrences are excluded (`references_exclude_strings_and_comments` asserts `==2`,
not the 4 a substring grep would yield); `foo` ≠ `foobar`. 11 new hermetic tests
(real tree-sitter over `TempDir`); definitions path byte-identical (19 prior
tests unchanged). **685 passed / 0 failed / 2 ignored**, all four gates re-run
green independently; production clean of `unwrap`/`expect`/`panic`/`unsafe`/
`#[allow]`. Clean **59-turn first-try** with full bookkeeping (status flip +
Update Log + single `feat:` commit `11b34cb`); approved `_`. **One nit (no fold,
no bounce):** `format_references`'s truncation note reuses the definitions copy
"…add a kind filter…", contradictory in references mode where `kind` is rejected
— architect-induced (spec pinned the "same shape"); fix opportunistically in a
future `symbols`-touching phase. No new dep.

**Prior active-phase pointer (now done):**

**phase-02 approved 2026-06-09 (approved_first_try).**

**phase-02 scope (active — M12 Arc 0):** additive new CLI command, mcp-crate only
(mirrors `init.rs`/`health`). New `mcp/src/doctor.rs` with pure helpers
(`command_binary` first-token extractor; `resolve_binary(name, &[PathBuf])`
exact-match PATH resolver — `is_file()` only, no substring, no dir match,
separator-bearing names checked as literal paths; `build_report(&CommandConfig,
&[PathBuf])`; `DoctorReport::tier0_ok()`; `format_report`) + a thin impure `run`
that reads the real PATH via `std::env::var_os`/`split_paths`. **Tier 0** = the
configured `[commands]` binaries (dedup by binary, roles merged) → **required**, a
missing one makes `doctor` exit non-zero. **Tier 1** = the three per-language
verifier enhancers (`cargo`/`tsc`/`ruff`, install hints reused verbatim from
phase-01) → advisory, **fail-open** (never affects exit code — the pinned property
`tier0_ok` ignores Tier 1). Plus a `Commands::Doctor { config, json }` clap
variant + dispatch arm + `mod doctor;`. `--json` mirrors runs/scorecard. ~11
hermetic unit tests (incl. 3 pinned negatives: dir-of-same-name, substring,
blank-command) + 1 clap-parse test + 3 E2E (exit-0 all-present, exit-1
missing-Tier-0, `--json` shape). No new dep (std only). **Out of scope:** language
detection, touching `init`, version-checking, MCP/dashboard/bootstrap wiring.
Executor target: Qwen/Qwen3.6-27B-FP8.

**phase-02 done** (2026-06-09, approved_first_try): `rexymcp doctor` —
toolchain-availability command. New mcp-only `mcp/src/doctor.rs` with the pure
helpers `command_binary` (first-token extractor), `resolve_binary(name, &[PathBuf])`
(exact-match PATH resolver — `is_file()` only, separator-bearing names checked as
literal paths), `build_report(&CommandConfig, &[PathBuf])`, `DoctorReport::tier0_ok()`,
`format_report` + a thin impure `run`/`path_dirs` reading the real PATH. **Tier 0**
= configured `[commands]` binaries (dedup by binary, roles merged into the note) →
required, a missing one exits non-zero; **Tier 1** = the three verifier enhancers
(`cargo`/`tsc`/`ruff`, hints reused from phase-01) → advisory, fail-open
(`tier0_ok` ignores Tier 1). Plus a `Commands::Doctor { config, json }` clap variant
+ dispatch arm + `mod doctor;`. **289 passed** mcp (270 baseline + 19 new, incl. the
3 pinned negatives dir-of-same-name/substring/blank + a clap-parse test). All three
E2E reproduced at review against the real binary (exit-0 all-present with the `cargo`
row showing 4 merged roles; exit-1 `definitely-not-a-real-binary-xyz` MISSING; `--json`
parseable `tier0`/`tier1`). No new dep (std only). Clean 58-turn first-try with full
bookkeeping (status flip + Update Log + single `feat:` commit `45a4c6f`); approved
`_`. Cosmetic-only quirk: the Update Log self-stamps `00:00`/"executor" (the recurring
local-LLM clock/identity quirk; phase-06 datetime injection fixes it once `rexymcp
serve` is restarted — still pending below).

**phase-01 done** (2026-06-09, approved_first_try): verifier missing-binary →
`Skipped` advisory. Added `VerifierResult::Skipped(String)` + the pure
`spawn_failure(tool, install_hint, &io::Error)` classifier (`NotFound` → `Skipped`
naming binary + remedy; else `Failed`), routed the three spawn arms through it,
added `Skipped` arms to `capture_baseline` + the agent-loop verify match. The
no-strike property is structural (only `Checked` pushes to
`recent_verifier_error_counts`), confirmed at review. 3 new tests (2 pure
classifier incl. the `PermissionDenied`→`Failed` negative + 1 loop advisory); one
extra exhaustive-match arm (`python_verifier_handles_missing_ruff` test) was
anticipated by the spec. **674 passed / 2 ignored** executor, all four gates green
on independent re-run. Clean 64-turn first-try with full bookkeeping (status flip
+ Update Log + single `fix:` commit `f2f8759` all landed by the executor);
approved `af18e14`. Cosmetic-only quirk: the Update Log self-labels "rexyMCP
executor LLM" (the recurring local-LLM identity quirk; phase-06 datetime injection
fixes the date once `rexymcp serve` is restarted — still pending below).

**📌 M12 — Executor Tooling kicked off (2026-06-09, with the user).** Milestone
[README](milestones/M12-executor-tooling/README.md) written; `architecture.md`
§ Status #12 marked in-progress. **Locked scope: three arcs —**
**Arc 0 (toolchain robustness)** first, then **Arc B** (all three code-intelligence
wins: find-references + rustc suggested-fix spans + structured `cargo test` failure
parsing), then **Arc A** (task tracking + dashboard panel, `task_tracking` default
on). **Seven planned phases:** 01 verifier missing-binary degrade (`Skipped`, no
governor strike), 02 `rexymcp doctor`, 03–05 Arc B, 06–07 Arc A — the architect
expands each on demand. **Drafting watch-items:** (a) phase-06 adds a new
`SessionEvent::TaskUpdate` variant → the known `dashboard/filter.rs` 7-site
match-arm wall (hard-failed M10 phase-03/04/06); enumerate every arm or split the
mechanical fixups. (b) the toolchain-dependency discipline is folded into
WORKFLOW.md/STANDARDS.md/architect SKILL (commit `5cc2ff2`). (c) no new toolchain
dep in M12 — every shell-out reuses `cargo` (already required) or compiled-in
tree-sitter grammars.

**M11 — Polish is complete** 🎉 (all seven phases approved_first_try, 2026-06-09 —
see the [retrospective](milestones/M11-polish/README.md#retrospective--2026-06-09)).

**📌 Post-M11 operational follow-up (do before the next dispatch):** **restart
`rexymcp serve`** so the rebuilt binary picks up phase-06's datetime injection —
the live MCP process does not reload server code after a rebuild (known
stale-server behaviour). Until restarted, the next executor session still runs the
pre-phase-06 prompt and will keep stamping hallucinated dates.

**📌 Deferred cleanup sweep (out of every M11 phase's scope; gather for a future
micro-phase):** (1) two `eprintln!` in production at `mcp/src/server.rs:426`/`:450`;
(2) stale `RUNAWAY_OUTPUT_BYTES` doc-comment ref in
`executor/src/tools/read_file.rs:17`; (3) the executor's wrong identity
self-labelling in Update Logs (cosmetic).

**Prior active-phase pointer (now done):**

**phase-06 done** (2026-06-09, approved_first_try): executor temporal grounding —
prepended `Today's date is YYYY-MM-DD (UTC).` to the assembled system prompt,
formatted from the injected `deps.clock` epoch-millis. New private
`format_utc_date(now_ms)` (pure civil-from-days integer arithmetic — **no date
dependency**; the original scope sketch's "chrono already a dep" was wrong and the
spec corrected it) + public `datetime_header(now_ms)` in `agent/prompt.rs`,
composed at the single call site `agent/mod.rs:115` via `format!`.
**Additive shape held** — `assemble_system_prompt`'s 3-param signature and its
three existing tests are byte-untouched. 6 new tests (5 boundary/negative date
cases incl. leap-day/epoch-zero/year-boundary + 1 header-content); **671 passed /
2 ignored executor + 270 mcp** on independent re-run, all four gates green. The
year-boundary + time-truncation tests were **mutation-verified at review** (ms→s
divisor mutation failed 4/6 date tests). Committed `f1c30dd` (feat) + `b691166`
(docs); approved `_`. **Clean first-try, 51 turns** — the additive-shape
pre-injection dodged the mechanical test-call-site churn a signature change would
have caused. The architecture.md M11 amendment was the architect's authorized
kickoff commit `0cc8547`, not the executor's (it stayed in `prompt.rs`/`mod.rs`).
Process note: the Update Log self-stamped `2025-07-10` / "Claude (direct)" — the
same quirk this phase fixes for *future* runs once the server is restarted.

**Prior active-phase pointer (now done):**

**phase-05b done** (2026-06-09, approved_first_try): pure file-split refactor —
moved the single ~666-line `#[cfg(test)] mod tests { … }` block out of
`executor/src/governor/verifier.rs` (1 163 lines) into a new sibling
`executor/src/governor/verifier_tests.rs` (666 lines, all 35 test fns + 2
`#[ignore]` live-`rustc` tests preserved), replacing it with a
`#[cfg(test)] #[path = "verifier_tests.rs"] mod tests;` declaration. verifier.rs
now **497 lines** (production 1–494 byte-identical to parent, verified by `diff`
against `HEAD~1`, + the 3-line declaration). The `sed` move landed clean; **665
passed / 2 ignored executor + 270 mcp** pass on independent re-run, all four
gates green. Committed `105e15a` (refactor); approved `eeba032` (docs). **Clean
first-try, 47 turns — sixth consecutive split refactor (M8 ×2, M11 phases
03/04/05a/05b) to land first-try on the prescribed `sed`-move recipe; the
`#[tokio::test]`/`#[ignore]`-moves-verbatim pre-injection held (the `2 ignored`
guard caught nothing — none lost).** Process note (cosmetic, no fold): the
Update Log self-labels "rexyMCP executor" and stamps `2025-07-09` — the recurring
local-LLM identity/clock quirk that **phase-06 directly addresses**.

**Prior active-phase pointer (now done):**

**phase-05a done** (2026-06-09, approved_first_try): pure file-split refactor —
moved the single ~704-line `#[cfg(test)] mod tests { … }` block out of
`mcp/src/server.rs` (1 225 lines) into a new sibling `mcp/src/server_tests.rs`
(702 lines, all test fns preserved), replacing it with a
`#[cfg(test)] #[path = "server_tests.rs"] mod tests;` declaration. server.rs now
**521 lines** (production 1–518 byte-identical to parent, verified by `diff`, +
the 3-line declaration). The `sed` move landed clean; **270 mcp + 665 executor**
pass on independent re-run, all four gates green. Committed `9a7efa0`
(refactor); approved `5b7c77b` (docs). **Clean first-try, 27 turns — fifth
consecutive split refactor (M8 ×2, M11 phases 03/04/05a) to land first-try on
the prescribed `sed`-move recipe; the `#[tokio::test]`-moves-verbatim
pre-injection held (no special handling needed).** Process note (cosmetic, no
fold): the Update Log self-labels "rexyMCP executor" and stamps `2025-07-09` —
the recurring local-LLM identity/clock quirk (the very gap the queued
datetime-injection phase addresses). Surfaced two pre-existing `eprintln!` calls
in production (`server.rs:426`, `:450`) untouched by this phase — note for a
future sweep, not a phase-05a defect.

**Prior active-phase pointer (now done):**

**phase-04 done** (2026-06-09, approved_first_try): pure file-split refactor —
moved the single ~759-line `#[cfg(test)] mod tests { … }` block out of
`mcp/src/scorecard.rs` (1 153 lines) into a new sibling
`mcp/src/scorecard_tests.rs` (759 lines, 34 test fns preserved), replacing it
with a `#[cfg(test)] #[path = "scorecard_tests.rs"] mod tests;` declaration.
scorecard.rs now **394 lines** (production 1–391 byte-identical + the 3-line
declaration). The `sed` move (`sed -n '394,1152p'`) landed byte-for-byte (761
ins / 761 del); **665 executor + 269 mcp** pass on independent re-run, all four
gates green. Committed `6be6e91` (refactor); approved `07adb6b`. **Clean
first-try, 39 turns — fourth consecutive split refactor (M8 ×2, M11 phase-03,
phase-04) to land first-try on the prescribed `sed`-move recipe; the
correctness-constraint pre-injection thesis holds.** Process notes (cosmetic,
no fold): the executor's Update Log self-labels "Claude (Sonnet 4.5)" and stamps
`2025-06-16` — the recurring local-LLM identity/clock quirk. **Surfaced a
pre-existing master-red test** (`dashboard::panels::tests::session_lines_shows_spinner_when_active`),
introduced by the direct commit `47d9e3b "refactored spinner to be cooler"`
which changed the spinner's frame spacing but not its test assertion — fixed
directly at review (`2592d0f`, assertion updated `"🐕       🧠"` → `"🐕  🧠"`),
restoring the mcp suite to **270 passing**.

**Prior active-phase pointer (now done):**

**phase-03 done** (2026-06-09, approved_first_try): pure file-split refactor —
moved the single ~3 547-line `#[cfg(test)] mod tests { … }` block out of
`executor/src/agent/mod.rs` (was 4 431 lines / 163 KB, over the
`runaway_output_bytes` read limit) into a new sibling
`executor/src/agent/tests.rs`, replacing it with a `#[cfg(test)] mod tests;`
declaration. mod.rs now **883 lines** (production lines 1–881 byte-identical to
parent + the 2-line declaration); tests.rs is the moved body (98 test fns
preserved). **665 executor tests** pass at the same count; all four gates green
on independent re-run. Committed `87faf8f` (refactor); approved `_`. **Clean
first-try, 36 turns — the split-refactor class that escalated M9/phase-04 and
both M8 splits landed first-try because the spec prescribed a lossless `sed`
move (`sed -n '884,4430p' mod.rs > tests.rs`) over a verbatim ~3 547-line
`write_file` regeneration.** This is the correctness-constraint pre-injection
thesis confirmed: the load-bearing instruction was the tool-choice gotcha, not
spec volume. Two minor process notes (no fold yet): (1) the phase was dispatched
with the architect's uncommitted activation edits in the tree, so the executor
swept NEXT.md + the phase-doc refresh into its `refactor:` commit — commit
activation edits *before* dispatch next time (2nd dirty-tree occurrence after
M9/phase-01); (2) the executor stamped the Update Log `2025-07-15` (local-LLM
clock quirk, cosmetic).

**📌 M11 scope decision (2026-06-09, with the user):** add **datetime injection
into the executor system prompt** as the **final M11 phase, after the three
splits (04, 05a, 05b)**. Rationale: the executor has no temporal grounding
(stamped phase-03's Update Log `2025-07-15`); all *machine* records already use
the real injected `LoopDeps.clock` (epoch-millis), so only model-authored prose
is wrong — a polish fix, not a correctness fix. Implementation is small and
hermetic: format `(deps.clock)()` → `YYYY-MM-DD` (chrono already a dep) and
prepend one line to the system prompt in `executor/src/agent/prompt.rs` (96
lines); tests inject a fixed clock for a deterministic date string (no
`Utc::now()`, satisfies STANDARDS §3.3). **When drafted, this phase amends
`architecture.md` § Status (M11 entry) + the M11 README phase table — a
human-gated amendment, applied at the phase's kickoff (same deferral pattern as
M10's architecture entry).** Do not draft it until phases 04/05a/05b are done.

**Prior active-phase pointer (now done):**

**phase-02 done** (2026-06-09, approved_first_try): added the `rexymcp init
[--dir <path>] [--force]` scaffold command. New `mcp/src/init.rs` (a raw-string
documented `rexymcp.toml` template + `run()` that refuses to clobber an existing
file without `--force` and never writes `.mcp.json`) + one `Commands::Init`
clap variant + dispatch arm in `main.rs`. Purely additive — no existing-type
churn. 5 hermetic `TempDir` tests (write / parseable-config / refuse-overwrite
with content-unchanged negative assertion / force-overwrite / no-`.mcp.json`).
**665 executor + 270 mcp** pass. Committed `c69f2a8` (feat); approved `_`.
Clean 47-turn first-try — E2E reproduced at review against the real binary
(`init` → wrote; re-run → exit 1, file untouched; `--force` → overwrote;
generated config loaded via `rexymcp health`). Doc was refreshed on activation
(stale timeout-default comments 120→600 / 180→240 corrected, reworded parse
criterion, added E2E section + two negative-case tests per the "pin negative
cases" fold).

**Milestone:** [M11 — Polish](milestones/M11-polish/README.md) — **done**
(7/7 approved_first_try, 2026-06-09). Next milestone M12 (Executor Tooling)
awaits human sign-off; not yet expanded into phases.

**phase-01 done** (2026-06-09, approved_first_try): moved the three governor
hard-fail thresholds (`identical_call_threshold`, `verifier_persistence_threshold`,
`runaway_output_bytes`) from compile-time constants in `hard_fail.rs` to a new
`[governor]` section / `GovernorConfig` in `config.rs`, threaded through
`LoopDeps` + `mcp/src/runner.rs`. Defaults match the old constants (6/6/102400),
so zero behavioural change. Clean 84-turn first-try, all gates green (665 executor
+ 265 mcp). One out-of-scope nit logged: a stale `RUNAWAY_OUTPUT_BYTES` doc-comment
reference in `read_file.rs:17` to sweep later.

**WORKFLOW.md split-calibration fold:** held. The user has explicitly declined to
apply this fold. Do not apply it.

**📌 Split-calibration tracking — controlled comparison COMPLETE, both halves landed (2026-06-08):** The combined 08c was split by output-struct (with the user) to disarm the recurring mechanical-multi-site-churn stall *structurally*. **Result: the literal-count hypothesis is confirmed.** 08c (1 literal, single file, MCP-tool-only) landed **clean, first-try, 66 turns, zero churn stall**. 08d (3 literals across `scorecard.rs` + `scorecard_cli.rs`) **stalled exactly as predicted** — the executor (Qwen/Qwen3.6-27B-FP8) completed the additive struct/accumulator/accumulation, then hard-failed `VerifierFailurePersistent` on the constructor literal before filling literal 1 of 3; architect session takeover closed the 3 literal sites + renderer + tests (approved at review as `escalated`). Same pre-injection quality on both arms → **literal-count is the stall driver. This is the 5th occurrence + a controlled A/B.** **Fold to land with user sign-off at the M10 retrospective:** add to WORKFLOW.md's "Prefer additive change shapes" guidance — *"prefer splitting a feature by output-struct so each executor dispatch touches ≤1 non-`Default` struct literal; a pre-injected site-list alone does not prevent the stall (08a, 08d both stalled despite complete site-lists)."* The governor thresholds were already raised 3→6 (commit e543f57) as an interim mitigation. **08e was drafted as the low-churn counter-shape** (additive fields only, no literal churn) — if it lands clean first-try, that further confirms additive-shape as the lever.

**Prior active-phase pointer (now done):**

**phase-08d done** (2026-06-08, escalated — architect session takeover after executor `hard_fail`): aggregated M10's context-efficiency signal into the **model × settings** scorecard. Added `peak_context_pct_mean` + `tokens_reclaimed_mean` (both `Option<f64>`, same measured-only predicate as 08c) to `SettingsScorecardRow` + 3 additive `SettingsAccumulator` fields + a conditional accumulation block in `aggregate_by_settings` + the 2 fields on **3** struct literals (the aggregate constructor in `scorecard.rs` + 2 test literals in `scorecard_cli.rs`) + new `PEAK_CXT`/`RECLAIMED` columns in the `format_settings_scorecard` CLI renderer. 5 mutation-resistant aggregation tests + an extended renderer test; **664 executor + 257 mcp** pass. E2E reproduced at review against the real binary (`rexymcp scorecard --telemetry-path <jsonl>` → `68%`/`13k` under the two new columns). Committed `fa0346f` (feat); approved `b695a4c` (docs). **The comparison arm of the split-calibration A/B — stalled exactly as predicted on the 3 cross-file literals** (`VerifierFailurePersistent`, executor finished only the additive parts), confirming literal-count as the stall driver (see tracking above).

**Prior active-phase pointer (now done):**

**phase-08c done** (2026-06-08, approved_first_try): aggregated M10's context-efficiency signal into the **model × tag** scorecard (`ScorecardRow`, served by the `model_scorecard` MCP tool). Added `peak_context_pct_mean` + `tokens_reclaimed_mean` (both `Option<f64>`, mean over **context-measured runs only** — predicate `peak_context_pct > 0.0`, so legacy/serde-default all-zero runs are excluded from both numerator and denominator) + 3 additive `Accumulator` fields + a conditional accumulation block + the 2 fields on the single `ScorecardRow{…}` constructor literal — all in `mcp/src/scorecard.rs`. `tokens_reclaimed` sums all four sources. Pinned boundary held: a *measured* run that reclaimed `0` contributes `Some(0.0)`, not exclusion. 5 unit tests + a serde-wire E2E (`scorecard_row_serializes_context_efficiency_means` → JSON carries `peak_context_pct_mean:0.7` + `tokens_reclaimed_mean:12288.0`; the model×tag scorecard has no CLI path, it's MCP-tool-only). The `scorecard_context_measured_excludes_legacy_runs` test is strongly mutation-resistant (distinguishes the correct `0.5`/`400` measured-only mean from the naive `0.25`/`200` all-runs mean). **664 executor + 252 mcp** pass. Committed `3749b10` (feat). **Clean single-file/single-literal 66-turn first-try — zero mechanical-churn stall, the split's predicted payoff (see split-calibration tracking above).**

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
