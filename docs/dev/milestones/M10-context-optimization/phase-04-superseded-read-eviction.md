# Phase 04: superseded-read eviction

**Milestone:** M10 — Context optimization
**Status:** todo
**Depends on:** phase-03 (the per-lever `OutputFiltered` reclaim-event pattern this phase mirrors for `ReadEvicted`). Arc A (phase-01/02) complete. First Arc B *behavior* phase.
**Estimated diff:** ~200 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

When the model reads a file at turn 3 and then edits it at turn 7, the original
turn-3 read content is still sitting in the message history — but it is now
**wrong** (the file changed). It wastes context and is a correctness hazard: the
model could reason from pre-edit content. This phase makes an edit (`patch` /
`write_file`) immediately replace every prior `read_file` result for that same
file with a short breadcrumb pointing the model at a fresh re-read. Context
shrinks and the stale content is gone. The reclaim is recorded as a per-lever
`SessionEvent::ReadEvicted` (mirroring phase-03's `OutputFiltered`), so the win is
measurable on the live dashboard and aggregatable onto `PhaseRun` in phase-07.

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
- `executor/src/agent/mod.rs` lines 651–684 — the call site, right after the
  tool exchange is appended, the phase-03 `OutputFiltered` block, and the
  working-set record.
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

**The call site** (`executor/src/agent/mod.rs`). After phase-03, the order is:
`append_tool_exchange` (line 651) → the phase-03 `OutputFiltered` emit block
(lines 653–675) → the working-set record block (lines 677–684). The eviction call
goes **after** the working-set record block (after line 684), before the post-write
format hook that follows it:

```rust
        append_tool_exchange(&mut messages, &tool_call, &content, turns);

        // … phase-03 OutputFiltered emit block (lines 653–675) …

        // Record the working set: a read makes a file patch-eligible; a successful
        // patch refreshes its mtime so a follow-up patch needs no re-read.
        if succeeded
            && (tool_call.name == "read_file" || tool_call.name == "patch")
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            record_mtime(&mut working_set, &path);
        }
        // ← Task 3's eviction block goes here (line 685), before the format hook.
```

`edit_path: Option<PathBuf>` is already in scope here (`mod.rs:587`,
`edit_target(&tool_call, deps.project_root)`) — it is `Some` exactly for a
`write_file`/`patch` call and carries the resolved target path. `turns` and
`deps.project_root` are also in scope.

**Observability.** Two channels: (1) the breadcrumb is visible in the next model
call's messages — the agent-loop tests inspect `client.calls()[N].messages` (see
`patch_after_reading_is_allowed`, `mod.rs:2138`), and eviction mutates the
in-memory `messages` sent on the *next* turn; (2) a `SessionEvent::ReadEvicted`
event is logged (consumed by the dashboard transcript + log-query tools now;
aggregated onto `PhaseRun` in phase-07).

**The per-lever event pattern is established by phase-03** (`OutputFiltered`), and
phase-03's review pinned the exact blast radius (its first dispatch hard-failed by
under-listing it — see Task 4 for the full enumeration). Adding a `SessionEvent`
variant touches:

- `mcp/src/log_query.rs` `event_type_str` — the **production** kind-string fn.
- `mcp/src/dashboard/filter.rs` — **seven** sites: `FILTER_ITEM_COUNT`, the
  `ActivityFilter` field, its `Default`, and the `allows` / `toggle` / `is_enabled`
  / `item_label` matches (+ the `filter_default_disables_progress` test assertion).
- `mcp/src/dashboard/transcript.rs` `record_lines` — the transcript render arm.
- `executor/src/agent/mod.rs` test-helper `event_kind` (inside `mod tests`) — an
  exhaustive match, so the test crate won't compile without it.

