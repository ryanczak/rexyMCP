# Phase 05a: async-polling skill rewrite + `cancelled` handling

**Milestone:** M30 — Executor Interruption
**Status:** todo
**Depends on:** phase-02 (async `execute_phase` + `get_run_status`), phase-03 (`stop_phase` + `cancelled`), phase-04/04b (`rexymcp stop` sentinel)
**Estimated diff:** ~180 lines (Markdown skills only)
**Tags:** language=markdown, kind=refactor, size=m

## Goal

Realign the plugin **skills** with M30's async `execute_phase` contract. Today the
skills assume `execute_phase` returns a `PhaseResult` synchronously; as of M30 it
returns `{ run_id }` and the result is reaped via `get_run_status(run_id)`
(long-poll → running/done/failed/unknown). This phase rewrites the **`dispatch`**
skill to drive that async contract, adds the **`cancelled`** outcome across the
skills, and gives **`auto`** a `STOP(cancelled)` condition. It is **contract-doc /
skill prose** — authored directly by the architect (Claude) and self-reviewed,
like the M27 `/rexymcp:auto` skill phase; it does **not** dispatch to the local
executor.

## Execution note

**Direct execution (Claude-authored).** These are Markdown skills the architect
owns (the collaborative skill/contract-doc domain). No `execute_phase` dispatch.
The `docs/architecture.md` / `WORKFLOW.md` contract-doc updates + the
plugin-template mirror are **phase-05b** (kept separate so each is reviewable).

## Design (decided with the user, 2026-07-10)

1. **`cancelled` → `STOP(cancelled)` in the auto loop.** A cancel is deliberate
   human/architect intent — the loop surfaces it (partial diff + reason) and hands
   back; it **never** silently re-dispatches a stopped phase.
2. **Human-only stop agency.** The loop stays passive: it polls to terminal and
   lets the run's own terminators (max_turns / wall_clock_secs / governor) bound
   it. `rexymcp stop` (human, second terminal) and `stop_phase` (architect, manual,
   between polls) are documented levers, not autonomous loop behavior.
3. **Detect-and-adapt in `dispatch`.** Inspect `execute_phase`'s return: `run_id`
   present → poll `get_run_status`; a `status` field present (old blocking serve or
   the `run-phase` CLI) → use it directly. Survives the stale-serve transition.
4. **Poll until terminal, no skill-level cap.** `get_run_status` long-polls ~15s
   server-side, so the skill just re-calls it until the state leaves `running`;
   `rexymcp status` is the human's visibility.

## Spec

### 1. Rewrite `plugin/skills/dispatch/SKILL.md` § "Invoke execute_phase" + result handling

Replace the synchronous "call execute_phase → PhaseResult" flow with the
detect-and-adapt async flow:

- **§2 (Invoke)**: call `execute_phase`; inspect the return object:
  - **`run_id` present** → enter the poll loop: call `get_run_status(run_id)`
    repeatedly; while `state == "running"` keep calling (each call long-polls ~15s
    server-side — this is efficient, not a spin). On:
    - `state == "done"` → take `result` (the `PhaseResult`) and go to the
      status-branch handling (§3–§5 below).
    - `state == "failed"` → surface the infra `error` string and stop (this is an
      infrastructure failure, not a model `hard_fail`); suggest checking
      `executor_health` / the log.
    - `state == "unknown"` **after** at least one `running`/`done` sighting → the
      run was **lost** (serve likely restarted mid-run, per the stale-serve note);
      surface "run lost — check `rexymcp status`, then re-dispatch," do **not**
      report success.
  - **`status` present instead** (old blocking serve, or `run-phase` CLI) → use the
    object directly as the `PhaseResult` (no polling). One conditional; keeps the
    skill working across the binary transition.
- Add a short **"Stopping a running phase"** note: the human runs `rexymcp stop`
  in a second terminal (global stop-all → the run returns `cancelled`); the
  architect may call `stop_phase(run_id)` manually between polls. A stopped run
  comes back with `status: "cancelled"`.

### 2. Add the `cancelled` branch to `dispatch` (new §, mirroring §3–§5)

When the terminal `PhaseResult.status == "cancelled"`:
- Surface: **status: cancelled**, `cancellation.reason` (`user_stop` / `claude_stop`),
  `cancellation.stage` + `turns_done`, the partial `files_changed` / `diff`, and a
  **"working tree left dirty (unreverted, uncommitted)"** note.
