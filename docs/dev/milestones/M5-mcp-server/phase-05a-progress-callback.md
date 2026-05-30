# Phase 05a: progress callback seam + Progress log events (executor side)

**Milestone:** M5 — MCP server
**Status:** todo
**Depends on:** M4 (`execute_phase` loop, `pre_edit_content`/`build_diff` machinery, `SessionEvent::Progress` + `FileNumstat` reserved in M4 phase-03), `similar` crate (already in workspace).
**Estimated diff:** ~500 lines (progress module + 4 emission sites + tests + LoopDeps wiring)
**Tags:** language=rust, kind=feature, size=m

## Goal

Add the **progress-callback seam** the M4 loop has been reserving since phase-03:
an injected `&dyn ProgressCallback` on `LoopDeps` that the loop calls at four
emission points (turn start, tool dispatch, verifier, final command), each
carrying the current files-changed numstat. Each emission also writes a
**`SessionEvent::Progress`** record to the JSONL session log — so the durable
half of the architecture's "consumer split" (human watches live notifications;
Claude queries logged progress post-return) is already complete after this phase.

Phase-05b (drafted after this lands) will wire the *consumer* side: the mcp
server's `execute_phase` handler builds a callback that emits MCP
`notifications/progress` and passes it through `run_phase` → `LoopDeps`.

This split mirrors M4 phase-07's cohesive-seams discipline: 05a is purely
executor-side, hermetic-testable with a `Mutex<Vec<ProgressEvent>>` mock; 05b is
the mcp consumer that depends only on the callback contract this phase pins.

## Architecture references

- `docs/architecture.md` — "Liveness" (`MCP progress notifications` as the loop
  advances); Status §M4 ("`PhaseResult.log_path`"); Status §M5 (progress
  notifications + `roots/list`).
- M4 README — "Progress heartbeats (design decision — implemented in M5,
  schema reserved in M4 phase-03)". The full consumer split is in there:
  human watches live MCP notifications (mid-call abort decision point); Claude
  queries the logged `Progress` events post-return via
  `executor_log_search`. The heartbeat is **a liveness summary, never a second
  source of truth.**
- M4 phase-03: `SessionEvent::Progress { turn, stage, files_changed:
  Vec<FileNumstat>, message }` + `FileNumstat { path, added, removed }`
  reserved for this phase.
- M4 phase-07e: `build_diff(pre_edit_content, project_root)` — the existing
  diff machinery, the basis for the numstat helper here (a lighter-weight
  numstat-only variant; see § 2).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README Notes.
2. Read this entire phase doc, **especially M4 README § "Progress heartbeats"**
   (it pins the consumer split; do not invent a parallel design).
3. Confirm `SessionEvent::Progress` + `FileNumstat` exist as documented in
   `executor/src/store/sessions/event.rs`. They do (M4 phase-03).
4. Confirm `pre_edit_content: HashMap<PathBuf, Option<String>>` is the loop's
   working-set state (`executor/src/agent/mod.rs`); `build_diff` reads from it
   to produce `PhaseResult.diff` + `files_changed`. The numstat helper added
   here is the lighter cousin.
5. Confirm `similar = "2"` is already a workspace dep. It is.

## Spec

### 1. New module — `executor/src/agent/progress.rs`

```rust
use crate::store::sessions::event::FileNumstat;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The callback the loop invokes at each emission point. `Send + Sync` so it
/// can cross await points and be shared across the rmcp request task (05b).
pub trait ProgressCallback: Send + Sync {
    fn on_progress(&self, event: &ProgressEvent);
}

/// Blanket impl over closures so callers can pass a `|e| { ... }` directly.
impl<F: Fn(&ProgressEvent) + Send + Sync> ProgressCallback for F {
    fn on_progress(&self, event: &ProgressEvent) { self(event) }
}

/// One liveness event. Mirrors the payload of `SessionEvent::Progress` so the
/// loop converts directly when logging. Kept as a separate type so the
/// callback contract is independent of the log schema's evolution.
#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub turn: usize,
    /// Short stage tag: `"turn_start"`, `"tool:<name>"`, `"verify"`,
    /// `"command:<name>"`. See § 3 for the canonical set.
    pub stage: String,
    /// Per-file +/- counts from `pre_edit_content` vs. on-disk content.
    pub files_changed: Vec<FileNumstat>,
    /// One-line human-readable summary (architecture's encoded-as-message
    /// requirement). Format pinned in § 4.
    pub message: String,
}

/// Compute the per-file numstat from the loop's working-set. Reads each file
/// from disk and compares against its pre-edit content. Best-effort: a file
/// that's vanished or now unreadable contributes `(0, 0)` rather than
/// erroring — the heartbeat is never a second source of truth.
pub fn numstat_from_pre_edit(
    pre_edit_content: &HashMap<PathBuf, Option<String>>,
    project_root: &Path,
) -> Vec<FileNumstat>;
```

