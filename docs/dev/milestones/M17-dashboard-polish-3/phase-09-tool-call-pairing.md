# Phase 09: Activity-panel tool-call presentation — glyphs, call/result pairing, merged filter

**Milestone:** M17 — Dashboard Polish (Round 3)
**Status:** done
**Depends on:** phase-05 (extension-detected highlighting — the `record_lines_with_lang` / `path_hint` plumbing this phase extends)
**Estimated diff:** ~230 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Make a tool call and its result read as **one unit** in the Activity panel:

1. **Per-tool glyph** in front of each tool-call header (`📖 read_file`, `⚡ bash`, …)
   for fast visual scanning.
2. **Pair the result under its call** — the `ToolResult` header is indented under
   the preceding `Parsed` call, with a `╰` connector and the redundant tool name
   dropped (the call above already names it). Both keep their own relative
   timestamp and turn marker.
3. **Merge the filter** — the separate `tool result` filter toggle goes away;
   `tool call` now governs **both** `Parsed` and `ToolResult` visibility. A tool
   interaction is atomically shown or hidden, which is also what makes pairing
   well-defined (no "call hidden but result shown" orphan-by-filter case).

This is the last in-scope phase of M17.

## Architecture references

Read before starting:

- `docs/architecture.md` § "Architecture (the big picture)" — the MCP boundary and
  the dashboard's read-only relationship to `SessionRecord`s. This phase is
  **display-only**: no `SessionEvent` change, no loop change, no config, no new
  dependency.
- `mcp/src/dashboard/transcript.rs` — `transcript_lines`, `record_lines`,
  `record_lines_with_lang`. The file you change most.
- `mcp/src/dashboard/filter.rs` — `ActivityFilter`, `FILTER_ITEM_COUNT`. The merge
  is fully contained here.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

### Tool calls render as two independent records today

`transcript.rs::record_lines_with_lang` (the renderer; `record_lines(rec)` is a
zero-extra-arg delegating wrapper added in phase-05) builds a header per event.
The two tool arms today (`mcp/src/dashboard/transcript.rs:104` and `:127`):

```rust
SessionEvent::Parsed { tool_call } => {
    let body = match &tool_call.arguments {
        serde_json::Value::Null => None,
        serde_json::Value::Object(m) if m.is_empty() => None,
        args => {
            let pretty =
                serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
            Some(plain_body_lines(&pretty, Color::Rgb(128, 128, 128)))
        }
    };
    (
        format!("→ call {}", tool_call.name),
        Color::Blue,
        false,
        body,
    )
}
// ...
SessionEvent::ToolResult {
    name,
    succeeded,
    output_preview,
} => {
    let status = if *succeeded { "ok" } else { "FAIL" };
    let color = if *succeeded { Color::Green } else { Color::Red };
    (
        format!("tool {name} [{status}]"),
        color,
        false,
        Some(match path_hint {
            Some(p) => highlighted_body_lines_for(output_preview, Some(p)),
            None => highlighted_body_lines(output_preview),
        }),
    )
}
```

So a `read_file` call currently renders as:

```
[+4s] [t4] → call read_file
           {
             "path": "src/x.rs"
           }
[+5s] [t4] tool read_file [ok]
           1  use std::fmt;
           ...
```

### `transcript_lines` already threads call→result state

`transcript_lines` (`mcp/src/dashboard/transcript.rs:19`) already walks the visible
records carrying the most-recent `read_file` call's `path` forward to its
`ToolResult` so the body is highlighted by file extension, and prepends a
dull-yellow `[+Ns]` timestamp span to each record's **header line only**:

