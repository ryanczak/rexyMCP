# Phase 06: redundant-read dedupe

**Milestone:** M10 — Context optimization
**Status:** done
**Depends on:** phase-04 (the superseded-read eviction this phase composes with — a deduped reference must never point at an already-evicted read) and phase-03 (the per-lever reclaim-event pattern this phase mirrors for `ReadDeduped`). Arc A (phase-01/02) complete. Second Arc B *behavior* lever.
**Estimated diff:** ~260 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

When the model re-reads a file it already read this session, and the file has not
changed on disk, re-injecting the full content is pure waste — it bloats context
and is the exact behaviour that drives the `IdenticalToolCallRepetition` stall
(`governor/hard_fail.rs`, `IDENTICAL_CALL_THRESHOLD = 3`). This phase makes a
`read_file` of an **unchanged** file whose content is **still live in context**
return a compact `[already-read: unchanged since turn N …]` reference instead of
the full file body. The model is told the content is already above it and how to
force a real re-read if it truly needs one. The reclaim is recorded as a per-lever
`SessionEvent::ReadDeduped` (mirroring phase-03's `OutputFiltered` and phase-04's
`ReadEvicted`), so the win is measurable on the live dashboard and aggregatable
onto `PhaseRun` in phase-08.

This is the second Arc B (semantic context-lifecycle) lever and it is **safe by
construction** — it declines unless two independent conditions both hold:

1. the file's current on-disk mtime equals the mtime recorded in the working set
   when it was last read (no untracked edit since), **and**
2. a prior **whole-file** `read_file` result for that path is still present in the
   message history and still carries real content (not a `[superseded:`
   eviction breadcrumb from phase-04, not an `[already-read:` reference from this
   phase).

If either fails, the real read happens. And a ranged read (`start_line`/
`end_line`) or an explicit `force: true` always bypasses dedupe entirely, so the
model can never be denied content it asks for.

## Architecture references

Read before starting:

- `executor/src/agent/tools.rs` — the working-set helpers
  (`read_before_edit_refusal`, `record_mtime`, `resolve_path`,
  `evict_superseded_reads`) and the message shape. The two new functions live in
  this file alongside them. `read_before_edit_refusal` (lines 48–75) is the
  worked example for "a decision function that does its own mtime stat";
  `evict_superseded_reads` (lines 102–151) is the worked example for "scan the
  message history for `read_file` exchanges by path."
- `executor/src/agent/mod.rs` lines 585–707 — the dispatch call site, the
  read-before-edit gate match, the `OutputFiltered`/`ReadEvicted` emit blocks, and
  the working-set record block. The dedupe short-circuit and emit go here.
- `docs/dev/milestones/M10-context-optimization/README.md` §"What is novel to
  rexyMCP (Arc B)" item 2 (redundant re-read awareness) — the design intent.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).
5. Run `cargo test` and record the passing count — the completion log must show
   the same count plus the new tests.

## Current state

**Message shape (identical to phase-04).** A `read_file` exchange is two
consecutive messages appended by `append_tool_exchange`
(`executor/src/agent/tools.rs:195`): an **assistant** message carrying the
`read_file` tool call (the path, and any `start_line`/`end_line`, live in its
JSON-string `arguments`), immediately followed by a **tool** message carrying the
result `content` in `tool_results[0]`. The **path** is on the assistant message
(`tool_calls[0].arguments`, a JSON string); the **content** is on the *next*
message (`tool_results[0].content`, `tool_name == "read_file"`). They are always
adjacent and in this order. `evict_superseded_reads` (tools.rs:102) already walks
this exact pairing — mirror its scan.

**The working set** is `HashMap<PathBuf, SystemTime>` (`agent/mod.rs`, threaded as
`working_set`). `record_mtime` (tools.rs:79) inserts a file's current mtime when
it is read or successfully patched. `read_before_edit_refusal` (tools.rs:48) shows
the canonical "compare working-set mtime to a fresh `std::fs::metadata(&path)…
modified()` stat" idiom — quote it; the dedupe mtime gate is the same shape:

```rust
        Some(recorded) => {
            let current = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            match current {
                Some(now) if now == *recorded => None,  // unchanged
                _ => Some(/* changed */),
            }
        }