**`numstat_from_pre_edit` algorithm:**

For each `(path, before_opt)` in `pre_edit_content`, sorted by path for
determinism:
1. Read current `after` from disk via `std::fs::read_to_string(path)`. On
   failure, treat as empty string. (Deleted files have zero `after` lines and
   non-zero `removed`.)
2. Use `similar::TextDiff::from_lines(before_opt.as_deref().unwrap_or(""),
   &after)` and count `iter_all_changes()` filtering by `ChangeTag::Insert`
   (→ `added`) and `ChangeTag::Delete` (→ `removed`).
3. Skip entries where both `added == 0 && removed == 0` (file unchanged from
   pre-edit baseline — no signal worth emitting).
4. Path string is the file path **relative to `project_root`** when possible
   (use `path.strip_prefix(project_root).unwrap_or(path).to_string_lossy()`).
   Falls back to the absolute path when the file is somehow outside the
   project root.

Return `Vec<FileNumstat>` (possibly empty — emitted in early turns before any
edit lands).

### 2. LoopDeps wiring — `executor/src/agent/mod.rs`

Add one field to `LoopDeps`:

```rust
pub struct LoopDeps<'a> {
    // …existing fields…
    /// Optional liveness callback. `None` disables progress entirely (no
    /// callback invocations, no `Progress` log events, no numstat
    /// computation). Best-effort when `Some`: a callback that panics is
    /// outside this contract; the loop assumes the callback is safe.
    pub progress: Option<&'a dyn ProgressCallback>,
}
```

This is the **only** new `LoopDeps` field. All existing tests in
`executor/src/agent/mod.rs` (and the 1–2 in `mcp/src/runner.rs` /
`mcp/src/server.rs`) must add `progress: None` to their constructions. That is
the *one* authorized cross-cutting test edit (no logic change in any test).

### 3. Emission points — `executor/src/agent/mod.rs`

The loop emits at exactly **four** sites. Compute the numstat *once per
emission* (cost is proportional to the number of tracked files, microseconds
on a typical phase). Skip everything when `deps.progress.is_none()`:

1. **`turn_start`** — at the top of each turn, after `turns` is incremented.
   Stage: `"turn_start"`. Message: see § 4.
2. **`tool:<name>`** — just before invoking each tool from the registry (the
   site where the loop calls into `tool.run(...)`). Stage: `format!("tool:{}",
   tool_name)`. Emit *once per tool call*, not once per turn (a turn can have
   multiple tool calls).
3. **`verify`** — just before invoking `deps.verifier.verify(...)` after an
   edit-class tool. Stage: `"verify"`.
4. **`command:<name>`** — on the clean-completion path, just before each of
   the four final commands (`fmt`/`build`/`lint`/`test`) runs. Stage:
   `format!("command:{}", which)`. Skip when the command is unconfigured.

For each emission:
- Compute `let numstat = numstat_from_pre_edit(&pre_edit_content,
  deps.project_root);`
- Build `ProgressEvent { turn, stage, files_changed: numstat.clone(), message
  }`.
- `deps.progress.unwrap().on_progress(&event);`
- **Also** `log_event(&log_handle, &redactor, deps.clock, turn,
  SessionEvent::Progress { turn, stage: event.stage, files_changed: numstat,
  message: event.message });` — the durable half. Same best-effort posture as
  every other `log_event` call (no error propagation).

Both side effects are independent: a callback that panics shouldn't prevent
the log write, and a log-write failure shouldn't prevent the callback. The
order is `callback first, log second` — the callback is the more
time-sensitive (user-facing liveness), the log is the durable record.

> Editor's caution: in long turns the loop also calls
> `agent::execute_phase`-internal helpers that are NOT progress-emission
> points: chat completion, parser output, governor scoring, hard-fail check,
> read-before-edit gate, budget compaction. These are decisions/transitions
> the loop already logs as their own `SessionEvent` kinds; they do **not**
> warrant separate `Progress` events. Keep emission to the four sites above.

### 4. Message format

Architecture mandates the numstat is "encoded in the notification's `message`
string." Use a single-line format Claude (and any UI built on the log) can
parse cheaply:

```
turn=<n> stage=<stage> +<TOTAL_ADD>/-<TOTAL_DEL> files=<N>[ <path>:+<a>/-<r>]*
```

Examples:

