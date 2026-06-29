# M25 — Polish & Config Pass

**Goal:** Land a batch of operator-facing polish and executor-configuration fixes
— grouped by subsystem so each phase is one executor session — without changing
any session-event/telemetry schema or adding a dependency.

**Status:** in-progress

**Depends on:** none (each phase is independent; dispatch in any order, though the
numbering is the suggested order).

## Why now

Six discrete issues accumulated from dogfooding the dashboard and the executor
loop. None is large enough to be its own milestone; batched and grouped by
subsystem they form one coherent polish pass:

| # (issue) | Area | Fix |
|---|---|---|
| 0 | executor tool | `update_task` returns an actionable recovery hint on null/empty/malformed args, not a generic advisory |
| 6 | executor config / AI wire | `enable_thinking` knob (default off, per-model overridable) → `chat_template_kwargs` |
| 1 | dashboard Budget panel | show Executor/Architect savings rows only when > $0.00 |
| 2 | dashboard Budget panel | render Executor/Architect rows as parenthesized debits |
| 3 | dashboard Session panel | remove the `Last update` line |
| 4 | dashboard Activity panel | wrap on word boundaries, not mid-word |
| 5 | dashboard Tasks panel | double the title pan speed |

## Exit criteria

- A null/empty/malformed `update_task` call returns a model-visible advisory that
  names the exact required argument shape **with a concrete example** and lists
  the still-incomplete task ids; the metadata-emitting success path is unchanged.
- `enable_thinking` is a `[executor]` knob defaulting to **false**, overridable
  per-model via `[models."<id>"]`, and a `false` value suppresses reasoning on the
  wire via `chat_template_kwargs.enable_thinking`.
- The Budget panel omits the Executor and Architect savings rows when their cost
  is `$0.00`, and when shown renders them as parenthesized debits (e.g.
  `($0.12)`); Baseline/Net/Assists rows are unchanged.
- The Session panel no longer renders a `Last update` line.
- The Activity panel never splits a word across a wrap boundary that the word
  could fit on the next line.
- The Tasks panel title pan advances twice as fast per tick as before.
- All four gates green; no `SessionEvent`/telemetry schema change; no new
  dependency.

## Architecture references

- `docs/architecture.md` § Status #25 (added at kickoff).
- Issue 0: `executor/src/tools/update_task.rs`; the upstream null→`{}` coercion at
  `executor/src/parser/validate.rs:51-54` (context only — **not** changed).
- Issue 6: `executor/src/config.rs` (`ExecutorConfig`, `ModelOverride`,
  `resolve_for_model`), `executor/src/ai/mod.rs` (`SamplingParams`, `make_client`),
  `executor/src/ai/backends/openai.rs` (`build_chat_body`), `mcp/src/runner.rs`
  (two `SamplingParams` call sites), `mcp/src/init.rs` (template docs).
- Issues 1–3: `mcp/src/dashboard/panels.rs` (`savings_lines`, `session_lines`,
  `last_update_line`).
- Issues 4–5: `mcp/src/dashboard/render.rs` (`wrap_line`),
  `mcp/src/dashboard/panels.rs` (`scrolled_title`).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | `update_task` null-args recovery hint ([phase-01-update-task-arg-hint.md](phase-01-update-task-arg-hint.md)) | done |
| 02 | Default thinking off (`enable_thinking`) | not drafted |
| 03 | Budget & Session panel polish | not drafted |
| 04 | Activity & Tasks panel polish | not drafted |

Phases are drafted **on demand** via `/rexymcp:architect next` — only phase-01 is
drafted now.

## Notes

### Decisions (2026-06-28, with the user)

- **Issue 3 — remove `Last update` entirely** (not refactor). The dashboard
  already surfaces freshness via turn/stage/age elsewhere.
- **Issue 6 — `enable_thinking` is per-model overridable**, mirroring
  `temperature`/`seed`/`max_tokens` in the `[models."<id>"]` table, not just an
  `[executor]` global.
- **Issue 0 — fix lives in the tool, not the parser.** `update_task.execute`
  already receives `{}` (text-parsed path, normalized at `validate.rs`) or
  `Value::Null` (native path) and rejects both; the change is the *quality* of the
  rejection message. The global null→`{}` coercion stays (touching it has
  whole-tool-surface blast radius).
- **Grouping by subsystem.** 0 and 6 are executor-crate (separate concerns →
  separate phases); 1/2/3 are all `panels.rs` (one phase); 4/5 split `render.rs` +
  `panels.rs` (one phase). Each phase is < ~250 lines of diff.

### Working-tree note (2026-06-28)

At kickoff, `executor/src/ai/backends/openai.rs` carried an **uncommitted** change
(a "Begin." user-seed for vLLM/Qwen3 payloads that reject a non-user opening
turn). It is unrelated to M25 but lives in the file phase-02 edits. Commit or
stash it before dispatching phase-02 so the clean-tree pre-flight holds and the
two changes don't entangle.
