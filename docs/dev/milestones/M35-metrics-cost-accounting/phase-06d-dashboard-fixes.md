# Phase 06d: Dashboard correctness — full phase id (fixes milestone + phase display) + budget toggle hint

**Milestone:** M35 — Metrics & Cost Accounting Overhaul
**Status:** done
**Depends on:** phase-06c-iii-b
**Estimated diff:** ~140 lines
**Tags:** language=rust, kind=fix, size=s

## Goal

Fix three dashboard-facing correctness issues:

1. **[issue 5] The session panel shows a truncated phase** — `phase-06` instead of
   `phase-06c-iii-b`.
2. **[issue 4] The session panel's Milestone is usually wrong** — it fails to resolve.
3. **[issue 2] The Budget panel doesn't advertise its `b` toggle** — the Activity panel
   shows `[f=filter]` in its border; Budget should show a `[b=…]` hint the same way.

**Issues 4 and 5 share one root** and are fixed together: `derive_phase_id`
(`mcp/src/runner.rs`) collapses `phase-06c-iii-b-per-skill-breakdown.md` to the coarse
`"phase-06"` (it stops at the first non-digit). That coarse id becomes the session
label (→ truncated phase display) **and** the `resolve_milestone` prefix-match input (→
`"phase-06-"` matches no `phase-06c-…` doc → wrong/no milestone). Making
`derive_phase_id` return the **full** phase id (`"phase-06c-iii-b"`) fixes both — and
also fixes **bug-05b-1's root** (the same coarse id made sub-phases indistinguishable in
`scorecard`/`profile`/`runs`; the `profile.rs` doc-stem workaround stays but is no longer
load-bearing). **User-approved (2026-07-21):** yes, the telemetry `phase_id` grouping
gets finer going forward — desirable (06a vs 06b vs 06c-i become distinct); M35 already
waived back-compat, so pre-existing coarse records stay coarse.

**Issue 3 (the trailing blank row) is NOT in this phase** — investigation showed it is a
shared-fixed-height layout constraint (the three header panels share one height sized to
the tallest, Budget; the top-skill line from 06c-iii-b makes Budget the tallest), not a
simple over-provisioning to trim. It needs a separate design decision (see the milestone
README note); do **not** touch the header-band height here.

## Architecture references

Read before starting:

- `mcp/src/runner.rs` — `derive_phase_id` (76–92, the fix target) and its existing tests
  (`derive_phase_id_standard`/`_non_matching`, ~502–512). It is called once
  (`runner.rs:233`) to set both the session `phase` label and the telemetry `phase_id`.
- `mcp/src/dashboard/mod.rs` — `resolve_milestone` / `resolve_milestone_dir` (190–240):
  `resolve_milestone_dir` builds `prefix = "{phase}-"` and matches phase-doc filenames
  starting with it. With the full phase id this matches the running phase's doc. **No
  change needed here** — it works once `derive_phase_id` is fixed; just add a test.
- `mcp/src/dashboard/render.rs` — the Budget panel is `panel(" Budget ", budget)`
  (render.rs:235); the Activity panel's border is `.title(" Activity [f=filter] ")`.
- `mcp/src/dashboard/panels.rs` — `pub(crate) fn panel(title: &'static str, …)`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.
5. **Test placement:** tests in the existing `#[cfg(test)] mod tests` block of the file
   under test (`runner.rs`, `dashboard/mod.rs`).
6. **Editing discipline (load-bearing — three prior 06c runs hard-failed here):** edit
   with `patch`/`patch_lines`, **never a whole-file `write_file`**. View with
   `read_file` (`start_line`/`end_line`), **never `sed -n`/`cat`, and NEVER run the same
   command twice** — the governor hard-fails on identical repeated calls. Read once, act.
   `cargo check -p rexymcp` after each file.

## Current state

**`derive_phase_id`** (runner.rs:76–92) — the coarse derivation to fix:

```rust
/// `phase-01-phase-runner.md` → `"phase-01"`; non-matching → whole stem.
pub fn derive_phase_id(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if let Some(after) = stem.strip_prefix("phase-") {
        let digits_end = after
            .char_indices()
            .find(|(_, c)| !c.is_ascii_digit())
            .map(|(i, _)| i)
            .unwrap_or(after.len());
        if digits_end > 0 {
            return stem[..6 + digits_end].to_string(); // "phase-" (6) + "06" → "phase-06"
        }
    }
    stem.to_string()
}
```