```

**Why scanning the live `messages` vec is the correct "still in context" test.**
The compactor (`context/compactor.rs`) frees tokens two ways: pass 1
**signaturizes** `<tool_result>`-wrapped *`role:"user"`* messages (the text shape,
*not* the structured `tool_results` that `append_tool_exchange` produces), and
pass 2 **evicts** whole non-system messages with `Vec::remove`. So a structured
`read_file` result that compaction decided to drop is *gone from `messages`
entirely* — a scan of `messages` won't find it. If the scan *does* find a live,
non-breadcrumb read result, that content is genuinely still being sent to the
model on the next turn. This is what makes "you already have it above" a true
statement. (Phase-04's eviction replaces the content in place with a `[superseded:`
breadcrumb; the dedupe scan must skip those — see Task 1.)

**The call site** (`executor/src/agent/mod.rs`). The relevant region, lines
585–630, is the edit-path resolution + the read-before-edit gate match that
produces `(succeeded, content, tool_meta)`:

```rust
        let edit_path = edit_target(&tool_call, deps.project_root);

        // Step 4.5 — read-before-edit gate (07d). …
        let (succeeded, content, tool_meta) =
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal, None),
                None => {
                    // … baseline + pre-edit capture + emit_progress …
                    dispatch(deps.registry, &tool_call).await
                }
            };
```

`read_file` is never gated by `read_before_edit_refusal` (that returns `None` for
any non-`patch` call), so the dedupe short-circuit is independent of it. After the
match, `append_tool_exchange` (line 652) appends the exchange, then the
`OutputFiltered` block (654–676), the working-set `record_mtime` block (678–685),
and the `ReadEvicted` block (687–707). `turns`, `deps.project_root`,
`working_set`, `messages`, `log_event`, `log_handle`, `redactor`, `deps.clock`,
and `SessionEvent` are all in scope here.

**The per-lever event blast radius (the wall — read this twice).** Adding a
`SessionEvent` variant has bitten this milestone twice (phase-03 dispatch-1,
phase-04 dispatch-1), both `VerifierFailurePersistent` on a **partial
`filter.rs`**. The complete, grep-verified set of sites that need a new arm —
`read_deduped` is the **14th** event kind (index `13`); there are currently 13
(`read_evicted` is index `12`, `FILTER_ITEM_COUNT == 13`):

- `mcp/src/log_query.rs` `event_type_str` (line 14) — the **production**
  kind-string fn. One arm. (The fixture-based round-trip test does **not**
  enumerate every variant — it walks `fixture_records()`, which has no
  `ReadDeduped` record — so it needs **no** change.)
- `mcp/src/dashboard/filter.rs` — **seven** sites: `FILTER_ITEM_COUNT` (line 3),
  the `ActivityFilter` struct field (after `read_evicted`, line 21), its `Default`
  (after `read_evicted: true`, line 39), the `allows()` arm (after the
  `ReadEvicted` arm, line 59), and the `toggle()` / `is_enabled()` / `item_label()`
  index-`13` arms (after the index-`12` `read_evicted` arms, lines 77/96/115) — plus
  the `filter_default_disables_progress` test assertion (after
  `assert!(f.read_evicted);`, line 162).
- `mcp/src/dashboard/transcript.rs` `record_lines` — one render arm, after the
  `ReadEvicted` arm (line 159).
- `executor/src/agent/mod.rs` test-helper `event_kind` (inside `#[cfg(test)] mod
  tests`, line 1714, after `SessionEvent::ReadEvicted { .. } => "read_evicted"`,
  line 1729) — an exhaustive match, so the **test** crate won't compile without it.

The `status.rs` `_ => {}`, `cap.rs` catch-all, and generic-serde sites
(`agent/log.rs`, `store/sessions/jsonl.rs`, `event.rs` derives) need **no** change.
Phase-04's `ReadEvicted` arms (already on the tree) are the immediate worked
example to copy at every site above.

## Spec

Numbered tasks in execution order.

### 1. Add `last_live_read` + `redundant_read_reference` to `executor/src/agent/tools.rs`

Add both near the other working-set helpers (after `evict_superseded_reads`).
Match the **behaviour** pinned by the Test plan; minor structural choices are
yours.

```rust
/// Stable marker prefix for a redundant-read dedupe reference. Used to build the
/// reference and to detect an already-deduped result so a second re-read points
/// at the original live read, never at another reference.
const REDUNDANT_READ_PREFIX: &str = "[already-read:";

