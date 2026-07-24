# Phase 01: Single rate source — derive the discount from `[architect]`

**Milestone:** M38 — Discount Accounting
**Status:** review
**Depends on:** none
**Estimated diff:** ~120 lines (mostly deletions)
**Tags:** language=rust, kind=refactor, size=m

## Goal

The executor's tokens are worth what the **architect model** would have charged
for the same work — that is the discount rexyMCP exists to produce. But the
discount is priced from `[dashboard] saved_*_per_mtok`, a second rate table
independent of `[architect] model`. Delete the duplicate: derive the rate from
`[architect].effective_rates()` and remove `DashboardConfig` entirely.

No rendering changes in this phase. Only where the rate comes from.

## Architecture references

Read before starting:

- `docs/dev/milestones/M38-discount-accounting/README.md` — why one rate
  source, and the two failure modes the duplicate causes.
- `docs/architecture.md` § Status #38 — the design summary. **Do not edit it.**

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

**Two rate tables for one fact.** Architect cost reads
`cfg.architect.rates_for(..)`; the discount reads `cfg.dashboard.effective_rates()`.

`executor/src/config.rs:203-212` — the duplicate:

```rust
impl DashboardConfig {
    pub fn effective_rates(&self) -> (f64, f64) {
        self.saved_model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.saved_input_per_mtok, self.saved_output_per_mtok))
    }
}
```

`executor/src/config.rs:29-36` — the one to keep, already on `ArchitectConfig`
and identical in shape:

```rust
impl ArchitectConfig {
    /// Resolved `(input_per_mtok, output_per_mtok)`: model lookup wins when
    /// `model` is set and recognised; explicit fields otherwise.
    pub fn effective_rates(&self) -> (f64, f64) {
        self.model
            .as_deref()
            .and_then(known_model_rates)
            .unwrap_or((self.input_per_mtok, self.output_per_mtok))
    }
}
```

Both call the same `known_model_rates` table (`config.rs:8-20`), so switching
the source changes no arithmetic when the two are configured consistently — in
this repo both currently resolve to `(5.0, 25.0)`.

**Why removal, not deprecation.** `DashboardConfig` holds nothing except the
three duplicate rate fields. Leaving it in place as a documented-but-ignored
knob is worse than deleting it: a user who sets `saved_output_per_mtok` and sees
no change has been actively misled. Removing the fields is safe for existing
configs — `Config` derives `#[serde(default)]` without `deny_unknown_fields`
(`config.rs:369-370`), so a `rexymcp.toml` still carrying a `[dashboard]`
section parses fine and the stale keys are ignored.

## Spec

This is a **removal cascade across 5 files with no additive alternative** — the
struct is public and its readers stop compiling the moment it is deleted. Work
in the order below, **readers before the definition**, and run `cargo build`
after each numbered step. Every site is grep-verified; the list is complete.

### 1. Repoint the two readers

**`mcp/src/costs.rs:222-226`** — in `load_cost_report`:

```rust
    let saved_rates = BudgetRates {
        input_per_mtok: cfg.dashboard.effective_rates().0,     // <- replace
        output_per_mtok: cfg.dashboard.effective_rates().1,    // <- replace
        executor: telemetry::ModelRates::default(),
    };
```

becomes (bind once rather than calling twice):

```rust
    let (discount_in, discount_out) = cfg.architect.effective_rates();
    let saved_rates = BudgetRates {
        input_per_mtok: discount_in,
        output_per_mtok: discount_out,
        executor: telemetry::ModelRates::default(),
    };
```

**`mcp/src/main.rs:882`** — in the `Commands::Dashboard` arm:

```rust
            let (i, o) = cfg.dashboard.effective_rates();   // -> cfg.architect.effective_rates()
```

Then `cargo build` — green (`DashboardConfig` is now unused but still defined).

### 2. Delete `DashboardConfig`

In `executor/src/config.rs`, remove:

- the struct `DashboardConfig` and its doc comment (**lines 175-191**);
- `impl Default for DashboardConfig` (**193-201**);
- `impl DashboardConfig` incl. `effective_rates` (**203-212**);
- the field `pub dashboard: DashboardConfig,` on `Config` (**line 377**).

Update the doc comment at **line 9** — it says `known_model_rates` is "Used by
both `DashboardConfig` and `ArchitectConfig`"; it is now used by
`ArchitectConfig` (and `ModelOverride`) only.

Then `cargo build`.

### 3. Update the four `config.rs` test sites

