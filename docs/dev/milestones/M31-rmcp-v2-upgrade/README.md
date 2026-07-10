# M31 — rmcp v2 Upgrade

**Goal:** The `mcp` crate builds and serves on `rmcp` 2.2.x (up from 1.8.0),
with all four gates green and the live `rexymcp serve` handshake verified.

**Status:** planning

**Depends on:** none

## Why now

The `mcp` crate pins `rmcp = "1.7"` (resolves to 1.8.0 in `Cargo.lock`), a full
major version behind. The v2 line matters for three reasons:

1. **Spec alignment.** v2.0.0's headline breaking change is "align model types
   with MCP 2025-11-25 spec" — the protocol revision Claude Code is converging
   on. Staying on 1.x means drifting from the spec the only client we serve
   actually speaks.
2. **Security and conformance fixes.** 2.0–2.2 carry OAuth resource-spoofing /
   SSRF fixes, a streamable-HTTP session-leak fix, cancelled-request handling
   fixes, and 2025-11-25 conformance-audit fixes. Most are outside our stdio
   transport, but cancelled-request handling is squarely in M30 territory.
3. **The surface is small today.** Only three files touch `rmcp`
   (`mcp/src/server.rs`, `mcp/src/server_tests.rs`, `mcp/src/main.rs`), and the
   migration inventory below shows exactly one confirmed source break. Every
   release we skip makes the eventual jump bigger.

The upgrade path is documented in
[rust-sdk discussion #716](https://github.com/modelcontextprotocol/rust-sdk/discussions/716)
(the v2 migration guide for PRs #715/#720/#739): most public model structs
became `#[non_exhaustive]` and gained builder-style constructors
(`Type::new(required).with_optional(val)`), and several error/status enums
became `#[non_exhaustive]` (matches need wildcard arms).

## Verified migration surface (2026-07-10, against docs.rs rmcp 2.2.0)

Every `rmcp` API this repo uses, checked against the published 2.2.0 docs —
this is the pre-injection groundwork for phase-01:

| API | Where used | 2.2.0 status |
|---|---|---|
| features `server`, `macros`, `transport-io` | `mcp/Cargo.toml:17` | all three still exist |
| `ServerHandler::{get_info, call_tool, list_tools, get_tool}` | `server.rs:625-831` | signatures unchanged (incl. `Option<PaginatedRequestParams>`, `MaybeSendFuture`) |
| `Tool::new(name, description, input_schema)` | `server.rs:795,800,816,822` | unchanged (struct is non-exhaustive, but we never literal-construct it) |
| `ListToolsResult { tools, next_cursor, meta }` literal | `server.rs:807` | **not** non-exhaustive; literal still compiles |
| `ServerInfo::default()` + field mutation | `server.rs:631` | `Default` still implemented; field mutation unaffected by non-exhaustive |
| `ProgressNotificationParam { .. }` **struct literal** | `server.rs:152`, `server_tests.rs:557` | **BREAKS** — now `#[non_exhaustive]` (+ new `meta` field). Use `ProgressNotificationParam::new(token, progress).with_message(msg)` (`total: None` simply drops out) |
| `ProgressToken(NumberOrString::Number(42))` | `server_tests.rs:558` | unchanged — still `pub struct ProgressToken(pub NumberOrString)`, not non-exhaustive |
| `rmcp::tool_router` / `rmcp::tool` macros, `ToolCallContext::new`, `schema_for_type`, `Parameters`/`Json` wrappers | `server.rs` | no rename found; compiler confirms |
| `rmcp::ErrorData::{invalid_params, internal_error}` | `server.rs` | unchanged |
| `CallToolResult::success`, `Content::new(RawContent::text(..), None)` | `server.rs:722,778` | no rename found; compiler confirms |
| `rmcp::serve_server` + `rmcp::transport::stdio()` | `main.rs:452-453` | no rename found; compiler confirms |

We match no `rmcp` enums exhaustively, so the non-exhaustive-enum change
(`RmcpError`, `ServerInitializeError`, …) should not bite; the compiler is the
final word.

**Roots corroboration stays deferred.** M26 deferred wiring the client's real
`roots/list` because rmcp 1.8.0 deprecated `list_roots` per MCP SEP-2577; v2
follows the same spec line, so this milestone does not re-open it
(`roots::corroborate` keeps receiving an empty `roots_list`).

## Exit criteria

- `mcp/Cargo.toml` constraint bumped to the 2.2 line; `Cargo.lock` resolves
  `rmcp` to 2.2.x. No other dependency constraint changes.
- All four gates green (`cargo fmt --all --check`, `cargo build`, clippy
  `-D warnings`, `cargo test`).
- A **restarted** `rexymcp serve` (per the stale-serve pattern) completes the
  initialize handshake with Claude Code and lists all 10 tools; a live async
  `execute_phase` → `get_run_status` round-trip works — this doubles as the
  M30 live smoke test that closed unexercised.
- `execute_phase` and `continue_phase` return `structured_content` (alongside
  the existing text block) and declare output schemas in `list_tools` /
  `get_tool` (phase-02).

## Architecture references

- `docs/architecture.md` § Status #31 (this milestone) and § Layer 2 (the
  `rmcp` stdio server the upgrade touches).
- `docs/architecture.md` § Status #26 ("Roots corroboration deferred" — the
  deferral this milestone explicitly does not re-open).