/// Find the most recent **live whole-file** `read_file` result for `path` in
/// `messages`. "Live" = the result content is still real — not a phase-04
/// `[superseded:` eviction breadcrumb and not an `[already-read:` reference from a
/// prior dedupe. "Whole-file" = the read call carried no `start_line`/`end_line`
/// (a ranged prior read does not cover the whole file, so it can't stand in for a
/// whole-file re-read). Returns `(turn, content_tokens)` of that prior read —
/// `turn` for the breadcrumb, `content_tokens` (chars/4 estimate, same heuristic
/// as the budget) for the `tokens_saved` calculation — or `None` if no such read
/// is still in context.
///
/// Pure over the slice (no filesystem); the mtime/on-disk check is the caller's
/// (`redundant_read_reference`). Scanning the live `messages` vec *is* the
/// "still in context" test: a read the compactor evicted is removed from
/// `messages`, so it won't be found here.
pub(super) fn last_live_read(
    messages: &[Message],
    path: &Path,
    project_root: &Path,
) -> Option<(usize, usize)> {
    let mut found: Option<(usize, usize)> = None;
    for i in 0..messages.len() {
        // messages[i] must be an assistant whole-file `read_file` call for `path`.
        let args = messages[i]
            .tool_calls
            .as_ref()
            .and_then(|tcs| tcs.first())
            .filter(|tc| tc.name == "read_file")
            .and_then(|tc| serde_json::from_str::<serde_json::Value>(&tc.arguments).ok());
        let Some(args) = args else { continue };
        if args.get("start_line").is_some() || args.get("end_line").is_some() {
            continue; // ranged prior read — does not cover the whole file
        }
        let matches_path = args
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| {
                let pb = std::path::PathBuf::from(p);
                let resolved = if pb.is_absolute() { pb } else { project_root.join(pb) };
                resolved == path
            })
            .unwrap_or(false);
        if !matches_path {
            continue;
        }
        // The result is on the next (tool) message; it must still be real content.
        if let Some(next) = messages.get(i + 1)
            && let Some(results) = next.tool_results.as_ref()
            && let Some(r) = results.first()
            && r.tool_name == "read_file"
            && !r.content.starts_with(SUPERSEDED_PREFIX)
            && !r.content.starts_with(REDUNDANT_READ_PREFIX)
        {
            let turn = messages[i].turn.unwrap_or(0);
            found = Some((turn, crate::context::tokens::count(&r.content)));
        }
    }
    found
}

/// Decide whether a `read_file` call is a redundant re-read of an unchanged file
/// the model already has in context. Returns `(reference, tokens_saved,
/// prior_turn)` when so — the caller feeds `reference` back as the tool result and
/// emits a `ReadDeduped` event — or `None` to perform the real read.
///
/// Declines (real read) when: the call is ranged (`start_line`/`end_line`) or
/// `force: true` (the model's escape hatches); the file is not in the working set
/// (never read this session); the on-disk mtime no longer matches the recorded
/// one (changed since — incl. untracked `bash` edits); or no live whole-file prior
/// read survives in `messages`. Does its own mtime stat, mirroring
/// `read_before_edit_refusal`, so it is exercised by the loop integration tests.
pub(super) fn redundant_read_reference(
    tool_call: &ToolCall,
    messages: &[Message],
    working_set: &HashMap<PathBuf, SystemTime>,
    project_root: &Path,
) -> Option<(String, usize, usize)> {
    if tool_call.name != "read_file" {
        return None;
    }
    let args = &tool_call.arguments;
    if args.get("start_line").is_some()
        || args.get("end_line").is_some()
        || args.get("force").and_then(|v| v.as_bool()) == Some(true)
    {
        return None;
    }
    let path = resolve_path(tool_call, project_root)?;
    let recorded = working_set.get(&path)?;
    let current = std::fs::metadata(&path).ok().and_then(|m| m.modified().ok())?;
    if current != *recorded {
        return None;
    }
    let (prior_turn, prior_tokens) = last_live_read(messages, &path, project_root)?;
    let reference = format!(
        "{REDUNDANT_READ_PREFIX} unchanged since your read at turn {prior_turn}; that content \
         is still above in this conversation — re-read with start_line/end_line or force:true \
         only if you need it again]"
    );
    let tokens_saved = prior_tokens.saturating_sub(crate::context::tokens::count(&reference));
    Some((reference, tokens_saved, prior_turn))
}
```

`SUPERSEDED_PREFIX` (tools.rs:87), `resolve_path`, `tokens::count`, `Message`,
`ToolCall`, `HashMap`, `PathBuf`, `SystemTime`, `Path` are all already in scope in
`tools.rs`. No new imports beyond what `evict_superseded_reads` already uses.

### 2. Advertise the `force` escape hatch in the `read_file` schema

In `executor/src/tools/read_file.rs`, add a `force` property to the `schema()`
JSON so the model can discover the escape hatch the dedupe reference names. The
tool itself ignores it (`ReadFileArgs` does not `deny_unknown_fields`, and the
loop — not the tool — reads `force`), so this is documentation only. After the
`end_line` property in the `properties` object:

```rust
                "force": {
                    "type": "boolean",
                    "description": "Set true to force a full re-read even if the file is unchanged since your last read this session (bypasses redundant-read dedupe)."
                }
