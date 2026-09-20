# Governor is blind to "editing without checking" — and daemoneye had "checking without progressing" switched off

**Filed:** 2026-09-10 by the DaemonEye architect session
**Area:** `executor/src/governor/hard_fail.rs`, `executor/src/config.rs` (`GovernorConfig`)
**Related:** M34 (stall hardening), M37 (read-only exemption), M25-daemoneye (governor port)
**Evidence:** four session logs in `~/src/daemoneye/.rexymcp/sessions/` — two
`budget_exceeded` at the 600-turn cap, two `complete` for contrast — and the two
scripts in `scripts/` that produced every number below.

## Summary

Two DaemonEye phases hit the 600-turn `max_turns` cap in one day. Both were
runs with **no path to completion** (each traced to an architect spec defect,
folded separately in daemoneye's WORKFLOW.md), so the cap terminated them
correctly — raising it was considered and declined. The question this issue
answers is: **when was each run obviously lost, and could the governor have
ended it there?**

Answer: both were obvious by roughly **turn 120–165**, i.e. at about a quarter
of the budget, and they failed in two *different* shapes:

| Run | Shape | Obvious at | Why the governor stayed silent |
|---|---|---|---|
| `06a-6aa3467a` | **editing without checking** — 156 patches to one file, **3 gate runs, last at turn 46**, net diff +8 lines | ~turn 118 | every patch resets `NoProgressStall`; identical greps were interleaved with patches so `check_identical_repetition` never saw 6 in a row; novelty stayed at 13–14/24, above the floor of 6 |
| `05a-6aa2edc0` | **checking without progressing** — 137 gate runs, a **172-call read-only streak (turns 65–236)**, last green test at turn 2 | ~turn 125–165 | `NoProgressStall` **would have fired at ~turn 125** with the default threshold of 60 — but daemoneye's `rexymcp.toml` sets `read_only_stall_threshold = 0` |

The first is a genuine detector gap. The second is a config defect with a
misleading comment (see § "The disabled detector"). Both are worth fixing; the
first needs a new signal, the second needs one line.

## Measurements

All numbers from `scripts/budget-exceeded-dynamics.py` and
`scripts/budget-exceeded-gates.py` run over the four logs. "Mutation" means a
`Category::Write` tool (`patch`/`patch_lines`/`write_file`), matching
`tools::mutates_files`. "Gate" means a `bash` call matching
`cargo (test|build|clippy)`.

### Failed: `session-phase-06a-6aa3467a.jsonl` (600 calls, budget_exceeded)

```
mutations: 171   last mutation at turn 570
gate runs: 3     first at 3   last at 46          <- 554 turns without compiling
mutations since last gate: crossed 10 @81, 20 @118, 30 @151, peak 162 @570
distinct normalized calls: 103 / 600  (17% novel)
top repeats:
   181x  read_file:stream.rs
   156x  patch:stream.rs
   111x  bash:grep -n 'let mut run_log\|let _run_log' src/daemon/stream.rs
read_evicted per 50 turns from t100 onward: 14-17  (re-reading its own reads out of context)
novelty_sample events: turns 24, 26, then NOTHING until 594   (distinct_targets 13-14, constant)
```

The 156 patches netted **+8 lines**. This is edit/revert oscillation across
a file, not progress — and it is invisible to every current detector because
each detector treats a `Write` as progress.

### Failed: `session-phase-05a-6aa2edc0.jsonl` (599 calls, budget_exceeded)

```
mutations: 64    last mutation at turn 597
gate runs: 137   61 red, 16 green (build), 60 unclassified
longest no-mutation streak: 172 calls, turns 65..236   <- NoProgressStall territory
last GREEN *test* run: turn 2;  tail: 24 consecutive red gates (508..598)
distinct normalized calls: 411 / 599  (69% novel)      <- novelty detectors cannot see this one
```

High novelty, constant gate-running, zero green tests after the refactor. The
only signal that separates it from a healthy run is the 172-call read-only
streak — exactly what `check_read_only_stall` measures.

### Successful, for the false-positive floor

```
05a-6aa30032 (complete, 157 calls):  max mutations-since-gate 4;  longest no-mut streak 27;  max consecutive red 2
05b-6aa30db0 (complete, 308 calls):  max mutations-since-gate 18; longest no-mut streak 60;  max consecutive red 2
```

Note `05b`'s **60-call read-only streak at turns 1–60** — its initial read-in
phase. A read-only threshold of 60 sits *exactly* on a healthy run's edge; the
M34 demotion 20 → 60 was presumably for this reason. 100 is the safe floor
from this corpus.

## Why the current detectors miss both

From `executor/src/governor/hard_fail.rs` and `executor/src/config.rs`
(defaults: `identical_call_threshold 6`, `oscillation_window 8`,
`read_only_stall_threshold 60`, `novelty_window 24`,
`novelty_distinct_floor 6`, `novelty_action Advisory`):

1. **`check_read_only_stall` resets on any `mutates_files` call** (`:318-323`).
   Correct for its purpose — but it means a run that *patches constantly and
   never compiles* can never trip it. 06a made 171 mutations; the counter
   never exceeded a handful.
2. **`check_identical_repetition` needs `threshold` consecutive identical
   calls.** 06a ran one byte-identical grep 111 times, interleaved with
   patches and reads, so no run of 6 ever formed.
3. **`check_oscillation` needs ≤ `oscillation_distinct_max` (2) distinct calls
   in an 8-window.** 06a's window held read/patch/grep/read — 3+ distinct —
   every time.
4. **`measure_novelty` saw 06a clearly and reported nothing actionable.**
   `distinct_targets` held at 13–14 of 24 — above the floor of 6, and
   `NoveltySample` is deduped on that value (`agent/mod.rs:1300-1318`), which
   is why the log is silent from turn 26 to 594: *the number never changed*.
   In advisory mode a constant-but-above-floor novelty is, by construction, not
   a signal. The 06a pattern is not low novelty of *targets*; it is high
   activity with **no verification**.

None of the four measures the thing that distinguishes 06a: **mutations
accumulated since the last gate run.**

## The disabled detector (daemoneye config defect, but the comment is the bug)

`~/src/daemoneye/rexymcp.toml:48-52`:

```toml
read_only_stall_threshold = 0    # consecutive non-mutating tool calls → hard-fail; resets on
                                  # any patch/write_file (0 disables). rexyMCP default.
                                  # Was 100 (raised when the default was 20, for large
                                  # size=l phases that read many files before the first
                                  # write); reset to the default 2026-07-25.
```

The rexyMCP default is **60** (`config.rs:175`), not 0. Whoever reset it on
2026-07-25 believed they were restoring the default and instead disabled the
detector. With 60 it fires on 05a at ~turn 125 (172-streak begins at 65); with
the prior local value of 100 it fires at 165. Either ends the run at a quarter
of the budget.

**Resolved on the daemoneye side 2026-09-10: set to 200** (PE decision, "for
now"). 200 catches 05a at turn ~265 — still under half the budget — and clears
the healthy 60-call read-in phase 05b opened with by a wide margin, so it is a
conservative re-enable rather than a calibrated threshold. The comment was
corrected to state the real default.

**A detail that makes this worse than a typo:** daemoneye's `rexymcp.toml` is
**gitignored**. The 0 never appeared in a diff, a review, or `git log`; it was
found only by reading the file while writing this issue. Any per-project
governor override lives outside version control by design, which means a
detector can be silently disabled with no artifact anywhere.

This is daemoneye's line to fix (done), but rexyMCP can make the mistake harder:
`rexymcp init`'s generated comment for this key should state the real default,
and `rexymcp doctor` (or config load) could warn when a hard-fail detector is
set to 0 — "disabled" is a legitimate choice but should never be an accident.

## Proposed detector: `EditWithoutGate`

A new signal with the same shape as `NoProgressStall`, over the same
`VecDeque<ToolCallSnapshot>`:

> **`mutations_since_gate >= threshold`** — count `Category::Write` calls since
> the most recent gate run; a gate run is a `Run`-category call whose command
> matches the project's `[commands]` `build`/`test`/`lint` (already known to the
> executor), so this needs no new classification.

- **Catches 06a at turn 118** (threshold 20) or **~130** (threshold 25).
- **False-positive floor on this corpus: 18** (05b). 25 is the safe default;
  it should be **advisory-first** like M34's novelty detector, sampled into the
  session log as an `EditWithoutGateSample { mutations_since_gate }`, and
  promoted after `calibrate_governor` replays it against the store.
- Resets on any gate run regardless of outcome — a red gate is still the
  executor *checking*, which is the behaviour we want to reward. The
  red-vs-green question is a *different* detector (below).

Why not just lower `identical_call_threshold` or widen `oscillation_window`?
Because 06a was **69%-of-the-time doing something new** by those measures. It
was not repeating itself; it was editing blind. The distinguishing variable is
verification, not variety.

## Secondary proposal: `RedGateStreak`

05a's tail — 24 consecutive red gates with the last green *test* at turn 2 —
suggests a second signal: **N consecutive gate runs with no green test result.**
Floor on the healthy runs is 2. This needs the gate-outcome classification the
executor already does for gate-feedback re-injection
(`gate_feedback_repeat_threshold`), split by `test` vs `build` — a green build
with red tests is exactly 05a's steady state. Lower priority: with
`read_only_stall_threshold` restored, 05a is already caught earlier by the
existing detector.

## What this is not

- **Not a case for raising `max_turns`.** Both runs would have spent any
  additional budget the same way. The cap did its job; the ask is to do it
  ~450 turns sooner.
- **Not evidence the executor model is weak.** Both failures traced to spec
  defects (a false lifetime claim; a phantom sixth exit path + line-number
  anchors). A stronger model would have thrashed on the same unsatisfiable
  criteria. The governor's job is to bound the cost of an architect's mistake.
- **Not calibrated.** n=2 failures, n=2 successes, one model, one project. The
  telemetry store holds two more `budget_exceeded` runs (project `caede0f9`,
  Qwen3.6, phases 04 and 06) whose session logs would widen the corpus before
  any threshold is promoted from advisory.

## Reproduce

```sh
cd ~/src/daemoneye
python3 ~/src/rexyMCP/docs/dev/issues/scripts/budget-exceeded-dynamics.py \
  .rexymcp/sessions/session-phase-05a-6aa2edc0.jsonl \
  .rexymcp/sessions/session-phase-06a-6aa3467a.jsonl \
  .rexymcp/sessions/session-phase-05a-6aa30032.jsonl \
  .rexymcp/sessions/session-phase-05b-6aa30db0.jsonl
python3 ~/src/rexyMCP/docs/dev/issues/scripts/budget-exceeded-gates.py <same four>
```

Both scripts read the JSONL session format directly (`parsed` → `tool_result`
pairs, `metrics`, `read_evicted`, `session_end`) and take any number of logs.
