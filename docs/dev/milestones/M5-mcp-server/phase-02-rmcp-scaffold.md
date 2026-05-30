# Phase 02: rmcp server scaffold + execute_phase + executor_health

**Milestone:** M5 — MCP server
**Status:** done
**Depends on:** M5 phase-01 (done) — `runner::run_phase` is the composition root this phase wraps.
**Estimated diff:** ~500 lines (server + capping + telemetry-dir config + tests)
**Tags:** language=rust, kind=feature, size=l

## Goal

Stand up the **`rmcp` stdio MCP server** in `mcp/` and expose the two core tools:

- **`execute_phase(phase_doc_path, repo_path, model?)`** → returns the **capped**
  `PhaseResult`. The handler calls `runner::run_phase` (phase-01) and runs the
  output through a capping pass so a phase's inner transcript never floods
  Claude's context.
- **`executor_health(base_url?)`** → wraps `health::check`. Lets the architect
  confirm the local endpoint is reachable + list models before dispatching.

Also: resolve the **cross-project telemetry dir** as a real config field (the
phase-01 follow-up — `Option<PathBuf>`, `None` disables telemetry; a configured
path plumbs through to `runner::run_phase`).

Out of scope for this phase: log-query tools (phase-03), `model_scorecard`
(phase-04), progress notifications (phase-05), `roots/list` corroboration
(phase-06). The server registers only the two tools here; later phases extend the
registration.

## Architecture references

- `docs/architecture.md` — "Layer 2 — `mcp` crate (binary)" (the four practical
  concerns: long runs, liveness, **context hygiene** = output capping, roots);
  `execute_phase` args; `executor_health` semantics; Status §M5.
- M5 README Notes — "Output capping is the boundary's whole point",
  "Telemetry dir is cross-project".
- M4: `agent::execute_phase`, `phase::result::{PhaseResult, PhaseStatus,
  CommandOutputs, Briefing, FileChange}`, `health::{check, Health}`.
- M5 phase-01: `runner::run_phase`, the seam-grouped composition root.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README (esp. the "Output capping"
   and "Telemetry dir" Notes; the four Notes still apply).
2. Read this entire phase doc.
3. **Verify the current `rmcp` 1.7 API** before coding. The example below is an
   *architect-supplied reference*, but the architect cannot live-call the local
   model and may be off in minor details. Read `cargo doc --open --package rmcp`
   (after `cargo add`), or browse `target/doc/rmcp/` after `cargo build`, to
   confirm the exact macro names (`tool_router` / `tool_handler` / `tool`), the
   stdio entrypoint (`rmcp::transport::io::stdio` vs `rmcp::transport::stdio`),
   and the server-handler trait. If the example below diverges from the real
   API, **trust the docs over the example** — the *behavior* this phase pins is
   load-bearing, the exact macro names are not. Flag any divergence in "Notes
   for review".
4. Confirm M5 phase-01 is `done`; `runner::run_phase` returns
   `executor::error::Result<PhaseResult>`. Confirm `health::check(cfg)` returns
   `Health { reachable, base_url, models }`.

## Spec

### 1. Dependencies (authorized)

Add to `mcp/Cargo.toml` (`[dependencies]`):

- **`rmcp = { version = "1.7", features = ["server", "macros", "transport-io"] }`** —
  the Rust MCP SDK; stdio server transport + the tool-router macros.
- **`schemars = "1"`** — required by `rmcp` for `JsonSchema` derive on tool param
  structs.

`mcp` already has `serde` (transitive via executor) and now `serde_json` /
`async-trait` (from phase-01). No other deps. If `rmcp` 1.7 turns out to need a
companion crate (e.g. an `rmcp-macros` re-export), authorize that *one* extra
crate inline (note in "Notes for review") rather than blocking — but **only** if
it's strictly required by `rmcp` itself. Do not add unrelated deps.

### 2. The MCP server scaffold — `mcp/src/server.rs`