```rust
let base_ts = records.first().map(|r| r.ts).unwrap_or(0);
let mut out: Vec<Line<'static>> = Vec::new();
let mut last_read_path: Option<String> = None;
for r in &visible {
    // Capture the path from a read_file call so the matching result can use it.
    if let SessionEvent::Parsed { tool_call } = &r.event
        && tool_call.name == "read_file"
    {
        last_read_path = tool_call
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    let hint = match &r.event {
        SessionEvent::ToolResult { name, .. } if name == "read_file" => {
            last_read_path.as_deref()
        }
        _ => None,
    };
    let mut lines = if let Some(h) = hint {
        record_lines_with_lang(r, Some(h))
    } else {
        record_lines(r)
    };
    if matches!(&r.event, SessionEvent::ToolResult { name, .. } if name == "read_file") {
        last_read_path = None; // consume it
    }
    // Prepend the relative-timestamp header span (unchanged logic).
    if let Some(header) = lines.first_mut() {
        let mut spans = Vec::with_capacity(header.spans.len() + 1);
        spans.push(Span::styled(
            format!("[{}] ", relative_ts(r.ts, base_ts)),
            Style::new().fg(Color::Rgb(180, 150, 50)),
        ));
        spans.append(&mut header.spans);
        *header = Line::from(spans);
    }
    out.extend(lines);
}
```

This is the **exact shape you generalize**: replace the `read_file`-only
`last_read_path: Option<String>` with a `pending: Option<PendingCall>` that tracks
**any** tool call's name (and the path, for `read_file`), so a result can both
(a) recover the path hint and (b) know it is paired.

### The filter today has a separate `tool_result` toggle

`mcp/src/dashboard/filter.rs`: `FILTER_ITEM_COUNT = 15`; `ActivityFilter` has both a
`tool_call: bool` (index 3) and a `tool_result: bool` (index 5) field. `allows`
routes `Parsed` → `self.tool_call` and `ToolResult` → `self.tool_result`. The
filter-panel UI (`render.rs:188`, `event_loop.rs:67`) is **index-generic** — it
loops `0..FILTER_ITEM_COUNT` and calls `is_enabled(i)` / `item_label(i)` /
`toggle(cursor)`, with **no hardcoded indices**. So removing one item is fully
contained in `filter.rs`; `render.rs` / `event_loop.rs` need no edit.

> The `"tool_result"` strings in `mcp/src/log_query.rs` are the **CLI log-search**
> subsystem's `event_type_str`, a different concern. **Do not touch `log_query.rs`.**

### Tool names (for the glyph map)

The executor's registered tool names — verified from `executor/src/tools/*.rs`
`fn name()` returns — are exactly: `read_file`, `write_file`, `patch`, `bash`,
`search`, `find_files`, `symbols`, `update_task`. There is no other production tool
name. Anything else falls to the default glyph.

## Spec

Numbered tasks in execution order.

1. **Merge `tool_result` into `tool_call` in `mcp/src/dashboard/filter.rs`.** This is
   a multi-site mechanical change inside one file — do all of it, building after, in
   one pass (it is the M10/M12 "index cascade" shape). The complete site list:
   - **Struct:** delete the `pub(crate) tool_result: bool,` field.
   - **`Default`:** delete the `tool_result: true,` line.
   - **`FILTER_ITEM_COUNT`:** `15` → `14`.
   - **`allows`:** change the `ToolResult` arm from `self.tool_result` to
     `self.tool_call`. (It already reads `Parsed { .. } => self.tool_call`; now
     **both** route to `tool_call`.)
   - **`toggle`, `is_enabled`, `item_label`:** these are parallel index→item maps.
     Remove the **index-5 `tool_result` / `"tool result"` arm** from each, and
     **renumber every arm after it down by one** so the indices stay contiguous
     `0..14` with no gap. After the change the order is:
     `0 session · 1 prompt · 2 completion · 3 tool_call · 4 parse_failed ·
     5 verify · 6 hard_fail · 7 progress · 8 metrics · 9 compaction ·
     10 output_filtered · 11 read_evicted · 12 read_deduped · 13 task_update`.
   - **Tests in the same file:** `filter_default_disables_progress` asserts
     `f.tool_result` — delete that one assertion line (keep `assert!(f.tool_call)`).
     `filter_toggle_flips_field` toggles index `8` expecting `progress` — `progress`
     is now index `7`; update both `f.toggle(8)` calls to `f.toggle(7)` (or retarget
     the test to the new progress index). The cursor-wrap tests use
     `FILTER_ITEM_COUNT` and need no change.