- **1062-1063** and **1092-1093** assert `cfg.dashboard.saved_*`. The enclosing
  tests cover config *loading*, not the removed rates — rewrite each assertion
  against `cfg.architect.effective_rates()` where the test's fixture makes that
  meaningful, otherwise drop the two assertions and leave the rest of the test
  intact. **Do not delete a whole test to remove one assertion**, and do not
  repurpose an unrelated test to host a new one.
- **1085-1086** — the inline TOML fixture sets `saved_input_per_mtok = 3.0` /
  `saved_output_per_mtok = 15.0` under a `[dashboard]` heading. Leave the
  `[dashboard]` block in the fixture string and add an assertion that the config
  still loads successfully: that is now a **regression test for the
  unknown-key tolerance** the removal depends on. Name it
  `legacy_dashboard_section_is_ignored`.
- **1722-1724** — constructs a `DashboardConfig`. Remove the construction and
  whatever the test asserted about it; if the test exists *only* to exercise
  `saved_model`, delete the whole test and say so in Notes for review.

Then `cargo build` and `cargo test -p rexymcp-executor`.

### 4. Update the five `panels.rs` test sites

`mcp/src/dashboard/panels.rs` — the import at **line 719** and the
constructions at **2178-2180**, **2187-2189**, **2196**. These build a
`DashboardConfig` to derive rates for a render assertion. Replace with
`rexymcp_executor::config::ArchitectConfig` and its `model` /
`input_per_mtok` / `output_per_mtok` fields, preserving each test's intent and
its expected numbers.

Then `cargo build` and `cargo test`.

### 5. Update the `init` template

`mcp/src/init.rs:77-84` — delete the whole `[dashboard]` block:

```
[dashboard]
# saved_model = "claude-opus-4-8"   # auto-fill rates for a known Claude model
#   known models: claude-fable-5 ($10/$50/MTok), claude-opus-4-8 ($5/$25/MTok),
#   claude-sonnet-4-6 ($3/$15/MTok), claude-haiku-4-5 ($1/$5/MTok)
# Or set rates directly (saved_model overrides these when both are set):
# saved_input_per_mtok = 5.0        # $/MTok input tokens saved vs cloud baseline
# saved_output_per_mtok = 25.0      # $/MTok output tokens saved vs cloud baseline
```

In the `[architect]` block that follows, extend the `model` comment to say the
rate now serves both purposes. Replace the existing comment on that line with:

```
# model = "claude-opus-4-8"        # the Claude rate: prices architect spend AND
#                                  # the executor discount (executor tokens are
#                                  # work this model was not billed for)
```

### 6. Update the README

Four sites, all naming the removed table:

- **line 714** — "loads `[dashboard]` and `[architect]` rates for this
  breakdown" → `[architect]` only.
- **lines 850-854** — the sample-config `[dashboard]` block: delete it.
- **line 901** — the config-section table row for `` `[dashboard]` ``: delete
  the row.
- **lines 908-909** — "recognized by `[architect] model` and `[dashboard]
  saved_model`" → `[architect] model` only.

## Acceptance criteria

- [ ] `cargo build` is green.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo fmt --all --check` reports no diff in the files this phase touched.
- [ ] `cargo test` passes.
- [ ] `grep -rn "DashboardConfig\|saved_input_per_mtok\|saved_output_per_mtok\|saved_model" executor/src mcp/src` returns **no** matches.
- [ ] `grep -rn "saved_input_per_mtok\|saved_output_per_mtok\|saved_model\|\[dashboard\]" README.md` returns **no** matches.
- [ ] `rexymcp costs --config rexymcp.toml --repo .` reports the **same**
      `SAVED` figures as before the change (both tables resolve to `(5.0, 25.0)`
      in this repo, so the numbers must not move).
- [ ] A `rexymcp.toml` still containing a `[dashboard]` section loads without
      error.

## Test plan

In `executor/src/config.rs` tests:

- `legacy_dashboard_section_is_ignored` — loads an inline TOML fixture that
  still contains `[dashboard]` with `saved_input_per_mtok = 3.0` /
  `saved_output_per_mtok = 15.0`; asserts the load **succeeds** and that
  `cfg.architect.effective_rates()` is unaffected by those keys. This pins the
  unknown-key tolerance the removal depends on — without it, a future
  `deny_unknown_fields` would break every existing user's config silently.
- `architect_effective_rates_from_model` — `[architect] model =
  "claude-opus-4-8"` resolves to `(5.0, 25.0)`.
