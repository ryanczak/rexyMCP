---
name: dispatch
description: >
  Dispatch a phase to the local-LLM executor via execute_phase. Use after
  /rexymcp:architect has drafted a phase doc and the user is ready to run it.
model: sonnet
argument-hint: "<phase>"
allowed-tools: Read, Bash(*)
---

# Dispatch Skill

This skill is **thin glue** around the `execute_phase` MCP tool. It does not
review, does not re-dispatch, does not decide escalation levers. It invokes
the executor, surfaces the result, and suggests the next step. Each step is
a user gate.

## Read these first

Before any action:

1. Read `<repo>/docs/dev/NEXT.md` to confirm the active phase pointer. If it
   says "none," there is no active phase — tell the user and stop.
2. Read the phase doc itself (resolve `<phase>` from the argument — it may be
   a short id like `phase-01` or a full path). Confirm its `Status:` line is
   `todo`. If it is `in-progress`, `review`, or `done`, tell the user the
   phase is not dispatchable in its current state and stop.
3. Read `<repo>/rexymcp.toml` to confirm bootstrap is complete (the
   `[executor]` and `[commands]` sections are present). If missing, point the
   user at `/rexymcp:architect` to bootstrap first.

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR` or the
nearest directory containing the milestone layout (`docs/dev/milestones/`).

## 1. Pre-flight: executor reachability

Invoke the `executor_health` MCP tool (no args, or pass `base_url` if the
user supplied an override from `rexymcp.toml`). If the endpoint is
unreachable, surface the error to the user and stop. Do **not** invoke
`execute_phase` against a dead endpoint.

Example healthy response: `{"status": "ok", "model": "qwen-32b", ...}`
Example unhealthy response: connection refused, timeout, or 5xx.

## 2. Invoke execute_phase

Call the `execute_phase` MCP tool with these arguments:

- `phase_doc_path`: the absolute path to the phase doc. Resolve from the
  `<phase>` argument using the milestone convention:
  `docs/dev/milestones/M<n>-<slug>/phase-<nn>-<slug>.md`.
- `repo_path`: the target repo root (from `CLAUDE_PROJECT_DIR` or Claude
  Code's roots).
- `model` (optional): if the user supplied a model override, pass it here.

The executor will run the phase. Claude Code surfaces MCP progress
notifications to the user automatically — you do not need to manage them.
The user will see progress as the executor works.

## 3. On return: complete

When `PhaseResult.status == "complete"`, surface the result summary to the
user:

- **Status:** complete
- **Files changed:** list from `PhaseResult.files_changed`
- **Command outputs:** tail of each command's output (pass/fail + last few
  lines)
- **Log path:** the `log_path` for drill-down via `/rexymcp:review` or the
  `executor_log_tail` / `get_turn` tools

Suggest the next step: "Run `/rexymcp:review <phase>` to verify against the
Definition of Done."

Do **not** review here — that is a separate skill with a separate verdict.

## 4. On return: hard_fail

When `PhaseResult.status == "hard_fail"`, a `briefing` is present. Surface
its fields to the user:

- **One-line summary:** `briefing.one_line`
- **Current blocker:** `briefing.current_blocker`
- **What was tried:** `briefing.what_was_tried` (list)
- **Diagnostics:** `briefing.diagnostics`

Suggest the next step: "Run `/rexymcp:escalate <phase>` to decide whether to
refine and re-dispatch, or take over."

Do **not** decide the escalation lever here — that is the escalate skill's
job.

## 5. On return: budget_exceeded

When `PhaseResult.status == "budget_exceeded"`, the shape is the same as
`hard_fail` — a `briefing` is present. Surface the same fields and suggest
the same next step: `/rexymcp:escalate <phase>`.

## 6. What you do not do

- You do **not** review the executor's work. That is `/rexymcp:review`.
- You do **not** decide escalation levers. That is `/rexymcp:escalate`.
- You do **not** re-dispatch automatically. The user gates each step.
- You do **not** flip phase status. Status management belongs to the
  architect (on dispatch) and the review skill (on approval).
- You do **not** auto-advance to review or escalate. The user advances
  explicitly.
