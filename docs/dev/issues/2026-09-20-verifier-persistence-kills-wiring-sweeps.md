# `VerifierFailurePersistent` kills wiring sweeps — 7 of its 8 fires were on runs that completed on the very next dispatch

**Filed:** 2026-09-20 by the DaemonEye architect session
**GitHub:** https://github.com/ryanczak/rexyMCP/issues/10
**Area:** `executor/src/governor/hard_fail.rs::check_verifier_persistence` (`:188`),
`executor/src/agent/mod.rs` (`:1266-1284`, `:1324`), `executor/src/config.rs`
(`GovernorConfig::verifier_persistence_threshold`, `:114`)
**Related:** `2026-09-10-governor-blind-to-edit-without-gate.md` (same
family: a detector reading a *shape of change* as a *shape of failure*);
daemoneye M22 phase-03a and M21 phase-04b (the two runs that prompted this)
**Evidence:** all 435 session logs in `~/src/daemoneye/.rexymcp/sessions/`
and the three scripts in `scripts/verifier-persistence-*.py` that produced
every number below.

## Summary

`VerifierFailurePersistent` has fired **eight times** in DaemonEye's history.
Replaying each window shows **seven were wiring sweeps** — the executor
editing one distinct site per turn through a type change whose tree cannot
compile until the last site lands — and **one was a genuine stall**. Every one
of the seven completed on the next dispatch (mean 185 turns); the one stall's
two re-dispatches both hard-failed again.

The rule is `≥ threshold consecutive verifies with > 0 author diagnostics and
a non-decreasing count`. Its progress notion is "the count went down". A
wiring sweep's count *cannot* go down until the final edit — each consumer
site edited before the definition adds an error — so the rule reads finished
work as no progress. The detector is measuring the change shape.

**Raising the threshold from 6 to 10 was proposed and is declined here with
data**: no run in the corpus that this detector did *not* kill has ever
reached a streak of 6, so 10 is equivalent to switching the detector off. The
fix is to change **what extends the streak**, not how long it is. Keying the
streak on *re-editing a file already edited in the streak* keeps the one true
positive and drops all seven false ones on this corpus, with no change to the
healthy-run floor.

## The two runs that prompted this

**M21 phase-04b, `04b-6ab02346`, hard_fail at turn 87.** The phase added a
field to an existing enum variant (`PendingCall::OpenShell { host }`). Every
verify in the fatal window, with the diagnostics it saw:

```
v 72  n=1  E0061@mod.rs:857                       <- fixing a real arity error (count fell 9→3→2→1 over turns 65–72)
v 74  n=1  E0026@mod.rs:856                       <- executor/mod.rs now names the field; the variant lacks it
v 78  n=1  E0026@mod.rs:856                          args.rs edited
v 83  n=2  E0026@mod.rs:856  E0559@args.rs:499       args.rs again — a different impl block
v 85  n=2  E0026@mod.rs:856  E0027@stream.rs:693     events.rs edited (args error gone, stream error new)
v 87  n=2  E0026@mod.rs:856  E0559@stream.rs:700     stream.rs edited → fires
```

Six patches, **five distinct files**, the diagnostic set changing at four of
five transitions. The one file the sweep had not yet reached —
`src/ai/types/pending.rs`, where the field had to be *added* — was the next
edit. Resumed via `execute_phase` on the `in-progress` doc; the resume is
running as this is written. The turns 65→72 prefix is worth noting: the count
*fell* while the executor fixed real type errors, which correctly reset the
window. The rule works for the failure it was designed for.

**M22 phase-03a, `03a-6aa5fb1e`, hard_fail at turn 296.** Added a variant
(`PendingCall::ListShells` + `AiEvent::ListShells`). Fatal window:

```
v 288  n=1  E0004@mod.rs:281
v 289  n=2  E0004@mod.rs:281  E0004@stream.rs:309
v 290  n=2  (same)
v 292  n=2  (same)
v 293  n=2  (same)
v 296  n=2  (same)                                   → fires
```

Looks like a stall — the same two errors five times. It was not. Turns
287–296 patched `summary()`, `tool_name()`, the `AiEvent` variant,
`GetTerminalContextArgs`, its `ToolArgs` impl, the dispatch arm and the
dispatch fixture: **seven patches, four distinct files**, every one a
different site. The two persistent `E0004`s are the two match arms the sweep
had not reached yet (`execute_tool_call`, the stream loop). Two more arms;
re-dispatched, completed in 130 turns.