The `status.rs` `_ => {}`, `cap.rs` catch-all, and the generic-serde sites
(`agent/log.rs`, `store/sessions/jsonl.rs`) need **no** change. Phase-03's
`OutputFiltered` arms (already on the tree) are the worked example to mirror for
`ReadEvicted` at every site above.

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
/// context. Returns `(reads_evicted, tokens_reclaimed)` where `tokens_reclaimed`
/// is the summed `tokens::count(original) - tokens::count(breadcrumb)` across the
/// evicted reads (the chars/4 estimate, same heuristic as the budget) — the loop
/// uses it to emit a `ReadEvicted` event.
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
) -> (usize, usize) {
    let breadcrumb = format!(
        "{SUPERSEDED_PREFIX} file edited at turn {turn}; this earlier read is stale — \
         re-read with read_file for current content]"
    );
    let breadcrumb_tokens = crate::context::tokens::count(&breadcrumb);
    let mut evicted = 0;
    let mut tokens_reclaimed = 0usize;
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
            tokens_reclaimed +=
                crate::context::tokens::count(&r.content).saturating_sub(breadcrumb_tokens);
            r.content = breadcrumb.clone();
            evicted += 1;
        }
    }
    (evicted, tokens_reclaimed)
}
```

### 2. Add the `ReadEvicted` variant

In `executor/src/store/sessions/event.rs`, add after `OutputFiltered` (the
phase-03 variant), mirroring its shape:

```rust
    /// Emitted when a successful edit supersedes prior `read_file` results for a
    /// file (M10 Arc B). `reads_evicted` results were replaced by a re-read
    /// breadcrumb; `tokens_reclaimed` is the chars/4 estimate of context freed.
    ReadEvicted {
        path: String,
        reads_evicted: usize,
        tokens_reclaimed: usize,
    },
```

### 3. Call it at the edit site + emit `ReadEvicted` in `executor/src/agent/mod.rs`

Immediately **after** the working-set record block (`mod.rs:684`, the closing `}`
of the `if succeeded && (… read_file … patch …)` block — and before the post-write
format hook that begins at line ~686), add:

```rust
        // Superseded-read eviction (M10 Arc B): a successful edit makes every
        // prior read of this file stale. Replace those read results with a
        // re-read breadcrumb to reclaim context and remove the stale-content
        // hazard. Always safe — the read-before-edit gate forces a re-read.
        if succeeded && let Some(path) = &edit_path {
            let (reads_evicted, tokens_reclaimed) =
                evict_superseded_reads(&mut messages, path, turns, deps.project_root);
            if reads_evicted > 0 {
                log_event(
                    &log_handle,
                    &redactor,
                    deps.clock,
                    turns,
                    SessionEvent::ReadEvicted {
                        path: path.display().to_string(),
                        reads_evicted,
                        tokens_reclaimed,
                    },
                );
            }
        }
```

`log_event`, `log_handle`, `redactor`, `deps.clock`, `turns`, and `SessionEvent`
are all in scope here (the loop logs many events nearby).

Import: `evict_superseded_reads` is `pub(super)` in `tools.rs`; reference it the
same way the call site already references `append_tool_exchange`, `record_mtime`,
`resolve_path` — add it to the existing `use tools::{ … }` import block at the top
of `mod.rs` (lines 48–51, which already bring in `record_mtime`).

### 4. Add the required `ReadEvicted` match arms

Mirror the existing `OutputFiltered` arm in each site — it is the immediate worked
example, added in phase-03, and every line below has an `OutputFiltered` neighbour
you can copy. **Heed phase-03's calibration:** its first dispatch hard-failed
because the spec under-listed `filter.rs`'s per-event-kind sites — `filter.rs`
carries **seven** sites (a const, a struct field, a `Default` field, and four
per-event matches), not three. List below is exhaustive; do all of it.

**(a)** `executor/src/agent/mod.rs` — the **test-helper** `event_kind` match
(inside `#[cfg(test)] mod tests`, ~line 1696, the arm
`SessionEvent::OutputFiltered { .. } => "output_filtered"`). Add after it:

```rust
            SessionEvent::ReadEvicted { .. } => "read_evicted",
```

