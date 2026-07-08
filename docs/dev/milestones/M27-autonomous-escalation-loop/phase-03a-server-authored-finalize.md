# Phase 03a: Server-authored finalize — write the Status flip + baseline Update Log on `complete`

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** todo
**Depends on:** phase-02b (done)
**Estimated diff:** ~420 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Server-authored bookkeeping, half 1 of 2 (D8/D9). On a `complete`
`execute_phase` run, the **server** — not the executor — writes the phase
doc's Status flip (`in-progress` → `review`), appends a baseline completion
entry to the Update Log, flips the milestone README's phase-table row, and
commits those two doc changes as a **separate** `docs:` commit. This deletes
the class of `budget_exceeded` / `StuckGateFeedback` failures where a
MEDIUM-tier model writes correct code and then dies in the "bookkeeping tail"
(exactly the M22 failure mode — and the failure we hit twice dispatching
phase-02b).

This phase is **additive and dormant-safe**: the executor *still* authors its
own bookkeeping (the pre-completion bookkeeping gate in `command.rs` stays
until phase-03b). The finalize step is written so it **no-ops** whenever the
phase doc's Status is already `review` — which is exactly the state the
executor leaves it in today. So finalize does nothing observable until
phase-03b retires the executor's gate and the doc reaches finalize still at
`in-progress`. **Do not remove the executor bookkeeping gate in this phase**
(that is phase-03b) — this phase only *adds* the server-side author and a
channel to carry the executor's qualitative summary.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #27 — "server-authored bookkeeping (D8/D9 —
  on a `complete` run the *server* writes the Status flip and a baseline
  Update Log entry from data it already holds…)".
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § Exit
  criteria, the bullet beginning "On a `complete` `execute_phase`, the
  **server** writes the phase doc's Status flip…".

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### The data the server already holds

`PhaseResult` (`executor/src/phase/result.rs`) is what the loop returns and
the server has in hand after `agent::execute_phase`. It already carries
everything a baseline completion entry needs — `files_changed`, the
`command_outputs` gate tails, and `log_path`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseResult {
    pub status: PhaseStatus,
    pub files_changed: Vec<FileChange>,
    pub diff: String,
    pub command_outputs: CommandOutputs,
    pub update_log: String,
    pub briefing: Option<Briefing>,
    pub log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}
```

The one thing the server does *not* hold is the executor's qualitative prose
(what it built, any adaptations/divergences worth flagging at review). Task 1
adds a channel for that: the executor's final message text (the completion
signal — the assistant turn with no tool call) is captured into a new
`completion_summary` field, which the server splices into the entry.

### Where the server gets the result

`mcp/src/runner.rs` `run_phase_with` is the inner assembler both the MCP
`execute_phase` tool and the CLI `run-phase` path funnel through. Its tail
today:

```rust
    let mut result = agent::execute_phase(&input, deps).await?;
    result.warnings.extend(input_warnings);
    Ok(result)
```

`run_phase_with` already has the two seams finalize needs: `seams.clock`
(a `&dyn Fn() -> u64` returning epoch-ms) and `seams.runner` (a
`&dyn CommandRunner` — used today for the final gate set; reused here for the
`git` calls). The `CommandRunner` trait (`executor/src/agent/command.rs`):

```rust
#[async_trait]
pub trait CommandRunner: Send + Sync {
    async fn run(&self, command: &str, cwd: &Path) -> CommandResult;
}
// CommandResult { pub output: String, pub success: bool }
```

### The complete path in the loop

`executor/src/agent/mod.rs`, the "true completion" tail (all gates passed):

```rust
                    // All configured gates passed — this is a true completion.
                    log_session_end(&log_handle, &redactor, deps.clock, "complete", turns);
                    emit_phase_run(&deps, input, "complete", gates, &metrics, &scorer, turns);
                    let artifacts = build_artifacts(
                        &pre_edit_content,
                        deps.project_root,
                        log_path.clone(),
                        "complete",
                        turns,
                        command_outputs,
                    );
                    return Ok(PhaseResult::complete(artifacts));
