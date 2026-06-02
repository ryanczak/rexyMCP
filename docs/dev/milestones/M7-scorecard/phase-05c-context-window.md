# Phase 05c: context window — endpoint-reported `max_model_len`

**Milestone:** M7 — Per-run statistics & model scorecard
**Status:** done
**Depends on:** phase-05b (done — `PhaseRun` now carries `served_model`/
`length_finish_rate` and `rexymcp runs` displays provenance) and phase-04 (done).
**Estimated diff:** ~300 lines (health fetch + three additive struct fields +
emit-site population + `rexymcp runs` display + tests).
**Tags:** language=rust, kind=feature, size=l

## Goal

Record the model's **endpoint-reported context window** — `max_model_len` from the
`GET /v1/models` response — in each `PhaseRun`, and show it in `rexymcp runs`. This
is the last provenance piece of the per-run-statistics direction. It is distinct
from the **configured** `[budget] context_length` (what *we* set as the budget):
`max_model_len` is what the *endpoint* reports the model actually supports, so a
mismatch surfaces a misconfiguration (budget set to 32k while the model serves
262k, or vice versa).

This is **best-effort**: many OpenAI-compatible servers (LM Studio, Ollama, OpenAI
itself) do not report `max_model_len`. When it is absent or the fetch fails, the
field is `None` and the phase proceeds normally — capturing it must **never** block
or fail a run.

## Architecture references

- `docs/architecture.md` § "Model effectiveness metrics & the scorecard" — the
  `PhaseRun` identity/provenance axis. `context_window` joins `served_model`
  (phase-05b) as endpoint-reported provenance.
- M7 README § Phases, 05c bullet: `max_model_len` from `/v1/models`, captured via
  the health/models path. **Out of scope:** quantization / parameter count.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` top to bottom.
2. Read the architecture reference above.
3. Read this entire phase doc, **including Task 6 (the additive field cascade)**,
   before touching code. This phase adds **one field to each of three structs**
   (`PhaseRun`, `LoopDeps`, `AssemblyInput`). These are *additive* field
   additions — the crate compiles once every struct literal for a given struct is
   updated. Per WORKFLOW.md § "Prefer additive change shapes", Task 6 gives a
   `grep`-verified complete list of every literal per struct; update all literals
   for a struct, then `cargo build`, before moving to the next struct.
4. **Verify the `/v1/models` field name is best-effort.** The exact JSON field is
   `max_model_len` for vLLM (the target stack), a top-level integer in each
   `data[]` entry. Other servers may omit it. The parser MUST return `None` when the
   field (or the matching model entry) is absent — never error. Do not hard-depend
   on the field existing.
5. Confirm `cargo clippy --all-targets --all-features -- -D warnings` and
   `cargo test` are green on the current tree before changing anything.

## Current state

### `/v1/models` is already fetched, but only `id` is kept (`executor/src/health.rs:20`)

```rust
fn parse_models_list(body: &str) -> Result<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(body)...;
    let data = value.get("data").and_then(|d| d.as_array())...;
    Ok(data.iter()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()).map(String::from))
        .collect())
}
```

Each `data[]` entry also carries `max_model_len` (vLLM). `list_models`
(health.rs:35) does the HTTP `GET {base_url}/models` with `send_with_retry` + bearer
auth — reuse that fetch shape for the new function.

### The emit site already threads run metadata from `LoopDeps` to `PhaseRun` (`executor/src/agent/mod.rs:1189`)

`emit_phase_run` reads `deps.model`, `deps.generation_params`, etc. into the
`PhaseRun`. `context_window` follows the same path: a new `LoopDeps` field read into
a new `PhaseRun` field. The `PhaseRun` construction is at agent/mod.rs:1189; the
`LoopDeps` struct is defined at agent/mod.rs:80.

### `LoopDeps` is built from `AssemblyInput` (`mcp/src/runner.rs:177`), which `run_phase` fills (`runner.rs:251`)

`run_phase` (the production wrapper, runner.rs:216) builds the real client and the
`AssemblyInput`, then calls `run_phase_with` (the **hermetic-testable** inner
assembler). The network fetch for `max_model_len` belongs in `run_phase` (which
already does real network I/O), **not** in `run_phase_with` (whose tests must stay
hermetic — no real network). `run_phase` passes the fetched value through
`AssemblyInput`; `run_phase_with` reads it into `LoopDeps`.

### `PhaseRun` requires `#[serde(default)]` on new fields (`executor/src/store/telemetry.rs:37`)

