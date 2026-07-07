# Codebase review & recommendations — 2026-07-07

A whole-codebase assessment performed post-M25 (no active milestone), covering the
`executor` crate, the `mcp` crate, the plugin package, and the docs/milestone
history. Intended as planning input for M26+. Findings marked **verified** were
checked directly against the code, not just reported by analysis passes.

## Overall state

The codebase is in good shape: 25 milestones closed, no active phase, and the doc
contract has stabilized (M22–M25 retrospectives all concluded "no new folds"). The
recent milestones (M19 gate enforcement, M21 task coverage, M22–M24 recovery work)
structurally eliminated the dominant failure classes (`false_completion`, empty-output
death spirals, truncation stalls, no-op-patch dead ends). The remaining weak spots are
mostly *seams*: config that exists but is never consumed, MCP surface asymmetries, and
governor blind spots that the netviz/brainyscript e2e runs haven't happened to hit yet.

Scale for reference: ~45k LOC of Rust across the workspace; `agent/mod.rs` is 1,290
lines with the entire turn loop in one ~1,150-line function.

---

## 1. Housekeeping (small, concrete, do first)

1. **Unwired config** — **verified**: `[budget] gate_retries`, `[budget] escalation_slots`,
   `[executor] tier`, `[escalation] max_assists`, and `Config::effective_gate_retries` /
   `effective_max_turns` are defined and tested in `executor/src/config.rs` and written
   by `rexymcp calibrate`, but nothing outside `config.rs` / `mcp/src/calibrate.rs` /
   `mcp/src/init.rs` ever reads them (grep-confirmed). The entire tier system and assist
   budget are non-functional; gate retries are bounded only by `max_turns`. Comments say
   "wired in M21" but M21 shipped the task-coverage gate instead. Either wire these into
   the loop or delete them — today `calibrate` writes knobs that silently do nothing.
2. **`REXYMCP.md` is stale** — line ~47 still says "M1–M6 done, active work is M7
   (model scorecard & routing)" and line ~39 describes the `mcp` crate as "currently a
   clap CLI exposing `health`; becomes the rmcp stdio MCP server (M5)". M7 closed
   2026-06-02; the real frontier is post-M25. `NEXT.md` and `architecture.md` are correct.
3. **Roots corroboration is dead code** — **verified**: `roots_list` is hardcoded to an
   empty `Vec` at `mcp/src/server.rs:490`, so `execute_phase` only ever corroborates
   against `CLAUDE_PROJECT_DIR`/`ANTIGRAVITY_PROJECT_DIR`, while the tool description
   claims `roots/list` corroboration. Wire `roots/list` through the rmcp peer or correct
   the description.
4. **`rexymcp run-phase` writes no telemetry** — `telemetry_dir: None` at
   `mcp/src/main.rs:331`, and it skips corroboration. Any phase run via CLI silently
   vanishes from the scorecard, undercutting the "eval dataset as a byproduct of normal
   use" premise. It should telemeter by default, with an explicit `--no-telemetry` opt-out.
5. **Two divergent plugin manifests** — `plugin/plugin.json` (name `rexymcp-plugin`,
   described as "for Google Antigravity") vs `plugin/.claude-plugin/plugin.json` (name
   `rexymcp`). Consolidate, or clearly mark one as the Antigravity variant.
6. **Silent-degradation spots** — each deserves at least a surfaced warning:
   - missing `STANDARDS.md` becomes an empty string via `unwrap_or_default()`
     (`mcp/src/server.rs:98`);
   - a non-writable repo silently disables session logging (`executor/src/agent/mod.rs:190`);
   - `parse_phase_doc` (`mcp/src/runner.rs:28–64`) yields empty goal/criteria/tags on
     heading drift with no warning to the architect.
7. **Resolved item, for the record** — the latent `format_no_match` UTF-8 byte-slice
   panic in `executor/src/parser/feedback.rs` was fixed in M23 phase-03 (char-safe
   truncation + regression test `format_no_match_handles_multibyte_boundary`). Verified
   present; no longer outstanding.

