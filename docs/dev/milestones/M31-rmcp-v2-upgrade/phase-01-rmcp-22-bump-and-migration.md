# Phase 01: `rmcp` 1.8→2.2 bump + API migration

**Milestone:** M31 — rmcp v2 Upgrade
**Status:** done
**Depends on:** none
**Estimated diff:** ~20 source lines (two files) + the one `Cargo.toml` constraint + `Cargo.lock` churn
**Tags:** language=rust, kind=refactor, size=s

## Goal

Bump the `mcp` crate's `rmcp` dependency from the `1.7` line (currently locked
at `1.8.0`) to the `2.2` line. v2 aligns the SDK's model types with the MCP
2025-11-25 spec and makes most public model structs `#[non_exhaustive]` with
builder-style constructors. The architect verified the full API surface this
crate uses against the published 2.2.0 docs at draft time (2026-07-10): **one
confirmed source break** (`ProgressNotificationParam` struct literals, two
sites) with the exact replacement code given below; everything else is expected
to compile unchanged. React only to what the compiler flags beyond that.

## Architecture references

Read before starting:

- `docs/architecture.md` § Status #31 — names this milestone and its scope.
- This milestone's [README](README.md) § "Verified migration surface" — the
  per-API compatibility table this phase executes against.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture references above.
3. Read this entire phase doc before touching any code.
4. Confirm the repo is on a clean branch with no uncommitted changes.

## Current state

`rmcp` is an **`mcp`-crate-direct** dependency (not a workspace dependency —
the workspace-root `Cargo.toml` does not declare it; only `mcp/Cargo.toml`
does). There is exactly one constraint to change.

`mcp/Cargo.toml`, line 17:

```toml
rmcp = { version = "1.7", features = ["server", "macros", "transport-io"] }
```

`Cargo.lock` resolves it to `1.8.0`. All three feature names exist unchanged
in 2.2.0.

`rmcp` is referenced in exactly **three** source files
(`grep -rln 'rmcp' mcp/src` — nothing in `executor/` uses it):

- `mcp/src/server.rs` — the `#[rmcp::tool_router]` impl, the `ServerHandler`
  impl (`get_info` / `call_tool` / `list_tools` / `get_tool`), the
  `McpProgressNotifier`, and `rmcp::ErrorData` / `CallToolResult` / `Content`
  usage.
- `mcp/src/server_tests.rs` — one `ProgressNotificationParam` literal in
  `progress_notifier_maps_fields_correctly`.
- `mcp/src/main.rs` — `rmcp::transport::stdio()` + `rmcp::serve_server(...)`
  (lines 452–453).

### The two known break sites (fix with exactly this shape)

In rmcp 2.x, `ProgressNotificationParam` is `#[non_exhaustive]` (it also
gained a new `meta` field). A struct literal — **and** functional-update
syntax like `..Default::default()` — no longer compiles outside the defining
crate. The fix is the new builder constructor:

```rust
pub fn new(progress_token: ProgressToken, progress: f64) -> Self
pub fn with_total(self, total: f64) -> Self
pub fn with_message(self, message: impl Into<String>) -> Self
```

**Site 1 — `mcp/src/server.rs:152` (inside `McpProgressNotifier::on_progress`):**

```rust
// BEFORE (1.8):
let _ = peer
    .notify_progress(ProgressNotificationParam {
        progress_token: token,
        progress,
        total: None,
        message: Some(message),
    })
    .await;

// AFTER (2.2):
let _ = peer
    .notify_progress(
        ProgressNotificationParam::new(token, progress).with_message(message),
    )
    .await;
```

(`total: None` simply drops out — omitting `.with_total(..)` leaves it `None`.
`message` is a `String`; `with_message` takes `impl Into<String>`, so pass it
directly, no `Some(..)` wrapper.)

**Site 2 — `mcp/src/server_tests.rs:557` (inside
`progress_notifier_maps_fields_correctly`):**