Phase-doc naming convention: `phase-<id>-<descriptive-slug>.md`, where `<id>` is
`-`-joined **id segments** — numbers (`06`), number+letter (`06a`, `06c`), roman
numerals (`i`/`ii`/`iii`), or single letters (`a`/`b`) — followed by a descriptive slug
of real words. Examples → desired id:
`phase-01-phase-runner` → `phase-01`; `phase-06a-costs-cli-core` → `phase-06a`;
`phase-05a-iii-scorecard-by-cli` → `phase-05a-iii`;
`phase-06c-iii-b-per-skill-breakdown` → `phase-06c-iii-b`.

**`panel(" Budget ", budget)`** (render.rs:235) vs Activity's `.title(" Activity [f=filter] ")`.

## Spec

### Task 1 — `derive_phase_id` returns the full phase id (runner.rs)

Rewrite so it keeps **all leading id segments**, stopping at the first descriptive word.
An **id segment** (after `phase-`, split on `-`) is one that: contains an ASCII digit,
**or** is a single ASCII letter (`a`–`z`, len 1), **or** is a roman numeral (all chars in
`{i, v, x}`). Stop at the first segment that is none of these (a real word like `costs`,
`per`, `phase`, `scorecard`). Reassemble `"phase-" + kept-segments.join("-")`. If no
segment qualifies (non-matching name), return the whole stem (unchanged fallback).

```rust
pub fn derive_phase_id(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let Some(after) = stem.strip_prefix("phase-") else {
        return stem.to_string();
    };
    let is_id_seg = |s: &str| {
        !s.is_empty()
            && (s.chars().any(|c| c.is_ascii_digit())
                || (s.len() == 1 && s.chars().all(|c| c.is_ascii_lowercase()))
                || s.chars().all(|c| matches!(c, 'i' | 'v' | 'x')))
    };
    let kept: Vec<&str> = after.split('-').take_while(|s| is_id_seg(s)).collect();
    if kept.is_empty() {
        return stem.to_string();
    }
    format!("phase-{}", kept.join("-"))
}
```

(`"iii"` matches both the roman-numeral and — coincidentally — the all-`i` cases; either
way it is kept. A single `"b"` is kept by the len-1 rule. `"per"` / `"costs"` are kept by
none → stop.)

### Task 2 — Budget panel `b`-toggle border hint (render.rs)

Change `panel(" Budget ", budget)` (render.rs:235) to include a hint mirroring Activity's
`[f=filter]`, e.g. `panel(" Budget [b=$/tok] ", budget)`. Exact hint text is not pinned;
it must name the `b` key and appear in the panel border. (`panel` takes a `&'static str`,
so a string literal is fine.)

## Acceptance criteria

- [ ] `derive_phase_id` returns the full id: `phase-06c-iii-b-per-skill-breakdown.md` →
      `"phase-06c-iii-b"`; `phase-05a-iii-scorecard-by-cli.md` → `"phase-05a-iii"`;
      `phase-06a-costs-cli-core.md` → `"phase-06a"`; `phase-01-phase-runner.md` →
      `"phase-01"` (unchanged); a non-`phase-` name → the whole stem (unchanged).
- [ ] `resolve_milestone` (dashboard) resolves the correct milestone when given a full
      phase id that matches a phase doc in a milestone dir (add a test with a temp
      `docs/dev/milestones/M<n>-…/phase-<full-id>-….md`).
