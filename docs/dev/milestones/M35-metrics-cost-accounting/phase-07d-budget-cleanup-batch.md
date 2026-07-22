# Phase 07d: M35-close cleanup batch — Profile help, remove Budget Assists row, Budget toggle-hint text

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
**Depends on:** phase-07c
**Estimated diff:** ~90 lines
**Tags:** language=rust, kind=fix, size=s

## Goal

Three small, independent M35-close cleanup fixes (user-directed, 2026-07-22):

1. **[#1] Fix the `profile` CLI help.** 07c's executor invented an **inaccurate** `about`
   line — `"Show the profile: per-model token and latency breakdown"` — but `profile` has
   **no latency** metric. Correct the wording.
2. **[#2] Remove the `Assists:` row** from the dashboard Budget panel's savings block.
3. **[#4] Change the Budget border hint** from `[b=$/tok]` to `[b=toggle view]`.

These are three unrelated one-to-few-line changes across three files; they do **not**
interact. (The related trailing-blank-row and negative-value-alignment fixes are separate
phases — see Out of scope.)

## Architecture references

Read before starting:

- `mcp/src/main.rs:221` — the `Profile` command's `about` doc-comment (#1 target). Its
  `--cost` flag doc (~247) and the MCP `model_profile` description in `mcp/src/server.rs`
  (~604) describe what `profile` actually does.
- `mcp/src/dashboard/panels.rs` — `savings_lines` (506–709): the two `Assists:` push
  lines (659, 707), the `project_escalation_count` param (511), the doc comment (504),
  and the two Assists-asserting tests (~1850, ~2052) (#2 targets).
- `mcp/src/dashboard/render.rs:235` — `panel(" Budget [b=$/tok] ", budget)` (#4 target).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Editing discipline:** `patch`/`patch_lines`, never a whole-file `write_file`;
   `read_file` with line ranges, never `sed -n`/`cat`, never the same command twice.
   `cargo build -p rexymcp` after each file. **If a syntax/build error appears, run
   `cargo build` once and read the compiler's error location — do NOT hunt for it by
   re-reading the file in a loop** (that repeated inspection is what oscillation-hard-failed
   an earlier run). Read once, fix, rebuild.

## Current state

**[#1] `Profile` `about`** (`main.rs:221–224`) — the inaccurate line to fix; keep the
"See also" line beneath it:

```rust
    /// Show the profile: per-model token and latency breakdown
    ///
    /// See also: runs, scorecard, costs, calibrate-governor.
    Profile {
```

What `profile` actually does: it renders a **per-(model, tag) capability profile**
(strengths — gate-pass rate, approved-first-try rate, reliability means — and ranked
failure classes); with `--cost` it instead reports **tokens & cost per approved phase**.
There is no latency metric.

**[#2] the `Assists:` rows** in `savings_lines` — one in the tokens-mode branch
(`panels.rs:659`) and one in the dollars-mode branch (`panels.rs:707`), both:

```rust
    out.push(Line::from(format!("  Assists: {project_escalation_count}")));
```

The param feeding them (`panels.rs:511`): `project_escalation_count: u32,`. After the two
rows are removed it becomes unused. The doc comment (`panels.rs:504`) ends
"…plus an Assists count." Two tests assert the row:
- `savings_lines_omits_zero_debit_rows` (~1830) — comment says "header + Baseline + Net +
  Assists = 4 lines" and asserts `texts[3].contains("Assists:")` (line ~1850).
- `savings_lines_assists_shows_project_escalation_count` (~2052) — a dedicated Assists test.

**[#4] the Budget border** (`render.rs:235`):

```rust
    frame.render_widget(panel(" Budget [b=$/tok] ", budget), budget_area);
```

## Spec

### Task 1 — correct the `profile` help (main.rs)

Rewrite the `Profile` `about` line (main.rs:221) to describe `profile` accurately — a
per-model (× tag) **capability** profile, with `--cost` giving tokens & cost per approved
phase. **Remove the false "latency" claim.** Keep the blank `///` line and the
`/// See also: runs, scorecard, costs, calibrate-governor.` line exactly as they are. Exact
wording is yours; it must be accurate (no "latency") and stay a first-line short `about`.

### Task 2 — remove the Budget `Assists:` row (panels.rs)

1. Delete **both** `out.push(Line::from(format!("  Assists: {project_escalation_count}")));`
   lines (the tokens-mode one at ~659 and the dollars-mode one at ~707).
2. The `project_escalation_count` param is now unused. **Do not remove the param** — it is
   passed positionally at ~15 call sites (removing it is a wide signature cascade, out of
   scope). Instead **rename it to `_project_escalation_count`** (the intentionally-unused
   idiom) so `-D warnings` stays green, and add a one-line `//` note that it is retained to
   avoid a call-site cascade / that full removal is a deferred cleanup.
3. Update the `savings_lines` doc comment (~504): drop the "plus an Assists count" clause
   so the doc matches the new output.
4. Fix `savings_lines_omits_zero_debit_rows` (~1830): the block now yields **3** lines
   (header + Baseline + Net) — update the "= 4 lines" comment to "= 3 lines" and **remove**
   the `assert!(texts[3].contains("Assists:"), …)` line. Leave the Baseline/Net/Executor-
   omitted assertions intact.
5. **Delete** the `savings_lines_assists_shows_project_escalation_count` test (~2052) — it
   verifies removed behavior.

Do **not** touch `DashboardData.project_escalation_count` (mod.rs), its computation, the
`render.rs` call site, or the other `savings_lines` call sites — the field stays (it is
still passed into the now-underscored param), so nothing outside `panels.rs` changes.

### Task 3 — Budget border hint text (render.rs)

Change `panel(" Budget [b=$/tok] ", budget)` (render.rs:235) to
`panel(" Budget [b=toggle view] ", budget)`. One string literal; nothing else.

## Acceptance criteria

- [ ] `rexymcp profile --help` shows an accurate `about` with **no "latency"**, and still
      shows the "See also: runs, scorecard, costs, calibrate-governor." line.
- [ ] The Budget panel savings block renders **no `Assists:` row** in either tokens or
      dollars mode; `savings_lines` output contains no line starting with `Assists:`.
- [ ] `-D warnings` is clean (the retained param is underscored, not flagged).
- [ ] The Budget panel border reads `Budget [b=toggle view]` (not `[b=$/tok]`).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] No change to `DashboardData.project_escalation_count`, its mod.rs computation, or any
      `savings_lines` call site; no signature change to `savings_lines`.

## Test plan

- **`savings_lines_omits_zero_debit_rows`** (edited) — now asserts the block is header +
  Baseline + Net (no Assists row); the "= 4 lines"→"= 3 lines" comment and the dropped
  `texts[3]` Assists assertion pin the removal.
- **Add** a small assertion (in an existing dollars-mode `savings_lines` test, or a new
  `savings_lines_has_no_assists_row`) that **no** rendered line starts with `Assists:` —
  the mutation-sensitive pin that the row is gone.
- The `savings_lines_assists_shows_project_escalation_count` test is **deleted**.
- No test is needed for the two help/border string changes (rendering, verified E2E below).

## End-to-end verification

1. `cargo run -p rexymcp -- profile --help` — quote the corrected `about` line (no
   "latency") and confirm the "See also" line is still present.
2. `cargo run -p rexymcp -- dashboard --repo .` is a live TUI (can't be captured
   hermetically) — instead confirm the border string in the diff (`render.rs:235` reads
   `[b=toggle view]`) and rely on the unit tests for the Assists removal. State this.

## Authorizations

- Editing `mcp/src/main.rs` (Profile `about`), `mcp/src/dashboard/panels.rs`
  (`savings_lines` + its tests), and `mcp/src/dashboard/render.rs` (Budget border) is in
  scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **[#3] The trailing blank row** at the bottom of the Session/Budget/Context panels — a
  **separate phase** (the header-band height, which depends on this phase's Assists removal
  landing first, since it changes the Budget panel's height). Do **not** touch
  `render.rs`'s `Layout::vertical([Length(11), …])` header band here.
- **[#5] Negative-value column alignment** in the Budget savings block (parenthesized
  debits not aligning with non-parenthesized values) — a **separate phase**; leave
  `savings_lines`' `make_row`/`paren` value formatting untouched here.
- **Full removal of the `project_escalation_count` plumbing** (the `DashboardData` field,
  its mod.rs computation, the param, and the ~15 call-site args) — deferred; the wide
  signature cascade is not worth it in this batch.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
