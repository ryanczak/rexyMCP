# Bug 1 on M10: `Budget::estimate` ignores tool exchanges — context_pct is always wrong

**Severity:** major
**Status:** fixed (M10 phase-05, commit `43fa08b`, 2026-06-07)
**Filed:** 2026-06-07
**Discovered:** phase-04 session `session-phase-04-6a263b1c.jsonl`

## What's wrong

`Budget::estimate` (`executor/src/context/budget.rs:44–50`) counts only `msg.content`
for each message. But `append_tool_exchange` (`executor/src/agent/tools.rs:123`) pushes
two messages with `content: String::new()` — the actual content lives in
`msg.tool_calls[0].arguments` (the assistant message) and
`msg.tool_results[0].content` (the tool message). Every file-read result, bash output,
and patch argument is invisible to the estimate.

Observed in session `session-phase-04-6a263b1c.jsonl`: all 44 `context_pct` events
show `0.14964545372946772` (16,672 / 111,410 tokens) — identical across every turn.
Meanwhile the real per-request context (from API-reported `input_tokens` deltas) grew
from ~14k to ~49k tokens. The estimate never moved because `msg.content` is empty on
all tool messages.

```rust
// budget.rs:44–50 — the broken estimate
pub fn estimate(&self, system_prompt: &str, messages: &[Message]) -> usize {
    let mut total = tokens::count(system_prompt);
    for msg in messages {
        total = total.saturating_add(tokens::count(&msg.content));  // always 0 for tool messages
    }
    total
}
```

## What should happen

`estimate` must also count:
- `tc.arguments` for each `AiToolCall` in `msg.tool_calls`
- `tr.content` for each `AiToolResult` in `msg.tool_results`

Downstream impact:
1. **Dashboard Budget panel** — `context_pct` in every `SessionEvent::Metrics` record is
   wrong; the dashboard always shows a flat ~15%.
2. **Compactor** — `budget.would_overflow` uses the same estimate, so
   `context/compactor.rs` effectively never fires based on true context pressure. The
   `RunawayOutput` hard-fail is the actual pressure valve, but it fires much later
   (100 KB) and aborts rather than compacting.
3. **Phase-07 metrics** — `PhaseRun` context-efficiency fields will aggregate the wrong
   `context_pct` values from the JSONL. M10's "measure the win" thesis is built on a
   broken instrument if this isn't fixed before phase-07 lands.

## How to fix

In `executor/src/context/budget.rs`, extend `estimate` to count tool exchange content:

```rust
pub fn estimate(&self, system_prompt: &str, messages: &[Message]) -> usize {
    let mut total = tokens::count(system_prompt);
    for msg in messages {
        total = total.saturating_add(tokens::count(&msg.content));
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
    }
    total
}
```

Update the `fraction_used_returns_ratio` test (`budget.rs:137`) to use a message with
tool results so it exercises the new paths.

## Verification

- [ ] After the fix, re-running a session shows `context_pct` growing turn-over-turn.
- [ ] `Budget::estimate` on a synthetic message slice with a tool-result of N chars
      returns `tokens::count(N-char string) > 0`.
- [ ] `cargo test -p rexymcp-executor budget` passes.