```

Do **not** add `force` to the `ReadFileArgs` struct and do **not** change the
tool's read behaviour — `force` is honoured by the loop's dedupe gate, not by the
tool. (Adding the unused field to `ReadFileArgs` would be a dead field clippy
flags.)

### 3. Add the `ReadDeduped` variant

In `executor/src/store/sessions/event.rs`, add after `ReadEvicted` (the phase-04
variant), mirroring its shape:

```rust
    /// Emitted when a `read_file` of an unchanged file was deduped to a compact
    /// reference instead of re-injecting its content (M10 Arc B). `tokens_saved`
    /// is the chars/4 estimate of context avoided; `prior_turn` is the turn whose
    /// read the reference points back at.
    ReadDeduped {
        path: String,
        tokens_saved: usize,
        prior_turn: usize,
    },
```

### 4. Short-circuit the read + emit `ReadDeduped` in `executor/src/agent/mod.rs`

**(a)** Add `redundant_read_reference` to the existing `use tools::{ … }` import
block (lines 48–52, which already brings in `evict_superseded_reads`,
`read_before_edit_refusal`, `record_mtime`, `resolve_path`). `last_live_read` is
called only from inside `redundant_read_reference`, so it is **not** imported here.

**(b)** Immediately **before** the `let (succeeded, content, tool_meta) = match
read_before_edit_refusal(…)` expression (line 593), compute the dedupe decision,
then make the dedupe case short-circuit the dispatch:

```rust
        // Redundant-read dedupe (M10 Arc B): a `read_file` of an unchanged file
        // whose content is still live in context returns a compact reference
        // instead of re-injecting it — reclaims context and attacks the
        // IdenticalToolCallRepetition stall. Safe: declines unless the mtime
        // matches AND a live prior whole-file read survives; ranged / force:true
        // reads always fall through to a real read.
        let dedupe =
            redundant_read_reference(&tool_call, &messages, &working_set, deps.project_root);

        let (succeeded, content, tool_meta) = if let Some((reference, _, _)) = &dedupe {
            (true, reference.clone(), None)
        } else {
            match read_before_edit_refusal(&tool_call, &working_set, deps.project_root) {
                Some(refusal) => (false, refusal, None),
                None => {
                    // … existing baseline + pre-edit capture + emit_progress + dispatch,
                    //     UNCHANGED …
                    dispatch(deps.registry, &tool_call).await
                }
            }
        };
```

Keep the body of the `None =>` arm exactly as it is today (baseline capture,
pre-edit content capture, the `EmitCtx`/`emit_progress` block, and the trailing
`dispatch(...).await`). Only the outer `if let … else { match … }` wrapper is new.

**(c)** After the working-set `record_mtime` block (line 685) and alongside the
`ReadEvicted` emit block (the `ReadDeduped` emit may go immediately before or after
it — they are independent), emit the event when a dedupe happened:

```rust
        // Per-lever reclaim event (M10 Arc B): record the deduped re-read.
        if let Some((_, tokens_saved, prior_turn)) = dedupe
            && let Some(path) = resolve_path(&tool_call, deps.project_root)
        {
            log_event(
                &log_handle,
                &redactor,
                deps.clock,
                turns,
                SessionEvent::ReadDeduped {
                    path: path.display().to_string(),
                    tokens_saved,
                    prior_turn,
                },
            );
        }
```

A deduped read has `succeeded == true` and `name == "read_file"`, so the existing
`record_mtime` block (line 678) still runs and re-records the (unchanged) mtime —
harmless and idempotent. `edit_path` is `None` for a read, so the eviction,
post-write-hook, and verify blocks are all skipped, unchanged.

### 5. Add the required `ReadDeduped` match arms

Mirror the `ReadEvicted` arm at each site. **Heed the calibration:** `filter.rs`
carries **seven** sites; a partial `filter.rs` is exactly what hard-failed
phase-03 and phase-04. List below is exhaustive; do **all** of it, then `cargo
build` **before** writing any test.

**(a)** `executor/src/agent/mod.rs` test-helper `event_kind` (line 1714), after
the `ReadEvicted` arm (line 1729):

```rust
            SessionEvent::ReadDeduped { .. } => "read_deduped",
```

**(b)** `mcp/src/log_query.rs` `event_type_str` (line 14), after the `ReadEvicted`
arm (line 29):

```rust
        SessionEvent::ReadDeduped { .. } => "read_deduped",