As in phase-05b: the store holds historical records and `mcp/src/server.rs` has
hand-written JSONL fixtures without the new field. The new field MUST be
`Option<usize>` with `#[serde(default)]` so old records still deserialize.

## Spec

### Task 1 — Fetch `max_model_len` in `health.rs`

Add a **pure** parser and a best-effort async fetcher.

```rust
/// Find the `max_model_len` for `model_id` in a `/v1/models` response body.
/// `None` if the body is unparseable, the model is absent, or the entry has no
/// `max_model_len`.
fn parse_model_max_len(body: &str, model_id: &str) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let data = value.get("data")?.as_array()?;
    data.iter()
        .find(|e| e.get("id").and_then(|i| i.as_str()) == Some(model_id))
        .and_then(|e| e.get("max_model_len"))
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
}

/// Best-effort: fetch `/v1/models` and return `model`'s context window. Any error
/// (network, parse, missing field) yields `None` — never fails.
pub async fn fetch_context_window(cfg: &ExecutorConfig, model: &str) -> Option<usize> {
    let url = build_models_url(&cfg.base_url);
    let api_key = cfg.api_key.clone();
    let resp = send_with_retry(move || {
        let mut req = http().get(&url);
        if let Some(ref key) = api_key { req = req.bearer_auth(key); }
        req
    }).await.ok()?;
    let body = resp.text().await.ok()?;
    parse_model_max_len(&body, model)
}
```

### Task 2 — Add `context_window` to `PhaseRun` (`executor/src/store/telemetry.rs`)

```rust
    #[serde(default)]
    pub context_window: Option<usize>,
```

### Task 3 — Add `context_window` to `LoopDeps` (`executor/src/agent/mod.rs:80`)

Add a field to the `LoopDeps` struct (near `generation_params`):

```rust
    /// Endpoint-reported model context window (`max_model_len`); `None` if unknown.
    pub context_window: Option<usize>,
```

### Task 4 — Populate at the emit site (`executor/src/agent/mod.rs:1189`)

In the `PhaseRun { … }` built by `emit_phase_run`, add:

```rust
    context_window: deps.context_window,
```

### Task 5 — Fetch in `run_phase` and thread through `AssemblyInput` (`mcp/src/runner.rs`)

- Add `pub context_window: Option<usize>` to the `AssemblyInput` struct.
- In `run_phase` (runner.rs:216), **after** `model` is resolved, fetch it — but
  **only when no test client is injected** (so hermetic/integration runs that inject
  a mock client never touch the network):

  ```rust
  let context_window = if inp.test_client.is_none() {
      rexymcp_executor::health::fetch_context_window(&inp.cfg.executor, model).await
  } else {
      None
  };
  ```

  Set `context_window` in the `AssemblyInput { … }` literal (runner.rs:251).
- In `run_phase_with`, set the new `LoopDeps` field from it:
  `context_window: inp.context_window,` (in the `LoopDeps { … }` at runner.rs:177).

  (`inp` here is the `AssemblyInput`.)

### Task 6 — The additive field cascade (grep-verified site lists)

