# Phase 07: content-aware compaction priority

**Milestone:** M10 — Context optimization
**Status:** review
**Depends on:** phase-04 (`[superseded:` breadcrumbs) and phase-06 (`[already-read:` references) — both produce already-reclaimed husks this phase must recognize and skip; phase-05 (`Budget::estimate` now counts `tool_calls`/`tool_results` content) — the correctness fix this phase carries into the compactor's own token accounting. Third Arc B lever.
**Estimated diff:** ~300 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

When the context budget overflows, the compactor today is **value-blind**: it
signaturizes a message shape the loop no longer produces, then evicts whole
messages oldest-first. This phase makes compaction **content-aware** — it shrinks
the lowest-value tool output **first** (noisy command output before file reads),
protects the most recent turns (where the live diagnostics and working context
are), and preserves message structure by signaturizing content **in place**
instead of removing messages. All of it lives in one file
(`executor/src/context/compactor.rs`) and reuses the exact `[compacted: …]`
marker and in-place-breadcrumb technique already proven by phase-04's
`evict_superseded_reads`.

This phase also carries a **correctness fix discovered while scoping it** (see
Current state): since phase-05 taught `Budget::estimate` to count
`tool_calls`/`tool_results` content, the compactor's internal running-token math —
which still counts only `msg.content` — under-counts what it frees on the real
message shape. This phase fixes that with a shared per-message token helper.

## Architecture references

Read before starting:

- `executor/src/context/compactor.rs` — the whole file. The two existing passes,
  the `[compacted:` signature format, `is_tool_result`/`is_already_signaturized`,
  and all 8 existing tests. This phase adds to this file and **must keep all 8
  existing tests passing unchanged.**
- `executor/src/agent/tools.rs` lines 85–151 — `evict_superseded_reads`: the
  worked example for "walk `messages`, find a structured tool-result, replace
  `tool_results[0].content` in place with a breadcrumb, count the tokens
  reclaimed." Your new signaturization pass is the same move with a `[compacted:`
  marker instead of `[superseded:`.
- `executor/src/context/budget.rs` lines 44–60 — `Budget::estimate`: the
  per-message token loop (`content` + `tool_calls[].arguments` +
  `tool_results[].content`) your new `message_tokens` helper must mirror exactly.
- `docs/architecture.md` line 117 — "Apply the context budget; compact if needed
  (never evict system messages)." The invariant this phase preserves. No
  architecture.md change is needed or authorized.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes (`git status`).
5. Run `cargo test` and record the passing count — the completion log must show
   the same count plus the new tests. (628 executor at the start of M10's later
   phases; confirm the current number yourself.)

## Current state

**The compactor today** (`executor/src/context/compactor.rs`). `compact()` runs
two passes to get under `target = ceiling × 0.75`:

```rust
    // ── Pass 1: signaturize tool-result messages oldest-first ──
    for msg in messages.iter_mut() {
        if running_total <= target { break; }
        if !is_tool_result(msg) || is_already_signaturized(msg) { continue; }
        let old_tokens = tokens::count(&msg.content);
        let signature = format_signature(msg);
        let new_tokens = tokens::count(&signature);
        msg.content = signature;
        running_total = running_total.saturating_sub(old_tokens).saturating_add(new_tokens);
        messages_signaturized += 1;
    }

    // ── Pass 2: evict oldest non-system messages until under target ──
    while running_total > target {
        let evict_idx = messages.iter().position(|m| m.role != "system");
        let Some(idx) = evict_idx else { break; };
        let removed = messages.remove(idx);
        running_total = running_total.saturating_sub(tokens::count(&removed.content));
        messages_evicted += 1;
    }
```

**Two facts about the real message shape make this value-blind and
mis-accounted:**