```rust
// BEFORE (1.8):
let params = ProgressNotificationParam {
    progress_token: ProgressToken(NumberOrString::Number(42)),
    progress: event.turn as f64,
    total: None,
    message: Some(event.message.clone()),
};

// AFTER (2.2):
let params = ProgressNotificationParam::new(
    ProgressToken(NumberOrString::Number(42)),
    event.turn as f64,
)
.with_message(event.message.clone());
```

The test's **assertions stay byte-identical** — reading fields
(`params.progress`, `params.total`, `params.message`) of a non-exhaustive
struct is still allowed; only literal construction is not.
`ProgressToken(NumberOrString::Number(42))` itself is unchanged in 2.2.0
(still `pub struct ProgressToken(pub NumberOrString)`, not non-exhaustive).

### What is expected to compile unchanged (verified against docs.rs 2.2.0)

Do **not** preemptively edit any of these; they survive per the published
2.2.0 docs, with the compiler as the final word:

- `ServerHandler::{get_info, call_tool, list_tools, get_tool}` signatures,
  including `Option<PaginatedRequestParams>` and
  `rmcp::service::MaybeSendFuture`.
- `Tool::new(name, description, input_schema)` (`server.rs:795,800,816,822`).
- The `ListToolsResult { tools, next_cursor, meta }` literal (`server.rs:807`)
  — this struct is **not** non-exhaustive in 2.2.0.
- `ServerInfo::default()` + field assignment (`server.rs:631-634`) —
  `Default` is still implemented, and field *mutation* is unaffected by
  non-exhaustive.
- `rmcp::ErrorData::{invalid_params, internal_error}`,
  `CallToolResult::success`, `Content::new(RawContent::text(..), None)`.
- The `#[rmcp::tool_router]` / `#[rmcp::tool]` macros, the
  `Parameters<T>` / `Json<T>` wrappers,
  `rmcp::handler::server::tool::{ToolCallContext, schema_for_type}`.
- `rmcp::transport::stdio()` + `rmcp::serve_server` (`main.rs:452-453`).

## Gotchas

