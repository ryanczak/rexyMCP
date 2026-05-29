# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-07a — turn-loop
core](milestones/M4-agent-loop/phase-07a-loop-core.md) — drafted, `todo`.
Awaiting the user to dispatch. (Phase-07 was split a–d; this is the first
sub-phase. The dropped "06b" native-event→`ToolCall` adapter is absorbed into
07a's native-call seam — see M4 README § Notes.)

**Routing:** **direct execution (Claude), not opencode.** The loop's tests script
model output containing `<tool_call>`/`<think>` close-tags and escaped quotes
inside JSON-in-Rust string literals — the same opencode-hostile content class that
forced direct execution for all five M3 parser phases (WORKFLOW § "Route
opencode-hostile content"). Pre-route to direct execution; normal review gate.

**Last completed:** [M4 / phase-06 — `PhaseResult` + briefing
contract](milestones/M4-agent-loop/phase-06-phase-result.md) — approved_first_try
2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–06 done; 07a–07d + 08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