---

## 2. Improvements to current functionality

### Governor blind spots

The stall detectors cover identical repetition, monotone verifier persistence, and
single-call runaway output — but not the shapes in between:

- An A,B,A,B read↔patch **oscillation** never trips `IdenticalToolCallRepetition`
  (requires N *consecutive byte-identical* calls).
- Verifier errors bouncing 2,1,2,1 never trip `VerifierFailurePersistent` (requires a
  non-decreasing sequence).
- `RunawayOutput` measures only the *last* call's post-filter fed-back length, so a
  multi-call flood (or a filtered cargo dump) passes.
- There is **no wall-clock or cost ceiling** at all — only turns + estimated context
  tokens. A fast-looping model can burn unbounded real time within 200 turns.
- The Laplace-smoothed per-tool `Scorer` (`executor/src/governor/scorer.rs`) is recorded
  in telemetry but never feeds back into control flow.

Cheap fix set: a sliding-window distinct-call-set repetition detector, a windowed
cumulative-output check, and a `[budget] wall_clock_secs` knob. These are the same class
of fold M22/M23 established — the difference is adding them *before* the e2e run that
finds them the hard way.

### Verifier practicality

- **`tsc` path resolution**: invoked as a bare PATH binary
  (`executor/src/governor/verifier.rs:431`), so it `Skipped`s in most real Node repos
  where tsc lives in `node_modules/.bin`. Try the local binary first, then `npx tsc`,
  then PATH.
- **Per-edit cost**: `cargo check` runs the whole crate on every successful edit
  (baseline dedup only helps startup) — expensive on large target workspaces.
- **Warnings dropped**: only error-level diagnostics are fed back; clippy problems
  surface only at the final lint gate. Late feedback is expensive feedback for a weak model.
- **Language coverage** stops at Rust/TS/Python; Go (`go vet`) is a cheap addition given
  the architect skill already detects Go projects.
- Making verifier commands **configurable per-language** in `rexymcp.toml` solves the
  tsc path problem and custom-toolchain projects in one move.

### MCP / CLI symmetry

- **Recording a review verdict — the supervision half of the eval loop — is a Bash CLI
  call** (`rexymcp review`), while dispatch is an MCP tool. Add a `record_review` MCP
  tool; also removes the hand-assembled `--phase-doc` path that can silently
  misattribute a review (falls back to `phase_id` folding on mismatch).
- Add a **`phase_status` MCP tool** wrapping the `status` fold. Claude Code sends no
  `progressToken` (so `notifications/progress` never fires); a poll-style status tool is
  the only in-band liveness signal an architect session can get.
- CLI verbs with no MCP twin: `runs`, `status`, `dashboard`, `doctor`, `review`,
  `calibrate`, `init`. Not all need one, but `status` and `review` do.

### Error structure across the MCP boundary