- Suggest the next step: the run was deliberately stopped — decide whether to
  **resume** (`/rexymcp:escalate <phase>` → `continue_phase` from the partial work),
  re-dispatch, or abandon. Do **not** auto-advance.

### 3. Add `STOP(cancelled)` to `plugin/skills/auto/SKILL.md`

- In §2 (the loop) step 4 "Branch on `PhaseResult.status`": add a **`cancelled`**
  arm → **STOP(cancelled)** (go to §5 harvest + loop report; do not review, do not
  escalate, do not re-dispatch).
- In §3 (Stop conditions): add **cancelled** as the fifth condition — "a run
  returned `cancelled` (a human `rexymcp stop` or an architect `stop_phase`
  fired). A deliberate interrupt is human territory: the loop surfaces the partial
  diff + reason and hands back." Update the "four stop conditions" language to
  **five** consistently (the §3 header, the §1/§2 mentions, the "four stop
  conditions" phrase in the intro).
- In §4 (Journaling): the `boundary` activity's `--outcome` gains `cancelled` as a
  valid stop reason.
- In the loop report (§5): the "Stopped" line gains the `cancelled` case
  ("interrupted (<reason>) on <phase> — partial diff attached").

### 4. Coherence one-liners in `review` and `escalate`

- **`plugin/skills/review/SKILL.md` §2** (Refuse non-review results): add
  `cancelled` alongside `hard_fail`/`budget_exceeded` — a `cancelled` result is
  **partial**, not reviewable; point at `/rexymcp:escalate` (resume) or a
  re-dispatch decision, and stop.
- **`plugin/skills/escalate/SKILL.md`**: in the lever guidance, note `cancelled`
  as a natural **resume** candidate — `continue_phase` re-enters the phase from the
  dirty working tree + a briefing of what was interrupted. (Add `cancelled` to the
  §1 status check so escalate accepts it, mirroring `hard_fail`/`budget_exceeded`.)

## Acceptance criteria

- [ ] `dispatch` skill drives the async contract (poll `get_run_status`) **and**
      falls back to a direct `PhaseResult` when `status` is present (detect-and-adapt).
- [ ] `dispatch` handles all four `get_run_status` states (running/done/failed/
      unknown) and all four `PhaseResult` statuses (complete/hard_fail/
      budget_exceeded/**cancelled**).
- [ ] `auto` has a fifth stop condition `STOP(cancelled)`; the "four stop
      conditions" language is updated to five everywhere it appears.
- [ ] `review` refuses a `cancelled` result; `escalate` accepts `cancelled` as a
      resume candidate.
- [ ] All four skill files have valid frontmatter and no broken cross-references
      (`/rexymcp:*` slash-command names unchanged).
- [ ] `cargo build` / `cargo test` still green (no Rust changed — sanity only).

## Test plan

Not applicable in the unit-test sense — these are prose skills. Verification is by
**inspection against the acceptance criteria** + the end-to-end check below. (State
this explicitly per the phase-doc template's E2E-N/A rule.)

## End-to-end verification

- Re-read each edited skill end-to-end and confirm: the async flow is
  internally consistent, the `cancelled` path is coherent across all four skills,
  and no skill still claims `execute_phase` returns a `PhaseResult` directly
  (except the documented back-compat branch).
- Confirm the plugin still loads: frontmatter (`name`/`description`/`allowed-tools`)
  intact on all four skills; slash-command names unchanged.
- `grep -rn "PhaseResult" plugin/skills/dispatch/SKILL.md` — confirm the remaining
  mentions are the async/back-compat handling, not the old synchronous assumption.
- Run `cargo build` + `cargo test` once as a sanity check that no Rust was touched.

## Authorizations

- [ ] May modify the plugin skills (`plugin/skills/{dispatch,auto,review,escalate}/SKILL.md`):
      **Yes** — that is this phase's deliverable.
- [ ] May touch `docs/architecture.md` / `WORKFLOW.md`: **No** — that is phase-05b.
- [ ] May add dependencies: **No** (no code).

## Out of scope

- **`architecture.md` / `WORKFLOW.md` / plugin-template mirror** updates — phase-05b.
- **Autonomous stop heuristics** — the loop stays passive (human-only stop).
- **Run-scoped stop (`--run`)** and **CLI-path reason-stamping** — still deferred.

## Update Log

(Architect-authored; direct execution.)

<!-- entries appended below this line -->