1. **Pass 1 never matches a real tool exchange.** `is_tool_result(msg)` requires
   `msg.role == "user" && content.starts_with("<tool_result>")`. But the loop
   builds tool exchanges via `append_tool_exchange` (`agent/tools.rs`), which
   produces an **`assistant`** message carrying `tool_calls` (with `content`
   empty) followed by a **`tool`** message carrying `tool_results` (with `content`
   empty, the payload in `tool_results[0].content`). Neither matches pass 1. So on
   real data, **all** reclamation falls to pass 2's whole-message eviction.

2. **Pass 2 under-counts what it frees.** It decrements `running_total` by
   `tokens::count(&removed.content)` — but a structured `tool`/`assistant` message
   has **empty `content`**; its tokens live in `tool_results[]`/`tool_calls[]`.
   Since phase-05, `tokens_before` (from `Budget::estimate`) *does* count those,
   so the running total starts high but barely drops as messages are evicted —
   the loop evicts far more than necessary and reports a near-zero reduction.

**The structured shape**, from `append_tool_exchange` (`agent/tools.rs`):

```rust
    messages.push(Message {                 // the tool-call half
        role: "assistant".to_string(),
        content: String::new(),
        tool_calls: Some(vec![AiToolCall { /* name, arguments (JSON string) */ }]),
        tool_results: None,
        turn: Some(turn),
    });
    messages.push(Message {                 // the tool-result half (the payload)
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult { tool_name, content /* the result */ }]),
        turn: Some(turn),
    });
```

**Why signaturize in place instead of value-ordered eviction.** Reordering
whole-message eviction by value would let the compactor remove an `assistant`
tool-call message while keeping its paired `tool` result (or vice versa),
orphaning a `tool_results` entry from the `tool_call` it answers — a structural
break an OpenAI-compatible backend can reject. Shrinking a tool result's
**content in place** (the phase-04 technique) reclaims the same tokens while
leaving every message — and every tool-call/tool-result pair — intact. So this
phase's new work is a **value-ranked signaturization pass**, not an eviction
reorder. Pass 2 (oldest-first eviction) stays as the unchanged last-resort
fallback for the rare case signaturization can't reach target.

**Already-reclaimed husks to skip.** Phase-04 leaves `[superseded:` breadcrumbs
and phase-06 leaves `[already-read:` references in `tool_results[0].content`;
this phase leaves `[compacted:` signatures. All three are already small — there
is nothing to reclaim by signaturizing them again, and re-wrapping would stack
markers. The new pass must skip any tool result whose content already starts with
`[superseded:` / `[already-read:` or contains `[compacted:`. (Those two prefix
literals are `const`s private to `agent/tools.rs`; duplicate the **string
literals** here — do not make the consts `pub` or import them. `[compacted:`
matches the existing `is_already_signaturized` marker.)

**`CompactionReport` stays exactly as it is** — four fields (`tokens_before`,
`tokens_after`, `messages_signaturized`, `messages_evicted`). The new pass
increments the existing `messages_signaturized`. **Do not add a field.** Its
consumers (`SessionEvent::Compaction`, `mcp/src/status.rs`,
`mcp/src/dashboard/transcript.rs`) would each need updating, and the
per-source breakdown belongs to phase-08, which adds the metrics reader that
consumes it. (WORKFLOW.md §"Derive intentionally": do not wire state whose
consumer does not exist yet.) **This phase touches no file other than
`compactor.rs`.**

## Spec

All work is in `executor/src/context/compactor.rs`. Numbered in execution order.
**Compile (`cargo build`) after task 4, before writing any test.**

### 1. Add a `message_tokens` helper (fixes the accounting)

Add a private fn mirroring `Budget::estimate`'s per-message loop
(`budget.rs:44`):

```rust
/// Tokens a single message contributes, counting the same parts as
/// `Budget::estimate`: `content` + every `tool_calls[].arguments` +
/// every `tool_results[].content`. The compactor's running total must use
/// this — a structured tool/assistant message carries its payload in
/// `tool_calls`/`tool_results`, not `content`.
fn message_tokens(msg: &Message) -> usize {
    let mut total = tokens::count(&msg.content);
    if let Some(tcs) = &msg.tool_calls {
        for tc in tcs {
            total = total.saturating_add(tokens::count(&tc.arguments));
        }
    }
    if let Some(trs) = &msg.tool_results {
        for tr in trs {
            total = total.saturating_add(tokens::count(&tr.content));
        }
    }
    total
}
```