```

The local `completion` string (declared per-turn as
`let mut completion = String::new();`, accumulated from `AiEvent::Token`) holds
the model's final message here. The post-think text is
`crate::parser::strip_think_blocks(&completion)` (`fn strip_think_blocks(s:
&str) -> String`, already `pub`) — that is what Task 1 captures.

### Doc shapes finalize matches (pin these exactly)

- The Status line in a phase-doc frontmatter is exactly `**Status:** in-progress`
  (and the target is `**Status:** review`).
- The milestone README phase-table row looks like (note the trailing status
  cell): `| 03a | Server-authored finalize … ([phase-03a-server-authored-finalize.md](phase-03a-server-authored-finalize.md)) | in-progress |`.
- No date/time crate is available (no `chrono`, no `time`) and **you may not
  add one** (dependency = blocker). The entry's timestamp is the raw
  epoch-ms integer from the clock.

## Spec

Numbered tasks in execution order.

### Task 1 — Add the `completion_summary` channel to `PhaseResult`

In `executor/src/phase/result.rs`:

1a. Add a field to the `Artifacts` struct (after `log_path`):

```rust
    /// Path to the on-disk JSONL session log; `None` when the log failed to open.
    pub log_path: Option<PathBuf>,
    /// The executor's final message text (post-think), captured on the
    /// `complete` path. Empty on failure paths and until phase-03b makes the
    /// executor put its Summary/Notes here. Spliced into the server-authored
    /// completion entry.
    pub completion_summary: String,
```

1b. Add the matching field to `PhaseResult` (after `warnings`):

```rust
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub completion_summary: String,
```

1c. In `PhaseResult::assemble`, copy it through:

```rust
            log_path: artifacts.log_path,
            warnings: Vec::new(),
            completion_summary: artifacts.completion_summary,
```

1d. Every `Artifacts { … }` literal must now set the field. There are two
production/helper sites plus one test helper — update all three:

- `executor/src/agent/outcome.rs` `build_artifacts` — add
  `completion_summary: String::new(),` to the `Artifacts { … }` it returns
  (all terminal paths default to empty here).
- `executor/src/phase/result.rs` the `fn artifacts()` test helper — add
  `completion_summary: String::new(),`.
- Grep after editing: `grep -rn "Artifacts {" executor/src` and confirm every
  struct-literal site compiles (the two above; the others are the struct def
  and the return-type annotation, not literals).

1e. Populate it on the complete path only. In `executor/src/agent/mod.rs`, the
"true completion" tail quoted in Current state — change the `let artifacts`
binding to `let mut`, set the field from the post-think completion text, then
return:

```rust
                    let mut artifacts = build_artifacts(
                        &pre_edit_content,
                        deps.project_root,
                        log_path.clone(),
                        "complete",
                        turns,
                        command_outputs,
                    );
                    artifacts.completion_summary =
                        crate::parser::strip_think_blocks(&completion);
                    return Ok(PhaseResult::complete(artifacts));
```

Leave every other `build_artifacts` call site (hard_fail / budget_exceeded
paths) unchanged — they keep the empty default.

### Task 2 — Add the `finalize` module

Create `mcp/src/finalize.rs` and declare it in `mcp/src/main.rs` with
`mod finalize;` (insert alphabetically, after `mod doctor;` and before
`mod init;`).

The module's one public entry point is `finalize_complete`. It **no-ops**
(returns `Ok(false)`) unless the result is `Complete` **and** the phase doc's
Status is still `in-progress`; on the active path it flips Status, appends the
baseline entry, flips the README row, commits, and returns `Ok(true)`.
Structure (pin the behavior; you may adjust helper names/signatures to
compile cleanly):

```rust
use std::path::{Path, PathBuf};

use rexymcp_executor::agent::command::CommandRunner;
use rexymcp_executor::phase::{PhaseResult, PhaseStatus};

/// Inputs for the server-authored completion finalize step.
pub struct FinalizeInput<'a> {
    pub phase_doc_path: &'a Path,
    pub repo_root: &'a Path,
    pub result: &'a PhaseResult,
    pub now_ms: u64,
    pub runner: &'a dyn CommandRunner,
}

