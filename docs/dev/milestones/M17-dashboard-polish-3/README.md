# M17 — Dashboard Polish (Round 3)

**Goal:** Six dashboard refinements — restore the dog-chasing-brain spinner at
full panel width, tidy the Session/Budget/Reclaim panel labels, surface the
active milestone name, scroll overflowing task titles, and upgrade Activity-panel
syntax highlighting to cover Markdown and extension-detected source files.

**Status:** in-progress

**Depends on:** M15 (complete), M16 (complete)

**Exit criteria:**
- [ ] The Session panel's `last update:` line sits directly under `duration:`,
      and every label in the Session, Budget, and Reclaim panels is
      capitalized (`Phase:`, `Tokens in:`, `Events:`, …); `$ saved:` is left
      as-is (a symbol, not a word).
- [ ] The liveness spinner is the dog-chasing-its-brain animation again, and it
      spans the full Session-panel inner width (the chase distance scales with
      width); it disappears once the session ends.
- [ ] The Session panel shows a `Milestone:` line at the top with the active
      milestone's human-readable name, derived from the milestone directory that
      holds the running phase's doc; long names truncate with `…`.
- [ ] Task titles in the Tasks panel that exceed the panel width pan
      left-and-right within the available space; titles that fit do not move.
- [ ] Activity-panel Completion bodies are highlighted as Markdown, and
      `read_file` tool-result bodies are highlighted using the language inferred
      from the file's extension.
- [ ] All gates pass: `cargo fmt --all --check`, `cargo build` (zero warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`.

## Architecture references

- `mcp/src/dashboard/panels.rs` — `session_lines`, `spinner_line`,
  `last_update_line`, `budget_lines`, `reclaim_lines`, `dollars_saved_line`,
  `tasks_lines`, `truncate_title`
- `mcp/src/dashboard/render.rs` — header-band assembly, `session_inner_width`,
  the `session_lines` / `last_update_line` / `spinner_line` composition, the
  `tasks_lines` call site
- `mcp/src/dashboard/mod.rs` — `DashboardData`, `load_data`
- `mcp/src/dashboard/event_loop.rs` — `run_loop`, the `spinner_tick` counter
- `mcp/src/dashboard/transcript.rs` — `transcript_lines`, `record_lines`
- `mcp/src/dashboard/highlight.rs` — `highlighted_body_lines`, `detect_syntax`,
  `completion_body_lines`, `body_lines` (syntect-based)

## Phases

| #  | Phase | Status | Kind | Size |
|----|-------|--------|------|------|
| 01 | Move `last update:` under `duration:` + capitalize panel labels ([phase-01-labels.md](phase-01-labels.md)) | done | feat | xs |
| 02 | Restore full-width dog-chasing-brain spinner ([phase-02-spinner.md](phase-02-spinner.md)) | todo | feat | s |
| 03 | `Milestone:` row in the Session panel ([phase-03-milestone.md](phase-03-milestone.md)) | todo | feat | s |
| 04 | Scroll overflowing task titles ([phase-04-task-scroll.md](phase-04-task-scroll.md)) | todo | feat | m |
| 05 | Markdown + extension-detected syntax highlighting ([phase-05-highlighting.md](phase-05-highlighting.md)) | todo | feat | m |

Phase 05 is the last in-scope M17 phase; it closes the milestone once approved.

## Notes

### Why M17 exists

A daily-use pass over the dashboard surfaced six gaps after M15 (Round 2):

1. **The spinner lost its character.** A refactor simplified the
   dog-chasing-brain animation down to a single dog doing a triangle-wave walk.
   The original (a dog chasing its own brain until it overtakes it, with a
   `💨` burst) should come back — but parametric on width so it fills the panel.
2. **`last update:` drifted from `duration:`.** It belongs immediately under
   `duration:` in the Session panel.
3. **Panel labels read as lowercase fragments.** Capitalizing the first letter
   of each label (`Phase:`, `Tokens in:`, `Events:`) reads cleaner.
4. **No milestone context.** The Session panel names the phase but not the
   milestone. The milestone name is encoded in the milestone *directory* name
   (`docs/dev/milestones/M15-dashboard-polish-2/`), so it can be derived without
   any new config or session-event field.
5. **Long task titles get clipped.** Titles wider than the Tasks panel are
   truncated with `…`; panning them back and forth shows the whole name.
6. **Activity highlighting is incomplete.** The panel already highlights
   tool-result bodies via **syntect** (`highlight.rs`), but Markdown is never
   detected (so Completion prose is unstyled) and language detection is
   content-heuristic rather than extension-based (so a `read_file` of a `.py` /
   `.ts` / `.sh` file is often left plain).

### Display-only constraint, and the one dependency note

Phases 01–05 are **all** display-layer. **No new `SessionEvent` variants** in any
phase. **No new `Cargo.toml` dependency in any phase** — phase 05 was originally
scoped to add tree-sitter, but the `mcp` crate already depends on `syntect`
(`mcp/Cargo.toml:21`), a full Sublime-grammar highlighter whose default set
already bundles Rust, Python, JSON, JavaScript/TypeScript, Markdown, Bash/Shell,
TOML and more. Phase 05 therefore *extends the existing syntect path* rather than
introducing a parallel highlighter — strictly less code, zero new deps, broader
language coverage. (Decision made with the user 2026-06-11.)

### Pre-injected anti-stall shapes

Two phases change a function the dashboard tests call from many sites. To dodge
the documented mechanical-multi-site-churn stall (M10/M12 calibration), the specs
pin **low-churn shapes**:

- **Phase 03** composes the `Milestone:` line in `render.rs` (the established
  `last_update_line` / `spinner_line` "optional line pushed in render" precedent)
  rather than changing `session_lines`' signature — so its 8 test call sites stay
  untouched.
- **Phase 05** adds a new `record_lines_with_lang(rec, hint)` and leaves
  `record_lines(rec)` as a zero-arg-delegating wrapper, so the existing
  `record_lines` / `record_text` test call sites stay untouched.

Only **phase 04** genuinely must change `tasks_lines`' signature (the scroll tick
has to reach the per-task clip math); its spec enumerates all 6 call sites
(1 prod + 5 test) so the executor traverses them in one pass.