Then, in **pass 2**, replace the decrement
`running_total.saturating_sub(tokens::count(&removed.content))` with
`running_total.saturating_sub(message_tokens(&removed))`. (Leave pass 1 as it is —
it operates on the text shape where `content` *is* the payload.)

### 2. Add the recent-turn protection constant

```rust
/// Protect the most recent turns from signaturization — the live diagnostics,
/// the current edit's reads, and the working context the model is actively
/// using all live here. A tool result whose turn is within this many turns of
/// the newest turn in the conversation is never signaturized by the
/// value-ranked pass.
const RECENT_TURNS_PROTECTED: usize = 3;
```

### 3. Add a `reclaim_rank` classifier (the value rank)

Pure; decides whether a message is a candidate for in-place signaturization and,
if so, its value rank (**lower = reclaim first**). Returns `None` for anything
the pass must leave alone.

```rust
/// Value rank for in-place signaturization. `Some(rank)` marks a structured
/// tool-result message whose content can be shrunk to a `[compacted: …]`
/// signature; lower rank is reclaimed first. `None` means "leave it alone":
/// not a structured tool result, already reclaimed, or recency-protected.
///
/// Rank order (reclaim cheapest-to-lose first): non-`read_file` tool output
/// (bash/search/etc. — noisy, regenerable) ranks 0, before `read_file` results
/// (the model's working file context) at rank 1.
fn reclaim_rank(msg: &Message, newest_turn: Option<usize>) -> Option<u8> {
    if msg.role != "tool" {
        return None;
    }
    let r = msg.tool_results.as_ref()?.first()?;
    // Already-reclaimed husks / signatures — nothing to gain, don't re-wrap.
    if r.content.contains("[compacted:")
        || r.content.starts_with("[superseded:")
        || r.content.starts_with("[already-read:")
    {
        return None;
    }
    // Protect the last RECENT_TURNS_PROTECTED turns (diagnostics live here).
    if let (Some(t), Some(n)) = (msg.turn, newest_turn)
        && n.saturating_sub(t) < RECENT_TURNS_PROTECTED
    {
        return None;
    }
    if r.tool_name == "read_file" {
        Some(1)
    } else {
        Some(0)
    }
}
```

### 4. Add the value-ranked signaturization pass

Insert a new pass **between pass 1 and pass 2**. It collects every candidate,
orders them by `(rank, turn, index)` so the lowest-value oldest output is
reclaimed first, and signaturizes each tool result's content in place until the
running total reaches target. Indices stay valid because nothing is removed.

```rust
    // ── Pass 1.5: value-ranked, in-place signaturization of structured tool
    // results. Preferred over eviction: shrinks content while preserving every
    // message and every tool-call/tool-result pair. ──
    let newest_turn = messages.iter().filter_map(|m| m.turn).max();
    let mut candidates: Vec<(u8, usize, usize)> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| reclaim_rank(m, newest_turn).map(|rank| (rank, m.turn.unwrap_or(0), i)))
        .collect();
    candidates.sort();
    for (_, _, idx) in candidates {
        if running_total <= target {
            break;
        }
        let before = message_tokens(&messages[idx]);
        signaturize_tool_result(&mut messages[idx]);
        let after = message_tokens(&messages[idx]);
        running_total = running_total.saturating_sub(before.saturating_sub(after));
        messages_signaturized += 1;
    }
```

