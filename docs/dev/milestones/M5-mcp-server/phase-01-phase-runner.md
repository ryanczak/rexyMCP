# Phase 01: phase-runner wiring (config + phase doc → execute_phase)

**Milestone:** M5 — MCP server
**Status:** todo
**Depends on:** M4 (done)
**Estimated diff:** ~400 lines (runner module + CLI subcommand + tests)
**Tags:** language=rust, kind=feature, size=m

## Goal

Build the **composition root** that turns a `Config` + a phase-doc path + a
target-repo path into a single `agent::execute_phase` call — the glue the M5
`execute_phase` MCP tool (phase-02) will invoke. This is the leaf the rest of M5
depends on: the server's tool handlers, progress, and roots-corroboration all sit
on top of one `run_phase(...)`.

Net-new (no Rexy donor), **no MCP dependency yet** (`rmcp` is phase-02), and fully
unit-testable with `MockAiClient` over a `TempDir`. Ships three pieces in a new
`mcp/src/runner.rs`: a pure **phase-doc parser**, a **registry builder**, and the
**`run_phase` assembler** (split behind a clock/seam-injecting inner fn so tests
need no real client/clock).

## Architecture references

- `docs/architecture.md` — "Layer 2 — `mcp` crate (binary)" (`execute_phase` args:
  `phase_doc_path`, `repo_path`, optional `model`; "Calls the `executor` library
  in-process and returns `PhaseResult`").
- M5 README Notes — "The system clock lives at the composition root", "The
  executor contract + STANDARDS.md are inputs", "Telemetry dir is cross-project".
- M4: `agent::{execute_phase, PhaseInput, LoopDeps}`, `agent::verify::RealVerifier`,
  `agent::command::RealCommandRunner`, `ai::OpenAiClient`, `context::Budget`,
  `security::Scope`, `tools::{read_file, write_file, patch, find_files, search,
  symbols, bash, ToolRegistry, Tool}`, `ai::ToolSchema`,
  `store::telemetry::GenerationParams`, `config::{Config, CommandConfig}`.

## Pre-flight

1. Read `docs/dev/STANDARDS.md` and the M5 README (esp. the four Notes:
   composition-root clock, contract-is-input, cross-project telemetry, capping).
2. Read this entire phase doc.
3. Confirm M4 is `done` and the workspace builds clean. Confirm the production
   seams are public: `OpenAiClient::new(api_key, model, base_url)`,
   `RealVerifier`, `RealCommandRunner`, `Budget::from_context(context_length,
   max_context_pct)`, `GenerationParams::default()`, and the seven tool
   constructors (each `(Scope) -> Arc<dyn Tool>`; `bash` also takes a timeout).
   They are — **no `executor/` edit is needed or permitted** in this phase.

## Spec

Create `mcp/src/runner.rs`; declare `mod runner;` in `mcp/src/main.rs`. Three
pieces:

### 1. `parse_phase_doc(markdown: &str) -> PhaseDocFields`

A **pure** parser over the phase-doc markdown. `PhaseDocFields { goal: String,
acceptance_criteria: String, tags: Vec<String> }`.

- `goal` — the text of the `## Goal` section: everything after the `## Goal`
  heading up to the next `## ` heading (trimmed). Absent section → empty string.
- `acceptance_criteria` — same extraction for the `## Acceptance criteria` section.
- `tags` — parse the `**Tags:**` line (e.g. `**Tags:** language=rust, kind=feature,
  size=m`) into `["language=rust", "kind=feature", "size=m"]` (split on commas,
  trim each). Absent line → empty `Vec`.

Section extraction is line-oriented (match a heading line, collect until the next
`## ` / `# ` line or EOF). No regex crate needed; plain string scanning. Tolerate
stray surrounding whitespace.

The **phase id** (`"phase-01"`) is **not** parsed from content — derive it in
`run_phase` from the phase-doc *path stem* (see below). `parse_phase_doc` does not
take or return it.

### 2. `build_registry(scope: &Scope, bash_timeout_secs: u32) -> (ToolRegistry, Vec<ToolSchema>)`

Register the full built-in tool set scoped to the repo root, in a **deterministic
order**: `read_file`, `write_file`, `patch`, `find_files`, `search`, `symbols`,
`bash` (the last with `bash_timeout_secs`). Then derive the `Vec<ToolSchema>` by
iterating the registry and mapping each tool's `name()` / `description()` /
`schema()` into `ToolSchema { name, description, parameters }` — one schema per
registered tool, same order. (`Tool` exposes `schema(&self) -> serde_json::Value`;
that `Value` is the `parameters`.)

### 3. The assembler — `run_phase` + an inner seam

Split assembly from production-seam construction so the assembler is unit-testable
without a real client/clock (the M4 "inject IO behind a seam" rule, STANDARDS
§3.3):

```rust
// Inner: takes the seams + clock as injected params. Hermetic-testable.
#[allow(clippy::too_many_arguments)]  // ONLY if clippy demands it AND the phase
                                      // doc authorizes — otherwise group args.
async fn run_phase_with(
    cfg: &Config,
    phase_doc_path: &Path,
    repo_path: &Path,
    executor_contract: &str,
    standards: &str,
    model: &str,
    telemetry_dir: Option<&Path>,
    client: &dyn AiClient,
    verifier: &dyn FileVerifier,
    runner: &dyn CommandRunner,
    clock: &dyn Fn() -> u64,
) -> Result<PhaseResult>;

// Production wrapper: builds the REAL seams + system clock, delegates.
pub async fn run_phase(
    cfg: &Config,
    phase_doc_path: &Path,
    repo_path: &Path,
    executor_contract: &str,
    standards: &str,
    model_override: Option<&str>,
    telemetry_dir: Option<&Path>,
) -> Result<PhaseResult>;
```

> **Note on `#[allow]`:** the hard rules forbid `#[allow]` to mask a diagnostic.
> Prefer **grouping the seams into a small struct** (e.g. a private `Seams<'a> {
> client, verifier, runner, clock }`) over a 12-arg fn — that is the clean fix and
> needs no allow. Only if a justified clippy lint remains may you use a *scoped*
> `#[allow]` with a one-line reason — and if you reach for one, **stop and note it
> in "Notes for review"** rather than landing it silently.

`run_phase_with` does the assembly:
1. Read the phase-doc file at `phase_doc_path`; `parse_phase_doc` it.
2. Derive `phase` from the path file stem: strip the trailing description so
   `phase-01-phase-runner.md` → `"phase-01"` (take the `phase-<NN>` prefix; if the
   stem doesn't match that shape, use the whole stem).
3. `Scope::new(repo_path)` — propagate a scope error as `Err` (a real
   infra/usage failure, not a model-visible outcome).
4. `build_registry(&scope, cfg…)` → `(registry, tools)`.
5. `Budget::from_context(cfg.budget.context_length, cfg.budget.max_context_pct)`.
   **`context_length` does not exist on `BudgetConfig` yet — add it** (see
   Adaptation 5 + the authorized `config.rs` edit): the model's context-window
   size in tokens, from which the ceiling is `context_length × max_context_pct /
   100`.
6. Build `PhaseInput { executor_contract, standards, phase_doc, goal,
   acceptance_criteria, phase, tags }` (`phase_doc` = the raw file text).
7. Build `LoopDeps { client, registry: &registry, tools: &tools, budget: &budget,
   max_turns: cfg.budget.max_turns, project_root: repo_path, model, session_id:
   &generate_session_id(), clock, verifier, commands: &cfg.commands, runner,
   generation_params: GenerationParams::default(), telemetry_dir }`.
8. `agent::execute_phase(&input, deps).await` → return the `PhaseResult`.

`run_phase` (production) builds the client as
`OpenAiClient::new(cfg.executor.api_key.clone().unwrap_or_default(), model,
cfg.executor.base_url.clone())` where `model = model_override.unwrap_or(&cfg.
executor.model)`. `api_key` is `Option<String>` — an empty string when absent is
correct (local endpoints ignore it). `base_url` is passed as-is; the
`ExecutorConfig::default()` already supplies `http://localhost:1234/v1`, so no
further fallback belongs here. Then `RealVerifier`; `RealCommandRunner`; a system
clock closure `|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d|
d.as_millis() as u64).unwrap_or(0)` — then delegates to `run_phase_with`.

### 4. CLI caller (so the module is not dead code)

A binary crate with an uncalled `mod runner` triggers `dead_code` under
`clippy -D warnings`, and `#[allow(dead_code)]` is a hard-rule violation. Give
`run_phase` a real caller: add a **`run-phase` subcommand** to the existing clap
CLI in `mcp/src/main.rs`:

```
rexymcp run-phase --config <path> --phase-doc <path> --repo <path> [--model <id>]
```

It loads `Config` (`load_with_env`), reads `standards` best-effort from
`<repo>/docs/dev/STANDARDS.md` (empty if absent), uses an **empty
`executor_contract`** for now (embedding is M6 — leave a one-line note, not a
TODO), resolves `telemetry_dir = None` (cross-project resolution is phase-02),
calls `run_phase`, and prints the returned `PhaseResult` as pretty JSON. This is a
real, useful manual-execution entry point that phase-02's MCP tool will sit beside
(not replace). Output capping is **phase-02** — print in full here.

## Adaptations / decisions

1. **No embedding.** `executor_contract` + `standards` are **inputs**. M5 does not
   build the embedded contract (M6). The CLI passes empty contract + repo
   `STANDARDS.md`; the phase-02 tool will pass what the server resolves. Do not add
   an embedding mechanism here.
2. **System clock at the root only.** The real `SystemTime` clock is injected by
   `run_phase`; `run_phase_with` never reads wall-clock. This keeps the loop
   deterministic and the assembler testable with a fixed clock.
3. **`telemetry_dir` is a plumbed `Option<&Path>`.** Do not hardcode it under the
   repo or pick a cross-project location here — that policy is phase-02. `None`
   simply disables `PhaseRun` emission (the loop already treats it that way).
5. **`BudgetConfig` gains a `context_length` field (authorized `executor/`
   edit).** `Budget::from_context` needs the model's context-window size, which
   `BudgetConfig` doesn't carry today. Hardcoding a constant in `runner.rs` would
   be throwaway, so add the field where it belongs: `pub context_length: usize`
   on `BudgetConfig` in `executor/src/config.rs`, defaulted in its `Default` impl
   to **`32768`** (a conservative local-model window — the user raises it per
   model; under-estimating compacts early/safely, over-estimating risks
   overflowing the real endpoint). Update the existing `config.rs` TOML
   round-trip test(s) to include/assert the new field. This is the **only**
   permitted `executor/` change in this phase. Per-model resolution from the
   endpoint's `/models` metadata is **not** in scope — phase-02 may revisit.
6. **Errors:** file-read / scope-construction / config failures are
   infra/usage failures → `executor::error::Error` via `?` (or `anyhow` only in
   `main`'s subcommand handler, the binary entry). Model-visible outcomes stay
   inside `PhaseResult` as M4 already models them. No new `unwrap`/`expect` in
   production paths.

## Acceptance criteria

- [ ] `mcp/src/runner.rs` exists; `mod runner;` is wired into `mcp/src/main.rs`;
      `run_phase` + `parse_phase_doc` + `build_registry` are reachable.
- [ ] `BudgetConfig` has a `context_length: usize` field, defaulted to `32768`;
      `cfg.budget.context_length` feeds `Budget::from_context`; the `config.rs`
      TOML round-trip test covers it.
- [ ] `parse_phase_doc` extracts the `## Goal` and `## Acceptance criteria` section
      bodies and the `**Tags:**` line into `Vec<String>`; a doc missing any of
      them yields empty string / empty Vec (no panic).
- [ ] `build_registry` registers all **seven** built-in tools scoped to the given
      root and returns exactly one `ToolSchema` per tool, names matching, in the
      fixed order; the schemas' `parameters` come from each tool's `schema()`.
- [ ] The phase id derives from the path stem: `phase-01-phase-runner.md` →
      `"phase-01"`; a non-matching stem falls back to the whole stem.
- [ ] `run_phase_with`, given a `Config` + a temp phase-doc file + a temp repo dir
      + a `MockAiClient` + `NoopVerifier`/`NoopRunner` (or equivalents) + a fixed
      clock, assembles `PhaseInput`/`LoopDeps` and returns a `PhaseResult`
      (assert a real terminal status, not just "no panic").
- [ ] `rexymcp run-phase …` parses its args, builds `Config`, and calls
      `run_phase` (exercised by a CLI-parse test; the full network path is
      `#[ignore]`-gated or simply not unit-tested — it needs a live endpoint).
- [ ] **Negatives:** `parse_phase_doc` on input with no `## Goal` → empty goal;
      a `**Tags:**` line with extra spaces still splits cleanly; `Scope::new` on a
      non-existent root surfaces as `Err` from `run_phase_with` (not a panic).
- [ ] **No `#[allow(dead_code)]` and no unjustified `#[allow]`** survive (the seam
      struct, not a 12-arg `#[allow]`, is the intended shape).
- [ ] No new dependency. All four required commands pass with zero new warnings.

## Test plan

Hermetic + deterministic, in a `#[cfg(test)] mod tests` at the bottom of
`runner.rs`:

- **`parse_phase_doc`** — positive (a fixture doc with Goal/Acceptance/Tags),
  negatives (missing Goal, missing Tags, spaced Tags line, a `## Goal` followed
  immediately by another `##`).
- **`build_registry`** — count == 7, names == the fixed ordered set, one schema
  per tool, `parameters` non-null for a tool with params (e.g. `read_file`).
- **phase-id derivation** — `phase-01-foo.md` → `phase-01`; `weird-name.md` →
  `weird-name`.
- **`run_phase_with`** — write a phase-doc + an empty repo into a `TempDir`, inject
  a `MockAiClient` scripted to call no tools / complete immediately, a fixed clock,
  Noop verifier/runner; assert a `PhaseResult` with the expected terminal status
  and that `telemetry_dir = None` emits no telemetry file.
- **CLI** — a clap parse test for `run-phase` arg wiring. Do **not** unit-test the
  real-`OpenAiClient` network path (it needs a live endpoint); if a live smoke test
  is wanted, gate it `#[ignore]`.

Use the existing `MockAiClient` (and `NoopVerifier` / `NoopRunner` patterns from
`agent::mod` tests — re-create minimal local equivalents if those are
`#[cfg(test)]`-private to the agent module).

## End-to-end verification

> Not applicable yet — there is no MCP transport until phase-02 and the production
> `run_phase` path needs a live local endpoint. The assembler is exercised by unit
> tests with `MockAiClient` over a `TempDir`. The first real end-to-end dispatch
> (Claude → `execute_phase` MCP tool → `run_phase` → live model) lands in phase-02,
> gated behind `executor_health`.

## Authorizations

- [x] **May create** `mcp/src/runner.rs`; **may modify** `mcp/src/main.rs`
      (`mod runner;` + the `run-phase` clap subcommand + its handler).
- [x] **May modify `executor/src/config.rs`** solely to add
      `context_length: usize` to `BudgetConfig` (+ its `Default` = `32768`, +
      the TOML test) — see Adaptation 5. **Nothing else in `executor/`.**
- [ ] **No new dependencies.** `rmcp` is phase-02.
- [ ] May **NOT** modify any other part of `executor/` — every other seam it
      needs is already public.
- [ ] May **NOT** build the rmcp server / MCP tools (02), log-query tools (03),
      `model_scorecard` (04), progress notifications (05), or roots corroboration
      (06).
- [ ] May **NOT** add the embedded executor contract or any embedding mechanism
      (that is M6).
- [ ] May **NOT** modify `docs/architecture.md`, `STANDARDS.md`, `WORKFLOW.md`,
      `AGENTS.md`, or any phase doc other than this one.

## Out of scope

- **The MCP server, transport, and tools** — phase-02+.
- **Output capping (`MAX_MCP_OUTPUT_TOKENS`)** — phase-02 (print in full here).
- **Cross-project telemetry-dir resolution policy** — phase-02 (plumb `Option`
  here).
- **Embedding the executor contract / STANDARDS** — M6 (inputs here).
- **Progress notifications / `Progress` events** — phase-05.

## Update Log

(Filled in by the executor. See WORKFLOW.md § "Update Log entries".)

<!-- entries appended below this line -->