```

**(c)** `mcp/src/dashboard/filter.rs` — **all seven** edits (`read_deduped` is the
14th kind, index `13`):

```rust
    // 1. the count const (line 3): bump 13 → 14
    pub(crate) const FILTER_ITEM_COUNT: usize = 14;

    // 2. struct field (ActivityFilter), after `read_evicted: bool,` (line 21):
    pub(crate) read_deduped: bool,

    // 3. Default impl, after `read_evicted: true,` (line 39):
    read_deduped: true,

    // 4. allows() match, after the ReadEvicted arm (line 59):
    SessionEvent::ReadDeduped { .. } => self.read_deduped,

    // 5. toggle() match, after `12 => self.read_evicted = !self.read_evicted,` (line 77):
    13 => self.read_deduped = !self.read_deduped,

    // 6. is_enabled() match, after `12 => self.read_evicted,` (line 96):
    13 => self.read_deduped,

    // 7. item_label() match, after `12 => "read evicted",` (line 115):
    13 => "read deduped",
```

Also extend the `filter_default_disables_progress` test (after
`assert!(f.read_evicted);`, line 162) with `assert!(f.read_deduped);`.

The first four edits are needed to **compile** (`E0063` missing `Default` field,
`E0004` non-exhaustive `allows`); the last three (`toggle`/`is_enabled`/
`item_label`) compile via the `_ =>` fall-through but silently desync the 14th
toggle/label if omitted — they are required.

**(d)** `mcp/src/dashboard/transcript.rs` `record_lines` — a render arm mirroring
the `ReadEvicted` one (right after it, line 159). `Color` is already imported; the
tuple shape is `(summary, color, bold, body)`:

```rust
            SessionEvent::ReadDeduped {
                tokens_saved,
                prior_turn,
                ..
            } => (
                format!("deduped re-read (already read turn {prior_turn}): -{tokens_saved} tokens"),
                Color::Cyan,
                false,
                None,
            ),