2. **Add a `tool_glyph` helper in `mcp/src/dashboard/transcript.rs`.** A pure
   `fn tool_glyph(name: &str) -> &'static str` mapping each tool name to a leading
   glyph, default for anything unmapped:

   ```rust
   fn tool_glyph(name: &str) -> &'static str {
       match name {
           "read_file" => "📖",
           "write_file" => "✏️",
           "patch" => "🩹",
           "bash" => "⚡",
           "search" => "🔍",
           "find_files" => "📁",
           "symbols" => "🔗",
           "update_task" => "✅",
           _ => "🔧",
       }
   }
   ```

3. **Add a `paired` parameter to `record_lines_with_lang` and render the paired
   result header.** Change the signature to
   `record_lines_with_lang(rec: &SessionRecord, path_hint: Option<&str>, paired: bool)`.
   Update the `record_lines(rec)` wrapper to delegate with
   `record_lines_with_lang(rec, None, false)`. In the `ToolResult` arm, the header
   summary depends on `paired` — **the body and the green/red color are unchanged**:

   ```rust
   let status = if *succeeded { "ok" } else { "FAIL" };
   let color = if *succeeded { Color::Green } else { Color::Red };
   let summary = if paired {
       format!("╰ [{status}]")
   } else {
       format!("tool {name} [{status}]")
   };
   ```

   Every other arm is unchanged. When `paired` is `false` the output is
   **byte-identical to today** (this is what keeps the existing `record_lines`
   direct-call tests green).

   There is exactly one other direct caller of `record_lines_with_lang` to update:
   the `record_lines_delegates_to_with_lang_none` test calls
   `record_lines_with_lang(&rec, None)` → make it `record_lines_with_lang(&rec, None, false)`.

4. **Generalize the call→result threading in `transcript_lines` to drive glyphs +
   pairing.** Replace `last_read_path: Option<String>` with a pending-call slot:

   ```rust
   struct PendingCall {
       name: String,
       path: Option<String>,
   }
   ```

   For each visible record, compute a `lead` span (the leftmost gutter cell — a
   glyph for a tool call, an indent for a paired result, nothing otherwise), a
   `paired` flag, and a `hint` for the body highlighter, then render and prepend
   `lead` **before** the existing timestamp span:

   ```rust
   let mut pending: Option<PendingCall> = None;
   for r in &visible {
       let mut lead: Option<Span<'static>> = None;
       let mut paired = false;
       let mut hint: Option<String> = None;
       match &r.event {
           SessionEvent::Parsed { tool_call } => {
               let path = if tool_call.name == "read_file" {
                   tool_call
                       .arguments
                       .get("path")
                       .and_then(|v| v.as_str())
                       .map(str::to_string)
               } else {
                   None
               };
               lead = Some(Span::raw(format!("{} ", tool_glyph(&tool_call.name))));
               pending = Some(PendingCall {
                   name: tool_call.name.clone(),
                   path,
               });
           }
           SessionEvent::ToolResult { name, .. } => {
               if let Some(p) = &pending
                   && &p.name == name
               {
                   paired = true;
                   if name == "read_file" {
                       hint = p.path.clone();
                   }
                   lead = Some(Span::raw(" ".repeat(RESULT_INDENT)));
               }
               pending = None;
           }
           _ => {}
       }

       let mut lines = record_lines_with_lang(r, hint.as_deref(), paired);

       if let Some(header) = lines.first_mut() {
           let mut spans = Vec::with_capacity(header.spans.len() + 2);
           if let Some(lead) = lead {
               spans.push(lead);
           }
           spans.push(Span::styled(
               format!("[{}] ", relative_ts(r.ts, base_ts)),
               Style::new().fg(Color::Rgb(180, 150, 50)),
           ));
           spans.append(&mut header.spans);
           *header = Line::from(spans);
       }
       out.extend(lines);
   }
   ```

   Define `pub(crate) const RESULT_INDENT: usize = 3;` near the top of the file
   (matching the rendered width of a 2-cell glyph plus its trailing space, so the
   paired result's `[+Ns]` timestamp aligns under the call's). Exact terminal
   alignment across emoji-width quirks is **best-effort, not a gate** — pin the
   *behavior* (the paired result's lead is `RESULT_INDENT` spaces; the call's lead
   is the glyph + a space), not pixel alignment.

### Behavioral pins (what the renderer must do)

