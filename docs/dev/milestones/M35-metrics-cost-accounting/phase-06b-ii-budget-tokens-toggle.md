# Phase 06b-ii: Dashboard `b`-key tokens ⇄ currency toggle for the Budget savings block

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
**Depends on:** phase-06b-i
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Add a **`b`-key toggle** to the live dashboard that flips the Budget panel's
savings block between **currency** (the `$` Baseline/Executor/Architect/Net it
shows today) and **tokens** (the token counts backing those numbers). Mirrors the
event panel's `f`-key pattern (a `ViewState` flag flipped by a key handler).

**Tokens-mode rendering (resolved with the user 2026-07-20):** keep the **same
four rows** (Baseline / Executor / Architect / Net) across Session/Milestone/
Project; in tokens mode each cell shows the token count where one is meaningful:

- **Executor** = the four executor token buckets summed (`executor_in +
  executor_out + executor_cache_read + executor_cache_write`).
- **Architect** = the four architect token buckets summed (`input +
  cache_creation + cache_read + output`).
- **Baseline** = the executor input+output tokens the baseline rate is priced over
  (`executor_in + executor_out`).
- **Net** = `—` (no token equivalent).

Currency mode is **unchanged** (06b-i behavior via `costs::scope_report`).

This is 06b-ii — the isolated interactive piece of phase-06b. 06b-i (the rewire +
cache + de-dup) is `done`; this phase only adds the toggle.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 6 ("gains a `b`-key tokens ⇄ currency toggle (the event panel's `f`-key
  pattern)").
- `mcp/src/dashboard/render.rs` — `ViewState` + `render_dashboard` (the state that
  threads into the panels).
- `mcp/src/dashboard/event_loop.rs` — the `run_loop` key-handler block with the
  `f`-key toggle to mirror.
- `mcp/src/dashboard/panels.rs` — `savings_lines` (extended with the display mode)
  and `ScopeCosts` / `ArchitectTokens` (the token sources).

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** the `savings_lines` tokens-mode tests go in
   `mcp/src/dashboard/panels.rs`'s existing `#[cfg(test)] mod tests`.
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view a
   file/line range use `read_file` (`start_line`/`end_line`), not `sed -n`/`cat` —
   `bash` refuses `sed -i` and repeated identical reads trip the governor. On a
   `patch` "0 matches"/"changed on disk", `read_file` again then re-patch.

## Current state

- **`ViewState`** (render.rs:18): `{ offset: u16, follow: bool, spinner:
  Option<usize>, filter: FilterState }`. Built in `run_loop` (event_loop.rs:48)
  and passed by `&` to `render_dashboard`.
- **`run_loop`** (event_loop.rs) tracks `offset`, `follow`, `spinner_tick`,
  `filter_state` as locals; the non-filter key match (event_loop.rs:80–110) has
  the `f`-key handler:

  ```rust
  match key.code {
      KeyCode::Char('q') | KeyCode::Esc => break,
      KeyCode::Char('f') => {
          filter_state.open = true;
          filter_state.cursor = 0;
      }
      // … Up/Down/PageUp/… …
      _ => {}
  }
  ```

  and builds the state each frame:

  ```rust
  let state = ViewState { offset, follow, spinner: …, filter: filter_state.clone() };
  total_wrapped = render_dashboard(frame, frame.area(), &data, now_ms, &state, rates);
  ```

- **`render_dashboard`** (render.rs:163): `(frame, area, data, now_ms, state:
  &ViewState, rates: BudgetRates)`; assembles the Budget panel:

  ```rust
  budget.extend(budget_lines(&data.summary));
  budget.extend(savings_lines(
      &data.summary, rates, data.milestone_costs, data.project_costs,
      data.project_escalation_count,
  ));
  ```

- **`savings_lines`** (panels.rs) — post-06b-i, computes per-scope `ScopeReport`
  via `crate::costs::scope_report` and renders Baseline/Executor/Architect/Net
  from the dollar values (`fmt_opt` for `baseline`/`net`, `fmt_dollars` for
  `executor`/`architect`), with the `has_milestone` 2-vs-3-scope layout,
  `make_row`, `debit_row`, and the `Assists:` line. It already has the three
  `ScopeCosts` in hand (Session built inline from the summary; `milestone_costs`;
  `project_costs`) — those carry the token counts tokens-mode needs.