/// Server-authored bookkeeping for a completed phase. No-op (returns
/// `Ok(false)`) unless the result is `Complete` and the phase doc's
/// `**Status:**` line still reads `in-progress` — so this is inert while the
/// executor still authors its own bookkeeping (see phase-03b). On the active
/// path: flip Status to `review`, append a baseline completion entry, flip the
/// sibling milestone README's phase-table row, commit the doc changes as a
/// separate `docs:` commit, and return `Ok(true)`.
pub async fn finalize_complete(inp: &FinalizeInput<'_>) -> std::io::Result<bool> {
    if inp.result.status != PhaseStatus::Complete {
        return Ok(false);
    }
    let doc = std::fs::read_to_string(inp.phase_doc_path)?;
    if !status_is_in_progress(&doc) {
        return Ok(false);
    }

    let code_sha = git_head(inp.runner, inp.repo_root).await;
    let entry = baseline_entry(inp.result, inp.now_ms, &code_sha);
    let flipped = flip_status_to_review(&doc);
    let new_doc = append_entry(&flipped, &entry);
    std::fs::write(inp.phase_doc_path, new_doc)?;

    let mut staged: Vec<PathBuf> = vec![inp.phase_doc_path.to_path_buf()];
    if let Some(readme) = inp.phase_doc_path.parent().map(|p| p.join("README.md")) {
        if let Ok(readme_doc) = std::fs::read_to_string(&readme) {
            if let Some(stem) = inp.phase_doc_path.file_name().and_then(|s| s.to_str()) {
                if let Some(updated) = flip_readme_row(&readme_doc, stem) {
                    std::fs::write(&readme, updated)?;
                    staged.push(readme);
                }
            }
        }
    }

    git_commit_docs(inp.runner, inp.repo_root, &staged).await;
    Ok(true)
}
```

Helper behavior to implement (each is a small pure fn except the two `git_*`):

- `status_is_in_progress(doc) -> bool` — true iff some line, trimmed, equals
  `**Status:** in-progress`. **Pin negatives:** a line reading
  `**Status:** review`, `**Status:** todo`, or `**Status:** done` returns
  false; prose that merely *contains* the word "in-progress" elsewhere does
  not count (match the `**Status:**`-prefixed line only).
- `flip_status_to_review(doc) -> String` — replace the single frontmatter line
  `**Status:** in-progress` with `**Status:** review`, leaving everything else
  byte-identical. Replace only the first such line.
- `baseline_entry(result, now_ms, code_sha) -> String` — build the entry
  (format pinned in Task 3).
- `append_entry(doc, entry) -> String` — return `doc` with the entry appended
  at end of file, separated by a blank line (`format!("{}\n{}\n",
  doc.trim_end(), entry)`). Entries in this repo accrue at the bottom of the
  Update Log, after the `<!-- entries appended below this line -->` marker; a
  plain end-of-file append lands there.
- `flip_readme_row(readme_doc, phase_doc_filename) -> Option<String>` — find
  the one table row that contains `phase_doc_filename` (e.g.
  `phase-03a-server-authored-finalize.md`) **and** ends (after trimming) with
  `| in-progress |`; replace that row's trailing `| in-progress |` with
  `| review |`. Return `None` if no such row (README row already `review`, or
  no README table row references this phase) — a `None` is not an error, it
  just means "nothing to stage." **Pin negatives:** a sibling row for a
  *different* phase file must be untouched; a row that references this phase
  but is already `| review |` / `| done |` is left as-is (returns `None`).
- `git_head(runner, repo_root) -> String` — run `git rev-parse HEAD` via the
  runner in `repo_root`; return the trimmed stdout on success, or the string
  `"unknown"` on failure (never error out — the entry is best-effort).
- `git_commit_docs(runner, repo_root, paths)` — stage exactly `paths` and
  commit. Run, in order: `git add -- <path> <path> …` (space-join the paths,
  each shell-quoted is unnecessary here since our paths have no spaces, but
  build the arg list from the real paths), then
  `git commit -m "docs: server-authored completion bookkeeping"`. Both via the
  runner in `repo_root`. Ignore failures (best-effort; a failure surfaces as a
  dirty tree the reviewer sees). Do **not** run `git add -A` — stage only the
  doc paths, so the executor's separate code commit is never disturbed.

### Task 3 — The baseline entry format

`baseline_entry` returns exactly this shape (a machine-parseable header with
raw epoch-ms, since no date crate is available):

```
### Update — ts=<now_ms> (complete, server-authored)

**Summary:** <completion_summary, trimmed — or "(no summary provided by executor)" when empty>

**Gates:** format=<g>, build=<g>, lint=<g>, test=<g>

**Command output tails:**

```
<for each of format/build/lint/test whose command_outputs field is Some: the label in caps, a newline, the tail; separated by blank lines. If all four are None, the single line "(no command output captured)".>
```

**Files changed:**
<for each FileChange: "- `<path>` — <change_summary>"; or the single line "(none)" when files_changed is empty>

