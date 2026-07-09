# M28 — Edit-Tool Arg Recovery

**Goal:** When a small model, near max context, emits an edit-tool call with a
truncated/missing required field, the executor returns an **actionable recovery
message** naming the missing field and what it did supply — instead of a raw
serde error the model can't act on — so the loop self-corrects rather than
hard-failing.

**Status:** in-progress (kicked off 2026-07-09)

**Depends on:** none (localized to the tool arg-deserialization seam)

## Why now

Surfaced live during the M27 `/rexymcp:auto` validation run against brainyscript
(and filed as [issue #1](https://github.com/ryanczak/rexyMCP/issues/1)). As the
context window fills, `google/gemma-4-26b-a4b-qat`'s tool-call arguments truncate,
and the required `path` field is dropped first. The edit tools deserialize args
with `serde_json::from_value::<Args>(args)` and, on failure, surface the raw error
verbatim: `invalid arguments: missing field \`path\``. That string gives the model
nothing to recover from, so the failure repeats until a governor stall fires — the
run's session log showed **8×** `missing field \`path\`` before it stopped.

This is the same class of gap M24 closed for the `patch` no-op arm and M22
phase-04 closed for `update_task`: a dead-end tool error replaced with a
**model-visible recovery hint**. `update_task` already has the exact shape to
mirror (`invalid_args_hint()` → `advisory(...)`, `executor/src/tools/
update_task.rs:35`). The fix is deterministic and low-risk — it improves the error
*message*, and deliberately does **not** try to guess the missing `path` value
(auto-reconstructing a write target risks writing to the wrong file).

## Exit criteria

- A `write_file` or `patch` call whose args fail to deserialize returns a
  `ToolResult` error that **names the tool, the required fields (with an example
  shape), and which required fields *were* present** (breadcrumbs), plus a
  next-step — never the bare `invalid arguments: <serde error>` string.
- Valid `write_file` / `patch` calls are byte-for-byte unaffected (happy path
  unchanged; the hint only fires on a deserialization failure).
- Non-object args (e.g. a JSON string/array/null) produce the hint without a
  panic.
- The recovery message does **not** fabricate a `path` value or auto-retry.
- All four gates green; no new dependency; telemetry/schema unchanged.

## Architecture references

- `docs/architecture.md` § Status #4 (the read-before-edit invariant / edit-tool
  contract) and #24 (M24 — Edit-Loop Recovery, the precedent of enriching a
  dead-end tool error into a recovery message).
- [issue #1](https://github.com/ryanczak/rexyMCP/issues/1) — the reported bug.
- `executor/src/tools/update_task.rs:35` — the `invalid_args_hint()` worked
  example this milestone mirrors.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Actionable missing-field recovery hint for `write_file` + `patch` ([phase-01-edit-tool-missing-field-hint.md](phase-01-edit-tool-missing-field-hint.md)) | done |

## Notes

**Scope decision (2026-07-09).** The raw `invalid arguments: {e}` pattern appears
in **10 tools** (`write_file`, `patch`, `patch_lines`, `move_file`,
`delete_file`, `bash`, `search`, `find_files`, `symbols`, `read_file`).
Phase-01 fixes only the two the issue names and reproduced on — `write_file` and
`patch` — the edit tools where a dropped `path` is most damaging (a corrupted
write vs. a failed read). The shared helper is written to be reused, so extending
coverage to the other edit tools (`patch_lines`/`move_file`/`delete_file`) — and
optionally the read/search tools — is a cheap **phase-02** if the pattern proves
out. Not front-loaded to avoid a wide-blast-radius mechanical churn in one phase.

**Deliberately out of scope (issue #1 suggested solutions 1 & 3):**
*auto-reconstructing* the missing `path` from context breadcrumbs (risks writing
to the wrong file) and *context-pressure guards* that bias toward smaller edits
when the budget is low (a loop-behavior change, not a tool-error change). Both are
larger, more speculative, and separable; revisit as their own phases only if the
message-only fix proves insufficient in a follow-up e2e.

<!-- retrospective appended at milestone close -->
