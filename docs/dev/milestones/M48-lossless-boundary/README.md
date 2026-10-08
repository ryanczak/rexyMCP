# M48 — Lossless Boundary, Loud Backstop, Exact Repetition

**Goal:** Command output reaches the model lossless (ANSI strip, duplicate
collapse, head/tail with a recovery file — never a dropped line); a config
that switches the read-only backstop off is refused at load; and N
byte-identical consecutive tool calls terminate the run whether or not the
window mutated a file.

**Status:** in-progress *(opened 2026-10-08)*

**Depends on:** M10 (the boundary filter being retired), M34
(`NoProgressStall` and `read_only_stall_threshold`), M37 (the non-mutating
exemption being narrowed).

**Origin:** [GitHub issue #13](https://github.com/ryanczak/rexyMCP/issues/13)
and its in-repo write-up
`docs/dev/issues/2026-10-08-cargo-filter-strips-piped-output.md`
(+ `docs/dev/issues/scripts/output-filter-savings.py`).

## Why this milestone exists

A downstream run re-executed one command 482 times in 35 minutes. The
command was `cargo test … | grep -E '^test |^test result'`; the phase doc
told the model to expect the ten `test … ok` lines that `grep` selects; the
cargo filter (`output_filter.rs` `is_cargo_noise`) dropped every one of
them, with no marker saying so. The model could not tell "the tests did not
print" from "the filter ate it", and nothing in the governor stopped it:
the project's config had `read_only_stall_threshold = 0` (documented as
"disables", set by mistake months earlier), and the identical-call detector
has exempted non-mutating windows since M37.

Measured over the local telemetry store (1154 runs, 479 with both figures),
the whole boundary filter reclaims **0.041 %** of input tokens — median
0.05 % per run, p90 1.8 %. The lossy half of it is the only part that can
mislead the model; the lossless half already handles the one shape (a
long `cargo test`) where the lossy half earned anything.

## Exit criteria

- **Lossless boundary.** `bash` output passes through `normalize` +
  `compact_with_recovery` only. No line is dropped on content; the only
  reductions are ANSI escapes, exact consecutive duplicates, and the
  head/tail cut that writes the full text to `.rexymcp/output/`. The
  `is_cargo_command` / `cargo_filter` / `is_cargo_noise` /
  `parse_test_failures` / `format_failure_digest` / `filter_for_command`
  symbols are gone. The `[context] output_filter` kill-switch and the
  `OutputFiltered` session event survive (the event's `filter` label is
  always `"generic"`).
- **Loud backstop.** `Config::load` returns `Error::Config` naming the key
  when `[governor] read_only_stall_threshold` or any
  `[models."<id>"] read_only_stall_threshold` is `0`. The `rexymcp` binary
  therefore refuses to start any subcommand on such a config. The
  `rexymcp init` template no longer says "0 disables".
- **Exact repetition.** `check_identical_repetition` fires on
  `identical_call_threshold` byte-identical consecutive calls regardless of
  whether the window contains a file-mutating call. `check_oscillation`
  keeps the M37 exemption unchanged. The `calibrate-governor` replay's
  `identical_run` signal already ignores mutation state, so the replay and
  the live rule agree with no replay change.
- All four gates green at every phase boundary.

## Architecture references

- `docs/architecture.md` § Status #48 — this milestone's summary; § Status
  #10 Arc A — the filter being retired; § Status #37 — the exemption being
  narrowed.
- `executor/src/context/output_filter.rs`, `executor/src/tools/bash.rs` —
  phase-01.
- `executor/src/config.rs` `Config::load`, `GovernorConfig`,
  `ModelOverride`; `mcp/src/init.rs` — phase-02.
- `executor/src/governor/hard_fail.rs` `check_identical_repetition`,
  `window_has_mutation` — phase-03.

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | Retire the cargo filter ([phase-01-retire-cargo-filter.md](phase-01-retire-cargo-filter.md)): delete the structured cargo path and its digest; `bash` calls `compact_with_recovery` directly; two tests pin that `test … ok` lines survive the boundary | done |
| 02 | Refuse a zero stall threshold ([phase-02-refuse-zero-stall-threshold.md](phase-02-refuse-zero-stall-threshold.md)): `Config::load` rejects `read_only_stall_threshold = 0` globally and per model; init template and doc comments updated | review      |
| 03 | Identical repetition on non-mutating windows ([phase-03-identical-repetition-unexempted.md](phase-03-identical-repetition-unexempted.md)): drop the exemption from `check_identical_repetition` only; invert the two exemption tests; add the issue's `bash` shape and an agent-level test | todo |

Ordering: independent of one another; 01 first because it is the issue's
headline and the largest diff, 02 and 03 are each under 100 lines.

## Notes

- **Phase-01 review follow-up (human decision):** the shipped
  `piped_cargo_style_output_is_not_filtered` passes against the pre-phase
  code because its command starts with `sh -c`, not `cargo`. Prefixing it
  with `cargo --version >/dev/null && ` makes it a true regression test for
  issue #13 (verified to fail at `7714050`). Not folded into phase-02 or 03,
  which touch unrelated files.
- **Architect-owned follow-ups at close (not executor tasks):** README
  lines describing "a structured compressor for noisy build output"
  (§ Troubleshooting table, § Configuration reference `[context]` row, the
  M10 feature paragraph) and the two README config comments that say
  `0 disables` for `read_only_stall_threshold`; the README's `[governor]`
  comment that says the identical-call detector exempts no-mutation
  windows.
- **Downstream impact of phase-02:** any project whose `rexymcp.toml`
  carries `read_only_stall_threshold = 0` stops loading until the value is
  fixed. That is the intended loudness; the DaemonEye config is the known
  case.
- **Partial reversal of a user decision, on the user's instruction.** M37
  (2026-07-23) exempted non-mutating windows from both the oscillation and
  identical-repetition detectors, and the scope note recorded that a model
  looping `cargo build` six times is intentionally exempt. Phase-03
  reverses that for the identical-repetition detector only, on the
  2026-10-08 decision that N byte-identical calls carry no diagnostic
  information after the first repeat. The oscillation exemption, which is
  what the `sed -n` inspection loops actually needed, stays.
- **Why no "filter changed your output" marker:** once nothing lossy runs
  there is nothing to warn about. The recovery-file marker already covers
  the head/tail cut.