**Commit:** <code_sha>

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).
```

Where `<g>` for each gate maps the `Option<bool>` in `result.command_outputs`
→ wait: the gate pass/fail booleans are **not** on `command_outputs` (that
holds only the output tails). Derive `<g>` from whether the tail is present
and the run completed: use the presence of the corresponding
`command_outputs.<field>` as `run`/`skipped` only — specifically map
`Some(_) => "run"`, `None => "skipped"`. (The true pass/fail booleans live in
the loop's `Gates` and are not carried in `PhaseResult`; a `complete` result
means every configured gate already passed, so "run" is accurate and
sufficient for the baseline entry. Do not invent a pass/fail you don't have.)

### Task 4 — Wire finalize into `run_phase_with`

In `mcp/src/runner.rs`, change the tail of `run_phase_with` to call finalize
after stamping warnings. Finalize failure is **non-fatal** and surfaces as a
warning (never turns a `Complete` into an `Err`):

```rust
    let mut result = agent::execute_phase(&input, deps).await?;
    result.warnings.extend(input_warnings);

    let finalize_input = crate::finalize::FinalizeInput {
        phase_doc_path: inp.phase_doc_path,
        repo_root: inp.repo_path,
        result: &result,
        now_ms: (seams.clock)(),
        runner: seams.runner,
    };
    if let Err(e) = crate::finalize::finalize_complete(&finalize_input).await {
        result
            .warnings
            .push(format!("server finalize failed: {e}"));
    }
    Ok(result)
