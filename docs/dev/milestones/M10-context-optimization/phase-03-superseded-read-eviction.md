# Phase 03: superseded-read eviction

**Milestone:** M10 — Context optimization
**Status:** todo
**Depends on:** phase-01, phase-02 (Arc A complete). This is the first Arc B phase.
**Estimated diff:** ~160 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

When the model reads a file at turn 3 and then edits it at turn 7, the original
turn-3 read content is still sitting in the message history — but it is now
**wrong** (the file changed). It wastes context and is a correctness hazard: the
model could reason from pre-edit content. This phase makes an edit (`patch` /
`write_file`) immediately replace every prior `read_file` result for that same
file with a short breadcrumb pointing the model at a fresh re-read. Context
shrinks and the stale content is gone.

This is the first Arc B (semantic context-lifecycle) lever — it uses the
read→edit transition the agent loop already tracks. The breadcrumb is always
safe: the read-before-edit gate already forces a re-read before the next `patch`,
so removing stale read content can never cause a wrong action — at worst the model
re-reads (one extra turn).

## Architecture references

Read before starting:

- `executor/src/agent/tools.rs` — the working-set helpers
  (`read_before_edit_refusal`, `record_mtime`, `resolve_path`, `edit_target`,
  `append_tool_exchange`). The new function lives in this file alongside them.
- `executor/src/agent/mod.rs` lines 651–660 — the call site, right after the
  tool exchange is appended and the working set is recorded.
- `docs/dev/milestones/M10-context-optimization/README.md` §"What is novel to
  rexyMCP (Arc B)" item 1 (superseded-read eviction) — the design intent.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).
5. Run `cargo test` and record the passing count — the completion log must show
   the same count plus the new tests.

## Current state

**Message shape.** A `read_file` exchange is two consecutive messages, appended by
`append_tool_exchange` (`executor/src/agent/tools.rs:123`): an **assistant**
message carrying the `read_file` tool call (the path is in its JSON-string
`arguments`), immediately followed by a **tool** message carrying the result
`content`. Quoted from `tools.rs:123-154`:

```rust
pub(super) fn append_tool_exchange(messages: &mut Vec<Message>, tc: &ToolCall, content: &str, turn: usize) {
    let id = next_tool_id();
    let arguments = serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string());
    messages.push(Message {
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(vec![AiToolCall { id: id.clone(), name: tc.name.clone(), arguments, thought_signature: None }]),
        tool_results: None,
        turn: Some(turn),
    });
    messages.push(Message {
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult { tool_call_id: id, tool_name: tc.name.clone(), content: content.to_string() }]),
        turn: Some(turn),
    });
}
```

So: the **path** lives on the assistant message (`tool_calls[0].arguments`, a JSON
string), and the **content** to replace lives on the *next* message
(`tool_results[0].content`). They are always adjacent and in this order.

**`AiToolCall` / `AiToolResult` fields** (from `crate::ai::types`, already imported
at the top of `tools.rs`): `AiToolCall { id, name, arguments, thought_signature }`;
`AiToolResult { tool_call_id, tool_name, content }`. `Message { role, content,
tool_calls, tool_results, turn }`.

**Path resolution idiom** — mirror `resolve_path` (`tools.rs:25-32`), which joins a
relative path against the project root:

```rust
pub(super) fn resolve_path(tool_call: &ToolCall, project_root: &Path) -> Option<PathBuf> {
    let path = PathBuf::from(tool_call.arguments.get("path").and_then(|v| v.as_str())?);
    Some(if path.is_absolute() { path } else { project_root.join(path) })
}
```

The new function parses the path from the *stored JSON string* (not a `ToolCall`),
so it re-implements this join inline (`PathBuf::is_absolute` / `project_root.join`).

**The call site** (`executor/src/agent/mod.rs:651-660`), where the exchange is
appended and the working set is recorded:

```rust
        append_tool_exchange(&mut messages, &tool_call, &content, turns);

        // Record the working set: a read makes a file patch-eligible; a successful
        // patch refreshes its mtime so a follow-up patch needs no re-read.
        if succeeded
            && (tool_call.name == "read_file" || tool_call.name == "patch")
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            record_mtime(&mut working_set, &path);
        }
```

`edit_path: Option<PathBuf>` is already in scope here (`mod.rs:587`,
`edit_target(&tool_call, deps.project_root)`) — it is `Some` exactly for a
`write_file`/`patch` call and carries the resolved target path. `turns` and
`deps.project_root` are also in scope.

**Observability for the integration test.** The agent-loop tests inspect the
messages sent to the model via `client.calls()[N].messages` (see
`patch_after_reading_is_allowed`, `mod.rs:2112`). Because eviction mutates the
in-memory `messages` that are sent on the *next* turn, the breadcrumb is visible
in the subsequent call's messages. No new `SessionEvent` is added (the dashboard
has no consumer for it yet; `PhaseRun` metrics for reclaimed context are phase-06).

## Spec

