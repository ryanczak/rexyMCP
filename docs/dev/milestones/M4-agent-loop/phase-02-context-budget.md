# Phase 02: context budget + compaction

**Milestone:** M4 — Headless agent loop + governor/verifier
**Status:** review
**Depends on:** phase-01 (done)
**Estimated diff:** ~620 lines (tokens + budget + compactor + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Lift the context-budget engine: a token-count heuristic, a stateless `Budget`
(token ceiling + overflow queries), and a `compact()` that frees tokens when the
budget overflows — signaturize old tool-results first, then evict oldest
non-system messages, **never** evicting `role = "system"`. The agent loop
(phase-06) calls these between turns to keep the prompt under the model's window.

## Architecture references

- `docs/architecture.md` — "The executor turn cycle" step 2 (apply the context
  budget; compact if needed; never evict system messages).
- Rexy source: `rexy/src/context/{tokens,budget,compactor}.rs`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M4 README Notes.
2. Read this entire phase doc.
3. Confirm phase-01 is `done`; `crate::ai::types::Message` exists (M1) — **verify
   its shape**: the budget/compactor use `msg.role` (`&str`) and `msg.content`
   (`String`). If rexyMCP's `Message` differs, adapt the field accesses (do not
   change `Message`). `crate::config::BudgetConfig` exists with `max_context_pct:
   u8`.
4. **Read** `rexy/src/context/{tokens,budget,compactor}.rs`. Lift faithfully with
   the adaptations below.

## Spec

Create `executor/src/context/mod.rs` (declares `pub mod tokens; pub mod budget;
pub mod compactor;` — **only those three**, not Rexy's `file_cache` /
`project_state` / `working` / `usage`), plus the three files. Wire `pub mod
context;` into `lib.rs`.

- **`tokens.rs`** — pure lift of `count(&str) -> usize` (the ~4-char-chunk
  heuristic, CJK = 1 token, punctuation = 1 token). No dependencies. No changes.
- **`budget.rs`** — lift `Budget { ceiling: usize }` with `estimate` /
  `would_overflow` / `fraction_used` (over `system_prompt: &str` + `messages:
  &[Message]`) and the `usize::MAX` "never overflow" `Default`. **Adaptation:**
  drop the `crate::profiles::Profile` dependency (rexyMCP has no profiles).
  Replace `from_config(cfg, profile)` with:
  - `pub fn new(ceiling: usize) -> Self`
  - `pub fn from_context(context_length: usize, max_context_pct: u8) -> Self`
    computing `ceiling = context_length × max_context_pct / 100` (saturating).
  The loop (phase-06) decides where `context_length` comes from; phase-02 does not
  read it from config (rexyMCP config has no context-length field yet — out of
  scope here).
- **`compactor.rs`** — lift `compact(...) -> CompactionReport` (signaturize
  tool-results oldest-first, then evict oldest non-system, to
  `TARGET_FRACTION = 0.75` of the ceiling) + `CompactionReport`. **Adaptation:**
  operate on `messages: &mut Vec<Message>`, **not** Rexy's `Session` (which
  rexyMCP doesn't have until phase-06). Signature:
  `pub fn compact(messages: &mut Vec<Message>, budget: &Budget, system_prompt:
  &str) -> CompactionReport`. Keep the `<tool_result>…</tool_result>` detection
  and the `[compacted: N bytes / M tokens — …]` signature format.

**Adaptations (general):**

1. **Strip Rexy references** that don't exist here: `profiles::Profile`,
   `agent::session::Session`, the `memory` scratchpad note (context/mod.rs), and
   the `TurnEvent::Compaction(report)` doc reference in `compactor.rs` (the loop's
   event type is phase-06 — describe the report as the compactor's return value,
   not a `TurnEvent`).