## Every fire, classified

Windows from `scripts/verifier-persistence-windows.py`; the "next dispatch"
column from the phase's next log in the same directory.

| Run | Turns | Fatal window | Files patched in window | Diagnostic set changed | Verdict | Next dispatch |
|---|---|---|---|---|---|---|
| `11-6a3ab0e1` | 151 | `E0004`×3 on one arm, then `E0609`, then a 4-error set across `ghost.rs`/`server.rs`/`session.rs` | 3 | 2 of 5 | **sweep** (new field, consumers first) | complete, 203 |
| `05a-6a740b0c` | 62 | `E0308`×8 in `search.rs`, identical set for 5 of 6 verifies | 1 | 1 of 5 | **stall** | hard_fail, hard_fail |
| `09b-6aa56b6d` | 163 | one `E0308`/`E0609` in `handlers.rs:873` while imports, call site, `Ok` arm, dispatch arm and signature of a new handler were written top to bottom | 2 | 2 of 5 | **sweep** (greenfield handler) | complete, 137 |
| `03a-6aa5fb1e` | 296 | above | 4 | 1 of 5 | **sweep** (new variant) | complete, 130 |
| `04b-6aa722f6` | 364 | `E0425`→`E0277`→`E0599`→`E0004`→`E0599`→`E0004`×4, walking dispatch → args → events → stream → pending | 5 | 5 of 5 | **sweep** (new variant) | complete, 184 |
| `09c-6aac01ff` | 41 | `E0599@mod.rs:1000` persistent (a method not yet written) plus a rotating second error as args/events/pending/stream were wired | 4 | 3 of 5 | **sweep** (new variant, method last) | complete, 232 |
| `09d-6aac58e1` | 76 | `E0026`/`E0061`/`E0277` growing 2→2→2→6→6→11 as `FgArgs` and three fn signatures changed and their call sites broke | 2 | 5 of 5 | **sweep** (signature fan-out) | complete, 303 |
| `04b-6ab02346` | 87 | above | 5 | 4 of 5 | **sweep** (new field) | *(resume in flight)* |

Seven of eight are the same shape from the outside — a definition and its
uses disagreeing, `E0004`/`E0026`/`E0027`/`E0559`/`E0063`/`E0061`, plus
`E0425`/`E0599` when the sweep references a symbol it has not written yet —
and the same shape from the inside: **a new site every turn**. The one stall
(`05a`) is the opposite on both axes: one file, one diagnostic set, six
verifies.

Note the *identical diagnostic set* is not the discriminator one would guess:
`03a` and `05a` both held an identical set for five verifies, and `04b-6aa722f6`
never repeated a set. What separates them is whether the executor's writes
were landing on new ground.

## What the current rule sees

From `hard_fail.rs:188-207`: the last `threshold` author-diagnostic counts
must all be `> 0` and non-decreasing oldest→newest. From `agent/mod.rs:1266`:
a verify runs after each write and the count pushed is `author.len()` — only
diagnostics the baseline partition attributes to the executor's own edits,
which is correct and is not the problem.

The non-decreasing clause is the progress test, and it is a good one for the
failure the detector was built for — an executor fighting a diagnostic it
cannot clear shows a flat or rising count. It is exactly wrong for a sweep,
whose count is non-decreasing *by construction*: every consumer edited before
the definition adds one error, and they all vanish at once at the end. The
rule cannot tell "six turns of flailing" from "six sites, one per turn".

## Why not raise the threshold

`scripts/verifier-persistence-streaks.py` computes the rule's streak for all
435 logs:

| Population | Max streak |
|---|---|
| the 8 runs this detector killed | 6 (truncated by the kill — their natural length is unknown) |
| every other run — 427 logs, including every `complete`, every `budget_exceeded`, every other `hard_fail` | **5** |

So at threshold 10 the detector fires on **nothing in the corpus**. That is
not a calibration; it is a disable with extra steps. The six `complete` runs
that reached 5 are also worth looking at: four are single-file iteration
(`07a-6aa9cbfa`, `11-6a3f4dbe`, `06-6a846c7b` — one lingering error in one
file, patched five times), one is a sweep that the batch-then-build guidance
had already been applied to (`03b-6aa627ae`, `E0004@stream.rs:309` five times
across three files), and `01a-6aadd016` is a real flail that oscillated below
the rule and went on to the 600-turn cap. **Threshold 6 sits one verify above
a healthy run's edge** — the same finding the 2026-09-10 issue made about the
read-only threshold.