- **If a further non-exhaustive break appears** (error: "cannot create
  non-exhaustive struct using struct expression"), the fix is that type's
  `::new(required_fields)` constructor plus `.with_*()` builders — the 2.x
  house pattern shown above. `..Default::default()` is **not** a fix (also
  rejected on non-exhaustive structs). If a type has no constructor covering
  a field we need, file a blocker — do not work around it with transmutes,
  serde round-trips, or a vendored copy.
- **Deprecation warnings are errors** under the clippy gate (`-D warnings`),
  and `#[allow(deprecated)]` is a hard-rule violation. If 2.2 deprecates
  something we call, migrate to the replacement the deprecation message
  names; if the message is ambiguous, file a blocker quoting it.
- Fix formatting with `rustfmt <file>` on the files you touched only — never
  `cargo fmt --all` (the writing form).

## Spec

1. **Bump the one version constraint** — in `mcp/Cargo.toml`, change line 17
   from `rmcp = { version = "1.7", features = ["server", "macros", "transport-io"] }`
   to `rmcp = { version = "2.2", features = ["server", "macros", "transport-io"] }`.
   Change **only** the version string; keep the features array exactly as is.
   Do **not** touch the workspace-root `Cargo.toml` or `executor/Cargo.toml`.

2. **Update the lockfile** — run the package-scoped update:
   `cargo update -p rmcp`. Confirm `Cargo.lock` now contains an `rmcp` entry
   at a `2.2.x` version and no `1.8.0` entry remains. If the package-scoped
   update declines (ambiguity error), run `cargo build` instead — a plain
   build resolves the new `^2.2` constraint and writes the lock. A bare
   `cargo update` (no `-p` filter) would churn unrelated crates and is a
   scope violation. New transitive crates appearing in the lock are automatic
   resolver consequences of the authorized bump, not independent dependency
   adds. Commit the `Cargo.lock` change together with the `mcp/Cargo.toml`
   change.

3. **Fix the two known break sites** — apply the exact BEFORE→AFTER
   replacements shown in "Current state" above: `mcp/src/server.rs:152`
   (the `notify_progress` argument) and `mcp/src/server_tests.rs:557`
   (the `params` binding). Leave the surrounding code — including the test's
   assertions — untouched.

4. **Build and react only to what the compiler flags.** Run `cargo build`.
   Per the draft-time analysis, the expectation is a green build after task 3.
   If the compiler flags anything else in `mcp/src/server.rs`,
   `mcp/src/server_tests.rs`, or `mcp/src/main.rs`, fix that specific call
   per the Gotchas section and record the change in "Notes for review". If a
   break appears that you cannot resolve from this phase doc's analysis, file
   a blocker with the exact `cargo build` error rather than guessing.

5. **Run the remaining gates** — `cargo clippy --all-targets --all-features
   -- -D warnings`, `cargo fmt --all --check`, and `cargo test`, as separate
   invocations (not `&&`-chained). All must pass.

## Acceptance criteria

- [ ] `mcp/Cargo.toml` line 17 reads
      `rmcp = { version = "2.2", features = ["server", "macros", "transport-io"] }`
      — version string bumped, features array unchanged.
- [ ] Workspace-root `Cargo.toml` and `executor/Cargo.toml` are unchanged.
- [ ] `Cargo.lock` contains an `rmcp` entry at a `2.2.x` version
      (`grep -A1 'name = "rmcp"' Cargo.lock`) and no `1.8.x` entry remains.
- [ ] `mcp/src/server.rs` and `mcp/src/server_tests.rs` contain **zero**
      `ProgressNotificationParam {` struct literals
      (`grep -rn 'ProgressNotificationParam {' mcp/src` → no matches); both
      sites use `ProgressNotificationParam::new(..)`.
- [ ] Test `progress_notifier_maps_fields_correctly` passes with its
      assertions unmodified.
- [ ] `cargo build` succeeds with zero new warnings.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo test` passes.
- [ ] No `#[allow(..)]` was added anywhere; no file outside
      `mcp/Cargo.toml` + `Cargo.lock` + the three rmcp source files was edited.

## Test plan

No new tests. `rmcp` is a protocol/SDK utility crate; this phase adds no
behavior. Existing guards exercise the bumped crate:

- `progress_notifier_maps_fields_correctly` (`mcp/src/server_tests.rs:546`)
  compiles the migrated `ProgressNotificationParam::new(..)` construction and
  asserts the field mapping is unchanged — its assertions are the
  regression pin for task 3.
- The full `mcp` server test suite (~483 tests) compiles `server.rs` /
  `main.rs` against rmcp 2.2 end-to-end; a green `cargo test -p rexymcp`
  is itself the API-compatibility evidence.

## End-to-end verification

The shipped artifact is the rebuilt `rexymcp` binary speaking MCP over stdio
via rmcp 2.2. Verify against it:

1. `cargo tree -i rmcp` — quote the resolved `2.2.x` version in the completion
   Update Log, confirming the new version actually links into `rexymcp`.
2. `cargo test -p rexymcp 2>&1 | tail -5` — quote the pass/fail summary line.

(The live `rexymcp serve` restart + Claude Code handshake/dispatch smoke test
is **review-time, architect-side work** — the executor cannot restart the
architect's MCP client. Do not attempt it; the two checks above are the
executor's E2E obligation.)

## Authorizations

- [x] May edit `mcp/Cargo.toml`: bump `rmcp` version `"1.7"` → `"2.2"`
      (features array unchanged). No other line of any `Cargo.toml`.
- [x] May update `Cargo.lock` via `cargo update -p rmcp` (or a `cargo build`
      resolve). Transitive lock churn is authorized as an automatic resolver
      consequence of the `rmcp` bump — not as independent dependency changes.

This is **only** authorization to bump `rmcp`. No other dependency may be
added, removed, or bumped. A bare `cargo update` (no `-p` filter) is a scope
violation.

## Out of scope

- **Adopting any new rmcp 2.x capability** — structured tool output
  (`CallToolResult::structured`, `Tool::with_output_schema`) is the next
  phase; MCP tasks (SEP-1686), elicitation, meta/trace helpers, icons, and
  tool titles are not in this milestone. The goal is the version bump with
  the smallest possible diff.
- Wiring `roots/list` corroboration (`roots::corroborate` keeps receiving an
  empty `roots_list` — a standing deferral, see the milestone README).
- Adding wildcard `_ =>` arms "just in case" to matches the compiler does not
  flag (we match no rmcp enums exhaustively today).
- Touching the `executor/` crate, the plugin skills, or any doc other than
  this phase doc.
- Refactoring `call_tool`'s hand-rolled dispatch, the tool descriptions, or
  anything else in `server.rs` the compiler does not force.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

### Update — 2026-07-10 23:13 (started)

**Executor:** rexyMCP executor
**Action:** Started phase — bumping `rmcp` from 1.8 to 2.2, fixing two `ProgressNotificationParam` break sites.

<!-- entries appended below this line -->
### Update — ts=1783725614517 (complete, server-authored)

**Summary:** All tasks complete. Here's the summary:

**What was done:** Bumped `rmcp` from `1.7` to `2.2` in `mcp/Cargo.toml`, updated `Cargo.lock`, and migrated the two known `ProgressNotificationParam` break sites plus one additional break the compiler flagged: `Content`/`RawContent` were replaced by `ContentBlock` in rmcp 2.x. Both call sites in `server.rs` now use `ContentBlock::text()` instead of `Content::new(RawContent::text(...), None)`.

**Verification results:** `cargo build` clean, `cargo clippy` clean, `cargo fmt --all --check` clean, `cargo test -p rexymcp` — 512 passed, 0 failed. `cargo tree -i rmcp` confirms `rmcp v2.2.0` links into `rexymcp`. Zero `ProgressNotificationParam {` struct literals remain in `mcp/src`. `Cargo.lock` shows `rmcp` at version `2.2.0`.

**Gates:** format=run, build=run, lint=run, test=run

**Command output tails:**

```
FORMAT


BUILD
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.06s


LINT
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s


TEST
_path_given ... ok
test tools::update_task::tests::invalid_args_hint_reports_all_complete ... ok
test tools::update_task::tests::invalid_args_hint_lists_incomplete_ids ... ok
test tools::update_task::tests::invalid_state_returns_advisory_error ... ok
test tools::update_task::tests::malformed_args_returns_advisory_error ... ok
test tools::update_task::tests::metadata_shape_is_unchanged ... ok
test tools::symbols::tests::caps_at_max_results ... ok
test tools::update_task::tests::result_flags_redundant_remark ... ok
test tools::update_task::tests::success_output_names_task ... ok
test tools::update_task::tests::result_lists_remaining_incomplete_ids ... ok
test tools::update_task::tests::null_args_returns_recovery_hint ... ok
test tools::update_task::tests::result_reports_all_complete_when_last_done ... ok
test tools::update_task::tests::unknown_id_returns_advisory_error ... ok
test tools::write_file::tests::append_creates_file_if_missing ... ok
test tools::write_file::tests::appends_to_existing_file ... ok
test tools::write_file::tests::append_false_overwrites ... ok
test tools::write_file::tests::creates_new_file ... ok
test tools::write_file::tests::missing_path_returns_recovery_hint ... ok
test tools::write_file::tests::non_object_args_do_not_panic ... ok
test tools::write_file::tests::rejects_malformed_args ... ok
test tools::write_file::tests::reports_missing_parent_dir ... ok
test tools::write_file::tests::overwrites_existing_file ... ok
test tools::write_file::tests::scope_escape_returns_advisory_error_and_writes_nothing ... ok
test tools::symbols::tests::exact_match_no_substring ... ok
test tools::write_file::tests::success_output_includes_line_count ... ok
test tools::symbols::tests::references_single_file_path ... ok
test tools::symbols::tests::finds_python_function_and_class ... ok
test tools::symbols::tests::references_across_multiple_files ... ok
test tools::symbols::tests::references_snippet_shows_source_line ... ok
test tools::symbols::tests::references_truncation_note_omits_kind_filter ... ok
test ai::backends::openai::tests::is_retriable_transport_true_for_reqwest_error ... ok
test tools::symbols::tests::reports_line_and_column ... ok
test tools::symbols::tests::unsupported_extension_skipped_in_dir_walk ... ok
test tools::symbols::tests::respects_gitignore ... ok
test tools::symbols::tests::metadata_carries_definitions_and_files_count ... ok
test tools::bash::tests::cargo_command_records_cargo_filter_label ... ok
test tools::symbols::tests::finds_rust_struct_and_trait ... ok
test governor::verifier::tests::verify_rust_returns_checked_empty_on_clean_code ... ok
test governor::verifier::tests::capture_baseline_dedupes_by_project_root ... ok
test governor::verifier::tests::verify_rust_returns_checked_with_errors_on_broken_code ... ok
test governor::verifier::tests::capture_baseline_skips_unsupported_files ... ok
test tools::bash::tests::cargo_command_output_is_filtered_through_cargo_filter ... ok
test ai::backends::openai::tests::midstream_stall_is_not_retried ... ok
test ai::backends::openai::tests::first_token_stall_retries_then_succeeds ... ok
test ai::tests::stream_next_uses_supplied_timeout ... ok
test tools::bash::tests::default_timeout_used_when_arg_absent ... ok
test tools::bash::tests::arg_timeout_overrides_constructor_default ... ok
test tools::bash::tests::times_out_advisory_failure ... ok
test ai::backends::openai::tests::first_token_stall_exhausts_retries_then_errors ... ok
test health::tests::check_returns_unreachable_on_connection_error ... ok

test result: ok. 949 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.09s


running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/main.rs (target/debug/deps/rexymcp-4e85b51f198fbe9f)
     Running unittests src/lib.rs (target/debug/deps/executor-c1650299697d7408)
   Doc-tests executor

```

**Files changed:**
- `docs/dev/milestones/M31-rmcp-v2-upgrade/README.md` — +1 -1
- `docs/dev/milestones/M31-rmcp-v2-upgrade/phase-01-rmcp-22-bump-and-migration.md` — +6 -1
- `mcp/Cargo.toml` — +1 -1
- `mcp/src/server.rs` — +6 -16
- `mcp/src/server_tests.rs` — +5 -6

**Commit:** 7c26d539ced7cd6f656320754feac721c2cbac31

**Notes:** server-authored completion entry (executor no longer owns the bookkeeping tail; see M27 phase-03).

### Review verdict — 2026-07-10

- **Verdict:** approved_first_try
- **Bounces:** none
- **Executor:** AEON-7/Qwen3.6-27B-AEON
- **Scope deviations:** none beyond what the phase doc pre-authorized — the
  `Content`/`RawContent` → `ContentBlock` migration (server.rs:6, 718, 771)
  was compiler-forced (task 4 explicitly permits fixing whatever the
  compiler flags beyond the two known break sites) and matches the Gotchas
  builder/constructor pattern (`ContentBlock::text(json_str)`), with no
  `#[allow]` and no preemptive wildcard match arms added.
- **Calibration:** none — the milestone README's "Verified migration
  surface" table had listed `Content::new(RawContent::text(..), None)` as
  expected to compile unchanged; the compiler disagreed. Independent
  re-run of all four gates (fmt --check, build, clippy -D warnings, test)
  confirms green; `cargo test -p rexymcp` shows 512 passed, 0 failed,
  matching the executor's claim; `progress_notifier_maps_fields_correctly`
  passes with assertions byte-identical to the phase doc's pre-injected
  AFTER shape.

