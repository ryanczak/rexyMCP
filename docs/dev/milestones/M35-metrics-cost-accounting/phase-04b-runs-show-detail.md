# Phase 04b: `runs show <id>` — per-run detail view

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-04a
**Estimated diff:** ~220 lines
**Tags:** language=rust, kind=feature, size=m

## Goal

Add `rexymcp runs show <id>` — drill into one run by its 8-hex id (phase-04a's
`metrics::run_id`) and print its **full** record: token breakdown (incl. cache
classes), cost, tok/s, gates, verdict, bounces/bugs/warnings, context
efficiency, and timing. This is the "every recorded number is displayed
somewhere" half of the run-level surface; the `runs` list (04a) is the index,
`runs show` is the detail.

## Architecture references

Read before starting:

- `docs/dev/milestones/M35-metrics-cost-accounting/README.md` — exit-criterion
  bullet 4 (`runs show <id>` drills into one run: full token breakdown incl.
  cache, gates, verdict, bugs/warnings, cost).
- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` record whose fields this view surfaces.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement rule:** every new `#[test]` fn goes **INSIDE** its file's
   existing `#[cfg(test)] mod tests { ... }` block. Never at file scope.
6. **Editing discipline (now enforced):** edit only with `write_file` / `patch`
   / `patch_lines`. `sed -i` / `perl -i` are **refused by the bash tool**. If a
   `patch` fails with `0 matches for old_str` or `it changed on disk`, the file
   drifted — **`read_file` it again, then re-issue the patch**. Do not try to
   force an edit through the shell.

## Current state

### The `runs` CLI today (after 04a)

`mcp/src/main.rs` — the `Runs` command variant (a single flat command) and its
dispatch arm at `main.rs:517`. Field set: `config, model, tags, limit,
telemetry_path, json`. The top-level `Cli` already nests one subcommand
(`#[command(subcommand)] command: Option<Commands>` at main.rs:34); **no other
nested subcommand exists** — this phase adds the first `runs`-level one.

`mcp/src/runs.rs` (after 04a) exposes: `pub fn load_runs(config_path, telemetry_path, filter) -> Result<Vec<PhaseRun>, String>`
(loads + folds reviews + filters), `pub fn format_runs(runs, now_ms, config) -> String`,
and the render helpers `fmt_tokens` / `fmt_cost` / `fmt_tok_per_sec` + `metrics::run_id`.
The mcp crate imports `use rexymcp_executor::store::metrics;` and
`use rexymcp_executor::config::Config;` in `runs.rs`.

### The fields to display

`PhaseRun` (`executor/src/store/telemetry.rs:121`) carries everything the detail
view shows: `ts, model, phase_id, phase_doc_path, tags, status, escalated,
gates` (`Gates { fmt, build, lint, test: Option<bool> }`), `parse_failure_rate,
repairs_per_call, verifier_retries, tool_success_rate, turns, wall_clock_s,
gen_time_s, tokens` (`TokenBreakdown { input_tokens, output_tokens,
cache_read_tokens, cache_write_tokens }`, `.total()`), `warnings, bugs_filed,
bounces_to_approval, architect_verdict` (all `Option`, folded from reviews),
`served_model, length_finish_rate, context_window, context_efficiency`
(`ContextEfficiency { peak_context_pct, … }`, `metrics::reclaimed_total`),
`project_id, milestone_id, tier_telemetry`.

## Spec

### Task 1 — `find_run_by_id` (prefix match, in `runs.rs`)

Add a lookup that resolves an id **or unambiguous prefix** (git-sha style) to a
single run:

```rust
/// Resolve a run id (a full 8-hex `metrics::run_id` or an unambiguous prefix)
/// to exactly one run. Errors are user-facing strings: not-found, or ambiguous
/// (lists the colliding ids).
pub fn find_run_by_id<'a>(runs: &'a [PhaseRun], id: &str) -> Result<&'a PhaseRun, String> {
    let matches: Vec<&PhaseRun> = runs
        .iter()
        .filter(|r| metrics::run_id(r).starts_with(id))
        .collect();
    match matches.as_slice() {
        [] => Err(format!("no run matches id '{id}'")),
        [one] => Ok(one),
        many => Err(format!(
            "id '{id}' is ambiguous — {} runs match: {}",
            many.len(),
            many.iter()
                .map(|r| metrics::run_id(r))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}
```

### Task 2 — `format_run_detail` (in `runs.rs`)