```

Note `run_phase_with` lives in the `runner` module of the `rexymcp` binary
crate, so `crate::finalize::…` resolves. Confirm the existing runner.rs tests
still pass unchanged — they use phase docs whose Status is `review` or absent,
so finalize returns `Ok(false)` (no-op) and touches nothing.

## Acceptance criteria

- [ ] `PhaseResult` and `Artifacts` have a `completion_summary: String` field;
      it is populated (post-think) only on the loop's `complete` path and empty
      on every failure path.
- [ ] `PhaseResult` JSON with a non-empty `completion_summary` round-trips; an
      empty one is omitted from JSON (skip_serializing_if); a JSON missing the
      key deserializes with an empty default.
- [ ] `mcp/src/finalize.rs` exists and is declared `mod finalize;` in
      `mcp/src/main.rs`.
- [ ] `finalize_complete` returns `Ok(false)` and mutates nothing when the
      result status is not `Complete`, and when the phase doc's Status is
      already `review` (the dormant-safe no-op that keeps this phase inert
      until 03b).
- [ ] On a `Complete` result + an `in-progress` phase doc, `finalize_complete`
      flips `**Status:** in-progress` → `**Status:** review`, appends a
      `(complete, server-authored)` entry containing the command-output tails,
      the files-changed list, the code commit sha, and the spliced
      `completion_summary`, and returns `Ok(true)`.
- [ ] `flip_readme_row` flips only the row referencing this phase's file and
      only when it ends `| in-progress |`; a sibling row for a different phase
      is byte-identical after the call (pinned negative).
- [ ] finalize stages **only** the doc paths (`git add -- <doc> [<readme>]`,
      never `git add -A`) and commits with a `docs:` message; git failures are
      swallowed (best-effort).
- [ ] `run_phase_with` calls finalize after stamping warnings; a finalize
      `Err` becomes a `result.warnings` entry, never an `Err` return.
- [ ] Existing `mcp/src/runner.rs` tests pass unchanged (finalize no-ops for
      their docs).
- [ ] The executor's own bookkeeping gate (`command::bookkeeping_feedback`) is
      **still present and unchanged** — retiring it is phase-03b, explicitly
      out of scope here.
- [ ] `cargo build` zero new warnings; `cargo clippy --all-targets
      --all-features -- -D warnings` passes; `cargo fmt --all --check` passes;
      `cargo test` passes (existing + new).

## Test plan

New tests (hermetic, `TempDir`; use a recording mock `CommandRunner` that
captures the commands it is asked to run and returns canned stdout for
`git rev-parse HEAD`):

- `phase_result_completion_summary_round_trips` (result.rs) — non-empty value
  survives a JSON round-trip.
- `phase_result_empty_completion_summary_omitted_from_json` (result.rs) —
  empty value is not serialized.
- `finalize_noop_when_status_already_review` (finalize.rs) — an `in-progress`?
  No: a `review` doc + `Complete` result → returns `Ok(false)`, doc byte-
  identical, and the recording runner saw **no** `git commit`. (This is the
  load-bearing dormant-safety pin.)
- `finalize_noop_when_result_not_complete` (finalize.rs) — a `HardFail` result
  → `Ok(false)`, no mutation.
- `finalize_flips_status_and_appends_entry` (finalize.rs) — `in-progress` doc +
  `Complete` result with two `files_changed`, `command_outputs` tails, and a
  non-empty `completion_summary` → doc Status becomes `review`, and the
  appended entry contains: `(complete, server-authored)`, each file path, the
  summary text, and the sha the mock returned for `git rev-parse HEAD`.
- `finalize_updates_matching_readme_row_only` (finalize.rs) — a README with two
  rows (this phase `| in-progress |` and a sibling phase `| in-progress |`) →
  only this phase's row becomes `| review |`; the sibling row is unchanged.
- `finalize_stages_only_doc_paths` (finalize.rs) — the recording runner's
  captured `git add` command references the phase doc (and README) paths and is
  **not** `git add -A`.

## End-to-end verification

The counting-style unit tests above call the **real** `finalize_complete`
against **real** on-disk phase-doc + README files in a `TempDir` (only the
`git` subprocess is mocked, via the injected `CommandRunner`), so the
file-mutation path is exercised end-to-end. Additionally, prove the
dormant-safety claim against this very repo's tooling by building the binary
and confirming the finalize no-ops on an already-`review` doc — paste into the
completion Update Log:

1. `cargo build` (produces `target/debug/rexymcp`).
2. Confirm no behavior change to a normal run: the existing
   `run_phase_with_assembles_and_returns_result` test (its fixture doc is
   `**Status:** review`) passes — `cargo test -p rexymcp
   run_phase_with_assembles_and_returns_result` → ok. This is the proof that a
   completed run whose doc is already `review` (today's executor-authored
   world) is untouched by the new server author.

## Authorizations

None. No new dependency (**do not** add a date/time crate — use raw epoch-ms),
no `Cargo.toml` edit, no `architecture.md`/`STANDARDS.md`/`WORKFLOW.md` edit,
no edit to `executor/templates/executor_contract.md` (the contract amendment
is phase-03b).

## Out of scope

- **Retiring `command::bookkeeping_feedback`** (the executor's pre-completion
  bookkeeping gate) and **amending the executor contract**. That is phase-03b.
  This phase must leave both untouched — finalize is dormant behind the still-
  active executor author.
- **The start-of-phase flip** (`todo` → `in-progress`). It stays with the
  executor; the README exit criterion scopes server authorship to the
  `in-progress → review` completion transition only.
- **A human-readable timestamp** in the entry. No date crate exists and adding
  one is a blocker; raw epoch-ms is the pinned format.
- **Pass/fail gate booleans in the entry.** `PhaseResult` carries only output
  tails, not the `Gates` booleans; do not thread `Gates` into `PhaseResult`
  for this — "run/skipped" from tail presence is the pinned behavior.
- **This repo's live phase docs.** Do not hand-edit any milestone doc's Status
  as part of testing; all finalize tests operate in a `TempDir`.

## Gotchas (pre-injected)

- **Dormant safety is the whole point of the no-op guard.** The `status_is_in_progress`
  check is what keeps this phase from changing any observable behavior until
  03b. If you find yourself wanting finalize to act on a `review` doc, stop —
  that would double-author (executor + server) in the window before 03b.
- **`git add -A` is forbidden here.** Staging everything would sweep the
  executor's uncommitted-or-not source into the server's `docs:` commit and
  merge the two commits the design deliberately keeps separate. Stage only the
  doc paths.
- **The gate booleans are not on `PhaseResult`.** `command_outputs` holds only
  `Option<String>` tails. Map presence → `"run"`/`"skipped"`; do not fabricate
  a pass/fail. A `complete` status already means the gates passed.
- **Append at end of file.** The Update Log's entries live at the bottom after
  the HTML marker comment; a trailing append lands in the right place without
  needing to locate the marker.
- **Match the `**Status:**` line, not substrings.** `status_is_in_progress`
  and the flip must key off the frontmatter line whose trimmed form starts
  with `**Status:**`, never off prose containing "in-progress".
- **Reuse the injected seams.** `run_phase_with` already holds `seams.clock`
  and `seams.runner`; pass them into `FinalizeInput`. Do not construct a new
  system clock or shell out directly with `std::process` — that would break the
  hermetic testability the seams exist for.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