(This is a test helper, not production — but its match is exhaustive, so the test
crate won't compile without the arm.)

**(b)** `mcp/src/log_query.rs` — the **production** `event_type_str` fn (line 14,
arm at line 28 `SessionEvent::OutputFiltered { .. } => "output_filtered"`):

```rust
        SessionEvent::ReadEvicted { .. } => "read_evicted",
```

(The fixture-based `event_type_str_round_trips_all_variants` test at line ~227 does
**not** enumerate every variant — it walks `fixture_records()`, which has no
`ReadEvicted` record — so it needs **no** change. Leave it alone.)

**(c)** `mcp/src/dashboard/filter.rs` — `read_evicted` is the **13th** event kind
(index `12`); it currently has 12 (`output_filtered` is index `11`). **All seven**
of these edits are required — the first four to compile (`E0063` missing `Default`
field, `E0004` non-exhaustive `allows`), the last three to keep the filter panel in
sync (they compile via `_ =>` but silently desync the 13th toggle/label otherwise):

```rust
    // 1. the count const (line 3): bump 12 → 13
    pub(crate) const FILTER_ITEM_COUNT: usize = 13;

    // 2. struct field (ActivityFilter), after `output_filtered: bool,`:
    pub(crate) read_evicted: bool,

    // 3. Default impl, after `output_filtered: true,`:
    read_evicted: true,

    // 4. allows() match, after the OutputFiltered arm:
    SessionEvent::ReadEvicted { .. } => self.read_evicted,

    // 5. toggle() match, after `11 => self.output_filtered = !self.output_filtered,`:
    12 => self.read_evicted = !self.read_evicted,

    // 6. is_enabled() match, after `11 => self.output_filtered,`:
    12 => self.read_evicted,

    // 7. item_label() match, after `11 => "output filtered",`:
    12 => "read evicted",
```

Also extend the `filter_default_disables_progress` test (after
`assert!(f.output_filtered);`) with `assert!(f.read_evicted);` to mirror the other
kinds.

**(d)** `mcp/src/dashboard/transcript.rs` `record_lines` — a render arm mirroring
the `OutputFiltered` one (right after it, ~line 149). `Color` is already imported
(`OutputFiltered` uses `Color::Cyan`); the tuple shape is `(summary, color, bold,
body)`:

```rust
            SessionEvent::ReadEvicted {
                reads_evicted,
                tokens_reclaimed,
                ..
            } => (
                format!("evicted {reads_evicted} stale read(s): -{tokens_reclaimed} tokens"),
                Color::Cyan,
                false,
                None,
            ),
```

Do **not** add a `status.rs` summarize arm (the `_ => {}` catch-all is correct;
the `StatusSummary` fold + scorecard aggregation is phase-07).

## Acceptance criteria

- [ ] `grep -n 'pub(super) fn evict_superseded_reads' executor/src/agent/tools.rs` matches.
- [ ] `grep -n 'evict_superseded_reads' executor/src/agent/mod.rs` matches (the call site).
- [ ] `grep -n 'ReadEvicted' executor/src/store/sessions/event.rs` matches the new variant.
- [ ] After a successful `patch`/`write_file` of `foo`, every prior `read_file`
      result for `foo` in the message history has its content replaced by a
      breadcrumb starting `[superseded:`.
- [ ] `evict_superseded_reads` returns `(reads_evicted, tokens_reclaimed)` with
      `tokens_reclaimed > 0` when a non-trivial read is evicted.
- [ ] A `read_file` result for a **different** file is left untouched, and the
      return is `(0, 0)`. **(negative)**
- [ ] A non-`read_file` tool result (e.g. a `bash` result) is never evicted, even
      across the same turn. **(negative)**
- [ ] Eviction is idempotent: running it twice (e.g. editing the same file twice)
      does not stack breadcrumbs or change the count on the second run. **(negative)**
- [ ] The breadcrumb text mentions the turn number and instructs a `read_file`
      re-read.
- [ ] In an agent-loop run that reads `foo` then patches `foo`, the model call
      *after* the patch carries the breadcrumb in place of the original read
      content (verified via `client.calls()`), **and** a `ReadEvicted` event is
      logged with `reads_evicted >= 1`.
- [ ] An edit that supersedes no prior read logs **no** `ReadEvicted` event.
      **(negative)**
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
  → returns `(1, n)` with `n > 0`, the tool result content starts with
  `[superseded:` and no longer contains "OLD CONTENT".

- `evict_superseded_reads_resolves_relative_read_path` — the read call's `path`
  arg is relative (`"foo.rs"`) and `project_root` is `/r`; editing `/r/foo.rs`
  → the read is evicted (confirms the relative-join resolution).

- `evict_superseded_reads_leaves_other_files_untouched` — read of `/r/bar.rs`,
  edit `/r/foo.rs` → returns `(0, 0)`, the `bar.rs` read content is unchanged.
  **(negative case)**

- `evict_superseded_reads_ignores_non_read_results` — a `bash` exchange (assistant
  bash call + tool result whose `tool_name == "bash"`) for a command string that
  has no bearing → returns `(0, 0)`, the bash result content is unchanged.
  **(negative)**

- `evict_superseded_reads_is_idempotent` — run eviction twice for the same edited
  path → first run returns `(1, _)`, second run returns `(0, 0)`, and the content
  is a single breadcrumb (does not contain a nested/second `[superseded:`).
  **(negative)**

- `evict_superseded_reads_evicts_multiple_prior_reads` — two separate
  `read_file(/r/foo.rs)` exchanges in the history → returns `(2, _)`, both
  contents are breadcrumbs.

- `evict_superseded_reads_breadcrumb_mentions_turn_and_reread` — breadcrumb
  contains `"turn 7"` and `"read_file"`.

Agent-loop integration test in `executor/src/agent/mod.rs`'s existing
`#[cfg(test)] mod tests` (mirror `patch_after_reading_is_allowed`, `mod.rs:2138`,
which uses `MockFileVerifier::new(vec![])` + `run_with_verifier(&dir, &client,
&verifier, 8)`; the log-read helper is `records(dir.path())`, the same one
phase-03's `loop_emits_output_filtered_event_for_filtered_bash` uses):

- `loop_evicts_prior_read_after_patch` — script the model to (turn 1)
  `read_file(foo)`, (turn 2) `patch(foo, …)`, (turn 3) `token("done")`. After the
  run, assert that `client.calls()[2].messages` (the messages sent on the call
  after the patch) contains a `read_file` tool result whose content starts with
  `[superseded:`, and does **not** contain the original file content string.
  Use `run_with_verifier(&dir, &client, &verifier, 8)` with a clean
  `MockFileVerifier`, exactly as `patch_after_reading_is_allowed` does.

- `loop_logs_read_evicted_event_after_patch` — same script; assert a
  `ReadEvicted` record (with `reads_evicted >= 1`) is in the session log. Mirror
  whichever existing loop test asserts a logged event (the `Compaction`/`Metrics`
  event tests — find one and reuse its log-read path).

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
  compaction*) — that is phase-06 (Arc B item 3). This phase evicts eagerly at
  edit time; it does **not** touch `context/compactor.rs`, `TARGET_FRACTION`, or
  the compaction passes.
- **Redundant re-read dedupe** (returning "unchanged since turn N" when the model
  re-reads an unchanged file) — that is phase-05 (Arc B item 2). Do not modify
  `read_file` or the read path.
- **A config kill-switch.** Eviction is always-safe (breadcrumb + read-before-edit
  gate guarantee recovery), and a switch would require threading config through the
  loop's `LoopDeps` — a wide-blast-radius change out of proportion to a one-line
  call site. Deferred; revisit only if a real need appears.
- **A `PhaseRun` metric / scorecard field** for evictions — phase-07 aggregates the
  `ReadEvicted` (and `OutputFiltered`) events from the session JSONL onto
  `PhaseRun` *with* its scorecard reader. This phase emits the durable event; it
  does **not** add a `RunMetrics`/`PhaseRun` field whose reader does not yet exist.
- **A `status.rs` `StatusSummary` fold / dashboard summary panel** for evictions —
  also phase-07. This phase emits + renders the transcript line only.
- **Evicting reads of files edited via shell `bash` (e.g. `sed -i`).** Only
  `write_file`/`patch` (the tracked edit-class calls, `edit_path.is_some()`)
  trigger eviction — bash-side edits are not tracked by the working set and are
  out of scope.
- **Touching the read-before-edit gate, the working set's mtime logic, or the
  baseline/verifier flow.** Eviction is additive alongside them.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