Render one run as a multi-line `label: value` block. `now_ms` is injected (age
is testable, mirroring `format_runs`). **Pin the *content* (every field below
appears with its value); the exact labels/layout are yours.** Reuse the 04a
helpers and `metrics` functions — do not re-derive cost, tok/s, reclaimed, or
the id.

```rust
/// Full single-run detail. `now_ms` injected for a testable age.
pub fn format_run_detail(run: &PhaseRun, now_ms: u64, config: &Config) -> String {
    let rates = config.model_rates(&run.model);
    // Build "label: value" lines. Include, at minimum:
    //   id (metrics::run_id), model, phase_id, age (humanize_age), status,
    //   architect_verdict (— if None), escalated,
    //   gates (fmt/build/lint/test via gate_char),
    //   tokens: input / output / cache_read / cache_write / total,
    //   cost (metrics::token_cost(&run.tokens, &rates); fmt_cost),
    //   tok/s (metrics::tokens_per_sec(run.tokens.output_tokens, run.gen_time_s); fmt_tok_per_sec),
    //   turns, wall_clock_s, gen_time_s,
    //   verifier_retries, parse_failure_rate, repairs_per_call, tool_success_rate,
    //   served_model, length_finish_rate, context_window,
    //   context: peak_context_pct, reclaimed (metrics::reclaimed_total),
    //   bugs_filed, warnings, bounces_to_approval (— if None).
    // ...
}
```

