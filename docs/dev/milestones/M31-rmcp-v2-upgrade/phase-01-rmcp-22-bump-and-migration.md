# Phase 01: `rmcp` 1.8→2.2 bump + API migration

**Milestone:** M31 — rmcp v2 Upgrade
**Status:** in-progress
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