- **Token sources:** `ScopeCosts { executor_in, executor_out, executor_cache_read,
  executor_cache_write, architect: ArchitectTokens }`; `ArchitectTokens { input,
  cache_creation, cache_read, output }` (all `u64`).

## Spec

### Task 1 — `BudgetDisplay` enum (panels.rs)

Add near `BudgetRates`:

```rust
/// Whether the Budget savings block renders dollar amounts or token counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BudgetDisplay {
    #[default]
    Dollars,
    Tokens,
}
```

### Task 2 — Thread it through `ViewState` → `render_dashboard` → `savings_lines`

- **`ViewState`** (render.rs): add `pub(crate) budget_display: BudgetDisplay`.
- **`render_dashboard`** (render.rs): pass `state.budget_display` as a new last
  argument to `savings_lines`.
- **`savings_lines`** (panels.rs): add a final `display: BudgetDisplay` parameter.

### Task 3 — `run_loop`: the `b`-key handler (event_loop.rs)

- Add a local `let mut budget_display = BudgetDisplay::Dollars;` alongside the
  other loop state.
- In the non-filter key match, add a `b` handler mirroring `f`:

  ```rust
  KeyCode::Char('b') => {
      budget_display = match budget_display {
          BudgetDisplay::Dollars => BudgetDisplay::Tokens,
          BudgetDisplay::Tokens => BudgetDisplay::Dollars,
      };
  }
  ```

- Include it in the per-frame `ViewState { … budget_display }`.

### Task 4 — Tokens-mode rendering in `savings_lines` (panels.rs)

Branch the **cell values** on `display`; keep the row structure, header, layout,
and `Assists:` line identical. In `Dollars` mode, render exactly as today (no
change). In `Tokens` mode, each scope's cells come from its `ScopeCosts` (`sc`):

- **Baseline** cell: `fmt_tokens(sc.executor_in + sc.executor_out)`.
- **Executor** cell: `fmt_tokens(sc.executor_in + sc.executor_out +
  sc.executor_cache_read + sc.executor_cache_write)`.
- **Architect** cell: `fmt_tokens(sc.architect.input + sc.architect.cache_creation
  + sc.architect.cache_read + sc.architect.output)`.
- **Net** cell: `"—"` (always, in tokens mode).

Use `u64::saturating_add` for the sums. Add a small local compact token formatter
(behavior pinned, exact suffixes not): `0 → "—"`; `>= 1_000_000 → "{:.1}M"`;
`>= 1_000 → "{:.1}k"`; else the raw count. (The Session scope's `ScopeCosts` has
cache = 0 and `architect = default`, so Session Architect renders `—` naturally.)

The header may indicate the mode (e.g. append `(tok)` in tokens mode) — pin only
that the header **differs** between modes so a test can distinguish them; exact
text is not pinned.

Keep the `debit_row` hide-when-empty behavior working in both modes (an all-empty
row stays hidden); Baseline and Net rows remain unconditional as today.

## Acceptance criteria

- [ ] Pressing `b` in the live dashboard flips the Budget savings block between
      dollars and token counts (state lives on `ViewState.budget_display`, toggled
      by the `b` handler in `run_loop`).
- [ ] `savings_lines(.., BudgetDisplay::Tokens)` renders the four rows with token
      counts per the mapping above (Executor/Architect/Baseline summed; Net `—`);
      `savings_lines(.., BudgetDisplay::Dollars)` is byte-for-byte the 06b-i
      output (existing tests pass unchanged **except** for the new trailing arg).