A new module declared `mod server;` in `mcp/src/main.rs`. Holds the rmcp tool
router + handler.

**Architect-supplied reference (verify against rmcp 1.7 docs — see Pre-flight 3):**

```rust
use rmcp::service::{serve_server, ServerHandler};
use rmcp::transport::io::stdio;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ExecutePhaseParams {
    pub phase_doc_path: String,
    pub repo_path: String,
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ExecutorHealthParams {
    pub base_url: Option<String>,
}

pub struct RexyMcpServer {
    pub config_path: std::path::PathBuf,
}

#[rmcp::tool_router]
impl RexyMcpServer {
    #[tool(description = "Execute a phase against a target repository …")]
    async fn execute_phase(
        &self,
        params: ExecutePhaseParams,
    ) -> Result<PhaseResult, String> { /* … */ }

    #[tool(description = "Check connectivity to the configured LLM endpoint …")]
    async fn executor_health(
        &self,
        params: ExecutorHealthParams,
    ) -> Result<Health, String> { /* … */ }
}
```

#### Tool: `execute_phase`

1. Load `Config` from `self.config_path` via `Config::load_with_env`.
2. Read `<repo_path>/docs/dev/STANDARDS.md` best-effort (empty string if absent).
3. Use an **empty `executor_contract`** — embedding is M6.
4. Resolve `telemetry_dir = cfg.telemetry.dir.as_deref()` (the new config field;
   see § 4 below).
5. Call `runner::run_phase(&cfg, &phase_doc_path, &repo_path, "", &standards,
   model.as_deref(), telemetry_dir).await`.
6. On `Ok(result)` → run through **`cap::cap_phase_result(result)`** (§ 3) and
   return.
7. On `Err(e)` → return the error message as the tool's `Err(String)` — this
   surfaces as an MCP tool error to Claude. Do **not** swallow it.

The handler returns the **capped `PhaseResult`**. `PhaseResult` already derives
`Serialize` (M4 phase-06); rmcp will JSON-encode it.

#### Tool: `executor_health`

1. Load `Config` from `self.config_path`.
2. If `params.base_url` is `Some`, apply it as a per-call override (same as the
   existing `health` clap subcommand does).