- Early turn, no edits yet: `turn=1 stage=turn_start +0/-0 files=0`
- Mid-phase: `turn=4 stage=tool:patch +18/-3 files=2 src/lib.rs:+12/-2 src/util.rs:+6/-1`
- Final command: `turn=7 stage=command:test +18/-3 files=2 src/lib.rs:+12/-2 src/util.rs:+6/-1`

**Top-N truncation** in the message (not the structured `files_changed` field):
include at most **5** per-file segments in the message string to keep it
human-skimmable; if the total file count exceeds 5, append ` …+<K>`. The
structured `files_changed: Vec<FileNumstat>` carries the full list — the
message is the human-summary view.

Format helper lives in `progress.rs`:

```rust
pub fn format_message(
    turn: usize,
    stage: &str,
    files_changed: &[FileNumstat],
) -> String;
```

Total adds/removes are the sum across all `files_changed` entries (not just
the top 5).

## Adaptations / decisions

1. **Trait + closure blanket impl** (not just a bare `Fn(&_)`). Lets 05b pass
   a struct that holds the rmcp peer (more ergonomic than a giant capturing
   closure) and lets tests pass a `move |e| ...` closure. Both paths work.
2. **`progress: None` disables everything.** No silent emission, no log write.
   This keeps tests that don't care about progress fast and identical to today.
3. **`SessionEvent::Progress` is logged regardless of whether the MCP-side
   callback is wired** — once a callback is present, the log entry happens. So
   05a alone makes the *log half* of the consumer split live. 05b adds the
   notifications half.
4. **Best-effort per-file read** in `numstat_from_pre_edit`. A vanished or
   permission-denied file contributes nothing; we don't error a heartbeat.
5. **No new dependency.** `similar` is already in the workspace (M4 phase-07e
   uses it for the diff). Phase-05a reuses it.
6. **Numstat computed per emission, not memoized.** Profile if dogfood shows
   it's hot; for typical phases (few files, small content) it's negligible.
7. **Test sites that construct `LoopDeps`** add `progress: None` — *the only*
   permitted cross-cutting change. No test logic changes.
8. **No emission in tests today** beyond the dedicated progress-callback tests
   below — the rest of the test corpus uses `progress: None`.

## Acceptance criteria

- [ ] `executor/src/agent/progress.rs` exists; declared in `agent::mod.rs`
      as `pub mod progress;`. `ProgressCallback` trait, `ProgressEvent`
      struct, `numstat_from_pre_edit`, and `format_message` are public.
- [ ] `LoopDeps` has a new field `progress: Option<&'a dyn ProgressCallback>`;
      *every* existing `LoopDeps {...}` construction in the workspace adds
      `progress: None` (executor agent tests, `mcp/src/runner.rs`,
      `mcp/src/server.rs`, anywhere else found via `grep -rn 'LoopDeps {'`).
- [ ] `numstat_from_pre_edit`: a clean file returns no entry (skipped); an
      edited file's `added`/`removed` match the `similar` line-diff counts; a
      deleted file (now unreadable) contributes `(0, after_lines_zero)` →
      `added=0, removed=before_lines`; relative-path normalization works;
      output is sorted by path.
- [ ] `format_message` formats per § 4: the prefix
      `turn=N stage=<stage> +X/-Y files=Z`, then up to 5 per-file segments,
      then ` …+K` overflow suffix when `files_changed.len() > 5`. Empty
      `files_changed` → `…files=0` and no per-file segments.
- [ ] The loop emits at exactly the four sites named in § 3 — `turn_start`,
      `tool:<name>`, `verify`, `command:<name>`. **No emission** at chat,
      parse, governor, hard-fail, or read-before-edit sites.
- [ ] On each emission: callback called *and* `SessionEvent::Progress`
      logged (best-effort, both independent). Stage strings match the spec
      exactly (`turn_start`, `tool:<name>`, `verify`, `command:<name>`).
- [ ] `progress: None` → zero callback invocations *and* zero
      `SessionEvent::Progress` log entries (assert by reading back the
      log and grepping for `event_type:"progress"`).
- [ ] **Negatives / edges:**
  - A callback that panics does not bring down the loop *if* the spec wants
    panic-isolation. (Recommended: do **not** add a `catch_unwind` — the
    callback contract is "don't panic"; document and rely on caller.)
    Pin this explicitly: a panicking callback aborts the phase. Test it.
  - A non-writable session log dir does not prevent callback invocation
    (independent side effects).
  - `numstat_from_pre_edit` over an empty `pre_edit_content` → empty Vec.
