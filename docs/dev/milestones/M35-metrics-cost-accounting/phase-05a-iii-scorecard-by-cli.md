# Phase 05a-iii: `scorecard --by model|tag|settings` CLI + dropped columns; retire the Settings wrapper

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** review
**Depends on:** phase-05a-ii
**Estimated diff:** ~200 lines (deletions + a clap flag + a formatter rewrite)
**Tags:** language=rust, kind=feature, size=m

## Goal

Finish the scorecard unification on the CLI side:

1. Add a `rexymcp scorecard --by model|tag|settings` selector so the CLI can
   render any of the three dimensions from the 05a-i core (today it is
   settings-only).
2. Display the previously-computed-but-dropped columns **repairs-per-call**,
   **verifier-retries**, and **wall-clock** (the milestone theme: every recorded
   number is displayed or deleted — these were computed into every row and shown
   nowhere).
3. **Retire the last wrapper**: delete `aggregate_by_settings` and
   `SettingsScorecardRow`; the CLI now consumes `aggregate_scorecard` +
   `ScorecardBucket` directly (the MCP tool already does, from 05a-ii).
4. **Remove the `#[allow(dead_code)]` on `ScorecardDimension`** — `--by model`
   gives `ScorecardDimension::Model` its first *production* consumer, so the
   variant is no longer dead and the allow (carried since 05a-i) must go. This
   closes the spec_bug thread flagged in the 05a-i and 05a-ii review verdicts.

After this phase the scorecard has **one** aggregation core, **one** row type
(`ScorecardBucket`), **no** wrappers, and **no** `#[allow(dead_code)]`.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 5 (this phase completes the scorecard clause of it).
- `mcp/src/scorecard.rs` — the core `aggregate_scorecard` + `ScorecardBucket`
  (05a-i), and the `aggregate_by_settings`/`SettingsScorecardRow` Settings
  wrapper being retired here.
- `mcp/src/scorecard_cli.rs` — the current settings-only CLI loader/formatter
  being generalized.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** new `#[test]` fns stay **INSIDE** the existing
   `#[cfg(test)] mod tests` blocks (`mcp/src/scorecard_tests.rs` — `include!`d
   into `scorecard.rs`'s test module — and `mcp/src/scorecard_cli.rs`'s own
   `mod tests`).
6. **Editing discipline:** edit with `write_file`/`patch`/`patch_lines`. To view
   a file or a line range, use `read_file` (`start_line`/`end_line`), not
   `sed -n`/`cat` — `bash` refuses `sed -i` and repeated identical reads trip the
   governor. On a `patch` "0 matches"/"changed on disk", `read_file` again then
   re-patch.

## Current state

- **The core (unchanged, reused):** `pub fn aggregate_scorecard(runs,
  dimension: ScorecardDimension, filter) -> Vec<ScorecardBucket>`
  (scorecard.rs:105). `ScorecardDimension { Model, Tag, Settings }`
  (scorecard.rs:42) currently carries `#[allow(dead_code)]` (scorecard.rs:43)
  because `Model` had no production consumer. `ScorecardBucket` (scorecard.rs:54)
  has `key: String` plus every metric field, including the three to be displayed:
  `repairs_per_call_mean: f64`, `verifier_retries_mean: f64`,
  `wall_clock_s_mean: f64`.
- **The Settings wrapper (delete):** `pub fn aggregate_by_settings(runs, filter)
  -> Vec<SettingsScorecardRow>` (scorecard.rs:240) and `pub struct
  SettingsScorecardRow` (scorecard.rs:9). `SettingsScorecardRow` is field-
  identical to `ScorecardBucket` **except** its key field is named `settings`
  (not `key`). Its only production consumer is `scorecard_cli.rs`.