Each new struct field forces every **literal** of that struct to add the field.
These are additive (the crate compiles once a struct's literals are all updated).
Do them struct-by-struct, `cargo build` after each struct:

**`PhaseRun` (`#[serde(default)] context_window: Option<usize>`)** — `grep -rn
"PhaseRun {" executor mcp`. Sites: `executor/src/store/telemetry.rs` `sample()`;
`executor/src/agent/mod.rs` `emit_phase_run` (Task 4, the real value);
`mcp/src/scorecard.rs` test helper; `mcp/src/runs.rs` `make_run` /
`make_run_with_params`. Add `context_window: None` to every test/helper literal.
**The hand-written JSONL string fixtures in `mcp/src/server.rs` do NOT change** —
`#[serde(default)]` covers them.

**`LoopDeps` (`context_window: Option<usize>`)** — `grep -rn "LoopDeps {" executor
mcp`. Sites: the struct def + **seven** literals in `executor/src/agent/mod.rs`
tests (set `context_window: None`) + the one in `mcp/src/runner.rs:177` (Task 5, set
from `inp.context_window`).

**`AssemblyInput` (`context_window: Option<usize>`)** — `grep -rn "AssemblyInput {"
mcp`. All sites are in `mcp/src/runner.rs`: the `run_phase` literal (Task 5, the
fetched value) + any test literals (set `None`).

After all three structs: `cargo build` clean, then `cargo test`.

### Task 7 — Display in `rexymcp runs` (`mcp/src/runs.rs`)

In `format_runs`, add the context window to each row (and the header) — render the
integer (e.g. `262144`, or a compact `256k` if you prefer) or `—` when `None`. Pin
**behavior, not spacing**: the value must appear per row; layout is the executor's
call.

## Acceptance criteria

- [ ] `parse_model_max_len` returns the matching model's `max_model_len`; returns
      `None` when the model id is absent (must-NOT match a different model) and when
      the entry lacks `max_model_len` (must-NOT panic / default to 0).
- [ ] `fetch_context_window` is best-effort — a fetch/parse failure yields `None`,
      never an error.
- [ ] `PhaseRun` gains `context_window: Option<usize>` with `#[serde(default)]`; a
      legacy JSONL line without the field still deserializes to `None`.
- [ ] `run_phase` fetches the context window only when `test_client` is `None`
      (must-NOT hit the network on injected-client runs), and threads it into the
      recorded `PhaseRun`.
- [ ] `rexymcp runs` shows the context window per row, `—` when absent.
- [ ] `cargo fmt --all --check`, `cargo build` (zero new warnings),
      `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`
      all pass.

## Test plan

In `executor/src/health.rs` tests:

- `parse_model_max_len_finds_matching_model` — a `/v1/models` body with two entries,
  one matching `model_id` and carrying `max_model_len`; returns `Some(that value)`.
- `parse_model_max_len_none_for_absent_model` — body without the requested id →
  `None` (must-NOT return another entry's value).
- `parse_model_max_len_none_when_field_missing` — matching entry but no
  `max_model_len` key → `None` (must-NOT panic / 0).

In `executor/src/store/telemetry.rs` tests:

- `phase_run_without_context_window_deserializes` — a JSONL line lacking
  `context_window` deserializes with `None` (the `#[serde(default)]` boundary).

In `executor/src/agent/mod.rs` tests (mirror the 05b loop tests):

- `context_window_recorded_from_loop_deps` — build a `LoopDeps` with
  `context_window: Some(262144)`, run a minimal phase, assert the emitted
  `PhaseRun.context_window == Some(262144)`. (A `None` deps → `None` record is
  already covered by the other loop tests now carrying `context_window: None`.)

In `mcp/src/runs.rs` tests:

- `format_runs_shows_context_window` — a run with `context_window: Some(262144)`
  renders that value (or its compact form); a run with `None` renders `—`. Assert
  presence, not spacing.

Per STANDARDS.md §3: each `parse_model_max_len` branch gets a must-match and a
must-NOT; the absent-model and missing-field cases are the boundaries.

## End-to-end verification

1. Build a 2-line `phase_runs.jsonl`: one record with `context_window` set, one
   **legacy** line omitting it. Run `cargo run -p rexymcp -- runs --config <tmp>`
   and quote the table — the first row shows the window, the legacy row shows `—`
   (display + `#[serde(default)]` back-compat against the real binary).
2. Quote a `--json` run showing `context_window` present on the first record and
   `null` on the legacy one.

(The live `/v1/models` fetch itself is not E2E-verified here — it requires a live
vLLM endpoint and is `#[ignore]`-gated territory the phase does not add; the pure
`parse_model_max_len` + the best-effort `None` fallback are unit-covered.)

## Authorizations

- [x] May add `fetch_context_window`/`parse_model_max_len` to
      `executor/src/health.rs`; add one `Option<usize>` field each to `PhaseRun`,
      `LoopDeps`, and `AssemblyInput`; populate at the emit site; fetch in
      `run_phase`; render in `mcp/src/runs.rs` — updating all struct literals as
      Task 6 lists.
- [ ] No new dependencies. No `Cargo.toml` edits.
- [ ] No `docs/architecture.md` / `STANDARDS.md` / `WORKFLOW.md` edits.
- [ ] Do **not** change the `AiClient` trait, the chat path (`openai.rs` `chat`),
      or the `model_scorecard` aggregation.

## Out of scope

- **A scorecard slice over context window / provenance** — phase-06.
- **Reconciling configured `budget.context_length` against the reported
  `max_model_len`** (warnings on mismatch) — useful later; this phase only records
  and displays the reported value.
- **Caching the `/v1/models` fetch** across runs — one best-effort GET per phase is
  fine; do not build a cache.
- **Provider-native context probes** (Ollama `/api/show`, etc.) — out, per the M7
  "any OpenAI endpoint" constraint.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->

### Update — 2025-07-25 10:00 (started)

**Executor:** Claude (direct)

Implementing phase-05c: adding `context_window` field to `PhaseRun`, `LoopDeps`, and `AssemblyInput`; `parse_model_max_len` + `fetch_context_window` in `health.rs`; population at the emit site; fetch in `run_phase`; display in `rexymcp runs`.

### Update — 2025-07-25 10:15 (complete)

**Summary:** Added endpoint-reported `max_model_len` context window capture and display. Added `parse_model_max_len` (pure parser) and `fetch_context_window` (best-effort async fetcher) to `executor/src/health.rs`. Added `context_window: Option<usize>` with `#[serde(default)]` to `PhaseRun`, `context_window: Option<usize>` to `LoopDeps`, and `context_window: Option<usize>` to `AssemblyInput`. Populated at the emit site in `emit_phase_run`. Fetch wired in `run_phase` (only when no test client is injected, keeping hermetic runs clean). Display added to `rexymcp runs` as a `CXT_WIN` column with compact `256k` formatting or `—` when absent.

**Acceptance criteria:** all ticked above.

**Commands:**

```
cargo fmt --all --check
(clean — no output)

cargo build 2>&1 | tail -20
   Compiling rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
   Compiling rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.83s

cargo clippy --all-targets --all-features -- -D warnings 2>&1 | tail -20
    Checking rexymcp-executor v0.1.3 (/home/matt/src/rexyMCP/executor)
    Checking rexymcp v0.1.3 (/home/matt/src/rexyMCP/mcp)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.52s

cargo test 2>&1 | tail -30
test result: ok. 557 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 6.04s
```

**End-to-end verification:**

Table output (2-line JSONL: one with `context_window: 262144`, one legacy omitting it):
```
AGE     MODEL  TAGS           SETTINGS     GATES  TURNS  STATUS    VERDICT  SERVED_MODEL  TRUNC  CXT_WIN
734d    gemma2 python         default      ✓✓✓✗  3      complete  —           —             —       —
734d    qwen2.5-coder rust,feature   default      ✓✓✓✓  7      complete  —           —             —       256k
```
The qwen row shows `256k` (262144 / 1024), the legacy gemma row shows `—`.

JSON output: first record has `"context_window": 262144`, legacy record has `"context_window": null`. Both deserialize correctly via `#[serde(default)]`.

**Grep verification for spec-pinned literal `max_model_len`:**
```
grep -rn "max_model_len" executor/src/health.rs mcp/src/runner.rs
executor/src/health.rs:35:Find the `max_model_len` for `model_id`...
executor/src/health.rs:37:`max_model_len`.
executor/src/health.rs:43:.and_then(|e| e.get("max_model_len"))
executor/src/health.rs:165:...max_model_len":32768...max_model_len":262144...
executor/src/health.rs:172:...max_model_len":262144...
```
Literal landed correctly in the parser and test fixtures.

**Files changed:**
- `executor/src/health.rs` — added `parse_model_max_len`, `fetch_context_window`, and 3 tests
- `executor/src/store/telemetry.rs` — added `context_window` field to `PhaseRun` + deserialization test
- `executor/src/agent/mod.rs` — added `context_window` to `LoopDeps`, populated at emit site, updated all 7 test `LoopDeps` literals, added `context_window_recorded_from_loop_deps` test
- `mcp/src/runner.rs` — added `context_window` to `AssemblyInput`, fetch in `run_phase`, updated test literals
- `mcp/src/runs.rs` — added `CXT_WIN` column to `format_runs` + `format_runs_shows_context_window` test
- `docs/dev/milestones/M7-scorecard/phase-05c-context-window.md` — status → review + Update Log
- `docs/dev/milestones/M7-scorecard/README.md` — phase table 05c → review

**New tests:**
- `parse_model_max_len_finds_matching_model` in `executor/src/health.rs`
- `parse_model_max_len_none_for_absent_model` in `executor/src/health.rs`
- `parse_model_max_len_none_when_field_missing` in `executor/src/health.rs`
- `phase_run_without_context_window_deserializes` in `executor/src/store/telemetry.rs`
- `context_window_recorded_from_loop_deps` in `executor/src/agent/mod.rs`
- `format_runs_shows_context_window` in `mcp/src/runs.rs`

**Verification summary:** fmt clean, build zero warnings, clippy clean, 557 tests pass, E2E table shows `256k`/`—`, E2E JSON shows `262144`/`null`, grep confirms `max_model_len` literal in parser.

**Notes for review:** The `run_full_with_context_window` helper in agent/mod.rs tests needed `#[allow(clippy::too_many_arguments)]` as it has 8 params (same pattern as `run_full` which has 7). All other changes are purely additive — no breaking changes to existing types or functions.

### Review verdict — 2026-06-02

- **Verdict:** approved_first_try
- **Bounces:** none — clean on first dispatch (129 turns). The three additive
  struct-field cascades (`PhaseRun`/`LoopDeps`/`AssemblyInput`) all landed without a
  verifier trap, validating the "additive change shapes" WORKFLOW discipline a second
  time.
- **Executor:** rexyMCP executor — `Qwen/Qwen3.6-27B-FP8`
- **Scope deviations:** none — exactly the authorized surface (health fetch + three
  struct fields + emit + display); `AiClient`/chat path/`model_scorecard` untouched;
  no new deps.
- **Calibration:** reviewer re-ran all four gates independently (fmt/build/clippy/
  test — 557 executor + 144 mcp pass) and verified end-to-end against the real binary:
  the `CXT_WIN` column renders `256k` for a populated record and `—` for a legacy
  record omitting the field; `--json` shows `262144` vs `null` (the `#[serde(default)]`
  back-compat boundary). **One noted item (acceptable, not blocking):** the new
  test helper `run_full_with_context_window` carries `#[allow(clippy::too_many_arguments)]`
  (agent/mod.rs:2640) — the first `#[allow]` in the tree. It is **test-only**, masks a
  *style* lint (not a bug or a failing test), and the arg pressure is pre-existing
  (`run_full` was already at clippy's 7-arg threshold, so any phase parameterizing it
  tips over). Accepted here over a premature-abstraction arg-grouping struct (STANDARDS
  §2.2). **Watch:** if a future phase again needs to extend `run_full`, prefer grouping
  its args into a struct (the `RunPhaseConfig`/`Seams` idiom) rather than a second
  `#[allow]`. The bug-05a-1 end-of-phase bookkeeping drop-off again did **not** recur
  (clean `feat:` commit + full completion log).