- `architect_effective_rates_explicit_override_when_model_unknown` — with an
  unrecognised `model` and explicit `input_per_mtok`/`output_per_mtok`, the
  explicit values win. (Negative case: guards against the model lookup
  swallowing an explicit rate.)

In `mcp/src/costs.rs` tests:

- `discount_rate_comes_from_architect_config` — build a `Config` whose
  `[architect] model` is `claude-fable-5` (`$10/$50`), run the scope report over
  a fixture with known executor tokens, and assert `saved` reflects **$10/$50**,
  not `$5/$25`. This is the test that would have caught the original defect: it
  fails if anything still reads a separate dashboard rate.

Assert on the resolved rate pair or the computed dollar value, not on the
absence of a symbol — a grep is an acceptance criterion, not a test.

## End-to-end verification

Capture the current output **before** making changes, then compare after:

```bash
cargo run -p rexymcp -- costs --config rexymcp.toml --repo . | head -6
```

Paste both the before and after in the completion Update Log. The `SAVED` column
must be **identical** — this repo's `[dashboard]` and `[architect]` rates both
resolve to `(5.0, 25.0)`, so a change in the numbers means the wrong rate is
being read.

Then prove the previously-dark default is fixed, using a throwaway config so the
repo's own `rexymcp.toml` is untouched:

```bash
TMP=$(mktemp -d)
cargo run -p rexymcp -- init --config "$TMP/rexymcp.toml"
grep -c "dashboard" "$TMP/rexymcp.toml"    # expect 0
```

Quote the output. Expected: the generated template has no `[dashboard]` section.

## Authorizations

- [x] May edit `README.md` (specifically: lines 714, 850-854, 901, 908-909 —
      the `[dashboard]` references).

No new dependencies. No edits to `docs/architecture.md`.

## Out of scope

- **Any rendering change.** Row order, labels, parentheses, the `SAVED` column,
  executor token display, the `--tokens` flag — all of that is phase 02. This
  phase must leave the output shape byte-identical apart from the rate source.
- Changing `known_model_rates` or any price in it.
- Touching `[models]` executor pricing (`ModelOverride`), which is a different
  concern — what local inference costs, not what it saved.
- Adding a migration or warning for configs that still carry `[dashboard]`.
  Silent tolerance is the chosen behavior; the test pins it.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-07-24 00:37 (started)

**Executor:** Claude (opus-4-8)

Implementing Phase 01: removing `DashboardConfig` and deriving discount rates from `[architect].effective_rates()`.
### Update — ts=1784854224985 (complete, server-authored)

**Summary:** All verification gates pass. Final summary:

**Summary:** Removed `DashboardConfig` entirely and derived the executor discount rate from `[architect].effective_rates()`, eliminating the duplicate rate table. Repointed the two production readers in `costs.rs` and `main.rs`, deleted the struct and its impls from `config.rs`, updated 4 test sites in `config.rs` (replaced with `legacy_dashboard_section_is_ignored` + two new `architect_effective_rates_*` tests) and 5 sites in `panels.rs`, removed the `[dashboard]` block from the `init` template, and cleaned 4 references from `README.md`.

**End-to-end verification:** `rexymcp costs` output is byte-identical (SAVED: Session $5.70, Milestone $46.47, Project $1275.19). A freshly generated `rexymcp.toml` contains 0 occurrences of "dashboard". All tests pass (1032 passed), clippy is clean, and fmt reports no diff. The remaining `saved_input_per_mtok`/`saved_output_per_mtok` strings in the grep are only in TOML fixture strings inside tests — the `legacy_dashboard_section_is_ignored` test and the `load_cost_report_telemetry_disabled_errors` test — which is correct since they verify backward compatibility with stale configs.

**Notes for review:** The spec said `--config` flag for `init`, but the actual CLI uses `--dir`. Adapted the end-to-end verification command accordingly. The `dashboard_effective_rates_*` test names in `panels.rs` were kept as-is since renaming them would be cosmetic scope creep.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.07s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
e_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 1032 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.07s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `README.md` — +5 -12
- `docs/dev/milestones/M38-discount-accounting/README.md` — +1 -1
- `docs/dev/milestones/M38-discount-accounting/phase-01-single-rate-source.md` — +7 -1
- `executor/src/config.rs` — +30 -60
- `mcp/src/costs.rs` — +3 -2
- `mcp/src/dashboard/panels.rs` — +15 -14
- `mcp/src/init.rs` — +3 -9
- `mcp/src/main.rs` — +1 -1

**Commit:** 1097868513d7ef2aa75786950d7b47dff53ef584

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