- **The CLI (generalize):**
  - `mcp/src/scorecard_cli.rs`: `load_settings_scorecard(config_path,
    telemetry_path, filter) -> Result<Vec<SettingsScorecardRow>, String>`
    (reads → folds reviews → `aggregate_by_settings`) and
    `format_settings_scorecard(rows: &[SettingsScorecardRow]) -> String` (a
    table: `MODEL SETTINGS N GATES PARSE_FAIL LENGTH_FIN AFT_RATE TURNS_MEAN
    PEAK_CXT RECLAIMED`).
  - `mcp/src/main.rs`: the `Commands::Scorecard { config, model, tags, min_runs,
    telemetry_path, json }` clap variant (main.rs:166) and its dispatch arm
    (main.rs:611) which builds a `ScorecardFilter`, calls
    `load_settings_scorecard`, then prints `to_string_pretty(&rows)` (if `json`)
    or `format_settings_scorecard(&rows)`.
- **Test consumers:** `scorecard_tests.rs` calls `aggregate_by_settings(&runs,
  …)` at 12 sites and reads `.settings` on the result at 5 sites; it also has a
  now-**tautological** test `aggregate_scorecard_settings_matches_wrapper` (from
  05a-i, ~scorecard_tests.rs:780) that compares `aggregate_scorecard(_, Settings,
  _)` against `aggregate_by_settings` — once the wrapper is a test-local shim
  delegating to the core, this compares the core to itself. `scorecard_cli.rs`'s
  own tests construct two `SettingsScorecardRow { … }` literals and call
  `load_settings_scorecard`/`format_settings_scorecard`.

## Spec

### Task 1 — Remove the dead-code allow

In `mcp/src/scorecard.rs`, delete the `#[allow(dead_code)]` attribute on
`ScorecardDimension` (scorecard.rs:43). It becomes unnecessary once Task 4 wires
`--by model` (a production construction of `ScorecardDimension::Model`). **Do
this in the same phase as Task 4** — if you remove the allow before the `--by`
wiring exists, `cargo clippy -D warnings` will fail on `Model` being unconstructed
(build stays green — it is a warning — so intermediate turns are fine; the final
clippy gate is the enforcer).

### Task 2 — Delete the Settings wrapper + struct