Numbered tasks in execution order.

### 1. Add `evict_superseded_reads` to `executor/src/agent/tools.rs`

Add a module-level constant and the function, near the other working-set helpers
(after `record_mtime`). The reference implementation — match its **behavior**
(pinned by the Test plan); minor structural choices are yours:

```rust
/// Stable marker prefix for an evicted (superseded) read result. Used both to
/// build the breadcrumb and to detect an already-evicted result (idempotence).
const SUPERSEDED_PREFIX: &str = "[superseded:";

/// Replace the content of every prior `read_file` tool-result for `edited_path`
/// with a short re-read breadcrumb, because the file's on-disk content changed
/// when the model edited it — the earlier read is now stale and only wastes
/// context. Returns the number of read results evicted.
///
/// Safe by construction: the read-before-edit gate already forces a re-read
/// before the next `patch`, so removing stale read content never causes a wrong
/// action. Idempotent: a result already replaced by a breadcrumb (detected via
/// `SUPERSEDED_PREFIX`) is left alone, so re-editing the same file does not
/// re-evict or stack breadcrumbs.
pub(super) fn evict_superseded_reads(
    messages: &mut [Message],
    edited_path: &Path,
    turn: usize,
    project_root: &Path,
) -> usize {
    let breadcrumb = format!(
        "{SUPERSEDED_PREFIX} file edited at turn {turn}; this earlier read is stale — \
         re-read with read_file for current content]"
    );
    let mut evicted = 0;
    for i in 0..messages.len() {
        // Is messages[i] an assistant `read_file` call whose path == edited_path?
        let matches_read = messages[i]
            .tool_calls
            .as_ref()
            .and_then(|tcs| tcs.first())
            .filter(|tc| tc.name == "read_file")
            .and_then(|tc| serde_json::from_str::<serde_json::Value>(&tc.arguments).ok())
            .and_then(|v| v.get("path").and_then(|p| p.as_str()).map(str::to_string))
            .map(|p| {
                let pb = std::path::PathBuf::from(&p);
                let resolved = if pb.is_absolute() { pb } else { project_root.join(pb) };
                resolved == edited_path
            })
            .unwrap_or(false);
        if !matches_read {
            continue;
        }
        // The result content is on the next message (the tool message).
        if let Some(next) = messages.get_mut(i + 1)
            && let Some(results) = next.tool_results.as_mut()
            && let Some(r) = results.first_mut()
            && r.tool_name == "read_file"
            && !r.content.starts_with(SUPERSEDED_PREFIX)
        {
            r.content = breadcrumb.clone();
            evicted += 1;
        }
    }
    evicted
}
```

### 2. Call it at the edit site in `executor/src/agent/mod.rs`

Immediately **after** the working-set record block (`mod.rs:660`, the closing `}`
of the `if succeeded && (… read_file … patch …)` block), add:

```rust
        // Superseded-read eviction (M10 Arc B): a successful edit makes every
        // prior read of this file stale. Replace those read results with a
        // re-read breadcrumb to reclaim context and remove the stale-content
        // hazard. Always safe — the read-before-edit gate forces a re-read.
        if succeeded && let Some(path) = &edit_path {
            evict_superseded_reads(&mut messages, path, turns, deps.project_root);
        }
```

The return value is intentionally unused here — counting reclaimed context onto
`PhaseRun` is phase-06's job (do **not** add a metrics field for it now; that is
the "wire in a consumer that doesn't exist yet" trap).

Import: `evict_superseded_reads` is `pub(super)` in `tools.rs`; reference it the
same way the call site already references `append_tool_exchange`, `record_mtime`,
`resolve_path` (they share a `use super::tools::…` / `use super::…` import — find
the existing import line in `mod.rs` that brings in `record_mtime` and add
`evict_superseded_reads` to it).

## Acceptance criteria

- [ ] `grep -n 'pub(super) fn evict_superseded_reads' executor/src/agent/tools.rs` matches.
- [ ] `grep -n 'evict_superseded_reads' executor/src/agent/mod.rs` matches (the call site).
- [ ] After a successful `patch`/`write_file` of `foo`, every prior `read_file`
      result for `foo` in the message history has its content replaced by a
      breadcrumb starting `[superseded:`.
- [ ] A `read_file` result for a **different** file is left untouched. **(negative)**
- [ ] A non-`read_file` tool result (e.g. a `bash` result) is never evicted, even
      across the same turn. **(negative)**
- [ ] Eviction is idempotent: running it twice (e.g. editing the same file twice)
      does not stack breadcrumbs or change the count on the second run. **(negative)**
- [ ] The breadcrumb text mentions the turn number and instructs a `read_file`
      re-read.
