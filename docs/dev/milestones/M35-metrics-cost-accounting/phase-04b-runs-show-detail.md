# Phase 04b: `runs show <id>` — per-run detail view

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** todo
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