Render `Option` fields as `—` when `None` (matches the list's convention). It is
fine to reuse `fmt_tokens`/`fmt_cost`/`fmt_tok_per_sec`, or to show raw counts —
your call, but every field above must be present with its value.

### Task 3 — The `runs show <id>` clap subcommand

This is the one external-API-shaped piece. **The pattern below is
compile-and-parse-verified against clap 4.6 — reproduce its shape.** In
`mcp/src/main.rs`, give the `Runs` variant an optional nested subcommand
alongside its existing list flags, and add a `RunsCommand` enum:

```rust
    /// List individual PhaseRun records, or show one in detail
    Runs {
        /// Path to the config file
        #[arg(long)]
        config: PathBuf,
        /// Restrict to one model (exact match)
        #[arg(long)]
        model: Option<String>,
        /// Restrict to runs whose tags contain this tag; repeat for AND
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Max rows (most recent first); 0 = no limit
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Override the telemetry phase_runs.jsonl path
        #[arg(long)]
        telemetry_path: Option<PathBuf>,
        /// Emit JSON instead of a human table
        #[arg(long)]
        json: bool,
        /// Subcommand: `show <id>` drills into one run. Absent = list.
        #[command(subcommand)]
        command: Option<RunsCommand>,
    },
```

Add the enum next to `Commands` (it derives `Subcommand`):

```rust
#[derive(Subcommand)]
enum RunsCommand {
    /// Show one run's full detail by id (8-hex, or an unambiguous prefix)
    Show {
        /// Run id from the `ID` column of `rexymcp runs`
        id: String,
    },
}
```

**Verified parses (clap 4.6):** `runs --config r.toml` → `command: None` (list);
`runs --config r.toml show a3f9c1e2` → `command: Some(Show { id })`; `runs
--config r.toml show` (no id) → error (id is a required positional). `--config`
sits on the parent and is given before the `show` token — no
`args_conflicts_with_subcommands` attribute is needed.

### Task 4 — Dispatch the two branches (in `main.rs`)

The existing `Commands::Runs { … }` dispatch arm (main.rs:517) destructures the
list fields. Add `command` to that destructure and branch:

- **`command: None`** → the existing list behavior, unchanged (build the
  `RunsFilter`, `load_runs`, then JSON or `format_runs(&selected, now_ms, &cfg)`
  — keep exactly what 04a produces).
- **`command: Some(RunsCommand::Show { id })`** → load **all** runs (reuse
  `load_runs` with a no-op filter: `RunsFilter { model: None, tags: &[], limit: 0 }`),
  then:

  ```rust
  let all = match runs::load_runs(&config, telemetry_path.as_deref(), &filter) {
      Ok(v) => v,
      Err(e) => { eprintln!("{e}"); std::process::exit(1); }
  };
  match runs::find_run_by_id(&all, &id) {
      Ok(run) => {
          let cfg = match rexymcp_executor::config::Config::load_with_env(&config) {
              Ok(c) => c,
              Err(e) => { eprintln!("failed to load config: {e}"); std::process::exit(1); }
          };
          let now_ms = /* same UNIX-millis block as the list branch */;
          println!("{}", runs::format_run_detail(run, now_ms, &cfg));
      }
      Err(e) => { eprintln!("{e}"); std::process::exit(1); }
  }
  ```

  (`show` ignores `--model`/`--tag`/`--limit`/`--json` — it addresses one run by
  id. That is fine; they simply don't apply.)

The CLI parse test `cli_parse_runs_collects_filters` (main.rs:~997) already
matches with `..`, so it is unaffected by the new field.

## Acceptance criteria

- [ ] `rexymcp runs show <full-8-hex-id>` prints a multi-line detail block for
      that run containing its id, model, each token class
      (input/output/cache-read/cache-write), total, cost, tok/s, the four gate
      results, verdict, and bugs/warnings.
- [ ] An **unambiguous prefix** (e.g. the first 4 hex chars, if unique) resolves
      to the same run.
- [ ] A prefix matching **≥2** runs errors with an "ambiguous" message listing
      the colliding ids and exits non-zero; an id matching **0** runs errors
      "no run matches" and exits non-zero.
- [ ] `rexymcp runs --config …` (no `show`) still prints the 04a list,
      unchanged.
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, and
      `cargo test` all pass.

## Test plan

In `mcp/src/runs.rs` tests (reuse the `make_run(ts, model, tags, verdict)`
helper; `metrics::run_id` is deterministic from `ts|model|phase_id`, so distinct
`ts` values give distinct ids):

- `find_run_by_id_resolves_full_id` — two runs with distinct ts; looking up one
  run's full `metrics::run_id` returns that run.
- `find_run_by_id_resolves_unambiguous_prefix` — a prefix that matches exactly
  one run's id resolves to it. (Pick a prefix length that is unique across the
  two fixtures; if the two ids share a long common prefix, vary a fixture's `ts`
  so they differ early — do **not** hard-code an id string.)
- `find_run_by_id_none_is_error` — an id like `"zzzzzzzz"` (not a hex prefix of
  any run) returns `Err` containing `no run matches`.
- `find_run_by_id_ambiguous_is_error` — the **empty prefix** `""` against ≥2
  runs returns `Err` containing `ambiguous` (negative pin: `""` is a prefix of
  every id, so it must be rejected, not silently resolved).
- `format_run_detail_shows_all_key_fields` — a run with non-zero
  `tokens.{input,output,cache_read,cache_write}`, `gen_time_s > 0`, a
  `[models]`-priced `Config`, gates all `Some(true)`, `architect_verdict =
  Some("approved_first_try")`, `bugs_filed = Some(0)` renders a block that
  `contains` the id, the model, `"cache"` (a cache-token label),
  a `$`-prefixed cost, a numeric tok/s, `"approved_first_try"`, and each gate.

In `mcp/src/main.rs` tests (mirror `cli_parse_runs_collects_filters`):

- `cli_parse_runs_show_id` — `Cli::try_parse_from(["rexymcp","runs","--config",
  "rexymcp.toml","show","a3f9c1e2"])` parses to `Commands::Runs { command:
  Some(RunsCommand::Show { id }), .. }` with `id == "a3f9c1e2"`.
- `cli_parse_bare_runs_is_list` — the same without `show …` parses to
  `Commands::Runs { command: None, .. }` (negative pin: bare `runs` is still the
  list).

## End-to-end verification

The real store has runs with ids (04a's `runs` shows them). Build and drill in
on a real id (read-only):

```bash
cargo build
cargo run -p rexymcp -- runs --config rexymcp.toml           # copy an ID from the table
cargo run -p rexymcp -- runs --config rexymcp.toml show <ID> # full detail
cargo run -p rexymcp -- runs --config rexymcp.toml show <first-4-of-ID>  # prefix
```

Expected: the full-id and the (unique) prefix both print the same detail block —
token breakdown incl. cache, cost (`—` since the executor model is unpriced),
tok/s, gates, verdict. Paste the `show <ID>` output in the completion Update Log.

## Authorizations

None. (No new dependencies; clap is already a dependency. No
`docs/architecture.md` edit.)

## Out of scope

- The `runs` **list** columns — 04a shipped them; do not change the table.
- A `--json` variant of `show` — the list's `--json` stays; `show` is
  human-only this phase (a `show --json` can come later if wanted).
- `scorecard`/`profile`/dashboard/`costs` — phases 05–06.
- Changing `PhaseRun` or any stored shape — `show` is pure read/render.
- `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`, other phase docs.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-20 (escalation)

**Chosen lever:** resume (`continue_phase`)
**Rationale:** `hard_fail` (`IdenticalToolCallRepetition`, 6× identical `sed -n`)
with the production work **100% landed and correct** — `find_run_by_id` +
`format_run_detail` in `runs.rs`, the compile-verified clap `RunsCommand::Show`
subcommand, and the show dispatch branch (`main.rs:539-561`). The **only** break
is a single missing `}` (250 open braces vs 249 close): the model deleted
`mod tests`'s closing brace while adding the two CLI tests, then looped
`read_file`-equivalent `sed -n` reads trying to inspect the fix until the
identical-call governor stopped it (89 turns). The spec was fine — a resume with
the exact fix (add the two CLI tests + close the module) preserves the correct
work; re-dispatch would redo it. **Guard note (for the user's observation "the
executor still wants sed"):** all 16 sed calls this run were **read-only
`sed -n`** (inspection) — **zero `sed -i`**, so the M35 destructive-edit guard
correctly did not fire and the file was **not** cannibalized (contrast 04a's
~300-line loss). The residual habit is harmless inspection; the resume guidance
steers it to `read_file`.

### Update — 2026-07-20 19:38 (started)

**Executor:** phase-04b executor
**Started:** implementing `runs show <id>` — `find_run_by_id`, `format_run_detail`, clap subcommand, dispatch
### Update — ts=1784577633929 (complete, server-authored)

**Summary:** **Summary + Notes for review:**

Resumed phase 04b from a prior hard-failed run. The production code (Tasks 1–4) was already 100% correct on disk — `find_run_by_id`, `format_run_detail`, the `RunsCommand::Show` clap subcommand, and the show dispatch branch all landed from the prior executor. The only remaining work was fixing a missing closing `}` on the `#[cfg(test)] mod tests` block in `main.rs` and adding the two required CLI parse tests (`cli_parse_runs_show_id` and `cli_parse_bare_runs_is_list`). Fixed the brace, added the tests, ran `rustfmt` on both `main.rs` and `runs.rs` to satisfy the formatter, then verified all gates pass: `cargo build`, `cargo test` (1024 passed), `cargo clippy`, and `cargo fmt --all --check` all clean. End-to-end verification confirmed `runs show <full-id>` and `runs show <prefix>` both render the complete detail block with token breakdown, cost, tok/s, gates, verdict, and all other fields. The `runs` list command (`--config ...` without `show`) remains unchanged.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Checking rexymcp v0.9.1 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.09s


TEST
st tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::defaults_to_scope_root_when_no_path_given ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
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
- `mcp/src/main.rs` — +37 -1

**Commit:** 472b4d93829c1a319537e2f2510baea1d85703df

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-20

- **Verdict:** approved_after_1
- **Bounces:** 1 (dispatch `hard_fail` `IdenticalToolCallRepetition` → resolved by a
  briefing-seeded **resume**, not an architect takeover — so the model data point
  is preserved)
- **Executor:** AEON-7/Qwen3.6-27B-AEON (dispatch hard_fail at 89 turns → resume
  complete at 27 turns)
- **Scope deviations:** none — `find_run_by_id` (prefix match) + `format_run_detail`
  + the clap `RunsCommand::Show` subcommand + dispatch branch all match the spec;
  no `runs show --json`, no list-column change.
- **Calibration:** none to fold. **First positive data point on the sed guard**
  (shipped 2026-07-20, live this dispatch): the dispatch hit patch-drift on
  `main.rs` but — `sed -i` now refused — could not cannibalize the file; it used
  read-only `sed -n` and looped, and the `IdenticalToolCallRepetition` governor
  stopped it at 89 turns with a single deleted `}`. Contrast 04a (no guard):
  patch-drift → `sed -i` loop → 600 turns, ~300 lines destroyed. The guard
  converted a catastrophic failure into a one-brace resume.
- **Review rigor:** independent gates green (547 mcp + 1024 executor, 2 ignored);
  `find_run_by_id`'s none-match and ambiguous(empty-prefix) arms both
  mutation-verified (returning `Ok` instead of `Err` fails the respective test).
  E2E on a real id: `runs show 865419ed` renders the full detail block (all four
  token classes + total, cost, tok/s, gates, verdict, timing, context, bugs/
  warnings/bounces); prefix `8654` resolves to the same run; `zzzz` → "no run
  matches"; bare `runs` still lists.