- [ ] The Budget panel border shows a `b`-toggle hint (like Activity's `[f=filter]`).
- [ ] The header-band **height is unchanged** (issue 3 is out of scope).
- [ ] `cargo fmt --all --check`, `cargo build`,
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test` all pass.
- [ ] No behavior change to the executor loop, telemetry writers, or `scope_costs`/costs.

## Test plan

In `mcp/src/runner.rs` `mod tests` (extend the existing `derive_phase_id` tests):

- **`derive_phase_id_keeps_letter_suffix`** — `phase-06a-costs-cli-core.md` → `"phase-06a"`.
- **`derive_phase_id_keeps_multipart_id`** — `phase-06c-iii-b-per-skill-breakdown.md` →
  `"phase-06c-iii-b"`; `phase-05a-iii-scorecard-by-cli.md` → `"phase-05a-iii"`.
  **Mutation-sensitive:** must fail under the old digits-only derivation (which gives
  `"phase-06"` / `"phase-05"`).
- **`derive_phase_id_standard_unchanged`** — keep/assert `phase-01-phase-runner.md` →
  `"phase-01"` and the non-matching `weird-name.md` → `"weird-name"` (the existing two
  tests already cover these — leave them; add the new cases above).

In `mcp/src/dashboard/mod.rs` `mod tests` (reuse the existing `resolve_milestone` test
scaffolding, which builds a temp `docs/dev/milestones` tree):

- **`resolve_milestone_matches_full_phase_id`** — create a milestone dir with a phase doc
  named for a **multi-part** id (e.g. `M35-…/phase-06c-iii-b-x.md`) and assert
  `resolve_milestone(repo, Some("phase-06c-iii-b"))` returns that milestone — and that the
  coarse `Some("phase-06")` does **not** (the prefix `"phase-06-"` matches no file). This
  pins the fix's payoff.

## End-to-end verification

`derive_phase_id` + the Budget hint are exercised on the next real dispatch/dashboard,
but that is out of this phase's control. For the artifact check: run `cargo test` and
quote the new `derive_phase_id_keeps_multipart_id` + `resolve_milestone_matches_full_phase_id`
results in the Update Log, and note that the **next** dispatched phase's session log +
dashboard will carry the full phase id (the coarse-id session records already on disk are
unaffected — forward-looking). Optionally `cargo run -p rexymcp -- dashboard --repo .` to
eyeball the ` Budget [b=…] ` border, but the unit tests are the pinned evidence.

## Authorizations

- Editing `mcp/src/runner.rs` and `mcp/src/dashboard/render.rs` (and adding a test to
  `mcp/src/dashboard/mod.rs`) is in scope.
- **No new dependencies.**
- No `docs/architecture.md`/`STANDARDS.md`/`WORKFLOW.md`/other-phase-doc edits.
- No `unsafe`, no `#[allow]`, no new `unwrap`/`expect`/`panic!` in production paths.

## Out of scope

- **Issue 3 (trailing blank row).** It is a shared-fixed-height layout constraint (the
  three header panels share one height sized to the tallest, Budget), not
  over-provisioning — trimming the band would clip Budget. Needs a separate design call
  (per-panel heights, or moving the top-skill line to the title). **Do not touch the
  header-band `Layout::vertical([Length(11), …])`.**
- **`profile.rs`'s `phase_label_str`** — the bug-05b-1 display workaround stays; do not
  remove it (harmless, and the doc-stem it uses is still correct).
- **Re-attributing existing coarse-`phase-06` telemetry records** — the change is
  forward-looking; do not migrate the store.
- **Changing `resolve_milestone_dir`'s matching logic** — it works correctly with the
  full phase id; only add a test.

## Update Log

### Update — 2026-07-22 05:45 (started)

**Executor:** `runner.rs` and `dashboard/render.rs` edits + tests for full phase id derivation and budget panel hint.

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
### Update — ts=1784699436384 (complete, server-authored)

**Summary:** All verification commands pass. Here's the summary:

**Summary:** Implemented all three tasks: (1) Rewrote `derive_phase_id` in `runner.rs` to keep all leading id segments (digits, single letters, roman numerals) instead of stopping at the first non-digit — `phase-06c-iii-b-per-skill-breakdown.md` now yields `"phase-06c-iii-b"` instead of `"phase-06"`, fixing both the truncated phase display and the milestone resolution failure. (2) Added `[b=$/tok]` hint to the Budget panel border in `render.rs`. (3) Added tests: `derive_phase_id_keeps_letter_suffix`, `derive_phase_id_keeps_multipart_id`, and `resolve_milestone_matches_full_phase_id` (which verifies the full id matches while the coarse id does not). All 4 `derive_phase_id` tests pass, the new `resolve_milestone_matches_full_phase_id` test passes, and `cargo fmt/build/clippy/test` all pass clean.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
es_advisory ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::symbols::tests::no_symbols_returns_advisory_error ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::symbols::tests::references_respects_max_results ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::symbols::tests::kind_filter_returns_only_matching_kind ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::symbols::tests::respects_gitignore ... ok
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

test result: ok. 1031 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M35-metrics-cost-accounting/phase-06d-dashboard-fixes.md` — +5 -1
- `mcp/src/dashboard/mod.rs` — +21 -0
- `mcp/src/dashboard/render.rs` — +1 -1
- `mcp/src/runner.rs` — +36 -13

**Commit:** cbdc37922c2e29d70ed95e58a709ff92b368e5bc

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-21

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** AEON-7/Qwen3.6-27B-AEON
- **Scope deviations:** none — `derive_phase_id` rewrite + one-line `render.rs` Budget
  hint + three tests, exactly as specced. `Layout::vertical` header-band height untouched
  (issue 3 correctly deferred to 06d-2); `profile.rs` workaround left in place;
  `resolve_milestone_dir` logic unchanged (test-only addition).
- **Calibration:** minor — the completion Update Log quoted the full `cargo test` tail
  rather than calling out the two pinned tests (`derive_phase_id_keeps_multipart_id`,
  `resolve_milestone_matches_full_phase_id`) under an "End-to-end verification" heading as
  the phase's E2E section asked. Substance was present; reviewer independently re-ran both
  by name (pass) plus all four gates green (fmt/build/clippy, `1031 passed; 0 failed`).

