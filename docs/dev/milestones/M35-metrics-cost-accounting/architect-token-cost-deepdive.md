# Architect (Claude Code) token-cost deep-dive — M35-close input

**Date:** 2026-07-21 · **Source:** `~/.claude/projects/-home-matt-src-rexyMCP/*.jsonl`
(59 transcripts), deduped by `message.id`. Token counts exact; **costs are
estimates** with representative Claude $/Mtok rates (opus 15/18.75/1.5/75,
sonnet 3/3.75/0.3/15, fable/haiku estimated) — 06c-ii formalizes the price table.

## Per-skill usage

| Skill | msgs | input | cache_wr | cache_rd | output | est $ | % |
|---|---|---|---|---|---|---|---|
| rexymcp:dispatch | 2,212 | 38k | 16.1M | 888.9M | 407k | 1,531 | 49% |
| other (design/direct) | 1,169 | 239k | 8.1M | 288.2M | 1.1M | 595 | 19% |
| rexymcp:review | 926 | 14k | 11.0M | 214.6M | 510k | 451 | 14% |
| rexymcp:architect | 788 | 198k | 6.5M | 156.8M | 1.3M | 411 | 13% |
| rexymcp:escalate | 172 | 462 | 2.8M | 54.8M | 152k | 110 | 4% |
| rexymcp:auto | 120 | 43k | 462k | 25.2M | 198k | 44 | 1% |
| **TOTAL** | 5,387 | 533k | 44.9M | 1,628M | 3.7M | **3,142** | |

## Finding — dispatch monitoring dominates

**Dispatch = 49% of total Claude-Code cost, ~= the other five skills combined**,
and it is almost entirely the in-flight monitoring poll loop:

- 2,212 dispatch turns (~2.4× review, ~2.8× architect) but only 407k output
  tokens — *less* work-output than review or architect. Dispatch does the least
  reasoning yet costs the most.
- Cost is dominated by **888.9M cache_read** (~$1,333 of $1,531): every
  `get_run_status` poll + every session-log `grep`/`tail` is a turn that
  re-reads the whole context (~400k cache_read/turn). The 15s poll-and-narrate
  loop **is** the cost.
- Review ($451) and architect ($411) — the substantive skills — cost a third of
  dispatch despite equal-or-greater output. They are efficient; dispatch is not.

## Recommendations (feed the two M35-close folds)

1. **Monitoring protocol (biggest lever).** Dispatch → confirm started → **stop
   polling** → human watches `rexymcp status`/dashboard → reap when signalled.
   A clean dispatch is ~3–5 turns vs hundreds/thousands. Removes most of the
   888.9M cache_read — plausibly **~40% of total project Claude cost.** Also drop
   turn-by-turn narration and repeated session-jsonl parsing.
2. **Cancellation policy.** Not a token lever, but paired: the poll loop existed
   partly to justify impatience-cancels. Enumerated allow-list only (human
   instruction / mis-dispatch / infra fault); slow-or-stuck is the governor's.
3. **Skill-text size (second-order).** The full SKILL.md loads per `/rexymcp:*`
   call (cache_write on first, cache_read after). Dispatch/review/architect skills
   are large; trimming the injected text cuts the per-invocation floor. Measure
   the marginal skill-load cost once the monitoring loop is removed (it's masked
   by the loop today).

**Caveat / next step:** the tiny-output heuristic under-isolated monitoring turns
(poll turns emit 51–200 output — a sentence + the tool call — not <50), so the
"monitoring overhead" is argued from turn-count + cache_read dominance, not a
per-turn tag. When 06c lands per-skill ledger data, cross-check against this.