```

Do **not** add a `status.rs` summarize arm (the `_ => {}` catch-all is correct;
the `StatusSummary` fold + scorecard aggregation is phase-08).

## Acceptance criteria

- [ ] `grep -n 'pub(super) fn last_live_read' executor/src/agent/tools.rs` matches.
- [ ] `grep -n 'pub(super) fn redundant_read_reference' executor/src/agent/tools.rs` matches.
- [ ] `grep -n 'ReadDeduped' executor/src/store/sessions/event.rs` matches the new variant.
- [ ] A second whole-file `read_file` of a file unchanged since its first read,
      with that first read still in the message history, returns a tool result
      whose content starts with `[already-read:` (not the file body), and a
      `ReadDeduped` event is logged.
- [ ] The dedupe reference text mentions the prior turn number and names both
      escape hatches (`start_line`/`end_line` and `force`).
- [ ] A **ranged** re-read (`start_line`/`end_line` present) is **not** deduped —
      it performs a real read and logs no `ReadDeduped`. **(negative)**
- [ ] A re-read with `force: true` is **not** deduped. **(negative)**
- [ ] A re-read after the file changed on disk (working-set mtime ≠ current mtime)
      is **not** deduped. **(negative)**
- [ ] A re-read whose only prior read was superseded-evicted (phase-04 breadcrumb)
      or already-deduped is **not** deduped — no live read to point at, so a real
      read happens. **(negative)**
- [ ] The first read of a file (no working-set entry) is never deduped. **(negative)**
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Unit tests in the `#[cfg(test)] mod tests` block at the bottom of
`executor/src/agent/tools.rs`. Reuse the existing `make_read_exchange` /
`make_bash_exchange` helpers (tools.rs:322/351); add a small helper for a
**ranged** read exchange and one for a `[superseded:`/`[already-read:` result if
convenient. `last_live_read` is pure (no fs). `redundant_read_reference` does an
mtime stat — drive it with a `tempfile::TempDir` file plus a `working_set`
populated by `record_mtime` (hermetic; a `TempDir` is allowed).

`last_live_read` (pure):

- `last_live_read_finds_prior_whole_file_read` — messages = read-exchange
  (`/r/foo.rs`, sizable content) → `Some((turn, tokens))` with `tokens > 0`.
- `last_live_read_resolves_relative_path` — read call path `"foo.rs"`, query
  `/r/foo.rs`, root `/r` → `Some(_)`.
- `last_live_read_skips_superseded_breadcrumb` — the read result content starts
  with `[superseded:` → `None`. **(negative)**
- `last_live_read_skips_already_read_reference` — content starts with
  `[already-read:` → `None`. **(negative, idempotence)**
- `last_live_read_ignores_ranged_prior_read` — the read call's args carry
  `start_line` → `None` (a ranged read doesn't cover the whole file). **(negative)**
- `last_live_read_ignores_other_file` — read of `/r/bar.rs`, query `/r/foo.rs` →
  `None`. **(negative)**
- `last_live_read_returns_latest_of_multiple` — two whole-file reads of
  `/r/foo.rs` at turns 1 and 4 → returns the later `turn` (4).

`redundant_read_reference` (TempDir + working_set):

- `redundant_read_reference_dedupes_unchanged_reread` — write a sizable file,
  `record_mtime` it into the working set, messages hold a live whole-file read of
  it → `Some((reference, tokens_saved, prior_turn))` with `reference` starting
  `[already-read:` and `tokens_saved > 0`.
- `redundant_read_reference_skips_when_mtime_differs` — insert a deliberately
  wrong recorded mtime for the path (e.g. `SystemTime::UNIX_EPOCH`) into the
  working set so it cannot equal the file's real mtime → `None`. **(negative; the
  mtime gate — deterministic, no sleep/timing.)**
- `redundant_read_reference_skips_ranged_read` — the `read_file` `ToolCall` carries
  `start_line` → `None`. **(negative)**
- `redundant_read_reference_skips_force_read` — the `ToolCall` carries
  `force: true` → `None`. **(negative)**
- `redundant_read_reference_none_when_not_in_working_set` — empty working set →
  `None` (first read). **(negative)**
- `redundant_read_reference_none_when_no_live_prior_read` — working set has the
  path with the correct mtime, but the only prior read result is a `[superseded:`
  breadcrumb → `None` (nothing live to point at). **(negative — the live-content
  safety gate.)**

Agent-loop integration tests in `executor/src/agent/mod.rs`'s `#[cfg(test)] mod
tests`. Mirror `loop_logs_read_evicted_event_after_patch` (mod.rs:2236):
`MockAiClientScript::new(vec![ … ])` with `native("read_file", json!({…}))` /
`native("write_file", …)` / `token("done")`, `run_with_verifier(&dir, &client,
&verifier, 8)`, `MockFileVerifier::new(vec![])`, records via `records(dir.path())`,
messages via `client.calls()[n].messages`.

- `loop_dedupes_unchanged_reread` — script: turn 1 `read_file(foo)`, turn 2
  `read_file(foo)` (same whole-file), turn 3 `token("done")`. Assert
  `client.calls()[2].messages` (sent after the second read) contains a `read_file`
  tool result whose content starts with `[already-read:`. (Write `foo` with content
  large enough that `tokens_saved > 0`.)
- `loop_logs_read_deduped_event` — same script; assert `records(dir.path())`
  contains a `ReadDeduped` event.
- `loop_does_not_dedupe_after_edit` — script: turn 1 `read_file(foo)`, turn 2
  `write_file(foo, "new content …")` (changes mtime; refreshes the working set;
  phase-04 supersedes the prior read), turn 3 `read_file(foo)`, turn 4
  `token("done")`. Assert the turn-3 read is a **real** read — the `read_file` tool
  result in `client.calls()[3].messages` does **not** start with `[already-read:`
  — and that **no** `ReadDeduped` event was logged. **(negative — the safety
  interaction with phase-04.)**

## End-to-end verification

This phase ships no runtime-loadable real artifact (no CLI surface, no config the
binary loads) — it is internal agent-loop behaviour. Per WORKFLOW.md, the
agent-loop integration test `loop_dedupes_unchanged_reread` using the real
`run_loop` over a `MockAiClient` + a real `TempDir` file **is** the end-to-end
exercise of the shipped behaviour: it runs the actual loop, reads a real on-disk
file twice, and observes the `[already-read:` reference in what the loop sends the
model on the next turn. In the completion log, run:

`cargo test loop_dedupes_unchanged_reread -- --nocapture`

and quote the assertion outcome (pass) and, if practical, the `[already-read:`
reference string observed in `client.calls()[2].messages`.

## Authorizations

None. No new dependency (`serde_json` is already used throughout `tools.rs`). No
`docs/architecture.md` change. No `Cargo.toml` change.

## Out of scope

- **Touching the governor / hard-fail thresholds** (`RUNAWAY_OUTPUT_BYTES`,
  `IDENTICAL_CALL_THRESHOLD`). Dedupe changes only the *content* a re-read returns;
  `succeeded` and the recorded tool-call snapshot are unchanged, so the
  `IdenticalToolCallRepetition` governor behaves exactly as before. M10 reduces the
  *pressure* that leads to the stall; it does not widen the governor
  (README §"Out of scope").
- **A config kill-switch.** Dedupe is always-safe (mtime gate + live-content gate +
  ranged/`force` escape hatches), and a switch would require threading config
  through `LoopDeps` — a wide-blast-radius change out of proportion to the call
  site. Deferred (same call phase-04 made for eviction); revisit only if a real
  need appears.
- **Changing `read_file`'s read behaviour, range semantics, or the 500-line cap.**
  Task 2 adds only a `force` *schema property* for discoverability; the tool's
  `execute` is unchanged and `force` is honoured by the loop, not the tool. Do not
  touch the security/scope boundary.
- **Content-aware compaction priority** (ranking reads during compaction) — that is
  phase-07. This phase does not touch `context/compactor.rs`.
- **A `PhaseRun` metric / scorecard field** for dedupes — phase-08 aggregates the
  `ReadDeduped` (and `OutputFiltered`/`ReadEvicted`) events from the session JSONL
  onto `PhaseRun` *with* its reader. This phase emits the durable event only; it
  does **not** add a `RunMetrics`/`PhaseRun` field whose consumer does not yet
  exist (WORKFLOW.md §"Derive intentionally").
- **A `status.rs` `StatusSummary` fold / dashboard summary panel** for dedupes —
  also phase-08. This phase emits + renders the transcript line only.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-06-08 (progress, executor-dispatch-2)

**Tests-only re-dispatch (executor, Qwen/Qwen3.6-27B-FP8):** implementing the ~13 unit tests in `tools.rs` and 3 agent-loop integration tests in `mod.rs` per the Test plan. Production code is already committed and compiles clean.

### Update — 2026-06-08 (complete)

**Executor:** Qwen/Qwen3.6-27B-FP8

**Tests added:** 13 unit tests in `executor/src/agent/tools.rs` (7 `last_live_read_*` pure tests + 6 `redundant_read_reference_*` TempDir tests) and 3 agent-loop integration tests in `executor/src/agent/mod.rs` (`loop_dedupes_unchanged_reread`, `loop_logs_read_deduped_event`, `loop_does_not_dedupe_after_edit`).

**Verification commands:**
- `cargo fmt --all --check` — clean
- `cargo build` — clean
- `cargo clippy --all-targets --all-features -- -D warnings` — clean
- `cargo test` — 644 passed, 0 failed, 2 ignored (pre-existing 628 executor + 243 mcp = 871 total; new count is 644 executor + 243 mcp = 887, so 16 new tests landed)
- `cargo test loop_dedupes_unchanged_reread -- --nocapture` — passed; `[already-read:` reference observed in `client.calls()[2].messages`

**End-to-end verification:** `loop_dedupes_unchanged_reread` runs the real `run_loop` over `MockAiClient` + a real `TempDir` file, reads it twice, and asserts the `[already-read:` reference appears in the second-turn tool result. Passed.

**Grep verification:**
```
$ grep -n 'pub(super) fn last_live_read' executor/src/agent/tools.rs
525:pub(super) fn last_live_read(
$ grep -n 'pub(super) fn redundant_read_reference' executor/src/agent/tools.rs
572:pub(super) fn redundant_read_reference(
```

**Files changed:** `executor/src/agent/tools.rs` (13 new unit tests), `executor/src/agent/mod.rs` (3 new integration tests), `docs/dev/milestones/M10-context-optimization/phase-06-redundant-read-dedupe.md` (status flip + Update Log), `docs/dev/milestones/M10-context-optimization/README.md` (phase table status flip).

**Notes for review:** None — all tests pass, all gates clean.

### Update — 2026-06-08 (progress, architect)

**Dispatch-1 (executor, Qwen/Qwen3.6-27B-FP8, 41 turns):** the executor wrote
**all the novel production code correctly** — `last_live_read` +
`redundant_read_reference` in `tools.rs`, the dedupe short-circuit + `ReadDeduped`
emit in `mod.rs`, the `ReadDeduped` variant in `event.rs`, the `force` schema
property in `read_file.rs`, and 4 of the 7 `filter.rs` sites. It then **falsely
reported `complete`** with the build broken: it omitted 6 mechanical exhaustive-
match arms (the `filter.rs` `toggle`/`is_enabled`/`item_label` index-13 arms + the
test assertion, `log_query::event_type_str`, `transcript::record_lines`, and the
`mod.rs` test `event_kind` helper) **and wrote none of the ~16 specified tests.**

This is the **3rd occurrence** of the `filter.rs` partial-exhaustive-match wall
(phase-03 dispatch-1, phase-04 dispatch-1, phase-06 dispatch-1) — the WORKFLOW.md
§Calibration fold trigger. **User decision (2026-06-08): hold — no process fold
yet**; the compile-first-then-test re-dispatch checklist remains the mitigation.

**Architect closeout (production code only):** applied the 6 missing match arms,
ran `rustfmt` on the one phase-touched file with format drift (`tools.rs`), and
confirmed the gates: `cargo build` clean, `cargo clippy … -D warnings` clean,
`cargo fmt --all --check` clean, `cargo test` green (628 executor + 243 mcp — the
**pre-existing** counts, since no new tests landed yet). Committed the production
code as a checkpoint. **User decision (2026-06-08): re-dispatch a tests-only
scope** rather than have the architect write the test half.

#### Notes for executor (tests-only re-dispatch)

**The production code is COMPLETE and committed. Do NOT modify any non-test code.**
Specifically, do not touch the bodies of `last_live_read`,
`redundant_read_reference` (`executor/src/agent/tools.rs`), the dedupe block or
emit in `executor/src/agent/mod.rs`, `event.rs`, `read_file.rs`, or any of the
`mcp/` match arms — they compile and pass clippy/fmt. Your **only** job is to add
the tests the §"Test plan" specifies:

1. The ~13 unit tests in the `#[cfg(test)] mod tests` block at the bottom of
   `executor/src/agent/tools.rs` (the `last_live_read_*` pure tests and the
   `redundant_read_reference_*` `TempDir`+`working_set` tests) — every test is
   named and behavior-pinned in the Test plan.
2. The 3 agent-loop integration tests in `executor/src/agent/mod.rs`'s
   `#[cfg(test)] mod tests` — `loop_dedupes_unchanged_reread`,
   `loop_logs_read_deduped_event`, `loop_does_not_dedupe_after_edit` — mirroring
   `loop_logs_read_evicted_event_after_patch` (mod.rs:2236 region) and
   `loop_does_not_evict_*` for the negative.

Reuse the existing `make_read_exchange` / `make_bash_exchange` helpers
(tools.rs:322/351). Hermetic only — a `tempfile::TempDir` is allowed; no real
network, no `sleep`, no unseeded RNG. Drive the mtime gate deterministically
(insert a wrong recorded mtime, e.g. `SystemTime::UNIX_EPOCH`, for the
`*_skips_when_mtime_differs` case — never sleep to age a file).

**Ordering:** the code already compiles, so run `cargo test` after each test
batch; finish with the full gate set (`fmt --all --check`, `clippy … -D
warnings`, `build`, `test`) and the E2E `cargo test loop_dedupes_unchanged_reread
-- --nocapture`. The completion log's test count must be the current 628 executor
plus your new tests. Then fill the Update Log completion entry and commit.

### Review verdict — 2026-06-08

- **Verdict:** approved_after_1
- **Bounces:** 1 (no bug doc filed — dispatch-1 false-reported `complete` with a
  broken build; resolved by architect closeout + a tests-only re-dispatch rather
  than a bug-report cycle)
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Scope deviations:** none — shipped exactly the spec (the two `tools.rs`
  helpers, the `mod.rs` short-circuit + `ReadDeduped` emit, the `event.rs`
  variant, the `force` schema-only property, all match arms, and the full
  16-test plan).
- **Independent re-verification:** `cargo build` clean (zero new warnings),
  `cargo clippy --all-targets --all-features -- -D warnings` clean, `cargo fmt
  --all --check` clean, `cargo test` green (**644 executor + 243 mcp**; +16 from
  phase-05's 628). E2E `loop_dedupes_unchanged_reread` passes and was
  **mutation-checked**: neutralizing the dedupe short-circuit makes it fail on
  `second read of unchanged file should return an [already-read: reference`,
  proving the test exercises the real path. Production diff carries no
  `unwrap`/`expect`/`panic`/`unsafe`/`#[allow]`/`#[ignore]`/`TODO`/`dbg!`/
  `println!`. All 12 acceptance criteria verified (incl. all 6 negative gates:
  ranged, `force:true`, changed-mtime, superseded-only-prior, already-deduped,
  first-read).
- **Calibration:** **3rd occurrence** of the `filter.rs` partial-exhaustive-match
  wall (phase-03/04/06 dispatch-1) — the WORKFLOW.md §Calibration fold trigger.
  **User held the fold (2026-06-08);** the compile-first-then-test re-dispatch
  checklist + architect closeout of the mechanical arms remains the mitigation.
  **New sub-pattern this dispatch:** the executor false-reported `complete` with a
  broken build *and* skipped the entire specified test suite — a harder miss than
  the prior two `VerifierFailurePersistent` bounces (which at least failed
  loudly). Tracked in NEXT.md; watching for a 4th occurrence (of either the
  match-arm wall or the false-complete/tests-skipped pattern) to revisit the fold
  with the user. Counter-note: the **tests-only re-dispatch against a clean,
  compiling, committed tree was clean** — narrow-scope re-dispatch with a
  Notes-for-executor block is an effective recovery lever.
