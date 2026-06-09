# M11 — Polish

**Goal:** Improve maintainability, tuneability, and quality of life without
changing any externally-visible behaviour. Four sub-goals: (1) make the
governor's hard-fail thresholds configurable via `rexymcp.toml` instead of
compile-time constants; (2) add a `rexymcp init` command that scaffolds a
documented `rexymcp.toml` so new projects can get started without reading source
(`.mcp.json` is intentionally excluded — the plugin registers via marketplace,
and a `.mcp.json` causes duplicate server conflicts in Claude Code); (3) decompose the four largest source files so no production file
exceeds the executor's RunawayOutput limit and each file has one clear concern;
(4) give the executor temporal grounding — inject the real date into its system
prompt so it stops stamping hallucinated dates in its Update Log.

**Status:** done — all seven phases (01, 02, 03, 04, 05a, 05b, 06) approved
2026-06-09. Retrospective below.

**Depends on:** M1–M10 (all complete). No new feature work here; this milestone
references existing behaviour only.

## Motivation

| Pain point | Root cause |
|---|---|
| `IDENTICAL_CALL_THRESHOLD`, `VERIFIER_PERSISTENCE_THRESHOLD`, `RUNAWAY_OUTPUT_BYTES` are compile-time constants in `governor/hard_fail.rs` | Users who want to tune them for a fast/slow model must recompile |
| No `rexymcp init` command | New users must hand-author `rexymcp.toml` by reading the source |
| `executor/src/agent/mod.rs` is 4 420 lines (≈130 KB) | Exceeds the 100 KB RunawayOutput limit — the executor can only range-read it; test suite is 80% of the file |
| `mcp/src/scorecard.rs` is 1 153 lines, `mcp/src/server.rs` is 1 225 lines, `executor/src/governor/verifier.rs` is 1 163 lines | Same pattern: large test suites buried in the same file as production code, obscuring structure |
| The executor stamps hallucinated dates (`2025-07-09`, `2025-07-15`) in its Update Log | The local model has no clock; the system prompt never told it the date, though `deps.clock` is already injected everywhere else |

## Phases

| Phase | Title | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Governor thresholds → `[governor]` config | done | feature | m |
| 02 | `rexymcp init` scaffold command | done | feature | m |
| 03 | Split `agent/mod.rs` — extract test suite | done | refactor | m |
| 04 | Split `scorecard.rs` — extract test suite | done | refactor | s |
| 05a | Split `server.rs` — extract test suite | done | refactor | s |
| 05b | Split `verifier.rs` — extract test suite | done | refactor | s |
| 06 | Inject current date into executor system prompt | done | feature | s |

## Exit criteria

- [ ] All four threshold values (`identical_call_threshold`,
  `verifier_persistence_threshold`, `runaway_output_bytes`) are read from
  `rexymcp.toml` at runtime; existing compile-time constants removed.
- [ ] `rexymcp init` writes a documented `rexymcp.toml` and refuses to overwrite
  without `--force`. No `.mcp.json` is written.
- [ ] `executor/src/agent/mod.rs` production code is ≤ 900 lines; test code
  lives in a separate `agent/tests.rs` file.
- [ ] `mcp/src/scorecard.rs`, `mcp/src/server.rs`, and
  `executor/src/governor/verifier.rs` each have tests extracted to a sibling
  `*_tests.rs` file (or a `tests/` subdir where Rust module rules require it).
- [ ] The executor system prompt opens with a `Today's date is YYYY-MM-DD
  (UTC).` line formatted from the injected clock; no date dependency is added.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Notes

### Retrospective — 2026-06-09

**Outcome: 7/7 phases approved_first_try, zero bounces, zero escalations.** The
cleanest milestone to date. Breakdown:

| Phase | Kind | Turns | Verdict |
|---|---|---|---|
| 01 governor config | feature | 84 | approved_first_try |
| 02 `rexymcp init` | feature | 47 | approved_first_try |
| 03 split `agent/mod.rs` | refactor | 36 | approved_first_try |
| 04 split `scorecard.rs` | refactor | 39 | approved_first_try |
| 05a split `server.rs` | refactor | 27 | approved_first_try |
| 05b split `verifier.rs` | refactor | 47 | approved_first_try |
| 06 datetime injection | feature | 51 | approved_first_try |

Executor throughout: Qwen/Qwen3.6-27B-FP8.

**What worked:**

- **The `sed`-move split recipe is now proven six times running** (M8 ×2, M11
  03/04/05a/05b). Prescribing a lossless `sed -n 'A,Bp' src > dst` move plus the
  `#[path = "…_tests.rs"]` single-file-module gotcha — instead of a `write_file`
  regeneration — eliminated both the transcription risk and the repeated-patch
  churn that escalated earlier large splits. Boundary line numbers were
  grep-verified at draft and re-verified at activation each time; none drifted.
- **Additive shapes dodged the mechanical-churn stall.** Phase-01 hooked config
  through `LoopDeps` additively; phase-06 added `datetime_header` composed at the
  one call site rather than widening `assemble_system_prompt` (which would have
  forced edits to its three existing tests — exactly the identical-patch churn
  that has stalled the executor). The "prefer additive change shapes" fold paid
  off directly.
- **Spec-correctness pre-injection mattered more than volume.** Phase-06's
  load-bearing pre-injection was a verified verbatim algorithm + the explicit
  correction that *chrono is not a dependency* (the original scope sketch was
  wrong). Had the spec repeated the sketch's error, the executor would have either
  blocked or added an unauthorized dep. The architect verifying the external claim
  (`grep chrono Cargo.toml`) before drafting is what made the phase land clean.

**Recurring quirk — now structurally addressed:** every phase's Update Log
self-stamped a hallucinated date (`2025-07-09`, `2025-07-10`, `2025-07-15`) and a
wrong executor identity ("rexyMCP executor" / "Claude (direct)"). Phase-06 fixes
the *date* half by injecting `Today's date is YYYY-MM-DD (UTC).` into the system
prompt — but **the running `rexymcp serve` must be restarted to pick up the
rebuilt binary** before the fix takes effect on the next dispatch (see the
known stale-server behaviour). The identity-label half is unaddressed and remains
cosmetic.

**Calibration / folds:** **none.** No new recurring pattern emerged that the docs
don't already capture. The split-calibration fold (split features by output-struct
to keep each dispatch ≤1 struct literal) remains **user-declined and held** — not
applied. A clean 7/7 milestone produced no bounces to learn from.

**Pre-existing items noted but out of scope (for a future sweep, not M11):**

- Two `eprintln!` calls in production at `mcp/src/server.rs:426` and `:450`
  (surfaced during phase-05a review).
- A stale `RUNAWAY_OUTPUT_BYTES` doc-comment reference in
  `executor/src/tools/read_file.rs:17` (surfaced during phase-01).
- The executor's identity self-labelling in Update Logs (cosmetic).