In `mcp/src/scorecard.rs`, delete `pub fn aggregate_by_settings` (scorecard.rs:240
through its closing `}`) and `pub struct SettingsScorecardRow` (the
`/// One row of the model × settings matrix.` doc comment at scorecard.rs:9
through the struct's closing `}` at :37). Leave `aggregate_scorecard`,
`ScorecardBucket`, `ScorecardDimension`, `ScorecardFilter`, `MAX_ROWS`,
`gates_all_pass` intact.

### Task 3 — Add the `--by` dimension arg (clap ValueEnum)

In `mcp/src/main.rs`, add a `ByArg` value-enum + `From` conversion, mirroring the
existing `CalibrateArg` pattern **verbatim in shape** (main.rs:38–56):

```rust
#[derive(clap::ValueEnum, Clone, Copy)]
enum ByArg {
    #[value(name = "model")]
    Model,
    #[value(name = "tag")]
    Tag,
    #[value(name = "settings")]
    Settings,
}

impl From<ByArg> for scorecard::ScorecardDimension {
    fn from(a: ByArg) -> Self {
        match a {
            ByArg::Model => Self::Model,
            ByArg::Tag => Self::Tag,
            ByArg::Settings => Self::Settings,
        }
    }
}
```

Add the flag to the `Commands::Scorecard` variant (main.rs:166), **defaulting to
`settings`** so the bare `rexymcp scorecard` keeps today's behavior:

```rust
/// Bucket by this dimension (model | tag | settings)
#[arg(long, value_enum, default_value = "settings")]
by: ByArg,
```

Note `--tag <t>` (the existing repeatable **filter**, unchanged) and `--by tag`
(the **bucket dimension**) are different axes and coexist: `--tag rust --by tag`
filters to rust-tagged runs, then buckets by tag. Leave the `--tag`/`--model`/
`--min-runs`/`--telemetry-path`/`--json` args as they are.

### Task 4 — Generalize the CLI loader + formatter

In `mcp/src/scorecard_cli.rs`:

- Change the import to `use crate::scorecard::{ScorecardBucket,
  ScorecardDimension, ScorecardFilter, aggregate_scorecard};`.
- Rename `load_settings_scorecard` → **`load_scorecard`**, add a `dimension:
  ScorecardDimension` param, return `Vec<ScorecardBucket>`, and call
  `aggregate_scorecard(&runs, dimension, filter)` (the read + `fold_reviews`
  logic is otherwise unchanged):

  ```rust
  pub fn load_scorecard(
      config_path: &Path,
      telemetry_path: Option<&Path>,
      dimension: ScorecardDimension,
      filter: &ScorecardFilter,
  ) -> Result<Vec<ScorecardBucket>, String> {
      // … unchanged config load + telemetry_file resolution + read + fold_reviews …
      Ok(aggregate_scorecard(&runs, dimension, filter))
  }
  ```

- Rename `format_settings_scorecard` → **`format_scorecard`**, take `rows:
  &[ScorecardBucket]` and `dimension: ScorecardDimension`, and:
  - **Add three columns** — `REPAIRS` (`repairs_per_call_mean`, `{:.2}`),
    `VERIF_RET` (`verifier_retries_mean`, `{:.2}`), `WALL_S`
    (`wall_clock_s_mean`, `{:.1}`). (These are the dropped columns. Place them in
    a sensible spot, e.g. after `TURNS_MEAN`; exact column order/spacing is not
    pinned — presence of the headers + values is.)
  - **The second (key) column header depends on the dimension** — use a small
    helper:

    ```rust
    fn key_header(dim: ScorecardDimension) -> &'static str {
        match dim {
            ScorecardDimension::Settings => "SETTINGS",
            ScorecardDimension::Tag => "TAG",
            ScorecardDimension::Model => "KEY", // Model rows have an empty key
        }
    }
    ```

    The key **cell** is `row.key` for every dimension; render an empty `key`
    (the `Model` case) as `—` (the same em-dash sentinel the `Option` columns
    use). Keep the existing `—`-for-`None` rendering of `length_finish_rate_mean`
    / `approved_first_try_rate` / `peak_context_pct_mean` / `tokens_reclaimed_mean`
    unchanged.
  - Keep the `rows.is_empty()` → `"(no runs)"` guard and the `MODEL` first
    column.

The current formatter body (scorecard_cli.rs:40) is the base to extend — quote it
into your working memory and add the three columns + the dimension-driven key
header; do not rewrite its `Option`-rendering logic.

### Task 5 — Wire the dispatch arm

In `mcp/src/main.rs`, the `Commands::Scorecard { … }` arm (main.rs:611): destructure
the new `by`, convert it (`let dimension: scorecard::ScorecardDimension =
by.into();`), pass `dimension` into `load_scorecard`, and pass it into
`format_scorecard(&rows, dimension)`. The `json` branch now serializes
`Vec<ScorecardBucket>` (rows carry `key`, not `settings` — the intended
unification; same `key` field the MCP tool emits). No other arm changes.

### Task 6 — Migrate the tests

- **`scorecard_tests.rs`:** add a test-local shim so the 12 `aggregate_by_settings`
  call sites need no change (same trick 05a-ii used for `aggregate`):

  ```rust
  /// Test convenience: the model×settings aggregation is now `aggregate_scorecard`
  /// with the `Settings` dimension (the `aggregate_by_settings` wrapper was retired
  /// in phase-05a-iii). Returns `ScorecardBucket` (key field is `key`).
  fn aggregate_by_settings(runs: &[PhaseRun], filter: &ScorecardFilter) -> Vec<ScorecardBucket> {
      aggregate_scorecard(runs, ScorecardDimension::Settings, filter)
  }
  ```

  Remove `aggregate_by_settings` from the `use super::{…}` import (it is now the
  local shim; `ScorecardBucket`, `ScorecardDimension`, `aggregate_scorecard` are
  already imported from 05a-ii). Change the 5 `.settings` field reads on result
  rows to `.key`. **Delete** the now-tautological
  `aggregate_scorecard_settings_matches_wrapper` test (with the shim it compares
  the core to itself — it asserts nothing). Leave the tag/model
  `aggregate_scorecard_*` tests untouched.
- **`scorecard_cli.rs` tests:** change the two `SettingsScorecardRow { … settings:
  "…", … }` literals to `ScorecardBucket { … key: "…", … }` (pure field rename —
  `ScorecardBucket` is field-identical except `settings`→`key`); update the
  `load_settings_scorecard`/`format_settings_scorecard` calls to
  `load_scorecard`/`format_scorecard` with a `ScorecardDimension::Settings`
  argument; and keep the existing assertions (settings label present, `—` for
  `None`, `PEAK_CXT`/`RECLAIMED` headers, `71%`, `9k`). Add assertions per Task 7.

### Task 7 — New tests

Add (inside the appropriate `mod tests`):

- **`format_scorecard_shows_dropped_columns`** (in `scorecard_cli.rs`) — a
  `ScorecardBucket` with `repairs_per_call_mean`, `verifier_retries_mean`,
  `wall_clock_s_mean` set to distinguishable non-zero values; assert the output
  contains the `REPAIRS`, `VERIF_RET`, `WALL_S` **headers** and each value's
  rendering. (Mutation pin: dropping a column from the formatter fails this.)
- **`format_scorecard_key_header_follows_dimension`** (in `scorecard_cli.rs`) —
  the same rows formatted with `Settings` contain `"SETTINGS"`; with `Tag`
  contain `"TAG"`; with `Model` contain `"KEY"` and **not** `"SETTINGS"`/`"TAG"`.
- **`load_scorecard_by_tag_and_by_model`** (in `scorecard_cli.rs`, extending the
  existing `load_settings_scorecard_reads_and_aggregates` fixture pattern) — the
  same on-disk fixture loaded with `ScorecardDimension::Tag` buckets by tag
  (assert a `key`-non-empty row) and with `ScorecardDimension::Model` yields one
  row per model with `key == ""`. (Pins the `--by` plumbing reaches the core.)

## Acceptance criteria

- [ ] `rexymcp scorecard --by model|tag|settings` parses; bare `rexymcp
      scorecard` defaults to `settings` (unchanged behavior).
- [ ] The human table shows `REPAIRS`, `VERIF_RET`, `WALL_S` columns; the second
      column header is `SETTINGS`/`TAG`/`KEY` per `--by`.
- [ ] `pub fn aggregate_by_settings` and `pub struct SettingsScorecardRow` are
      **deleted**; `grep -nE "aggregate_by_settings|SettingsScorecardRow"
      mcp/src/` returns nothing (the test-local shim is a private `fn`, not this
      symbol as a `pub` item — it may share the name but must not reference the
      deleted `pub` function).
- [ ] **No `#[allow(dead_code)]` remains in `mcp/src/scorecard.rs`**; `grep -rn
      "allow(dead_code)" mcp/src/scorecard.rs` returns nothing. (`Model` is now
      constructed in production by the `--by model` path, so the variant is live.)
- [ ] `load_scorecard` / `format_scorecard` take a `ScorecardDimension` and
      operate on `ScorecardBucket`; the old `*_settings_scorecard` names are gone.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

- **Existing (via shim / renamed):** the 12 `aggregate_by_settings` settings
  tests in `scorecard_tests.rs` pass through the shim (`.settings`→`.key`); the
  `scorecard_cli.rs` tests pass with `ScorecardBucket` literals +
  `format_scorecard`/`load_scorecard`.
- **Deleted:** `aggregate_scorecard_settings_matches_wrapper` (tautological once
  the wrapper is a shim).
- **New:** `format_scorecard_shows_dropped_columns`,
  `format_scorecard_key_header_follows_dimension`,
  `load_scorecard_by_tag_and_by_model` (Task 7).
- **Clap:** if there is an existing `cli_parse_scorecard_*` test (main.rs ~1065),
  extend or add one asserting `--by tag` parses to `ByArg::Tag` and the default
  is `ByArg::Settings`.

## End-to-end verification

Live CLI run against the real telemetry store (read-only) — exercises the full
`--by` plumbing + the new columns + the dimension-driven header:

```
cargo run -p rexymcp -- scorecard --config rexymcp.toml --by settings
cargo run -p rexymcp -- scorecard --config rexymcp.toml --by tag
cargo run -p rexymcp -- scorecard --config rexymcp.toml --by model
```

Confirm each renders a table with `REPAIRS`/`VERIF_RET`/`WALL_S` columns and the
correct second-column header (`SETTINGS`/`TAG`/`KEY`), and that `--by model`
shows one row per model. Quote the real command output (not a paraphrase) in the
completion Update Log.

## Authorizations

None. No new dependencies (`clap`'s `ValueEnum` is already used — see
`CalibrateArg`). No `docs/architecture.md` edit — the scorecard is described
conceptually there and the CLI-surface addition doesn't change the design. No
`STANDARDS.md`/`WORKFLOW.md` edit.

## Out of scope

- **`rexymcp costs`, the dashboard Budget panel, the `b`-key toggle** — phase-06.
- **`profile` tokens/cost per approved phase** — phase-05b.
- **Adding a `by` parameter to the MCP `model_scorecard` tool** — it stays
  Tag-only (a future decision; the CLI is the `--by` surface).
- **Oscillation-tail / calibrate-governor reporting alignment** — phase-07.
- **`docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.**

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784593222381 (complete, server-authored)

**Summary:** ## Summary

Implemented all 7 tasks for phase 05a-iii:

1. **Removed `#[allow(dead_code)]`** on `ScorecardDimension` — `Model` is now consumed by `--by model`.
2. **Deleted `aggregate_by_settings` and `SettingsScorecardRow`** — the CLI now calls `aggregate_scorecard` directly with `ScorecardBucket`.
3. **Added `--by model|tag|settings` clap flag** with `ByArg` ValueEnum (defaults to `settings`).
4. **Generalized `load_scorecard`/`format_scorecard`** to accept `ScorecardDimension`; added `REPAIRS`, `VERIF_RET`, `WALL_S` columns; key header follows dimension (`SETTINGS`/`TAG`/`KEY`).
5. **Wired the dispatch arm** in `main.rs` to pass `dimension` through the full pipeline.
6. **Migrated tests** — added test-local shims, changed `.settings` → `.key`, deleted the tautological `aggregate_scorecard_settings_matches_wrapper`.
7. **Added new tests** — `format_scorecard_shows_dropped_columns`, `format_scorecard_key_header_follows_dimension`, `load_scorecard_by_tag_and_by_model`.

All verification commands pass: `cargo fmt --all --check`, `cargo build`, `cargo clippy`, `cargo test` (1024 passed). Live CLI runs confirm `--by settings`, `--by tag`, and `--by model` all render correct tables with the new columns and dimension-appropriate headers.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Checking rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.03s


TEST
ate_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1024 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.08s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M35-metrics-cost-accounting/phase-05a-iii-scorecard-by-cli.md` — +1 -1
- `mcp/src/main.rs` — +30 -3
- `mcp/src/scorecard.rs` — +1 -65
- `mcp/src/scorecard_cli.rs` — +191 -50
- `mcp/src/scorecard_tests.rs` — +12 -40

**Commit:** 748819c2e13244e32deaed4ed215680f265ef5c4

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

