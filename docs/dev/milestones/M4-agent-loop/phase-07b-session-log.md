# Phase 07b: session-log integration

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** review
**Depends on:** phase-07a (the loop), phase-03 (`store::sessions`: `SessionLogger`,
`SessionRecord`, `SessionEvent`, `open_session_log`, `session_log`), phase-04
(`security::redact::Redactor`). All done.
**Estimated diff:** ~250 lines (clock + log wiring through the loop + redaction +
tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Make the loop **observable**: weave the redacted JSONL **session log** through the
07a turn cycle as a pure side effect. Every step that produces an event — the
assembled system prompt, each raw completion, the parsed `ToolCall` or
`ParseFailure`, each tool result, and the terminal status — is written (redacted)
to `<repo_root>/.rexymcp/sessions/…`, the queryable record the M5 troubleshooting
tools read back on demand. Logging **never changes what the loop returns**
(architecture line 131): it is best-effort and its content does not feed any loop
decision.

This sub-phase wires the event kinds the 07a steps produce: `SessionStart`,
`Prompt`, `Completion`, `Parsed`, `ParseFailed`, `ToolResult`, `SessionEnd`. The
`Verify` / `HardFail` events belong to 07c and `Progress` to M5 — out of scope
(§ Out of scope).

## Architecture references

Read before starting:

- `docs/architecture.md` — "The executor turn cycle" (the closing paragraph:
  "Every step that produces an event … is appended (redacted) to the session log …
  Logging is a side effect of the loop; it never changes what the loop returns").
- `docs/architecture.md` — "Session log & troubleshooting tools": format (one JSON
  object per line, one per turn event), **redaction** ("Every record is passed
  through the lifted redaction layer before it is written"), and **location**
  (`<repo_root>/.rexymcp/sessions/<phase>-<session_id>.jsonl`).
- M4 README § Notes — "Timestamps without `chrono`": `SessionRecord.ts` is a
  caller-set `u64` (epoch millis); the loop **injects** it from a clock, never
  reads a real `Utc::now()` (determinism — STANDARDS §3.3).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. Read these surfaces:
   - `executor/src/store/sessions/jsonl.rs` — `open_session_log(log_dir,
     session_id) -> io::Result<SessionLogHandle>` (an `Arc<Mutex<SessionLogger>>`),
     and **`session_log(handle, ts, turn, event)` — which intentionally discards
     write errors** (best-effort; the docstring says so). Filename is
     `session-{session_id}.jsonl`.
   - `executor/src/store/sessions/event.rs` — `SessionRecord { ts, turn, event }`
     and the `SessionEvent` variants (note the exact field names: `SessionStart {
     session_id, model, phase }`, `Prompt { rendered }`, `Completion { raw }`,
     `Parsed { tool_call }`, `ParseFailed { failure }`, `ToolResult { name,
     succeeded, output_preview }`, `SessionEnd { status, turns }`).
   - `executor/src/security/redact.rs` — `Redactor::new()` and `.redact(&str) ->
     String`; the `[REDACTED:<kind>]` markers and the matching tests (reuse a
     sample secret literal from those tests in your redaction test).
   - `executor/src/agent/mod.rs` — the 07a loop you are threading logging into.

## Current state

The 07a loop (`executor/src/agent/mod.rs`) runs **silent**: it composes the turn
cycle and returns a `PhaseResult` but writes nothing. `LoopDeps` carries `client /
registry / tools / budget / max_turns / project_root`; `PhaseInput` carries the
three prompt strings + `goal` + `acceptance_criteria`. There is **no clock**, no
`model` / `session_id` / phase-slug input, and no `SessionLogger`. `store::sessions`
and `security::redact` are built and tested but have no caller in the loop.

## Spec

All edits are in `executor/src/agent/` (and its tests). Do **not** modify
`store::sessions`, `security::redact`, or the `SessionEvent` schema.

### 1. New loop inputs (clock + identity)

Extend the loop's inputs with what the log needs — grouping is your call (a
`SessionCtx` carrier on `LoopDeps`, or individual fields):

- `model: &str` (or `String`) — for `SessionStart.model`.
- `session_id: &str` — caller-provided (M5 calls `generate_session_id()`; tests
  pass a fixed id, for determinism). The loop does **not** generate it.
- A **phase slug** (e.g. `"phase-07b"`) — add to `PhaseInput` (it already holds
  the phase *doc*; add a short `phase` identifier). Used for `SessionStart.phase`
  and the log filename.
- A **clock**: `clock: &dyn Fn() -> u64` returning epoch millis. The loop calls it
  for each record's `ts`. No real `Utc::now()` anywhere in the loop.

Update the 07a tests to construct these new fields (a fixed `session_id`, a
deterministic clock — e.g. a counter over an `AtomicU64`, or a constant).

### 2. Open the log (best-effort)

At loop start, derive `log_dir = project_root.join(".rexymcp").join("sessions")`
and open the log with a composed id that carries the phase: pass
`format!("{phase}-{session_id}")` to `open_session_log` (so the architecture's
`<phase>-<session_id>` lands in the filename). 

**Opening is best-effort.** If `open_session_log` returns `Err` (e.g. the dir is
not creatable), the loop proceeds **without** logging — hold an
`Option<SessionLogHandle>`, log nothing when `None`, and **do not** return `Err` or
panic. This is the deliberate best-effort contract phase-03 already encodes in
`session_log` (and the architecture's "logging never changes what the loop
returns"); add a one-line comment citing that so the swallow reads as intentional,
not an accidental dropped `Result`.

### 3. Redaction (every record, before it is written)

Every record is redacted before it reaches disk. The cleanest faithful approach,
given the `Redactor` works on `&str`: **round-trip the event through the redactor**
— `serde_json::to_string(&event)` → `redactor.redact(&json)` →
`serde_json::from_str(&redacted)` back to a `SessionEvent`. This redacts **all**
string content uniformly (prompt, completion, tool output, the `ParseFailure` raw +
feedback, the `ToolCall` arguments) and is safe because every `[REDACTED:<kind>]`
marker is bracket/alphanumeric — valid inside a JSON string value, so the parse
round-trips. (Field-wise redaction is acceptable too, but it must cover the nested
`Parsed`/`ParseFailed` payloads, not just the top-level strings.)

Use `Redactor::new()` — the prefix + tagged-value layers. Do **not**
`.with_high_entropy()` (it would mask base64/hashes that legitimately appear in
code). Construct one `Redactor` at loop start and reuse it.

Wrap this in a single helper used at every log site, so no record can bypass it.

### 4. Where each event is logged (map to the 07a steps)

Thread these through the existing loop (turn numbers are the loop's `turns`
counter; `SessionStart` / `Prompt` use turn `0`):

- **`SessionStart { session_id, model, phase }`** — once, immediately after opening
  the log (before the first model call).
- **`Prompt { rendered }`** — once, the **assembled system prompt** (the constant
  produced by `assemble_system_prompt`). Per-turn message growth is reconstructable
  from the `Completion` / `ToolResult` sequence; do not re-log it each turn.
- Per turn, after draining the model: **`Completion { raw }`** with the turn's
  accumulated completion text (for a native-only turn the text may be empty — log
  it anyway).
- When the output becomes a `ToolCall` (native **or** parsed): **`Parsed {
  tool_call }`** (the `parser::ToolCall`, including its `origin`).
- On a parse failure: **`ParseFailed { failure }`** (the `ParseFailure`).
- After a dispatch: **`ToolResult { name, succeeded, output_preview }`** — the
  `output_preview` is the dispatch `content` (truncate to a sane preview length,
  your call — e.g. a few hundred chars; the full output is not the log's job).
- **`SessionEnd { status, turns }`** — on **every** `Ok`-returning terminal path:
  `status = "complete"` for completion, `"budget_exceeded"` for the turn-cap and
  the post-compaction-overflow paths (including the overflow-before-any-chat path).
  On the infra-`Err` path (`AiEvent::Error` / `chat` `Err`), a `SessionEnd {
  status: "error" }` before returning `Err` is **optional** (the partial log
  already shows the failure) — your call; if you log it, do so best-effort.

Logging must not change control flow: compute the `PhaseResult` exactly as 07a
does, log `SessionEnd`, then return it. The returned `PhaseResult` is **byte-for-
byte what 07a returned** — this phase adds no field to it (log-path surfacing is
07d).

### 5. Error model

- All log writes go through `session_log` (or the convenience that wraps the
  handle), whose errors are **intentionally discarded** — logging is best-effort.
- No `?` on a log call may abort the loop. No `.unwrap()` / `.expect()` on a log
  result.
- The loop's own infra errors (backend) are unchanged from 07a (`Error::Backend`).

## Acceptance criteria

- [ ] A run writes a JSONL file under `<project_root>/.rexymcp/sessions/` whose
      name contains **both** the phase slug and the `session_id`.
- [ ] The first record is `SessionStart` (with the `session_id` / `model` / phase),
      followed by a `Prompt` carrying the assembled system prompt.
- [ ] A dispatched turn logs `Completion`, then `Parsed`, then `ToolResult`
      (`succeeded` matching the dispatch outcome). A parse-failure turn logs
      `Completion` then `ParseFailed`.
- [ ] The last record is `SessionEnd` with `status == "complete"` (clean) or
      `"budget_exceeded"` (turn-cap / overflow), and `turns` matching the run.
- [ ] **Redaction:** a secret planted in a tool's output (and one in a completion)
      does **not** appear in the on-disk JSONL; a `[REDACTED:` marker does.
- [ ] **Best-effort:** when the log dir cannot be created, the loop still returns
      the same `PhaseResult` (right `status`) and does **not** error or panic.
- [ ] Record `ts` comes from the injected clock (deterministic in tests); no real
      `Utc::now()` in the loop.
- [ ] No new dependency; no `tracing`; no `Verify` / `HardFail` / `Progress`
      events; no change to `PhaseResult`'s fields; `store::sessions` and
      `security::redact` unmodified.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic (`tempfile::TempDir` as `project_root`; read the log back with
`store::sessions::read_session_log`, reconstructing the path from the known dir +
composed id), deterministic (`MockAiClientScript`, a fixed `session_id`, an
injected counter/constant clock). Pin the redaction **negative**.

- `creates_log_file_named_with_phase_and_session_id`.
- `logs_session_start_first_then_prompt`.
- `logs_completion_parsed_and_tool_result_for_dispatched_turn`.
- `logs_parse_failed_for_malformed_turn`.
- `logs_session_end_complete_on_clean_finish`.
- `logs_session_end_budget_exceeded_on_turn_cap`.
- `redacts_secret_in_tool_output_before_writing` (**negative** — plant a secret
  matching a `redact.rs` pattern in a file, read it via a tool call; assert the
  secret literal is absent from the file bytes and `[REDACTED:` is present).
- `redacts_secret_in_completion_before_writing` (**negative**).
- `logging_failure_does_not_change_result` — pre-create `project_root/.rexymcp` as
  a *file* so `create_dir_all` fails; assert the run still returns `Complete` and
  does not panic.
- `injected_clock_sets_record_ts` — a constant clock → every record's `ts` equals
  the constant.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. The session log is
> written by `execute_phase`, exercised here via `MockAiClient*` + `TempDir` and
> read back with `read_session_log`. The M5 query tools and a live run are the
> first real end-to-end.

## Authorizations

- [x] **May modify** `executor/src/agent/**` (the loop + its tests, including the
      07a tests' construction of the new inputs).
- [ ] **No new dependencies**; no `tracing`.
- [ ] May **NOT** modify `executor/src/store/**`, `executor/src/security/**`, the
      `SessionEvent` schema, `executor/src/phase/**` (no `PhaseResult` field
      change — 07d surfaces the log path), `Cargo.toml`, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, or another phase doc.

## Out of scope

- **`Verify` / `HardFail` events** — 07c logs them when it adds the verifier and
  hard-fail detector. (Wire only the seven event kinds 07a's steps produce.)
- **`Progress` events** + MCP `notifications/progress` — M5.
- **Surfacing the log path in `PhaseResult`** — 07d (with `diff` / `files_changed`
  / `command_outputs`). The caller already knows the dir + `session_id`.
- **Log-query tools** (`executor_log_search` / `_tail` / `get_turn`) — Layer 2, M5.
- **`PhaseRun` telemetry** — phase-08.
- The `scorer.record` consumer (07a calibration note) — 07c.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-29 (started)

**Executor:** Claude Code (direct) — pre-routed off opencode per NEXT.md
(`<tool_call>` fixtures + JSON-in-Rust + planted secret literals).

Adding `model` / `session_id` / `clock` to `LoopDeps` and a `phase` slug to
`PhaseInput`, opening the log best-effort under
`<root>/.rexymcp/sessions/session-<phase>-<id>.jsonl`, and threading
`SessionStart`/`Prompt`/`Completion`/`Parsed`/`ParseFailed`/`ToolResult`/
`SessionEnd` through the loop via a `log_event` helper that round-trips each event
through `Redactor::new()`. Redaction test uses the OpenAI-key sample
(`sk-proj-…` → `[REDACTED:openai_key]`). Existing 07a tests adapt via the shared
`input()` / `deps()` helpers (a `'static` zero clock keeps call sites unchanged).

### Update — 2026-05-29 (complete)

**Summary:** Threaded the redacted JSONL session log through the 07a loop.
`PhaseInput` gained a `phase` slug; `LoopDeps` gained `model` / `session_id` /
`clock: &dyn Fn() -> u64` (injected — no `Utc::now`). At loop start the log is
opened best-effort under `<root>/.rexymcp/sessions/` with a `{phase}-{session_id}`
composed id (`.ok()` drops a setup failure on purpose, with a comment citing the
"logging never changes the return" contract), then `SessionStart` + `Prompt` are
logged. Per turn: `Completion` (after drain), `Parsed` (native or `Found`) or
`ParseFailed`, and `ToolResult` (with a 500-char `output_preview`); every
`Ok`-returning path logs `SessionEnd` with the matching status (`complete` /
`budget_exceeded`), and the infra-`Err` path logs `SessionEnd{status:"error"}`
best-effort. A single `log_event` helper redacts every record by round-tripping
its JSON through `Redactor::new()` (`to_string → redact → from_str`, falling back
to the original on the can't-happen serde error), so no log site can bypass
redaction. The returned `PhaseResult` is unchanged from 07a (log-path surfacing is
07d). No deviations from the spec.

**Acceptance criteria:** all met.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.64s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -1
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.03s

cargo test 2>&1 | grep "test result:" (lib line)
test result: ok. 450 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

(450 = 440 prior + 10 session-log tests. `agent::` alone: 25 passed.)

**Spec-pinned literal grep** (redaction marker + event kinds):

```
grep -c '\[REDACTED:openai_key\]' executor/src/agent/mod.rs → 2 (both redaction negatives assert it)
grep -c 'SessionEvent::' executor/src/agent/mod.rs        → all seven logged kinds + the test event_kind matcher
```

**End-to-end verification:**

Not applicable — phase ships no runtime-loadable artifact. The log is written by
`execute_phase` and read back with `read_session_log` over a `TempDir`; the M5
query tools and a live run are the first real end-to-end.

**Files changed:**
- `executor/src/agent/mod.rs` — `PhaseInput.phase`; `LoopDeps.{model,session_id,
  clock}`; log open + `log_event` / `log_session_end` / `redact_event` /
  `output_preview` helpers; logging threaded through every loop path; 10 new tests
  + updated `input()` / `deps()` helpers + a `log_path` / `event_kind` test helper.

**New tests:** `creates_log_file_named_with_phase_and_session_id`,
`logs_session_start_first_then_prompt`,
`logs_completion_parsed_and_tool_result_for_dispatched_turn`,
`logs_parse_failed_for_malformed_turn`, `logs_session_end_complete_on_clean_finish`,
`logs_session_end_budget_exceeded_on_turn_cap`,
`redacts_secret_in_tool_output_before_writing` (negative),
`redacts_secret_in_completion_before_writing` (negative),
`logging_failure_does_not_change_result`, `injected_clock_sets_record_ts`.

**Commits:** (pending — committed below)

**Notes for review:**
- Redaction is whole-event round-trip (covers nested `Parsed`/`ParseFailed`
  payloads, not just top-level strings), per the spec's recommended approach. The
  `unwrap_or(event)` fallback is a safety net for an infallible serialize, not a
  swallow (STANDARDS §2.1).
- `Prompt` logs the assembled system prompt once at start (not per turn); per-turn
  context is reconstructable from the `Completion` / `ToolResult` sequence, as the
  spec directs.
- The infra-`Err` path logs `SessionEnd{status:"error"}` (the spec made this
  optional; included it so every opened log closes with a terminal record). The
  `chat().await` map_err path returns before any drain and is left unlogged.
- `scorer.record` still has no consumer (07a calibration note) — unchanged here;
  07c wires the reader.

verification: fmt OK · clippy OK · tests 450 passed · build OK