with the in-place signaturizer (mirrors `evict_superseded_reads`'s replace-in-
place move, and the existing `format_signature`'s `[compacted:` marker):

```rust
/// Replace a structured tool-result's content with a compact signature, in
/// place. The `[compacted:` marker matches `is_already_signaturized` and the
/// pass-1 text-shape format, so re-compaction skips it.
fn signaturize_tool_result(msg: &mut Message) {
    if let Some(results) = msg.tool_results.as_mut()
        && let Some(r) = results.first_mut()
    {
        let byte_size = r.content.len();
        let token_count = tokens::count(&r.content);
        r.content = format!(
            "[compacted: {byte_size} bytes / {token_count} tokens — tool result compacted for budget]"
        );
    }
}
```

**Compile now** (`cargo build`) before writing tests.

### 5. Tests

Add to the existing `#[cfg(test)] mod tests` block. Reuse the existing
`make_system`/`make_user`/`make_assistant`/`make_tool_result` helpers; add a
small helper that builds a **structured** tool exchange — a `tool` message with
`tool_results: Some(vec![AiToolResult { tool_name, content, .. }])` and
`turn: Some(t)` — since the existing helpers don't cover that shape. (`Message`,
`AiToolResult` are imported in the module already or via `crate::ai::types`;
match the import style `evict_superseded_reads`'s tests use.) See the Test plan.

## Acceptance criteria

- [ ] `grep -n 'fn message_tokens' executor/src/context/compactor.rs` matches.
- [ ] `grep -n 'fn reclaim_rank' executor/src/context/compactor.rs` matches.
- [ ] `grep -n 'fn signaturize_tool_result' executor/src/context/compactor.rs` matches.
- [ ] `grep -n 'RECENT_TURNS_PROTECTED' executor/src/context/compactor.rs` matches.
- [ ] All **8 pre-existing** compactor tests still pass **unchanged** (no edits to
      their bodies).
- [ ] On overflow with old, large structured `read_file`/`bash` tool results
      present, `compact` shrinks a result's `tool_results[0].content` to a string
      containing `[compacted:` **without removing the message** (the message count
      for the signaturized exchange is preserved), and `tokens_after <
      tokens_before`.
- [ ] When signaturizing exactly one candidate suffices to reach target and both
      an old `bash` result and an old `read_file` result are candidates, the
      **`bash` (non-`read_file`) result is signaturized and the `read_file`
      result is left intact**. **(value ordering)**
- [ ] A structured tool result in the **most recent turn** is **not** signaturized
      under pressure when other candidates suffice. **(negative — recency
      protection)**
- [ ] A tool result whose content already contains `[compacted:`, or starts with
      `[superseded:` / `[already-read:`, is **not** re-signaturized. **(negative —
      idempotence / husk skip)**
- [ ] `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --all --check`, and `cargo test` all pass; test count is the
      pre-flight count plus the new tests.

## Test plan

Unit tests in the `#[cfg(test)] mod tests` block at the bottom of
`compactor.rs`. All hermetic and pure (no fs, no network, no clock). Add a helper
for a structured tool exchange, e.g.:

```rust
fn make_tool_msg(tool_name: &str, content: &str, turn: usize) -> Message {
    Message {
        role: "tool".to_string(),
        content: String::new(),
        tool_calls: None,
        tool_results: Some(vec![AiToolResult {
            tool_call_id: "c1".to_string(),
            tool_name: tool_name.to_string(),
            content: content.to_string(),
        }]),
        turn: Some(turn),
    }
}
```

`message_tokens` (pure):

- `message_tokens_counts_tool_results_content` — a `make_tool_msg("read_file",
  big, 1)` (empty `content`, payload in `tool_results`) → `message_tokens` equals
  `tokens::count(big)` and is `> 0`.
- `message_tokens_counts_tool_call_arguments` — an assistant message with
  `tool_calls` carrying non-empty `arguments` → counted (`> 0` despite empty
  `content`).
- `message_tokens_plain_equals_content` — `make_user("hello world")` →
  `message_tokens` equals `tokens::count("hello world")`.

`reclaim_rank` (pure):

- `reclaim_rank_command_output_before_read` — `make_tool_msg("bash", big, 1)` with
  `newest_turn = Some(10)` → `Some(0)`; `make_tool_msg("read_file", big, 1)` →
  `Some(1)`.
- `reclaim_rank_protects_recent_turns` — `make_tool_msg("bash", big, 9)` with
  `newest_turn = Some(10)` (within `RECENT_TURNS_PROTECTED = 3`) → `None`.
  **(negative)**
- `reclaim_rank_skips_already_compacted` — a tool result whose content contains
  `[compacted:` → `None`. **(negative)**
- `reclaim_rank_skips_superseded_and_already_read_husks` — content starting with
  `[superseded:` → `None`; content starting with `[already-read:` → `None`.
  **(negative)**
- `reclaim_rank_skips_non_tool_messages` — `make_user`, `make_assistant`,
  `make_system` → `None`. **(negative)**

`compact` integration over the structured shape:

- `compact_signaturizes_structured_tool_result_in_place` — `[system, big
  read_file tool result at turn 1, … enough recent filler to set newest_turn high
  …]`, small budget → after compact the turn-1 result's content contains
  `[compacted:`, the message is **still present** (not removed),
  `report.messages_signaturized >= 1`, and `report.tokens_after <
  report.tokens_before`.
- `compact_reclaims_command_output_before_file_read` — one old `bash` result and
  one old `read_file` result, both large and both past the recency window;
  budget tuned so signaturizing **one** reaches target → the `bash` result's
  content contains `[compacted:` and the `read_file` result's content is
  **unchanged** (still the original big string). **(value ordering)**
- `compact_protects_recent_tool_result` — a large structured tool result at the
  newest turn plus enough older candidates that signaturizing the older ones
  reaches target → the newest-turn result is **unchanged** and
  `report.messages_evicted == 0`. **(negative — recency protection)**
- `compact_signaturization_preserves_pairing_and_count` — messages = `[assistant
  tool_call, tool result]` pairs (old, large); after compact, `messages.len()` is
  unchanged and `report.messages_evicted == 0` (signaturization alone sufficed),
  while at least one result now contains `[compacted:`.
- `compact_idempotent_on_structured_signature` — run `compact` twice; the second
  run does **not** re-signaturize the already-`[compacted:` results
  (`report2.messages_signaturized` counts only any not-yet-touched candidates,
  and the already-compacted content is byte-identical between runs).

**Do not modify the 8 existing tests.** If one fails, you changed pass-1/pass-2
behavior — revert that and confine your changes to the additive pass.

## End-to-end verification

Not applicable in the CLI sense — this phase ships no runtime-loadable artifact
(no new CLI surface, no config the binary loads); it is internal compactor
behavior. The faithful exercise is the `compact()` unit tests **run over the
exact structured `Message` shape `append_tool_exchange` produces** (the
`make_tool_msg` helper builds that shape: `role:"tool"`, payload in
`tool_results[0].content`). Additionally, the existing agent-loop compaction test
`logs_compaction_event_when_budget_overflows` (`agent/mod.rs`) must remain green,
confirming the loop still compacts on overflow and logs the `Compaction` event
with the unchanged `CompactionReport`. In the completion log, run and quote:

`cargo test logs_compaction_event_when_budget_overflows -- --nocapture`

and one of the new value-ordering tests:

`cargo test compact_reclaims_command_output_before_file_read -- --nocapture`

## Authorizations

None. No new dependency. No `docs/architecture.md` change. No `Cargo.toml`
change. No change to any file other than `executor/src/context/compactor.rs`.

## Out of scope

- **Adding a field to `CompactionReport`** (a per-source reclaim breakdown). That
  needs a consumer, which phase-08 (context-efficiency metrics on `PhaseRun`)
  adds; wiring it now would touch `SessionEvent::Compaction`, `status.rs`, and
  `dashboard/transcript.rs` for state nothing reads yet. Keep the four-field
  report exactly as-is.
- **A new `SessionEvent` variant.** This phase emits nothing new — the existing
  `SessionEvent::Compaction` already carries `messages_signaturized`, which now
  includes the value-ranked pass. Do **not** touch `event.rs`,
  `dashboard/filter.rs`, `log_query.rs`, or any match-arm site.
- **Value-ordered *eviction* (reordering pass 2).** Pass 2 stays oldest-first;
  reordering whole-message removal by value risks orphaning a `tool_result` from
  its `tool_call`. The value ranking applies only to in-place signaturization.
- **Removing or rewriting the legacy pass-1 text-shape signaturization.** It no
  longer matches real messages but its 8 tests pin behavior; leave it and its
  tests untouched. Deleting it is a separate cleanup, not this phase.
- **Signaturizing assistant prose / `tool_calls` arguments.** The reclaimable bulk
  is in `tool_results[].content`; assistant tool-call messages carry only small
  JSON args. Pass 1.5 targets `role:"tool"` results only.
- **Changing `TARGET_FRACTION`, the budget, or the read/output filters
  (phase-01/02), eviction breadcrumbs (phase-04), or dedupe (phase-06).** This
  phase only changes *which* content the compactor shrinks first and *how it
  accounts* for it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-01-28 (started)

**Executor:** rexyMCP executor LLM

Implementing content-aware compaction priority: `message_tokens` helper, `reclaim_rank` classifier, `signaturize_tool_result` in-place signaturizer, value-ranked pass 1.5, recency protection constant, and 13 new tests.

### Update — 2025-01-28 (complete)

**Tasks completed:**

1. Added `message_tokens()` helper mirroring `Budget::estimate`'s per-message loop — counts `content` + `tool_calls[].arguments` + `tool_results[].content`. Fixed pass 2's token under-count by using `message_tokens(&removed)` instead of `tokens::count(&removed.content)`.
2. Added `RECENT_TURNS_PROTECTED = 3` constant.
3. Added `reclaim_rank()` classifier: returns `Some(0)` for non-`read_file` tool results, `Some(1)` for `read_file`, `None` for non-tool messages, already-reclaimed husks (`[compacted:`, `[superseded:`, `[already-read:`), or recency-protected turns.
4. Added `signaturize_tool_result()` in-place signaturizer and pass 1.5 (value-ranked signaturization) between pass 1 and pass 2.
5. Added 13 new tests: 3 for `message_tokens`, 5 for `reclaim_rank`, 5 for `compact` integration over structured message shape.

**Verification commands:**

```
$ cargo fmt --all --check
(no output — clean)

$ cargo build
Finished `dev` profile [optimized] target(s) in 0.15s

$ cargo clippy --all-targets --all-features -- -D warnings
Finished `dev` profile [optimized] target(s) in 0.15s

$ cargo test
test result: ok. 657 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

Pre-flight test count: 644 passed. Post-completion: 657 passed (+13 new tests). All 8 pre-existing compactor tests still pass unchanged.

**End-to-end verification:**

```
$ cargo test logs_compaction_event_when_budget_overflows -- --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored

$ cargo test compact_reclaims_command_output_before_file_read -- --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored
```

**Grep for spec-pinned literals:**

```
$ grep -n 'fn message_tokens' executor/src/context/compactor.rs
160:fn message_tokens(msg: &Message) -> usize {
$ grep -n 'fn reclaim_rank' executor/src/context/compactor.rs
183:fn reclaim_rank(msg: &Message, newest_turn: Option<usize>) -> Option<u8> {
$ grep -n 'fn signaturize_tool_result' executor/src/context/compactor.rs
211:fn signaturize_tool_result(msg: &mut Message) {
$ grep -n 'RECENT_TURNS_PROTECTED' executor/src/context/compactor.rs
26:const RECENT_TURNS_PROTECTED: usize = 3;
```

**Files changed:** `executor/src/context/compactor.rs` (only file touched per authorization).

**Commit:** `feat: content-aware compaction priority — value-ranked in-place signaturization pass`

**Notes for review:** None. All acceptance criteria met.
