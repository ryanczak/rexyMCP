# Dogfood log — M6 end-to-end run

Fill this in as you follow the dogfood procedure. One section per procedure
step. Be concrete — quote output, note timestamps, record exact values.

---

## Setup

- **rexyMCP version:** (output of `rexymcp --version`) rexymcp 0.1.0
- **Claude Code version:** (output of `claude --version`) 2.1.159 (Claude Code)
- **Plugin install method:** (test mode `--plugin-dir` / persistent `~/.claude/plugins/`) --plugin-dir
- **Target repo:** (URL + commit hash) ~/src/ms_pacman
- **Local LLM:** (base URL, model name, context window size) http://brain:8000/v1, Qwen/Qwen3.6-27B-FP8, 262144
- **Date started:** Mon Jun  1 07:30:30 AM PDT 2026
- **Date completed:** Mon Jun  1 07:30:30 AM PDT 2026

---

## Bootstrap

- **Detected commands:** (what the bootstrap routine inferred from the project) 
/rexymcp:architect

● I'll start by reading the required resources, then check the target repo's bootstrap state. Let me do this in parallel.
● Now let me survey the target repo /home/matt/src/ms_pacman and check the five bootstrap artifacts.
● A nearly empty repo — just package.json. All five artifacts missing, so this is a fresh bootstrap. Let me inspect
  package.json to resolve the command set.
● This is a fresh, essentially empty Node.js project (just a placeholder package.json) named ms_pacman, with no toolchain,
  lockfile, or source. rexymcp is installed at /home/matt/.cargo/bin/rexymcp. Nothing is bootstrapped yet, so I'll run the full
  four-step bootstrap — but bootstrap is interactive wherever the user should decide. Let me gather the decisions I can't
  responsibly default.
● User answered Claude's questions:
  ⎿  · The repo is a bare Node project with no build/lint/test toolchain. Which toolchain should I configure (this resolves the
     {FORMAT_COMMAND}/{BUILD_COMMAND}/{LINT_COMMAND}/{TEST_COMMAND} placeholders everywhere downstream)? → Vite + Vitest +
     ESLint + Prettier
     · Which local-LLM endpoint should the executor dispatch to? (sets [executor] provider + base_url in rexymcp.toml) → (notes
     only)
     · Which executor model? (sets [executor] model and lets me size [budget] context_length sensibly — pick what you actually
     have pulled, or type the exact tag via Other) → (notes only)
     · What's the product scope for ms_pacman? (drives the architecture doc + milestone roadmap I write after bootstrap) →
     Faithful Ms. Pac-Man clone

● Got it. Decisions captured:
  - Toolchain: Vite + Vitest + ESLint + Prettier
  - Executor: OpenAI-compatible endpoint http://brain:8000/v1, model Qwen/Qwen3.6-27B-FP8, context 262144
  - Scope: Faithful Ms. Pac-Man clone

  One field nobody set: [telemetry] dir (the cross-project store). I'll default it to /home/matt/.rexymcp/telemetry and flag it
  for you to correct. Now running the four bootstrap steps.

  Step 1 — write rexymcp.toml:

● Write(rexymcp.toml)
  ⎿  Wrote 19 lines to rexymcp.toml
       1 [executor]
       2 provider = "openai"
       3 model = "Qwen/Qwen3.6-27B-FP8"
       4 base_url = "http://brain:8000/v1"
       5
       6 [commands]
       7 format = "npx prettier --check ."
       8 build = "npx vite build"
       9 lint = "npx eslint ."
      10 test = "npx vitest run"
     … +9 lines (ctrl+o to expand)

● Step 2 — lay down resolved process docs by substituting the four placeholders into the templates:

● Bash(set -e
      mkdir -p /home/matt/src/ms_pacman/docs/dev/milestones…)
  ⎿  WROTE: /home/matt/src/ms_pacman/docs/dev/STANDARDS.md
     WROTE: /home/matt/src/ms_pacman/docs/dev/WORKFLOW.md
     === residual placeholders (should be none) ===
     none

● Step 3 — write CLAUDE.md; Step 4 — register the MCP server in .mcp.json:

● Write(CLAUDE.md)
  ⎿  Wrote 31 lines to CLAUDE.md
       1 # CLAUDE.md
       2
       3 This file orients Claude Code as the **architect** for the ms_pacman
       4 project, working alongside the rexyMCP MCP server and a local-LLM executor.
       5
       6 ## Read these first
       7
       8 1. `docs/dev/STANDARDS.md` — engineering Definition of Done.
       9 2. `docs/dev/WORKFLOW.md` — phase lifecycle, status transitions, Update
      10    Log templates.
     … +21 lines (ctrl+o to expand)

● Write(.mcp.json)
  ⎿  Wrote 8 lines to .mcp.json

● Bootstrap complete. Now explore-then-design. The repo had no source to survey beyond package.json, and you've set the scope
  (faithful Ms. Pac-Man clone). Writing the design doc, the active-milestone README, and NEXT.md.