- [ ] **Handler success-path tests** (calibration carry-forward — phase-04
      bar): a test that constructs `LoopDeps` with a `Mutex<Vec<ProgressEvent>>`-
      pushing callback, runs `execute_phase` against a `MockAiClient` scripted
      to (a) emit a tool call → (b) cause a verify → (c) reach final commands,
      then asserts the recorded `ProgressEvent` sequence contains: one
      `turn_start`, one `tool:<name>`, one `verify`, then 1–4 `command:<...>`
      depending on the test config. Assert ordering, turn numbers, and
      stage tags.
- [ ] No `#[allow]`; no `unwrap()` / `expect()` / `panic!()` in production
      paths (test code exempt); no Rexy phase references.
- [ ] **Calibration carry-forward (mandatory):** declare every scope deviation
      in "Notes for review". This phase touches `executor/` substantively —
      flag every new public surface and every changed signature.
- [ ] **No new dependency.** All four required commands pass with zero new
      warnings.

## Test plan

Hermetic + deterministic.

In `executor/src/agent/progress.rs` `#[cfg(test)] mod tests`:

- **`numstat_from_pre_edit`** — a fixture `TempDir` repo + a hand-built
  `pre_edit_content` map: clean file (skipped), edited file (counts match
  `similar`), deleted file (read fails → `(0, before_lines)` or skip per
  spec — pin the negative), file under a nested dir (relative path
  normalization), unreadable file (best-effort skip), sort order
  deterministic.
- **`format_message`** — prefix-only when no files; ≤5 per-file when small;
  `…+K` overflow when many; totals are sum-across-all not sum-of-top-5.

In `executor/src/agent/mod.rs` extended test block:

- **`progress_none_emits_nothing`** — run a small phase with `progress: None`,
  assert the session log has no `progress` event (parse the JSONL).
- **`progress_some_emits_turn_start_and_tool`** — `MockAiClient` scripted to
  one tool call, capture the `ProgressEvent`s; assert the sequence and the
  `SessionEvent::Progress` entries in the log match 1:1 (same stage, turn,
  files_changed length, message).
- **`progress_emits_verify_after_edit_class_tool`** — script a `patch` tool
  call so verify runs; assert one `verify` event between the `tool:patch`
  and the next `turn_start`.
- **`progress_emits_commands_on_clean_completion`** — script a phase that
  ends in `complete`; assert one `command:<name>` per configured command in
  the final set.
- **`callback_panic_is_not_caught`** — a callback that panics terminates the
  phase. (Justification: the alternative — `catch_unwind` — adds complexity
  and obscures bugs. The callback contract is "don't panic". Pin the
  behavior so future readers don't add `catch_unwind` speculatively.)
- **`progress_independent_of_log_write_failure`** — make the session log dir
  unwriteable; assert callback still receives events. (Best-effort log,
  per the M4 pattern.)

## End-to-end verification

> Not applicable yet — the rmcp notification half is 05b. 05a is exercised
> by unit tests with a `Mutex<Vec<_>>`-capturing callback. Real wire-level
> MCP progress notification flow lands when 05b wires the rmcp peer.

## Authorizations

- [x] **May create** `executor/src/agent/progress.rs`.
- [x] **May modify** `executor/src/agent/mod.rs` to: (a) declare
      `pub mod progress;`, (b) add `progress: Option<&'a dyn ProgressCallback>`
      to `LoopDeps`, (c) insert the four emission sites in the existing turn
      cycle, (d) extend the `#[cfg(test)] mod tests` with the progress-related
      tests named in the Test plan, (e) add `progress: None` to every
      existing `LoopDeps {...}` construction in the file.
- [x] **May modify** `mcp/src/runner.rs` and `mcp/src/server.rs` solely to add
      `progress: None` to their `LoopDeps {...}` constructions. **Nothing
      else** in `mcp/` this phase — the consumer side is 05b.
- [ ] **No new dependencies.**
- [ ] May **NOT** add the rmcp consumer (05b), roots corroboration (06), or
      any other tool to the server.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `AGENTS.md`, or any other phase doc.
- [ ] **Calibration carry-forward (mandatory):** declare every scope deviation
      in "Notes for review", even defensible ones. Phase-04 had zero
      deviations; if `LoopDeps` ends up needing a tightened bound (the
      phase-02 `Send+Sync` pattern, e.g. because the trait object is held
      across an `.await`) or any other shape change, flag it.

## Out of scope

- **MCP `notifications/progress` consumer** — phase-05b.
- **Progress events at chat / parse / governor / hard-fail / read-before-edit
  sites** — explicitly excluded (architecture treats those as their own log
  events, not heartbeats).
- **`catch_unwind` around the callback** — explicit non-feature (Adaptation in
  acceptance criteria).
- **Memoizing the numstat across emissions** — premature optimization; revisit
  only if dogfood shows it's hot.
- **Roots corroboration** — phase-06.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