`execute_phase` collapses every `run_phase` failure into `internal_error(e.to_string())`
(`mcp/src/server.rs:526`). Config errors, scope violations, and backend unreachability
all look identical to the architect. Map `executor::error::Error` variants to distinct
MCP error codes/data so the dispatch skill can branch ("backend down → suggest
`executor_health`" vs "config invalid → point at the toml").

### Structural refactor of the loop

`agent::execute_phase` is a single ~1,150-line function with the terminal-return
boilerplate (`log_session_end` + `emit_phase_run` + `build_artifacts`) duplicated across
~7 exit paths — one missed `emit_phase_run` on a new path silently drops telemetry, and
`EmitCtx` is reconstructed ~8 times. Extract a `TurnLoop` struct with a single
`finish(status)` exit; this makes future M19/M21/M22-style gate additions much safer.
Classic M11-style "split" milestone. Also: the A3 stall guard (`agent/mod.rs:661–730`)
recomputes the three gate-feedback helpers twice per turn (peek + act).

### Known no-ops to close out

- **Post-write format hook runs the `--check` (verify-only) form**, so auto-format never
  actually happens (`agent/command.rs::run_post_write_hooks`) — the executor-vs-reviewer
  fmt divergence seen in M21 phase-01 will recur.
- **`read_before_edit` gates `patch` but not `write_file`** (`agent/tools.rs:37,53`) — a
  model can blind-overwrite a file it never read, precisely the failure the gate exists
  to prevent.

### Smaller items

- `bash_timeout_secs` hardcoded to 30 at the `build_registry` call site
  (`mcp/src/runner.rs:197`) despite other knobs being configurable.
- Hardcoded limits worth a config pass someday: circuit breaker (5 failures / 60s
  cooldown), HTTP client timeout (300s), retry count (2), heartbeat (15s),
  `MAX_DIFF_CHARS` (50k), `MAX_COMMAND_TAIL_CHARS` (4k), bash truncation
  (head 20 / tail 80 / threshold 100 lines), compactor `TARGET_FRACTION` (0.75) and
  `RECENT_TURNS_PROTECTED` (3).
- The bash destructive-command blocklist (`security/bash_classify.rs`) is not
  configurable per-project (can't allow `git push`, can't add project-specific bans).
- `append_tool_exchange` serializes args with `unwrap_or_else(|| "{}")`
  (`agent/tools.rs:383`) — an un-serializable arg silently dispatches as `{}`.
- Blocking `std::fs`/`std::process` calls inside the async loop without
  `spawn_blocking` (`agent/mod.rs:1004`, `agent/outcome.rs:99`, `tools/bash.rs:214`).
- No `tracing`/metrics export anywhere; the JSONL session log + `PhaseRun` are the only
  observability, both best-effort.
- Known cosmetic quirk (multiple retrospectives, no fold): the executor stamps
  "Claude (Opus)" in its Update Log identity while telemetry records the real model.

---

## 3. New feature ideas

### 3.1 `continue_phase` (resume) — the missing third escalation lever

Already sketched in `architecture.md` as a candidate; reserved in the escalate skill.
The strongest version is **briefing-seeded resume**, not full transcript rehydration
(which preserves the context rot that the re-dispatch lever exists to escape): start a
fresh context from the phase doc + the briefing + architect guidance + the current diff,
with `task_states` restored from the session log. That gets "don't redo the 90% that's
done" without replaying the rot. The JSONL log plus working-set tracking already
serialize most of the needed state.

### 3.2 Server-authored bookkeeping (D8/D9) — the queued design conversation

Flagged repeatedly in M22/M23 docs as the most concrete pending item: rexyMCP writes the
Status flip and a baseline Update Log entry itself, so a MEDIUM-tier model that wrote
correct code stops dying in the bookkeeping tail (the exact M22 failure mode). It moves
*who authors the bookkeeping* from executor to server and touches the executor contract,
so it needs a talk-through before any phase is drafted — but the executor already knows
everything required (files changed, gates run, tasks done), and it would delete an
entire class of `StuckGateFeedback` / `budget_exceeded` outcomes. **Top pick for the
M26 seed.**

### 3.3 Advisory model routing from the scorecard

The architecture explicitly forbids an *automated* tag→model router, and that's right —
but the data now exists to make dispatch smarter without automating the decision: when
`/rexymcp:dispatch` runs, the skill calls `model_profile` for the phase's tags and
surfaces e.g. "for tag `parser`, qwen-32b is 9/10 gates-pass over 12 runs; gemma-27b is
4/9 — you're dispatching to gemma-27b, consider switching." Human's call, but an
informed one. Skill-layer only — cheap to prototype. This finally makes the scorecard
*actionable in the loop* rather than a report you remember to read.

### 3.4 Real token counting

The chars/4 heuristic (`executor/src/context/tokens.rs`) is materially wrong for code
(typically ~3.2–3.5 chars/token), so the whole compaction machinery triggers off a
skewed estimate. vLLM and llama.cpp both expose a `/tokenize` endpoint; a `Tokenizer`
trait with an HTTP-backed impl (cached, falling back to the heuristic) makes `Budget`
honest, and `peak_context_pct` in the scorecard becomes a real number instead of an
estimate of an estimate.

### 3.5 Parallel / multi tool calls per turn

Only the **first** native tool call per turn is honored (`agent/mod.rs:398`); the rest
are silently dropped. Qwen3 and recent local models emit multi-call turns readily. Even
sequential execution of all calls in one turn (no concurrency needed) cuts turn counts —
and turns are the scarce budget resource for local models with slow prefill.

### 3.6 Bash sandboxing as real confinement

Today bash confinement is cwd-pin + env-strip + an evadable substring classifier
(base64/eval/`$()` indirection); the env allowlist keeps `HOME`, so `~/.ssh` / `~/.aws`
are readable, and there is no egress control. On Linux, wrapping bash in `bwrap`
(bind-mount the repo rw, toolchain ro, no home, optional `--unshare-net`) upgrades scope
confinement from advisory to enforced without touching the tool contract. Config-gated,
`Skipped`-style fallback when bwrap is absent — same Tier-1 fail-open pattern as the
verifier enhancers.

### 3.7 Dispatch queue / phase pipelining

Independent phases (e.g., M25's five dep bumps, which ran 5/5 clean with zero source
edits) could run as a queued batch: `execute_phase_batch` dispatches N phase docs
sequentially server-side, stops at the first non-complete, returns one result set. The
human gate stays at review; you just stop paying architect round-trip latency between
mechanical phases.

### 3.8 Cross-project scorecard

Telemetry is per-`telemetry.dir`, keyed by project id. An `--all-projects` aggregation
(or a shared default dir) turns brainyscript + netviz + future targets into one
competency matrix. Sample size is the scorecard's stated weakness; pooling across
targets is the cheapest way to grow N per model×tag.

### 3.9 Structured phase-outcome diffing for review

The review skill re-runs gates and greps for banned constructs, but the architect still
eyeballs the diff. A `review_pack` MCP tool returning the diff annotated with which
acceptance criterion each hunk plausibly serves (mechanical file/symbol matching against
the phase doc, not model-driven) plus untouched-criteria warnings would speed the
slowest human step in the cycle.

---

## 4. Previously deferred items (for completeness)

Tracked in the milestone docs; listed here so this review is a one-stop planning input:

- **D8/D9 server-authored bookkeeping** — see §3.2.
- **M18 thread 4 cold-start calibration battery** — shelved by design 2026-06-13;
  requires an explicit architecture.md amendment (departs from passive-telemetry-only)
  before any phase is drafted.
- **M24 phase-02** — extend no-op recovery to the `patch` ambiguous/zero-match arms;
  held pending an e2e showing the model stalls there.
- **TruncationStall terminator** — M23 chose recover-first; add only if e2e shows
  length-truncation loops persist.
- **Full LSP client** (M12 Arc B) — build only if failure-class tagging shows
  symbol-resolution is a dominant class.
- **`project_review` tool** — designed in architecture.md (~lines 339–345); confirm
  whether it fully shipped vs. remains partly aspirational.

---

## 5. Suggested ordering

1. **M26 "polish & hardening"** in the established mold: the §1 housekeeping list plus
   the governor/verifier items from §2 (oscillation detector, wall-clock budget, tsc
   resolution, `write_file` read-gate, post-write hook fix).
2. **D8/D9 server-authored bookkeeping** (§3.2) — own milestone, after a design
   talk-through (touches the executor contract).
3. **Resume lever** (§3.1) — own milestone, after a design talk-through.
4. **Routing advisory** (§3.3) — skill-layer only; prototype opportunistically.
5. Everything else as e2e evidence or appetite dictates.
