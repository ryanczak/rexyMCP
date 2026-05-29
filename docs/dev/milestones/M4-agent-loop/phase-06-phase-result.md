# Phase 06: `PhaseResult` + briefing contract

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** todo
**Depends on:** phase-01 (`Diagnostic`/`Severity`), phase-05
(`HardFailSignal`, `ToolCallSnapshot`). All done.
**Estimated diff:** ~450 lines (result + briefing types + renderer + assembly
helpers + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

The executor's **return contract**: the single `PhaseResult` value
`execute_phase` hands back across the MCP boundary, and the **briefing** it
carries when a phase does not complete. This is the *entire* interface Claude
reasons over — the inner loop's tokens, repair history, and raw transcript stay
in the session log; the `PhaseResult` is the lean summary, the briefing is the
escalation hand-off. **Escalation = Claude:** the briefing is *returned*, never
sent to a cloud model (architecture § "Escalation = Claude Code itself").

This phase ships the **types + the briefing renderer + the briefing-assembly
helpers**. The loop that *populates* a `PhaseResult` (runs the turn cycle,
generates the diff, picks the blocker) is phase-07.

## Architecture references

Read before starting:

- `docs/architecture.md` — "The `PhaseResult` / briefing contract" (lines
  ~169–186): the exact `PhaseResult` shape and the "lean summary, log holds the
  rest" principle. And the briefing section list (lines ~164–167): **goal**
  (verbatim), **acceptance criteria**, **current code state** (diagnostics +
  working-set files), **what was tried** (1–2 lines each), **current blocker**
  (exact diagnostic), **budget remaining**.
- Rexy source: `rexy/src/escalation/packet.rs` — the `Packet` struct + six-section
  `render()` + the `summarize_attempts` / `collect_working_files` helpers. **Lift
  the renderer and helpers; drop the cloud transport, the planner/TODO section,
  the `Session` coupling, and the escalation-slot budget directive** (see
  Adaptations). Ignore `escalation/{engine,detector,policy,audit}.rs` entirely.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references and the M4 README.
3. Read this entire phase doc before touching any code.
4. **Read** `rexy/src/escalation/packet.rs` (renderer + helpers only).
5. Read `executor/src/governor/verifier.rs` for the exact `Severity` variants and
   `Diagnostic` fields, and `executor/src/governor/hard_fail.rs` for
   `HardFailSignal` + `ToolCallSnapshot`.
6. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

There is no result/briefing module. `executor/src/lib.rs` declares the existing
modules (`ai`, `config`, `error`, `governor`, `health`, `parser`, `security`,
`store`, `tools`). `Diagnostic`/`Severity` live in
`executor::governor::verifier`; `HardFailSignal`/`ToolCallSnapshot` in
`executor::governor::hard_fail` (both `Serialize`+`Deserialize`).

Rexy's `packet.rs` is coupled to `crate::agent::session::Session`,
`crate::planner::{plan::Plan, todo::Step}`, and an escalation-slot
`BudgetDirective`. **None of those exist in rexyMCP and none are in scope.** The
lift must be decoupled to take explicit structured inputs (the same shape the
phase-07 loop will hold), exactly as phase-05 decoupled the hard-fail detector
from Rexy's `Session`.

## Spec

Create an `executor/src/phase/` module (`pub mod phase;` in `lib.rs`) holding the
`PhaseResult` side and the `Briefing` side. File split within the module is your
call (e.g. `result.rs` + `briefing.rs`, or inline). Re-export the public types so
they are reachable as `executor::phase::{PhaseResult, Briefing, …}`.

### `PhaseResult` — the return type (architecture-pinned)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus { Complete, HardFail, BudgetExceeded }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChange { pub path: PathBuf, pub change_summary: String }

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandOutputs {
    pub format: Option<String>,
    pub build: Option<String>,
    pub lint: Option<String>,
    pub test: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub status: PhaseStatus,
    pub files_changed: Vec<FileChange>,
    pub diff: String,                       // unified diff (capping is phase-07's job)
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    pub briefing: Option<Briefing>,         // Some IFF status != Complete
}
```

`PhaseStatus` is **prescriptive** — `#[serde(rename_all = "snake_case")]` must
serialize the variants to exactly `"complete"`, `"hard_fail"`,
`"budget_exceeded"` (M5 returns this as JSON Claude reads; the strings are
contract). `CommandOutputs` field names serialize to exactly `format` / `build`
/ `lint` / `test`; `FileChange` to `path` / `change_summary`.

**The status↔briefing invariant** (`briefing.is_some() == (status != Complete)`)
is enforced through constructors, not left to the caller:

```rust
impl PhaseResult {
    pub fn complete(files_changed, diff, command_outputs, update_log) -> Self;        // briefing = None
    pub fn hard_fail(briefing, files_changed, diff, command_outputs, update_log) -> Self;
    pub fn budget_exceeded(briefing, files_changed, diff, command_outputs, update_log) -> Self;
}
```

(Parameter grouping is your call — e.g. a small `Artifacts { files_changed, diff,
command_outputs, update_log }` carrier is fine to avoid repeating four args.
Pin only that `complete` yields `briefing: None` and the other two yield
`briefing: Some`.)

### `Briefing` — the escalation hand-off (lift + decouple)

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptSummary { pub one_line: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkingFile { pub path: PathBuf, pub content: String }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Blocker {
    HardFail(crate::governor::hard_fail::HardFailSignal),
    BudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Briefing {
    pub goal: String,                         // verbatim from the phase doc
    pub acceptance_criteria: String,
    pub diagnostics: Vec<crate::governor::verifier::Diagnostic>,  // current code state
    pub working_files: Vec<WorkingFile>,      // post-edit content, capped
    pub what_was_tried: Vec<AttemptSummary>,
    pub current_blocker: Blocker,
    pub budget_remaining: String,             // caller-rendered line, e.g. "2 of 12 turns remaining"
}

pub const MAX_WORKING_FILES: usize = 5;
pub const MAX_ATTEMPT_CHARS: usize = 200;
```

**`Blocker` has exactly two variants** — the two non-`Complete` statuses.
`HardFail(HardFailSignal)` reuses phase-05's signal (and its `describe()`);
`BudgetExceeded` needs no payload (the budget line + diagnostics carry the
detail). Do **not** add Rexy's `Diagnostic(..)` / `FailingToolCall { .. }`
variants — in rexyMCP a hard-fail is always one of the three detector signals,
and tool failures are model-visible `ToolResult`s, not escalation triggers.

**Assembly helpers** (decoupled from `Session` — take explicit inputs the loop
holds):

```rust
pub fn summarize_attempts(recent_tool_calls: &VecDeque<ToolCallSnapshot>) -> Vec<AttemptSummary>;
pub fn collect_working_files(recent_tool_calls: &VecDeque<ToolCallSnapshot>, project_root: &Path) -> Vec<WorkingFile>;
```

- `summarize_attempts` — one `AttemptSummary` per call, `"Tried <tool> <args>;
  succeeded|failed."`, truncated to `MAX_ATTEMPT_CHARS` chars with a trailing `…`
  (`\u{2026}`) when over. Lift Rexy's `compact_args` (path-aware) behavior.
- `collect_working_files` — walk `recent_tool_calls` newest-first; for `patch` /
  `write_file` calls take the `"path"` arg; dedupe; cap at `MAX_WORKING_FILES`;
  read **post-edit** content from disk (resolve relative paths against
  `project_root`); **skip** files that fail to read (don't fail the whole
  briefing). Lift Rexy's logic.

### `Briefing::render(&self) -> String` — lift the renderer, drop TODO

Lift Rexy's six-section markdown renderer, with these changes:
- **Sections (pinned, in order):** `# Goal`, `# Acceptance criteria`, `# Current
  code state`, `# What was tried`, `# Current blocker`, `# Budget remaining`.
- **Current code state** has exactly two subheaders: `## Diagnostics` and
  `## Files in the working set`. **Drop the `## TODO state` subheader entirely**
  (no planner in rexyMCP).
- **Current blocker** renders from the two-variant `Blocker`:
  `HardFail(sig)` → `sig.describe()`; `BudgetExceeded` → a one-line statement
  that the executor exhausted its budget.
- **Budget remaining** renders `self.budget_remaining` verbatim under the header
  (no escalation-slot / suggested-tool-cap directive — that was an instruction to
  a cloud model rexyMCP does not call).
- Keep Rexy's `render_diagnostic` (path:line:col severity [code] message) and the
  empty-section `(none)` / `(no prior attempts)` fallbacks.

**Not redacted.** The briefing is returned to **Claude** (the trusted architect),
not written to disk, so working-file content is **not** passed through the
phase-04 redactor — Claude needs the truth to help (Rexy: "the model gets the
truth"). Redaction applies to the on-disk session log only; the briefing is not a
session-log record. Do not import `security::redact` here.

## Acceptance criteria

- [ ] `executor/src/phase/` exists with `PhaseResult`/`PhaseStatus`/`FileChange`/
      `CommandOutputs` and `Briefing`/`Blocker`/`AttemptSummary`/`WorkingFile`;
      `pub mod phase;` in `lib.rs`; types reachable as `executor::phase::*`.
- [ ] `PhaseStatus` serializes to exactly `"complete"` / `"hard_fail"` /
      `"budget_exceeded"`; `CommandOutputs` to `format`/`build`/`lint`/`test`.
- [ ] `PhaseResult::complete` yields `briefing: None`; `hard_fail` /
      `budget_exceeded` yield `briefing: Some(_)` (the invariant).
- [ ] `Briefing::render` emits the six section headers in order; the code-state
      section has `## Diagnostics` and `## Files in the working set` and **no**
      `## TODO state`.
- [ ] `summarize_attempts` truncates to `≤ MAX_ATTEMPT_CHARS` with a trailing `…`;
      `collect_working_files` caps at `MAX_WORKING_FILES`, dedupes, and skips
      unreadable paths.
- [ ] A `PhaseResult` with a `Some(briefing)` round-trips through JSON.
- [ ] No new dependency; no `tracing`; no import of `security::redact`,
      `Session`, or any planner type.
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic (`tempfile::TempDir` for working-file reads), deterministic, pure
functions. Pin positives **and** negatives (WORKFLOW § "Pin negative cases").

**PhaseResult / status:**
- `status_serializes_to_snake_case` — each variant → the exact pinned string.
- `complete_has_no_briefing` / `hard_fail_has_briefing` /
  `budget_exceeded_has_briefing` — the invariant.
- `phase_result_with_briefing_round_trips_through_json`.
- `command_outputs_serialize_with_pinned_keys` — JSON contains `format`/`build`/
  `lint`/`test`.

**Briefing render:**
- `render_emits_six_section_headers_in_order`.
- `render_code_state_has_diagnostics_and_files_subheaders`.
- `render_omits_todo_state` (**negative** — rendered text does NOT contain
  `"TODO"` / `"## TODO state"`).
- `render_hard_fail_blocker_uses_signal_describe`.
- `render_budget_exceeded_blocker_states_exhaustion`.
- `render_budget_remaining_echoes_the_line`.
- `render_diagnostic_includes_path_line_severity_message`.

**Assembly helpers:**
- `summarize_attempts_marks_succeeded_and_failed`.
- `summarize_attempts_truncates_long_summaries` (ends with `…`, `≤ MAX`).
- `collect_working_files_caps_at_five`.
- `collect_working_files_dedupes_repeated_paths`.
- `collect_working_files_skips_unreadable`.

## End-to-end verification

> Not applicable — phase ships no runtime-loadable artifact. `PhaseResult` and
> `Briefing` are returned by the loop (phase-07) and surfaced over MCP (M5);
> here they are types + a renderer exercised by unit tests.

## Authorizations

- [x] **May create** `executor/src/phase/**`; **may modify** `executor/src/lib.rs`
      (`pub mod phase;`).
- [ ] **No new dependencies**; no `tracing`.
- [ ] May **NOT** implement the loop, generate the diff, lift Rexy's escalation
      engine/detector/policy/audit, the planner/TODO, the `Session` type, or any
      cloud transport.
- [ ] May **NOT** import `security::redact` (the briefing is not redacted).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `verifier.rs`, `hard_fail.rs`, or another phase doc.

## Out of scope

- **The turn-cycle loop** that builds a `PhaseResult` and picks the blocker —
  phase-07. This phase ships types + renderer + helpers.
- **Diff generation / capping**, `files_changed` population, command-output
  capture — phase-07 (this phase only defines the fields).
- **The deferred M3 native-parser items** (`Origin::Native`, `parser/native.rs`,
  `stream.rs`) — the README sketched them "around here," but they are a parser
  concern orthogonal to the briefing and would blow this phase past one focused
  session. They get their own phase; do **not** touch them here.
- **Redaction of the briefing** — not redacted by design (returned to Claude).
- **The "resume" / `continue_phase` lever** — a candidate, not committed
  (architecture § Escalation).

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
