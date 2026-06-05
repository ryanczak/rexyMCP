# Phase 04: Split executor/src/agent/mod.rs into focused submodules

**Milestone:** M9 — Executor runtime hardening
**Status:** review
**Depends on:** M9/phase-03 (read_file cap — done)
**Estimated diff:** ~0 net lines (moves ~550 lines out of mod.rs into 6 sibling files)
**Tags:** language=rust, kind=refactor, size=m

## Goal

`executor/src/agent/mod.rs` is 4 507 lines. The private helper functions that
support `execute_phase` (session-log writing, tool dispatch lifecycle, phase-result
construction, metrics accumulation) are grouped by concern but all live in one
file, making it impractical to read with the 500-line cap. This phase extracts
those helpers into 4 new private sibling modules and extends 2 existing ones.
`mod.rs` shrinks by ~550 lines; `execute_phase` and the test suite stay put.

**This is a pure structural refactor. No logic changes. No new dependencies.**
All 585 tests must pass unchanged.

## Architecture references

- `executor/src/agent/mod.rs` — the file being split.
- `executor/src/agent/command.rs`, `progress.rs` — existing modules being extended.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read this entire phase doc before touching any code.
3. Confirm `cargo test` passes with 585 tests.
4. **Do NOT read `executor/src/agent/mod.rs` whole.** Use `start_line`/`end_line`.
   Reading in chunks ≤ 500 lines is required. Key ranges are pre-injected below.

## Current state

`mod.rs` declares these existing submodules (`pub mod`):

```
pub mod command;   // CommandResult, CommandRunner, RealCommandRunner
pub mod contract;
pub mod progress;  // ProgressCallback, ProgressEvent, numstat_from_pre_edit, format_message
pub mod prompt;
pub mod verify;
```

`command.rs` and `progress.rs` are `pub mod` and are referenced by
`mcp/src/runner.rs` and `mcp/src/server.rs`. New modules are `mod` (private).

### Function groups and their line ranges in mod.rs

Read each section with `read_file start_line=X end_line=Y` — do NOT read the
whole file.

| Destination | Functions / items | mod.rs lines |
|---|---|---|
| `log.rs` (new) | `log_event`, `log_session_end`, `redact_event` | 812–857 |
| `tools.rs` (new) | `output_preview`+const, `resolve_path`, `edit_target`, `read_before_edit_refusal`, `record_mtime`, `render_diagnostics`, `dispatch`, `append_tool_exchange`, `assistant_text`, `user_text` | 859–949, 974–1033 |
| `outcome.rs` (new) | `hard_fail_result` | 951–972 |
| `outcome.rs` (new) | `turns_line`, `budget_exceeded_result`, `build_artifacts`, `build_diff`+const | 1035–1132 |
| `progress.rs` (extend) | `EmitCtx`, `emit_progress` | 1135–1176 |
| `command.rs` (extend) | `run_command_set`+const, `run_post_write_hooks`, `run_one`, `tail` | 1180–1248 |
| `metrics.rs` (new) | `RunMetrics`, `emit_phase_run` | 1252–1361 |

Lines 1–65 (header), 66–807 (`execute_phase`), and 1363–4507 (tests) are
**not extracted** — they stay in `mod.rs`.

## Spec

All edits are in `executor/src/agent/`. No other directory.

### Ordering constraint

> **Why this matters:** the executor's verifier hard-fails after **3 consecutive
> turns with build errors**. A naïve order leaves the build red across several
> edit turns and trips that governor. Follow the sequence below exactly — it keeps
> the build green (or only E0255-red for one tight burst) at every checkpoint.

**Phase A0 (Tasks 1–4): create the new orphan files.** Write `log.rs`,
`tools.rs`, `outcome.rs`, `metrics.rs`. They are not yet declared in `mod.rs`, so
they are not compiled and the build stays **green**.

**Phase A1 (new — link the modules BEFORE extending compiled modules).** Add the
four declarations to `mod.rs`'s header, immediately after the existing
`pub mod verify;` line:

```rust
mod log;
mod metrics;
mod outcome;
mod tools;
```

