# M29 — Cleanup

**Goal:** Fix two unrelated infrastructure gaps found during the M27/M28
validation runs: the server-authored finalize going dormant when an executor
skips the `todo→in-progress` start-flip, and a flaky (non-hermetic) tsc-resolution
test that fails under parallel `cargo test`.

**Status:** in-progress (kicked off 2026-07-09)

**Depends on:** none

## Why now

Both surfaced while dispatching M28 phase-01:

1. **Finalize dormant on a `todo` doc.** M27's server-authored `finalize_complete`
   (03a) flips a completed phase's `**Status:**` `in-progress → review` and writes
   the bookkeeping — but it is guarded on `in-progress` and **explicitly rejects
   `todo`** (`mcp/src/finalize.rs:54,60`). When an executor completes the work but
   never performed the `todo→in-progress` start-flip (the M28 AEON run did exactly
   this), the doc stays `todo`, finalize no-ops, and the phase is left with no
   bookkeeping. The fix is the same robustness thesis M27 was built on (and that
   04b applied to a bounced status line): the **server** owns the completion
   bookkeeping, so it must complete a phase regardless of whether the executor's
   best-effort start-flip happened.

2. **Flaky tsc test.** `verify_typescript_spawns_resolved_local_binary`
   (`executor/src/governor/verifier_tests.rs:888`) writes a fake
   `node_modules/.bin/tsc` shell script and then **exec's it** — a classic
   **ETXTBSY** ("text file busy") race: under parallel `cargo test`, a concurrent
   `Command::spawn` in another thread can hold a writable fd to the just-written
   file across its exec window, so the spawn intermittently fails and the test
   panics (observed once in the M28 review; passes in isolation). It violates
   STANDARDS §3.3 (deterministic tests).

## Exit criteria

- `finalize_complete` flips a **`todo`** phase doc (and its README row) to
  `review` on a `complete` result, just as it does an `in-progress` one — while
  still no-oping on an already-`review`/`done` doc and still rejecting look-alikes
  (`todoish`, `in-progressish`).
- The tsc-resolution test is deterministic (no write-then-exec race) and still
  proves that a local `node_modules/.bin/tsc` is the resolved program.
- All four gates green, including a **repeated** `cargo test` run (the flake must
  not recur); no new dependency.

## Architecture references

- `docs/architecture.md` § Status #27 (M27 — the server-authored finalize this
  extends) and #26 (M26 phase-08 — the tsc resolver whose test this fixes).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Finalize tolerates a `todo` start + hermetic tsc-resolver test ([phase-01-finalize-todo-and-tsc-test.md](phase-01-finalize-todo-and-tsc-test.md)) | done |

## Notes

Two independent fixes in different crates (`mcp/src/finalize.rs`,
`executor/src/governor/verifier_tests.rs`) bundled as one small cleanup phase
(the M14/M25 grab-bag-cleanup precedent) — each is well under one session and
neither warrants its own dispatch round-trip.

<!-- retrospective appended at milestone close -->
