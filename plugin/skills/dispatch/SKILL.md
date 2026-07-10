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
   `todo` or `in-progress`. **`todo` = fresh first dispatch; `in-progress` =
   re-dispatch after a bounce or escalation refinement (the executor reads
   the phase doc + any bug docs + the Update Log's Notes-for-executor block
   and continues).** If it is `review` or `done`, tell the user the phase is
   not dispatchable in its current state and stop.
3. Read `<repo>/rexymcp.toml` to confirm bootstrap is complete (the
   `[executor]` and `[commands]` sections are present). If missing, point the
   user at `/rexymcp:architect` to bootstrap first.

The repo root is `<repo>` — resolve it from `CLAUDE_PROJECT_DIR`, `ANTIGRAVITY_PROJECT_DIR`, or the
nearest directory containing the milestone layout (`docs/dev/milestones/`).

## 1. Pre-flight: executor reachability

Invoke the `executor_health` MCP tool (no args, or pass `base_url` if the
user supplied an override from `rexymcp.toml`). If the endpoint is
unreachable, surface the error to the user and stop. Do **not** invoke
`execute_phase` against a dead endpoint.

Example healthy response: `{"status": "ok", "model": "qwen-32b", ...}`
Example unhealthy response: connection refused, timeout, or 5xx.

## 2. Invoke execute_phase — and reap the result (async)

Call the `execute_phase` MCP tool with these arguments:

- `phase_doc_path`: the absolute path to the phase doc. Resolve from the
  `<phase>` argument using the milestone convention:
  `docs/dev/milestones/M<n>-<slug>/phase-<nn>-<slug>.md`.
- `repo_path`: the target repo root (from `CLAUDE_PROJECT_DIR`, `ANTIGRAVITY_PROJECT_DIR`, or the
  client's workspace roots).
- `model` (optional): if the user supplied a model override, pass it here.

**As of M30, `execute_phase` is an async job** — it spawns the run inside the
serve process and returns immediately, before the phase finishes. **Detect and
adapt** on what it returns:

- **`{ run_id }` present** → the run is in flight; reap it by **polling**
  `get_run_status(run_id)`. Call it, and while the returned `state == "running"`,
  **call it again** — each call bounded-long-polls (~15s) server-side, so this is
  efficient, not a busy spin. Keep polling until the state leaves `running`. There
  is **no** skill-level poll cap: the run is bounded by its own terminators
  (`max_turns`, `wall_clock_secs`, the governor). On the terminal state:
  - `state == "done"` → `result` is the `PhaseResult`; branch on `result.status`
    per §3–§6.
  - `state == "failed"` → an **infrastructure** error (config load / scope / IO),
    **not** a model `hard_fail`. Surface `error` and stop; suggest `executor_health`
    and the log. Do not route to `/rexymcp:escalate` — there is no briefing.
  - `state == "unknown"` **after** you have already seen `running`/`done` for this
    id → the run was **lost** (the serve process likely restarted mid-run — a
    rebuilt binary doesn't hot-swap a running `serve`, and the registry is
    in-memory). Surface "run lost — check `rexymcp status`, then re-dispatch"; do
    **not** report success.
- **`status` present instead of `run_id`** (an older blocking serve binary, or the
  `rexymcp run-phase` CLI path) → the object **is** the `PhaseResult`. Use it
  directly, no polling. This one branch keeps the skill working across the binary
  transition until every serve process runs the M30 async binary.

The user watches progress via `rexymcp status` / `rexymcp dashboard` (Claude Code
sends no MCP progressToken, so live notifications don't fire).

### Stopping a running phase

While a phase is in flight it can be stopped — the M30 interrupt path:

- **Human, second terminal:** `rexymcp stop` writes the `.rexymcp/stop` sentinel;
  the serve-side watcher cancels every live run in that repo (global stop-all).
- **Architect, between polls:** call `stop_phase(run_id)` — you are free to issue
  it precisely because you are polling, not blocked inside one long call.

Either way the run comes back terminal with `status: "cancelled"` (§6): a partial
diff, a `cancellation.reason`, and the working tree left dirty. Stopping is a
**deliberate** act — the skill never fires it on its own.

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

## 6. On return: cancelled

When the terminal `PhaseResult.status == "cancelled"`, the run was **deliberately
stopped** mid-phase (a human `rexymcp stop` or an architect `stop_phase`). Surface:

- **Status:** cancelled
- **Reason:** `cancellation.reason` — `user_stop` (human sentinel) or `claude_stop`
  (architect `stop_phase`)
- **Where:** `cancellation.stage` + `cancellation.turns_done` (turns completed
  before the stop)
- **Partial work:** `files_changed` / `diff` — and note that the **working tree is
  left dirty** (unreverted, uncommitted) for triage.

Suggest the next step: the run was interrupted on purpose, so the user decides —
**resume** the partial work (`/rexymcp:escalate <phase>` → `continue_phase`),
re-dispatch fresh, or abandon. Do **not** auto-advance and do **not** treat a
cancel as a failure to escalate on your own.

## 7. What you do not do

- You do **not** review the executor's work. That is `/rexymcp:review`.
- You do **not** decide escalation levers. That is `/rexymcp:escalate`.
- You do **not** re-dispatch automatically. The user gates each step.
- You do **not** stop a running phase on your own — stopping is a deliberate
  human/architect act (`rexymcp stop` / `stop_phase`), never a skill decision.
- You do **not** flip phase status. Status management belongs to the
  architect (on dispatch) and the review skill (on approval).
- You do **not** auto-advance to review or escalate. The user advances
  explicitly.