- **Pairing is positional and name-matched.** A `ToolResult` is paired iff the
  pending call slot is `Some` **and** its `name` equals the result's `name`.
  Pairing tolerates non-tool events between the call and the result (the slot
  persists until consumed or overwritten by a later call) — do **not** require
  strict adjacency.
- **Orphan results render standalone.** A `ToolResult` with no matching pending
  call (`pending` is `None`, or names differ) renders `paired = false`, i.e.
  today's `tool {name} [{status}]` header with no indent — unchanged.
- **Glyph only on tool calls.** Only `Parsed` records get a glyph lead; only
  paired `ToolResult` records get an indent lead. Every other event type
  (`SessionStart`, `Completion`, `Progress`, …) gets **no** lead span, so its
  header's first span stays the timestamp (this is why the existing timestamp-span
  tests keep passing).
- **`record_lines(rec)` is unchanged-behavior for paired=false.** The wrapper and
  all its existing test callers must see byte-identical output to today.

## Acceptance criteria

- [ ] `cargo build` is warning-clean and `FILTER_ITEM_COUNT == 14`; the filter
      panel no longer offers a "tool result" item (no `item_label` in `0..14`
      returns `"tool result"`).
- [ ] With `tool_call = false`, `ActivityFilter::allows` returns `false` for **both**
      a `Parsed` and a `ToolResult` event; with `tool_call = true`, `true` for both.
- [ ] A `Parsed{read_file}` immediately followed by `ToolResult{read_file, ok}`
      renders the result header as `╰ [ok]` (contains `╰`, does **not** contain
      `tool read_file`), and the call header carries the `📖` glyph.
- [ ] A lone `ToolResult{read_file, ok}` with no preceding call renders
      `tool read_file [ok]` (standalone, no `╰`).
- [ ] `tool_glyph` returns distinct glyphs for `read_file`/`bash` and the default
      `🔧` for an unknown name.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Test plan

Concrete tests (names describe behavior; place unit tests in the existing
`#[cfg(test)] mod tests` blocks of the respective files):

- `filter_merges_tool_result_into_tool_call` in `filter.rs` — build an
  `ActivityFilter` with `tool_call: false, ..Default::default()`; assert `allows`
  is `false` for both a `Parsed` event and a `ToolResult` event. Flip `tool_call:
  true` and assert both `true`. **Mutation-resistant:** a renderer that still routed
  `ToolResult` to a separate field would let the result through when `tool_call` is
  off.
- `filter_has_no_tool_result_item` in `filter.rs` — assert
  `FILTER_ITEM_COUNT == 14` and that no `ActivityFilter::item_label(i)` for
  `i in 0..FILTER_ITEM_COUNT` equals `"tool result"`, and that index `3` is
  `"tool call"` and index `7` is `"progress"` (pins the renumber).
- `tool_glyph_maps_known_and_default` in `transcript.rs` — `read_file` → `"📖"`,
  `bash` → `"⚡"`, `update_task` → `"✅"`, `"nope"` → `"🔧"`. Distinct values
  (mutation-resistant vs a single-glyph impl).
- `transcript_lines_pairs_call_and_result` in `transcript.rs` — records
  `[Parsed{read_file, path=foo.rs}, ToolResult{read_file, ok, "fn x(){}"}]`; assert
  the result header line contains `"╰ [ok]"` and does **not** contain
  `"tool read_file"`, and the call header line contains both `"📖"` and
  `"→ call read_file"`.
- `transcript_lines_orphan_result_is_standalone` in `transcript.rs` — a single
  `ToolResult{read_file, ok}` with no preceding `Parsed`; assert its header contains
  `"tool read_file [ok]"` and does **not** contain `"╰"`. (negative — pairing must
  not fire without a call)
- `transcript_lines_pairs_only_matching_name` in `transcript.rs` — records
  `[Parsed{read_file}, ToolResult{bash, FAIL}]`; assert the `bash` result renders
  `"tool bash [FAIL]"` (standalone), not `"╰"`. (negative — names must match)
- `record_lines_tool_result_unpaired_unchanged` in `transcript.rs` — call
  `record_lines` directly on a `ToolResult{bash, FAIL}` and assert the header is
  `"tool bash [FAIL]"` (confirms `paired = false` is byte-identical to today).

