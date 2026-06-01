# Dogfood log — M6 end-to-end run

Fill this in as you follow the dogfood procedure. One section per procedure
step. Be concrete — quote output, note timestamps, record exact values.

---

## Setup

- **rexyMCP version:** (output of `rexymcp --version`)
- **Claude Code version:** (output of `claude --version`)
- **Plugin install method:** (test mode `--plugin-dir` / persistent `~/.claude/plugins/`)
- **Target repo:** (URL + commit hash)
- **Local LLM:** (base URL, model name, context window size)
- **Date started:**
- **Date completed:**

---

## Bootstrap

- **Detected commands:** (what the bootstrap routine inferred from the project)
- **User overrides:** (anything you had to correct)
- **Files written:** (list each file the bootstrap created)
- **Unexpected prompts or errors:** (quote them)
- **AGENTS.md written?** (yes/no — should be no)

---

## Design

- **Architecture doc quality:** (1–5, one-line rationale)
- **Milestone decomposition quality:** (1–5, one-line rationale)
- **Phase-01 draft quality:** (1–5, one-line rationale)
- **Pre-injection quality:** (1–5 — did the phase doc include worked examples,
  idioms, or reference docs the executor will need?)
- **Notes:** (anything notable about the design process)

---

## Dispatch

- **Health-check result:** (reachable / unreachable, any error message)
- **Progress notifications:** (how often they fired, approximate count)
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

-
