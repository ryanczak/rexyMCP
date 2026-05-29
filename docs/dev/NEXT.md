# NEXT — Active phase pointer

Single source of truth for which phase the executor works on next. The principal
engineer (architect) maintains this file. The executor reads it first
(AGENTS.md § "First action") and works the phase it points at.

**Active phase:** [M2 / phase-05 — `bash` tool + destructive-command classifier](milestones/M2-executor-tools/phase-05-bash.md)

**Status:** in-progress — bounced from review; one open bug to fix: [bug-05-1](milestones/M2-executor-tools/bugs/bug-05-1.md) (classifier over-blocks benign commands like `cargo test shutdown`).

**Milestone:** [M2 — Executor tools & security](milestones/M2-executor-tools/README.md) — in-progress (phase-01 through phase-04 done).

---

If the linked phase doc's status is `done` or `review`, this pointer is **stale**
— file a blocker and stop rather than guessing the next phase. The architect
updates this file when a phase is approved and the next one is ready to dispatch.
