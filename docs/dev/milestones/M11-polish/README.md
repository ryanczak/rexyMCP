# M11 — Polish

**Goal:** Improve maintainability, tuneability, and quality of life without
changing any externally-visible behaviour. Three sub-goals: (1) make the
governor's hard-fail thresholds configurable via `rexymcp.toml` instead of
compile-time constants; (2) add a `rexymcp init` command that scaffolds a
documented `rexymcp.toml` so new projects can get started without reading source
(`.mcp.json` is intentionally excluded — the plugin registers via marketplace,
and a `.mcp.json` causes duplicate server conflicts in Claude Code); (3) decompose the four largest source files so no production file
exceeds the executor's RunawayOutput limit and each file has one clear concern.

**Status:** in progress — phases 01, 02, 03, 04, 05a done; phase 05b remaining.

**Depends on:** M1–M10 (all complete). No new feature work here; this milestone
references existing behaviour only.

## Motivation

| Pain point | Root cause |
|---|---|
| `IDENTICAL_CALL_THRESHOLD`, `VERIFIER_PERSISTENCE_THRESHOLD`, `RUNAWAY_OUTPUT_BYTES` are compile-time constants in `governor/hard_fail.rs` | Users who want to tune them for a fast/slow model must recompile |
| No `rexymcp init` command | New users must hand-author `rexymcp.toml` by reading the source |
| `executor/src/agent/mod.rs` is 4 420 lines (≈130 KB) | Exceeds the 100 KB RunawayOutput limit — the executor can only range-read it; test suite is 80% of the file |
| `mcp/src/scorecard.rs` is 1 153 lines, `mcp/src/server.rs` is 1 225 lines, `executor/src/governor/verifier.rs` is 1 163 lines | Same pattern: large test suites buried in the same file as production code, obscuring structure |

## Phases

| Phase | Title | Status | Kind | Size |
|---|---|---|---|---|
| 01 | Governor thresholds → `[governor]` config | done | feature | m |
| 02 | `rexymcp init` scaffold command | done | feature | m |
| 03 | Split `agent/mod.rs` — extract test suite | done | refactor | m |
| 04 | Split `scorecard.rs` — extract test suite | done | refactor | s |
| 05a | Split `server.rs` — extract test suite | done | refactor | s |
| 05b | Split `verifier.rs` — extract test suite | todo | refactor | s |

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
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Notes

<!-- retrospective written here after milestone close -->
