# M24 — Edit-Loop Recovery

**Goal:** Give the executor enough context in the `patch` tool's failure messages
to recover from a stuck edit loop on its own, instead of repeating an identical
no-op patch until the governor halts the run.

**Status:** in-progress (phase-01 drafted 2026-06-18)

**Depends on:** M22 (the `IdenticalToolCallRepetition` governor stall that
currently catches this loop as a *symptom*), M23 (the recover-first principle —
enrich the model-visible signal so it escapes before a terminator fires).

## Why now

A fresh netviz e2e run (`google/gemma-4-26b-a4b-qat`, MEDIUM-tier, phase-03,
`session-phase-03-6a342a42.jsonl`) hard-failed on a **new** mechanism — distinct
from M22's empty spiral and M23's truncation. Reconstructed from the session log:

| turn | tool | result |
|---|---|---|
| 3 | `patch` | **succeeded** — added `ETHERTYPE_IPV6` + `IPV6_HEADER_LENGTH` right after `ETHERTYPE_IPV4`, but those constants already existed lower in the file → **duplicates introduced** |
| 5 | `patch` | failed — `old_str` and `new_str` **byte-identical** (the 5-const block, unchanged) → `no-op patch: old_str equals new_str` |
| 6–10 | `patch` ×5 | identical call, identical `no-op` error each time |
| 10 | — | governor `hard_fail`: *identical patch call repeated 6 times* |

The model was trying to remove the duplicates it had just created, but submitted
an `old_str`/`new_str` pair that were the same text, so the edit was a no-op. The
error it got back — **`no-op patch: old_str equals new_str`** — is technically
correct but gives the model nothing to act on: it doesn't say *where* that text
currently sits, doesn't say the file already contains it, and doesn't suggest a
next step. With no new information, the model re-emitted the same call until
M22's `IdenticalToolCallRepetition` stall halted the run.

The governor stall is the *safety net*; it caught the loop three turns in. M24's
job — per the M23 recover-first principle — is to make the **tool's own error
message** carry enough context that the model corrects course before the net is
needed: show where the text already lives in the file, flag when it appears more
than once (the duplicate tell), and name the escape (`read_file`, then move on).

## Exit criteria

- A `patch` call whose `old_str` equals `new_str` returns a model-visible error
  that (a) states plainly that the patch would change nothing, (b) shows the
  current location and a line-numbered context window when the text is present in
  the file, and (c) names a concrete next step (`read_file`, then proceed).
- When the no-op `old_str` appears **more than once** in the file, the error flags
  the occurrence count and points at disambiguating with a larger `old_str` — the
  direct signal for the duplicate-introduction failure above.
- When the no-op `old_str` is **absent** from the file, the error says so and
  directs the model to `read_file` rather than fabricating a location.
- The governor `IdenticalToolCallRepetition` stall is **unchanged** — it remains
  the backstop; this milestone only enriches the signal upstream of it.
- All pre-existing `patch` tests pass unmodified.

## Architecture references

- `docs/architecture.md` § Status #24 (added at kickoff).
- `executor/src/tools/patch.rs` — the `patch` tool: the early no-op guard
  (`old_str == new_str`, lines 82–88), the `match_count` match arms (135–190),
  and the `fuzzy_hint` helper (194+) whose windowed-context shape the new no-op
  hint mirrors.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | `patch` no-op recovery context ([phase-01-patch-noop-context.md](phase-01-patch-noop-context.md)) | todo |

Single phase for now. If the follow-up e2e shows the model also stalls on the
ambiguous-match (`n =>`) or zero-match (`0 =>`) arms, a phase-02 would extend the
same enrichment there — held until the data shows a need.

## Notes

### Scope decisions (2026-06-18, with the user)

- **Enrich the tool error, do not add a new terminator.** M22's
  `IdenticalToolCallRepetition` already bounds the loop; the gap is *recovery*, not
  *termination*. Mirrors M23's recover-first call. No governor change this
  milestone.
- **Only the `patch` tool's no-op arm.** `patch_lines` has separate range-based
  error semantics and was not implicated in the failure; the other `patch` arms
  (`0 =>` fuzzy_hint, `n =>` ambiguous) already give actionable context. Scope is
  the one arm that returns a dead-end message.
- **Surface context, don't auto-fix.** The tool shows the model where the text
  lives and flags duplicates; it does not try to detect or remove the duplicates
  itself. The model decides — the tool just stops withholding the information it
  already has.