Do **not** add any `use` re-exports yet, and do **not** remove any original
function bodies yet. This compiles the four new modules (their `pub(super)` items
namespaced under `log::`, `tools::`, etc. — no collision with the originals that
still live in `mod.rs`). The build stays **green** (you may see transient
unused-import warnings inside the new modules; those clear in Phase B).

**Critical:** this step must precede Phase A2. `progress.rs` and `command.rs` are
already-compiled `pub mod`s. The moment you extend them with `use super::log::…`
(Task 5) or the `(CommandOutputs, Gates)` return (Task 6), those references need
`mod log;`/`mod metrics;` to already exist. If you extend the compiled modules
first, the build goes red with `unresolved import super::log` and stays red.

**Phase A2 (Tasks 5–6): extend the already-compiled modules.** Append to
`progress.rs` and `command.rs` as specified. Because Phase A1 declared the new
modules, `super::log::log_event` and the `CommandOutputs`/`Gates` types resolve.
The build stays **green**.

**Phase B (Task 7): wire `mod.rs` and delete the originals.** Apply Task 7a's
header replacement as written — it is the final desired header and already
contains the four `mod` lines you added in Phase A1 (re-applying them is
idempotent). The new content 7a adds on top of A1 is the block of `use`
re-exports. Adding a `use` re-export alongside an original
definition produces E0255 ("name defined multiple times"). Immediately remove the
original function bodies to clear every E0255. To minimize consecutive red turns,
**add the re-exports and delete the originals in as few turns as possible** — do
not pause on a red build. The build is green again when all removals are done.

---

### Task 1 — Create `executor/src/agent/log.rs`

Read mod.rs lines 812–857 first, then write the new file.

**All three functions** become `pub(super)`.

Imports the new file needs:

```rust
use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, session_log};
```

No cross-sibling imports. `redact_event` is private (only called by `log_event`
within this file — do not add `pub(super)` to it).

---

### Task 2 — Create `executor/src/agent/tools.rs`

Read mod.rs lines 859–949, then 974–1033. (Skip 951–972, which is
`hard_fail_result` — that goes to `outcome.rs`.)

Move the constant `OUTPUT_PREVIEW_CHARS` here (redeclare it as a private
`const`; it will be removed from mod.rs in Task 7).

**All public-facing functions** become `pub(super)`. `dispatch` is `async
pub(super)`. Internal helpers referenced only within this file can stay private.

Imports the new file needs:

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::ai::next_tool_id;
use crate::ai::types::{Message, ToolCall as AiToolCall, ToolResult as AiToolResult};
use crate::governor::verifier::{Diagnostic, Severity};
use crate::parser::ToolCall;
use crate::tools::ToolRegistry;
```

---

### Task 3 — Create `executor/src/agent/outcome.rs`

Read mod.rs lines 951–972 (`hard_fail_result`) and 1035–1132 (`turns_line`,
`budget_exceeded_result`, `build_artifacts`, `build_diff`).

Move the constant `MAX_DIFF_CHARS` here (redeclare privately; removed from
mod.rs in Task 7). `build_diff` is private (only called by `build_artifacts`
within this file).

**All exported functions** become `pub(super)`.

This module needs `PhaseInput` and `LoopDeps` from its parent — use `super::`:

```rust
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use similar::{ChangeTag, TextDiff};

use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot};
use crate::governor::verifier::Diagnostic;
use crate::phase::{
    Artifacts, Blocker, Briefing, CommandOutputs, FileChange, PhaseResult,
    collect_working_files, summarize_attempts,
};

use super::PhaseInput;
```

---

### Task 4 — Create `executor/src/agent/metrics.rs`

Read mod.rs lines 1252–1361.

`RunMetrics` and `emit_phase_run` both become `pub(super)`. The `impl RunMetrics`
methods (`started_at`, `add_tokens`) can be `pub(super)` or just `pub` on the
impl — match the original visibility.

This module needs `LoopDeps` and `PhaseInput` from its parent:

```rust
use crate::ai::types::TokenBreakdown;
use crate::governor::scorer::Scorer;
use crate::store::telemetry::{self, Gates, GenerationParams, PhaseRun};