## Phases

| #  | Phase | Status |
|----|-------|--------|
| 01 | rmcp 2.2 bump + API migration ([phase-01-rmcp-22-bump-and-migration.md](phase-01-rmcp-22-bump-and-migration.md)) | done |
| 02 | Structured output for `execute_phase` / `continue_phase` (phase-02, not yet drafted) | todo |

## Notes

Phase-01 follows the M25 dep-bump recipe (bump the one constraint →
`cargo update -p rmcp` → react only to compiler flags → four gates), extended
for a major bump: the phase doc pre-injects the verified before/after for the
one known break site (`ProgressNotificationParam`) and explicitly authorizes
the `mcp/Cargo.toml` + `Cargo.lock` edit (a hard-rule trigger otherwise). The
live-handshake exit criterion is review-time work (architect-side, after a
serve restart), not executor work.

**Phase-02 — structured tool output (decided with the user, 2026-07-10).**
The eight router tools already go through rmcp's `Json<T>` wrapper, but the
two hand-rolled tools return their payloads as a JSON *string* inside a text
content block (`server.rs:718-725` for `execute_phase`'s `{ run_id }`,
`server.rs:774-781` for `continue_phase`'s `PhaseResult`). rmcp 2.2.0's
`CallToolResult` carries a `structured_content: Option<Value>` field with
`CallToolResult::structured(value)` / `structured_error(value)` constructors,
and `Tool::with_output_schema::<T>()` (server feature) advertises a typed
output schema. Phase-02 has the two tools return `structured_content`
(keeping the text block for back-compat — spec-recommended) and declares
output schemas on the two hand-built `Tool` entries in
`list_tools`/`get_tool`. No client dependency — Claude Code consumes
`structuredContent` today. Depends on phase-01 (the constructors are
2.x-only).

**Adoption survey (2026-07-10).** Two v2 capabilities were evaluated with the
user and **not** taken:

- **MCP tasks (SEP-1686).** rmcp 2.2.0 ships the full experimental task
  surface from the 2025-11-25 spec (task-augmented `tools/call` →
  `CreateTaskResult`, `tasks/get` / `tasks/result` polling, `tasks/cancel`,
  `TaskStatusNotification`, `TasksCapability`) — a 1:1 match for M30's
  hand-rolled job model (`run_id` ↔ task ID, `get_run_status` ↔ `tasks/get`,
  `stop_phase` ↔ `tasks/cancel`). **Blocked on the client:** Claude Code's
  MCP tool calls are still synchronous/blocking; SEP-1686 support is an open
  feature request
  ([claude-code#18617](https://github.com/anthropics/claude-code/issues/18617))
  — the same client gap as progress notifications and the `context.ct` cancel
  token. Recorded as a **future milestone candidate**: migrate (or
  dual-expose) the M30 job model onto spec tasks once Claude Code lands
  support. Watch that issue.
- **Elicitation, cancelled-request handling, meta/trace helpers, icons.**
  Elicitation cuts against the no-live-channel design (human gates live in
  the architect's skills); `notifications/cancelled` is a dead channel
  (Claude Code orphans requests — verified in M30); the rest have no consumer
  in our loop.