- [ ] In an agent-loop run that reads `foo` then patches `foo`, the model call
      *after* the patch carries the breadcrumb in place of the original read
      content (verified via `client.calls()`).
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Unit tests in a new `#[cfg(test)] mod tests` block at the bottom of
`executor/src/agent/tools.rs`. Construct `Message` / `AiToolCall` /
`AiToolResult` directly (all in `crate::ai::types`, already imported). A small
local helper that builds a read-exchange pair (assistant call + tool result) and
an edit-result message keeps these terse. Path resolution is pure (`PathBuf::join`,
no filesystem), so these tests need **no** `TempDir`.

- `evict_superseded_reads_replaces_prior_read_of_edited_file` — messages =
  [read_file(`/r/foo.rs`) call, read result "OLD CONTENT"]; call
  `evict_superseded_reads(&mut msgs, Path::new("/r/foo.rs"), 7, Path::new("/r"))`
  → returns 1, the tool result content starts with `[superseded:` and no longer
  contains "OLD CONTENT".

- `evict_superseded_reads_resolves_relative_read_path` — the read call's `path`
  arg is relative (`"foo.rs"`) and `project_root` is `/r`; editing `/r/foo.rs`
  → the read is evicted (confirms the relative-join resolution).

- `evict_superseded_reads_leaves_other_files_untouched` — read of `/r/bar.rs`,
  edit `/r/foo.rs` → returns 0, the `bar.rs` read content is unchanged.
  **(negative case)**

- `evict_superseded_reads_ignores_non_read_results` — a `bash` exchange (assistant
  bash call + tool result whose `tool_name == "bash"`) for a command string that
  has no bearing → returns 0, the bash result content is unchanged. **(negative)**

- `evict_superseded_reads_is_idempotent` — run eviction twice for the same edited
  path → first run returns 1, second run returns 0, and the content is a single
  breadcrumb (does not contain a nested/second `[superseded:`). **(negative)**

- `evict_superseded_reads_evicts_multiple_prior_reads` — two separate
  `read_file(/r/foo.rs)` exchanges in the history → returns 2, both contents are
  breadcrumbs.

- `evict_superseded_reads_breadcrumb_mentions_turn_and_reread` — breadcrumb
  contains `"turn 7"` and `"read_file"`.

Agent-loop integration test in `executor/src/agent/mod.rs`'s existing
`#[cfg(test)] mod tests` (mirror `patch_after_reading_is_allowed`, `mod.rs:2112`):

- `loop_evicts_prior_read_after_patch` — script the model to (turn 1)
  `read_file(foo)`, (turn 2) `patch(foo, …)`, (turn 3) `token("done")`. After the
  run, assert that `client.calls()[2].messages` (the messages sent on the call
  after the patch) contains a `read_file` tool result whose content starts with
  `[superseded:`, and does **not** contain the original file content string.
  Use `run_with_verifier(&dir, &client, &verifier, 8)` with a clean
  `MockFileVerifier`, exactly as `patch_after_reading_is_allowed` does.

## End-to-end verification

This phase ships no runtime-loadable real artifact (no CLI surface, no config the
binary loads) — it is internal agent-loop behavior. Per WORKFLOW.md, the
agent-loop integration test (`loop_evicts_prior_read_after_patch`) using the real
`run_loop` over a `MockAiClient` + real `TempDir` filesystem **is** the
end-to-end exercise of the shipped behavior: it runs the actual loop, performs a
real on-disk patch, and observes the evicted breadcrumb in what the loop sends the
model. In the completion log, run:

`cargo test loop_evicts_prior_read_after_patch -- --nocapture`

and quote the assertion outcome (pass) and, if practical, the breadcrumb string
observed in `client.calls()[2].messages`.

## Authorizations

None. No new dependency (`serde_json` is already used throughout `tools.rs`). No
`docs/architecture.md` change. No `Cargo.toml` change.

## Out of scope

- **Content-aware compaction priority** (ranking superseded reads first *during
  compaction*) — that is phase-05 (Arc B item 3). This phase evicts eagerly at
  edit time; it does **not** touch `context/compactor.rs`, `TARGET_FRACTION`, or
  the compaction passes.
- **Redundant re-read dedupe** (returning "unchanged since turn N" when the model
  re-reads an unchanged file) — that is phase-04 (Arc B item 2). Do not modify
  `read_file` or the read path.
- **A config kill-switch.** Eviction is always-safe (breadcrumb + read-before-edit
  gate guarantee recovery), and a switch would require threading config through the
  loop's `LoopDeps` — a wide-blast-radius change out of proportion to a one-line
  call site. Deferred; revisit only if a real need appears.
- **A new `SessionEvent` variant or `PhaseRun` metric** for evictions — phase-06
  adds the reclaimed-context metric and its consumer together. Do not wire a
  metrics field whose reader does not yet exist.
- **Evicting reads of files edited via shell `bash` (e.g. `sed -i`).** Only
  `write_file`/`patch` (the tracked edit-class calls, `edit_path.is_some()`)
  trigger eviction — bash-side edits are not tracked by the working set and are
  out of scope.
- **Touching the read-before-edit gate, the working set's mtime logic, or the
  baseline/verifier flow.** Eviction is additive alongside them.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
