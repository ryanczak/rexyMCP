# The executor repeats a command one call at a time instead of batching it, and the governor only notices by killing the run

**Filed:** 2026-11-24 by the DaemonEye architect session as
[#19](https://github.com/ryanczak/rexyMCP/issues/19)
**Area:** `executor/templates/executor_contract.md`,
`executor/src/governor/hard_fail.rs` (`check_identical_repetition`),
`executor/src/agent/mod.rs`, `executor/src/config.rs` (`GovernorConfig`)
**Related:** #13 (a 482-repeat loop on a filtered `cargo test`), #14 (the
stall counter is blind to bash writes)
**Evidence:** DaemonEye
`.rexymcp/sessions/session-phase-10-6b05e1e9.jsonl` (M39 phase-10, GLM-5.3-Flash,
thinking on)

## Summary

One phase criterion asked for a stress loop: "passes 300 of 300 runs, 24 in
parallel", with no command given. The executor never wrote a loop, an
`xargs -P` or a script, and no bash call was refused. It ran one `cargo test`
per tool call:

- **Turns 31–36:** six near-identical variants, such as `… | tail -4 >
  /tmp/lag_t.txt && cat /tmp/lag_t.txt` and `…; cat …`, with and without
  the module path. `normalize_arguments` treats each variant as a new call.
- **Turns 37–46:** the byte-identical
  `cargo test --lib shell::host::tests::host_sends_lagged_when_subscriber_falls_behind -- --exact 2>&1 | tail -4`,
  ten times. Every result was the same `1 passed`.
- At turn 46 the run hard-failed with `IdenticalToolCallRepetition`
  (`consecutive_count: 10`; DaemonEye sets `identical_call_threshold = 10`).

The work in progress was correct, and the architect's own 300-run stress
loop on it found 0 failures. The run was lost to procedure alone.

This is the model's default, not a one-off. GLM handles "do X N times" or
"check every file" as N tool calls, and the contract never tells it
otherwise. #13 was the same shape: the same command repeated, with nothing
changing in between.

## Proposal 1: a batching rule in the executor contract

Add a short rule with a worked example to
`executor/templates/executor_contract.md`. This model copies examples far
more reliably than it follows prose.

> **Batch repeated work.** If you would run the same or nearly the same
> command more than 3 times (a stress loop, a sweep over files, one check
> per case), write one shell loop, or a script in `$TMPDIR`, that does all
> of it. Run it **once** and paste its summary line. Never repeat a tool
> call to collect repetitions.
>
> ```sh
> # stress a test 300 times, 24 in parallel; prints the failure count
> BIN=$(cargo test --lib --no-run 2>&1 | grep -o 'target/debug/deps/<crate>-[0-9a-f]*' | head -1)
> seq 300 | xargs -P 24 -I{} sh -c "$BIN --exact <test> -q >/dev/null 2>&1 || echo F" | grep -c F
>
> # one check per file, one summary
> for f in src/**/*.rs; do grep -q 'pattern' "$f" || echo "missing: $f"; done
> ```

Check that the bash classifier allows `xargs -P`, `for` loops and writing
`$TMPDIR` scripts. If it refuses any of them, the rule teaches a pattern
the executor can't use. The kill-loop classifier fix (anchored matching)
must not start refusing harmless `for` loops.

## Proposal 2: nudge on repetition before the hard-fail

Today `check_identical_repetition` does nothing until `threshold` identical
calls, then ends the run. Add an earlier, recoverable stage:

- **The trigger:** `repeat_nudge_threshold` consecutive calls, default 3.
  Each call is either identical after normalization, or **the same command
  modulo trailing output plumbing** (`| tail -N`, `| head -N`,
  `> /tmp/x; cat /tmp/x`, `2>&1`). Their tool outputs must be identical
  apart from timing fields such as `finished in 0.00s`.
- **The action:** inject one message through the same path that gate
  feedback and `task_coverage_feedback` already use:
  > You have run this command N times with the same result. Running it
  > again will give the same answer. If you need repetitions (a stress
  > loop, a sweep), write one loop or script that does all of them and run
  > it once. Otherwise, move on to the next task.
- Inject it once per run of repeats, never on every turn, and record it
  as a session event so telemetry can count nudges and whether the next
  call changed.
- **Keep the hard-fail** at `identical_call_threshold` as the backstop.

The "modulo output plumbing" normalization also closes the gap seen at turns
31–36, where re-plumbed variants of one command reset the identical-call
counter.

## Acceptance

- [ ] The contract carries the batching rule and both worked examples. A
      contract snapshot test pins the rule's heading.
- [ ] The bash classifier accepts the two example commands. Add a test.
- [ ] `repeat_nudge_threshold` exists in `GovernorConfig`, default 3, with
      `0` disabling it, plus a `rexymcp.toml` override.
- [ ] A mock-client agent test: three identical calls with identical
      outputs inject exactly one nudge message, and the run continues. A
      fourth call that differs resets the counter.
- [ ] A test: `cmd | tail -4`, `cmd | tail -4 > /tmp/x && cat /tmp/x` and
      `cmd 2>&1 | tail -6` count as the same command for the nudge, but
      not for the hard-fail.
- [ ] The hard-fail still fires at `identical_call_threshold`.
- [ ] A `NudgeInjected` session event, counted in the phase-run telemetry.