2. **No new dependencies.** `tokens` is a heuristic — do **not** add a real
   tokenizer crate (that's explicit non-goal polish; the heuristic is the spec).
3. `[compacted: ` is a **spec-pinned literal** (the signature format) — include a
   grep proof in the completion entry per AGENTS.md.

## Acceptance criteria

- [ ] `executor/src/context/{mod,tokens,budget,compactor}.rs` exist; `pub mod
      context;` in `lib.rs`.
- [ ] `tokens::count` returns a positive estimate for non-empty text (heuristic,
      no tokenizer dep).
- [ ] `Budget::from_context(32_768, 70)` → `ceiling == 22_937`; `estimate` sums
      `count(system_prompt)` + each `count(msg.content)`; `would_overflow` true at
      or above ceiling; `Default::default().ceiling == usize::MAX`.
- [ ] `compact` signaturizes a large `<tool_result>` (content replaced with a
      `[compacted: …]` body that preserves the `<tool_result>…</tool_result>`
      wrapper); then evicts **oldest non-system** messages to reach `0.75 ×
      ceiling`; **never evicts a `system` message**; is a no-op under target; is
      idempotent on already-signaturized messages.
- [ ] **Negatives:** `compact` with everything under budget changes nothing
      (report counts all zero); a `system` message survives even when it alone
      exceeds budget.
- [ ] No surviving Rexy references (grep `profiles`, `Session`, `TurnEvent`,
      `memory` → 0 in the new files); the `[compacted: ` literal is present
      (grep proof).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic, pure (no network, no subprocess, no clock). Lift Rexy's budget +
compactor tests, swapping `Session` for a `Vec<Message>` and the
`from_config`/profile test for a `from_context` formula test. Pin the negatives
(no-op under target; system-message survival) and the `0.75` target stop.

## End-to-end verification

> Not applicable — pure library functions exercised by unit tests. The loop that
> calls `budget`/`compact` between turns is M4 phase-06.

## Authorizations

- [x] **May create** `executor/src/context/{mod,tokens,budget,compactor}.rs`;
      **may modify** `executor/src/lib.rs` (`pub mod context;`).
- [ ] **No new dependencies** (no tokenizer crate).
- [ ] May **NOT** lift `file_cache` / `project_state` / `working` / `usage`, add a
      `Session` type, or read context-length from config — later/elsewhere.
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, or
      another phase doc.

## Out of scope

- **`Session`** and where `context_length` comes from — phase-06 (the loop).
- **A real BPE tokenizer** — explicit non-goal; the heuristic is the spec.
- **`file_cache` / `project_state` / working-memory / usage** Rexy context
  submodules — not part of rexyMCP's loop.

## Executor note

`compactor.rs` contains `<tool_result>` / `</tool_result>` close-tag literals and
the `[compacted: …]` format string — the content class that can trip opencode's
serializer (WORKFLOW.md § "Phase progression"). The grep proof on `[compacted: `
will catch a corrupted write; if opencode mangles it, re-apply the heredoc
workaround (AGENTS.md) or fall back to direct execution.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

### Update — 2026-05-28 (started)

**Executor:** Qwen3.6-27B-FP8 / opencode

Lifting `tokens.rs`, `budget.rs`, `compactor.rs` from Rexy's `context/` module with adaptations: dropped `Profile`/`Config`/`Session` dependencies, replaced `from_config` with `new` + `from_context`, adapted `compact` to operate on `&mut Vec<Message>`.

### Update — 2026-05-28 (complete)

**Summary:** Lifted three modules from Rexy's context engine with minimal adaptations. `tokens.rs` is a pure copy. `budget.rs` drops the `Profile` dependency and provides `new(ceiling)` + `from_context(context_length, max_context_pct)` constructors. `compactor.rs` operates on `&mut Vec<Message>` instead of Rexy's `Session`. All Rexy-specific references (`profiles`, `Session`, `TurnEvent`, `memory`) removed.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.0 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.0 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) built in 0.48s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.05s

cargo test 2>&1 | tail -30
test result: ok. 359 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out
```

**End-to-end verification:** Not applicable — pure library functions exercised by unit tests. The loop that calls `budget`/`compact` between turns is M4 phase-06.

**Files changed:**
- `executor/src/context/mod.rs` — declares `pub mod tokens; pub mod budget; pub mod compactor;`
- `executor/src/context/tokens.rs` — pure lift of `count()` heuristic + tests
- `executor/src/context/budget.rs` — adapted `Budget` with `new`/`from_context` + tests
- `executor/src/context/compactor.rs` — adapted `compact` over `&mut Vec<Message>` + tests
- `executor/src/lib.rs` — added `pub mod context;`

**New tests:**
- `tokens::tests` — 6 tests (empty, short word, long word, punctuation, CJK, mixed content)
- `budget::tests` — 8 tests (default, new, from_context formula, saturate, estimate, overflow true/false, fraction ratio, sentinel zero)
- `compactor::tests` — 8 tests (signaturize tool result, preserve system, evict oldest first, idempotent, no-op under target, stops at fraction, correct counts, system-only survival)

**Commits:**
- `6d7472b` — feat: lift context budget engine (tokens, budget, compactor) from Rexy

**Grep proofs:**
- `[compacted: ` found in `compactor.rs:111` (comment) and `compactor.rs:120` (format string) ✓
- `profiles` / `Session` / `TurnEvent` / `memory` → 0 matches in new files ✓

**Notes for review:** None. Lift is straightforward; tests adapted from Rexy with `Session` replaced by `Vec<Message>` and `from_config` replaced by `from_context`.

**verification: fmt OK · clippy OK · tests 359 passed · build OK**