## Proposed change: key the streak on re-editing, not on the count

`scripts/verifier-persistence-variants.py` replays every log against the
shipped rule and four candidates. Fires at threshold 6, and the false-positive
floor (max streak among `complete` runs):

| Rule | Fires | Which | Floor |
|---|---|---|---|
| **base** — shipped | 8 | all above | 5 |
| **struct** — a verify whose diagnostics are *all* structural codes (`E0004 E0026 E0027 E0559 E0063`) does not extend the streak | 3 | `05a` ✓, `09b` ✗, `09c` ✗ | 5 |
| **refile** — the streak extends only when the write that triggered the verify hit a **file already written in this streak** | **1** | **`05a` ✓** | 5 |
| **retarget** — as refile but keyed on (file, patch region) | 0 | — misses `05a` | 3 |
| **sameset** — extends only when the diagnostic set is identical to the previous verify's | 0 | — misses `05a` | 5 |

`refile` is the only variant that keeps the true positive and drops all seven
false ones. It also has the right semantics: the detector's purpose is "the
executor keeps hitting the same wall", and *re-editing the same file while the
error count does not fall* is that, whereas *first-touching a new file while
the count does not fall* is a sweep by definition. `struct` is attractive but
wrong twice — `09b` and `09c` each carried one non-structural code (`E0308`,
`E0599`) throughout their sweeps, so an all-structural test never fired for
them. `retarget` and `sameset` are too generous: `05a` patched eight different
lines of one file and the set drifted by one line at the end.

Concretely, in `agent/mod.rs` alongside `recent_verifier_error_counts`, keep
the path of the write that preceded each verify; in `check_verifier_persistence`
extend the window only while the triggering path is in the set of paths seen
since the window opened. Reset the path set when the count reaches 0, as the
count window does today.

**What refile does not fix.** Its floor is still 5: three healthy `complete`
runs iterated one file five times with one lingering error. Under refile at
threshold 6 they survive by one verify, exactly as today. Widening that margin
is a threshold question that should be answered from telemetry after the rule
change lands — the 2026-09-10 issue's advisory-first pattern
(`sample → calibrate_governor → promote`) fits, with `refile` sampled
alongside `base` so the store shows every case where they disagree.

## Daemoneye-side folds (done, recorded here so the two sides stay in sync)

- **The known remedy was not applied to the 04b spec** — that is an architect
  omission, not an executor failure. After the first occurrence (M22 03a) the
  architect's note said to batch every site of an enum change into one task
  and build only after the batch; 04b's Task 4 did not say so. Fixed in the
  resume notes and in the task text; memory widened from "adds a variant" to
  "any change to a type constructed or matched in more than one place".
- Two occurrences is a trend under daemoneye's calibration rule, not a fold.
  A third with the batch rule *in* the spec is the signal that the detector,
  not the spec, must change — this issue is filed now so that evidence lands
  somewhere.

## What this is not

- **Not a case for threshold 10.** See above: it fires on nothing.
- **Not evidence the executor model is weak.** GLM-5.3-Flash wrote correct
  code in every one of the seven sweeps; its only fault was file order, and
  the compiler cannot help it with that until the definition site lands.
- **Not calibrated beyond this corpus.** n = 8 fires, one project, three
  executor models across the era. The rule change is cheap to sample in
  advisory mode; promote it on the store, not on this table.

## Reproduce

```sh
S=~/src/daemoneye/.rexymcp/sessions
cd ~/src/rexyMCP/docs/dev/issues/scripts
python3 verifier-persistence-streaks.py  $S                       # streak + code counts, every log
python3 verifier-persistence-variants.py $S                       # the rule-variant table
python3 verifier-persistence-windows.py \
  $S/session-phase-03a-6aa5fb1e.jsonl $S/session-phase-04b-6ab02346.jsonl \
  $S/session-phase-05a-6a740b0c.jsonl                              # any window in detail
```

The scripts read the JSONL session format directly (`verify` events carry
`diagnostics[] {code,path,line,message}`; `parsed` events carry the tool
call; `session_end` carries the status) and take any logs.