use super::{LoopDeps, PhaseInput};
```

---

### Task 5 — Extend `executor/src/agent/progress.rs`

Read mod.rs lines 1135–1176 (`EmitCtx` struct + `emit_progress` fn).

**Append** both to the end of `progress.rs` (before `#[cfg(test)]` if one
exists, or at the very end). Do not restructure the existing file.

`EmitCtx` becomes `pub(super)`. `emit_progress` becomes `pub(super)`.

`emit_progress` calls `log_event` from the sibling `log` module. Once `mod log;`
is added to `mod.rs` in Task 7, this reference resolves. The new import to add
at the top of `progress.rs`:

```rust
use std::path::Path;

use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::SessionLogHandle;

use super::log::log_event;
```

Note: `progress.rs` already imports `HashMap`, `PathBuf`, and `similar`. Check
for duplicates before adding. `EmitCtx` holds a `pre_edit_content:
&'a HashMap<PathBuf, Option<String>>` field — confirm those types are in scope.

---

### Task 6 — Extend `executor/src/agent/command.rs`

Read mod.rs lines 1180–1248.

**Append** `run_command_set`, `run_post_write_hooks`, `run_one`, `tail`, and the
`MAX_COMMAND_TAIL_CHARS` constant to the end of `command.rs`.

`run_command_set` and `run_post_write_hooks` become `pub(super)`. `run_one` and
`tail` are private (only called within `command.rs`). `MAX_COMMAND_TAIL_CHARS`
is a private `const`.

`run_command_set` takes `ctx: &EmitCtx<'_>` — `EmitCtx` is in `progress.rs`.
It also **returns `(CommandOutputs, Gates)`** and constructs both in its body, so
those types must be imported too. New imports to add at the top of `command.rs`:

```rust
use crate::config::CommandConfig;
use crate::phase::CommandOutputs;
use crate::store::telemetry::Gates;

use super::progress::{EmitCtx, emit_progress};
```

Note: `super::progress::EmitCtx` exists after Task 5 extends `progress.rs`. Task
7 will add `mod log;` which unlocks `super::log::log_event` that `emit_progress`
calls — compilation of this chain is deferred until all mod declarations are in
place in Task 7.

---

### Task 7 — Update `executor/src/agent/mod.rs`

This is the final task and the only one that touches `mod.rs`. It has two parts.

#### 7a. Replace the header (lines 1–62)

Use `patch` to replace from the `pub mod command;` block through the
`MAX_COMMAND_TAIL_CHARS` constant with the new header below. The `old_str`
anchor is the block from `pub mod command;` to `const MAX_COMMAND_TAIL_CHARS`.

**New header** (exact replacement — replace lines 9–62 of mod.rs):

```rust
pub mod command;
pub mod contract;
pub mod progress;
pub mod prompt;
pub mod verify;

mod log;
mod metrics;
mod outcome;
mod tools;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use tokio::sync::mpsc;
use tokio::time::interval;

use crate::ai::AiClient;
use crate::ai::types::{AiEvent, Message, ToolSchema};
use crate::config::CommandConfig;
use crate::context::budget::Budget;
use crate::context::compactor::compact;
use crate::error::{Error, Result};
use crate::governor::hard_fail::{HardFailSignal, ToolCallSnapshot, evaluate};
use crate::governor::scorer::Scorer;
use crate::governor::verifier::{Baseline, Diagnostic, Severity, VerifierResult};
use crate::parser::{Origin, ParseResult, ToolCall, parse};
use crate::phase::{Blocker, CommandOutputs, PhaseResult};
use crate::security::redact::Redactor;
use crate::store::sessions::event::SessionEvent;
use crate::store::sessions::jsonl::{SessionLogHandle, open_session_log};
use crate::store::telemetry::{Gates, GenerationParams};
use crate::tools::ToolRegistry;
use command::{CommandResult, CommandRunner, run_command_set, run_post_write_hooks};
use log::{log_event, log_session_end};
use metrics::{RunMetrics, emit_phase_run};
use outcome::{build_artifacts, budget_exceeded_result, hard_fail_result, turns_line};
use progress::{EmitCtx, ProgressCallback, emit_progress};
use tools::{
    append_tool_exchange, assistant_text, dispatch, edit_target, output_preview,
    read_before_edit_refusal, record_mtime, render_diagnostics, resolve_path, user_text,
};
use verify::FileVerifier;

/// Heartbeat period (seconds) for re-emitting `awaiting_model` while the model
/// call is in flight. Keeps `rexymcp status`'s `last_ts` fresh during prefill.
const HEARTBEAT_PERIOD: std::time::Duration = std::time::Duration::from_secs(15);
```