● Write(docs/architecture.md)
  ⎿  Wrote 144 lines to docs/architecture.md
     … +134 lines (ctrl+o to expand)

● Now the M1 milestone README (following the WORKFLOW template) and NEXT.md. I'm not drafting phase docs — that's
  /rexymcp:architect next, per the gating discipline.

● Write(docs/dev/milestones/M1-foundations/README.md)
  ⎿  Wrote 54 lines to docs/dev/milestones/M1-foundations/README.md
     … +44 lines (ctrl+o to expand)

● Write(docs/dev/NEXT.md)
  ⎿  Wrote 10 lines to docs/dev/NEXT.md
● Let me verify the final state of the scaffold.
● Bash(find . -not -path './.git/*' -not -path './.git' -type f | sort && echo "" && echo "=== .mcp.json valid JSON? ===" &&
      python3 -c "import json;json.load(open('.m…)
  ⎿  ./CLAUDE.md
     … +9 lines (ctrl+o to expand)

● Bootstrap + initial design complete. Here's where ms_pacman stands.
  Bootstrap (4/4 steps)
  ┌───────────────────────┬───────────────────────────────────────────────────────────────────┐
  │       Artifact        │                              Status                               │
  ├───────────────────────┼───────────────────────────────────────────────────────────────────┤
  │ rexymcp.toml          │ ✅ written — commands resolved, executor + budget + telemetry set │
  ├───────────────────────┼───────────────────────────────────────────────────────────────────┤
  │ docs/dev/STANDARDS.md │ ✅ from template, placeholders resolved (no residuals)            │
  ├───────────────────────┼───────────────────────────────────────────────────────────────────┤
  │ docs/dev/WORKFLOW.md  │ ✅ from template, placeholders resolved                           │                               
  ├───────────────────────┼───────────────────────────────────────────────────────────────────┤
  │ CLAUDE.md             │ ✅ written (orients future sessions as architect)                 │
  ├───────────────────────┼───────────────────────────────────────────────────────────────────┤
  │ .mcp.json             │ ✅ rexymcp serve --config ./rexymcp.toml, valid JSON              │
  └───────────────────────┴───────────────────────────────────────────────────────────────────┘
  Per your answers: commands are npx vite build / npx vitest run / npx eslint . / npx prettier --check .; executor is
  Qwen/Qwen3.6-27B-FP8 at http://brain:8000/v1 (provider openai), context 262144. No AGENTS.md / executor-contract file written
  — that contract is embedded in the rexymcp binary by design.

  Design

  - docs/architecture.md — three-layer split (pure Simulation core / fixed-timestep Engine / browser Shell), data flows,
  non-goals, and a 6-milestone roadmap (M1 Foundations → M6 Audio/polish). The load-bearing constraint: game logic is
  framework/DOM/clock/RNG-free so the local executor can unit-test it headlessly.
  - docs/dev/milestones/M1-foundations/README.md — exit criteria + a provisional 4-phase split (scaffold → maze grid → engine
  loop → canvas render). No phase docs drafted yet.
  - docs/dev/NEXT.md — points at M1 phase-01 (to be drafted).

  Two things to flag

  1. [telemetry] dir was defaulted, not chosen by you — I set /home/matt/.rexymcp/telemetry. The skill says not to silently
  default, so: confirm or give me the cross-project store path you want and I'll update rexymcp.toml.
  2. I stopped before drafting phase-01 — that's the gate. Phase authoring is a separate, explicit act.

  Next: run /rexymcp:architect next to draft M1 phase-01 (project scaffold). When I draft it, I'll pre-inject working
  Vite/Vitest/ESLint flat-config snippets and a pre-flight step to verify those config schemas against live docs, since the
  executor has no web access.

------

- **User overrides:** (anything you had to correct)
None. 
- **Files written:** (list each file the bootstrap created)
./.mcp.json
./docs
./docs/dev
./docs/dev/WORKFLOW.md
./docs/dev/milestones
./docs/dev/milestones/M1-foundations
./docs/dev/milestones/M1-foundations/README.md
./docs/dev/milestones/M1-foundations/phase-01-project-scaffold.md
./docs/dev/STANDARDS.md
./docs/dev/NEXT.md
./docs/architecture.md
./rexymcp.toml
./CLAUDE.md
- **Unexpected prompts or errors:** (quote them)
1. [telemetry] dir was defaulted, not chosen by you — I set /home/matt/.rexymcp/telemetry. The skill says not to silently default, so: confirm or give me the cross-project store path you want and I'll update rexymcp.toml.
- **AGENTS.md written?** (yes/no — should be no)
No

---

## Design

- **Architecture doc quality:** (1–5, one-line rationale)
5 High quality document
- **Milestone decomposition quality:** (1–5, one-line rationale)
5 Milestones are well thought out.
- **Phase-01 draft quality:** (1–5, one-line rationale)
5 Very detailed.
- **Pre-injection quality:** (1–5 — did the phase doc include worked examples,
  idioms, or reference docs the executor will need?)
5. 
- **Notes:** (anything notable about the design process)
It looks like things went really well!

---

## Dispatch

- **Health-check result:** (reachable / unreachable, any error message)
- **Progress notifications:** (how often they fired, approximate count)
  Zero live notifications surfaced in Claude Code. Root cause (confirmed
  empirically): Claude Code's MCP client sends **no `progressToken`** in the
  `tools/call` `_meta`, and the MCP spec only lets a server emit
  `notifications/progress` for a request that opted in with one. Verified via
  the `execute_phase: client progress_token absent` stderr line in
  `~/.cache/claude-cli-nodejs/-home-matt-src-ms-pacman/mcp-logs-plugin-rexymcp-rexymcp/*.jsonl`.
  The `McpProgressNotifier` wiring is spec-correct but unreachable with this
  client. Live status is instead obtained out-of-band via `rexymcp status
  --repo <path>` (reads the session JSONL log). See "Surprises and breakages."
- **Turn count:** (how many executor turns)
- **Wall-clock time:** (minutes)
- **Compaction events:** (count, and how early in the phase they fired — e.g.
  "turn 3 of 12", "after ~4k tokens of context")
- **Executor behavior:** (stayed on track / lost state — describe if lost)
- **Phase result:** (complete / hard_fail / budget_exceeded)
- **Notes:** (anything notable about the dispatch)

---

## Review

- **Command rerun result:** (all passed / some failed — which ones?)
- **DoD walk result:** (all boxes ticked / which boxes were missed?)
- **Command-set differences:** (executor's results vs reviewer's — any
  discrepancies?)
- **Verdict:** (approved_first_try / approved_after_N / rejected)
- **Notes:** (anything notable about the review)

---

## Escalate path (optional — leave blank if not exercised)

- **Was escalate exercised?** (yes/no)
- **Briefing structure:** (what type — hard_fail / budget_exceeded / other)
- **Lever chosen:** (refine spec / session takeover / resume)
- **Why that lever:** (the skill's reasoning)
- **Refinement result:** (worked / didn't work / not attempted)
- **Executor behavior on ambiguous spec:** (flagged it correctly / silently
  improvised — if improvised, quote what it did)
- **Notes:** (anything notable about the escalate path)

---

## Telemetry

- **PhaseRun record path:** (file path to `phase_runs.jsonl`)
- **Key fields:**
  - `phase_id:`
  - `status:`
  - `turns:`
  - `wall_clock_secs:`
  - `tool_success_rate:`
  - `parse_failure_rate:`
  - `compaction_report:` (if present — messages_signaturized, messages_evicted,
    tokens_before, tokens_after)
- **Session log path:**
- **Anomalies:** (missing fields, implausible values, etc.)

---

## Compaction observations

This section is a primary objective — the M6 README pinned compaction as a
decision-driving concern (heuristic eviction vs summarization-based compaction).

- **How often did compaction fire?** (per phase, or "not at all")
- **How early in each phase?** (turn number or token count)
- **Did the executor lose anything to compaction?** (re-asking for context,
  repeating tool calls, forgetting a constraint — describe concretely)
- **Would summarization have helped?** (your judgment based on what was evicted
  and whether the executor struggled)
- **Context window vs phase size:** (was the context window adequate for the
  phase's scope, or was it tight from the start?)

---

## Surprises and breakages

Anything that didn't go as expected — errors, confusing prompts, unexpected
behavior, things that worked better than anticipated, things that broke.

- **Critical (fixed): tools not advertised.** rexyMCP did not advertise its
  tools in the MCP `initialize` handshake, so Claude Code never called
  `tools/list` and the tools appeared missing. Fixed by declaring the `tools`
  capability in `get_info` (commit b78a081).
- **Live progress notifications don't surface — Claude Code client limitation,
  not our bug.** The architecture assumed live MCP `notifications/progress`
  would give the human liveness during a run. They never fired because Claude
  Code's MCP client sends no `progressToken` (confirmed via the
  `execute_phase: client progress_token` stderr diagnostic). This is an
  external-API assumption in `docs/architecture.md` (Layer 2 § "Liveness") that
  did not hold against the real client. **Fold for 06b.**
  - Fix shipped: (1) the logged `SessionEvent::Progress` records were wrongly
    gated on the live callback being present, so a no-token run produced zero
    progress records even for post-hoc queries — decoupled so progress is
    always logged independent of any live watcher (commit c4567fb); (2) added
    `rexymcp status --repo <path>` as the out-of-band live-status mechanism,
    reading the per-record-flushed session JSONL (commit 3374336).
  - Takeaway: `rexymcp status` (pull) is the live-status path with Claude Code,
    not MCP progress (push). Don't rework the server to "fix" notifications —
    the gap is client-side.