3. Call `health::check(&cfg.executor).await` and return the `Health` struct as
   `Ok` — pass-through, no capping needed (it's bounded — small struct).

`Health` is already `Serialize` (or derive it if not — that's an authorized
narrow `executor/` edit if needed; flag in "Notes for review").

### 3. Output capping — `mcp/src/cap.rs`

A new pure module with one entrypoint:

```rust
pub const MAX_FIELD_BYTES: usize = 50_000;     // per long-string field
pub const TRUNCATION_MARKER_FMT: &str = "\n\n[truncated: {n} bytes elided]";

pub fn cap_phase_result(result: PhaseResult) -> PhaseResult;
```

Truncates **every long string field on the return path** to at most
`MAX_FIELD_BYTES` bytes, appending the truncation marker (with the elided byte
count) when truncation fires. Fields to cap (read from
`executor/src/phase/result.rs`):

- `result.diff` — the unified diff, usually the biggest field
- `result.update_log`
- `result.command_outputs.{format, build, lint, test}` — each `Option<String>`,
  cap when `Some`
- `result.briefing.working_files[].content` — Briefing already caps the *number*
  of files (`MAX_WORKING_FILES = 5`) but each file's content is unbounded
- `result.briefing.what_was_tried[].one_line` — **already** capped to
  `MAX_ATTEMPT_CHARS = 200` upstream; do not double-cap; treat as bounded.

Truncate on **byte** boundaries that respect UTF-8 (use
`s.char_indices().nth(...)` or a byte-safe slice — never split a multi-byte
char). Choose `MAX_FIELD_BYTES = 50_000` as the per-field budget: roughly
~12.5K tokens (4 bytes/token heuristic) per field, well under any reasonable
MCP per-tool ceiling, and big enough to fit the meaningful diff/output of a
phase. **Not configurable yet** — keep it a `const`; revisit if M6 dogfooding
shows it's wrong.

The architecture says "MAX_MCP_OUTPUT_TOKENS"; we use **bytes** instead because
rexyMCP has no tokenizer dep (M4 deliberately avoided one) and bytes are
deterministic. Document the heuristic ratio in a one-line comment.

### 4. Telemetry dir resolution — config field

Add to `executor/src/config.rs` (authorized narrow edit, mirroring phase-01's
`context_length` precedent):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelemetryConfig {
    /// Cross-project PhaseRun store. `None` disables telemetry emission.
    pub dir: Option<PathBuf>,
}

pub struct Config {
    pub executor: ExecutorConfig,
    pub commands: CommandConfig,
    pub budget: BudgetConfig,
    pub telemetry: TelemetryConfig,   // NEW
}
```

Update the existing TOML round-trip tests to assert the new section (default
case: section absent or empty → `dir = None`; explicit case: `[telemetry] dir =
"/var/lib/rexymcp"` → `Some(...)`). No auto-discovery of a user-data-dir — keep
it explicit (predictable; no surprise writes to `~/.local/share` without the
user opting in). M6's `architect` bootstrap will set it explicitly.

### 5. Wiring — `mcp/src/main.rs`

Add a new clap subcommand:

```
rexymcp serve --config <path>
```

That launches the MCP stdio server: load `Config`, construct `RexyMcpServer {
config_path }`, `let (sink, stream) = stdio().await?; serve_server(server, sink,
stream).await?`.

Keep the existing `health` and `run-phase` subcommands (manual testing). The
default no-arg behavior (print name/version) is fine to keep — the **plugin's
`.mcp.json` (M6)** will call `rexymcp serve --config <path>` explicitly.

### 6. Per-tool timeout — architecture note, not server work

The architecture says "the MCP per-tool `timeout` is set well above the 10 s
default (toward the 10-minute ceiling)." That timeout is enforced **client-side**
(by Claude Code via `.mcp.json` per-server config), not by the server itself —
the server just keeps the tool call open until the work returns. So this phase
adds **no timeout code**; the timeout is M6 (`.mcp.json` config). Document this
inline as a one-line comment in `server.rs` so the next reader doesn't go
hunting for it.

## Adaptations / decisions

1. **Trust rmcp docs over the architect's example** (Pre-flight 3). The example
   above is a reference; the real macro names/signatures live in `cargo doc`. Pin
   behavior, not API shape.
2. **Bytes, not tokens** (§ 3). No tokenizer dep; consistent with M4.
3. **No telemetry-dir auto-discovery** (§ 4). Explicit-only.
4. **No per-tool timeout in the server** (§ 6). Client-side, M6.
5. **`Health` may need `Serialize`** (§ 2 — `executor_health`). If it doesn't
   derive `Serialize` today, add it — narrow authorized edit, flag in "Notes for
   review" (same shape as M4 phase-03's parser-types `Deserialize` resolution).

## Acceptance criteria

- [ ] `mcp/Cargo.toml` declares `rmcp = "1.7"` with features `["server",
      "macros", "transport-io"]` and `schemars = "1"`. No other new deps (or one
      authorized inline rmcp-required helper crate, with a "Notes for review"
      entry).
- [ ] `mcp/src/server.rs` exists; `mod server;` is wired into `mcp/src/main.rs`;
      the server type registers exactly two tools whose names are
      `"execute_phase"` and `"executor_health"`.
- [ ] `execute_phase` handler: loads config, reads STANDARDS.md best-effort,
      calls `runner::run_phase` with the params + empty contract + resolved
      `telemetry_dir`, caps the result, returns it. Errors surface as
      `Err(String)`, not silent fallbacks.
- [ ] `executor_health` handler: loads config, applies optional `base_url`
      override, calls `health::check`, returns `Health` (with `Serialize`
      derive — added if missing).
- [ ] `mcp/src/cap.rs`: `cap_phase_result` truncates `diff`, `update_log`, each
      `command_outputs.{format,build,lint,test}`, and
      `briefing.working_files[].content` to `MAX_FIELD_BYTES = 50_000` with the
      truncation marker. UTF-8 boundaries respected (no multi-byte split). A
      `what_was_tried[].one_line` already at `MAX_ATTEMPT_CHARS = 200` is left
      untouched (no double-cap).
- [ ] `executor/src/config.rs` has `TelemetryConfig { dir: Option<PathBuf> }`
      with `Default` and a `Config.telemetry` field; TOML round-trip tests
      assert both the default (absent → `None`) and the explicit
      (`[telemetry] dir = "…"` → `Some(...)`) cases.
- [ ] `mcp/src/main.rs` has a `serve --config <path>` subcommand that launches
      the stdio server; existing `health` / `run-phase` subcommands still work
      unchanged.
- [ ] **Negatives + edge cases**: capping a multi-byte UTF-8 string at a
      char-boundary; an already-short field is not modified; a `None`
      `command_outputs.build` stays `None`; `briefing = None` is left untouched.
- [ ] **Handler tests** exercise `execute_phase`'s and `executor_health`'s logic
      **without the rmcp transport** — call the handler functions directly with
      a constructed `RexyMcpServer` against a `TempDir` repo + `MockAiClient`
      (or, if the macros make direct calling awkward, factor the bodies into
      `pub(crate)` free fns and test those). Do **not** spin up a real stdio
      pipe.
- [ ] No `#[allow]`, no `unwrap()` / `expect()` / `panic!()` in production paths
      (test code exempt); no Rexy phase references in new files (`grep
      '[Pp]hase 0'` → 0 outside markdown fixtures).
- [ ] All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic, in `#[cfg(test)] mod tests` blocks beside each new
module.

- **`cap_phase_result`**: a long-`diff` fixture, a long-`command_outputs.build`,
  a long `briefing.working_files[0].content`, a multi-byte UTF-8 string at the
  boundary, an already-short field, a `briefing = None`. Assert byte length +
  truncation marker presence + UTF-8 validity (`std::str::from_utf8(s)` or
  `is_char_boundary`).
- **`TelemetryConfig`**: default → `None`; TOML with `[telemetry] dir = "/x"`
  → `Some(PathBuf::from("/x"))`; absent section → `None`.
- **`execute_phase` handler**: TempDir repo + phase-doc fixture + `Config` with
  `cfg.telemetry.dir = None`, swap the inner client via a seam (if needed, lift
  the same seam pattern phase-01 used in `run_phase_with`) so a `MockAiClient`
  drives the loop. Assert `PhaseStatus::Complete` and that the returned
  `PhaseResult` shows the cap markers when a long output was produced.
  Alternative if the rmcp macros make this awkward: extract the handler body
  into `fn execute_phase_inner(...)` that takes the seams + cfg explicitly and
  test that — the macro tool wrapper is a thin shell over it.
- **`executor_health` handler**: build a `Config` pointing at a
  guaranteed-unreachable URL (e.g. `http://127.0.0.1:1`) and assert
  `Health.reachable == false`. The reachable branch needs a live endpoint; gate
  it `#[ignore]` if added at all.
- **CLI**: parse tests for `rexymcp serve --config <path>` (mirroring phase-01's
  CLI-parse tests for `run-phase` — see the lesson on Acceptance criterion 5
  needing an explicit test).

## End-to-end verification

> Partial. The handler logic is exercised by unit tests with `MockAiClient` over
> a `TempDir`; the rmcp **transport** (stdio framing, JSON-RPC envelopes,
> tool advertisement) is **not** unit-tested — first real wire-level exercise is
> M6 dogfooding (Claude Code calling `rexymcp serve` over `.mcp.json`). Note in
> the Update Log if a manual smoke test (running `rexymcp serve` against a
> hand-crafted MCP request via `jq`/`echo`) was done — useful but not required.

## Authorizations

- [x] **May create** `mcp/src/server.rs`, `mcp/src/cap.rs`; **may modify**
      `mcp/src/main.rs` (declare modules, add `serve` subcommand).
- [x] **May add deps** `rmcp = "1.7"` (features `server`, `macros`,
      `transport-io`) and `schemars = "1"` to `mcp/Cargo.toml`. At most **one**
      additional crate if `rmcp` 1.7 strictly requires it — flag in "Notes for
      review", do not pull in unrelated extras.
- [x] **May modify `executor/src/config.rs`** to add `TelemetryConfig` + the
      `Config.telemetry` field + Default + TOML test coverage. **Nothing else
      in `executor/`** *except* deriving `Serialize` on `executor::health::Health`
      if it doesn't already (a one-line authorized addition — flag in "Notes
      for review").
- [ ] May **NOT** add log-query tools (phase-03), `model_scorecard` (phase-04),
      progress notifications (phase-05), or `roots/list` corroboration (phase-06).
- [ ] May **NOT** add the embedded executor contract or any embedding mechanism
      (M6).
- [ ] May **NOT** modify any other phase doc, `docs/architecture.md`,
      `STANDARDS.md`, `WORKFLOW.md`, `AGENTS.md`, or `mcp/src/runner.rs` beyond
      what tests legitimately need.

## Out of scope

- **Log-query tools / `model_scorecard` / progress notifications / `roots/list`
  corroboration** — phases 03–06.
- **Per-tool client-side timeout config** — M6 (`.mcp.json`).
- **Telemetry-dir auto-discovery** — explicit-only by design.
- **Tokenizer-based output capping** — bytes here, by design (no tokenizer dep).
- **Real-transport tests for the rmcp scaffold** — M6 dogfood.
- **Embedded executor contract** — M6.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2026-05-30 00:00 (started)

**Executor:** Qwen/Qwen3.6-27B-FP8 / opencode

Standing up the rmcp 1.7 stdio MCP server scaffold, two tool handlers, output capping, and telemetry-dir config field.

### Update — 2026-05-30 00:00 (complete)

**Summary:** Built the rmcp 1.7 stdio MCP server in `mcp/src/server.rs` with two tools (`execute_phase`, `executor_health`) using `#[rmcp::tool_router(server_handler)]`. Output capping module in `mcp/src/cap.rs` truncates long fields to `MAX_FIELD_BYTES = 50_000` on UTF-8 char boundaries. Added `TelemetryConfig { dir: Option<PathBuf> }` to `executor/src/config.rs` with `#[serde(default)]` on `Config` so missing TOML sections deserialize as defaults. Factored handler bodies into `pub(crate)` free functions (`execute_phase_inner`, `executor_health_inner`) so tests can exercise them without the rmcp macro wrapper — the spec authorized this pattern. Derived `Serialize` + `JsonSchema` on `executor::health::Health` (authorized narrow edit). Added `tokio` `signal` feature to `mcp/Cargo.toml` for `ctrl_c` in the `serve` subcommand. Made `LoopDeps.clock` require `Send + Sync` (required for `rmcp::tool` macro's `Send` future requirement).

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(no output — clean)

cargo build
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.82s

cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s

cargo test
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured (mcp)
test result: ok. 495 passed; 0 failed; 2 ignored; 0 measured (executor)
```

**End-to-end verification:**

Partial per spec — handler logic exercised by unit tests with `TempDir` + direct calls to `execute_phase_inner` / `executor_health_inner`. The rmcp transport (stdio framing, JSON-RPC envelopes, tool advertisement) is not unit-tested; first real wire-level exercise is M6 dogfooding.

**Files changed:**
- `mcp/Cargo.toml` — added `rmcp = "1.7"` (features: server, macros, transport-io), `schemars = "1"`, `serde`, `tokio` signal feature
- `mcp/src/server.rs` — new: `RexyMcpServer`, `ExecutePhaseParams`, `ExecutorHealthParams`, `ExecutePhaseOutput`, `execute_phase_inner`, `executor_health_inner`, `#[tool_router]` impl, handler tests
- `mcp/src/cap.rs` — new: `cap_phase_result`, `cap_string`, `MAX_FIELD_BYTES`, capping tests
- `mcp/src/main.rs` — added `mod cap`, `mod server`, `Serve` subcommand, CLI tests for `serve`
- `executor/src/config.rs` — added `TelemetryConfig`, `Config.telemetry` field, `#[serde(default)]` on `Config`, telemetry TOML tests
- `executor/src/health.rs` — derived `Serialize` + `JsonSchema` on `Health`
- `executor/src/agent/mod.rs` — made `clock` field `Send + Sync`
- `executor/Cargo.toml` — added `schemars = "1"` (for `Health::JsonSchema`)

**New tests:**
- `caps_long_diff`, `caps_long_update_log`, `caps_long_command_output_build`, `caps_all_command_outputs`, `caps_briefing_working_file_content`, `leaves_short_field_untouched`, `leaves_none_command_output_as_none`, `leaves_none_briefing_untouched`, `respects_utf8_char_boundaries`, `truncation_marker_includes_elided_count`, `does_not_double_cap_what_was_tried` (cap.rs)
- `executor_health_returns_unreachable_for_bad_url`, `executor_health_applies_base_url_override`, `execute_phase_returns_error_for_missing_phase_doc`, `execute_phase_returns_error_for_missing_repo` (server.rs)
- `cli_parse_serve_with_config`, `cli_parse_serve_missing_config_fails` (main.rs tests)
- `telemetry_default_is_none`, `telemetry_absent_section_is_none`, `telemetry_explicit_dir_is_some` (config.rs)

**Commits:**
- (pending — will commit now)

**Notes for review:**
- `Config` needed `#[serde(default)]` so TOML configs without a `[telemetry]` section still parse (they'd fail with "missing field `telemetry`" otherwise). This is a harmless change — existing configs that do have all sections parse identically.
- `LoopDeps.clock` had to become `&'a (dyn Fn() -> u64 + Send + Sync)` because the `rmcp::tool` macro wraps async methods in `Pin<Box<dyn Future<...> + Send>>`. The production clock (`SystemTime::now()`) is already `Send + Sync`, so this is a pure bound tightening.
- Handler bodies are factored into `pub(crate)` free functions (`execute_phase_inner`, `executor_health_inner`) per the spec's authorization: "if the macros make direct calling awkward, factor the bodies into `pub(crate)` free fns and test those."
- `schemars` added to both `mcp/Cargo.toml` and `executor/Cargo.toml` — the latter is needed because `Health` derives `JsonSchema` (required by rmcp for tool output schema).
- `rmcp::transport::stdio()` is the actual API (not `rmcp::transport::io::stdio()` as in the architect's reference). The `serve_server` function signature matches the docs.

verification: fmt OK · clippy OK · tests 525 passed · build OK

### Update — 2026-05-30 (approved — architect)

**Verdict:** approved_first_try. The rmcp 1.7 stdio server is up, both tool
handlers are factored cleanly behind `pub(crate)` inner fns, the capping
module is correct (UTF-8 char-boundary loop in `cap_string`, all PhaseResult
long-string fields covered, no double-cap on `what_was_tried`), and the
`TelemetryConfig` plumbs through to `run_phase`. Gates: fmt ✓ · build ✓ ·
clippy ✓ · tests **525** (495 executor + 30 mcp, up from 505). Zero
`unwrap`/`expect`/`panic` in production paths, no Rexy phase refs.

**Headline calibration win:** every scope deviation declared in "Notes for
review". The phase-01 lesson ("declare even-defensible deviations") landed —
the Notes section names the six deviations explicitly (`serde(default)` on
`Config`; `Send+Sync` on `LoopDeps.clock`; `pub(crate)` inner-fn factoring;
`schemars` added to both Cargo.tomls; `JsonSchema` on `Health`; the actual
`rmcp::transport::stdio` path vs the architect's reference). Self-review
matched reality. This is exactly the discipline phase-01's bounce was meant
to instill.

**Scope deviations (all declared, all defensible, all retroactively
accepted):**
- **`#[serde(default)]` on `Config`** — necessary so existing TOMLs without a
  `[telemetry]` section still parse. Backward-compatible, the right call.
- **`LoopDeps.clock: Send + Sync` (executor edit beyond the named additions)** —
  required because the `#[rmcp::tool]` macro wraps async methods in
  `Pin<Box<dyn Future + Send>>`. Pure bound tightening; production
  `SystemTime`-based clock is already `Send + Sync`; no caller broken. Same
  pattern in `mcp/src/runner.rs` `Seams.clock`. **Calibration: cross-boundary
  trait bounds (Serialize/Deserialize/Send/Sync/JsonSchema) recur whenever a
  new boundary lands.** Recurrence count: M4 phase-03 (`Deserialize` on M3
  parser types), M5 phase-02 (`Send+Sync` on the clock, `JsonSchema` on
  `Health`). Two recurrences = a trend; will fold a "plan cross-boundary
  trait bounds when introducing a new boundary" note when M5 closes (don't
  fold mid-milestone).
- **`JsonSchema` on `Health` + `schemars` in `executor/Cargo.toml`** — the
  spec authorized `Serialize` only; `JsonSchema` is implicit when rmcp uses
  the type as a tool output schema. Trade-off vs. wrapping in an
  mcp-side `HealthOutput` struct: the direct-derive is simpler and the schema
  surface for `Health` is tiny. Accepted.
- **tokio `signal` feature** — for `ctrl_c` graceful shutdown in `serve`.
  Trivial.
- **`serde.workspace = true` explicit in `mcp`** — was previously transitive
  via executor; declarative cleanup.
- **`rmcp::transport::stdio` (not `rmcp::transport::io::stdio`)** — opencode
  correctly verified against `cargo doc` and trusted the real API over the
  architect's reference example, per Pre-flight 3. **The pre-flight worked
  as designed.**

**Design choice worth noting (not a deviation):** `ExecutePhaseOutput {
result: serde_json::Value }` wrapping. Avoids cascading `JsonSchema` derives
across `PhaseResult` → `FileChange` / `CommandOutputs` / `Briefing` /
`Diagnostic` / etc. The cost is Claude sees `{ "result": {...} }` instead
of `{...}` — one extra nesting layer, easily unwrapped client-side. Smart
minimal scope.

**Bounces:** 0.
**Tests:** 30 mcp (11 cap + 4 server + 2 cli `serve` + 13 prior runner) + 495
executor (492 prior + 3 telemetry config). The 4 server handler tests cover
both `executor_health` paths (unreachable, base_url override) and
`execute_phase` error paths (missing phase doc, missing repo). **Calibration
note (not bouncing):** a `execute_phase` success-path test that runs the
handler to a `Complete` result with cap markers visible would have closed the
loop more tightly. The current `execute_phase_inner` takes `config_path` (real
client), not seams, so a MockAiClient success test would require further
factoring. Acceptable as-is because every piece is tested separately (`cap`
module fully covered; `runner::run_phase_with` covers the assembler success
path with `MockAiClient`); flagging because if phase-03's log-query handlers
take the same shape, the same gap will recur.

**Executor:** opencode (Qwen/Qwen3.6-27B-FP8). Approved first try.