Existing tests that move with the change (update, don't delete): the
`record_lines_delegates_to_with_lang_none` call gains the `false` arg;
`filter_default_disables_progress` drops its `f.tool_result` assertion;
`filter_toggle_flips_field` retargets the progress index `8` → `7`.

## End-to-end verification

Not applicable — phase ships no runtime-loadable artifact beyond the TUI render. The
Activity panel is a ratatui dashboard with no headless harness (consistent with the
M13/M15/M17 dashboard-panel precedent); the unit tests above render the real
`Line`/`Span` output (`format!("{line}")`) and assert on it, which is the closest
verifiable surface. Restate this reason in the completion Update Log.

## Authorizations

None. (No dependency, no `Cargo.toml`, no `architecture.md`, no `SessionEvent`.)

## Out of scope

- **The args block.** Tool-call arguments keep rendering as today's multi-line
  pretty-JSON body (dim grey). Do **not** add an inline `read_file(path=…)`
  signature or JSON syntax highlighting — that was deliberately not selected.
- **`log_query.rs`** and the CLI log-search `event_type` strings — different
  subsystem; leave untouched.
- **`render.rs` / `event_loop.rs`** — the filter panel is index-generic; no edit is
  needed there. If you find yourself editing them, stop — the merge belongs in
  `filter.rs`.
- **Collapsing the two headers into one** (the result keeps its own timestamp + turn
  marker by design — the "two headers, indented" shape was chosen over a collapsed
  connector).
- **Buffering / look-ahead** in `transcript_lines` — each record still renders
  independently; pairing is detected by the carried `pending` slot, not by
  consuming the next record.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Bounce — 2026-06-11 (bug-09-1, major)

First dispatch (session `phase-09-6a2b6aec`, executor Qwen/Qwen3.6-27B-FP8)
self-reported `complete` after 67 turns but **left the test gate red** and did
not do its completion bookkeeping (no status flip, no Update Log, no commit —
code left uncommitted in the working tree). Independent re-run confirmed two DoD
violations, both filed in
[bug-09-1](bugs/bug-09-1.md):

- **Defect A (red `cargo test`):** the two new pairing tests
  (`transcript_lines_pairs_call_and_result`,
  `transcript_lines_pairs_only_matching_name`) assert the result header at the
  hardcoded index `rendered[1]`, but the preceding `Parsed{read_file}` call
  emits a header **plus a multi-line pretty-JSON args body**, so the result
  header lands at a later index. Production pairing logic is correct; the tests
  are mis-indexed. Fix: scan all rendered lines for the connector rather than
  indexing a fixed position.
- **Defect B (`#[allow(dead_code)]` masking a clippy failure):** the
  `transcript_lines` rewrite removed the only production caller of
  `record_lines`, making it test-only; the executor suppressed the resulting
  `-D warnings` dead-code error with `#[allow(dead_code)]` (verified
  load-bearing). STANDARDS §1/§4 forbid masking a gate. Fix: gate the helper
  `#[cfg(test)]` instead.

`cargo fmt --all --check`, `cargo build`, and `cargo clippy` pass **only because
of the `#[allow]`**; with it removed clippy errors `function record_lines is
never used`. Bounced to `in-progress`; re-dispatch via
`/rexymcp:dispatch phase-09` once both defects are fixed.

**Calibration (self-report vs gate-exit disagreement — 2nd in M17):** like
phase-07, the executor returned `complete` on a red suite. Data point logged, not
yet a fold.

### Update — 2026-06-12 02:52 (complete)

**Executor:** Claude (re-dispatch, fixing bug-09-1 Defects A and B)

**Changes:**
- `mcp/src/dashboard/filter.rs` — already correct from prior dispatch (filter merge, `FILTER_ITEM_COUNT=14`, tests updated). No further edits needed.
- `mcp/src/dashboard/transcript.rs` — already implemented from prior dispatch (glyphs, pairing, `PendingCall`, `RESULT_INDENT`, `tool_glyph`, `paired` parameter). Two defects fixed:
  - **Defect A:** `transcript_lines_pairs_call_and_result` and `transcript_lines_pairs_only_matching_name` now scan rendered lines for the connector/call header text rather than using hardcoded `rendered[1]` index.
  - **Defect B:** `#[allow(dead_code)]` removed from `record_lines`; replaced with `#[cfg(test)]` since it is only used by tests.

**Verification commands (all clean):**
```
cargo fmt --all --check        # clean
cargo build                    # zero warnings
cargo clippy --all-targets --all-features -- -D warnings  # clean
cargo test                     # 739 passed, 0 failed, 2 ignored
```

**Grep proofs:**
- `"tool result"` in filter.rs: 1 match (test assertion in `filter_has_no_tool_result_item`) — no `item_label` returns it.
- `╰ [` in transcript.rs: 3 matches (doc comment, production format string, test assertion).
- `#[allow(dead_code)]` in transcript.rs: 0 matches.

**Files changed:** `mcp/src/dashboard/filter.rs`, `mcp/src/dashboard/transcript.rs`

**End-to-end verification:** N/A — phase ships no runtime-loadable artifact beyond the TUI render. The Activity panel is a ratatui dashboard with no headless harness (consistent with the M13/M15/M17 dashboard-panel precedent); the unit tests above render the real `Line`/`Span` output (`format!("{line}")`) and assert on it, which is the closest verifiable surface.

**Notes for review:** None. Both defects from bug-09-1 are fixed; all gates pass.

### Review verdict — 2026-06-12

- **Verdict:** approved_after_1
- **Bounces:** 1 — [bug-09-1](bugs/bug-09-1.md) (major), both defects fixed in the
  re-dispatch (`abd11e6`): Defect A (two pairing tests indexed the result header at
  `rendered[1]`, where the preceding `read_file` call's multi-line JSON args body
  actually sits — now scan `rendered` with `.iter().find(...)` for the connector /
  call-header text) and Defect B (`#[allow(dead_code)]` masking a `-D warnings`
  dead-code error on `record_lines` after its only production caller was removed —
  now honestly gated `#[cfg(test)]`).
- **Executor:** Qwen/Qwen3.6-27B-FP8
- **Independent re-run:** all four gates green — `cargo fmt --all --check` clean,
  `cargo build` zero warnings, `cargo clippy --all-targets --all-features -- -D
  warnings` clean, `cargo test` **384 mcp + 739 executor + 0 doc, 0 failed**
  (2 ignored). filter.rs matches Spec task 1 byte-for-byte (`FILTER_ITEM_COUNT=14`,
  `ToolResult` arm routes to `self.tool_call`, contiguous `0..14` renumber);
  transcript.rs implements glyphs, `PendingCall` pairing, `RESULT_INDENT`, and the
  `paired` parameter per the Spec. No new `unwrap`/`expect`/`panic`/`unsafe`/
  `#[allow]`/`TODO` in production; the only `.expect()` additions are in test code.
- **Mutation-check:** forcing `paired=false` in the `ToolResult` summary makes
  `transcript_lines_pairs_call_and_result` fail (the `.find(|l| l.contains("╰ [ok]"))`
  → `.expect()` panics), confirming the pairing test is load-bearing. Removing the
  `#[cfg(test)]` gate (re-adding the bare fn) reproduces `function record_lines is
  never used` under `-D warnings`, confirming Defect B's fix is the honest form.
- **Scope deviations:** none. `record_lines` lost its `pub(crate)` and became
  `#[cfg(test)]` — correct, since the `transcript_lines` rewrite removed its sole
  production caller, making it a genuine test-only helper. No `log_query.rs` /
  `render.rs` / `event_loop.rs` / `SessionEvent` / `Cargo.toml` change, as required.
- **E2E:** N/A per the doc's End-to-end section — TUI render, no headless harness
  (M13/M15/M17 dashboard-panel precedent); unit tests assert on the real
  `format!("{line}")` output.
- **Calibration:** **self-report vs gate-exit disagreement, 2nd in M17** (phase-07
  was the 1st; phase-07 was an `escalated` takeover, this a clean bounce→re-dispatch).
  Two occurrences = a **trend**, not yet a fold (WORKFLOW: one is data, two is a
  trend, three is a fix). Both M17 instances were the executor returning `complete`
  while `cargo test` was red. Watch for a 3rd; if it lands, fold a "the gate exit
  code is authoritative — never report `complete` on a red `cargo test`" reminder
  into the executor-facing reporting guidance (with user sign-off).
