# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M4 / phase-07e — completion
artifacts](milestones/M4-agent-loop/phase-07e-completion-artifacts.md) — drafted,
`todo`. Awaiting the user to dispatch. (Last 07 sub-phase: unified diff +
`files_changed`, injected command-runner set on clean completion, `log_path` on
`PhaseResult`. After it only phase-08 remains in M4.)

**Routing:** **direct execution (Claude), not opencode.** Same opencode-hostile
content class as 07a–07d — `MockAiClientScript` `<tool_call>`/JSON-in-Rust
fixtures (WORKFLOW § "Route opencode-hostile content"). Pre-route to direct
execution; normal review gate.

**Last completed:** [M4 / phase-07d —
read-before-edit](milestones/M4-agent-loop/phase-07d-read-before-edit.md) —
approved_first_try 2026-05-29 (see phase doc Review verdict).

**Milestone:** [M4 — Headless agent loop + governor/verifier](milestones/M4-agent-loop/README.md) — in-progress (phase-01–06 + 07a–07d done; 07e + 08 remain).

---

Executor: do not pick up work while this says "none." Wait for the architect to
point this file at the next M2 phase doc.
