# Phase 06a: Per-role model delegation config substrate

**Milestone:** M27 — Autonomous Escalation Loop
**Status:** review
**Depends on:** phase-05b (done)
**Estimated diff:** ~70 lines
**Tags:** language=rust, kind=feature, size=s

## Goal

Add the two `[architect]` config keys the `/rexymcp:auto` loop (phase-06b) will
read to delegate per-role work to subagents on a chosen model: `dispatch_model`
and `review_model`. Additive and inert — nothing consumes them until 06b's skill
does. This is the dispatchable Rust half of the phase-06 split; 06b (the skill +
loop report + WORKFLOW mirror) is direct-execution and stays separate.

## Architecture references

Read before starting:

- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` § "Per-role
  model delegation (decided 2026-07-08, with the user)" — the design these two
  keys implement, including the load-bearing **inherit-by-default** rule and the
  deliberate absence of a `draft_model` key.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`ArchitectConfig` lives in `executor/src/config.rs:75-89` (struct) with a manual
`Default` impl at `executor/src/config.rs:91-101`. Today it carries only the cost-
rate fields (`model`, `input_per_mtok`, `output_per_mtok`,
`cache_read_per_mtok`, `cache_creation_per_mtok`). The struct already carries
`#[serde(default)]`, so adding fields is back-compat by construction.

The `rexymcp init` template's `[architect]` block is at `mcp/src/init.rs:81-88`
(commented example keys).

## Spec

The semantics, verbatim from the README: **`None` means inherit** — an unset role
model tells the loop to omit the subagent model override, which the native
subagent default resolves to the session (architect) model. An unset role model
does **NOT** fall back to `[architect] model` (that field is the cost-rate model,
a separate concern). There is deliberately **no** `draft_model` key.

1. **Add the two role-model fields** — in `executor/src/config.rs`, add
   `pub dispatch_model: Option<String>` and `pub review_model: Option<String>`
   to `ArchitectConfig` (struct at lines 75-89) and to its `Default` impl (lines
   91-101, both default to `None`). Give each a one-line `///` doc-comment naming
   the role it delegates and stating that `None` = inherit the session/architect
   model (not `[architect] model`). Do **not** add a resolver method — the two
   `Option<String>` fields *are* the resolution (`Some` = override, `None` =
   inherit); a helper here would be premature abstraction with no Rust caller
   (the consumer is 06b's prose skill, which reads the toml directly).

2. **Document the keys in the init template** — in `mcp/src/init.rs`, add two
   commented lines to the `[architect]` block (after the cache-rate lines, ~line
   88) mirroring the existing commented-key style:

   ```toml
   # dispatch_model = "claude-sonnet-5"   # /rexymcp:auto delegates dispatch to this model (default: inherit)
   # review_model = "claude-sonnet-5"     # /rexymcp:auto delegates review to this model (default: inherit)
   ```

## Acceptance criteria

- [ ] `cargo build` succeeds with zero new warnings; `clippy`, `fmt --check`,
      and `cargo test` all pass.
- [ ] A `rexymcp.toml` with `[architect] dispatch_model` / `review_model` set
      loads and both fields are `Some(...)` with the configured strings.
- [ ] A `rexymcp.toml` whose `[architect]` block omits both keys loads and both
      fields are `None` (back-compat with every existing config).
- [ ] The unset case is `None`, **not** the value of `[architect] model` (the
      no-fallback negative).

## Test plan

Add unit tests to the `#[cfg(test)] mod tests` block in `executor/src/config.rs`,
mirroring `load_parses_toml_executor_block` (config.rs:594) for the write-a-toml-
then-`Config::load` idiom:

- `load_parses_architect_role_models` — a config with `[architect] model =
  "claude-opus-4-8"`, `dispatch_model = "claude-sonnet-5"`, `review_model =
  "claude-haiku-4-5-20251001"` loads; assert each field equals its string.
- `architect_role_models_default_to_none` — a config with an `[architect]` block
  that omits both keys (e.g. only `model` set) loads with both role fields
  `None`. Include the **negative assertion** that `dispatch_model` is `None` even
  though `model` is `Some` — pinning that unset does not fall back to `model`.

## End-to-end verification

The keys are consumed by 06b's skill (which reads the toml) and by `Config::load`;
06a surfaces no new command output. Verify the real config-load path against the
running binary:

- `cargo run -p rexymcp -- doctor --config <toml-with-both-keys-set>` exits 0
  (proves the new keys parse through the real binary's config load).
- `cargo run -p rexymcp -- doctor --config <toml-without-the-keys>` exits 0
  (back-compat: an existing config still loads).

Quote both invocations' outcome in the completion Update Log.

## Authorizations

None. (No new dependency; no `docs/architecture.md` edit — the design is already
recorded in the milestone README.)

## Out of scope

- The `/rexymcp:auto` skill, the loop report, subagent delegation mechanics, and
  the WORKFLOW plugin-template mirror — all phase-06b.
- A resolver method or any Rust consumer of the new fields — none exists yet;
  adding one now would be dead, untested-in-context state.
- `rexymcp calibrate` — its `[architect]` skeleton (calibrate.rs:51) stays as-is;
  a user's explicit keys already survive re-calibrate via item preservation.
- The executor's `TokenBreakdown` and any harvester/accounting change — 05b
  already handles role-model rates architect-side.

## Update Log

### Update — 2026-07-09 18:01 (started)

**Executor:** model (phase-06a run)

**Work:** Adding `dispatch_model` and `review_model` fields to `ArchitectConfig`, updating `Default` impl, all existing struct literals, and the init template. Two new unit tests for parsing and default-to-None behavior.

<!-- entries appended below this line -->
### Update — ts=1783620679948 (complete, server-authored)

**Summary:** All done. Here's the summary:

**Summary + Notes for review:**

Added `dispatch_model` and `review_model` fields (`Option<String>`) to `ArchitectConfig` in `executor/src/config.rs`, with `None` meaning "inherit the session/architect model" (no fallback to `[architect] model`). Updated the `Default` impl and all four existing `ArchitectConfig` struct literals in tests. Documented both keys as commented examples in the `rexymcp init` template (`mcp/src/init.rs`). Added two unit tests: `load_parses_architect_role_models` (verifies both fields parse to `Some(...)`) and `architect_role_models_default_to_none` (verifies both are `None` when omitted, including the negative that `dispatch_model` is `None` even when `[architect] model` is set).

**End-to-end verification:** `cargo run -p rexymcp -- doctor --config /tmp/rexymcp-with-keys.toml` exited 0 (new keys parse through the real binary). `cargo run -p rexymcp -- doctor --config /tmp/rexymcp-without-keys.toml` exited 0 (back-compat). All 928 tests pass, clippy/fmt clean, zero new warnings.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
ive_task_to_done ... ok
test tools::symbols::tests::finds_rust_function_by_name ... ok
test tools::update_task::tests::flips_pending_task_to_active ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::symbols::tests::references_exclude_substring ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::respects_gitignore ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
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

test result: ok. 928 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-0ef0717e07dc6f8c)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M27-autonomous-escalation-loop/README.md` — +1 -1
- `docs/dev/milestones/M27-autonomous-escalation-loop/phase-06a-delegation-config-substrate.md` — +7 -1
- `executor/src/config.rs` — +82 -0
- `mcp/src/init.rs` — +2 -0

**Commit:** e805862f243a20fb1da4a090c2db69e1ebe2a0b5

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