- [ ] The header distinguishes the two modes (a tokens-mode test can tell them
      apart).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test`
      all pass.

## Test plan

In `mcp/src/dashboard/panels.rs` `mod tests`:

- **Existing `savings_lines` tests:** update each call to pass the new final
  `BudgetDisplay::Dollars` argument; their assertions are unchanged (dollars mode
  is behavior-preserving). This proves the currency path is untouched.
- **`savings_lines_tokens_mode_shows_token_counts`** — a `ScopeCosts` (project
  and/or milestone) with distinguishable non-zero `executor_in/out/cache_*` and
  `architect.*`; call `savings_lines(.., BudgetDisplay::Tokens)` and assert the
  Executor cell shows the **summed executor token count** (e.g. a `ScopeCosts`
  totalling 1_500_000 executor tokens renders `1.5M`), the Architect cell shows
  the architect token sum, and the **Net** cell is `—`. Must **fail** if the mode
  is ignored (i.e. it renders the dollar value instead).
- **`savings_lines_tokens_mode_header_differs_from_dollars`** — render the same
  inputs in both modes; assert the header line differs (so the mode is visible).
- **`budget_display_default_is_dollars`** — `BudgetDisplay::default() ==
  BudgetDisplay::Dollars` (pins the toggle's default).

(The `b`-key handler in `run_loop` is a 3-line state flip mirroring the `f`
handler; the TUI event loop is not hermetically unit-tested here — the
`savings_lines` tokens-mode tests + the `ViewState` field are the behavioral
surface. State this in the completion Update Log; do not claim a live keypress
test.)

## End-to-end verification

Build + a note (a live TUI keypress is not hermetically checkable): confirm
`cargo build` links and `cargo test` covers the `savings_lines` tokens path. If a
real terminal is available, `cargo run -p rexymcp -- dashboard --repo .` and press
`b` to eyeball the flip — but the Update Log must rely on the unit tests as the
behavioral evidence, not a screenshot.

## Authorizations

None. No new dependencies. No `docs/architecture.md` edit — the dashboard key-
bindings are not enumerated there. No `STANDARDS.md`/`WORKFLOW.md` edit. Editing
`mcp/src/dashboard/{render.rs,event_loop.rs,panels.rs}` is in scope.

## Out of scope

- **Toggling `budget_lines`** (the raw Tokens-in/out/Tok-s block) — only the
  **savings** block toggles; `budget_lines` already shows tokens and stays as is.
- **A cache *baseline* rate or any cost-math change** — 06b-i owns the cost core;
  this phase only changes rendering + the key handler.
- **The phase-05b `LatestRun`-struct cleanup** — `profile.rs` is untouched.
- **`docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784653316110 (complete, server-authored)

**Summary:** All 4 tasks are complete. Here is the summary:

## Summary + Notes for review

Implemented the `b`-key tokens ⇄ currency toggle for the Budget savings block across 3 files:

1. **`panels.rs`**: Added `BudgetDisplay` enum (`Dollars`/`Tokens`), `fmt_tokens` formatter (0→`"—"`, ≥1M→`"{:.1}M"`, ≥1k→`"{:.1}k"`, else raw), and tokens-mode rendering in `savings_lines` that shows summed token counts for Baseline/Executor/Architect with Net as `"—"`. Header changes to `"Savings (tok)"` in tokens mode. Updated all 12 existing test calls with `BudgetDisplay::Dollars`. Added 3 new tests: `budget_display_default_is_dollars`, `savings_lines_tokens_mode_shows_token_counts`, and `savings_lines_tokens_mode_header_differs_from_dollars`.

2. **`render.rs`**: Added `budget_display` field to `ViewState` and threaded `state.budget_display` through to `savings_lines`.

3. **`event_loop.rs`**: Added `b`-key handler that toggles `budget_display` between `Dollars` and `Tokens`, and passes it into `ViewState`.

All verification commands pass: `cargo fmt --all --check`, `cargo build`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` (1024 passed, 0 failed). The `b`-key handler in `run_loop` mirrors the `f`-key pattern and is not hermetically unit-tested (the TUI event loop is not tested in isolation); the behavioral surface is covered by the `savings_lines` tokens-mode tests + the `ViewState` field.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
   Compiling rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.75s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
e_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1024 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `mcp/src/dashboard/event_loop.rs` — +9 -1
- `mcp/src/dashboard/panels.rs` — +293 -20
- `mcp/src/dashboard/render.rs` — +4 -2

**Commit:** 539e5e91c996bef1afb9a7f0c23bf9f2db6e2c93

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