**After this patch the build will show E0255 errors** ("name X defined multiple
times") for every function that now has both a `use` re-export and an original
definition in mod.rs. This is expected and transient. Proceed immediately to
Task 7b to remove the duplicates. Do NOT attempt to fix E0255 by reverting the
use statements — fix it by removing the original function bodies.

#### 7b. Remove the extracted function bodies

Remove each group from mod.rs using `patch`. Work top-to-bottom through the
file. For each group, use the function's signature line as the start of
`old_str` and include through the closing `}`. Exact line ranges (post-7a,
since constants were removed, line numbers may shift by ~3):

1. **Log helpers** (original lines ~812–857): remove `fn log_event`, `fn log_session_end`, `fn redact_event`.
2. **Tool helpers group A** (original lines ~859–949): remove `fn output_preview` through `fn render_diagnostics` (6 functions). Note: `hard_fail_result` at ~951 is in the middle of this region but goes to `outcome.rs` — remove it as part of step 3.
3. **`hard_fail_result`** (original line ~951): remove it.
4. **Tool helpers group B** (original lines ~974–1033): remove `async fn dispatch` through `fn user_text` (4 functions).
5. **Outcome builders** (original lines ~1035–1132): remove `fn turns_line`, `fn budget_exceeded_result`, `fn build_artifacts`, `fn build_diff`.
6. **EmitCtx + emit_progress** (original lines ~1135–1176): remove `struct EmitCtx` and `fn emit_progress`.
7. **Command helpers** (original lines ~1180–1248): remove `async fn run_command_set`, `async fn run_post_write_hooks`, `async fn run_one`, `fn tail`.
8. **Metrics** (original lines ~1252–1361): remove `struct RunMetrics`, `impl RunMetrics`, `fn emit_phase_run`.

After each removal, one set of E0255 errors is resolved. After all 8 removals
the build must be clean: no errors, no new warnings.

## Acceptance criteria

- [ ] `cargo build` passes with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes — all 585 tests, 0 failed, 2 ignored.
- [ ] `executor/src/agent/mod.rs` no longer contains any of the extracted
      functions by name: `grep` for `fn log_event`, `fn output_preview`,
      `fn hard_fail_result`, `fn build_diff`, `fn emit_phase_run`,
      `fn run_command_set` — all must return zero matches.
- [ ] The 4 new files exist: `log.rs`, `tools.rs`, `outcome.rs`, `metrics.rs`.
- [ ] `progress.rs` contains `EmitCtx` and `emit_progress`.
- [ ] `command.rs` contains `run_command_set`.

## Test plan

No new tests — this is a pure structural refactor. The existing 585 tests
cover the behavior end-to-end through `execute_phase`. Passing all 585
unchanged is the acceptance criterion.

## End-to-end verification

Not applicable — phase ships no new runtime-loadable artifact. The change is
purely structural: same behavior, different file locations.

## Authorizations

- [x] **May modify** `executor/src/agent/mod.rs`
- [x] **May create** `executor/src/agent/log.rs`, `tools.rs`, `outcome.rs`, `metrics.rs`
- [x] **May extend** `executor/src/agent/progress.rs`, `command.rs`
- [ ] **No new dependencies.**
- [ ] **May NOT modify any other file.**

## Out of scope

- Moving the constants to their new modules is included (they're tightly coupled
  to the functions that use them). Only `HEARTBEAT_PERIOD` stays in `mod.rs`.
- Reorganizing the test section of `mod.rs`. Tests stay exactly as-is.
- Any logic changes — this is a move-only refactor.
- Further splitting `execute_phase` itself (the 690-line main loop).

## Update Log

(Filled in by the executor.)

<!-- entries appended below this line -->

### Notes for executor — 2026-06-04

The first dispatch hard-failed at turn 26 (verifier flagged build errors on 3
consecutive turns). The diagnosis was a **spec gap, not an executor mistake** —
you faithfully created all four new files and extended `progress.rs`/`command.rs`,
but the old ordering constraint guaranteed a red build:

1. **Ordering bug (now fixed in "Ordering constraint" above).** `progress.rs` and
   `command.rs` are already-compiled `pub mod`s. Extending them with
   `use super::log::log_event` and the `(CommandOutputs, Gates)` return *before*
   declaring `mod log; mod metrics; mod outcome; mod tools;` in `mod.rs` produced
   `unresolved import super::log` and kept the build red. The spec now adds a
   **Phase A1** that declares the four modules immediately after `pub mod verify;`
   and *before* you extend the compiled modules. Follow Phase A0 → A1 → A2 → B in
   order.

2. **Missing imports in `command.rs` (now fixed in Task 6).** `run_command_set`
   returns `(CommandOutputs, Gates)`; the import block omitted both types. Task 6
   now lists `use crate::phase::CommandOutputs;` and
   `use crate::store::telemetry::Gates;`.

**Your partial work from the first run is on disk and is correct — keep it.** The
four new files (`log.rs`, `metrics.rs`, `outcome.rs`, `tools.rs`) and the
`progress.rs`/`command.rs` extensions already exist. Re-dispatch resumes against
this tree. Concretely, what remains is: add the two missing imports to
`command.rs`, run **Phase A1** (declare the four `mod`s in `mod.rs`), confirm the
build is green, then do **Phase B / Task 7** (add the `use` re-exports and delete
the original function bodies). A throwaway verification confirmed that A1 + the
two imports alone takes the build from 5 errors to **zero errors**.

The verifier hard-fails after **3 consecutive red turns** — keep the Phase B
re-export-add + original-deletion tight and don't stall on an intermediate E0255
build.

### Update — 2026-06-04 18:30 (escalation, dispatch-1)

**Chosen lever:** refined re-dispatch
**Rationale:** First-dispatch hard_fail traced to two architect spec gaps (an
impossible Phase-A ordering that broke the build before Task 7, and an incomplete
`command.rs` import list); both are fixed in the spec, so a tightened re-dispatch
against the intact partial work is the cheap, telemetry-preserving fix — not a
takeover.

### Update — 2026-06-04 19:15 (escalation → session takeover, dispatch-2)

**Chosen lever:** session takeover (architect direct)
**Rationale:** Second dispatch (after refined re-dispatch) failed with
`IdenticalToolCallRepetition` on `read_file` — the executor applied Task 7a
correctly (module declarations + `use` re-exports + header replacement) but then
looped reading mod.rs instead of patching out the original function bodies (Task
7b). Same mechanical-churn stall pattern as phase-10b; second consecutive failure
→ session takeover trigger.

**Session takeover — architect implementation:**
- Removed original function bodies (lines 803–1362 post-7a) from mod.rs in one
  Python splice (560 lines removed).
- Fixed 5 unused-import warnings surfaced by the stripped header: dropped
  `LoopDeps` from `outcome.rs`, `GenerationParams` from `metrics.rs`, `Severity`
  / `Blocker` / `CommandResult` from mod.rs outer scope.
- Fixed 14 `cfg(test)` reference failures: `CommandResult`, `Blocker`,
  `MAX_COMMAND_TAIL_CHARS`, `PhaseRun` used in the test module were no longer in
  `super::*` scope after extraction. Added imports inside `mod tests {}`;
  promoted `MAX_COMMAND_TAIL_CHARS` to `pub(super)` in `command.rs`.
- Ran `rustfmt` on 4 touched files to fix line-length formatting.
- **Gates:** `cargo build` ✓, `cargo clippy -D warnings` ✓, `cargo fmt --check`
  ✓, `cargo test` — 585 passed / 0 failed / 2 ignored.

**Executor:** Claude (direct)
**Verdict:** escalated
